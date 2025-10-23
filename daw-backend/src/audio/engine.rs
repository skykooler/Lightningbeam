use crate::audio::buffer_pool::BufferPool;
use crate::audio::clip::ClipId;
use crate::audio::midi::{MidiClip, MidiClipId, MidiEvent};
use crate::audio::pool::AudioPool;
use crate::audio::project::Project;
use crate::audio::recording::RecordingState;
use crate::audio::track::{Track, TrackId};
use crate::command::{AudioEvent, Command};
use crate::effects::{Effect, GainEffect, PanEffect, SimpleEQ};
use std::sync::atomic::{AtomicU64, Ordering};
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
    event_tx: rtrb::Producer<AudioEvent>,

    // Shared playhead for UI reads
    playhead_atomic: Arc<AtomicU64>,

    // Event counter for periodic position updates
    frames_since_last_event: usize,
    event_interval_frames: usize,

    // Mix buffer for output
    mix_buffer: Vec<f32>,

    // ID counters
    next_midi_clip_id: MidiClipId,
    next_clip_id: ClipId,

    // Recording state
    recording_state: Option<RecordingState>,
    input_rx: Option<rtrb::Consumer<f32>>,
    recording_progress_counter: usize,
}

impl Engine {
    /// Create a new Engine with communication channels
    pub fn new(
        sample_rate: u32,
        channels: u32,
        command_rx: rtrb::Consumer<Command>,
        event_tx: rtrb::Producer<AudioEvent>,
    ) -> Self {
        let event_interval_frames = (sample_rate as usize * channels as usize) / 10; // Update 10 times per second

        // Calculate a reasonable buffer size for the pool (typical audio callback size * channels)
        let buffer_size = 512 * channels as usize;

        Self {
            project: Project::new(),
            audio_pool: AudioPool::new(),
            buffer_pool: BufferPool::new(8, buffer_size), // 8 buffers should handle deep nesting
            playhead: 0,
            sample_rate,
            playing: false,
            channels,
            command_rx,
            event_tx,
            playhead_atomic: Arc::new(AtomicU64::new(0)),
            frames_since_last_event: 0,
            event_interval_frames,
            mix_buffer: Vec::new(),
            next_midi_clip_id: 0,
            next_clip_id: 0,
            recording_state: None,
            input_rx: None,
            recording_progress_counter: 0,
        }
    }

    /// Set the input ringbuffer consumer for recording
    pub fn set_input_rx(&mut self, input_rx: rtrb::Consumer<f32>) {
        self.input_rx = Some(input_rx);
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
                audio_track.effects = track.effects;
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
    pub fn get_controller(&self, command_tx: rtrb::Producer<Command>) -> EngineController {
        EngineController {
            command_tx,
            playhead: Arc::clone(&self.playhead_atomic),
            sample_rate: self.sample_rate,
            channels: self.channels,
        }
    }

    /// Process audio callback - called from the audio thread
    pub fn process(&mut self, output: &mut [f32]) {
        // Process all pending commands
        while let Ok(cmd) = self.command_rx.pop() {
            self.handle_command(cmd);
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

            // Convert playhead from samples to seconds for timeline-based rendering
            let playhead_seconds = self.playhead as f64 / (self.sample_rate as f64 * self.channels as f64);

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

            // Update playhead
            self.playhead += output.len() as u64;

            // Update atomic playhead for UI reads
            self.playhead_atomic
                .store(self.playhead, Ordering::Relaxed);

            // Send periodic position updates
            self.frames_since_last_event += output.len() / self.channels as usize;
            if self.frames_since_last_event >= self.event_interval_frames / self.channels as usize
            {
                let position_seconds =
                    self.playhead as f64 / (self.sample_rate as f64 * self.channels as f64);
                let _ = self
                    .event_tx
                    .push(AudioEvent::PlaybackPosition(position_seconds));
                self.frames_since_last_event = 0;
            }
        } else {
            // Not playing, output silence
            output.fill(0.0);
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
            }
            Command::Pause => {
                self.playing = false;
            }
            Command::Seek(seconds) => {
                let samples = (seconds * self.sample_rate as f64 * self.channels as f64) as u64;
                self.playhead = samples;
                self.playhead_atomic
                    .store(self.playhead, Ordering::Relaxed);
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
                if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_time = new_start_time;
                    }
                }
            }
            Command::AddGainEffect(track_id, gain_db) => {
                // Get the track node and handle audio tracks, MIDI tracks, and groups
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "Gain") {
                            effect.set_parameter(0, gain_db);
                        } else {
                            track.add_effect(Box::new(GainEffect::with_gain_db(gain_db)));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "Gain") {
                            effect.set_parameter(0, gain_db);
                        } else {
                            track.add_effect(Box::new(GainEffect::with_gain_db(gain_db)));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(effect) = group.effects.iter_mut().find(|e| e.name() == "Gain") {
                            effect.set_parameter(0, gain_db);
                        } else {
                            group.add_effect(Box::new(GainEffect::with_gain_db(gain_db)));
                        }
                    }
                    None => {}
                }
            }
            Command::AddPanEffect(track_id, pan) => {
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "Pan") {
                            effect.set_parameter(0, pan);
                        } else {
                            track.add_effect(Box::new(PanEffect::with_pan(pan)));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "Pan") {
                            effect.set_parameter(0, pan);
                        } else {
                            track.add_effect(Box::new(PanEffect::with_pan(pan)));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(effect) = group.effects.iter_mut().find(|e| e.name() == "Pan") {
                            effect.set_parameter(0, pan);
                        } else {
                            group.add_effect(Box::new(PanEffect::with_pan(pan)));
                        }
                    }
                    None => {}
                }
            }
            Command::AddEQEffect(track_id, low_db, mid_db, high_db) => {
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "SimpleEQ") {
                            effect.set_parameter(0, low_db);
                            effect.set_parameter(1, mid_db);
                            effect.set_parameter(2, high_db);
                        } else {
                            let mut eq = SimpleEQ::new();
                            eq.set_parameter(0, low_db);
                            eq.set_parameter(1, mid_db);
                            eq.set_parameter(2, high_db);
                            track.add_effect(Box::new(eq));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        if let Some(effect) = track.effects.iter_mut().find(|e| e.name() == "SimpleEQ") {
                            effect.set_parameter(0, low_db);
                            effect.set_parameter(1, mid_db);
                            effect.set_parameter(2, high_db);
                        } else {
                            let mut eq = SimpleEQ::new();
                            eq.set_parameter(0, low_db);
                            eq.set_parameter(1, mid_db);
                            eq.set_parameter(2, high_db);
                            track.add_effect(Box::new(eq));
                        }
                    }
                    Some(crate::audio::track::TrackNode::Group(group)) => {
                        if let Some(effect) = group.effects.iter_mut().find(|e| e.name() == "SimpleEQ") {
                            effect.set_parameter(0, low_db);
                            effect.set_parameter(1, mid_db);
                            effect.set_parameter(2, high_db);
                        } else {
                            let mut eq = SimpleEQ::new();
                            eq.set_parameter(0, low_db);
                            eq.set_parameter(1, mid_db);
                            eq.set_parameter(2, high_db);
                            group.add_effect(Box::new(eq));
                        }
                    }
                    None => {}
                }
            }
            Command::ClearEffects(track_id) => {
                let _ = self.project.clear_effects(track_id);
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
                    // Notify UI about the new clip
                    let _ = self.event_tx.push(AudioEvent::ClipAdded(track_id, clip_id));
                }
            }
            Command::CreateMidiTrack(name) => {
                let track_id = self.project.add_midi_track(name.clone(), None);
                // Notify UI about the new MIDI track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
            }
            Command::CreateMidiClip(track_id, start_time, duration) => {
                // Create a new MIDI clip with unique ID
                let clip_id = self.next_midi_clip_id;
                self.next_midi_clip_id += 1;
                let clip = MidiClip::new(clip_id, start_time, duration);
                let _ = self.project.add_midi_clip(track_id, clip);
            }
            Command::AddMidiNote(track_id, clip_id, time_offset, note, velocity, duration) => {
                // Add a MIDI note event to the specified clip
                if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                        // Convert time to sample timestamp
                        let timestamp = (time_offset * self.sample_rate as f64) as u64;
                        let note_on = MidiEvent::note_on(timestamp, 0, note, velocity);
                        clip.events.push(note_on);

                        // Add note off event
                        let note_off_timestamp = ((time_offset + duration) * self.sample_rate as f64) as u64;
                        let note_off = MidiEvent::note_off(note_off_timestamp, 0, note, 64);
                        clip.events.push(note_off);

                        // Sort events by timestamp
                        clip.events.sort_by_key(|e| e.timestamp);
                    }
                }
            }
            Command::AddLoadedMidiClip(track_id, clip) => {
                // Add a pre-loaded MIDI clip to the track
                let _ = self.project.add_midi_clip(track_id, clip);
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
            Command::Reset => {
                // Reset the entire project to initial state
                // Stop playback
                self.playing = false;
                self.playhead = 0;
                self.playhead_atomic.store(0, Ordering::Relaxed);

                // Stop any active recording
                self.recording_state = None;

                // Clear all project data
                self.project = Project::new();

                // Clear audio pool
                self.audio_pool = AudioPool::new();

                // Reset buffer pool (recreate with same settings)
                let buffer_size = 512 * self.channels as usize;
                self.buffer_pool = BufferPool::new(8, buffer_size);

                // Reset ID counters
                self.next_midi_clip_id = 0;
                self.next_clip_id = 0;

                // Clear mix buffer
                self.mix_buffer.clear();

                // Notify UI that reset is complete
                let _ = self.event_tx.push(AudioEvent::ProjectReset);
            }
        }
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
    playhead: Arc<AtomicU64>,
    sample_rate: u32,
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

    /// Add or update gain effect on track
    pub fn add_gain_effect(&mut self, track_id: TrackId, gain_db: f32) {
        let _ = self.command_tx.push(Command::AddGainEffect(track_id, gain_db));
    }

    /// Add or update pan effect on track
    pub fn add_pan_effect(&mut self, track_id: TrackId, pan: f32) {
        let _ = self.command_tx.push(Command::AddPanEffect(track_id, pan));
    }

    /// Add or update EQ effect on track
    pub fn add_eq_effect(&mut self, track_id: TrackId, low_db: f32, mid_db: f32, high_db: f32) {
        let _ = self.command_tx.push(Command::AddEQEffect(track_id, low_db, mid_db, high_db));
    }

    /// Clear all effects from a track
    pub fn clear_effects(&mut self, track_id: TrackId) {
        let _ = self.command_tx.push(Command::ClearEffects(track_id));
    }

    /// Get current playhead position in samples
    pub fn get_playhead_samples(&self) -> u64 {
        self.playhead.load(Ordering::Relaxed)
    }

    /// Get current playhead position in seconds
    pub fn get_playhead_seconds(&self) -> f64 {
        let samples = self.playhead.load(Ordering::Relaxed);
        samples as f64 / (self.sample_rate as f64 * self.channels as f64)
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

    /// Create a new MIDI clip on a track
    pub fn create_midi_clip(&mut self, track_id: TrackId, start_time: f64, duration: f64) {
        let _ = self.command_tx.push(Command::CreateMidiClip(track_id, start_time, duration));
    }

    /// Add a MIDI note to a clip
    pub fn add_midi_note(&mut self, track_id: TrackId, clip_id: MidiClipId, time_offset: f64, note: u8, velocity: u8, duration: f64) {
        let _ = self.command_tx.push(Command::AddMidiNote(track_id, clip_id, time_offset, note, velocity, duration));
    }

    /// Add a pre-loaded MIDI clip to a track
    pub fn add_loaded_midi_clip(&mut self, track_id: TrackId, clip: MidiClip) {
        let _ = self.command_tx.push(Command::AddLoadedMidiClip(track_id, clip));
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

    /// Reset the entire project (clear all tracks, audio pool, and state)
    pub fn reset(&mut self) {
        let _ = self.command_tx.push(Command::Reset);
    }
}
