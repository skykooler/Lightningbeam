use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// PCM sample format for memory-mapped audio files
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PcmSampleFormat {
    I16,
    I24,
    F32,
}

/// How audio data is stored for a pool entry
#[derive(Debug, Clone)]
pub enum AudioStorage {
    /// Fully decoded interleaved f32 samples in memory
    InMemory(Vec<f32>),

    /// Memory-mapped PCM file (WAV/AIFF) — instant load, OS-managed paging
    Mapped {
        mmap: Arc<memmap2::Mmap>,
        data_offset: usize,
        sample_format: PcmSampleFormat,
        bytes_per_sample: usize,
        total_frames: u64,
    },

    /// Compressed audio — playback handled by disk reader's stream decoder.
    /// `decoded_for_waveform` is progressively filled by a background thread.
    Compressed {
        decoded_for_waveform: Vec<f32>,
        decoded_frames: u64,
        total_frames: u64,
    },
}

/// Audio file stored in the pool
#[derive(Debug, Clone)]
pub struct AudioFile {
    pub path: PathBuf,
    pub storage: AudioStorage,
    pub channels: u32,
    pub sample_rate: u32,
    pub frames: u64,
    /// Original file format (mp3, ogg, wav, flac, etc.)
    /// Used to determine if we should preserve lossy encoding during save
    pub original_format: Option<String>,
    /// Read-ahead buffer for streaming playback (Compressed files).
    /// When present, `render_from_file` reads from this buffer instead of `data()`.
    pub read_ahead: Option<Arc<super::disk_reader::ReadAheadBuffer>>,
}

impl AudioFile {
    /// Create a new AudioFile with in-memory interleaved f32 data
    pub fn new(path: PathBuf, data: Vec<f32>, channels: u32, sample_rate: u32) -> Self {
        let frames = (data.len() / channels as usize) as u64;
        Self {
            path,
            storage: AudioStorage::InMemory(data),
            channels,
            sample_rate,
            frames,
            original_format: None,
            read_ahead: None,
        }
    }

    /// Create a new AudioFile with original format information
    pub fn with_format(path: PathBuf, data: Vec<f32>, channels: u32, sample_rate: u32, original_format: Option<String>) -> Self {
        let frames = (data.len() / channels as usize) as u64;
        Self {
            path,
            storage: AudioStorage::InMemory(data),
            channels,
            sample_rate,
            frames,
            original_format,
            read_ahead: None,
        }
    }

    /// Create an AudioFile backed by a memory-mapped WAV/AIFF file
    pub fn from_mmap(
        path: PathBuf,
        mmap: memmap2::Mmap,
        data_offset: usize,
        sample_format: PcmSampleFormat,
        channels: u32,
        sample_rate: u32,
        total_frames: u64,
    ) -> Self {
        let bytes_per_sample = match sample_format {
            PcmSampleFormat::I16 => 2,
            PcmSampleFormat::I24 => 3,
            PcmSampleFormat::F32 => 4,
        };
        Self {
            path,
            storage: AudioStorage::Mapped {
                mmap: Arc::new(mmap),
                data_offset,
                sample_format,
                bytes_per_sample,
                total_frames,
            },
            channels,
            sample_rate,
            frames: total_frames,
            original_format: Some("wav".to_string()),
            read_ahead: None,
        }
    }

    /// Create a placeholder AudioFile for a compressed format (playback via disk reader)
    pub fn from_compressed(
        path: PathBuf,
        channels: u32,
        sample_rate: u32,
        total_frames: u64,
        original_format: Option<String>,
    ) -> Self {
        Self {
            path,
            storage: AudioStorage::Compressed {
                decoded_for_waveform: Vec::new(),
                decoded_frames: 0,
                total_frames,
            },
            channels,
            sample_rate,
            frames: total_frames,
            original_format,
            read_ahead: None,
        }
    }

    /// Get interleaved f32 sample data.
    ///
    /// - **InMemory**: returns the full slice directly.
    /// - **Mapped F32**: reinterprets the mmap'd bytes as `&[f32]` (zero-copy).
    /// - **Mapped I16/I24 or Compressed**: returns an empty slice (use
    ///   `read_samples()` or the disk reader's `ReadAheadBuffer` instead).
    pub fn data(&self) -> &[f32] {
        match &self.storage {
            AudioStorage::InMemory(data) => data,
            AudioStorage::Mapped {
                mmap,
                data_offset,
                sample_format,
                total_frames,
                ..
            } if *sample_format == PcmSampleFormat::F32 => {
                let byte_slice = &mmap[*data_offset..];
                let ptr = byte_slice.as_ptr();
                // Check 4-byte alignment (required for f32)
                if ptr.align_offset(std::mem::align_of::<f32>()) == 0 {
                    let len = (*total_frames as usize) * self.channels as usize;
                    let available = byte_slice.len() / 4;
                    let safe_len = len.min(available);
                    // SAFETY: pointer is aligned, mmap is read-only and outlives
                    // this borrow, and we clamp to the available byte range.
                    unsafe { std::slice::from_raw_parts(ptr as *const f32, safe_len) }
                } else {
                    &[]
                }
            }
            _ => &[],
        }
    }

    /// Read samples for a specific channel into the output buffer.
    /// Works for InMemory and Mapped storage. Returns the number of frames read.
    pub fn read_samples(
        &self,
        start_frame: usize,
        count: usize,
        channel: usize,
        out: &mut [f32],
    ) -> usize {
        let channels = self.channels as usize;
        let total_frames = self.frames as usize;

        match &self.storage {
            AudioStorage::InMemory(data) => {
                let mut written = 0;
                for i in 0..count.min(out.len()) {
                    let frame = start_frame + i;
                    if frame >= total_frames { break; }
                    let idx = frame * channels + channel;
                    out[i] = data[idx];
                    written += 1;
                }
                written
            }
            AudioStorage::Mapped { mmap, data_offset, sample_format, bytes_per_sample, .. } => {
                let mut written = 0;
                for i in 0..count.min(out.len()) {
                    let frame = start_frame + i;
                    if frame >= total_frames { break; }
                    let sample_index = frame * channels + channel;
                    let byte_offset = data_offset + sample_index * bytes_per_sample;
                    let end = byte_offset + bytes_per_sample;
                    if end > mmap.len() { break; }
                    let bytes = &mmap[byte_offset..end];
                    out[i] = match sample_format {
                        PcmSampleFormat::I16 => {
                            let val = i16::from_le_bytes([bytes[0], bytes[1]]);
                            val as f32 / 32768.0
                        }
                        PcmSampleFormat::I24 => {
                            // Sign-extend 24-bit to 32-bit
                            let val = ((bytes[0] as i32)
                                | ((bytes[1] as i32) << 8)
                                | ((bytes[2] as i32) << 16))
                                << 8
                                >> 8;
                            val as f32 / 8388608.0
                        }
                        PcmSampleFormat::F32 => {
                            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
                        }
                    };
                    written += 1;
                }
                written
            }
            AudioStorage::Compressed { .. } => {
                // Compressed files are read through the disk reader
                0
            }
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

            let mut min = f32::MAX;
            let mut max = f32::MIN;

            // Scan all samples in this window
            let data = self.data();
            for frame_idx in peak_start..peak_end {
                // For multi-channel audio, combine all channels
                for ch in 0..self.channels as usize {
                    let sample_idx = frame_idx * self.channels as usize + ch;
                    if sample_idx < data.len() {
                        let sample = data[sample_idx];
                        min = min.min(sample);
                        max = max.max(sample);
                    }
                }
            }

            // If no samples were found, clamp to safe defaults
            if min == f32::MAX {
                min = 0.0;
            }
            if max == f32::MIN {
                max = 0.0;
            }

            peaks.push(crate::io::WaveformPeak { min, max });
        }

        peaks
    }
}

/// Pool of shared audio files (audio clip content)
pub struct AudioClipPool {
    files: Vec<AudioFile>,
    /// Waveform chunk cache for multi-resolution waveform generation
    waveform_cache: crate::audio::waveform_cache::WaveformCache,
}

/// Type alias for backwards compatibility
pub type AudioPool = AudioClipPool;

impl AudioClipPool {
    /// Create a new empty audio clip pool
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            waveform_cache: crate::audio::waveform_cache::WaveformCache::new(100), // 100MB cache
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

    /// Get a mutable reference to an audio file by index
    pub fn get_file_mut(&mut self, index: usize) -> Option<&mut AudioFile> {
        self.files.get_mut(index)
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

        let audio_data = audio_file.data();
        let read_ahead = audio_file.read_ahead.as_deref();
        let use_read_ahead = audio_data.is_empty();
        let src_channels = audio_file.channels as usize;

        // Nothing to render: no data and no read-ahead buffer
        if use_read_ahead && read_ahead.is_none() {
            // Log once per pool_index to diagnose silent clips
            static LOGGED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
            let prev = LOGGED.swap(pool_index as u64, std::sync::atomic::Ordering::Relaxed);
            if prev != pool_index as u64 {
                eprintln!("[RENDER] pool={}: data empty, no read_ahead! storage={:?}, frames={}",
                    pool_index, std::mem::discriminant(&audio_file.storage), audio_file.frames);
            }
            return 0;
        }

        // Snapshot the read-ahead buffer range once for the entire render call.
        // This ensures all sinc interpolation taps within a single callback
        // see a consistent range, preventing crackle from concurrent updates.
        let (ra_start, ra_end) = if use_read_ahead {
            read_ahead.unwrap().snapshot()
        } else {
            (0, 0)
        };

        // Buffer-miss counter: how many times we wanted a sample the ring
        // buffer didn't have (frame in file range but outside buffer range).
        let mut buffer_misses: u32 = 0;

        // Read a single interleaved sample by (frame, channel).
        // Uses direct slice access for InMemory/Mapped, or the disk reader's
        // ReadAheadBuffer for compressed files.
        macro_rules! get_sample {
            ($frame:expr, $ch:expr) => {{
                if use_read_ahead {
                    let f = $frame as u64;
                    let s = read_ahead.unwrap().read_sample(f, $ch, ra_start, ra_end);
                    if s == 0.0 && (f < ra_start || f >= ra_end) {
                        buffer_misses += 1;
                    }
                    s
                } else {
                    let idx = ($frame) * src_channels + ($ch);
                    if idx < audio_data.len() { audio_data[idx] } else { 0.0 }
                }
            }};
        }
        let dst_channels = engine_channels as usize;
        let output_frames = output.len() / dst_channels;

        let src_start_position = start_time_seconds * audio_file.sample_rate as f64;

        let mut rendered_frames = 0;

        if audio_file.sample_rate == engine_sample_rate {
            // Fast path: matching sample rates — direct sample copy, no interpolation
            let src_start_frame = src_start_position.floor() as i64;

            // Continuity check: detect gaps/overlaps between consecutive callbacks (DAW_AUDIO_DEBUG=1)
            if std::env::var("DAW_AUDIO_DEBUG").is_ok() {
                use std::sync::atomic::{AtomicI64, Ordering as AO};
                static EXPECTED_NEXT: AtomicI64 = AtomicI64::new(-1);
                static DISCONTINUITIES: AtomicI64 = AtomicI64::new(0);
                let expected = EXPECTED_NEXT.load(AO::Relaxed);
                if expected >= 0 && src_start_frame != expected {
                    let count = DISCONTINUITIES.fetch_add(1, AO::Relaxed) + 1;
                    eprintln!("[RENDER CONTINUITY] DISCONTINUITY #{}: expected frame {}, got {} (delta={})",
                        count, expected, src_start_frame, src_start_frame - expected);
                }
                EXPECTED_NEXT.store(src_start_frame + output_frames as i64, AO::Relaxed);
            }

            for output_frame in 0..output_frames {
                let src_frame = src_start_frame + output_frame as i64;
                if src_frame < 0 || src_frame as u64 >= audio_file.frames {
                    break;
                }
                let sf = src_frame as usize;

                for dst_ch in 0..dst_channels {
                    let sample = if src_channels == dst_channels {
                        get_sample!(sf, dst_ch)
                    } else if src_channels == 1 {
                        get_sample!(sf, 0)
                    } else if dst_channels == 1 {
                        let mut sum = 0.0f32;
                        for src_ch in 0..src_channels {
                            sum += get_sample!(sf, src_ch);
                        }
                        sum / src_channels as f32
                    } else {
                        get_sample!(sf, dst_ch % src_channels)
                    };

                    output[output_frame * dst_channels + dst_ch] += sample * gain;
                }

                rendered_frames += 1;
            }
        } else {
            // Sample rate conversion with windowed sinc interpolation
            let rate_ratio = audio_file.sample_rate as f64 / engine_sample_rate as f64;
            const KERNEL_SIZE: usize = 32;
            const HALF_KERNEL: usize = KERNEL_SIZE / 2;

            for output_frame in 0..output_frames {
                let src_position = src_start_position + (output_frame as f64 * rate_ratio);
                let src_frame = src_position.floor() as i32;
                let frac = (src_position - src_frame as f64) as f32;

                if src_frame < 0 || src_frame as usize >= audio_file.frames as usize {
                    break;
                }

                for dst_ch in 0..dst_channels {
                    let src_ch = if src_channels == dst_channels {
                        dst_ch
                    } else if src_channels == 1 {
                        0
                    } else if dst_channels == 1 {
                        usize::MAX // sentinel: average all channels below
                    } else {
                        dst_ch % src_channels
                    };

                    let sample = if src_ch == usize::MAX {
                        let mut sum = 0.0;
                        for ch in 0..src_channels {
                            let mut channel_samples = [0.0f32; KERNEL_SIZE];
                            for (j, i) in (-(HALF_KERNEL as i32)..(HALF_KERNEL as i32)).enumerate() {
                                let idx = src_frame + i;
                                if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                                    channel_samples[j] = get_sample!(idx as usize, ch);
                                }
                            }
                            sum += windowed_sinc_interpolate(&channel_samples, frac);
                        }
                        sum / src_channels as f32
                    } else {
                        let mut channel_samples = [0.0f32; KERNEL_SIZE];
                        for (j, i) in (-(HALF_KERNEL as i32)..(HALF_KERNEL as i32)).enumerate() {
                            let idx = src_frame + i;
                            if idx >= 0 && (idx as usize) < audio_file.frames as usize {
                                channel_samples[j] = get_sample!(idx as usize, src_ch);
                            }
                        }
                        windowed_sinc_interpolate(&channel_samples, frac)
                    };

                    output[output_frame * dst_channels + dst_ch] += sample * gain;
                }

                rendered_frames += 1;
            }
        }

        if use_read_ahead && buffer_misses > 0 {
            static MISS_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let total = MISS_COUNT.fetch_add(buffer_misses as u64, std::sync::atomic::Ordering::Relaxed) + buffer_misses as u64;
            // Log every 100 misses to avoid flooding
            if total % 100 < buffer_misses as u64 {
                eprintln!("[RENDER] buffer misses this call: {}, total: {}, snap=[{}..{}], src_start_frame={}",
                    buffer_misses, total, ra_start, ra_end,
                    (start_time_seconds * audio_file.sample_rate as f64) as u64);
            }
        }

        rendered_frames * dst_channels
    }

    /// Generate waveform chunks for a file in the pool
    ///
    /// This generates chunks at a specific detail level and caches them.
    /// Returns the generated chunks.
    pub fn generate_waveform_chunks(
        &mut self,
        pool_index: usize,
        detail_level: u8,
        chunk_indices: &[u32],
    ) -> Vec<crate::io::WaveformChunk> {
        let file = match self.files.get(pool_index) {
            Some(f) => f,
            None => return Vec::new(),
        };

        let chunks = crate::audio::waveform_cache::WaveformCache::generate_chunks(
            file,
            pool_index,
            detail_level,
            chunk_indices,
        );

        // Store chunks in cache
        for chunk in &chunks {
            let key = crate::io::WaveformChunkKey {
                pool_index,
                detail_level: chunk.detail_level,
                chunk_index: chunk.chunk_index,
            };
            self.waveform_cache.store_chunk(key, chunk.peaks.clone());
        }

        chunks
    }

    /// Generate Level 0 (overview) chunks for a file
    ///
    /// This should be called immediately when a file is imported.
    /// Returns the generated chunks.
    pub fn generate_overview_chunks(
        &mut self,
        pool_index: usize,
    ) -> Vec<crate::io::WaveformChunk> {
        let file = match self.files.get(pool_index) {
            Some(f) => f,
            None => return Vec::new(),
        };

        self.waveform_cache.generate_overview_chunks(file, pool_index)
    }

    /// Get a cached waveform chunk
    pub fn get_waveform_chunk(
        &self,
        pool_index: usize,
        detail_level: u8,
        chunk_index: u32,
    ) -> Option<&Vec<crate::io::WaveformPeak>> {
        let key = crate::io::WaveformChunkKey {
            pool_index,
            detail_level,
            chunk_index,
        };
        self.waveform_cache.get_chunk(&key)
    }

    /// Check if a waveform chunk is cached
    pub fn has_waveform_chunk(
        &self,
        pool_index: usize,
        detail_level: u8,
        chunk_index: u32,
    ) -> bool {
        let key = crate::io::WaveformChunkKey {
            pool_index,
            detail_level,
            chunk_index,
        };
        self.waveform_cache.has_chunk(&key)
    }

    /// Get waveform cache memory usage in MB
    pub fn waveform_cache_memory_mb(&self) -> f64 {
        self.waveform_cache.memory_usage_mb()
    }

    /// Get number of cached waveform chunks
    pub fn waveform_chunk_count(&self) -> usize {
        self.waveform_cache.chunk_count()
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

        // Check if this is a lossy format that should be preserved
        let is_lossy = audio_file.original_format.as_ref().map_or(false, |fmt| {
            let fmt_lower = fmt.to_lowercase();
            fmt_lower == "mp3" || fmt_lower == "ogg" || fmt_lower == "aac"
                || fmt_lower == "m4a" || fmt_lower == "opus"
        });

        if is_lossy {
            // For lossy formats, read the original file bytes (if it still exists)
            if let Ok(original_bytes) = std::fs::read(&audio_file.path) {
                let data_base64 = general_purpose::STANDARD.encode(&original_bytes);
                return EmbeddedAudioData {
                    data_base64,
                    format: audio_file.original_format.clone().unwrap_or_else(|| "mp3".to_string()),
                };
            }
            // If we can't read the original file, fall through to WAV conversion
        }

        // For lossless/PCM or if we couldn't read the original lossy file,
        // convert the f32 interleaved samples to WAV format bytes
        let wav_data = Self::encode_wav(
            audio_file.data(),
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
        let fn_start = std::time::Instant::now();
        eprintln!("📊 [LOAD_SERIALIZED] Starting load_from_serialized with {} entries...", entries.len());

        let project_dir = project_path.parent()
            .ok_or_else(|| "Project path has no parent directory".to_string())?;

        let mut missing_indices = Vec::new();

        // Clear existing pool
        let clear_start = std::time::Instant::now();
        self.files.clear();
        eprintln!("📊 [LOAD_SERIALIZED] Clear pool took {:.2}ms", clear_start.elapsed().as_secs_f64() * 1000.0);

        // Find the maximum pool index to determine required size
        let max_index = entries.iter()
            .map(|e| e.pool_index)
            .max()
            .unwrap_or(0);

        // Ensure we have space for all entries
        let resize_start = std::time::Instant::now();
        self.files.resize(max_index + 1, AudioFile::new(PathBuf::new(), Vec::new(), 2, 44100));
        eprintln!("📊 [LOAD_SERIALIZED] Resize pool to {} took {:.2}ms", max_index + 1, resize_start.elapsed().as_secs_f64() * 1000.0);

        for (i, entry) in entries.iter().enumerate() {
            let entry_start = std::time::Instant::now();
            eprintln!("📊 [LOAD_SERIALIZED] Processing entry {}/{}: '{}'", i + 1, entries.len(), entry.name);

            let success = if let Some(ref embedded) = entry.embedded_data {
                // Load from embedded data
                eprintln!("📊 [LOAD_SERIALIZED]   Entry has embedded data (format: {})", embedded.format);
                match Self::load_from_embedded_into_pool(self, entry.pool_index, embedded.clone(), &entry.name) {
                    Ok(_) => {
                        eprintln!("[AudioPool] Successfully loaded embedded audio: {}", entry.name);
                        true
                    }
                    Err(e) => {
                        eprintln!("[AudioPool] Failed to load embedded audio {}: {}", entry.name, e);
                        false
                    }
                }
            } else if let Some(ref rel_path) = entry.relative_path {
                // Load from file path
                eprintln!("📊 [LOAD_SERIALIZED]   Entry has file path: {:?}", rel_path);
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

            eprintln!("📊 [LOAD_SERIALIZED] Entry {} took {:.2}ms (success: {})", i + 1, entry_start.elapsed().as_secs_f64() * 1000.0, success);
        }

        eprintln!("📊 [LOAD_SERIALIZED] ✅ Total load_from_serialized time: {:.2}ms", fn_start.elapsed().as_secs_f64() * 1000.0);

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

        let fn_start = std::time::Instant::now();
        eprintln!("📊 [POOL] Loading embedded audio '{}'...", name);

        // Decode base64
        let step1_start = std::time::Instant::now();
        let data = general_purpose::STANDARD
            .decode(&embedded.data_base64)
            .map_err(|e| format!("Failed to decode base64: {}", e))?;
        eprintln!("📊 [POOL]   Step 1: Decode base64 ({} bytes) took {:.2}ms", data.len(), step1_start.elapsed().as_secs_f64() * 1000.0);

        // Write to temporary file for symphonia to decode
        let step2_start = std::time::Instant::now();
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("lightningbeam_embedded_{}.{}", pool_index, embedded.format));

        std::fs::write(&temp_path, &data)
            .map_err(|e| format!("Failed to write temporary file: {}", e))?;
        eprintln!("📊 [POOL]   Step 2: Write temp file took {:.2}ms", step2_start.elapsed().as_secs_f64() * 1000.0);

        // Load the temporary file using existing infrastructure
        let step3_start = std::time::Instant::now();
        let result = Self::load_file_into_pool(self, pool_index, &temp_path);
        eprintln!("📊 [POOL]   Step 3: Decode audio with Symphonia took {:.2}ms", step3_start.elapsed().as_secs_f64() * 1000.0);

        // Clean up temporary file
        let _ = std::fs::remove_file(&temp_path);

        // Update the path to reflect it was embedded
        if result.is_ok() && pool_index < self.files.len() {
            self.files[pool_index].path = PathBuf::from(format!("<embedded: {}>", name));
        }

        eprintln!("📊 [POOL] ✅ Total load_from_embedded time: {:.2}ms", fn_start.elapsed().as_secs_f64() * 1000.0);

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

        // Detect original format from file extension
        let original_format = file_path.extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_lowercase());

        let audio_file = AudioFile::with_format(
            file_path.to_path_buf(),
            samples,
            channels,
            sample_rate,
            original_format,
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
