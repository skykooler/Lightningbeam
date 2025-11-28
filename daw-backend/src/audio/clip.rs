/// Audio clip instance ID type
pub type AudioClipInstanceId = u32;

/// Type alias for backwards compatibility
pub type ClipId = AudioClipInstanceId;

/// Audio clip instance that references content in the AudioClipPool
///
/// This represents a placed instance of audio content on the timeline.
/// The actual audio data is stored in the AudioClipPool and referenced by `audio_pool_index`.
///
/// ## Timing Model
/// - `internal_start` / `internal_end`: Define the region of the source audio to play (trimming)
/// - `external_start` / `external_duration`: Define where the clip appears on the timeline and how long
///
/// ## Looping
/// If `external_duration` is greater than `internal_end - internal_start`,
/// the clip will seamlessly loop back to `internal_start` when it reaches `internal_end`.
#[derive(Debug, Clone)]
pub struct AudioClipInstance {
    pub id: AudioClipInstanceId,
    pub audio_pool_index: usize,

    /// Start position within the audio content (seconds)
    pub internal_start: f64,
    /// End position within the audio content (seconds)
    pub internal_end: f64,

    /// Start position on the timeline (seconds)
    pub external_start: f64,
    /// Duration on the timeline (seconds) - can be longer than internal duration for looping
    pub external_duration: f64,

    /// Clip-level gain
    pub gain: f32,
}

/// Type alias for backwards compatibility
pub type Clip = AudioClipInstance;

impl AudioClipInstance {
    /// Create a new audio clip instance
    pub fn new(
        id: AudioClipInstanceId,
        audio_pool_index: usize,
        internal_start: f64,
        internal_end: f64,
        external_start: f64,
        external_duration: f64,
    ) -> Self {
        Self {
            id,
            audio_pool_index,
            internal_start,
            internal_end,
            external_start,
            external_duration,
            gain: 1.0,
        }
    }

    /// Create a clip instance from legacy parameters (for backwards compatibility)
    /// Maps old start_time/duration/offset to new timing model
    pub fn from_legacy(
        id: AudioClipInstanceId,
        audio_pool_index: usize,
        start_time: f64,
        duration: f64,
        offset: f64,
    ) -> Self {
        Self {
            id,
            audio_pool_index,
            internal_start: offset,
            internal_end: offset + duration,
            external_start: start_time,
            external_duration: duration,
            gain: 1.0,
        }
    }

    /// Check if this clip instance is active at a given timeline position
    pub fn is_active_at(&self, time_seconds: f64) -> bool {
        time_seconds >= self.external_start && time_seconds < self.external_end()
    }

    /// Get the end time of this clip instance on the timeline
    pub fn external_end(&self) -> f64 {
        self.external_start + self.external_duration
    }

    /// Get the end time of this clip instance on the timeline
    /// (Alias for external_end(), for backwards compatibility)
    pub fn end_time(&self) -> f64 {
        self.external_end()
    }

    /// Get the start time on the timeline
    /// (Alias for external_start, for backwards compatibility)
    pub fn start_time(&self) -> f64 {
        self.external_start
    }

    /// Get the internal (content) duration
    pub fn internal_duration(&self) -> f64 {
        self.internal_end - self.internal_start
    }

    /// Check if this clip instance loops
    pub fn is_looping(&self) -> bool {
        self.external_duration > self.internal_duration()
    }

    /// Get the position within the audio content for a given timeline position
    /// Returns None if the timeline position is outside this clip instance
    /// Handles looping automatically
    pub fn get_content_position(&self, timeline_pos: f64) -> Option<f64> {
        if timeline_pos < self.external_start || timeline_pos >= self.external_end() {
            return None;
        }

        let relative_pos = timeline_pos - self.external_start;
        let internal_duration = self.internal_duration();

        if internal_duration <= 0.0 {
            return None;
        }

        // Wrap around for looping
        let content_offset = relative_pos % internal_duration;
        Some(self.internal_start + content_offset)
    }

    /// Set clip gain
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.max(0.0);
    }
}
