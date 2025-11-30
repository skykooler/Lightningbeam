use std::path::{Path, PathBuf};
use std::f32::consts::PI;
use serde::{Deserialize, Serialize};

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

    /// Generate a waveform overview with the specified number of peaks
    /// This creates a downsampled representation suitable for timeline visualization
    pub fn generate_waveform_overview(&self, target_peaks: usize) -> Vec<crate::io::WaveformPeak> {
        self.generate_waveform_overview_range(0, self.frames as usize, target_peaks)
    }

    /// Generate a waveform overview for a specific range of frames
    ///
    /// # Arguments
    /// * `start_frame` - Starting frame index (0-based)
    /// * `end_frame` - Ending frame index (exclusive)
    /// * `target_peaks` - Desired number of peaks to generate
    pub fn generate_waveform_overview_range(
        &self,
        start_frame: usize,
        end_frame: usize,
        target_peaks: usize,
    ) -> Vec<crate::io::WaveformPeak> {
        if self.frames == 0 || target_peaks == 0 {
            return Vec::new();
        }

        let total_frames = self.frames as usize;
        let start_frame = start_frame.min(total_frames);
        let end_frame = end_frame.min(total_frames);

        if start_frame >= end_frame {
            return Vec::new();
        }

        let range_frames = end_frame - start_frame;
        let frames_per_peak = (range_frames / target_peaks).max(1);
        let actual_peaks = (range_frames + frames_per_peak - 1) / frames_per_peak;

        let mut peaks = Vec::with_capacity(actual_peaks);

        for peak_idx in 0..actual_peaks {
            let peak_start = start_frame + peak_idx * frames_per_peak;
            let peak_end = (start_frame + (peak_idx + 1) * frames_per_peak).min(end_frame);

            let mut min = 0.0f32;
            let mut max = 0.0f32;

            // Scan all samples in this window
            for frame_idx in peak_start..peak_end {
                // For multi-channel audio, combine all channels
                for ch in 0..self.channels as usize {
                    let sample_idx = frame_idx * self.channels as usize + ch;
                    if sample_idx < self.data.len() {
                        let sample = self.data[sample_idx];
                        min = min.min(sample);
                        max = max.max(sample);
                    }
                }
            }

            peaks.push(crate::io::WaveformPeak { min, max });
        }

        peaks
    }
}

/// Pool of shared audio files (audio clip content)
pub struct AudioClipPool {
    files: Vec<AudioFile>,
}

/// Type alias for backwards compatibility
pub type AudioPool = AudioClipPool;

impl AudioClipPool {
    /// Create a new empty audio clip pool
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
        }
    }

    /// Get the number of files in the pool
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// Check if the pool is empty
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Get file info for waveform generation (duration, sample_rate, channels)
    pub fn get_file_info(&self, pool_index: usize) -> Option<(f64, u32, u32)> {
        self.files.get(pool_index).map(|file| {
            (file.duration_seconds(), file.sample_rate, file.channels)
        })
    }

    /// Generate waveform overview for a file in the pool
    pub fn generate_waveform(&self, pool_index: usize, target_peaks: usize) -> Option<Vec<crate::io::WaveformPeak>> {
        self.files.get(pool_index).map(|file| {
            file.generate_waveform_overview(target_peaks)
        })
    }

    /// Generate waveform overview for a specific range of a file in the pool
    ///
    /// # Arguments
    /// * `pool_index` - Index of the file in the pool
    /// * `start_frame` - Starting frame index (0-based)
    /// * `end_frame` - Ending frame index (exclusive)
    /// * `target_peaks` - Desired number of peaks to generate
    pub fn generate_waveform_range(
        &self,
        pool_index: usize,
        start_frame: usize,
        end_frame: usize,
        target_peaks: usize,
    ) -> Option<Vec<crate::io::WaveformPeak>> {
        self.files.get(pool_index).map(|file| {
            file.generate_waveform_overview_range(start_frame, end_frame, target_peaks)
        })
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

impl Default for AudioClipPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Embedded audio data stored as base64 in the project file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedAudioData {
    /// Base64-encoded audio data
    pub data_base64: String,
    /// Original file format (wav, mp3, etc.)
    pub format: String,
}

/// Serializable audio pool entry for project save/load
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioPoolEntry {
    /// Index in the audio pool
    pub pool_index: usize,
    /// Original filename
    pub name: String,
    /// Path relative to project file (None if embedded)
    pub relative_path: Option<String>,
    /// Duration in seconds
    pub duration: f64,
    /// Sample rate
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u32,
    /// Embedded audio data (for files < 10MB)
    pub embedded_data: Option<EmbeddedAudioData>,
}

impl AudioClipPool {
    /// Serialize the audio clip pool for project saving
    ///
    /// Files smaller than 10MB are embedded as base64.
    /// Larger files are stored as relative paths to the project file.
    pub fn serialize(&self, project_path: &Path) -> Result<Vec<AudioPoolEntry>, String> {
        let project_dir = project_path.parent()
            .ok_or_else(|| "Project path has no parent directory".to_string())?;

        let mut entries = Vec::new();

        for (index, file) in self.files.iter().enumerate() {
            let file_path = &file.path;
            let file_path_str = file_path.to_string_lossy();

            // Check if this is a temp file (from recording) or previously embedded audio
            // Always embed these
            let is_temp_file = file_path.starts_with(std::env::temp_dir());
            let is_embedded = file_path_str.starts_with("<embedded:");

            // Try to get relative path (unless it's a temp/embedded file)
            let relative_path = if is_temp_file || is_embedded {
                None  // Don't store path for temp/embedded files, they'll be embedded
            } else if let Some(rel) = pathdiff::diff_paths(file_path, project_dir) {
                Some(rel.to_string_lossy().to_string())
            } else {
                // Fall back to absolute path if relative path fails
                Some(file_path.to_string_lossy().to_string())
            };

            // Check if we should embed this file
            // Always embed temp files (recordings) and previously embedded audio,
            // otherwise use size threshold
            let embedded_data = if is_temp_file || is_embedded || Self::should_embed(file_path) {
                // Embed from memory - we already have the audio data loaded
                Some(Self::embed_from_memory(file))
            } else {
                None
            };

            let entry = AudioPoolEntry {
                pool_index: index,
                name: file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("file_{}", index)),
                relative_path,
                duration: file.duration_seconds(),
                sample_rate: file.sample_rate,
                channels: file.channels,
                embedded_data,
            };

            entries.push(entry);
        }

        Ok(entries)
    }

    /// Check if a file should be embedded (< 10MB)
    fn should_embed(file_path: &Path) -> bool {
        const TEN_MB: u64 = 10_000_000;

        std::fs::metadata(file_path)
            .map(|m| m.len() < TEN_MB)
            .unwrap_or(false)
    }

    /// Embed audio from memory (already loaded in the pool)
    fn embed_from_memory(audio_file: &AudioFile) -> EmbeddedAudioData {
        use base64::{Engine as _, engine::general_purpose};

        // Convert the f32 interleaved samples to WAV format bytes
        let wav_data = Self::encode_wav(
            &audio_file.data,
            audio_file.channels,
            audio_file.sample_rate
        );

        let data_base64 = general_purpose::STANDARD.encode(&wav_data);

        EmbeddedAudioData {
            data_base64,
            format: "wav".to_string(),
        }
    }

    /// Encode f32 interleaved samples as WAV file bytes
    fn encode_wav(samples: &[f32], channels: u32, sample_rate: u32) -> Vec<u8> {
        let num_samples = samples.len();
        let bytes_per_sample = 4; // 32-bit float
        let data_size = num_samples * bytes_per_sample;
        let file_size = 36 + data_size;

        let mut wav_data = Vec::with_capacity(44 + data_size);

        // RIFF header
        wav_data.extend_from_slice(b"RIFF");
        wav_data.extend_from_slice(&(file_size as u32).to_le_bytes());
        wav_data.extend_from_slice(b"WAVE");

        // fmt chunk
        wav_data.extend_from_slice(b"fmt ");
        wav_data.extend_from_slice(&16u32.to_le_bytes()); // chunk size
        wav_data.extend_from_slice(&3u16.to_le_bytes()); // format code (3 = IEEE float)
        wav_data.extend_from_slice(&(channels as u16).to_le_bytes());
        wav_data.extend_from_slice(&sample_rate.to_le_bytes());
        wav_data.extend_from_slice(&(sample_rate * channels * bytes_per_sample as u32).to_le_bytes()); // byte rate
        wav_data.extend_from_slice(&((channels * bytes_per_sample as u32) as u16).to_le_bytes()); // block align
        wav_data.extend_from_slice(&32u16.to_le_bytes()); // bits per sample

        // data chunk
        wav_data.extend_from_slice(b"data");
        wav_data.extend_from_slice(&(data_size as u32).to_le_bytes());

        // Write samples as little-endian f32
        for &sample in samples {
            wav_data.extend_from_slice(&sample.to_le_bytes());
        }

        wav_data
    }

    /// Load audio pool from serialized entries
    ///
    /// Returns a list of pool indices that failed to load (missing files).
    /// The caller should present these to the user for resolution.
    pub fn load_from_serialized(
        &mut self,
        entries: Vec<AudioPoolEntry>,
        project_path: &Path,
    ) -> Result<Vec<usize>, String> {
        let project_dir = project_path.parent()
            .ok_or_else(|| "Project path has no parent directory".to_string())?;

        let mut missing_indices = Vec::new();

        // Clear existing pool
        self.files.clear();

        // Find the maximum pool index to determine required size
        let max_index = entries.iter()
            .map(|e| e.pool_index)
            .max()
            .unwrap_or(0);

        // Ensure we have space for all entries
        self.files.resize(max_index + 1, AudioFile::new(PathBuf::new(), Vec::new(), 2, 44100));

        for entry in entries {
            let success = if let Some(embedded) = entry.embedded_data {
                // Load from embedded data
                match Self::load_from_embedded_into_pool(self, entry.pool_index, embedded, &entry.name) {
                    Ok(_) => {
                        eprintln!("[AudioPool] Successfully loaded embedded audio: {}", entry.name);
                        true
                    }
                    Err(e) => {
                        eprintln!("[AudioPool] Failed to load embedded audio {}: {}", entry.name, e);
                        false
                    }
                }
            } else if let Some(rel_path) = entry.relative_path {
                // Load from file path
                let full_path = project_dir.join(&rel_path);

                if full_path.exists() {
                    Self::load_file_into_pool(self, entry.pool_index, &full_path).is_ok()
                } else {
                    eprintln!("[AudioPool] File not found: {:?}", full_path);
                    false
                }
            } else {
                eprintln!("[AudioPool] Entry has neither embedded data nor path: {}", entry.name);
                false
            };

            if !success {
                missing_indices.push(entry.pool_index);
            }
        }

        Ok(missing_indices)
    }

    /// Load audio from embedded base64 data
    fn load_from_embedded_into_pool(
        &mut self,
        pool_index: usize,
        embedded: EmbeddedAudioData,
        name: &str,
    ) -> Result<(), String> {
        use base64::{Engine as _, engine::general_purpose};

        // Decode base64
        let data = general_purpose::STANDARD
            .decode(&embedded.data_base64)
            .map_err(|e| format!("Failed to decode base64: {}", e))?;

        // Write to temporary file for symphonia to decode
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("lightningbeam_embedded_{}.{}", pool_index, embedded.format));

        std::fs::write(&temp_path, &data)
            .map_err(|e| format!("Failed to write temporary file: {}", e))?;

        // Load the temporary file using existing infrastructure
        let result = Self::load_file_into_pool(self, pool_index, &temp_path);

        // Clean up temporary file
        let _ = std::fs::remove_file(&temp_path);

        // Update the path to reflect it was embedded
        if result.is_ok() && pool_index < self.files.len() {
            self.files[pool_index].path = PathBuf::from(format!("<embedded: {}>", name));
        }

        result
    }

    /// Load an audio file into a specific pool index
    fn load_file_into_pool(&mut self, pool_index: usize, file_path: &Path) -> Result<(), String> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = std::fs::File::open(file_path)
            .map_err(|e| format!("Failed to open audio file: {}", e))?;

        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = file_path.extension() {
            hint.with_extension(&ext.to_string_lossy());
        }

        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();
        let decoder_opts = DecoderOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| format!("Failed to probe audio file: {}", e))?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| "No audio track found".to_string())?;

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &decoder_opts)
            .map_err(|e| format!("Failed to create decoder: {}", e))?;

        let track_id = track.id;
        let sample_rate = track.codec_params.sample_rate.unwrap_or(44100);
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u32;

        let mut samples = Vec::new();
        let mut sample_buf = None;

        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(_) => break,
            };

            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    if sample_buf.is_none() {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;
                        sample_buf = Some(SampleBuffer::<f32>::new(duration, spec));
                    }

                    if let Some(ref mut buf) = sample_buf {
                        buf.copy_interleaved_ref(decoded);
                        samples.extend_from_slice(buf.samples());
                    }
                }
                Err(_) => continue,
            }
        }

        let audio_file = AudioFile::new(
            file_path.to_path_buf(),
            samples,
            channels,
            sample_rate,
        );

        if pool_index >= self.files.len() {
            return Err(format!("Pool index {} out of bounds", pool_index));
        }

        self.files[pool_index] = audio_file;
        Ok(())
    }

    /// Resolve a missing audio file by loading from a new path
    /// This is called from the UI when the user manually locates a missing file
    pub fn resolve_missing_file(&mut self, pool_index: usize, new_path: &Path) -> Result<(), String> {
        Self::load_file_into_pool(self, pool_index, new_path)
    }
}
