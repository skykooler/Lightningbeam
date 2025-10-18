use super::clip::Clip;
use super::pool::AudioPool;

/// Track ID type
pub type TrackId = u32;

/// Audio track for Phase 4 with clips
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub clips: Vec<Clip>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

impl Track {
    /// Create a new track with default settings
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            name,
            clips: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
        }
    }

    /// Add a clip to this track
    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
    }

    /// Set track volume (0.0 = silence, 1.0 = unity gain, >1.0 = amplification)
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.max(0.0);
    }

    /// Set mute state
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Set solo state
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    /// Check if this track should be audible given the solo state of all tracks
    pub fn is_active(&self, any_solo: bool) -> bool {
        !self.muted && (!any_solo || self.solo)
    }

    /// Render this track into the output buffer at a given timeline position
    /// Returns the number of samples actually rendered
    pub fn render(
        &self,
        output: &mut [f32],
        pool: &AudioPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        let mut rendered = 0;

        // Render all active clips
        for clip in &self.clips {
            // Check if clip overlaps with current buffer time range
            if clip.start_time < buffer_end_seconds && clip.end_time() > playhead_seconds {
                rendered += self.render_clip(
                    clip,
                    output,
                    pool,
                    playhead_seconds,
                    sample_rate,
                    channels,
                );
            }
        }

        rendered
    }

    /// Render a single clip into the output buffer
    fn render_clip(
        &self,
        clip: &Clip,
        output: &mut [f32],
        pool: &AudioPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Determine the time range we need to render (intersection of buffer and clip)
        let render_start_seconds = playhead_seconds.max(clip.start_time);
        let render_end_seconds = buffer_end_seconds.min(clip.end_time());

        // If no overlap, return early
        if render_start_seconds >= render_end_seconds {
            return 0;
        }

        // Calculate offset into the output buffer (in interleaved samples)
        let output_offset_seconds = render_start_seconds - playhead_seconds;
        let output_offset_samples = (output_offset_seconds * sample_rate as f64 * channels as f64) as usize;

        // Calculate position within the clip's audio file (in seconds)
        let clip_position_seconds = render_start_seconds - clip.start_time + clip.offset;

        // Calculate how many samples to render in the output
        let render_duration_seconds = render_end_seconds - render_start_seconds;
        let samples_to_render = (render_duration_seconds * sample_rate as f64 * channels as f64) as usize;
        let samples_to_render = samples_to_render.min(output.len() - output_offset_samples);

        // Get the slice of output buffer to write to
        if output_offset_samples + samples_to_render > output.len() {
            return 0;
        }

        let output_slice = &mut output[output_offset_samples..output_offset_samples + samples_to_render];

        // Calculate combined gain
        let combined_gain = clip.gain * self.volume;

        // Render from pool with sample rate conversion
        // Pass the time position in seconds, let the pool handle sample rate conversion
        pool.render_from_file(
            clip.audio_pool_index,
            output_slice,
            clip_position_seconds,
            combined_gain,
            sample_rate,
            channels,
        )
    }
}
