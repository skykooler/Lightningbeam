use std::path::PathBuf;

/// Cubic Hermite interpolation for smooth resampling
/// p0, p1, p2, p3 are four consecutive samples
/// x is the fractional position between p1 and p2 (0.0 to 1.0)
#[inline]
fn hermite_interpolate(p0: f32, p1: f32, p2: f32, p3: f32, x: f32) -> f32 {
    // Hermite basis functions for smooth interpolation
    let c0 = p1;
    let c1 = 0.5 * (p2 - p0);
    let c2 = p0 - 2.5 * p1 + 2.0 * p2 - 0.5 * p3;
    let c3 = 0.5 * (p3 - p0) + 1.5 * (p1 - p2);

    ((c3 * x + c2) * x + c1) * x + c0
}

/// Audio file stored in the pool
#[derive(Debug, Clone)]
pub struct AudioFile {
    pub path: PathBuf,
    pub data: Vec<f32>,         // Interleaved samples
    pub channels: u32,
    pub sample_rate: u32,
    pub frames: u64,
}

impl AudioFile {
    /// Create a new AudioFile
    pub fn new(path: PathBuf, data: Vec<f32>, channels: u32, sample_rate: u32) -> Self {
        let frames = (data.len() / channels as usize) as u64;
        Self {
            path,
            data,
            channels,
            sample_rate,
            frames,
        }
    }

    /// Get duration in seconds
    pub fn duration_seconds(&self) -> f64 {
        self.frames as f64 / self.sample_rate as f64
    }
}

/// Pool of shared audio files
pub struct AudioPool {
    files: Vec<AudioFile>,
}

impl AudioPool {
    /// Create a new empty audio pool
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
        }
    }

    /// Add an audio file to the pool and return its index
    pub fn add_file(&mut self, file: AudioFile) -> usize {
        let index = self.files.len();
        self.files.push(file);
        index
    }

    /// Get an audio file by index
    pub fn get_file(&self, index: usize) -> Option<&AudioFile> {
        self.files.get(index)
    }

    /// Get number of files in the pool
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Render audio from a file in the pool with sample rate and channel conversion
    /// start_time_seconds: position in the audio file to start reading from (in seconds)
    /// Returns the number of samples actually rendered
    pub fn render_from_file(
        &self,
        pool_index: usize,
        output: &mut [f32],
        start_time_seconds: f64,
        gain: f32,
        engine_sample_rate: u32,
        engine_channels: u32,
    ) -> usize {
        if let Some(audio_file) = self.files.get(pool_index) {
            // Calculate starting frame position in the source file (frame = one sample per channel)
            let src_start_frame = start_time_seconds * audio_file.sample_rate as f64;

            // Calculate sample rate conversion ratio (frames)
            let rate_ratio = audio_file.sample_rate as f64 / engine_sample_rate as f64;

            let src_channels = audio_file.channels;
            let dst_channels = engine_channels;

            // Render frame by frame
            let output_frames = output.len() / dst_channels as usize;
            let mut rendered_frames = 0;

            for frame_idx in 0..output_frames {
                // Calculate the corresponding frame in the source file
                let src_frame_pos = src_start_frame + (frame_idx as f64 * rate_ratio);
                let src_frame_idx = src_frame_pos as usize;

                // Check bounds
                if src_frame_idx >= audio_file.frames as usize {
                    break;
                }

                // Calculate source sample index (interleaved)
                let src_sample_idx = src_frame_idx * src_channels as usize;

                // Check bounds for interpolation
                if src_sample_idx + src_channels as usize > audio_file.data.len() {
                    break;
                }

                // Cubic Hermite interpolation for high-quality time stretching
                let frac = (src_frame_pos - src_frame_idx as f64) as f32;

                // We need 4 points for cubic interpolation: p0, p1, p2, p3
                // where we interpolate between p1 and p2
                let p1_frame = src_frame_idx;
                let p0_frame = if p1_frame > 0 { p1_frame - 1 } else { p1_frame };
                let p2_frame = p1_frame + 1;
                let p3_frame = p1_frame + 2;

                let p0_idx = p0_frame * src_channels as usize;
                let p1_idx = p1_frame * src_channels as usize;
                let p2_idx = p2_frame * src_channels as usize;
                let p3_idx = p3_frame * src_channels as usize;

                let can_interpolate = p3_idx + src_channels as usize <= audio_file.data.len();

                // Read and convert channels
                for dst_ch in 0..dst_channels {
                    let sample = if src_channels == dst_channels {
                        // Same number of channels - direct mapping
                        let ch = dst_ch as usize;
                        if can_interpolate && frac > 0.0 {
                            let p0 = audio_file.data[p0_idx + ch];
                            let p1 = audio_file.data[p1_idx + ch];
                            let p2 = audio_file.data[p2_idx + ch];
                            let p3 = audio_file.data[p3_idx + ch];
                            hermite_interpolate(p0, p1, p2, p3, frac)
                        } else {
                            audio_file.data[p1_idx + ch]
                        }
                    } else if src_channels == 1 && dst_channels > 1 {
                        // Mono to multi-channel - duplicate to all channels
                        if can_interpolate && frac > 0.0 {
                            let p0 = audio_file.data[p0_idx];
                            let p1 = audio_file.data[p1_idx];
                            let p2 = audio_file.data[p2_idx];
                            let p3 = audio_file.data[p3_idx];
                            hermite_interpolate(p0, p1, p2, p3, frac)
                        } else {
                            audio_file.data[p1_idx]
                        }
                    } else if src_channels > 1 && dst_channels == 1 {
                        // Multi-channel to mono - average all source channels
                        let mut sum = 0.0f32;
                        for src_ch in 0..src_channels {
                            let ch = src_ch as usize;
                            let s = if can_interpolate && frac > 0.0 {
                                let p0 = audio_file.data[p0_idx + ch];
                                let p1 = audio_file.data[p1_idx + ch];
                                let p2 = audio_file.data[p2_idx + ch];
                                let p3 = audio_file.data[p3_idx + ch];
                                hermite_interpolate(p0, p1, p2, p3, frac)
                            } else {
                                audio_file.data[p1_idx + ch]
                            };
                            sum += s;
                        }
                        sum / src_channels as f32
                    } else {
                        // Mismatched channels - use modulo for simple mapping
                        let src_ch = (dst_ch % src_channels) as usize;
                        if can_interpolate && frac > 0.0 {
                            let p0 = audio_file.data[p0_idx + src_ch];
                            let p1 = audio_file.data[p1_idx + src_ch];
                            let p2 = audio_file.data[p2_idx + src_ch];
                            let p3 = audio_file.data[p3_idx + src_ch];
                            hermite_interpolate(p0, p1, p2, p3, frac)
                        } else {
                            audio_file.data[p1_idx + src_ch]
                        }
                    };

                    // Mix into output with gain
                    let output_idx = frame_idx * dst_channels as usize + dst_ch as usize;
                    output[output_idx] += sample * gain;
                }

                rendered_frames += 1;
            }

            rendered_frames * dst_channels as usize
        } else {
            0
        }
    }
}

impl Default for AudioPool {
    fn default() -> Self {
        Self::new()
    }
}
