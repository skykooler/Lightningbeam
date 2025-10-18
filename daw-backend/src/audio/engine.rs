use crate::audio::clip::ClipId;
use crate::audio::pool::AudioPool;
use crate::audio::track::{Track, TrackId};
use crate::command::{AudioEvent, Command};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Audio engine for Phase 4: timeline with clips and audio pool
pub struct Engine {
    tracks: Vec<Track>,
    audio_pool: AudioPool,
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

    // Mix buffer for combining tracks
    mix_buffer: Vec<f32>,
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

        Self {
            tracks: Vec::new(),
            audio_pool: AudioPool::new(),
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
        }
    }

    /// Add a track to the engine
    pub fn add_track(&mut self, track: Track) -> TrackId {
        let id = track.id;
        self.tracks.push(track);
        id
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

            // Clear mix buffer
            self.mix_buffer.fill(0.0);

            // Convert playhead from samples to seconds for timeline-based rendering
            let playhead_seconds = self.playhead as f64 / (self.sample_rate as f64 * self.channels as f64);

            // Check if any track is soloed
            let any_solo = self.tracks.iter().any(|t| t.solo);

            // Mix all active tracks using timeline-based rendering
            for track in &self.tracks {
                if track.is_active(any_solo) {
                    track.render(
                        &mut self.mix_buffer,
                        &self.audio_pool,
                        playhead_seconds,
                        self.sample_rate,
                        self.channels,
                    );
                }
            }

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
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.set_volume(volume);
                }
            }
            Command::SetTrackMute(track_id, muted) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.set_muted(muted);
                }
            }
            Command::SetTrackSolo(track_id, solo) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    track.set_solo(solo);
                }
            }
            Command::MoveClip(track_id, clip_id, new_start_time) => {
                if let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) {
                    if let Some(clip) = track.clips.iter_mut().find(|c| c.id == clip_id) {
                        clip.start_time = new_start_time;
                    }
                }
            }
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
        self.tracks.len()
    }
}

/// Controller for the engine that can be used from the UI thread
pub struct EngineController {
    command_tx: rtrb::Producer<Command>,
    playhead: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u32,
}

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

    /// Get current playhead position in samples
    pub fn get_playhead_samples(&self) -> u64 {
        self.playhead.load(Ordering::Relaxed)
    }

    /// Get current playhead position in seconds
    pub fn get_playhead_seconds(&self) -> f64 {
        let samples = self.playhead.load(Ordering::Relaxed);
        samples as f64 / (self.sample_rate as f64 * self.channels as f64)
    }
}
