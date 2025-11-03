use crate::audio::buffer_pool::BufferPool;
use crate::audio::clip::ClipId;
use crate::audio::midi::{MidiClip, MidiClipId, MidiEvent};
use crate::audio::node_graph::{nodes::*, AudioGraph};
use crate::audio::pool::AudioPool;
use crate::audio::project::Project;
use crate::audio::recording::{MidiRecordingState, RecordingState};
use crate::audio::track::{Track, TrackId, TrackNode};
use crate::command::{AudioEvent, Command, Query, QueryResponse};
use crate::io::MidiInputManager;
use petgraph::stable_graph::NodeIndex;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Audio engine for Phase 6: hierarchical tracks with groups
pub struct Engine {
    project: Project,
    audio_pool: AudioPool,
    buffer_pool: BufferPool,
    playhead: u64,          // Playhead position in samples
    sample_rate: u32,
    playing: bool,
    channels: u32,

    // Lock-free communication
    command_rx: rtrb::Consumer<Command>,
    midi_command_rx: Option<rtrb::Consumer<Command>>,
    event_tx: rtrb::Producer<AudioEvent>,
    query_rx: rtrb::Consumer<Query>,
    query_response_tx: rtrb::Producer<QueryResponse>,

    // Shared playhead for UI reads
    playhead_atomic: Arc<AtomicU64>,

    // Shared MIDI clip ID counter for synchronous access
    next_midi_clip_id_atomic: Arc<AtomicU32>,

    // Event counter for periodic position updates
    frames_since_last_event: usize,
    event_interval_frames: usize,

    // Mix buffer for output
    mix_buffer: Vec<f32>,

    // ID counters
    next_clip_id: ClipId,

    // Recording state
    recording_state: Option<RecordingState>,
    input_rx: Option<rtrb::Consumer<f32>>,
    recording_progress_counter: usize,

    // MIDI recording state
    midi_recording_state: Option<MidiRecordingState>,

    // MIDI input manager for external MIDI devices
    midi_input_manager: Option<MidiInputManager>,
}

impl Engine {
    /// Create a new Engine with communication channels
    pub fn new(
        sample_rate: u32,
        channels: u32,
        command_rx: rtrb::Consumer<Command>,
        event_tx: rtrb::Producer<AudioEvent>,
        query_rx: rtrb::Consumer<Query>,
        query_response_tx: rtrb::Producer<QueryResponse>,
    ) -> Self {
        let event_interval_frames = (sample_rate as usize * channels as usize) / 60; // Update 60 times per second

        // Calculate a reasonable buffer size for the pool (typical audio callback size * channels)
        let buffer_size = 512 * channels as usize;

        Self {
            project: Project::new(sample_rate),
            audio_pool: AudioPool::new(),
            buffer_pool: BufferPool::new(8, buffer_size), // 8 buffers should handle deep nesting
            playhead: 0,
            sample_rate,
            playing: false,
            channels,
            command_rx,
            midi_command_rx: None,
            event_tx,
            query_rx,
            query_response_tx,
            playhead_atomic: Arc::new(AtomicU64::new(0)),
            next_midi_clip_id_atomic: Arc::new(AtomicU32::new(0)),
            frames_since_last_event: 0,
            event_interval_frames,
            mix_buffer: Vec::new(),
            next_clip_id: 0,
            recording_state: None,
            input_rx: None,
            recording_progress_counter: 0,
            midi_recording_state: None,
            midi_input_manager: None,
        }
    }

    /// Set the input ringbuffer consumer for recording
    pub fn set_input_rx(&mut self, input_rx: rtrb::Consumer<f32>) {
        self.input_rx = Some(input_rx);
    }

    /// Set the MIDI input manager for external MIDI devices
    pub fn set_midi_input_manager(&mut self, manager: MidiInputManager) {
        self.midi_input_manager = Some(manager);
    }

    /// Set the MIDI command receiver for external MIDI input
    pub fn set_midi_command_rx(&mut self, midi_command_rx: rtrb::Consumer<Command>) {
        self.midi_command_rx = Some(midi_command_rx);
    }

    /// Add an audio track to the engine
    pub fn add_track(&mut self, track: Track) -> TrackId {
        // For backwards compatibility, we'll extract the track data and add it to the project
        let name = track.name.clone();
        let id = self.project.add_audio_track(name, None);

        // Copy over the track properties
        if let Some(node) = self.project.get_track_mut(id) {
            if let crate::audio::track::TrackNode::Audio(audio_track) = node {
                audio_track.clips = track.clips;
                audio_track.volume = track.volume;
                audio_track.muted = track.muted;
                audio_track.solo = track.solo;
            }
        }

        id
    }

    /// Add an audio track by name
    pub fn add_audio_track(&mut self, name: String) -> TrackId {
        self.project.add_audio_track(name, None)
    }

    /// Add a group track by name
    pub fn add_group_track(&mut self, name: String) -> TrackId {
        self.project.add_group_track(name, None)
    }

    /// Add a MIDI track by name
    pub fn add_midi_track(&mut self, name: String) -> TrackId {
        self.project.add_midi_track(name, None)
    }

    /// Get access to the project
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Get mutable access to the project
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.project
    }

    /// Get mutable reference to audio pool
    pub fn audio_pool_mut(&mut self) -> &mut AudioPool {
        &mut self.audio_pool
    }

    /// Get reference to audio pool
    pub fn audio_pool(&self) -> &AudioPool {
        &self.audio_pool
    }

    /// Get a handle for controlling playback from the UI thread
    pub fn get_controller(
        &self,
        command_tx: rtrb::Producer<Command>,
        query_tx: rtrb::Producer<Query>,
        query_response_rx: rtrb::Consumer<QueryResponse>,
    ) -> EngineController {
        EngineController {
            command_tx,
            query_tx,
            query_response_rx,
            playhead: Arc::clone(&self.playhead_atomic),
            next_midi_clip_id: Arc::clone(&self.next_midi_clip_id_atomic),
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }

    /// Process live MIDI input from all MIDI tracks
    fn process_live_midi(&mut self, output: &mut [f32]) {
        // Process all MIDI tracks to handle live input
        self.project.process_live_midi(output, self.sample_rate, self.channels);
    }

    /// Process audio callback - called from the audio thread
    pub fn process(&mut self, output: &mut [f32]) {
        // Process all pending commands
        while let Ok(cmd) = self.command_rx.pop() {
            self.handle_command(cmd);
        }

        // Process all pending MIDI commands
        loop {
            let midi_cmd = if let Some(ref mut midi_rx) = self.midi_command_rx {
                midi_rx.pop().ok()
            } else {
                None
            };

            if let Some(cmd) = midi_cmd {
                self.handle_command(cmd);
            } else {
                break;
            }
        }

        // Process all pending queries
        while let Ok(query) = self.query_rx.pop() {
            self.handle_query(query);
        }

        if self.playing {
            // Ensure mix buffer is sized correctly
            if self.mix_buffer.len() != output.len() {
                self.mix_buffer.resize(output.len(), 0.0);
            }

            // Ensure buffer pool has the correct buffer size
            if self.buffer_pool.buffer_size() != output.len() {
                // Reallocate buffer pool with correct size if needed
                self.buffer_pool = BufferPool::new(8, output.len());
            }

            // Convert playhead from frames to seconds for timeline-based rendering
            let playhead_seconds = self.playhead as f64 / self.sample_rate as f64;

            // Render the entire project hierarchy into the mix buffer
            self.project.render(
                &mut self.mix_buffer,
                &self.audio_pool,
                &mut self.buffer_pool,
                playhead_seconds,
                self.sample_rate,
                self.channels,
            );

            // Copy mix to output
            output.copy_from_slice(&self.mix_buffer);

            // Update playhead (convert total samples to frames)
            self.playhead += (output.len() / self.channels as usize) as u64;

            // Update atomic playhead for UI reads
            self.playhead_atomic
                .store(self.playhead, Ordering::Relaxed);

            // Send periodic position updates
            self.frames_since_last_event += output.len() / self.channels as usize;
            if self.frames_since_last_event >= self.event_interval_frames / self.channels as usize
            {
                let position_seconds = self.playhead as f64 / self.sample_rate as f64;
                let _ = self
                    .event_tx
                    .push(AudioEvent::PlaybackPosition(position_seconds));
                self.frames_since_last_event = 0;

                // Send MIDI recording progress if active
                if let Some(recording) = &self.midi_recording_state {
                    let current_time = self.playhead as f64 / self.sample_rate as f64;
                    let duration = current_time - recording.start_time;
                    let notes = recording.get_notes().to_vec();
                    let _ = self.event_tx.push(AudioEvent::MidiRecordingProgress(
                        recording.track_id,
                        recording.clip_id,
                        duration,
                        notes,
                    ));
                }
            }
        } else {
            // Not playing, but process live MIDI input
            self.process_live_midi(output);
        }

        // Process recording if active (independent of playback state)
        if let Some(recording) = &mut self.recording_state {
            if let Some(input_rx) = &mut self.input_rx {
                // Pull samples from input ringbuffer
                let mut samples = Vec::new();
                while let Ok(sample) = input_rx.pop() {
                    samples.push(sample);
                }

                // Add samples to recording
                if !samples.is_empty() {
                    match recording.add_samples(&samples) {
                        Ok(_flushed) => {
                            // Update clip duration every callback for sample-accurate timing
                            let duration = recording.duration();
                            let clip_id = recording.clip_id;
                            let track_id = recording.track_id;

                            // Update clip duration in project
                            if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                                if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                                    clip.duration = duration;
                                }
                            }

                            // Send progress event periodically (every ~0.1 seconds)
                            self.recording_progress_counter += samples.len();
                            if self.recording_progress_counter >= (self.sample_rate as usize / 10) {
                                let _ = self.event_tx.push(AudioEvent::RecordingProgress(clip_id, duration));
                                self.recording_progress_counter = 0;
                            }
                        }
                        Err(e) => {
                            // Recording error occurred
                            let _ = self.event_tx.push(AudioEvent::RecordingError(
                                format!("Recording write error: {}", e)
                            ));
                            // Stop recording on error
                            self.recording_state = None;
                        }
                    }
                }
            }
        }
    }

    /// Handle a command from the UI thread
    fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Play => {
                self.playing = true;
            }
            Command::Stop => {
                self.playing = false;
                self.playhead = 0;
                self.playhead_atomic.store(0, Ordering::Relaxed);
                // Stop all MIDI notes when stopping playback
                self.project.stop_all_notes();
            }
            Command::Pause => {
                self.playing = false;
                // Stop all MIDI notes when pausing playback
                self.project.stop_all_notes();
            }
            Command::Seek(seconds) => {
                let frames = (seconds * self.sample_rate as f64) as u64;
                self.playhead = frames;
                self.playhead_atomic
                    .store(self.playhead, Ordering::Relaxed);
                // Stop all MIDI notes when seeking to prevent stuck notes
                self.project.stop_all_notes();
            }
            Command::SetTrackVolume(track_id, volume) => {
                if let Some(track) = self.project.get_track_mut(track_id) {
                    track.set_volume(volume);
                }
            }
            Command::SetTrackMute(track_id, muted) => {
                if let Some(track) = self.project.get_track_mut(track_id) {
                    track.set_muted(muted);
                }
            }
            Command::SetTrackSolo(track_id, solo) => {
                if let Some(track) = self.project.get_track_mut(track_id) {
                    track.set_solo(solo);
                }
            }
            Command::MoveClip(track_id, clip_id, new_start_time) => {
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.start_time = new_start_time;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.start_time = new_start_time;
                        }
                    }
                    _ => {}
                }
            }
            Command::CreateMetatrack(name) => {
                let track_id = self.project.add_group_track(name.clone(), None);
                // Notify UI about the new metatrack
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, true, name));
            }
            Command::AddToMetatrack(track_id, metatrack_id) => {
                // Move the track to the new metatrack (Project handles removing from old parent)
                self.project.move_to_group(track_id, metatrack_id);
            }
            Command::RemoveFromMetatrack(track_id) => {
                // Move to root level (None as parent)
                self.project.move_to_root(track_id);
            }
            Command::SetTimeStretch(track_id, stretch) => {
                if let Some(crate::audio::track::TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.time_stretch = stretch.max(0.01); // Prevent zero or negative stretch
                }
            }
            Command::SetOffset(track_id, offset) => {
                if let Some(crate::audio::track::TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.offset = offset;
                }
            }
            Command::SetPitchShift(track_id, semitones) => {
                if let Some(crate::audio::track::TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.pitch_shift = semitones;
                }
            }
            Command::CreateAudioTrack(name) => {
                let track_id = self.project.add_audio_track(name.clone(), None);
                // Notify UI about the new audio track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
            }
            Command::AddAudioFile(path, data, channels, sample_rate) => {
                // Create AudioFile and add to pool
                let audio_file = crate::audio::pool::AudioFile::new(
                    std::path::PathBuf::from(path.clone()),
                    data,
                    channels,
                    sample_rate,
                );
                let pool_index = self.audio_pool.add_file(audio_file);
                // Notify UI about the new audio file
                let _ = self.event_tx.push(AudioEvent::AudioFileAdded(pool_index, path));
            }
            Command::AddAudioClip(track_id, pool_index, start_time, duration, offset) => {
                eprintln!("[Engine] AddAudioClip: track_id={}, pool_index={}, start_time={}, duration={}",
                    track_id, pool_index, start_time, duration);

                // Check if pool index is valid
                let pool_size = self.audio_pool.len();
                if pool_index >= pool_size {
                    eprintln!("[Engine] ERROR: pool_index {} is out of bounds (pool size: {})",
                        pool_index, pool_size);
                } else {
                    eprintln!("[Engine] Pool index {} is valid, pool has {} files",
                        pool_index, pool_size);
                }

                // Create a new clip with unique ID
                let clip_id = self.next_clip_id;
                self.next_clip_id += 1;
                let clip = crate::audio::clip::Clip::new(
                    clip_id,
                    pool_index,
                    start_time,
                    duration,
                    offset,
                );

                // Add clip to track
                if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                    track.clips.push(clip);
                    eprintln!("[Engine] Clip {} added to track {} successfully", clip_id, track_id);
                    // Notify UI about the new clip
                    let _ = self.event_tx.push(AudioEvent::ClipAdded(track_id, clip_id));
                } else {
                    eprintln!("[Engine] ERROR: Track {} not found or is not an audio track", track_id);
                }
            }
            Command::CreateMidiTrack(name) => {
                let track_id = self.project.add_midi_track(name.clone(), None);
                // Notify UI about the new MIDI track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
            }
            Command::CreateMidiClip(track_id, start_time, duration) => {
                // Get the next MIDI clip ID from the atomic counter
                let clip_id = self.next_midi_clip_id_atomic.fetch_add(1, Ordering::Relaxed);
                let clip = MidiClip::new(clip_id, start_time, duration);
                let _ = self.project.add_midi_clip(track_id, clip);
                // Notify UI about the new clip with its ID
                let _ = self.event_tx.push(AudioEvent::ClipAdded(track_id, clip_id));
            }
            Command::AddMidiNote(track_id, clip_id, time_offset, note, velocity, duration) => {
                // Add a MIDI note event to the specified clip
                if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                        // Timestamp is now in seconds (sample-rate independent)
                        let note_on = MidiEvent::note_on(time_offset, 0, note, velocity);
                        clip.events.push(note_on);

                        // Add note off event
                        let note_off_time = time_offset + duration;
                        let note_off = MidiEvent::note_off(note_off_time, 0, note, 64);
                        clip.events.push(note_off);

                        // Sort events by timestamp (using partial_cmp for f64)
                        clip.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
                    }
                }
            }
            Command::AddLoadedMidiClip(track_id, clip) => {
                // Add a pre-loaded MIDI clip to the track
                let _ = self.project.add_midi_clip(track_id, clip);
            }
            Command::UpdateMidiClipNotes(track_id, clip_id, notes) => {
                // Update all notes in a MIDI clip
                if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                        // Clear existing events
                        clip.events.clear();

                        // Add new events from the notes array
                        // Timestamps are now stored in seconds (sample-rate independent)
                        for (start_time, note, velocity, duration) in notes {
                            let note_on = MidiEvent::note_on(start_time, 0, note, velocity);
                            clip.events.push(note_on);

                            // Add note off event
                            let note_off_time = start_time + duration;
                            let note_off = MidiEvent::note_off(note_off_time, 0, note, 64);
                            clip.events.push(note_off);
                        }

                        // Sort events by timestamp (using partial_cmp for f64)
                        clip.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
                    }
                }
            }
            Command::RequestBufferPoolStats => {
                // Send buffer pool statistics back to UI
                let stats = self.buffer_pool.stats();
                let _ = self.event_tx.push(AudioEvent::BufferPoolStats(stats));
            }
            Command::CreateAutomationLane(track_id, parameter_id) => {
                // Create a new automation lane on the specified track
                let lane_id = match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        Some(track.add_automation_lane(parameter_id))
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        Some(track.add_automation_lane(parameter_id))
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        Some(group.add_automation_lane(parameter_id))
                    }
                    None => None,
                };

                if let Some(lane_id) = lane_id {
                    let _ = self.event_tx.push(AudioEvent::AutomationLaneCreated(
                        track_id,
                        lane_id,
                        parameter_id,
                    ));
                }
            }
            Command::AddAutomationPoint(track_id, lane_id, time, value, curve) => {
                // Add an automation point to the specified lane
                let point = crate::audio::AutomationPoint::new(time, value, curve);

                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.add_point(point);
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.add_point(point);
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(lane) = group.get_automation_lane_mut(lane_id) {
                            lane.add_point(point);
                        }
                    }
                    None => {}
                }
            }
            Command::RemoveAutomationPoint(track_id, lane_id, time, tolerance) => {
                // Remove automation point at specified time
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.remove_point_at_time(time, tolerance);
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.remove_point_at_time(time, tolerance);
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(lane) = group.get_automation_lane_mut(lane_id) {
                            lane.remove_point_at_time(time, tolerance);
                        }
                    }
                    None => {}
                }
            }
            Command::ClearAutomationLane(track_id, lane_id) => {
                // Clear all points from the lane
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.clear();
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.clear();
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(lane) = group.get_automation_lane_mut(lane_id) {
                            lane.clear();
                        }
                    }
                    None => {}
                }
            }
            Command::RemoveAutomationLane(track_id, lane_id) => {
                // Remove the automation lane entirely
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        track.remove_automation_lane(lane_id);
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        track.remove_automation_lane(lane_id);
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        group.remove_automation_lane(lane_id);
                    }
                    None => {}
                }
            }
            Command::SetAutomationLaneEnabled(track_id, lane_id, enabled) => {
                // Enable/disable the automation lane
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.enabled = enabled;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(lane) = track.get_automation_lane_mut(lane_id) {
                            lane.enabled = enabled;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(lane) = group.get_automation_lane_mut(lane_id) {
                            lane.enabled = enabled;
                        }
                    }
                    None => {}
                }
            }
            Command::StartRecording(track_id, start_time) => {
                // Start recording on the specified track
                self.handle_start_recording(track_id, start_time);
            }
            Command::StopRecording => {
                // Stop the current recording
                self.handle_stop_recording();
            }
            Command::PauseRecording => {
                // Pause the current recording
                if let Some(recording) = &mut self.recording_state {
                    recording.pause();
                }
            }
            Command::ResumeRecording => {
                // Resume the current recording
                if let Some(recording) = &mut self.recording_state {
                    recording.resume();
                }
            }
            Command::StartMidiRecording(track_id, clip_id, start_time) => {
                // Start MIDI recording on the specified track
                self.handle_start_midi_recording(track_id, clip_id, start_time);
            }
            Command::StopMidiRecording => {
                eprintln!("[ENGINE] Received StopMidiRecording command");
                // Stop the current MIDI recording
                self.handle_stop_midi_recording();
                eprintln!("[ENGINE] handle_stop_midi_recording() completed");
            }
            Command::Reset => {
                // Reset the entire project to initial state
                // Stop playback
                self.playing = false;
                self.playhead = 0;
                self.playhead_atomic.store(0, Ordering::Relaxed);

                // Stop any active recording
                self.recording_state = None;

                // Clear all project data
                self.project = Project::new(self.sample_rate);

                // Clear audio pool
                self.audio_pool = AudioPool::new();

                // Reset buffer pool (recreate with same settings)
                let buffer_size = 512 * self.channels as usize;
                self.buffer_pool = BufferPool::new(8, buffer_size);

                // Reset ID counters
                self.next_midi_clip_id_atomic.store(0, Ordering::Relaxed);
                self.next_clip_id = 0;

                // Clear mix buffer
                self.mix_buffer.clear();

                // Notify UI that reset is complete
                let _ = self.event_tx.push(AudioEvent::ProjectReset);
            }

            Command::SendMidiNoteOn(track_id, note, velocity) => {
                // Send a live MIDI note on event to the specified track's instrument
                self.project.send_midi_note_on(track_id, note, velocity);

                // If MIDI recording is active on this track, capture the event
                if let Some(recording) = &mut self.midi_recording_state {
                    if recording.track_id == track_id {
                        let absolute_time = self.playhead as f64 / self.sample_rate as f64;
                        eprintln!("[MIDI_RECORDING] NoteOn captured: note={}, velocity={}, absolute_time={:.3}s, playhead={}, sample_rate={}",
                                  note, velocity, absolute_time, self.playhead, self.sample_rate);
                        recording.note_on(note, velocity, absolute_time);
                    }
                }
            }

            Command::SendMidiNoteOff(track_id, note) => {
                // Send a live MIDI note off event to the specified track's instrument
                self.project.send_midi_note_off(track_id, note);

                // If MIDI recording is active on this track, capture the event
                if let Some(recording) = &mut self.midi_recording_state {
                    if recording.track_id == track_id {
                        let absolute_time = self.playhead as f64 / self.sample_rate as f64;
                        eprintln!("[MIDI_RECORDING] NoteOff captured: note={}, absolute_time={:.3}s, playhead={}, sample_rate={}",
                                  note, absolute_time, self.playhead, self.sample_rate);
                        recording.note_off(note, absolute_time);
                    }
                }
            }

            Command::SetActiveMidiTrack(track_id) => {
                // Update the active MIDI track for external MIDI input routing
                if let Some(ref midi_manager) = self.midi_input_manager {
                    midi_manager.set_active_track(track_id);
                }
            }

            // Node graph commands
            Command::GraphAddNode(track_id, node_type, x, y) => {
                eprintln!("[DEBUG] GraphAddNode received: track_id={}, node_type={}, x={}, y={}", track_id, node_type, x, y);

                // Get the track's graph (works for both MIDI and Audio tracks)
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => {
                        eprintln!("[DEBUG] Found MIDI track, using instrument_graph");
                        Some(&mut track.instrument_graph)
                    },
                    Some(TrackNode::Audio(track)) => {
                        eprintln!("[DEBUG] Found Audio track, using effects_graph");
                        Some(&mut track.effects_graph)
                    },
                    _ => {
                        eprintln!("[DEBUG] Track not found or invalid type!");
                        None
                    }
                };

                if let Some(graph) = graph {
                    // Create the node based on type
                    let node: Box<dyn crate::audio::node_graph::AudioNode> = match node_type.as_str() {
                            "Oscillator" => Box::new(OscillatorNode::new("Oscillator".to_string())),
                            "Gain" => Box::new(GainNode::new("Gain".to_string())),
                            "Mixer" => Box::new(MixerNode::new("Mixer".to_string())),
                            "Filter" => Box::new(FilterNode::new("Filter".to_string())),
                            "ADSR" => Box::new(ADSRNode::new("ADSR".to_string())),
                            "LFO" => Box::new(LFONode::new("LFO".to_string())),
                            "NoiseGenerator" => Box::new(NoiseGeneratorNode::new("Noise".to_string())),
                            "Splitter" => Box::new(SplitterNode::new("Splitter".to_string())),
                            "Pan" => Box::new(PanNode::new("Pan".to_string())),
                            "Quantizer" => Box::new(QuantizerNode::new("Quantizer".to_string())),
                            "Delay" => Box::new(DelayNode::new("Delay".to_string())),
                            "Distortion" => Box::new(DistortionNode::new("Distortion".to_string())),
                            "Reverb" => Box::new(ReverbNode::new("Reverb".to_string())),
                            "Chorus" => Box::new(ChorusNode::new("Chorus".to_string())),
                            "Compressor" => Box::new(CompressorNode::new("Compressor".to_string())),
                            "Constant" => Box::new(ConstantNode::new("Constant".to_string())),
                            "BpmDetector" => Box::new(BpmDetectorNode::new("BPM Detector".to_string())),
                            "EnvelopeFollower" => Box::new(EnvelopeFollowerNode::new("Envelope Follower".to_string())),
                            "Limiter" => Box::new(LimiterNode::new("Limiter".to_string())),
                            "Math" => Box::new(MathNode::new("Math".to_string())),
                            "EQ" => Box::new(EQNode::new("EQ".to_string())),
                            "Flanger" => Box::new(FlangerNode::new("Flanger".to_string())),
                            "FMSynth" => Box::new(FMSynthNode::new("FM Synth".to_string())),
                            "Phaser" => Box::new(PhaserNode::new("Phaser".to_string())),
                            "BitCrusher" => Box::new(BitCrusherNode::new("Bit Crusher".to_string())),
                            "Vocoder" => Box::new(VocoderNode::new("Vocoder".to_string())),
                            "RingModulator" => Box::new(RingModulatorNode::new("Ring Modulator".to_string())),
                            "SampleHold" => Box::new(SampleHoldNode::new("Sample & Hold".to_string())),
                            "WavetableOscillator" => Box::new(WavetableOscillatorNode::new("Wavetable".to_string())),
                            "SimpleSampler" => Box::new(SimpleSamplerNode::new("Sampler".to_string())),
                            "SlewLimiter" => Box::new(SlewLimiterNode::new("Slew Limiter".to_string())),
                            "MultiSampler" => Box::new(MultiSamplerNode::new("Multi Sampler".to_string())),
                            "MidiInput" => Box::new(MidiInputNode::new("MIDI Input".to_string())),
                            "MidiToCV" => Box::new(MidiToCVNode::new("MIDI→CV".to_string())),
                            "AudioToCV" => Box::new(AudioToCVNode::new("Audio→CV".to_string())),
                            "AudioInput" => Box::new(AudioInputNode::new("Audio Input".to_string())),
                            "AutomationInput" => Box::new(AutomationInputNode::new("Automation".to_string())),
                            "Oscilloscope" => Box::new(OscilloscopeNode::new("Oscilloscope".to_string())),
                            "TemplateInput" => Box::new(TemplateInputNode::new("Template Input".to_string())),
                            "TemplateOutput" => Box::new(TemplateOutputNode::new("Template Output".to_string())),
                            "VoiceAllocator" => Box::new(VoiceAllocatorNode::new("VoiceAllocator".to_string(), self.sample_rate, 8192)),
                            "AudioOutput" => Box::new(AudioOutputNode::new("Output".to_string())),
                            _ => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Unknown node type: {}", node_type)
                                ));
                                return;
                            }
                        };

                    // Add node to graph
                    let node_idx = graph.add_node(node);
                    let node_id = node_idx.index() as u32;
                    eprintln!("[DEBUG] Node added with index: {:?}, converted to u32 id: {}", node_idx, node_id);

                    // Save position
                    graph.set_node_position(node_idx, x, y);

                    // Automatically set MIDI-receiving nodes as MIDI targets
                    if node_type == "MidiInput" || node_type == "VoiceAllocator" {
                        graph.set_midi_target(node_idx, true);
                    }

                    // Automatically set AudioOutput nodes as the graph output
                    if node_type == "AudioOutput" {
                        graph.set_output_node(Some(node_idx));
                    }

                    eprintln!("[DEBUG] Emitting GraphNodeAdded event: track_id={}, node_id={}, node_type={}", track_id, node_id, node_type);
                    // Emit success event
                    let _ = self.event_tx.push(AudioEvent::GraphNodeAdded(track_id, node_id, node_type.clone()));
                } else {
                    eprintln!("[DEBUG] Graph was None, node not added!");
                }
            }

            Command::GraphAddNodeToTemplate(track_id, voice_allocator_id, node_type, _x, _y) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    {
                        let va_idx = NodeIndex::new(voice_allocator_id as usize);

                        // Create the node
                        let node: Box<dyn crate::audio::node_graph::AudioNode> = match node_type.as_str() {
                            "Oscillator" => Box::new(OscillatorNode::new("Oscillator".to_string())),
                            "Gain" => Box::new(GainNode::new("Gain".to_string())),
                            "Mixer" => Box::new(MixerNode::new("Mixer".to_string())),
                            "Filter" => Box::new(FilterNode::new("Filter".to_string())),
                            "ADSR" => Box::new(ADSRNode::new("ADSR".to_string())),
                            "LFO" => Box::new(LFONode::new("LFO".to_string())),
                            "NoiseGenerator" => Box::new(NoiseGeneratorNode::new("Noise".to_string())),
                            "Splitter" => Box::new(SplitterNode::new("Splitter".to_string())),
                            "Pan" => Box::new(PanNode::new("Pan".to_string())),
                            "Quantizer" => Box::new(QuantizerNode::new("Quantizer".to_string())),
                            "Delay" => Box::new(DelayNode::new("Delay".to_string())),
                            "Distortion" => Box::new(DistortionNode::new("Distortion".to_string())),
                            "Reverb" => Box::new(ReverbNode::new("Reverb".to_string())),
                            "Chorus" => Box::new(ChorusNode::new("Chorus".to_string())),
                            "Compressor" => Box::new(CompressorNode::new("Compressor".to_string())),
                            "Constant" => Box::new(ConstantNode::new("Constant".to_string())),
                            "BpmDetector" => Box::new(BpmDetectorNode::new("BPM Detector".to_string())),
                            "EnvelopeFollower" => Box::new(EnvelopeFollowerNode::new("Envelope Follower".to_string())),
                            "Limiter" => Box::new(LimiterNode::new("Limiter".to_string())),
                            "Math" => Box::new(MathNode::new("Math".to_string())),
                            "EQ" => Box::new(EQNode::new("EQ".to_string())),
                            "Flanger" => Box::new(FlangerNode::new("Flanger".to_string())),
                            "FMSynth" => Box::new(FMSynthNode::new("FM Synth".to_string())),
                            "Phaser" => Box::new(PhaserNode::new("Phaser".to_string())),
                            "BitCrusher" => Box::new(BitCrusherNode::new("Bit Crusher".to_string())),
                            "Vocoder" => Box::new(VocoderNode::new("Vocoder".to_string())),
                            "RingModulator" => Box::new(RingModulatorNode::new("Ring Modulator".to_string())),
                            "SampleHold" => Box::new(SampleHoldNode::new("Sample & Hold".to_string())),
                            "WavetableOscillator" => Box::new(WavetableOscillatorNode::new("Wavetable".to_string())),
                            "SimpleSampler" => Box::new(SimpleSamplerNode::new("Sampler".to_string())),
                            "SlewLimiter" => Box::new(SlewLimiterNode::new("Slew Limiter".to_string())),
                            "MultiSampler" => Box::new(MultiSamplerNode::new("Multi Sampler".to_string())),
                            "MidiInput" => Box::new(MidiInputNode::new("MIDI Input".to_string())),
                            "MidiToCV" => Box::new(MidiToCVNode::new("MIDI→CV".to_string())),
                            "AudioToCV" => Box::new(AudioToCVNode::new("Audio→CV".to_string())),
                            "AutomationInput" => Box::new(AutomationInputNode::new("Automation".to_string())),
                            "Oscilloscope" => Box::new(OscilloscopeNode::new("Oscilloscope".to_string())),
                            "TemplateInput" => Box::new(TemplateInputNode::new("Template Input".to_string())),
                            "TemplateOutput" => Box::new(TemplateOutputNode::new("Template Output".to_string())),
                            "AudioOutput" => Box::new(AudioOutputNode::new("Output".to_string())),
                            _ => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Unknown node type: {}", node_type)
                                ));
                                return;
                            }
                        };

                        // Add node to VoiceAllocator's template graph
                        match graph.add_node_to_voice_allocator_template(va_idx, node) {
                            Ok(node_id) => {
                                println!("Added node {} (ID: {}) to VoiceAllocator {} template", node_type, node_id, voice_allocator_id);
                                let _ = self.event_tx.push(AudioEvent::GraphNodeAdded(track_id, node_id, node_type.clone()));
                            }
                            Err(e) => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Failed to add node to template: {}", e)
                                ));
                            }
                        }
                    }
                }
            }

            Command::GraphRemoveNode(track_id, node_index) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    graph.remove_node(node_idx);
                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
            }

            Command::GraphConnect(track_id, from, from_port, to, to_port) => {
                eprintln!("[DEBUG] GraphConnect received: track_id={}, from={}, from_port={}, to={}, to_port={}", track_id, from, from_port, to, to_port);

                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => {
                        eprintln!("[DEBUG] Found MIDI track for connection");
                        Some(&mut track.instrument_graph)
                    },
                    Some(TrackNode::Audio(track)) => {
                        eprintln!("[DEBUG] Found Audio track for connection");
                        Some(&mut track.effects_graph)
                    },
                    _ => {
                        eprintln!("[DEBUG] Track not found for connection!");
                        None
                    }
                };
                if let Some(graph) = graph {
                    let from_idx = NodeIndex::new(from as usize);
                    let to_idx = NodeIndex::new(to as usize);
                    eprintln!("[DEBUG] Attempting to connect nodes: {:?} port {} -> {:?} port {}", from_idx, from_port, to_idx, to_port);

                    match graph.connect(from_idx, from_port, to_idx, to_port) {
                        Ok(()) => {
                            eprintln!("[DEBUG] Connection successful!");
                            let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                        }
                        Err(e) => {
                            eprintln!("[DEBUG] Connection failed: {:?}", e);
                            let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                track_id,
                                format!("{:?}", e)
                            ));
                        }
                    }
                } else {
                    eprintln!("[DEBUG] No graph found, connection not made");
                }
            }

            Command::GraphConnectInTemplate(track_id, voice_allocator_id, from, from_port, to, to_port) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    {
                        let va_idx = NodeIndex::new(voice_allocator_id as usize);

                        match graph.connect_in_voice_allocator_template(va_idx, from, from_port, to, to_port) {
                            Ok(()) => {
                                println!("Connected nodes in VoiceAllocator {} template: {} -> {}", voice_allocator_id, from, to);
                                let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                            }
                            Err(e) => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Failed to connect in template: {}", e)
                                ));
                            }
                        }
                    }
                }
            }

            Command::GraphDisconnect(track_id, from, from_port, to, to_port) => {
                eprintln!("[AUDIO ENGINE] GraphDisconnect: track={}, from={}, from_port={}, to={}, to_port={}", track_id, from, from_port, to, to_port);
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => {
                        eprintln!("[AUDIO ENGINE] Found audio track, disconnecting in effects_graph");
                        Some(&mut track.effects_graph)
                    }
                    _ => {
                        eprintln!("[AUDIO ENGINE] Track not found!");
                        None
                    }
                };
                if let Some(graph) = graph {
                    let from_idx = NodeIndex::new(from as usize);
                    let to_idx = NodeIndex::new(to as usize);
                    graph.disconnect(from_idx, from_port, to_idx, to_port);
                    eprintln!("[AUDIO ENGINE] Disconnect completed");
                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
            }

            Command::GraphSetParameter(track_id, node_index, param_id, value) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        graph_node.node.set_parameter(param_id, value);
                    }
                }
            }

            Command::GraphSetMidiTarget(track_id, node_index, enabled) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    {
                        let node_idx = NodeIndex::new(node_index as usize);
                        graph.set_midi_target(node_idx, enabled);
                    }
                }
            }

            Command::GraphSetOutputNode(track_id, node_index) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    graph.set_output_node(Some(node_idx));
                }
            }

            Command::GraphSavePreset(track_id, preset_path, preset_name, description, tags) => {
                let graph = match self.project.get_track(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    // Serialize the graph to a preset
                    let mut preset = graph.to_preset(&preset_name);
                    preset.metadata.description = description;
                    preset.metadata.tags = tags;
                    preset.metadata.author = String::from("User");

                    // Write to file
                    if let Ok(json) = preset.to_json() {
                        if let Err(e) = std::fs::write(&preset_path, json) {
                            let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                track_id,
                                format!("Failed to save preset: {}", e)
                            ));
                        }
                    } else {
                        let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                            track_id,
                            "Failed to serialize preset".to_string()
                        ));
                    }
                }
            }

            Command::GraphLoadPreset(track_id, preset_path) => {
                // Read and deserialize the preset
                match std::fs::read_to_string(&preset_path) {
                    Ok(json) => {
                        match crate::audio::node_graph::preset::GraphPreset::from_json(&json) {
                            Ok(preset) => {
                                // Extract the directory path from the preset path for resolving relative sample paths
                                let preset_base_path = std::path::Path::new(&preset_path).parent();

                                match AudioGraph::from_preset(&preset, self.sample_rate, 8192, preset_base_path) {
                                    Ok(graph) => {
                                        // Replace the track's graph
                                        match self.project.get_track_mut(track_id) {
                                            Some(TrackNode::Midi(track)) => {
                                                track.instrument_graph = graph;
                                                let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                                                let _ = self.event_tx.push(AudioEvent::GraphPresetLoaded(track_id));
                                            }
                                            Some(TrackNode::Audio(track)) => {
                                                track.effects_graph = graph;
                                                let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                                                let _ = self.event_tx.push(AudioEvent::GraphPresetLoaded(track_id));
                                            }
                                            _ => {}
                                        }
                                    }
                                    Err(e) => {
                                        let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                            track_id,
                                            format!("Failed to create graph from preset: {}", e)
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Failed to parse preset: {}", e)
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                            track_id,
                            format!("Failed to read preset file: {}", e)
                        ));
                    }
                }
            }

            Command::GraphSaveTemplatePreset(track_id, voice_allocator_id, preset_path, preset_name) => {
                use crate::audio::node_graph::nodes::VoiceAllocatorNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &track.instrument_graph;
                    let va_idx = NodeIndex::new(voice_allocator_id as usize);

                    // Get the VoiceAllocator node and serialize its template
                    if let Some(node) = graph.get_node(va_idx) {
                        // Downcast to VoiceAllocatorNode
                        let node_ptr = node as *const dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *const VoiceAllocatorNode;

                        unsafe {
                            let va_node = &*node_ptr;
                            let template_preset = va_node.template_graph().to_preset(&preset_name);

                            // Write to file
                            if let Ok(json) = template_preset.to_json() {
                                if let Err(e) = std::fs::write(&preset_path, json) {
                                    eprintln!("Failed to save template preset: {}", e);
                                }
                            }
                        }
                    }
                }
            }

            Command::SamplerLoadSample(track_id, node_id, file_path) => {
                use crate::audio::node_graph::nodes::SimpleSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to SimpleSamplerNode
                        let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *mut SimpleSamplerNode;

                        unsafe {
                            let sampler_node = &mut *node_ptr;
                            if let Err(e) = sampler_node.load_sample_from_file(&file_path) {
                                eprintln!("Failed to load sample: {}", e);
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerAddLayer(track_id, node_id, file_path, key_min, key_max, root_key, velocity_min, velocity_max) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to MultiSamplerNode
                        let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *mut MultiSamplerNode;

                        unsafe {
                            let multi_sampler_node = &mut *node_ptr;
                            if let Err(e) = multi_sampler_node.load_layer_from_file(&file_path, key_min, key_max, root_key, velocity_min, velocity_max) {
                                eprintln!("Failed to add sample layer: {}", e);
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerUpdateLayer(track_id, node_id, layer_index, key_min, key_max, root_key, velocity_min, velocity_max) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to MultiSamplerNode
                        let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *mut MultiSamplerNode;

                        unsafe {
                            let multi_sampler_node = &mut *node_ptr;
                            if let Err(e) = multi_sampler_node.update_layer(layer_index, key_min, key_max, root_key, velocity_min, velocity_max) {
                                eprintln!("Failed to update sample layer: {}", e);
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerRemoveLayer(track_id, node_id, layer_index) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to MultiSamplerNode
                        let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *mut MultiSamplerNode;

                        unsafe {
                            let multi_sampler_node = &mut *node_ptr;
                            if let Err(e) = multi_sampler_node.remove_layer(layer_index) {
                                eprintln!("Failed to remove sample layer: {}", e);
                            }
                        }
                    }
                }
            }

            Command::AutomationAddKeyframe(track_id, node_id, time, value, interpolation_str, ease_out, ease_in) => {
                use crate::audio::node_graph::nodes::{AutomationInputNode, AutomationKeyframe, InterpolationType};

                // Parse interpolation type
                let interpolation = match interpolation_str.to_lowercase().as_str() {
                    "linear" => InterpolationType::Linear,
                    "bezier" => InterpolationType::Bezier,
                    "step" => InterpolationType::Step,
                    "hold" => InterpolationType::Hold,
                    _ => {
                        eprintln!("Unknown interpolation type: {}, defaulting to Linear", interpolation_str);
                        InterpolationType::Linear
                    }
                };

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to AutomationInputNode using as_any_mut
                        if let Some(auto_node) = graph_node.node.as_any_mut().downcast_mut::<AutomationInputNode>() {
                            let keyframe = AutomationKeyframe {
                                time,
                                value,
                                interpolation,
                                ease_out,
                                ease_in,
                            };
                            auto_node.add_keyframe(keyframe);
                        } else {
                            eprintln!("Node {} is not an AutomationInputNode", node_id);
                        }
                    }
                }
            }

            Command::AutomationRemoveKeyframe(track_id, node_id, time) => {
                use crate::audio::node_graph::nodes::AutomationInputNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(auto_node) = graph_node.node.as_any_mut().downcast_mut::<AutomationInputNode>() {
                            auto_node.remove_keyframe_at_time(time, 0.001); // 1ms tolerance
                        } else {
                            eprintln!("Node {} is not an AutomationInputNode", node_id);
                        }
                    }
                }
            }

            Command::AutomationSetName(track_id, node_id, name) => {
                use crate::audio::node_graph::nodes::AutomationInputNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(auto_node) = graph_node.node.as_any_mut().downcast_mut::<AutomationInputNode>() {
                            auto_node.set_display_name(name);
                        } else {
                            eprintln!("Node {} is not an AutomationInputNode", node_id);
                        }
                    }
                }
            }
        }
    }

    /// Handle synchronous queries from the UI thread
    fn handle_query(&mut self, query: Query) {
        let response = match query {
            Query::GetGraphState(track_id) => {
                match self.project.get_track(track_id) {
                    Some(TrackNode::Midi(track)) => {
                        let graph = &track.instrument_graph;
                        let preset = graph.to_preset("temp");
                        match preset.to_json() {
                            Ok(json) => QueryResponse::GraphState(Ok(json)),
                            Err(e) => QueryResponse::GraphState(Err(format!("Failed to serialize graph: {:?}", e))),
                        }
                    }
                    Some(TrackNode::Audio(track)) => {
                        let graph = &track.effects_graph;
                        let preset = graph.to_preset("temp");
                        match preset.to_json() {
                            Ok(json) => QueryResponse::GraphState(Ok(json)),
                            Err(e) => QueryResponse::GraphState(Err(format!("Failed to serialize graph: {:?}", e))),
                        }
                    }
                    _ => {
                        QueryResponse::GraphState(Err(format!("Track {} not found", track_id)))
                    }
                }
            }
            Query::GetTemplateState(track_id, voice_allocator_id) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(voice_allocator_id as usize);
                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to VoiceAllocatorNode
                        let node_ptr = &*graph_node.node as *const dyn crate::audio::node_graph::AudioNode;
                        let node_ptr = node_ptr as *const VoiceAllocatorNode;
                        unsafe {
                            let va_node = &*node_ptr;
                            let template_preset = va_node.template_graph().to_preset("template");
                            match template_preset.to_json() {
                                Ok(json) => QueryResponse::GraphState(Ok(json)),
                                Err(e) => QueryResponse::GraphState(Err(format!("Failed to serialize template: {:?}", e))),
                            }
                        }
                    } else {
                        QueryResponse::GraphState(Err("Voice allocator node not found".to_string()))
                    }
                } else {
                    QueryResponse::GraphState(Err(format!("Track {} not found or is not a MIDI track", track_id)))
                }
            }
            Query::GetOscilloscopeData(track_id, node_id, sample_count) => {
                match self.project.get_oscilloscope_data(track_id, node_id, sample_count) {
                    Some((audio, cv)) => {
                        use crate::command::OscilloscopeData;
                        QueryResponse::OscilloscopeData(Ok(OscilloscopeData { audio, cv }))
                    }
                    None => QueryResponse::OscilloscopeData(Err(format!(
                        "Failed to get oscilloscope data from track {} node {}",
                        track_id, node_id
                    ))),
                }
            }
            Query::GetMidiClip(track_id, clip_id) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track(track_id) {
                    if let Some(clip) = track.clips.iter().find(|c| c.id == clip_id) {
                        use crate::command::MidiClipData;
                        QueryResponse::MidiClipData(Ok(MidiClipData {
                            duration: clip.duration,
                            events: clip.events.clone(),
                        }))
                    } else {
                        QueryResponse::MidiClipData(Err(format!("Clip {} not found in track {}", clip_id, track_id)))
                    }
                } else {
                    QueryResponse::MidiClipData(Err(format!("Track {} not found or is not a MIDI track", track_id)))
                }
            }

            Query::GetAutomationKeyframes(track_id, node_id) => {
                use crate::audio::node_graph::nodes::{AutomationInputNode, InterpolationType};
                use crate::command::types::AutomationKeyframeData;

                if let Some(TrackNode::Midi(track)) = self.project.get_track(track_id) {
                    let graph = &track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node(node_idx) {
                        // Downcast to AutomationInputNode
                        if let Some(auto_node) = graph_node.node.as_any().downcast_ref::<AutomationInputNode>() {
                            let keyframes: Vec<AutomationKeyframeData> = auto_node.keyframes()
                                .iter()
                                .map(|kf| {
                                    let interpolation_str = match kf.interpolation {
                                        InterpolationType::Linear => "linear",
                                        InterpolationType::Bezier => "bezier",
                                        InterpolationType::Step => "step",
                                        InterpolationType::Hold => "hold",
                                    }.to_string();

                                    AutomationKeyframeData {
                                        time: kf.time,
                                        value: kf.value,
                                        interpolation: interpolation_str,
                                        ease_out: kf.ease_out,
                                        ease_in: kf.ease_in,
                                    }
                                })
                                .collect();

                            QueryResponse::AutomationKeyframes(Ok(keyframes))
                        } else {
                            QueryResponse::AutomationKeyframes(Err(format!("Node {} is not an AutomationInputNode", node_id)))
                        }
                    } else {
                        QueryResponse::AutomationKeyframes(Err(format!("Node {} not found in track {}", node_id, track_id)))
                    }
                } else {
                    QueryResponse::AutomationKeyframes(Err(format!("Track {} not found or is not a MIDI track", track_id)))
                }
            }

            Query::GetAutomationName(track_id, node_id) => {
                use crate::audio::node_graph::nodes::AutomationInputNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track(track_id) {
                    let graph = &track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node(node_idx) {
                        // Downcast to AutomationInputNode
                        if let Some(auto_node) = graph_node.node.as_any().downcast_ref::<AutomationInputNode>() {
                            QueryResponse::AutomationName(Ok(auto_node.display_name().to_string()))
                        } else {
                            QueryResponse::AutomationName(Err(format!("Node {} is not an AutomationInputNode", node_id)))
                        }
                    } else {
                        QueryResponse::AutomationName(Err(format!("Node {} not found in track {}", node_id, track_id)))
                    }
                } else {
                    QueryResponse::AutomationName(Err(format!("Track {} not found or is not a MIDI track", track_id)))
                }
            }

            Query::SerializeAudioPool(project_path) => {
                QueryResponse::AudioPoolSerialized(self.audio_pool.serialize(&project_path))
            }

            Query::LoadAudioPool(entries, project_path) => {
                QueryResponse::AudioPoolLoaded(self.audio_pool.load_from_serialized(entries, &project_path))
            }

            Query::ResolveMissingAudioFile(pool_index, new_path) => {
                QueryResponse::AudioFileResolved(self.audio_pool.resolve_missing_file(pool_index, &new_path))
            }

            Query::SerializeTrackGraph(track_id, _project_path) => {
                // Get the track and serialize its graph
                if let Some(track_node) = self.project.get_track(track_id) {
                    let preset_json = match track_node {
                        TrackNode::Audio(track) => {
                            // Serialize effects graph
                            let preset = track.effects_graph.to_preset(format!("track_{}_effects", track_id));
                            serde_json::to_string_pretty(&preset)
                                .map_err(|e| format!("Failed to serialize effects graph: {}", e))
                        }
                        TrackNode::Midi(track) => {
                            // Serialize instrument graph
                            let preset = track.instrument_graph.to_preset(format!("track_{}_instrument", track_id));
                            serde_json::to_string_pretty(&preset)
                                .map_err(|e| format!("Failed to serialize instrument graph: {}", e))
                        }
                        TrackNode::Group(_) => {
                            // TODO: Add graph serialization when we add graphs to group tracks
                            Err("Group tracks don't have graphs to serialize yet".to_string())
                        }
                    };
                    QueryResponse::TrackGraphSerialized(preset_json)
                } else {
                    QueryResponse::TrackGraphSerialized(Err(format!("Track {} not found", track_id)))
                }
            }

            Query::LoadTrackGraph(track_id, preset_json, project_path) => {
                // Parse preset and load into track's graph
                use crate::audio::node_graph::preset::GraphPreset;

                let result = (|| -> Result<(), String> {
                    let preset: GraphPreset = serde_json::from_str(&preset_json)
                        .map_err(|e| format!("Failed to parse preset JSON: {}", e))?;

                    let preset_base_path = project_path.parent();

                    if let Some(track_node) = self.project.get_track_mut(track_id) {
                        match track_node {
                            TrackNode::Audio(track) => {
                                // Load into effects graph with proper buffer size (8192 to handle any callback size)
                                track.effects_graph = AudioGraph::from_preset(&preset, self.sample_rate, 8192, preset_base_path)?;
                                Ok(())
                            }
                            TrackNode::Midi(track) => {
                                // Load into instrument graph with proper buffer size (8192 to handle any callback size)
                                track.instrument_graph = AudioGraph::from_preset(&preset, self.sample_rate, 8192, preset_base_path)?;
                                Ok(())
                            }
                            TrackNode::Group(_) => {
                                // TODO: Add graph loading when we add graphs to group tracks
                                Err("Group tracks don't have graphs to load yet".to_string())
                            }
                        }
                    } else {
                        Err(format!("Track {} not found", track_id))
                    }
                })();

                QueryResponse::TrackGraphLoaded(result)
            }
            Query::CreateAudioTrackSync(name) => {
                let track_id = self.project.add_audio_track(name.clone(), None);
                eprintln!("[Engine] Created audio track '{}' with ID {}", name, track_id);
                // Notify UI about the new audio track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
                QueryResponse::TrackCreated(Ok(track_id))
            }
            Query::CreateMidiTrackSync(name) => {
                let track_id = self.project.add_midi_track(name.clone(), None);
                eprintln!("[Engine] Created MIDI track '{}' with ID {}", name, track_id);
                // Notify UI about the new MIDI track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
                QueryResponse::TrackCreated(Ok(track_id))
            }
            Query::GetPoolWaveform(pool_index, target_peaks) => {
                match self.audio_pool.generate_waveform(pool_index, target_peaks) {
                    Some(waveform) => QueryResponse::PoolWaveform(Ok(waveform)),
                    None => QueryResponse::PoolWaveform(Err(format!("Pool index {} not found", pool_index))),
                }
            }
            Query::GetPoolFileInfo(pool_index) => {
                match self.audio_pool.get_file_info(pool_index) {
                    Some(info) => QueryResponse::PoolFileInfo(Ok(info)),
                    None => QueryResponse::PoolFileInfo(Err(format!("Pool index {} not found", pool_index))),
                }
            }
        };

        // Send response back
        let _ = self.query_response_tx.push(response);
    }

    /// Handle starting a recording
    fn handle_start_recording(&mut self, track_id: TrackId, start_time: f64) {
        use crate::io::WavWriter;
        use std::env;

        // Check if track exists and is an audio track
        if let Some(crate::audio::track::TrackNode::Audio(_)) = self.project.get_track_mut(track_id) {
            // Generate a unique temp file path
            let temp_dir = env::temp_dir();
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let temp_file_path = temp_dir.join(format!("daw_recording_{}.wav", timestamp));

            // Create WAV writer
            match WavWriter::create(&temp_file_path, self.sample_rate, self.channels) {
                Ok(writer) => {
                    // Create intermediate clip
                    let clip_id = self.next_clip_id;
                    self.next_clip_id += 1;

                    let clip = crate::audio::clip::Clip::new(
                        clip_id,
                        0, // Temporary pool index, will be updated on finalization
                        start_time,
                        0.0, // Duration starts at 0, will be updated during recording
                        0.0,
                    );

                    // Add clip to track
                    if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                        track.clips.push(clip);
                    }

                    // Create recording state
                    let flush_interval_seconds = 1.0; // Flush every 1 second (safer than 5 seconds)
                    let recording_state = RecordingState::new(
                        track_id,
                        clip_id,
                        temp_file_path,
                        writer,
                        self.sample_rate,
                        self.channels,
                        start_time,
                        flush_interval_seconds,
                    );

                    // Check how many samples are currently in the input buffer and mark them for skipping
                    let samples_in_buffer = if let Some(input_rx) = &self.input_rx {
                        input_rx.slots()  // Number of samples currently in the buffer
                    } else {
                        0
                    };

                    self.recording_state = Some(recording_state);
                    self.recording_progress_counter = 0; // Reset progress counter

                    // Set the number of samples to skip on the recording state
                    if let Some(recording) = &mut self.recording_state {
                        recording.samples_to_skip = samples_in_buffer;
                        if samples_in_buffer > 0 {
                            eprintln!("Will skip {} stale samples from input buffer", samples_in_buffer);
                        }
                    }

                    // Notify UI that recording has started
                    let _ = self.event_tx.push(AudioEvent::RecordingStarted(track_id, clip_id));
                }
                Err(e) => {
                    // Send error event to UI
                    let _ = self.event_tx.push(AudioEvent::RecordingError(
                        format!("Failed to create temp file: {}", e)
                    ));
                }
            }
        } else {
            // Send error event if track not found or not an audio track
            let _ = self.event_tx.push(AudioEvent::RecordingError(
                format!("Track {} not found or is not an audio track", track_id)
            ));
        }
    }

    /// Handle stopping a recording
    fn handle_stop_recording(&mut self) {
        eprintln!("[STOP_RECORDING] handle_stop_recording called");

        // Check if we have an active MIDI recording first
        if self.midi_recording_state.is_some() {
            eprintln!("[STOP_RECORDING] Detected active MIDI recording, delegating to handle_stop_midi_recording");
            self.handle_stop_midi_recording();
            return;
        }

        // Handle audio recording
        if let Some(recording) = self.recording_state.take() {
            let clip_id = recording.clip_id;
            let track_id = recording.track_id;
            let sample_rate = recording.sample_rate;
            let channels = recording.channels;

            eprintln!("[STOP_RECORDING] Stopping recording for clip_id={}, track_id={}", clip_id, track_id);

            // Finalize the recording (flush buffers, close file, get waveform and audio data)
            let frames_recorded = recording.frames_written;
            eprintln!("[STOP_RECORDING] Calling finalize() - frames_recorded={}", frames_recorded);
            match recording.finalize() {
                Ok((temp_file_path, waveform, audio_data)) => {
                    eprintln!("[STOP_RECORDING] Finalize succeeded: {} frames written to {:?}, {} waveform peaks generated, {} samples in memory",
                              frames_recorded, temp_file_path, waveform.len(), audio_data.len());

                    // Add to pool using the in-memory audio data (no file loading needed!)
                    let pool_file = crate::audio::pool::AudioFile::new(
                        temp_file_path.clone(),
                        audio_data,
                        channels,
                        sample_rate,
                    );
                    let pool_index = self.audio_pool.add_file(pool_file);
                    eprintln!("[STOP_RECORDING] Added to pool at index {}", pool_index);

                    // Update the clip to reference the pool
                    if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.audio_pool_index = pool_index;
                            eprintln!("[STOP_RECORDING] Updated clip {} with pool_index {}", clip_id, pool_index);
                        }
                    }

                    // Delete temp file
                    let _ = std::fs::remove_file(&temp_file_path);

                    // Send event with the incrementally-generated waveform
                    eprintln!("[STOP_RECORDING] Pushing RecordingStopped event for clip_id={}, pool_index={}, waveform_peaks={}",
                              clip_id, pool_index, waveform.len());
                    let _ = self.event_tx.push(AudioEvent::RecordingStopped(clip_id, pool_index, waveform));
                    eprintln!("[STOP_RECORDING] RecordingStopped event pushed successfully");
                }
                Err(e) => {
                    eprintln!("[STOP_RECORDING] Finalize failed: {}", e);
                    let _ = self.event_tx.push(AudioEvent::RecordingError(
                        format!("Failed to finalize recording: {}", e)
                    ));
                }
            }
        } else {
            eprintln!("[STOP_RECORDING] No active recording to stop");
        }
    }

    /// Handle starting MIDI recording
    fn handle_start_midi_recording(&mut self, track_id: TrackId, clip_id: MidiClipId, start_time: f64) {
        // Check if track exists and is a MIDI track
        if let Some(crate::audio::track::TrackNode::Midi(_)) = self.project.get_track_mut(track_id) {
            // Create MIDI recording state
            let recording_state = MidiRecordingState::new(track_id, clip_id, start_time);
            self.midi_recording_state = Some(recording_state);

            eprintln!("[MIDI_RECORDING] Started MIDI recording on track {} for clip {}", track_id, clip_id);
        } else {
            // Send error event if track not found or not a MIDI track
            let _ = self.event_tx.push(AudioEvent::RecordingError(
                format!("Track {} not found or is not a MIDI track", track_id)
            ));
        }
    }

    /// Handle stopping MIDI recording
    fn handle_stop_midi_recording(&mut self) {
        eprintln!("[MIDI_RECORDING] handle_stop_midi_recording called");
        if let Some(mut recording) = self.midi_recording_state.take() {
            // Close out any active notes at the current playhead position
            let end_time = self.playhead as f64 / self.sample_rate as f64;
            eprintln!("[MIDI_RECORDING] Closing active notes at time {}", end_time);
            recording.close_active_notes(end_time);

            let clip_id = recording.clip_id;
            let track_id = recording.track_id;
            let notes = recording.get_notes().to_vec();
            let note_count = notes.len();
            let recording_duration = end_time - recording.start_time;

            eprintln!("[MIDI_RECORDING] Stopping MIDI recording for clip_id={}, track_id={}, captured {} notes, duration={:.3}s",
                      clip_id, track_id, note_count, recording_duration);

            // Update the MIDI clip using the existing UpdateMidiClipNotes logic
            eprintln!("[MIDI_RECORDING] Looking for track {} to update clip", track_id);
            if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                eprintln!("[MIDI_RECORDING] Found MIDI track, looking for clip {}", clip_id);
                if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                    eprintln!("[MIDI_RECORDING] Found clip, clearing and adding {} notes", note_count);
                    // Clear existing events
                    clip.events.clear();

                    // Update clip duration to match the actual recording time
                    clip.duration = recording_duration;

                    // Add new events from the recorded notes
                    // Timestamps are now stored in seconds (sample-rate independent)
                    for (start_time, note, velocity, duration) in notes.iter() {
                        let note_on = MidiEvent::note_on(*start_time, 0, *note, *velocity);

                        eprintln!("[MIDI_RECORDING] Note {}: start_time={:.3}s, duration={:.3}s",
                                  note, start_time, duration);

                        clip.events.push(note_on);

                        // Add note off event
                        let note_off_time = *start_time + *duration;
                        let note_off = MidiEvent::note_off(note_off_time, 0, *note, 64);
                        clip.events.push(note_off);
                    }

                    // Sort events by timestamp (using partial_cmp for f64)
                    clip.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
                    eprintln!("[MIDI_RECORDING] Updated clip {} with {} notes ({} events)", clip_id, note_count, clip.events.len());
                } else {
                    eprintln!("[MIDI_RECORDING] ERROR: Clip {} not found on track!", clip_id);
                }
            } else {
                eprintln!("[MIDI_RECORDING] ERROR: Track {} not found or not a MIDI track!", track_id);
            }

            // Send event to UI
            eprintln!("[MIDI_RECORDING] Pushing MidiRecordingStopped event to event_tx...");
            match self.event_tx.push(AudioEvent::MidiRecordingStopped(track_id, clip_id, note_count)) {
                Ok(_) => eprintln!("[MIDI_RECORDING] MidiRecordingStopped event pushed successfully"),
                Err(e) => eprintln!("[MIDI_RECORDING] ERROR: Failed to push event: {:?}", e),
            }
        } else {
            eprintln!("[MIDI_RECORDING] No active MIDI recording to stop");
        }
    }

    /// Get current sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Get number of channels
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// Get number of tracks
    pub fn track_count(&self) -> usize {
        self.project.track_count()
    }
}

/// Controller for the engine that can be used from the UI thread
pub struct EngineController {
    command_tx: rtrb::Producer<Command>,
    query_tx: rtrb::Producer<Query>,
    query_response_rx: rtrb::Consumer<QueryResponse>,
    playhead: Arc<AtomicU64>,
    next_midi_clip_id: Arc<AtomicU32>,
    sample_rate: u32,
    #[allow(dead_code)] // Used in public getter method
    channels: u32,
}

// Safety: EngineController is safe to Send across threads because:
// - rtrb::Producer<Command> is Send by design (lock-free queue for cross-thread communication)
// - Arc<AtomicU64> is Send + Sync (atomic types are inherently thread-safe)
// - u32 primitives are Send + Sync (Copy types)
// EngineController is only accessed through Mutex in application state, ensuring no concurrent mutable access.
unsafe impl Send for EngineController {}

impl EngineController {
    /// Start or resume playback
    pub fn play(&mut self) {
        let _ = self.command_tx.push(Command::Play);
    }

    /// Pause playback
    pub fn pause(&mut self) {
        let _ = self.command_tx.push(Command::Pause);
    }

    /// Stop playback and reset to beginning
    pub fn stop(&mut self) {
        let _ = self.command_tx.push(Command::Stop);
    }

    /// Seek to a specific position in seconds
    pub fn seek(&mut self, seconds: f64) {
        let _ = self.command_tx.push(Command::Seek(seconds));
    }

    /// Set track volume (0.0 = silence, 1.0 = unity gain)
    pub fn set_track_volume(&mut self, track_id: TrackId, volume: f32) {
        let _ = self
            .command_tx
            .push(Command::SetTrackVolume(track_id, volume));
    }

    /// Set track mute state
    pub fn set_track_mute(&mut self, track_id: TrackId, muted: bool) {
        let _ = self.command_tx.push(Command::SetTrackMute(track_id, muted));
    }

    /// Set track solo state
    pub fn set_track_solo(&mut self, track_id: TrackId, solo: bool) {
        let _ = self.command_tx.push(Command::SetTrackSolo(track_id, solo));
    }

    /// Move a clip to a new timeline position
    pub fn move_clip(&mut self, track_id: TrackId, clip_id: ClipId, new_start_time: f64) {
        let _ = self.command_tx.push(Command::MoveClip(track_id, clip_id, new_start_time));
    }

    /// Send a generic command to the audio thread
    pub fn send_command(&mut self, command: Command) {
        let _ = self.command_tx.push(command);
    }

    /// Get current playhead position in samples
    pub fn get_playhead_samples(&self) -> u64 {
        self.playhead.load(Ordering::Relaxed)
    }

    /// Get current playhead position in seconds
    pub fn get_playhead_seconds(&self) -> f64 {
        let frames = self.playhead.load(Ordering::Relaxed);
        frames as f64 / self.sample_rate as f64
    }

    /// Create a new metatrack
    pub fn create_metatrack(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateMetatrack(name));
    }

    /// Add a track to a metatrack
    pub fn add_to_metatrack(&mut self, track_id: TrackId, metatrack_id: TrackId) {
        let _ = self.command_tx.push(Command::AddToMetatrack(track_id, metatrack_id));
    }

    /// Remove a track from its parent metatrack
    pub fn remove_from_metatrack(&mut self, track_id: TrackId) {
        let _ = self.command_tx.push(Command::RemoveFromMetatrack(track_id));
    }

    /// Set metatrack time stretch factor
    /// 0.5 = half speed, 1.0 = normal, 2.0 = double speed
    pub fn set_time_stretch(&mut self, track_id: TrackId, stretch: f32) {
        let _ = self.command_tx.push(Command::SetTimeStretch(track_id, stretch));
    }

    /// Set metatrack time offset in seconds
    /// Positive = shift content later, negative = shift earlier
    pub fn set_offset(&mut self, track_id: TrackId, offset: f64) {
        let _ = self.command_tx.push(Command::SetOffset(track_id, offset));
    }

    /// Set metatrack pitch shift in semitones (for future use)
    pub fn set_pitch_shift(&mut self, track_id: TrackId, semitones: f32) {
        let _ = self.command_tx.push(Command::SetPitchShift(track_id, semitones));
    }

    /// Create a new audio track
    pub fn create_audio_track(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateAudioTrack(name));
    }

    /// Add an audio file to the pool (must be called from non-audio thread with pre-loaded data)
    pub fn add_audio_file(&mut self, path: String, data: Vec<f32>, channels: u32, sample_rate: u32) {
        let _ = self.command_tx.push(Command::AddAudioFile(path, data, channels, sample_rate));
    }

    /// Add a clip to an audio track
    pub fn add_audio_clip(&mut self, track_id: TrackId, pool_index: usize, start_time: f64, duration: f64, offset: f64) {
        let _ = self.command_tx.push(Command::AddAudioClip(track_id, pool_index, start_time, duration, offset));
    }

    /// Create a new MIDI track
    pub fn create_midi_track(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateMidiTrack(name));
    }

    /// Create a new audio track synchronously (waits for creation to complete)
    pub fn create_audio_track_sync(&mut self, name: String) -> Result<TrackId, String> {
        if let Err(_) = self.query_tx.push(Query::CreateAudioTrackSync(name)) {
            return Err("Failed to send track creation query".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::TrackCreated(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Track creation timeout".to_string())
    }

    /// Create a new MIDI track synchronously (waits for creation to complete)
    pub fn create_midi_track_sync(&mut self, name: String) -> Result<TrackId, String> {
        if let Err(_) = self.query_tx.push(Query::CreateMidiTrackSync(name)) {
            return Err("Failed to send track creation query".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::TrackCreated(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Track creation timeout".to_string())
    }

    /// Create a new MIDI clip on a track
    pub fn create_midi_clip(&mut self, track_id: TrackId, start_time: f64, duration: f64) -> MidiClipId {
        // Peek at the next clip ID that will be used
        let clip_id = self.next_midi_clip_id.load(Ordering::Relaxed);
        let _ = self.command_tx.push(Command::CreateMidiClip(track_id, start_time, duration));
        clip_id
    }

    /// Add a MIDI note to a clip
    pub fn add_midi_note(&mut self, track_id: TrackId, clip_id: MidiClipId, time_offset: f64, note: u8, velocity: u8, duration: f64) {
        let _ = self.command_tx.push(Command::AddMidiNote(track_id, clip_id, time_offset, note, velocity, duration));
    }

    /// Add a pre-loaded MIDI clip to a track
    pub fn add_loaded_midi_clip(&mut self, track_id: TrackId, clip: MidiClip) {
        let _ = self.command_tx.push(Command::AddLoadedMidiClip(track_id, clip));
    }

    /// Update all notes in a MIDI clip
    pub fn update_midi_clip_notes(&mut self, track_id: TrackId, clip_id: MidiClipId, notes: Vec<(f64, u8, u8, f64)>) {
        let _ = self.command_tx.push(Command::UpdateMidiClipNotes(track_id, clip_id, notes));
    }

    /// Request buffer pool statistics
    /// The statistics will be sent via an AudioEvent::BufferPoolStats event
    pub fn request_buffer_pool_stats(&mut self) {
        let _ = self.command_tx.push(Command::RequestBufferPoolStats);
    }

    /// Create a new automation lane on a track
    /// Returns an event AutomationLaneCreated with the lane ID
    pub fn create_automation_lane(&mut self, track_id: TrackId, parameter_id: crate::audio::ParameterId) {
        let _ = self.command_tx.push(Command::CreateAutomationLane(track_id, parameter_id));
    }

    /// Add an automation point to a lane
    pub fn add_automation_point(
        &mut self,
        track_id: TrackId,
        lane_id: crate::audio::AutomationLaneId,
        time: f64,
        value: f32,
        curve: crate::audio::CurveType,
    ) {
        let _ = self.command_tx.push(Command::AddAutomationPoint(
            track_id, lane_id, time, value, curve,
        ));
    }

    /// Remove an automation point at a specific time
    pub fn remove_automation_point(
        &mut self,
        track_id: TrackId,
        lane_id: crate::audio::AutomationLaneId,
        time: f64,
        tolerance: f64,
    ) {
        let _ = self.command_tx.push(Command::RemoveAutomationPoint(
            track_id, lane_id, time, tolerance,
        ));
    }

    /// Clear all automation points from a lane
    pub fn clear_automation_lane(
        &mut self,
        track_id: TrackId,
        lane_id: crate::audio::AutomationLaneId,
    ) {
        let _ = self.command_tx.push(Command::ClearAutomationLane(track_id, lane_id));
    }

    /// Remove an automation lane entirely
    pub fn remove_automation_lane(
        &mut self,
        track_id: TrackId,
        lane_id: crate::audio::AutomationLaneId,
    ) {
        let _ = self.command_tx.push(Command::RemoveAutomationLane(track_id, lane_id));
    }

    /// Enable or disable an automation lane
    pub fn set_automation_lane_enabled(
        &mut self,
        track_id: TrackId,
        lane_id: crate::audio::AutomationLaneId,
        enabled: bool,
    ) {
        let _ = self.command_tx.push(Command::SetAutomationLaneEnabled(
            track_id, lane_id, enabled,
        ));
    }

    /// Start recording on a track
    pub fn start_recording(&mut self, track_id: TrackId, start_time: f64) {
        let _ = self.command_tx.push(Command::StartRecording(track_id, start_time));
    }

    /// Stop the current recording
    pub fn stop_recording(&mut self) {
        let _ = self.command_tx.push(Command::StopRecording);
    }

    /// Pause the current recording
    pub fn pause_recording(&mut self) {
        let _ = self.command_tx.push(Command::PauseRecording);
    }

    /// Resume the current recording
    pub fn resume_recording(&mut self) {
        let _ = self.command_tx.push(Command::ResumeRecording);
    }

    /// Start MIDI recording on a track
    pub fn start_midi_recording(&mut self, track_id: TrackId, clip_id: MidiClipId, start_time: f64) {
        let _ = self.command_tx.push(Command::StartMidiRecording(track_id, clip_id, start_time));
    }

    /// Stop the current MIDI recording
    pub fn stop_midi_recording(&mut self) {
        let _ = self.command_tx.push(Command::StopMidiRecording);
    }

    /// Reset the entire project (clear all tracks, audio pool, and state)
    pub fn reset(&mut self) {
        let _ = self.command_tx.push(Command::Reset);
    }

    /// Send a live MIDI note on event to a track's instrument
    pub fn send_midi_note_on(&mut self, track_id: TrackId, note: u8, velocity: u8) {
        let _ = self.command_tx.push(Command::SendMidiNoteOn(track_id, note, velocity));
    }

    /// Send a live MIDI note off event to a track's instrument
    pub fn send_midi_note_off(&mut self, track_id: TrackId, note: u8) {
        let _ = self.command_tx.push(Command::SendMidiNoteOff(track_id, note));
    }

    /// Set the active MIDI track for external MIDI input routing
    pub fn set_active_midi_track(&mut self, track_id: Option<TrackId>) {
        let _ = self.command_tx.push(Command::SetActiveMidiTrack(track_id));
    }

    // Node graph operations

    /// Add a node to a track's instrument graph
    pub fn graph_add_node(&mut self, track_id: TrackId, node_type: String, x: f32, y: f32) {
        let _ = self.command_tx.push(Command::GraphAddNode(track_id, node_type, x, y));
    }

    pub fn graph_add_node_to_template(&mut self, track_id: TrackId, voice_allocator_id: u32, node_type: String, x: f32, y: f32) {
        let _ = self.command_tx.push(Command::GraphAddNodeToTemplate(track_id, voice_allocator_id, node_type, x, y));
    }

    pub fn graph_connect_in_template(&mut self, track_id: TrackId, voice_allocator_id: u32, from_node: u32, from_port: usize, to_node: u32, to_port: usize) {
        let _ = self.command_tx.push(Command::GraphConnectInTemplate(track_id, voice_allocator_id, from_node, from_port, to_node, to_port));
    }

    /// Remove a node from a track's instrument graph
    pub fn graph_remove_node(&mut self, track_id: TrackId, node_id: u32) {
        let _ = self.command_tx.push(Command::GraphRemoveNode(track_id, node_id));
    }

    /// Connect two nodes in a track's instrument graph
    pub fn graph_connect(&mut self, track_id: TrackId, from_node: u32, from_port: usize, to_node: u32, to_port: usize) {
        let _ = self.command_tx.push(Command::GraphConnect(track_id, from_node, from_port, to_node, to_port));
    }

    /// Disconnect two nodes in a track's instrument graph
    pub fn graph_disconnect(&mut self, track_id: TrackId, from_node: u32, from_port: usize, to_node: u32, to_port: usize) {
        let _ = self.command_tx.push(Command::GraphDisconnect(track_id, from_node, from_port, to_node, to_port));
    }

    /// Set a parameter on a node in a track's instrument graph
    pub fn graph_set_parameter(&mut self, track_id: TrackId, node_id: u32, param_id: u32, value: f32) {
        let _ = self.command_tx.push(Command::GraphSetParameter(track_id, node_id, param_id, value));
    }

    /// Set which node receives MIDI events in a track's instrument graph
    pub fn graph_set_midi_target(&mut self, track_id: TrackId, node_id: u32, enabled: bool) {
        let _ = self.command_tx.push(Command::GraphSetMidiTarget(track_id, node_id, enabled));
    }

    /// Set which node is the audio output in a track's instrument graph
    pub fn graph_set_output_node(&mut self, track_id: TrackId, node_id: u32) {
        let _ = self.command_tx.push(Command::GraphSetOutputNode(track_id, node_id));
    }

    /// Save the current graph as a preset
    pub fn graph_save_preset(&mut self, track_id: TrackId, preset_path: String, preset_name: String, description: String, tags: Vec<String>) {
        let _ = self.command_tx.push(Command::GraphSavePreset(track_id, preset_path, preset_name, description, tags));
    }

    /// Load a preset into a track's graph
    pub fn graph_load_preset(&mut self, track_id: TrackId, preset_path: String) {
        let _ = self.command_tx.push(Command::GraphLoadPreset(track_id, preset_path));
    }

    /// Save a VoiceAllocator's template graph as a preset
    pub fn graph_save_template_preset(&mut self, track_id: TrackId, voice_allocator_id: u32, preset_path: String, preset_name: String) {
        let _ = self.command_tx.push(Command::GraphSaveTemplatePreset(track_id, voice_allocator_id, preset_path, preset_name));
    }

    /// Load a sample into a SimpleSampler node
    pub fn sampler_load_sample(&mut self, track_id: TrackId, node_id: u32, file_path: String) {
        let _ = self.command_tx.push(Command::SamplerLoadSample(track_id, node_id, file_path));
    }

    /// Add a sample layer to a MultiSampler node
    pub fn multi_sampler_add_layer(&mut self, track_id: TrackId, node_id: u32, file_path: String, key_min: u8, key_max: u8, root_key: u8, velocity_min: u8, velocity_max: u8) {
        let _ = self.command_tx.push(Command::MultiSamplerAddLayer(track_id, node_id, file_path, key_min, key_max, root_key, velocity_min, velocity_max));
    }

    /// Update a MultiSampler layer's configuration
    pub fn multi_sampler_update_layer(&mut self, track_id: TrackId, node_id: u32, layer_index: usize, key_min: u8, key_max: u8, root_key: u8, velocity_min: u8, velocity_max: u8) {
        let _ = self.command_tx.push(Command::MultiSamplerUpdateLayer(track_id, node_id, layer_index, key_min, key_max, root_key, velocity_min, velocity_max));
    }

    /// Remove a layer from a MultiSampler node
    pub fn multi_sampler_remove_layer(&mut self, track_id: TrackId, node_id: u32, layer_index: usize) {
        let _ = self.command_tx.push(Command::MultiSamplerRemoveLayer(track_id, node_id, layer_index));
    }

    /// Send a synchronous query and wait for the response
    /// This blocks until the audio thread processes the query
    pub fn query_graph_state(&mut self, track_id: TrackId) -> Result<String, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetGraphState(track_id)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::GraphState(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(100));
        }

        Err("Query timeout".to_string())
    }

    /// Query a template graph state
    pub fn query_template_state(&mut self, track_id: TrackId, voice_allocator_id: u32) -> Result<String, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetTemplateState(track_id, voice_allocator_id)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::GraphState(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(100));
        }

        Err("Query timeout".to_string())
    }

    /// Query MIDI clip data
    pub fn query_midi_clip(&mut self, track_id: TrackId, clip_id: MidiClipId) -> Result<crate::command::MidiClipData, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetMidiClip(track_id, clip_id)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::MidiClipData(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(100));
        }

        Err("Query timeout".to_string())
    }

    /// Query oscilloscope data from a node
    pub fn query_oscilloscope_data(&mut self, track_id: TrackId, node_id: u32, sample_count: usize) -> Result<crate::command::OscilloscopeData, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetOscilloscopeData(track_id, node_id, sample_count)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(100);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::OscilloscopeData(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(50));
        }

        Err("Query timeout".to_string())
    }

    /// Query automation keyframes from an AutomationInput node
    pub fn query_automation_keyframes(&mut self, track_id: TrackId, node_id: u32) -> Result<Vec<crate::command::types::AutomationKeyframeData>, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetAutomationKeyframes(track_id, node_id)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(100);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::AutomationKeyframes(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(50));
        }

        Err("Query timeout".to_string())
    }

    /// Query automation node display name
    pub fn query_automation_name(&mut self, track_id: TrackId, node_id: u32) -> Result<String, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetAutomationName(track_id, node_id)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(100);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::AutomationName(result)) = self.query_response_rx.pop() {
                return result;
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(50));
        }

        Err("Query timeout".to_string())
    }

    /// Serialize the audio pool for project saving
    pub fn serialize_audio_pool(&mut self, project_path: &std::path::Path) -> Result<Vec<crate::audio::pool::AudioPoolEntry>, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::SerializeAudioPool(project_path.to_path_buf())) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5); // Longer timeout for file operations

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::AudioPoolSerialized(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }

    /// Get waveform for a pool index
    pub fn get_pool_waveform(&mut self, pool_index: usize, target_peaks: usize) -> Result<Vec<crate::io::WaveformPeak>, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetPoolWaveform(pool_index, target_peaks)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::PoolWaveform(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Query timeout".to_string())
    }

    /// Get file info from pool (duration, sample_rate, channels)
    pub fn get_pool_file_info(&mut self, pool_index: usize) -> Result<(f64, u32, u32), String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetPoolFileInfo(pool_index)) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::PoolFileInfo(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Query timeout".to_string())
    }

    /// Load audio pool from serialized entries
    pub fn load_audio_pool(&mut self, entries: Vec<crate::audio::pool::AudioPoolEntry>, project_path: &std::path::Path) -> Result<Vec<usize>, String> {
        // Send command via query mechanism
        if let Err(_) = self.query_tx.push(Query::LoadAudioPool(entries, project_path.to_path_buf())) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10); // Long timeout for loading multiple files

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::AudioPoolLoaded(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }

    /// Resolve a missing audio file by loading from a new path
    pub fn resolve_missing_audio_file(&mut self, pool_index: usize, new_path: &std::path::Path) -> Result<(), String> {
        // Send command via query mechanism
        if let Err(_) = self.query_tx.push(Query::ResolveMissingAudioFile(pool_index, new_path.to_path_buf())) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::AudioFileResolved(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }

    /// Serialize a track's effects/instrument graph to JSON
    pub fn serialize_track_graph(&mut self, track_id: TrackId, project_path: &std::path::Path) -> Result<String, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::SerializeTrackGraph(track_id, project_path.to_path_buf())) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::TrackGraphSerialized(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }

    /// Load a track's effects/instrument graph from JSON
    pub fn load_track_graph(&mut self, track_id: TrackId, preset_json: &str, project_path: &std::path::Path) -> Result<(), String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::LoadTrackGraph(track_id, preset_json.to_string(), project_path.to_path_buf())) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10); // Longer timeout for loading presets

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::TrackGraphLoaded(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }
}
