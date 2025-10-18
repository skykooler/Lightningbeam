/// Clip ID type
pub type ClipId = u32;

/// Audio clip that references data in the AudioPool
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: ClipId,
    pub audio_pool_index: usize,
    pub start_time: f64,        // Position on timeline in seconds
    pub duration: f64,          // Clip duration in seconds
    pub offset: f64,            // Offset into audio file in seconds
    pub gain: f32,              // Clip-level gain
}

impl Clip {
    /// Create a new clip
    pub fn new(
        id: ClipId,
        audio_pool_index: usize,
        start_time: f64,
        duration: f64,
        offset: f64,
    ) -> Self {
        Self {
            id,
            audio_pool_index,
            start_time,
            duration,
            offset,
            gain: 1.0,
        }
    }

    /// Check if this clip is active at a given timeline position
    pub fn is_active_at(&self, time_seconds: f64) -> bool {
        let clip_end = self.start_time + self.duration;
        time_seconds >= self.start_time && time_seconds < clip_end
    }

    /// Get the end time of this clip on the timeline
    pub fn end_time(&self) -> f64 {
        self.start_time + self.duration
    }

    /// Set clip gain
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.max(0.0);
    }
}
