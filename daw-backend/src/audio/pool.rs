use std::path::PathBuf;
use std::f32::consts::PI;

/// Windowed sinc interpolation for high-quality time stretching
/// This is stateless and can handle arbitrary fractional positions
#[inline]
fn sinc(x: f32) -> f32 {
    if x.abs() < 1e-5 {
        1.0
    } else {
        let px = PI * x;
        px.sin() / px
    }
}

/// Blackman window function
#[inline]
fn blackman_window(x: f32, width: f32) -> f32 {
    if x.abs() > width {
        0.0
    } else {
        let a0 = 0.42;
        let a1 = 0.5;
        let a2 = 0.08;
        // Map x from [-width, width] to [0, 1] for proper Blackman window evaluation
        let n = (x / width + 1.0) / 2.0;
        a0 - a1 * (2.0 * PI * n).cos() + a2 * (4.0 * PI * n).cos()
    }
}

/// High-quality windowed sinc interpolation
/// Uses a 32-tap windowed sinc kernel for smooth, artifact-free interpolation
/// frac: fractional position to interpolate at (0.0 to 1.0)
/// samples: array of samples centered around the target position
#[inline]
fn windowed_sinc_interpolate(samples: &[f32], frac: f32) -> f32 {
    let mut result = 0.0;
    let kernel_size = samples.len();
    let half_kernel = (kernel_size / 2) as f32;

    for i in 0..kernel_size {
        // Distance from interpolation point
        // samples[half_kernel] is at position 0, we want to interpolate at position frac
        let x = frac + half_kernel - (i as f32);
        let sinc_val = sinc(x);
        let window_val = blackman_window(x, half_kernel);
        result += samples[i] * sinc_val * window_val;
    }

    result
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

    /// Render audio from a file in the pool with high-quality windowed sinc interpolation
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
        let Some(audio_file) = self.files.get(pool_index) else {
            return 0;
        };

        let src_channels = audio_file.channels as usize;
        let dst_channels = engine_channels as usize;
        let output_frames = output.len() / dst_channels;

        // Calculate starting position in source with fractional precision
        let src_start_position = start_time_seconds * audio_file.sample_rate as f64;

        // Sample rate conversion ratio
        let rate_ratio = audio_file.sample_rate as f64 / engine_sample_rate as f64;

        // Kernel size for windowed sinc (32 taps = high quality, good performance)
        const KERNEL_SIZE: usize = 32;
        const HALF_KERNEL: usize = KERNEL_SIZE / 2;

        let mut rendered_frames = 0;

        // Render frame by frame with windowed sinc interpolation
        for output_frame in 0..output_frames {
            // Calculate exact fractional position in source
            let src_position = src_start_position + (output_frame as f64 * rate_ratio);
            let src_frame = src_position.floor() as i32;
            let frac = (src_position - src_frame as f64) as f32;

            // Check if we've gone past the end of the audio file
            if src_frame < 0 || src_frame as usize >= audio_file.frames as usize {
                break;
            }

            // Interpolate each channel
            for dst_ch in 0..dst_channels {
                let sample = if src_channels == dst_channels {
                    // Direct channel mapping
                    let ch_offset = dst_ch;

                    // Extract channel samples for interpolation
                    let mut channel_samples = Vec::with_capacity(KERNEL_SIZE);
                    for i in -(HALF_KERNEL as i32)..(HALF_KERNEL as i32) {
                        let idx = src_frame + i;
                        if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                            let sample_idx = (idx as usize) * src_channels + ch_offset;
                            channel_samples.push(audio_file.data[sample_idx]);
                        } else {
                            channel_samples.push(0.0);
                        }
                    }

                    windowed_sinc_interpolate(&channel_samples, frac)

                } else if src_channels == 1 && dst_channels > 1 {
                    // Mono to stereo - duplicate
                    let mut channel_samples = Vec::with_capacity(KERNEL_SIZE);
                    for i in -(HALF_KERNEL as i32)..(HALF_KERNEL as i32) {
                        let idx = src_frame + i;
                        if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                            channel_samples.push(audio_file.data[idx as usize]);
                        } else {
                            channel_samples.push(0.0);
                        }
                    }

                    windowed_sinc_interpolate(&channel_samples, frac)

                } else if src_channels > 1 && dst_channels == 1 {
                    // Multi-channel to mono - average all source channels
                    let mut sum = 0.0;

                    for src_ch in 0..src_channels {
                        let mut channel_samples = Vec::with_capacity(KERNEL_SIZE);
                        for i in -(HALF_KERNEL as i32)..(HALF_KERNEL as i32) {
                            let idx = src_frame + i;
                            if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                                let sample_idx = (idx as usize) * src_channels + src_ch;
                                channel_samples.push(audio_file.data[sample_idx]);
                            } else {
                                channel_samples.push(0.0);
                            }
                        }
                        sum += windowed_sinc_interpolate(&channel_samples, frac);
                    }

                    sum / src_channels as f32

                } else {
                    // Mismatched channels - use modulo mapping
                    let src_ch = dst_ch % src_channels;

                    let mut channel_samples = Vec::with_capacity(KERNEL_SIZE);
                    for i in -(HALF_KERNEL as i32)..(HALF_KERNEL as i32) {
                        let idx = src_frame + i;
                        if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                            let sample_idx = (idx as usize) * src_channels + src_ch;
                            channel_samples.push(audio_file.data[sample_idx]);
                        } else {
                            channel_samples.push(0.0);
                        }
                    }

                    windowed_sinc_interpolate(&channel_samples, frac)
                };

                // Mix into output with gain
                let output_idx = output_frame * dst_channels + dst_ch;
                output[output_idx] += sample * gain;
            }

            rendered_frames += 1;
        }

        rendered_frames * dst_channels
    }
}

impl Default for AudioPool {
    fn default() -> Self {
        Self::new()
    }
}
