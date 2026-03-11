use crate::audio::buffer_pool::BufferPool;
use crate::audio::clip::{AudioClipInstance, AudioClipInstanceId, ClipId};
use crate::audio::metronome::Metronome;
use crate::audio::midi::{MidiClip, MidiClipId, MidiClipInstance, MidiClipInstanceId, MidiEvent};
use crate::audio::node_graph::{nodes::*, AudioGraph};
use crate::audio::pool::AudioClipPool;
use crate::audio::project::Project;
use crate::audio::recording::{MidiRecordingState, RecordingState};
use crate::audio::track::{Track, TrackId, TrackNode};
use crate::command::{AudioEvent, Command, Query, QueryResponse};
use crate::io::MidiInputManager;
use petgraph::stable_graph::NodeIndex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

/// Read-only snapshot of all clip instances, updated after every clip mutation.
/// Shared between the audio thread (writer) and the UI thread (reader).
#[derive(Default, Clone)]
pub struct AudioClipSnapshot {
    pub audio: HashMap<TrackId, Vec<AudioClipInstance>>,
    pub midi:  HashMap<TrackId, Vec<MidiClipInstance>>,
}

/// Audio engine for Phase 6: hierarchical tracks with groups
pub struct Engine {
    project: Project,
    audio_pool: AudioClipPool,
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

    // Background chunk generation channel
    chunk_generation_rx: std::sync::mpsc::Receiver<AudioEvent>,
    chunk_generation_tx: std::sync::mpsc::Sender<AudioEvent>,

    // Shared clip snapshot for UI reads
    clip_snapshot: Arc<RwLock<AudioClipSnapshot>>,

    // Shared playhead for UI reads
    playhead_atomic: Arc<AtomicU64>,

    // Shared MIDI clip ID counter for synchronous access
    next_midi_clip_id_atomic: Arc<AtomicU32>,

    // Shared audio clip ID counter (shared with EngineController for pre-assigned IDs)
    next_audio_clip_id_atomic: Arc<AtomicU32>,

    // Event counter for periodic position updates
    frames_since_last_event: usize,
    event_interval_frames: usize,

    // Mix buffer for output
    mix_buffer: Vec<f32>,

    // ID counters (legacy, unused — kept for potential future use)
    // Audio clip IDs are now generated via next_audio_clip_id_atomic

    // Recording state
    recording_state: Option<RecordingState>,
    input_rx: Option<rtrb::Consumer<f32>>,
    recording_mirror_tx: Option<rtrb::Producer<f32>>,
    recording_progress_counter: usize,

    // MIDI recording state
    midi_recording_state: Option<MidiRecordingState>,

    // MIDI input manager for external MIDI devices
    midi_input_manager: Option<MidiInputManager>,

    // Metronome for click track
    metronome: Metronome,

    // Pre-allocated buffer for recording input samples (avoids allocation per callback)
    recording_sample_buffer: Vec<f32>,

    // Disk reader for streaming playback of compressed files
    disk_reader: Option<crate::audio::disk_reader::DiskReader>,

    // Input monitoring and metering
    input_monitoring: bool,
    input_gain: f32,
    input_level_peak: f32,
    input_level_counter: usize,
    output_level_peak_l: f32,
    output_level_peak_r: f32,
    output_level_counter: usize,
    track_level_counter: usize,

    // Callback timing diagnostics (enabled by DAW_AUDIO_DEBUG=1)
    debug_audio: bool,
    callback_count: u64,
    timing_worst_total_us: u64,
    timing_worst_commands_us: u64,
    timing_worst_render_us: u64,
    timing_sum_total_us: u64,
    timing_overrun_count: u64,
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

        // Create channel for background chunk generation
        let (chunk_generation_tx, chunk_generation_rx) = std::sync::mpsc::channel();

        // Shared atomic playhead for UI reads and disk reader
        let playhead_atomic = Arc::new(AtomicU64::new(0));

        // Initialize disk reader with shared playhead
        let disk_reader = crate::audio::disk_reader::DiskReader::new(
            Arc::clone(&playhead_atomic),
            sample_rate,
        );

        Self {
            project: Project::new(sample_rate),
            audio_pool: AudioClipPool::new(),
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
            chunk_generation_rx,
            chunk_generation_tx,
            clip_snapshot: Arc::new(RwLock::new(AudioClipSnapshot::default())),
            playhead_atomic,
            next_midi_clip_id_atomic: Arc::new(AtomicU32::new(0)),
            next_audio_clip_id_atomic: Arc::new(AtomicU32::new(0)),
            frames_since_last_event: 0,
            event_interval_frames,
            mix_buffer: Vec::new(),
            recording_state: None,
            input_rx: None,
            recording_mirror_tx: None,
            recording_progress_counter: 0,
            midi_recording_state: None,
            midi_input_manager: None,
            metronome: Metronome::new(sample_rate),
            recording_sample_buffer: Vec::with_capacity(4096),
            disk_reader: Some(disk_reader),
            input_monitoring: false,
            input_gain: 1.0,
            input_level_peak: 0.0,
            input_level_counter: 0,
            output_level_peak_l: 0.0,
            output_level_peak_r: 0.0,
            output_level_counter: 0,
            track_level_counter: 0,
            debug_audio: std::env::var("DAW_AUDIO_DEBUG").map_or(false, |v| v == "1"),
            callback_count: 0,
            timing_worst_total_us: 0,
            timing_worst_commands_us: 0,
            timing_worst_render_us: 0,
            timing_sum_total_us: 0,
            timing_overrun_count: 0,
        }
    }

    /// Set the input ringbuffer consumer for recording
    pub fn set_input_rx(&mut self, input_rx: rtrb::Consumer<f32>) {
        self.input_rx = Some(input_rx);
    }

    /// Set the recording mirror producer for streaming audio to UI during recording
    pub fn set_recording_mirror_tx(&mut self, tx: rtrb::Producer<f32>) {
        self.recording_mirror_tx = Some(tx);
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
    pub fn audio_pool_mut(&mut self) -> &mut AudioClipPool {
        &mut self.audio_pool
    }

    /// Get reference to audio pool
    pub fn audio_pool(&self) -> &AudioClipPool {
        &self.audio_pool
    }

    /// Rebuild the clip snapshot from the current project state.
    /// Call this after any command that adds, removes, or modifies clip instances.
    fn refresh_clip_snapshot(&self) {
        let mut snap = self.clip_snapshot.write().unwrap();
        snap.audio.clear();
        snap.midi.clear();
        for (track_id, node) in self.project.track_iter() {
            match node {
                crate::audio::track::TrackNode::Audio(t) => {
                    snap.audio.insert(track_id, t.clips.clone());
                }
                crate::audio::track::TrackNode::Midi(t) => {
                    snap.midi.insert(track_id, t.clip_instances.clone());
                }
                crate::audio::track::TrackNode::Group(_) => {}
            }
        }
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
            next_audio_clip_id: Arc::clone(&self.next_audio_clip_id_atomic),
            clip_snapshot: Arc::clone(&self.clip_snapshot),
            sample_rate: self.sample_rate,
            channels: self.channels,
            cached_export_response: None,
        }
    }

    /// Process audio callback - called from the audio thread
    pub fn process(&mut self, output: &mut [f32]) {
        let t_start = if self.debug_audio { Some(std::time::Instant::now()) } else { None };

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

        // Forward chunk generation events from background threads
        while let Ok(event) = self.chunk_generation_rx.try_recv() {
            match event {
                AudioEvent::WaveformDecodeComplete { pool_index, samples, decoded_frames: _df, total_frames: _tf } => {
                    // Forward samples directly to UI — no clone, just move
                    if let Some(file) = self.audio_pool.get_file(pool_index) {
                        let sr = file.sample_rate;
                        let ch = file.channels;
                        let _ = self.event_tx.push(AudioEvent::AudioDecodeProgress {
                            pool_index,
                            samples,
                            sample_rate: sr,
                            channels: ch,
                        });
                    }
                }
                other => {
                    if self.debug_audio {
                        if let AudioEvent::WaveformChunksReady { pool_index, detail_level, ref chunks } = other {
                            eprintln!("[AUDIO THREAD] Received {} chunks for pool {} level {}, forwarding to UI", chunks.len(), pool_index, detail_level);
                        }
                    }
                    let _ = self.event_tx.push(other);
                }
            }
        }

        let t_commands = if self.debug_audio { Some(std::time::Instant::now()) } else { None };

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

            // Reset per-clip read-ahead targets before rendering.
            self.project.reset_read_ahead_targets();

            // Render the entire project hierarchy into the mix buffer
            self.project.render(
                &mut self.mix_buffer,
                &self.audio_pool,
                &mut self.buffer_pool,
                playhead_seconds,
                self.sample_rate,
                self.channels,
                false,
            );

            // Copy mix to output
            output.copy_from_slice(&self.mix_buffer);

            // Mix in metronome clicks
            self.metronome.process(
                output,
                self.playhead,
                self.playing,
                self.sample_rate,
                self.channels,
            );

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
            // Not playing: render live MIDI (keyboard input + note-off tails) through the
            // normal group hierarchy so mixer gain is correctly applied.
            let playhead_seconds = self.playhead as f64 / self.sample_rate as f64;
            if self.mix_buffer.len() != output.len() {
                self.mix_buffer.resize(output.len(), 0.0);
            }
            if self.buffer_pool.buffer_size() != output.len() {
                self.buffer_pool = BufferPool::new(8, output.len());
            }
            self.project.render(
                &mut self.mix_buffer,
                &self.audio_pool,
                &mut self.buffer_pool,
                playhead_seconds,
                self.sample_rate,
                self.channels,
                true, // live_only
            );
            output.copy_from_slice(&self.mix_buffer);
        }

        // Compute stereo output peaks for master VU meter (independent of playback state)
        {
            let channels = self.channels as usize;
            for frame in output.chunks(channels) {
                if channels >= 2 {
                    self.output_level_peak_l = self.output_level_peak_l.max(frame[0].abs());
                    self.output_level_peak_r = self.output_level_peak_r.max(frame[1].abs());
                } else {
                    let v = frame[0].abs();
                    self.output_level_peak_l = self.output_level_peak_l.max(v);
                    self.output_level_peak_r = self.output_level_peak_r.max(v);
                }
            }
            self.output_level_counter += output.len();
            let meter_interval = self.sample_rate as usize / 20; // ~50ms
            if self.output_level_counter >= meter_interval {
                let _ = self.event_tx.push(AudioEvent::OutputLevel(self.output_level_peak_l, self.output_level_peak_r));
                self.output_level_peak_l = 0.0;
                self.output_level_peak_r = 0.0;
                self.output_level_counter = 0;
            }

            // Send per-track peak levels periodically
            self.track_level_counter += output.len();
            if self.track_level_counter >= meter_interval {
                let levels = self.project.collect_track_peaks();
                let _ = self.event_tx.push(AudioEvent::TrackLevels(levels));
                self.track_level_counter = 0;
            }
        }

        // Process input monitoring and/or recording (independent of playback state)
        let is_recording = self.recording_state.is_some();
        if is_recording || self.input_monitoring {
            if let Some(input_rx) = &mut self.input_rx {
                // Phase 1: Discard stale samples during recording skip phase
                if let Some(recording) = &mut self.recording_state {
                    while recording.samples_to_skip > 0 {
                        match input_rx.pop() {
                            Ok(_) => recording.samples_to_skip -= 1,
                            Err(_) => break,
                        }
                    }
                }

                // Phase 2: Pull fresh samples
                self.recording_sample_buffer.clear();
                while let Ok(sample) = input_rx.pop() {
                    // Apply input gain
                    self.recording_sample_buffer.push(sample * self.input_gain);
                }

                if !self.recording_sample_buffer.is_empty() {
                    // Compute input peak for VU metering
                    let input_peak = self.recording_sample_buffer.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
                    self.input_level_peak = self.input_level_peak.max(input_peak);
                    self.input_level_counter += self.recording_sample_buffer.len();
                    let meter_interval = self.sample_rate as usize / 20; // ~50ms
                    if self.input_level_counter >= meter_interval {
                        let _ = self.event_tx.push(AudioEvent::InputLevel(self.input_level_peak));
                        self.input_level_peak = 0.0;
                        self.input_level_counter = 0;
                    }

                    // Feed samples to recording if active
                    if let Some(recording) = &mut self.recording_state {
                        let skip = if recording.paused {
                            self.recording_sample_buffer.len()
                        } else {
                            recording.samples_to_skip.min(self.recording_sample_buffer.len())
                        };

                        match recording.add_samples(&self.recording_sample_buffer) {
                            Ok(_flushed) => {
                                // Mirror non-skipped samples to UI for live waveform display
                                if skip < self.recording_sample_buffer.len() {
                                    if let Some(ref mut mirror_tx) = self.recording_mirror_tx {
                                        for &sample in &self.recording_sample_buffer[skip..] {
                                            let _ = mirror_tx.push(sample);
                                        }
                                    }
                                }

                                // Update clip duration every callback for sample-accurate timing
                                let duration = recording.duration();
                                let clip_id = recording.clip_id;
                                let track_id = recording.track_id;

                                // Update clip duration in project as recording progresses
                                if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                                        clip.internal_end = clip.internal_start + duration;
                                        clip.external_duration = duration;
                                    }
                                }

                                // Send progress event periodically (every ~0.1 seconds)
                                self.recording_progress_counter += self.recording_sample_buffer.len();
                                if self.recording_progress_counter >= (self.sample_rate as usize / 10) {
                                    let _ = self.event_tx.push(AudioEvent::RecordingProgress(clip_id, duration));
                                    self.recording_progress_counter = 0;
                                }
                            }
                            Err(e) => {
                                let _ = self.event_tx.push(AudioEvent::RecordingError(
                                    format!("Recording write error: {}", e)
                                ));
                                self.recording_state = None;
                            }
                        }
                    }
                }
            }
        }

        // Timing diagnostics (DAW_AUDIO_DEBUG=1)
        if let (true, Some(t_start), Some(t_commands)) = (self.debug_audio, t_start, t_commands) {
            let t_end = std::time::Instant::now();
            let total_us = t_end.duration_since(t_start).as_micros() as u64;
            let commands_us = t_commands.duration_since(t_start).as_micros() as u64;
            let render_us = total_us.saturating_sub(commands_us);

            self.callback_count += 1;
            self.timing_sum_total_us += total_us;
            if total_us > self.timing_worst_total_us { self.timing_worst_total_us = total_us; }
            if commands_us > self.timing_worst_commands_us { self.timing_worst_commands_us = commands_us; }
            if render_us > self.timing_worst_render_us { self.timing_worst_render_us = render_us; }

            let frames = output.len() as u64 / self.channels as u64;
            let deadline_us = frames * 1_000_000 / self.sample_rate as u64;

            if total_us > deadline_us {
                self.timing_overrun_count += 1;
                eprintln!(
                    "[AUDIO TIMING] OVERRUN #{}: total={} us (deadline={} us) | cmds={} us, render={} us | buf={} frames",
                    self.timing_overrun_count, total_us, deadline_us, commands_us, render_us, frames
                );
            }

            if self.callback_count % 860 == 0 {
                let avg_us = self.timing_sum_total_us / self.callback_count;
                eprintln!(
                    "[AUDIO TIMING] avg={} us, worst: total={} us, cmds={} us, render={} us | overruns={}/{} ({:.1}%) | deadline={} us",
                    avg_us, self.timing_worst_total_us, self.timing_worst_commands_us, self.timing_worst_render_us,
                    self.timing_overrun_count, self.callback_count,
                    self.timing_overrun_count as f64 / self.callback_count as f64 * 100.0,
                    deadline_us
                );
            }
        }
    }

    /// Read audio from pool as mono f32 samples.
    /// Handles all storage types: InMemory/Mapped use read_samples(),
    /// Compressed falls back to decoding from the file path.
    fn read_mono_from_pool(pool: &crate::audio::pool::AudioClipPool, pool_index: usize) -> Option<(Vec<f32>, f32)> {
        let audio_file = pool.get_file(pool_index)?;
        let channels = audio_file.channels as usize;
        let frames = audio_file.frames as usize;
        let sample_rate = audio_file.sample_rate as f32;

        // Try read_samples first (works for InMemory and Mapped)
        let mut mono_samples = vec![0.0f32; frames];
        let read_count = if channels == 1 {
            audio_file.read_samples(0, frames, 0, &mut mono_samples)
        } else {
            let mut channel_buf = vec![0.0f32; frames];
            let mut count = 0;
            for ch in 0..channels {
                count = audio_file.read_samples(0, frames, ch, &mut channel_buf);
                for (i, &s) in channel_buf.iter().enumerate() {
                    mono_samples[i] += s;
                }
            }
            let scale = 1.0 / channels as f32;
            for s in &mut mono_samples {
                *s *= scale;
            }
            count
        };

        if read_count > 0 {
            return Some((mono_samples, sample_rate));
        }

        // Compressed storage: decode from file path using sample_loader
        let path = audio_file.path.to_string_lossy();
        if !path.starts_with("<embedded") {
            if let Ok(sample_data) = crate::audio::sample_loader::load_audio_file(&*path) {
                return Some((sample_data.samples, sample_data.sample_rate as f32));
            }
        }

        // Last resort: try interleaved data() and mix down
        let data = audio_file.data();
        if !data.is_empty() && channels > 0 {
            let actual_frames = data.len() / channels;
            let mut mono = vec![0.0f32; actual_frames];
            for frame in 0..actual_frames {
                let mut sum = 0.0f32;
                for ch in 0..channels {
                    sum += data[frame * channels + ch];
                }
                mono[frame] = sum / channels as f32;
            }
            return Some((mono, sample_rate));
        }

        eprintln!("[read_mono_from_pool] Failed to read audio from pool_index={}", pool_index);
        None
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
                // Reset disk reader buffers to the new playhead position
                if let Some(ref mut dr) = self.disk_reader {
                    dr.send(crate::audio::disk_reader::DiskReaderCommand::Seek { frame: 0 });
                }
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
                // Reset all node graphs to clear effect buffers (echo, reverb, etc.)
                self.project.reset_all_graphs();
                // Notify disk reader to refill buffers from new position
                if let Some(ref mut dr) = self.disk_reader {
                    dr.send(crate::audio::disk_reader::DiskReaderCommand::Seek { frame: frames });
                }
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
                // Moving just changes external_start, external_duration stays the same
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.external_start = new_start_time;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        // Note: clip_id here is the pool clip ID, not instance ID
                        if let Some(instance) = track.clip_instances.iter_mut().find(|c| c.clip_id == clip_id) {
                            instance.external_start = new_start_time;
                        }
                    }
                    _ => {}
                }
                self.refresh_clip_snapshot();
            }
            Command::TrimClip(track_id, clip_id, new_internal_start, new_internal_end) => {
                // Trim changes which portion of the source content is used
                // Also updates external_duration to match internal duration (no looping after trim)
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.internal_start = new_internal_start;
                            clip.internal_end = new_internal_end;
                            // By default, trimming sets external_duration to match internal duration
                            clip.external_duration = new_internal_end - new_internal_start;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        // Note: clip_id here is the pool clip ID, not instance ID
                        if let Some(instance) = track.clip_instances.iter_mut().find(|c| c.clip_id == clip_id) {
                            instance.internal_start = new_internal_start;
                            instance.internal_end = new_internal_end;
                            // By default, trimming sets external_duration to match internal duration
                            instance.external_duration = new_internal_end - new_internal_start;
                        }
                    }
                    _ => {}
                }
                self.refresh_clip_snapshot();
            }
            Command::ExtendClip(track_id, clip_id, new_external_duration) => {
                // Extend changes the external duration (enables looping if > internal duration)
                match self.project.get_track_mut(track_id) {
                    Some(crate::audio::track::TrackNode::Audio(track)) => {
                        if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                            clip.external_duration = new_external_duration;
                        }
                    }
                    Some(crate::audio::track::TrackNode::Midi(track)) => {
                        // Note: clip_id here is the pool clip ID, not instance ID
                        if let Some(instance) = track.clip_instances.iter_mut().find(|c| c.clip_id == clip_id) {
                            instance.external_duration = new_external_duration;
                        }
                    }
                    _ => {}
                }
                self.refresh_clip_snapshot();
            }
            Command::CreateMetatrack(name, parent_id) => {
                let track_id = self.project.add_group_track(name.clone(), parent_id);
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
            Command::SetTrimStart(track_id, trim_start) => {
                if let Some(crate::audio::track::TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.trim_start = trim_start.max(0.0);
                }
            }
            Command::SetTrimEnd(track_id, trim_end) => {
                if let Some(crate::audio::track::TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.trim_end = trim_end.map(|t| t.max(0.0));
                }
            }
            Command::CreateAudioTrack(name, parent_id) => {
                let track_id = self.project.add_audio_track(name.clone(), parent_id);
                // Notify UI about the new audio track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
            }
            Command::AddAudioFile(path, data, channels, sample_rate) => {
                println!("🎵 [ENGINE] Received AddAudioFile command for: {}", path);
                // Detect original format from file extension
                let path_buf = std::path::PathBuf::from(path.clone());
                let original_format = path_buf.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|s| s.to_lowercase());

                // Create AudioFile and add to pool
                let audio_file = crate::audio::pool::AudioFile::with_format(
                    path_buf.clone(),
                    data.clone(),  // Clone data for background thread
                    channels,
                    sample_rate,
                    original_format,
                );
                let pool_index = self.audio_pool.add_file(audio_file);
                println!("📦 [ENGINE] Added to pool at index {}", pool_index);

                // Generate Level 0 (overview) waveform chunks asynchronously in background thread
                let chunk_tx = self.chunk_generation_tx.clone();
                let duration = data.len() as f64 / (sample_rate as f64 * channels as f64);
                println!("🔄 [ENGINE] Spawning background thread to generate Level 0 chunks for pool {}", pool_index);
                std::thread::spawn(move || {
                    // Create temporary AudioFile for chunk generation
                    let temp_audio_file = crate::audio::pool::AudioFile::with_format(
                        path_buf,
                        data,
                        channels,
                        sample_rate,
                        None,
                    );

                    // Generate Level 0 chunks
                    let chunk_count = crate::audio::waveform_cache::WaveformCache::calculate_chunk_count(duration, 0);
                    println!("🔄 [BACKGROUND] Generating {} Level 0 chunks for pool {}", chunk_count, pool_index);
                    let chunks = crate::audio::waveform_cache::WaveformCache::generate_chunks(
                        &temp_audio_file,
                        pool_index,
                        0,  // Level 0 (overview)
                        &(0..chunk_count).collect::<Vec<_>>(),
                    );

                    // Send chunks via MPSC channel (will be forwarded by audio thread)
                    if !chunks.is_empty() {
                        println!("📤 [BACKGROUND] Generated {} chunks, sending to audio thread (pool {})", chunks.len(), pool_index);
                        let event_chunks: Vec<(u32, (f64, f64), Vec<crate::io::WaveformPeak>)> = chunks
                            .into_iter()
                            .map(|chunk| (chunk.chunk_index, chunk.time_range, chunk.peaks))
                            .collect();

                        match chunk_tx.send(AudioEvent::WaveformChunksReady {
                            pool_index,
                            detail_level: 0,
                            chunks: event_chunks,
                        }) {
                            Ok(_) => println!("✅ [BACKGROUND] Chunks sent successfully for pool {}", pool_index),
                            Err(e) => eprintln!("❌ [BACKGROUND] Failed to send chunks: {}", e),
                        }
                    } else {
                        eprintln!("⚠️  [BACKGROUND] No chunks generated for pool {}", pool_index);
                    }
                });

                // Notify UI about the new audio file
                let _ = self.event_tx.push(AudioEvent::AudioFileAdded(pool_index, path));
            }
            Command::AddAudioClip(track_id, clip_id, pool_index, start_time, duration, offset) => {
                // Create a new clip instance with the pre-assigned clip_id
                let clip = AudioClipInstance::from_legacy(
                    clip_id,
                    pool_index,
                    start_time,
                    duration,
                    offset,
                );

                // Add clip to track
                if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                    track.clips.push(clip);
                    let _ = self.event_tx.push(AudioEvent::ClipAdded(track_id, clip_id));
                }
                self.refresh_clip_snapshot();
            }
            Command::CreateMidiTrack(name, parent_id) => {
                let track_id = self.project.add_midi_track(name.clone(), parent_id);
                // Notify UI about the new MIDI track
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
            }
            Command::AddMidiClipToPool(clip) => {
                // Add the clip to the pool without placing it on any track
                self.project.midi_clip_pool.add_existing_clip(clip);
            }
            Command::CreateMidiClip(track_id, start_time, duration) => {
                // Get the next MIDI clip ID from the atomic counter
                let clip_id = self.next_midi_clip_id_atomic.fetch_add(1, Ordering::Relaxed);

                // Create clip content in the pool
                let clip = MidiClip::empty(clip_id, duration, format!("MIDI Clip {}", clip_id));
                self.project.midi_clip_pool.add_existing_clip(clip);

                // Create an instance for this clip on the track
                let instance_id = self.project.next_midi_clip_instance_id();
                let instance = MidiClipInstance::from_full_clip(instance_id, clip_id, duration, start_time);

                if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    track.clip_instances.push(instance);
                }

                // Notify UI about the new clip with its ID (using clip_id for now)
                let _ = self.event_tx.push(AudioEvent::ClipAdded(track_id, clip_id));
                self.refresh_clip_snapshot();
            }
            Command::AddMidiNote(track_id, clip_id, time_offset, note, velocity, duration) => {
                // Add a MIDI note event to the specified clip in the pool
                // Note: clip_id here refers to the clip in the pool, not the instance
                if let Some(clip) = self.project.midi_clip_pool.get_clip_mut(clip_id) {
                    // Timestamp is now in seconds (sample-rate independent)
                    let note_on = MidiEvent::note_on(time_offset, 0, note, velocity);
                    clip.add_event(note_on);

                    // Add note off event
                    let note_off_time = time_offset + duration;
                    let note_off = MidiEvent::note_off(note_off_time, 0, note, 64);
                    clip.add_event(note_off);
                } else {
                    // Try legacy behavior: look for instance on track and find its clip
                    if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                        if let Some(instance) = track.clip_instances.iter().find(|c| c.clip_id == clip_id) {
                            let actual_clip_id = instance.clip_id;
                            if let Some(clip) = self.project.midi_clip_pool.get_clip_mut(actual_clip_id) {
                                let note_on = MidiEvent::note_on(time_offset, 0, note, velocity);
                                clip.add_event(note_on);
                                let note_off_time = time_offset + duration;
                                let note_off = MidiEvent::note_off(note_off_time, 0, note, 64);
                                clip.add_event(note_off);
                            }
                        }
                    }
                }
            }
            Command::AddLoadedMidiClip(track_id, clip, start_time) => {
                // Add a pre-loaded MIDI clip to the track with the given start time
                let _ = self.project.add_midi_clip_at(track_id, clip, start_time);
                self.refresh_clip_snapshot();
            }
            Command::UpdateMidiClipNotes(_track_id, clip_id, notes) => {
                // Update all notes in a MIDI clip (directly in the pool)
                if let Some(clip) = self.project.midi_clip_pool.get_clip_mut(clip_id) {
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
            Command::RemoveMidiClip(track_id, instance_id) => {
                // Remove a MIDI clip instance from a track (for undo/redo support)
                let _ = self.project.remove_midi_clip(track_id, instance_id);
                self.refresh_clip_snapshot();
            }
            Command::RemoveAudioClip(track_id, instance_id) => {
                // Deactivate the per-clip disk reader before removing
                if let Some(ref mut dr) = self.disk_reader {
                    dr.send(crate::audio::disk_reader::DiskReaderCommand::DeactivateFile {
                        reader_id: instance_id as u64,
                    });
                }
                // Remove an audio clip instance from a track (for undo/redo support)
                let _ = self.project.remove_audio_clip(track_id, instance_id);
                self.refresh_clip_snapshot();
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
                self.audio_pool = AudioClipPool::new();

                // Reset buffer pool (recreate with same settings)
                let buffer_size = 512 * self.channels as usize;
                self.buffer_pool = BufferPool::new(8, buffer_size);

                // Reset ID counters
                self.next_midi_clip_id_atomic.store(0, Ordering::Relaxed);
                self.next_audio_clip_id_atomic.store(0, Ordering::Relaxed);

                // Clear mix buffer
                self.mix_buffer.clear();

                // Notify UI that reset is complete
                let _ = self.event_tx.push(AudioEvent::ProjectReset);
            }

            Command::SendMidiNoteOn(track_id, note, velocity) => {
                // Send a live MIDI note on event to the specified track's instrument
                self.project.send_midi_note_on(track_id, note, velocity);

                // Emit event to UI for visual feedback
                let _ = self.event_tx.push(AudioEvent::NoteOn(note, velocity));

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

                // Emit event to UI for visual feedback
                let _ = self.event_tx.push(AudioEvent::NoteOff(note));

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

            Command::SetMetronomeEnabled(enabled) => {
                self.metronome.set_enabled(enabled);
            }

            Command::SetInputMonitoring(enabled) => {
                self.input_monitoring = enabled;
            }

            Command::SetInputGain(gain) => {
                self.input_gain = gain;
            }

            Command::SetTempo(bpm, time_sig) => {
                self.metronome.update_timing(bpm, time_sig);
                self.project.set_tempo(bpm, time_sig.0);
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
                    Some(TrackNode::Group(track)) => {
                        Some(&mut track.audio_graph)
                    },
                    _ => {
                        eprintln!("[DEBUG] Track not found or invalid type!");
                        None
                    }
                };

                if let Some(graph) = graph {
                    // Create the node based on type
                    let node = match crate::audio::node_graph::nodes::create_node(&node_type, self.sample_rate, 8192) {
                        Some(n) => n,
                        None => {
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

                    // Automatically set MIDI source nodes as MIDI targets
                    // VoiceAllocator receives MIDI through its input port via connections,
                    // not directly — it needs a MidiInput node connected to its MIDI In
                    if node_type == "MidiInput" {
                        graph.set_midi_target(node_idx, true);
                    }

                    // Automatically set AudioOutput nodes as the graph output
                    if node_type == "AudioOutput" {
                        graph.set_output_node(Some(node_idx));
                    }

                    eprintln!("[DEBUG] Emitting GraphNodeAdded event: track_id={}, node_id={}, node_type={}", track_id, node_id, node_type);
                    // Emit success event
                    let _ = self.event_tx.push(AudioEvent::GraphNodeAdded(track_id, node_id, node_type.clone()));
                    self.set_track_graph_is_default(track_id, false);
                } else {
                    eprintln!("[DEBUG] Graph was None, node not added!");
                }
            }

            Command::GraphAddNodeToTemplate(track_id, voice_allocator_id, node_type, x, y) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    {
                        let va_idx = NodeIndex::new(voice_allocator_id as usize);

                        // Create the node
                        let node = match crate::audio::node_graph::nodes::create_node(&node_type, self.sample_rate, 8192) {
                            Some(n) => n,
                            None => {
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
                                // Set node position in the template graph
                                graph.set_position_in_voice_allocator_template(va_idx, node_id, x, y);
                                println!("Added node {} (ID: {}) to VoiceAllocator {} template at ({}, {})", node_type, node_id, voice_allocator_id, x, y);
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
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    graph.remove_node(node_idx);
                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
                self.set_track_graph_is_default(track_id, false);
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
                    Some(TrackNode::Group(track)) => {
                        Some(&mut track.audio_graph)
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
                            self.set_track_graph_is_default(track_id, false);
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

            Command::GraphDisconnectInTemplate(track_id, voice_allocator_id, from, from_port, to, to_port) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let va_idx = NodeIndex::new(voice_allocator_id as usize);

                    match graph.disconnect_in_voice_allocator_template(va_idx, from, from_port, to, to_port) {
                        Ok(()) => {
                            let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                        }
                        Err(e) => {
                            let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                track_id,
                                format!("Failed to disconnect in template: {}", e)
                            ));
                        }
                    }
                }
            }

            Command::GraphRemoveNodeFromTemplate(track_id, voice_allocator_id, node_index) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let va_idx = NodeIndex::new(voice_allocator_id as usize);

                    match graph.remove_node_from_voice_allocator_template(va_idx, node_index) {
                        Ok(()) => {
                            let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                        }
                        Err(e) => {
                            let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                track_id,
                                format!("Failed to remove node from template: {}", e)
                            ));
                        }
                    }
                }
            }

            Command::GraphSetParameterInTemplate(track_id, voice_allocator_id, node_index, param_id, value) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let va_idx = NodeIndex::new(voice_allocator_id as usize);

                    if let Err(e) = graph.set_parameter_in_voice_allocator_template(va_idx, node_index, param_id, value) {
                        let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                            track_id,
                            format!("Failed to set parameter in template: {}", e)
                        ));
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
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
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
                self.set_track_graph_is_default(track_id, false);
            }

            Command::GraphSetParameter(track_id, node_index, param_id, value) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        graph_node.node.set_parameter(param_id, value);
                    }
                }
                self.set_track_graph_is_default(track_id, false);
            }

            Command::GraphSetNodePosition(track_id, node_index, x, y) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    graph.set_node_position(node_idx, x, y);
                }
            }

            Command::GraphSetNodePositionInTemplate(track_id, voice_allocator_id, node_index, x, y) => {
                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let va_idx = NodeIndex::new(voice_allocator_id as usize);
                    graph.set_position_in_voice_allocator_template(va_idx, node_index, x, y);
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
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_index as usize);
                    graph.set_output_node(Some(node_idx));
                }
            }

            Command::GraphSetGroups(track_id, groups) => {
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    Some(TrackNode::Group(track)) => Some(&mut track.audio_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    graph.set_frontend_groups(groups);
                }
            }

            Command::GraphSetGroupsInTemplate(track_id, voice_allocator_id, groups) => {
                use crate::audio::node_graph::nodes::VoiceAllocatorNode;
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(voice_allocator_id as usize);
                    if let Some(graph_node) = graph.get_node_mut(node_idx) {
                        if let Some(va_node) = graph_node.as_any_mut().downcast_mut::<VoiceAllocatorNode>() {
                            va_node.template_graph_mut().set_frontend_groups(groups);
                        }
                    }
                }
            }

            Command::GraphSavePreset(track_id, preset_path, preset_name, description, tags) => {
                let graph = match self.project.get_track(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&track.effects_graph),
                    Some(TrackNode::Group(track)) => Some(&track.audio_graph),
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
                        match std::fs::write(&preset_path, json) {
                            Ok(_) => {
                                // Emit success event with path
                                let _ = self.event_tx.push(AudioEvent::GraphPresetSaved(
                                    track_id,
                                    preset_path.clone()
                                ));
                            }
                            Err(e) => {
                                let _ = self.event_tx.push(AudioEvent::GraphConnectionError(
                                    track_id,
                                    format!("Failed to save preset: {}", e)
                                ));
                            }
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
                                                track.graph_is_default = true;
                                                let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                                                let _ = self.event_tx.push(AudioEvent::GraphPresetLoaded(track_id));
                                            }
                                            Some(TrackNode::Audio(track)) => {
                                                track.effects_graph = graph;
                                                track.graph_is_default = true;
                                                let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                                                let _ = self.event_tx.push(AudioEvent::GraphPresetLoaded(track_id));
                                            }
                                            Some(TrackNode::Group(track)) => {
                                                track.audio_graph = graph;
                                                track.graph_is_default = true;
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
                        // Downcast to VoiceAllocatorNode using safe Any trait
                        if let Some(va_node) = node.as_any().downcast_ref::<VoiceAllocatorNode>() {
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

            Command::SetMetatrackSubtrackGraph(track_id, subtracks) => {
                let buffer_size = self.buffer_pool.buffer_size();
                if let Some(TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    let current = metatrack.current_subtracks();

                    // No-op if subtrack list is unchanged (prevents every-frame graph rebuilds)
                    if current == subtracks {
                        return;
                    }

                    if metatrack.graph_is_default {
                        // Default graph: full rebuild with new subtrack layout
                        metatrack.set_subtrack_graph(subtracks.clone(), self.sample_rate, buffer_size);
                    } else {
                        // User-modified graph: incremental port changes only
                        let current_ids: std::collections::HashSet<TrackId> =
                            current.iter().map(|&(id, _)| id).collect();
                        let new_ids: std::collections::HashSet<TrackId> =
                            subtracks.iter().map(|&(id, _)| id).collect();
                        for (id, name) in &subtracks {
                            if !current_ids.contains(id) {
                                metatrack.add_subtrack_to_graph(*id, name.clone(), buffer_size);
                            }
                        }
                        for &(id, _) in &current {
                            if !new_ids.contains(&id) {
                                metatrack.remove_subtrack_from_graph(id, buffer_size);
                            }
                        }
                    }
                    // Sync the group's children list so they render through the mixer graph.
                    // `move_to_group` removes each child from root_tracks (or another parent)
                    // and registers it under this group — idempotent if already there.
                    let new_child_ids: Vec<TrackId> = subtracks.iter().map(|&(id, _)| id).collect();
                    for &child_id in &new_child_ids {
                        // Only move if not already a child of this group
                        let already_child = self.project.get_track(track_id)
                            .and_then(|t| if let TrackNode::Group(g) = t { Some(g) } else { None })
                            .map(|g| g.children.contains(&child_id))
                            .unwrap_or(false);
                        if !already_child {
                            self.project.move_to_group(child_id, track_id);
                        }
                    }

                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
            }

            Command::AddMetatrackSubtrack(track_id, subtrack_id, name) => {
                let buffer_size = self.buffer_pool.buffer_size();
                if let Some(TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.add_subtrack_to_graph(subtrack_id, name, buffer_size);
                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
            }

            Command::RemoveMetatrackSubtrack(track_id, subtrack_id) => {
                let buffer_size = self.buffer_pool.buffer_size();
                if let Some(TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.remove_subtrack_from_graph(subtrack_id, buffer_size);
                    let _ = self.event_tx.push(AudioEvent::GraphStateChanged(track_id));
                }
            }

            Command::UpdateMetatrackSubtrackIds(track_id, subtracks) => {
                let buffer_size = self.buffer_pool.buffer_size();
                if let Some(TrackNode::Group(metatrack)) = self.project.get_track_mut(track_id) {
                    metatrack.update_subtrack_ids(subtracks, buffer_size);
                }
            }

            Command::SetGraphIsDefault(track_id, value) => {
                self.set_track_graph_is_default(track_id, value);
            }

            Command::GraphSetScript(track_id, node_id, source) => {
                use crate::audio::node_graph::nodes::ScriptNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(script_node) = graph_node.node.as_any_mut().downcast_mut::<ScriptNode>() {
                            match script_node.set_script(&source) {
                                Ok(ui_decl) => {
                                    // Send compile success event back to frontend
                                    let _ = self.event_tx.push(AudioEvent::ScriptCompiled {
                                        track_id,
                                        node_id,
                                        success: true,
                                        error: None,
                                        ui_declaration: Some(ui_decl),
                                        source: source.clone(),
                                    });
                                }
                                Err(e) => {
                                    let _ = self.event_tx.push(AudioEvent::ScriptCompiled {
                                        track_id,
                                        node_id,
                                        success: false,
                                        error: Some(e),
                                        ui_declaration: None,
                                        source,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            Command::GraphSetScriptSample(track_id, node_id, slot_index, data, sample_rate, name) => {
                use crate::audio::node_graph::nodes::ScriptNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(script_node) = graph_node.node.as_any_mut().downcast_mut::<ScriptNode>() {
                            script_node.set_sample(slot_index, data, sample_rate, name);
                        }
                    }
                }
            }

            Command::AmpSimLoadModel(track_id, node_id, model_path) => {
                use crate::audio::node_graph::nodes::AmpSimNode;

                eprintln!("[AmpSim] Loading model: {:?} for track {:?} node {}", model_path, track_id, node_id);
                let graph = match self.project.get_track_mut(track_id) {
                    Some(TrackNode::Midi(track)) => Some(&mut track.instrument_graph),
                    Some(TrackNode::Audio(track)) => Some(&mut track.effects_graph),
                    _ => None,
                };
                if let Some(graph) = graph {
                    let node_idx = NodeIndex::new(node_id as usize);
                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(amp_sim) = graph_node.node.as_any_mut().downcast_mut::<AmpSimNode>() {
                            let result = if let Some(bundled_name) = model_path.strip_prefix("bundled:") {
                                eprintln!("[AmpSim] Loading bundled model: {}", bundled_name);
                                amp_sim.load_bundled_model(bundled_name)
                            } else {
                                eprintln!("[AmpSim] Loading model from file: {}", model_path);
                                amp_sim.load_model(&model_path)
                            };
                            match &result {
                                Ok(()) => eprintln!("[AmpSim] Model loaded successfully"),
                                Err(e) => eprintln!("[AmpSim] Failed to load NAM model: {}", e),
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
                        // Downcast to SimpleSamplerNode using safe Any trait
                        if let Some(sampler_node) = graph_node.node.as_any_mut().downcast_mut::<SimpleSamplerNode>() {
                            if let Err(e) = sampler_node.load_sample_from_file(&file_path) {
                                eprintln!("Failed to load sample: {}", e);
                            }
                        }
                    }
                }
            }

            Command::SamplerLoadFromPool(track_id, node_id, pool_index) => {
                use crate::audio::node_graph::nodes::SimpleSamplerNode;

                let sample_result = Self::read_mono_from_pool(&self.audio_pool, pool_index);

                if let Some((mono_samples, sample_rate)) = sample_result {
                    if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                        let graph = &mut track.instrument_graph;
                        let node_idx = NodeIndex::new(node_id as usize);
                        if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                            if let Some(sampler_node) = graph_node.node.as_any_mut().downcast_mut::<SimpleSamplerNode>() {
                                sampler_node.set_sample(mono_samples, sample_rate);
                            }
                        }
                    }
                }
            }

            Command::SamplerSetRootNote(track_id, node_id, root_note) => {
                use crate::audio::node_graph::nodes::SimpleSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);
                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(sampler_node) = graph_node.node.as_any_mut().downcast_mut::<SimpleSamplerNode>() {
                            sampler_node.set_root_note(root_note);
                        }
                    }
                }
            }

            Command::MultiSamplerAddLayer(track_id, node_id, file_path, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to MultiSamplerNode using safe Any trait
                        if let Some(multi_sampler_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                            if let Err(e) = multi_sampler_node.load_layer_from_file(&file_path, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode) {
                                eprintln!("Failed to add sample layer: {}", e);
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerAddLayerFromPool(track_id, node_id, pool_index, key_min, key_max, root_key) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;
                use crate::audio::node_graph::nodes::LoopMode;

                let sample_result = Self::read_mono_from_pool(&self.audio_pool, pool_index);

                if let Some((mono_samples, sample_rate)) = sample_result {
                    if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                        let graph = &mut track.instrument_graph;
                        let node_idx = NodeIndex::new(node_id as usize);
                        if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                            if let Some(multi_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                                multi_node.add_layer(
                                    mono_samples, sample_rate,
                                    key_min, key_max, root_key,
                                    0, 127, None, None, LoopMode::OneShot,
                                );
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerUpdateLayer(track_id, node_id, layer_index, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        // Downcast to MultiSamplerNode using safe Any trait
                        if let Some(multi_sampler_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                            if let Err(e) = multi_sampler_node.update_layer(layer_index, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode) {
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
                        // Downcast to MultiSamplerNode using safe Any trait
                        if let Some(multi_sampler_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                            if let Err(e) = multi_sampler_node.remove_layer(layer_index) {
                                eprintln!("Failed to remove sample layer: {}", e);
                            }
                        }
                    }
                }
            }

            Command::MultiSamplerClearLayers(track_id, node_id) => {
                use crate::audio::node_graph::nodes::MultiSamplerNode;

                if let Some(TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    let graph = &mut track.instrument_graph;
                    let node_idx = NodeIndex::new(node_id as usize);

                    if let Some(graph_node) = graph.get_graph_node_mut(node_idx) {
                        if let Some(multi_sampler_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                            multi_sampler_node.clear_layers();
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

            Command::GenerateWaveformChunks {
                pool_index,
                detail_level,
                chunk_indices,
                priority: _priority, // TODO: Use priority for scheduling
            } => {
                println!("🔧 [ENGINE] Received GenerateWaveformChunks command: pool={}, level={}, chunks={:?}",
                    pool_index, detail_level, chunk_indices);
                // Get audio file data from pool
                if let Some(audio_file) = self.audio_pool.get_file(pool_index) {
                    println!("✅ [ENGINE] Found audio file in pool, queuing work in thread pool");
                    // Clone necessary data for background thread
                    let data = audio_file.data().to_vec();
                    let channels = audio_file.channels;
                    let sample_rate = audio_file.sample_rate;
                    let path = audio_file.path.clone();
                    let chunk_tx = self.chunk_generation_tx.clone();

                    // Generate chunks using rayon's thread pool to avoid spawning thousands of threads
                    rayon::spawn(move || {
                        // Create temporary AudioFile for chunk generation
                        let temp_audio_file = crate::audio::pool::AudioFile::with_format(
                            path,
                            data,
                            channels,
                            sample_rate,
                            None,
                        );

                        // Generate requested chunks
                        let chunks = crate::audio::waveform_cache::WaveformCache::generate_chunks(
                            &temp_audio_file,
                            pool_index,
                            detail_level,
                            &chunk_indices,
                        );

                        // Send chunks via MPSC channel (will be forwarded by audio thread)
                        if !chunks.is_empty() {
                            let event_chunks: Vec<(u32, (f64, f64), Vec<crate::io::WaveformPeak>)> = chunks
                                .into_iter()
                                .map(|chunk| (chunk.chunk_index, chunk.time_range, chunk.peaks))
                                .collect();

                            let _ = chunk_tx.send(AudioEvent::WaveformChunksReady {
                                pool_index,
                                detail_level,
                                chunks: event_chunks,
                            });
                        }

                        // Yield to other threads to reduce CPU contention with video playback
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    });
                } else {
                    eprintln!("❌ [ENGINE] Pool index {} not found for waveform generation", pool_index);
                }
            }

            Command::ImportAudio(path) => {
                if let Err(e) = self.do_import_audio(&path) {
                    eprintln!("[ENGINE] ImportAudio failed for {:?}: {}", path, e);
                }
            }
        }
    }

    /// Import an audio file into the pool: mmap for PCM, streaming for compressed.
    /// Returns the pool index on success. Emits AudioFileReady event.
    fn do_import_audio(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let path_str = path.to_string_lossy().to_string();

        let metadata = crate::io::read_metadata(path)
            .map_err(|e| format!("Failed to read metadata for {:?}: {}", path, e))?;

        eprintln!("[ENGINE] ImportAudio: format={:?}, ch={}, sr={}, n_frames={:?}, duration={:.2}s, path={}",
            metadata.format, metadata.channels, metadata.sample_rate, metadata.n_frames, metadata.duration, path_str);

        let pool_index = match metadata.format {
            crate::io::AudioFormat::Pcm => {
                let file = std::fs::File::open(path)
                    .map_err(|e| format!("Failed to open {:?}: {}", path, e))?;

                // SAFETY: The file is opened read-only. The mmap is shared
                // immutably. We never write to it.
                let mmap = unsafe { memmap2::Mmap::map(&file) }
                    .map_err(|e| format!("mmap failed for {:?}: {}", path, e))?;

                let header = crate::io::parse_wav_header(&mmap)
                    .map_err(|e| format!("WAV parse failed for {:?}: {}", path, e))?;

                let audio_file = crate::audio::pool::AudioFile::from_mmap(
                    path.to_path_buf(),
                    mmap,
                    header.data_offset,
                    header.sample_format,
                    header.channels,
                    header.sample_rate,
                    header.total_frames,
                );

                self.audio_pool.add_file(audio_file)
            }
            crate::io::AudioFormat::Compressed => {
                let sync_decode = std::env::var("DAW_SYNC_DECODE").is_ok();

                if sync_decode {
                    eprintln!("[ENGINE] DAW_SYNC_DECODE: doing full decode of {:?}", path);
                    let loaded = crate::io::AudioFile::load(path)
                        .map_err(|e| format!("DAW_SYNC_DECODE failed: {}", e))?;
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .map(|s| s.to_lowercase());
                    let audio_file = crate::audio::pool::AudioFile::with_format(
                        path.to_path_buf(),
                        loaded.data,
                        loaded.channels,
                        loaded.sample_rate,
                        ext,
                    );
                    let idx = self.audio_pool.add_file(audio_file);
                    eprintln!("[ENGINE] DAW_SYNC_DECODE: pool_index={}, frames={}", idx, loaded.frames);
                    idx
                } else {
                    let ext = path.extension()
                        .and_then(|e| e.to_str())
                        .map(|s| s.to_lowercase());

                    let total_frames = metadata.n_frames.unwrap_or_else(|| {
                        (metadata.duration * metadata.sample_rate as f64).ceil() as u64
                    });

                    let audio_file = crate::audio::pool::AudioFile::from_compressed(
                        path.to_path_buf(),
                        metadata.channels,
                        metadata.sample_rate,
                        total_frames,
                        ext,
                    );

                    let idx = self.audio_pool.add_file(audio_file);

                    eprintln!("[ENGINE] Compressed: total_frames={}, pool_index={}, has_disk_reader={}",
                        total_frames, idx, self.disk_reader.is_some());

                    // Spawn background thread to decode file progressively for waveform display
                    let bg_tx = self.chunk_generation_tx.clone();
                    let bg_path = path.to_path_buf();
                    let bg_total_frames = total_frames;
                    let _ = std::thread::Builder::new()
                        .name(format!("waveform-decode-{}", idx))
                        .spawn(move || {
                            crate::io::AudioFile::decode_progressive(
                                &bg_path,
                                bg_total_frames,
                                |audio_data, decoded_frames, total| {
                                    let _ = bg_tx.send(AudioEvent::WaveformDecodeComplete {
                                        pool_index: idx,
                                        samples: audio_data.to_vec(),
                                        decoded_frames,
                                        total_frames: total,
                                    });
                                },
                            );
                        });
                    idx
                }
            }
        };

        // Emit AudioFileReady event
        let _ = self.event_tx.push(AudioEvent::AudioFileReady {
            pool_index,
            path: path_str,
            channels: metadata.channels,
            sample_rate: metadata.sample_rate,
            duration: metadata.duration,
            format: metadata.format,
        });

        // For PCM files, send samples inline so the UI doesn't need to
        // do a blocking get_pool_audio_samples() query.
        if metadata.format == crate::io::AudioFormat::Pcm {
            if let Some(file) = self.audio_pool.get_file(pool_index) {
                let samples = file.data().to_vec();
                if !samples.is_empty() {
                    let _ = self.event_tx.push(AudioEvent::AudioDecodeProgress {
                        pool_index,
                        samples,
                        sample_rate: metadata.sample_rate,
                        channels: metadata.channels,
                    });
                }
            }
        }

        Ok(pool_index)
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
                    Some(TrackNode::Group(track)) => {
                        let graph = &track.audio_graph;
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
                        // Downcast to VoiceAllocatorNode using safe Any trait
                        if let Some(va_node) = graph_node.node.as_any().downcast_ref::<VoiceAllocatorNode>() {
                            let template_preset = va_node.template_graph().to_preset("template");
                            match template_preset.to_json() {
                                Ok(json) => QueryResponse::GraphState(Ok(json)),
                                Err(e) => QueryResponse::GraphState(Err(format!("Failed to serialize template: {:?}", e))),
                            }
                        } else {
                            QueryResponse::GraphState(Err("Node is not a VoiceAllocatorNode".to_string()))
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
            Query::GetVoiceOscilloscopeData(track_id, va_node_id, inner_node_id, sample_count) => {
                match self.project.get_voice_oscilloscope_data(track_id, va_node_id, inner_node_id, sample_count) {
                    Some((audio, cv)) => {
                        use crate::command::OscilloscopeData;
                        QueryResponse::OscilloscopeData(Ok(OscilloscopeData { audio, cv }))
                    }
                    None => QueryResponse::OscilloscopeData(Err(format!(
                        "Failed to get voice oscilloscope data from track {} VA {} node {}",
                        track_id, va_node_id, inner_node_id
                    ))),
                }
            }
            Query::GetMidiClip(_track_id, clip_id) => {
                // Get MIDI clip data from the pool
                if let Some(clip) = self.project.midi_clip_pool.get_clip(clip_id) {
                    use crate::command::MidiClipData;
                    QueryResponse::MidiClipData(Ok(MidiClipData {
                        duration: clip.duration,
                        events: clip.events.clone(),
                    }))
                } else {
                    QueryResponse::MidiClipData(Err(format!("Clip {} not found in pool", clip_id)))
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
            Query::CreateAudioTrackSync(name, parent_id) => {
                let track_id = self.project.add_audio_track(name.clone(), parent_id);
                eprintln!("[Engine] Created audio track '{}' with ID {} (parent: {:?})", name, track_id, parent_id);
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
                QueryResponse::TrackCreated(Ok(track_id))
            }
            Query::CreateMidiTrackSync(name, parent_id) => {
                let track_id = self.project.add_midi_track(name.clone(), parent_id);
                eprintln!("[Engine] Created MIDI track '{}' with ID {} (parent: {:?})", name, track_id, parent_id);
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, false, name));
                QueryResponse::TrackCreated(Ok(track_id))
            }
            Query::CreateMetatrackSync(name, parent_id) => {
                let track_id = self.project.add_group_track(name.clone(), parent_id);
                eprintln!("[Engine] Created metatrack '{}' with ID {} (parent: {:?})", name, track_id, parent_id);
                let _ = self.event_tx.push(AudioEvent::TrackCreated(track_id, true, name));
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
            Query::GetPoolAudioSamples(pool_index) => {
                match self.audio_pool.get_file(pool_index) {
                    Some(file) => {
                        // For Compressed storage, return decoded_for_waveform if available
                        let samples = match &file.storage {
                            crate::audio::pool::AudioStorage::Compressed {
                                decoded_for_waveform, decoded_frames, ..
                            } if *decoded_frames > 0 => {
                                decoded_for_waveform.clone()
                            }
                            _ => file.data().to_vec(),
                        };
                        QueryResponse::PoolAudioSamples(Ok((
                            samples,
                            file.sample_rate,
                            file.channels,
                        )))
                    }
                    None => QueryResponse::PoolAudioSamples(Err(format!("Pool index {} not found", pool_index))),
                }
            }
            Query::ExportAudio(settings, output_path) => {
                // Perform export directly - this will block the audio thread but that's okay
                // since we're exporting and not playing back anyway

                // Pass event_tx directly - Rust allows borrowing different fields simultaneously
                match crate::audio::export_audio(
                    &mut self.project,
                    &self.audio_pool,
                    &settings,
                    &output_path,
                    Some(&mut self.event_tx),
                ) {
                    Ok(()) => QueryResponse::AudioExported(Ok(())),
                    Err(e) => QueryResponse::AudioExported(Err(e)),
                }
            }
            Query::AddMidiClipSync(track_id, clip, start_time) => {
                // Add MIDI clip to track and return the instance ID
                let result = match self.project.add_midi_clip_at(track_id, clip, start_time) {
                    Ok(instance_id) => QueryResponse::MidiClipInstanceAdded(Ok(instance_id)),
                    Err(e) => QueryResponse::MidiClipInstanceAdded(Err(e.to_string())),
                };
                self.refresh_clip_snapshot();
                result
            }
            Query::AddMidiClipInstanceSync(track_id, mut instance) => {
                // Add MIDI clip instance to track (clip must already be in pool)
                // Assign instance ID
                let instance_id = self.project.next_midi_clip_instance_id();
                instance.id = instance_id;

                let result = match self.project.add_midi_clip_instance(track_id, instance) {
                    Ok(_) => QueryResponse::MidiClipInstanceAdded(Ok(instance_id)),
                    Err(e) => QueryResponse::MidiClipInstanceAdded(Err(e.to_string())),
                };
                self.refresh_clip_snapshot();
                result
            }
            Query::AddAudioFileSync(path, data, channels, sample_rate) => {
                // Add audio file to pool and return the pool index
                // Detect original format from file extension
                let path_buf = std::path::PathBuf::from(&path);
                let original_format = path_buf.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|s| s.to_lowercase());

                // Create AudioFile and add to pool
                let audio_file = crate::audio::pool::AudioFile::with_format(
                    path_buf.clone(),
                    data.clone(),  // Clone data for background thread
                    channels,
                    sample_rate,
                    original_format,
                );
                let pool_index = self.audio_pool.add_file(audio_file);

                // Generate Level 0 (overview) waveform chunks asynchronously in background thread
                let chunk_tx = self.chunk_generation_tx.clone();
                let duration = data.len() as f64 / (sample_rate as f64 * channels as f64);
                println!("🔄 [ENGINE] Spawning background thread to generate Level 0 chunks for pool {}", pool_index);
                std::thread::spawn(move || {
                    // Create temporary AudioFile for chunk generation
                    let temp_audio_file = crate::audio::pool::AudioFile::with_format(
                        path_buf,
                        data,
                        channels,
                        sample_rate,
                        None,
                    );

                    // Generate Level 0 chunks
                    let chunk_count = crate::audio::waveform_cache::WaveformCache::calculate_chunk_count(duration, 0);
                    println!("🔄 [BACKGROUND] Generating {} Level 0 chunks for pool {}", chunk_count, pool_index);
                    let chunks = crate::audio::waveform_cache::WaveformCache::generate_chunks(
                        &temp_audio_file,
                        pool_index,
                        0,  // Level 0 (overview)
                        &(0..chunk_count).collect::<Vec<_>>(),
                    );

                    // Send chunks via MPSC channel (will be forwarded by audio thread)
                    if !chunks.is_empty() {
                        println!("📤 [BACKGROUND] Generated {} chunks, sending to audio thread (pool {})", chunks.len(), pool_index);
                        let event_chunks: Vec<(u32, (f64, f64), Vec<crate::io::WaveformPeak>)> = chunks
                            .into_iter()
                            .map(|chunk| (chunk.chunk_index, chunk.time_range, chunk.peaks))
                            .collect();

                        match chunk_tx.send(AudioEvent::WaveformChunksReady {
                            pool_index,
                            detail_level: 0,
                            chunks: event_chunks,
                        }) {
                            Ok(_) => println!("✅ [BACKGROUND] Chunks sent successfully for pool {}", pool_index),
                            Err(e) => eprintln!("❌ [BACKGROUND] Failed to send chunks: {}", e),
                        }
                    } else {
                        eprintln!("⚠️  [BACKGROUND] No chunks generated for pool {}", pool_index);
                    }
                });

                // Notify UI about the new audio file (for event listeners)
                let _ = self.event_tx.push(AudioEvent::AudioFileAdded(pool_index, path));

                QueryResponse::AudioFileAddedSync(Ok(pool_index))
            }
            Query::ImportAudioSync(path) => {
                QueryResponse::AudioImportedSync(self.do_import_audio(&path))
            }
            Query::GetProject => {
                // Save graph presets before cloning — AudioTrack::clone() creates
                // a fresh default graph (not a copy), so the preset must be populated
                // first so the clone carries the serialized graph data.
                self.project.prepare_for_save();
                QueryResponse::ProjectRetrieved(Ok(Box::new(self.project.clone())))
            }
            Query::SetProject(new_project) => {
                // Replace the current project with the new one
                // Need to rebuild audio graphs with current sample_rate and buffer_size
                let mut project = *new_project;
                match project.rebuild_audio_graphs(self.buffer_pool.buffer_size()) {
                    Ok(()) => {
                        self.project = project;
                        QueryResponse::ProjectSet(Ok(()))
                    }
                    Err(e) => QueryResponse::ProjectSet(Err(format!("Failed to rebuild audio graphs: {}", e))),
                }
            }
            Query::DuplicateMidiClipSync(clip_id) => {
                match self.project.midi_clip_pool.duplicate_clip(clip_id) {
                    Some(new_id) => QueryResponse::MidiClipDuplicated(Ok(new_id)),
                    None => QueryResponse::MidiClipDuplicated(Err(format!("MIDI clip {} not found", clip_id))),
                }
            }
            Query::GetGraphIsDefault(track_id) => {
                let is_default = match self.project.get_track(track_id) {
                    Some(TrackNode::Midi(track)) => track.graph_is_default,
                    Some(TrackNode::Audio(track)) => track.graph_is_default,
                    Some(TrackNode::Group(track)) => track.graph_is_default,
                    _ => false,
                };
                QueryResponse::GraphIsDefault(is_default)
            }
        };

        // Send response back
        match self.query_response_tx.push(response) {
            Ok(_) => {},
            Err(_) => eprintln!("❌ [ENGINE] FAILED to send query response - queue full!"),
        }
    }

    /// Set graph_is_default on any track type.
    fn set_track_graph_is_default(&mut self, track_id: TrackId, value: bool) {
        match self.project.get_track_mut(track_id) {
            Some(TrackNode::Midi(track)) => track.graph_is_default = value,
            Some(TrackNode::Audio(track)) => track.graph_is_default = value,
            Some(TrackNode::Group(track)) => track.graph_is_default = value,
            _ => {}
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
                    // Create intermediate clip with a unique ID
                    let clip_id = self.next_audio_clip_id_atomic.fetch_add(1, Ordering::Relaxed);

                    let clip = crate::audio::clip::Clip::new(
                        clip_id,
                        0, // Temporary pool index, will be updated on finalization
                        0.0, // internal_start
                        0.0, // internal_end - Duration starts at 0, will be updated during recording
                        start_time, // external_start (timeline position)
                        start_time, // external_end - will be updated during recording
                    );

                    // Add clip to track
                    if let Some(crate::audio::track::TrackNode::Audio(track)) = self.project.get_track_mut(track_id) {
                        track.clips.push(clip);
                        self.refresh_clip_snapshot();
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

                    // Count stale samples so we can skip them incrementally
                    let samples_in_buffer = if let Some(input_rx) = &self.input_rx {
                        input_rx.slots()
                    } else {
                        0
                    };

                    self.recording_state = Some(recording_state);
                    self.recording_progress_counter = 0; // Reset progress counter

                    // Set samples to skip (drained incrementally across callbacks)
                    if let Some(recording) = &mut self.recording_state {
                        recording.samples_to_skip = samples_in_buffer;
                        if self.debug_audio && samples_in_buffer > 0 {
                            eprintln!("[AUDIO DEBUG] Will skip {} stale samples from input buffer", samples_in_buffer);
                        }
                    }

                    // Notify UI that recording has started
                    let _ = self.event_tx.push(AudioEvent::RecordingStarted(track_id, clip_id, self.sample_rate, self.channels));
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
                    // Recorded audio is always WAV format
                    let pool_file = crate::audio::pool::AudioFile::with_format(
                        temp_file_path.clone(),
                        audio_data,
                        channels,
                        sample_rate,
                        Some("wav".to_string()),
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
                    self.refresh_clip_snapshot();

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
            // Send note-off to the synth for any notes still held, so they don't get stuck
            let track_id_for_noteoff = recording.track_id;
            for note_num in recording.active_note_numbers() {
                self.project.send_midi_note_off(track_id_for_noteoff, note_num);
            }

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

            // Update the MIDI clip in the pool (new model: clips are stored centrally in the pool)
            eprintln!("[MIDI_RECORDING] Looking for clip {} in midi_clip_pool", clip_id);
            if let Some(clip) = self.project.midi_clip_pool.get_clip_mut(clip_id) {
                eprintln!("[MIDI_RECORDING] Found clip in pool, clearing and adding {} notes", note_count);
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

                // Also update the clip instance's internal_end and external_duration to match the recording duration
                if let Some(crate::audio::track::TrackNode::Midi(track)) = self.project.get_track_mut(track_id) {
                    if let Some(instance) = track.clip_instances.iter_mut().find(|i| i.clip_id == clip_id) {
                        instance.internal_end = recording_duration;
                        instance.external_duration = recording_duration;
                        eprintln!("[MIDI_RECORDING] Updated clip instance timing: internal_end={:.3}s, external_duration={:.3}s",
                                  instance.internal_end, instance.external_duration);
                    }
                }
            } else {
                eprintln!("[MIDI_RECORDING] ERROR: Clip {} not found in pool!", clip_id);
            }

            self.refresh_clip_snapshot();

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
    next_audio_clip_id: Arc<AtomicU32>,
    clip_snapshot: Arc<RwLock<AudioClipSnapshot>>,
    sample_rate: u32,
    #[allow(dead_code)] // Used in public getter method
    channels: u32,
    /// Cached export response found by other query methods
    cached_export_response: Option<Result<(), String>>,
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

    /// Enable or disable input monitoring (mic level metering)
    pub fn set_input_monitoring(&mut self, enabled: bool) {
        let _ = self.command_tx.push(Command::SetInputMonitoring(enabled));
    }

    /// Set the input gain multiplier (applied before recording)
    pub fn set_input_gain(&mut self, gain: f32) {
        let _ = self.command_tx.push(Command::SetInputGain(gain));
    }

    /// Move a clip to a new timeline position (changes external_start)
    pub fn move_clip(&mut self, track_id: TrackId, clip_id: ClipId, new_start_time: f64) {
        let _ = self.command_tx.push(Command::MoveClip(track_id, clip_id, new_start_time));
    }

    /// Trim a clip's internal boundaries (changes which portion of source content is used)
    /// This also resets external_duration to match internal duration (disables looping)
    pub fn trim_clip(&mut self, track_id: TrackId, clip_id: ClipId, new_internal_start: f64, new_internal_end: f64) {
        let _ = self.command_tx.push(Command::TrimClip(track_id, clip_id, new_internal_start, new_internal_end));
    }

    /// Extend or shrink a clip's external duration (enables looping if > internal duration)
    pub fn extend_clip(&mut self, track_id: TrackId, clip_id: ClipId, new_external_duration: f64) {
        let _ = self.command_tx.push(Command::ExtendClip(track_id, clip_id, new_external_duration));
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

    /// Get the shared clip snapshot. The UI can read this each frame to display
    /// the authoritative clip state from the backend.
    pub fn clip_snapshot(&self) -> Arc<RwLock<AudioClipSnapshot>> {
        Arc::clone(&self.clip_snapshot)
    }

    /// Create a new metatrack
    pub fn create_metatrack(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateMetatrack(name, None));
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

    /// Set metatrack trim start in seconds
    pub fn set_trim_start(&mut self, track_id: TrackId, trim_start: f64) {
        let _ = self.command_tx.push(Command::SetTrimStart(track_id, trim_start));
    }

    /// Set metatrack trim end in seconds (None = no end trim)
    pub fn set_trim_end(&mut self, track_id: TrackId, trim_end: Option<f64>) {
        let _ = self.command_tx.push(Command::SetTrimEnd(track_id, trim_end));
    }

    /// Create a new audio track
    pub fn create_audio_track(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateAudioTrack(name, None));
    }

    /// Add an audio file to the pool (must be called from non-audio thread with pre-loaded data)
    pub fn add_audio_file(&mut self, path: String, data: Vec<f32>, channels: u32, sample_rate: u32) {
        match self.command_tx.push(Command::AddAudioFile(path.clone(), data, channels, sample_rate)) {
            Ok(_) => println!("✅ [CONTROLLER] AddAudioFile command queued successfully: {}", path),
            Err(_) => eprintln!("❌ [CONTROLLER] Failed to queue AddAudioFile command (buffer full): {}", path),
        }
    }

    /// Add an audio file to the pool synchronously and get the pool index
    /// Returns the pool index where the audio file was added
    pub fn add_audio_file_sync(&mut self, path: String, data: Vec<f32>, channels: u32, sample_rate: u32) -> Result<usize, String> {
        let query = Query::AddAudioFileSync(path, data, channels, sample_rate);
        match self.send_query(query)? {
            QueryResponse::AudioFileAddedSync(result) => result,
            _ => Err("Unexpected query response".to_string()),
        }
    }

    /// Import an audio file asynchronously. The engine will memory-map WAV/AIFF
    /// files for instant availability, or set up stream decoding for compressed
    /// formats. Listen for `AudioEvent::AudioFileReady` to get the pool index.
    pub fn import_audio(&mut self, path: std::path::PathBuf) {
        let _ = self.command_tx.push(Command::ImportAudio(path));
    }

    /// Import an audio file synchronously and get the pool index.
    /// Does the same work as `import_audio` (mmap for PCM, streaming for
    /// compressed) but returns the real pool index directly.
    /// NOTE: briefly blocks the UI thread during file setup (sub-ms for PCM
    /// mmap; a few ms for compressed streaming init). If this becomes a
    /// problem for very large files, switch to async import with event-based
    /// pool index reconciliation.
    pub fn import_audio_sync(&mut self, path: std::path::PathBuf) -> Result<usize, String> {
        let query = Query::ImportAudioSync(path);
        match self.send_query(query)? {
            QueryResponse::AudioImportedSync(result) => result,
            _ => Err("Unexpected query response".to_string()),
        }
    }

    /// Generate the next unique audio clip instance ID (atomic, thread-safe)
    pub fn next_audio_clip_id(&self) -> AudioClipInstanceId {
        self.next_audio_clip_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Add a clip to an audio track (async, fire-and-forget)
    /// Returns the pre-assigned clip instance ID so callers can track the clip without a sync round-trip
    pub fn add_audio_clip(&mut self, track_id: TrackId, pool_index: usize, start_time: f64, duration: f64, offset: f64) -> AudioClipInstanceId {
        let clip_id = self.next_audio_clip_id.fetch_add(1, Ordering::Relaxed);
        let _ = self.command_tx.push(Command::AddAudioClip(track_id, clip_id, pool_index, start_time, duration, offset));
        clip_id
    }

    /// Add a clip to an audio track with a pre-assigned ID (for undo/redo, restoring deleted clips)
    pub fn add_audio_clip_with_id(&mut self, track_id: TrackId, clip_id: AudioClipInstanceId, pool_index: usize, start_time: f64, duration: f64, offset: f64) {
        let _ = self.command_tx.push(Command::AddAudioClip(track_id, clip_id, pool_index, start_time, duration, offset));
    }

    /// Create a new MIDI track
    pub fn create_midi_track(&mut self, name: String) {
        let _ = self.command_tx.push(Command::CreateMidiTrack(name, None));
    }

    /// Add a MIDI clip to the pool without placing it on any track
    /// This is useful for importing MIDI files into a clip library
    pub fn add_midi_clip_to_pool(&mut self, clip: MidiClip) {
        let _ = self.command_tx.push(Command::AddMidiClipToPool(clip));
    }

    /// Create a new audio track synchronously (waits for creation to complete)
    pub fn create_audio_track_sync(&mut self, name: String, parent: Option<TrackId>) -> Result<TrackId, String> {
        if let Err(_) = self.query_tx.push(Query::CreateAudioTrackSync(name, parent)) {
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
    pub fn create_midi_track_sync(&mut self, name: String, parent: Option<TrackId>) -> Result<TrackId, String> {
        if let Err(_) = self.query_tx.push(Query::CreateMidiTrackSync(name, parent)) {
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

    /// Create a new metatrack/group synchronously (waits for creation to complete)
    pub fn create_group_track_sync(&mut self, name: String, parent: Option<TrackId>) -> Result<TrackId, String> {
        if let Err(_) = self.query_tx.push(Query::CreateMetatrackSync(name, parent)) {
            return Err("Failed to send metatrack creation query".to_string());
        }

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(2);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::TrackCreated(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Metatrack creation timeout".to_string())
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

    /// Add a pre-loaded MIDI clip to a track at the given timeline position
    pub fn add_loaded_midi_clip(&mut self, track_id: TrackId, clip: MidiClip, start_time: f64) {
        let _ = self.command_tx.push(Command::AddLoadedMidiClip(track_id, clip, start_time));
    }

    /// Update all notes in a MIDI clip
    pub fn update_midi_clip_notes(&mut self, track_id: TrackId, clip_id: MidiClipId, notes: Vec<(f64, u8, u8, f64)>) {
        let _ = self.command_tx.push(Command::UpdateMidiClipNotes(track_id, clip_id, notes));
    }

    /// Remove a MIDI clip instance from a track (for undo/redo support)
    pub fn remove_midi_clip(&mut self, track_id: TrackId, instance_id: MidiClipInstanceId) {
        let _ = self.command_tx.push(Command::RemoveMidiClip(track_id, instance_id));
    }

    /// Remove an audio clip instance from a track (for undo/redo support)
    pub fn remove_audio_clip(&mut self, track_id: TrackId, instance_id: AudioClipInstanceId) {
        let _ = self.command_tx.push(Command::RemoveAudioClip(track_id, instance_id));
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

    /// Enable or disable the metronome click track
    pub fn set_metronome_enabled(&mut self, enabled: bool) {
        let _ = self.command_tx.push(Command::SetMetronomeEnabled(enabled));
    }

    /// Set project tempo (BPM) and time signature
    pub fn set_tempo(&mut self, bpm: f32, time_signature: (u32, u32)) {
        let _ = self.command_tx.push(Command::SetTempo(bpm, time_signature));
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

    pub fn graph_disconnect_in_template(&mut self, track_id: TrackId, voice_allocator_id: u32, from_node: u32, from_port: usize, to_node: u32, to_port: usize) {
        let _ = self.command_tx.push(Command::GraphDisconnectInTemplate(track_id, voice_allocator_id, from_node, from_port, to_node, to_port));
    }

    pub fn graph_remove_node_from_template(&mut self, track_id: TrackId, voice_allocator_id: u32, node_id: u32) {
        let _ = self.command_tx.push(Command::GraphRemoveNodeFromTemplate(track_id, voice_allocator_id, node_id));
    }

    pub fn graph_set_parameter_in_template(&mut self, track_id: TrackId, voice_allocator_id: u32, node_id: u32, param_id: u32, value: f32) {
        let _ = self.command_tx.push(Command::GraphSetParameterInTemplate(track_id, voice_allocator_id, node_id, param_id, value));
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

    /// Set the UI position of a node in a track's graph
    pub fn graph_set_node_position(&mut self, track_id: TrackId, node_id: u32, x: f32, y: f32) {
        let _ = self.command_tx.push(Command::GraphSetNodePosition(track_id, node_id, x, y));
    }

    pub fn graph_set_node_position_in_template(&mut self, track_id: TrackId, voice_allocator_id: u32, node_id: u32, x: f32, y: f32) {
        let _ = self.command_tx.push(Command::GraphSetNodePositionInTemplate(track_id, voice_allocator_id, node_id, x, y));
    }

    /// Set which node receives MIDI events in a track's instrument graph
    pub fn graph_set_midi_target(&mut self, track_id: TrackId, node_id: u32, enabled: bool) {
        let _ = self.command_tx.push(Command::GraphSetMidiTarget(track_id, node_id, enabled));
    }

    /// Set which node is the audio output in a track's instrument graph
    pub fn graph_set_output_node(&mut self, track_id: TrackId, node_id: u32) {
        let _ = self.command_tx.push(Command::GraphSetOutputNode(track_id, node_id));
    }

    /// Set frontend-only group definitions on a track's graph
    pub fn graph_set_groups(&mut self, track_id: TrackId, groups: Vec<crate::audio::node_graph::preset::SerializedGroup>) {
        let _ = self.command_tx.push(Command::GraphSetGroups(track_id, groups));
    }

    /// Set frontend-only group definitions on a VA template graph
    pub fn graph_set_groups_in_template(&mut self, track_id: TrackId, voice_allocator_id: u32, groups: Vec<crate::audio::node_graph::preset::SerializedGroup>) {
        let _ = self.command_tx.push(Command::GraphSetGroupsInTemplate(track_id, voice_allocator_id, groups));
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

    /// Load a NAM model into an AmpSim node
    pub fn amp_sim_load_model(&mut self, track_id: TrackId, node_id: u32, model_path: String) {
        let _ = self.command_tx.push(Command::AmpSimLoadModel(track_id, node_id, model_path));
    }

    /// Load a sample into a SimpleSampler node
    pub fn sampler_load_sample(&mut self, track_id: TrackId, node_id: u32, file_path: String) {
        let _ = self.command_tx.push(Command::SamplerLoadSample(track_id, node_id, file_path));
    }

    /// Load a sample from the audio pool into a SimpleSampler node
    pub fn sampler_load_from_pool(&mut self, track_id: TrackId, node_id: u32, pool_index: usize) {
        let _ = self.command_tx.push(Command::SamplerLoadFromPool(track_id, node_id, pool_index));
    }

    /// Set the root note for a SimpleSampler node
    pub fn sampler_set_root_note(&mut self, track_id: TrackId, node_id: u32, root_note: u8) {
        let _ = self.command_tx.push(Command::SamplerSetRootNote(track_id, node_id, root_note));
    }

    /// Add a sample layer to a MultiSampler node
    pub fn multi_sampler_add_layer(&mut self, track_id: TrackId, node_id: u32, file_path: String, key_min: u8, key_max: u8, root_key: u8, velocity_min: u8, velocity_max: u8, loop_start: Option<usize>, loop_end: Option<usize>, loop_mode: crate::audio::node_graph::nodes::LoopMode) {
        let _ = self.command_tx.push(Command::MultiSamplerAddLayer(track_id, node_id, file_path, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode));
    }

    /// Add a sample layer from the audio pool to a MultiSampler node
    pub fn multi_sampler_add_layer_from_pool(&mut self, track_id: TrackId, node_id: u32, pool_index: usize, key_min: u8, key_max: u8, root_key: u8) {
        let _ = self.command_tx.push(Command::MultiSamplerAddLayerFromPool(track_id, node_id, pool_index, key_min, key_max, root_key));
    }

    /// Update a MultiSampler layer's configuration
    pub fn multi_sampler_update_layer(&mut self, track_id: TrackId, node_id: u32, layer_index: usize, key_min: u8, key_max: u8, root_key: u8, velocity_min: u8, velocity_max: u8, loop_start: Option<usize>, loop_end: Option<usize>, loop_mode: crate::audio::node_graph::nodes::LoopMode) {
        let _ = self.command_tx.push(Command::MultiSamplerUpdateLayer(track_id, node_id, layer_index, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode));
    }

    /// Remove a layer from a MultiSampler node
    pub fn multi_sampler_remove_layer(&mut self, track_id: TrackId, node_id: u32, layer_index: usize) {
        let _ = self.command_tx.push(Command::MultiSamplerRemoveLayer(track_id, node_id, layer_index));
    }

    /// Clear all layers from a MultiSampler node
    pub fn multi_sampler_clear_layers(&mut self, track_id: TrackId, node_id: u32) {
        let _ = self.command_tx.push(Command::MultiSamplerClearLayers(track_id, node_id));
    }

    /// Set the full subtrack list for a metatrack's mixing graph (rebuilds the graph)
    pub fn set_metatrack_subtrack_graph(&mut self, track_id: TrackId, subtracks: Vec<(TrackId, String)>) {
        let _ = self.command_tx.push(Command::SetMetatrackSubtrackGraph(track_id, subtracks));
    }

    /// Add a subtrack port to a metatrack's mixing graph
    pub fn add_metatrack_subtrack(&mut self, track_id: TrackId, subtrack_id: TrackId, name: String) {
        let _ = self.command_tx.push(Command::AddMetatrackSubtrack(track_id, subtrack_id, name));
    }

    /// Remove a subtrack port from a metatrack's mixing graph
    pub fn remove_metatrack_subtrack(&mut self, track_id: TrackId, subtrack_id: TrackId) {
        let _ = self.command_tx.push(Command::RemoveMetatrackSubtrack(track_id, subtrack_id));
    }

    /// Re-associate backend TrackIds with SubtrackInputsNode slots (called after project load)
    pub fn update_metatrack_subtrack_ids(&mut self, track_id: TrackId, subtracks: Vec<(TrackId, String)>) {
        let _ = self.command_tx.push(Command::UpdateMetatrackSubtrackIds(track_id, subtracks));
    }

    /// Set the graph_is_default flag on a track (command, processed async)
    pub fn set_graph_is_default(&mut self, track_id: TrackId, value: bool) {
        let _ = self.command_tx.push(Command::SetGraphIsDefault(track_id, value));
    }

    /// Query whether a track's graph is the auto-generated default (synchronous)
    pub fn get_graph_is_default(&mut self, track_id: TrackId) -> bool {
        if let Err(_) = self.query_tx.push(Query::GetGraphIsDefault(track_id)) {
            return false;
        }
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);
        while start.elapsed() < timeout {
            if let Ok(QueryResponse::GraphIsDefault(v)) = self.query_response_rx.pop() {
                return v;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        false
    }

    /// Send a synchronous query and wait for the response
    /// This blocks until the audio thread processes the query
    /// Generic method that works with any Query/QueryResponse pair
    pub fn send_query(&mut self, query: Query) -> Result<QueryResponse, String> {
        // Send query
        if let Err(_) = self.query_tx.push(query) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(500);

        while start.elapsed() < timeout {
            if let Ok(response) = self.query_response_rx.pop() {
                return Ok(response);
            }
            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_micros(100));
        }

        Err("Query timeout".to_string())
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

    /// Query oscilloscope data from a node inside a VoiceAllocator's best voice
    pub fn query_voice_oscilloscope_data(&mut self, track_id: TrackId, va_node_id: u32, inner_node_id: u32, sample_count: usize) -> Result<crate::command::OscilloscopeData, String> {
        if let Err(_) = self.query_tx.push(Query::GetVoiceOscilloscopeData(track_id, va_node_id, inner_node_id, sample_count)) {
            return Err("Failed to send query - queue full".to_string());
        }

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(100);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::OscilloscopeData(result)) = self.query_response_rx.pop() {
                return result;
            }
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

        // Wait for response (with shorter timeout to avoid blocking UI during export)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(50);

        while start.elapsed() < timeout {
            if let Ok(response) = self.query_response_rx.pop() {
                match response {
                    QueryResponse::PoolWaveform(result) => return result,
                    QueryResponse::AudioExported(result) => {
                        // Cache for poll_export_completion()
                        println!("💾 [CONTROLLER] Caching AudioExported response from get_pool_waveform");
                        self.cached_export_response = Some(result);
                    }
                    _ => {} // Discard other responses
                }
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

        // Wait for response (with shorter timeout to avoid blocking UI during export)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(50);

        while start.elapsed() < timeout {
            if let Ok(response) = self.query_response_rx.pop() {
                match response {
                    QueryResponse::PoolFileInfo(result) => return result,
                    QueryResponse::AudioExported(result) => {
                        // Cache for poll_export_completion()
                        println!("💾 [CONTROLLER] Caching AudioExported response from get_pool_file_info");
                        self.cached_export_response = Some(result);
                    }
                    _ => {} // Discard other responses
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Query timeout".to_string())
    }

    /// Get raw audio samples from pool (samples, sample_rate, channels)
    pub fn get_pool_audio_samples(&mut self, pool_index: usize) -> Result<(Vec<f32>, u32, u32), String> {
        if let Err(_) = self.query_tx.push(Query::GetPoolAudioSamples(pool_index)) {
            return Err("Failed to send query - queue full".to_string());
        }

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5); // Longer timeout for large audio data

        while start.elapsed() < timeout {
            if let Ok(response) = self.query_response_rx.pop() {
                match response {
                    QueryResponse::PoolAudioSamples(result) => return result,
                    QueryResponse::AudioExported(result) => {
                        self.cached_export_response = Some(result);
                    }
                    _ => {}
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        Err("Query timeout".to_string())
    }

    /// Request waveform chunks to be generated
    /// This is an asynchronous command - chunks will be returned via WaveformChunksReady events
    pub fn generate_waveform_chunks(
        &mut self,
        pool_index: usize,
        detail_level: u8,
        chunk_indices: Vec<u32>,
        priority: u8,
    ) -> Result<(), String> {
        let command = Command::GenerateWaveformChunks {
            pool_index,
            detail_level,
            chunk_indices,
            priority,
        };

        if let Err(_) = self.command_tx.push(command) {
            return Err("Failed to send command - queue full".to_string());
        }

        Ok(())
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

    /// Start an audio export (non-blocking)
    ///
    /// Sends the export query to the audio thread and returns immediately.
    /// Use `poll_export_completion()` to check for completion.
    pub fn start_export_audio<P: AsRef<std::path::Path>>(&mut self, settings: &crate::audio::ExportSettings, output_path: P) -> Result<(), String> {
        // Send export query
        if let Err(_) = self.query_tx.push(Query::ExportAudio(settings.clone(), output_path.as_ref().to_path_buf())) {
            return Err("Failed to send export query - queue full".to_string());
        }
        Ok(())
    }

    /// Poll for export completion (non-blocking)
    ///
    /// Returns:
    /// - `Ok(Some(result))` if export completed (result may be Ok or Err)
    /// - `Ok(None)` if export is still in progress
    /// - `Err` should not happen in normal operation
    pub fn poll_export_completion(&mut self) -> Result<Option<Result<(), String>>, String> {
        // Check if we have a cached response from another query method
        if let Some(result) = self.cached_export_response.take() {
            println!("✅ [CONTROLLER] Found cached AudioExported response!");
            return Ok(Some(result));
        }

        // Keep popping responses until we find AudioExported or queue is empty
        while let Ok(response) = self.query_response_rx.pop() {
            println!("📥 [CONTROLLER] Received response: {:?}", std::mem::discriminant(&response));
            if let QueryResponse::AudioExported(result) = response {
                println!("✅ [CONTROLLER] Found AudioExported response!");
                return Ok(Some(result));
            }
            // Discard other query responses (they're for synchronous queries)
            println!("⏭️  [CONTROLLER] Skipping non-export response");
        }
        Ok(None)
    }

    /// Export audio to a file (blocking)
    ///
    /// This is a convenience method that calls start_export_audio and waits for completion.
    /// For non-blocking export with progress updates, use start_export_audio() and poll_export_completion().
    pub fn export_audio<P: AsRef<std::path::Path>>(&mut self, settings: &crate::audio::ExportSettings, output_path: P) -> Result<(), String> {
        self.start_export_audio(settings, &output_path)?;

        // Wait for response (with longer timeout since export can take a while)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(300); // 5 minute timeout for export

        while start.elapsed() < timeout {
            if let Some(result) = self.poll_export_completion()? {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        Err("Export timeout".to_string())
    }

    /// Get a clone of the current project for serialization
    pub fn get_project(&mut self) -> Result<crate::audio::project::Project, String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::GetProject) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::ProjectRetrieved(result)) = self.query_response_rx.pop() {
                return result.map(|boxed| *boxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }

    /// Set the project (replaces current project state)
    pub fn set_project(&mut self, project: crate::audio::project::Project) -> Result<(), String> {
        // Send query
        if let Err(_) = self.query_tx.push(Query::SetProject(Box::new(project))) {
            return Err("Failed to send query - queue full".to_string());
        }

        // Wait for response (with timeout)
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(10); // Longer timeout for loading project

        while start.elapsed() < timeout {
            if let Ok(QueryResponse::ProjectSet(result)) = self.query_response_rx.pop() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Err("Query timeout".to_string())
    }
}
