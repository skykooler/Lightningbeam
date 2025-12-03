//! Video decoding and management for Lightningbeam
//!
//! This module provides FFmpeg-based video decoding with LRU frame caching
//! for efficient video playback and preview.

use std::sync::{Arc, Mutex};
use std::num::NonZeroUsize;
use std::collections::HashMap;
use ffmpeg_next as ffmpeg;
use lru::LruCache;
use uuid::Uuid;

/// Metadata about a video file
#[derive(Debug, Clone)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub has_audio: bool,
}

/// Video decoder with LRU frame caching
struct VideoDecoder {
    path: String,
    width: u32,          // Original video width
    height: u32,         // Original video height
    output_width: u32,   // Scaled output width
    output_height: u32,  // Scaled output height
    fps: f64,
    duration: f64,
    time_base: f64,
    stream_index: usize,
    frame_cache: LruCache<i64, Vec<u8>>, // timestamp -> RGBA data
    input: Option<ffmpeg::format::context::Input>,
    decoder: Option<ffmpeg::decoder::Video>,
    last_decoded_ts: i64, // Track the last decoded frame timestamp
    keyframe_positions: Vec<i64>, // Index of keyframe timestamps for fast seeking
}

impl VideoDecoder {
    /// Create a new video decoder
    ///
    /// `max_width` and `max_height` specify the maximum output dimensions.
    /// Video will be scaled down if larger, preserving aspect ratio.
    fn new(path: String, cache_size: usize, max_width: Option<u32>, max_height: Option<u32>) -> Result<Self, String> {
        ffmpeg::init().map_err(|e| e.to_string())?;

        let input = ffmpeg::format::input(&path)
            .map_err(|e| format!("Failed to open video: {}", e))?;

        let video_stream = input.streams()
            .best(ffmpeg::media::Type::Video)
            .ok_or("No video stream found")?;

        let stream_index = video_stream.index();

        let context_decoder = ffmpeg::codec::context::Context::from_parameters(
            video_stream.parameters()
        ).map_err(|e| e.to_string())?;

        let decoder = context_decoder.decoder().video()
            .map_err(|e| e.to_string())?;

        let width = decoder.width();
        let height = decoder.height();
        let time_base = f64::from(video_stream.time_base());

        // Calculate output dimensions (scale down if larger than max)
        let (output_width, output_height) = if let (Some(max_w), Some(max_h)) = (max_width, max_height) {
            // Calculate scale to fit within max dimensions while preserving aspect ratio
            let scale = (max_w as f32 / width as f32).min(max_h as f32 / height as f32).min(1.0);
            ((width as f32 * scale) as u32, (height as f32 * scale) as u32)
        } else {
            (width, height)
        };

        // Try to get duration from stream, fallback to container
        let duration = if video_stream.duration() > 0 {
            video_stream.duration() as f64 * time_base
        } else if input.duration() > 0 {
            input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
        } else {
            // If no duration available, estimate from frame count and fps
            let fps = f64::from(video_stream.avg_frame_rate());
            if video_stream.frames() > 0 && fps > 0.0 {
                video_stream.frames() as f64 / fps
            } else {
                0.0 // Unknown duration
            }
        };

        let fps = f64::from(video_stream.avg_frame_rate());

        // Build keyframe index for fast seeking
        // This scans the video once to find all keyframe positions
        eprintln!("[Video Decoder] Building keyframe index for {}", path);
        let keyframe_positions = Self::build_keyframe_index(&path, stream_index)?;
        eprintln!("[Video Decoder] Found {} keyframes", keyframe_positions.len());

        Ok(Self {
            path,
            width,
            height,
            output_width,
            output_height,
            fps,
            duration,
            time_base,
            stream_index,
            frame_cache: LruCache::new(
                NonZeroUsize::new(cache_size).unwrap()
            ),
            input: None,
            decoder: None,
            last_decoded_ts: -1,
            keyframe_positions,
        })
    }

    /// Build an index of all keyframe positions in the video
    /// This enables fast seeking by knowing exactly where keyframes are
    fn build_keyframe_index(path: &str, stream_index: usize) -> Result<Vec<i64>, String> {
        let mut input = ffmpeg::format::input(path)
            .map_err(|e| format!("Failed to open video for indexing: {}", e))?;

        let mut keyframes = Vec::new();

        // Scan through all packets to find keyframes
        for (stream, packet) in input.packets() {
            if stream.index() == stream_index {
                // Check if this packet is a keyframe
                if packet.is_key() {
                    if let Some(pts) = packet.pts() {
                        keyframes.push(pts);
                    }
                }
            }
        }

        // Ensure keyframes are sorted (they should be already)
        keyframes.sort_unstable();

        Ok(keyframes)
    }

    /// Find the nearest keyframe at or before the target timestamp
    /// Returns the keyframe timestamp, or 0 if target is before first keyframe
    fn find_nearest_keyframe_before(&self, target_ts: i64) -> i64 {
        // Binary search to find the largest keyframe <= target_ts
        match self.keyframe_positions.binary_search(&target_ts) {
            Ok(idx) => self.keyframe_positions[idx],  // Exact match
            Err(0) => 0,  // Target is before first keyframe, seek to start
            Err(idx) => self.keyframe_positions[idx - 1],  // Use previous keyframe
        }
    }

    /// Get a decoded frame at the specified timestamp
    fn get_frame(&mut self, timestamp: f64) -> Result<Vec<u8>, String> {
        use std::time::Instant;
        let t_start = Instant::now();

        // Round timestamp to nearest frame boundary to improve cache hits
        // This ensures that timestamps like 1.0001s and 0.9999s both map to frame 1.0s
        let frame_duration = 1.0 / self.fps;
        let rounded_timestamp = (timestamp / frame_duration).round() * frame_duration;

        // Convert timestamp to frame timestamp
        let frame_ts = (rounded_timestamp / self.time_base) as i64;

        // Check cache
        if let Some(cached_frame) = self.frame_cache.get(&frame_ts) {
            eprintln!("[Video Timing] Cache hit for ts={:.3}s ({}ms)", timestamp, t_start.elapsed().as_millis());
            return Ok(cached_frame.clone());
        }

        // Determine if we need to seek
        // Seek if: no decoder open, going backwards, or jumping forward more than 2 seconds
        let need_seek = self.decoder.is_none()
            || frame_ts < self.last_decoded_ts
            || frame_ts > self.last_decoded_ts + (2.0 / self.time_base) as i64;

        if need_seek {
            let t_seek_start = Instant::now();

            // Find the nearest keyframe at or before our target using the index
            // This is the exact keyframe position, so we can seek directly to it
            let keyframe_ts_stream = self.find_nearest_keyframe_before(frame_ts);

            // Convert from stream timebase to AV_TIME_BASE (microseconds) for container-level seek
            // input.seek() with stream=-1 expects AV_TIME_BASE units, not stream units
            let keyframe_seconds = keyframe_ts_stream as f64 * self.time_base;
            let keyframe_ts_av = (keyframe_seconds * 1_000_000.0) as i64; // AV_TIME_BASE = 1000000

            eprintln!("[Video Seek] Target: {} | Keyframe(stream): {} | Keyframe(AV): {} | Index size: {}",
                frame_ts, keyframe_ts_stream, keyframe_ts_av, self.keyframe_positions.len());

            // Reopen input
            let mut input = ffmpeg::format::input(&self.path)
                .map_err(|e| format!("Failed to reopen video: {}", e))?;

            // Seek directly to the keyframe with a 1-unit window
            // Can't use keyframe_ts..keyframe_ts (empty) or ..= (not supported)
            input.seek(keyframe_ts_av, keyframe_ts_av..(keyframe_ts_av + 1))
                .map_err(|e| format!("Seek failed: {}", e))?;

            eprintln!("[Video Timing] Seek call took {}ms", t_seek_start.elapsed().as_millis());

            let context_decoder = ffmpeg::codec::context::Context::from_parameters(
                input.streams().best(ffmpeg::media::Type::Video).unwrap().parameters()
            ).map_err(|e| e.to_string())?;

            let decoder = context_decoder.decoder().video()
                .map_err(|e| e.to_string())?;

            self.input = Some(input);
            self.decoder = Some(decoder);
            // Set last_decoded_ts to just before the seek target so forward playback works
            // Without this, every frame would trigger a new seek
            self.last_decoded_ts = frame_ts - 1;
        }

        let input = self.input.as_mut().unwrap();
        let decoder = self.decoder.as_mut().unwrap();

        // Decode frames until we find the one closest to our target timestamp
        let mut best_frame_data: Option<Vec<u8>> = None;
        let mut best_frame_ts: Option<i64> = None;
        let t_decode_start = Instant::now();
        let mut decode_count = 0;
        let mut scale_time_ms = 0u128;

        for (stream, packet) in input.packets() {
            if stream.index() == self.stream_index {
                decoder.send_packet(&packet)
                    .map_err(|e| e.to_string())?;

                let mut frame = ffmpeg::util::frame::Video::empty();
                while decoder.receive_frame(&mut frame).is_ok() {
                    decode_count += 1;
                    let current_frame_ts = frame.timestamp().unwrap_or(0);
                    self.last_decoded_ts = current_frame_ts; // Update last decoded position

                    // Check if this frame is closer to our target than the previous best
                    let is_better = match best_frame_ts {
                        None => true,
                        Some(best_ts) => {
                            (current_frame_ts - frame_ts).abs() < (best_ts - frame_ts).abs()
                        }
                    };

                    if is_better {
                        let t_scale_start = Instant::now();

                        // Convert to RGBA and scale to output size
                        let mut scaler = ffmpeg::software::scaling::context::Context::get(
                            frame.format(),
                            frame.width(),
                            frame.height(),
                            ffmpeg::format::Pixel::RGBA,
                            self.output_width,
                            self.output_height,
                            ffmpeg::software::scaling::flag::Flags::BILINEAR,
                        ).map_err(|e| e.to_string())?;

                        let mut rgb_frame = ffmpeg::util::frame::Video::empty();
                        scaler.run(&frame, &mut rgb_frame)
                            .map_err(|e| e.to_string())?;

                        // Remove stride padding to create tightly packed RGBA data
                        let width = self.output_width as usize;
                        let height = self.output_height as usize;
                        let stride = rgb_frame.stride(0);
                        let row_size = width * 4; // RGBA = 4 bytes per pixel
                        let source_data = rgb_frame.data(0);

                        let mut packed_data = Vec::with_capacity(row_size * height);
                        for y in 0..height {
                            let row_start = y * stride;
                            let row_end = row_start + row_size;
                            packed_data.extend_from_slice(&source_data[row_start..row_end]);
                        }

                        scale_time_ms += t_scale_start.elapsed().as_millis();
                        best_frame_data = Some(packed_data);
                        best_frame_ts = Some(current_frame_ts);
                    }

                    // If we've reached or passed the target timestamp, we can stop
                    if current_frame_ts >= frame_ts {
                        // Found our frame, cache and return it
                        if let Some(data) = best_frame_data {
                            let total_time = t_start.elapsed().as_millis();
                            let decode_time = t_decode_start.elapsed().as_millis();
                            eprintln!("[Video Timing] ts={:.3}s | Decoded {} frames in {}ms | Scale: {}ms | Total: {}ms",
                                timestamp, decode_count, decode_time, scale_time_ms, total_time);
                            self.frame_cache.put(frame_ts, data.clone());
                            return Ok(data);
                        }
                        break;
                    }
                }
            }
        }

        eprintln!("[Video Decoder] ERROR: Failed to decode frame for timestamp {}", timestamp);
        Err("Failed to decode frame".to_string())
    }
}

/// Probe video file for metadata without creating a full decoder
pub fn probe_video(path: &str) -> Result<VideoMetadata, String> {
    ffmpeg::init().map_err(|e| e.to_string())?;

    let input = ffmpeg::format::input(path)
        .map_err(|e| format!("Failed to open video: {}", e))?;

    let video_stream = input.streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("No video stream found")?;

    let context_decoder = ffmpeg::codec::context::Context::from_parameters(
        video_stream.parameters()
    ).map_err(|e| e.to_string())?;

    let decoder = context_decoder.decoder().video()
        .map_err(|e| e.to_string())?;

    let width = decoder.width();
    let height = decoder.height();
    let time_base = f64::from(video_stream.time_base());

    // Try to get duration from stream, fallback to container
    let duration = if video_stream.duration() > 0 {
        video_stream.duration() as f64 * time_base
    } else if input.duration() > 0 {
        input.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE)
    } else {
        // If no duration available, estimate from frame count and fps
        let fps = f64::from(video_stream.avg_frame_rate());
        if video_stream.frames() > 0 && fps > 0.0 {
            video_stream.frames() as f64 / fps
        } else {
            0.0 // Unknown duration
        }
    };

    let fps = f64::from(video_stream.avg_frame_rate());

    // Check for audio stream
    let has_audio = input.streams()
        .best(ffmpeg::media::Type::Audio)
        .is_some();

    Ok(VideoMetadata {
        width,
        height,
        fps,
        duration,
        has_audio,
    })
}

/// A single decoded video frame with RGBA data
#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba_data: Arc<Vec<u8>>,
    pub timestamp: f64,
}

/// Manages video decoders and frame caching for multiple video clips
pub struct VideoManager {
    /// Pool of video decoders, one per clip
    decoders: HashMap<Uuid, Arc<Mutex<VideoDecoder>>>,

    /// Frame cache: (clip_id, timestamp_ms) -> frame
    /// Stores raw RGBA data for zero-copy rendering
    frame_cache: HashMap<(Uuid, i64), Arc<VideoFrame>>,

    /// Thumbnail cache: clip_id -> Vec of (timestamp, rgba_data)
    /// Low-resolution (64px width) thumbnails for scrubbing
    thumbnail_cache: HashMap<Uuid, Vec<(f64, Arc<Vec<u8>>)>>,

    /// Maximum number of frames to cache per decoder
    cache_size: usize,
}

impl VideoManager {
    /// Create a new video manager with default cache size
    pub fn new() -> Self {
        Self::with_cache_size(20)
    }

    /// Create a new video manager with specified cache size
    pub fn with_cache_size(cache_size: usize) -> Self {
        Self {
            decoders: HashMap::new(),
            frame_cache: HashMap::new(),
            thumbnail_cache: HashMap::new(),
            cache_size,
        }
    }

    /// Load a video file and create a decoder for it
    ///
    /// `target_width` and `target_height` specify the maximum dimensions
    /// for decoded frames. Video will be scaled down if larger.
    pub fn load_video(
        &mut self,
        clip_id: Uuid,
        path: String,
        target_width: u32,
        target_height: u32,
    ) -> Result<VideoMetadata, String> {
        // First probe the video for metadata
        let metadata = probe_video(&path)?;

        // Create decoder with target dimensions
        let decoder = VideoDecoder::new(
            path,
            self.cache_size,
            Some(target_width),
            Some(target_height),
        )?;

        // Store decoder in pool
        self.decoders.insert(clip_id, Arc::new(Mutex::new(decoder)));

        Ok(metadata)
    }

    /// Get a decoded frame for a specific clip at a specific timestamp
    ///
    /// Returns None if the clip is not loaded or decoding fails.
    /// Frames are cached for performance.
    pub fn get_frame(&mut self, clip_id: &Uuid, timestamp: f64) -> Option<Arc<VideoFrame>> {
        // Convert timestamp to milliseconds for cache key
        let timestamp_ms = (timestamp * 1000.0) as i64;
        let cache_key = (*clip_id, timestamp_ms);

        // Check frame cache first
        if let Some(cached_frame) = self.frame_cache.get(&cache_key) {
            return Some(Arc::clone(cached_frame));
        }

        // Get decoder for this clip
        let decoder_arc = self.decoders.get(clip_id)?;
        let mut decoder = decoder_arc.lock().ok()?;

        // Decode the frame
        let rgba_data = decoder.get_frame(timestamp).ok()?;
        let width = decoder.output_width;
        let height = decoder.output_height;

        // Create VideoFrame and cache it
        let frame = Arc::new(VideoFrame {
            width,
            height,
            rgba_data: Arc::new(rgba_data),
            timestamp,
        });

        self.frame_cache.insert(cache_key, Arc::clone(&frame));

        Some(frame)
    }

    /// Generate thumbnails for a video clip
    ///
    /// Thumbnails are generated every 5 seconds at 64px width.
    /// This should be called in a background thread to avoid blocking.
    pub fn generate_thumbnails(&mut self, clip_id: &Uuid, duration: f64) -> Result<(), String> {
        let decoder_arc = self.decoders.get(clip_id)
            .ok_or("Clip not loaded")?
            .clone();

        let mut decoder = decoder_arc.lock()
            .map_err(|e| format!("Failed to lock decoder: {}", e))?;

        let mut thumbnails = Vec::new();
        let interval = 5.0; // Generate thumbnail every 5 seconds
        let mut t = 0.0;

        while t < duration {
            // Decode frame at this timestamp
            if let Ok(rgba_data) = decoder.get_frame(t) {
                // Decode already scaled to output dimensions, but we want 128px width for thumbnails
                // We need to scale down further
                let current_width = decoder.output_width;
                let current_height = decoder.output_height;

                // Calculate thumbnail dimensions (128px width, maintain aspect ratio)
                let thumb_width = 128u32;
                let aspect_ratio = current_height as f32 / current_width as f32;
                let thumb_height = (thumb_width as f32 * aspect_ratio) as u32;

                // Simple nearest-neighbor downsampling for thumbnails
                let thumb_data = downsample_rgba(
                    &rgba_data,
                    current_width,
                    current_height,
                    thumb_width,
                    thumb_height,
                );

                thumbnails.push((t, Arc::new(thumb_data)));
            }

            t += interval;
        }

        // Store thumbnails in cache
        self.thumbnail_cache.insert(*clip_id, thumbnails);

        Ok(())
    }

    /// Get the thumbnail closest to the specified timestamp
    ///
    /// Returns None if no thumbnails have been generated for this clip.
    pub fn get_thumbnail_at(&self, clip_id: &Uuid, timestamp: f64) -> Option<(u32, u32, Arc<Vec<u8>>)> {
        let thumbnails = self.thumbnail_cache.get(clip_id)?;

        if thumbnails.is_empty() {
            return None;
        }

        // Binary search for closest thumbnail
        let idx = thumbnails.binary_search_by(|(t, _)| {
            t.partial_cmp(&timestamp).unwrap_or(std::cmp::Ordering::Equal)
        }).unwrap_or_else(|idx| {
            // If exact match not found, pick the closest
            if idx == 0 {
                0
            } else if idx >= thumbnails.len() {
                thumbnails.len() - 1
            } else {
                // Compare distance to previous and next
                let prev_dist = (thumbnails[idx - 1].0 - timestamp).abs();
                let next_dist = (thumbnails[idx].0 - timestamp).abs();
                if prev_dist < next_dist {
                    idx - 1
                } else {
                    idx
                }
            }
        });

        let (_, rgba_data) = &thumbnails[idx];

        // Return (width, height, data)
        // Thumbnails are always 128px width
        let thumb_width = 128;
        let thumb_height = (rgba_data.len() / (thumb_width * 4)) as u32;

        Some((thumb_width as u32, thumb_height, Arc::clone(rgba_data)))
    }

    /// Remove a video clip and its cached data
    pub fn unload_video(&mut self, clip_id: &Uuid) {
        self.decoders.remove(clip_id);

        // Remove all cached frames for this clip
        self.frame_cache.retain(|(id, _), _| id != clip_id);

        // Remove thumbnails
        self.thumbnail_cache.remove(clip_id);
    }

    /// Clear all frame caches (useful for memory management)
    pub fn clear_frame_cache(&mut self) {
        self.frame_cache.clear();
    }
}

impl Default for VideoManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple nearest-neighbor downsampling for RGBA images
fn downsample_rgba(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    let mut dst = Vec::with_capacity((dst_width * dst_height * 4) as usize);

    let x_ratio = src_width as f32 / dst_width as f32;
    let y_ratio = src_height as f32 / dst_height as f32;

    for y in 0..dst_height {
        for x in 0..dst_width {
            let src_x = (x as f32 * x_ratio) as u32;
            let src_y = (y as f32 * y_ratio) as u32;

            let src_idx = ((src_y * src_width + src_x) * 4) as usize;

            // Copy RGBA bytes
            dst.push(src[src_idx]);     // R
            dst.push(src[src_idx + 1]); // G
            dst.push(src[src_idx + 2]); // B
            dst.push(src[src_idx + 3]); // A
        }
    }

    dst
}

/// Extracted audio data from a video file
#[derive(Debug, Clone)]
pub struct ExtractedAudio {
    pub samples: Vec<f32>,
    pub channels: u32,
    pub sample_rate: u32,
    pub duration: f64,
}

/// Extract audio from a video file
///
/// This function performs the slow FFmpeg decoding without holding any locks.
/// The caller can then quickly add the audio to the DAW backend in a background thread.
///
/// Returns None if the video has no audio stream.
pub fn extract_audio_from_video(path: &str) -> Result<Option<ExtractedAudio>, String> {
    ffmpeg::init().map_err(|e| e.to_string())?;

    // Open video file
    let mut input = ffmpeg::format::input(path)
        .map_err(|e| format!("Failed to open video: {}", e))?;

    // Find audio stream
    let audio_stream_opt = input.streams()
        .best(ffmpeg::media::Type::Audio);

    // Return None if no audio stream
    if audio_stream_opt.is_none() {
        return Ok(None);
    }

    let audio_stream = audio_stream_opt.unwrap();
    let audio_index = audio_stream.index();

    // Get audio properties
    let context_decoder = ffmpeg::codec::context::Context::from_parameters(
        audio_stream.parameters()
    ).map_err(|e| e.to_string())?;

    let mut audio_decoder = context_decoder.decoder().audio()
        .map_err(|e| e.to_string())?;

    let sample_rate = audio_decoder.rate();
    let channels = audio_decoder.channels() as u32;

    // Decode all audio frames
    let mut audio_samples: Vec<f32> = Vec::new();

    for (stream, packet) in input.packets() {
        if stream.index() == audio_index {
            audio_decoder.send_packet(&packet)
                .map_err(|e| e.to_string())?;

            let mut audio_frame = ffmpeg::util::frame::Audio::empty();
            while audio_decoder.receive_frame(&mut audio_frame).is_ok() {
                // Convert audio to f32 packed format
                let format = audio_frame.format();
                let frame_channels = audio_frame.channels() as usize;

                // Create resampler to convert to f32 packed
                let mut resampler = ffmpeg::software::resampling::context::Context::get(
                    format,
                    audio_frame.channel_layout(),
                    sample_rate,
                    ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
                    audio_frame.channel_layout(),
                    sample_rate,
                ).map_err(|e| e.to_string())?;

                let mut resampled_frame = ffmpeg::util::frame::Audio::empty();
                resampler.run(&audio_frame, &mut resampled_frame)
                    .map_err(|e| e.to_string())?;

                // Extract f32 samples (interleaved format)
                let data_ptr = resampled_frame.data(0).as_ptr() as *const f32;
                let total_samples = resampled_frame.samples() * frame_channels;
                let samples_slice = unsafe {
                    std::slice::from_raw_parts(data_ptr, total_samples)
                };

                audio_samples.extend_from_slice(samples_slice);
            }
        }
    }

    // Flush audio decoder
    audio_decoder.send_eof().map_err(|e| e.to_string())?;
    let mut audio_frame = ffmpeg::util::frame::Audio::empty();
    while audio_decoder.receive_frame(&mut audio_frame).is_ok() {
        let format = audio_frame.format();
        let frame_channels = audio_frame.channels() as usize;

        let mut resampler = ffmpeg::software::resampling::context::Context::get(
            format,
            audio_frame.channel_layout(),
            sample_rate,
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            audio_frame.channel_layout(),
            sample_rate,
        ).map_err(|e| e.to_string())?;

        let mut resampled_frame = ffmpeg::util::frame::Audio::empty();
        resampler.run(&audio_frame, &mut resampled_frame)
            .map_err(|e| e.to_string())?;

        let data_ptr = resampled_frame.data(0).as_ptr() as *const f32;
        let total_samples = resampled_frame.samples() * frame_channels;
        let samples_slice = unsafe {
            std::slice::from_raw_parts(data_ptr, total_samples)
        };

        audio_samples.extend_from_slice(samples_slice);
    }

    // Calculate duration
    let total_samples_per_channel = audio_samples.len() / channels as usize;
    let duration = total_samples_per_channel as f64 / sample_rate as f64;

    Ok(Some(ExtractedAudio {
        samples: audio_samples,
        channels,
        sample_rate,
        duration,
    }))
}
