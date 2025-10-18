use std::path::PathBuf;

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

                // Linear interpolation for better quality
                let frac = src_frame_pos - src_frame_idx as f64;
                let next_frame_idx = src_frame_idx + 1;
                let next_sample_idx = next_frame_idx * src_channels as usize;
                let can_interpolate = next_sample_idx + src_channels as usize <= audio_file.data.len() && frac > 0.0;

                // Read and convert channels
                for dst_ch in 0..dst_channels {
                    let sample = if src_channels == dst_channels {
                        // Same number of channels - direct mapping
                        let ch = dst_ch as usize;
                        let s0 = audio_file.data[src_sample_idx + ch];
                        if can_interpolate {
                            let s1 = audio_file.data[next_sample_idx + ch];
                            s0 + (s1 - s0) * frac as f32
                        } else {
                            s0
                        }
                    } else if src_channels == 1 && dst_channels > 1 {
                        // Mono to multi-channel - duplicate to all channels
                        let s0 = audio_file.data[src_sample_idx];
                        if can_interpolate {
                            let s1 = audio_file.data[next_sample_idx];
                            s0 + (s1 - s0) * frac as f32
                        } else {
                            s0
                        }
                    } else if src_channels > 1 && dst_channels == 1 {
                        // Multi-channel to mono - average all source channels
                        let mut sum = 0.0f32;
                        for src_ch in 0..src_channels {
                            let s0 = audio_file.data[src_sample_idx + src_ch as usize];
                            let s = if can_interpolate {
                                let s1 = audio_file.data[next_sample_idx + src_ch as usize];
                                s0 + (s1 - s0) * frac as f32
                            } else {
                                s0
                            };
                            sum += s;
                        }
                        sum / src_channels as f32
                    } else {
                        // Mismatched channels - use modulo for simple mapping
                        let src_ch = (dst_ch % src_channels) as usize;
                        let s0 = audio_file.data[src_sample_idx + src_ch];
                        if can_interpolate {
                            let s1 = audio_file.data[next_sample_idx + src_ch];
                            s0 + (s1 - s0) * frac as f32
                        } else {
                            s0
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
