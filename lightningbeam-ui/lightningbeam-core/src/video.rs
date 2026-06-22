//! Video decoding and management for Lightningbeam
//!
//! This module provides FFmpeg-based video decoding with LRU frame caching
//! for efficient video playback and preview.

use std::sync::{Arc, Mutex};
use std::num::NonZeroUsize;
use std::collections::HashMap;
use std::path::PathBuf;
use ffmpeg_next as ffmpeg;
use ffmpeg_blob_io::BlobInput;
use lru::LruCache;
use uuid::Uuid;

use crate::beam_archive::BeamArchive;

/// Where a video clip's bytes live.
///
/// `Path` is an external file (referenced video, webcam capture, fresh import).
/// `Packed` streams from a `MediaKind::Video` blob inside the `.beam` container.
#[derive(Clone, Debug)]
pub enum VideoSource {
    /// External file path.
    Path(String),
    /// Packed in the container: open a fresh `BlobReader` over `media_id` in `db_path`.
    Packed { db_path: PathBuf, media_id: Uuid },
}

impl VideoSource {
    /// Open a fresh demuxer input for this source. A new `BlobReader` (own SQLite
    /// connection) is created per call, so this is safe to call on any thread and
    /// on every seek-reopen.
    fn open(&self) -> Result<OwnedInput, String> {
        match self {
            VideoSource::Path(p) => ffmpeg::format::input(p)
                .map(OwnedInput::Path)
                .map_err(|e| format!("Failed to open video: {}", e)),
            VideoSource::Packed { db_path, media_id } => {
                let archive = BeamArchive::open(db_path)?;
                let hint = archive.media_info(*media_id)?.map(|i| i.codec);
                let reader = archive.open_blob_reader(db_path, *media_id)?;
                BlobInput::open(Box::new(reader), hint.as_deref())
                    .map(OwnedInput::Blob)
                    .map_err(|e| format!("Failed to open packed video: {}", e))
            }
        }
    }

    /// Short label for logging.
    fn label(&self) -> String {
        match self {
            VideoSource::Path(p) => p.clone(),
            VideoSource::Packed { media_id, .. } => format!("packed:{}", media_id),
        }
    }
}

/// An open demuxer input, either file-backed or streaming from a container blob.
/// Both expose the same `ffmpeg` `Input` for decoding.
enum OwnedInput {
    Path(ffmpeg::format::context::Input),
    Blob(BlobInput),
}

impl OwnedInput {
    fn get(&mut self) -> &mut ffmpeg::format::context::Input {
        match self {
            OwnedInput::Path(i) => i,
            OwnedInput::Blob(b) => b.input_mut(),
        }
    }
}

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
pub struct VideoDecoder {
    source: VideoSource,
    _width: u32,          // Original video width
    _height: u32,         // Original video height
    output_width: u32,   // Scaled output width
    output_height: u32,  // Scaled output height
    fps: f64,
    _duration: f64,
    time_base: f64,
    stream_index: usize,
    frame_cache: LruCache<i64, Vec<u8>>, // timestamp -> RGBA data
    input: Option<OwnedInput>,
    decoder: Option<ffmpeg::decoder::Video>,
    last_decoded_ts: i64, // Track the last decoded frame timestamp
    keyframe_positions: Vec<i64>, // Index of keyframe timestamps for fast seeking
}

impl VideoDecoder {
    /// Create a new video decoder
    ///
    /// `max_width` and `max_height` specify the maximum output dimensions.
    /// Video will be scaled down if larger, preserving aspect ratio.
    /// `build_keyframes` controls whether to build the keyframe index immediately (slow)
    /// or defer it for async building later.
    fn new(source: VideoSource, cache_size: usize, max_width: Option<u32>, max_height: Option<u32>, build_keyframes: bool) -> Result<Self, String> {
        ffmpeg::init().map_err(|e| e.to_string())?;

        let mut owned = source.open()?;
        let input = owned.get();

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

        // Optionally build keyframe index for fast seeking
        let keyframe_positions = if build_keyframes {
            eprintln!("[Video Decoder] Building keyframe index for {}", source.label());
            let positions = Self::scan_keyframes(&source, stream_index)?;
            eprintln!("[Video Decoder] Found {} keyframes", positions.len());
            positions
        } else {
            eprintln!("[Video Decoder] Deferring keyframe index building for {}", source.label());
            Vec::new()
        };

        Ok(Self {
            source,
            _width: width,
            _height: height,
            output_width,
            output_height,
            fps,
            _duration: duration,
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

    /// The source this decoder reads from (file path or packed container blob).
    pub fn source(&self) -> VideoSource {
        self.source.clone()
    }

    /// Parameters needed to scan keyframes off-thread (source + video stream index).
    pub fn keyframe_scan_params(&self) -> (VideoSource, usize) {
        (self.source.clone(), self.stream_index)
    }

    /// Replace the keyframe index (built off-thread via [`VideoDecoder::scan_keyframes`]).
    pub fn set_keyframe_index(&mut self, positions: Vec<i64>) {
        self.keyframe_positions = positions;
    }

    /// Get the output width (scaled dimensions)
    pub fn get_output_width(&self) -> u32 {
        self.output_width
    }

    /// Get the output height (scaled dimensions)
    pub fn get_output_height(&self) -> u32 {
        self.output_height
    }

    /// Decode a frame at the specified timestamp (public wrapper)
    pub fn decode_frame(&mut self, timestamp: f64) -> Result<Vec<u8>, String> {
        self.get_frame(timestamp)
    }

    /// Build an index of all keyframe positions in the video by scanning packets
    /// from a fresh input. Does not touch `self` — call it off-thread (it is slow
    /// on long videos) and hand the result to [`VideoDecoder::set_keyframe_index`].
    pub fn scan_keyframes(source: &VideoSource, stream_index: usize) -> Result<Vec<i64>, String> {
        let mut owned = source.open()
            .map_err(|e| format!("Failed to open video for indexing: {}", e))?;
        let input = owned.get();

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

            // Reopen input (a fresh BlobReader for packed sources).
            let mut owned = self.source.open()
                .map_err(|e| format!("Failed to reopen video: {}", e))?;

            {
                let input = owned.get();
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
                self.decoder = Some(decoder);
            }
            self.input = Some(owned);
            // Set last_decoded_ts to just before the seek target so forward playback works
            // Without this, every frame would trigger a new seek
            self.last_decoded_ts = frame_ts - 1;
        }

        let input = self.input.as_mut().unwrap().get();
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

/// Generate timeline thumbnails for a video using a **dedicated** decoder that
/// is independent of any shared playback decoder — so thumbnail work never holds
/// a lock the UI/playback needs.
///
/// Thumbnails are sampled at keyframes ~`interval_secs` apart. Decoding at a
/// keyframe is cheap (≈one frame) versus decoding forward to an arbitrary
/// timestamp (the whole GOP). Frames are decoded directly at `thumb_width` (so
/// `get_thumbnail_at`'s 128-wide assumption holds) and tightly packed RGBA is
/// handed to `on_thumb` as `(timestamp_secs, data)`.
pub fn generate_keyframe_thumbnails(
    source: VideoSource,
    interval_secs: f64,
    thumb_width: u32,
    mut should_skip: impl FnMut(f64) -> bool,
    mut on_thumb: impl FnMut(f64, Arc<Vec<u8>>),
) -> Result<(), String> {
    // Own decoder at thumbnail resolution; builds its own keyframe index. The
    // large max-height lets width be the constraining dimension, so output width
    // is exactly `thumb_width`.
    let mut decoder = VideoDecoder::new(
        source,
        4,
        Some(thumb_width),
        Some(100_000),
        true, // build keyframe index (needed to sample at keyframes)
    )?;

    let keyframe_secs: Vec<f64> = decoder
        .keyframe_positions
        .iter()
        .map(|&ts| ts as f64 * decoder.time_base)
        .collect();

    let mut last_emitted = f64::NEG_INFINITY;
    for ks in keyframe_secs {
        if ks - last_emitted < interval_secs {
            continue;
        }
        // This keyframe is a target slot; advance regardless of skip so the chosen
        // slots are deterministic (lets a resumed pass target the same timestamps).
        last_emitted = ks;
        // Skip slots already covered (resume after a partial save / dedup).
        if should_skip(ks) {
            continue;
        }
        if let Ok(rgba) = decoder.get_frame(ks) {
            on_thumb(ks, Arc::new(rgba));
        }
    }
    Ok(())
}

/// Probe video file for metadata without creating a full decoder
pub fn probe_video(source: &VideoSource) -> Result<VideoMetadata, String> {
    ffmpeg::init().map_err(|e| e.to_string())?;

    let mut owned = source.open()?;
    let input = owned.get();

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

    /// Frame cache: (clip_id, timestamp_ms) -> frame. Stores decoded RGBA for
    /// zero-copy rendering. Bounded by a **byte budget** (not a frame count, which
    /// would be unsafe across resolutions — a 4K frame is ~33MB vs ~2MB at 800x600)
    /// so playback of arbitrarily long video never grows unbounded.
    frame_cache: LruCache<(Uuid, i64), Arc<VideoFrame>>,
    /// Running total of bytes held in `frame_cache` (sum of each frame's RGBA len),
    /// kept in sync on insert/evict/remove so eviction is O(1) per frame.
    frame_cache_bytes: usize,

    /// Thumbnail cache: clip_id -> Vec of (timestamp, rgba_data)
    /// Low-resolution (64px width) thumbnails for scrubbing
    thumbnail_cache: HashMap<Uuid, Vec<(f64, Arc<Vec<u8>>)>>,

    /// Clips whose thumbnail generation finished. Only complete sets are worth
    /// persisting — a partial set (saved mid-generation) is dropped so the load
    /// regenerates it fully rather than leaving it permanently incomplete.
    thumbnails_complete: std::collections::HashSet<Uuid>,

    /// Maximum number of frames to cache per decoder
    cache_size: usize,
}

/// Byte budget for [`VideoManager::frame_cache`] (decoded full-resolution frames).
/// At ~2MB/frame (800x600) this holds ~128 frames; at ~33MB/frame (4K) ~8 — in
/// both cases enough for the current frame plus a scrub window, while bounding RAM.
const FRAME_CACHE_BYTE_BUDGET: usize = 256 * 1024 * 1024;

impl VideoManager {
    /// Create a new video manager with default cache size
    pub fn new() -> Self {
        Self::with_cache_size(20)
    }

    /// Create a new video manager with specified cache size
    pub fn with_cache_size(cache_size: usize) -> Self {
        Self {
            decoders: HashMap::new(),
            frame_cache: LruCache::unbounded(),
            frame_cache_bytes: 0,
            thumbnail_cache: HashMap::new(),
            thumbnails_complete: std::collections::HashSet::new(),
            cache_size,
        }
    }

    /// Load a video file and create a decoder for it
    ///
    /// `target_width` and `target_height` specify the maximum dimensions
    /// for decoded frames. Video will be scaled down if larger.
    ///
    /// The keyframe index is NOT built during this call — scan it off-thread via
    /// [`VideoDecoder::scan_keyframes`] and store it with
    /// [`VideoDecoder::set_keyframe_index`] so the slow scan never blocks playback.
    pub fn load_video(
        &mut self,
        clip_id: Uuid,
        source: VideoSource,
        target_width: u32,
        target_height: u32,
    ) -> Result<VideoMetadata, String> {
        // First probe the video for metadata
        let metadata = probe_video(&source)?;

        // Create decoder with target dimensions, without building keyframe index
        let decoder = VideoDecoder::new(
            source,
            self.cache_size,
            Some(target_width),
            Some(target_height),
            false, // Don't build keyframe index synchronously
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

        // Get decoder for this clip. Clone the Arc so we don't hold a borrow of
        // `self.decoders` across the `&mut self` cache insert below.
        let decoder_arc = Arc::clone(self.decoders.get(clip_id)?);
        let mut decoder = decoder_arc.lock().ok()?;

        // Decode the frame
        let rgba_data = decoder.get_frame(timestamp).ok()?;
        let width = decoder.output_width;
        let height = decoder.output_height;
        drop(decoder); // release the lock before touching `self`

        // Create VideoFrame and cache it
        let frame = Arc::new(VideoFrame {
            width,
            height,
            rgba_data: Arc::new(rgba_data),
            timestamp,
        });

        self.cache_frame(cache_key, Arc::clone(&frame));

        Some(frame)
    }

    /// Insert a frame into the byte-budgeted cache, evicting least-recently-used
    /// frames until the total is within [`FRAME_CACHE_BYTE_BUDGET`].
    fn cache_frame(&mut self, key: (Uuid, i64), frame: Arc<VideoFrame>) {
        let bytes = frame.rgba_data.len();
        if let Some(old) = self.frame_cache.put(key, frame) {
            self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(old.rgba_data.len());
        }
        self.frame_cache_bytes += bytes;
        // Keep at least one frame resident even if it alone exceeds the budget.
        while self.frame_cache_bytes > FRAME_CACHE_BYTE_BUDGET && self.frame_cache.len() > 1 {
            if let Some((_, evicted)) = self.frame_cache.pop_lru() {
                self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(evicted.rgba_data.len());
            } else {
                break;
            }
        }
    }

    /// Get the decoder Arc for a clip (for external thumbnail generation)
    /// This allows external code to decode frames without holding the VideoManager lock
    pub fn get_decoder(&self, clip_id: &Uuid) -> Option<Arc<Mutex<VideoDecoder>>> {
        self.decoders.get(clip_id).cloned()
    }

    /// Snapshot all cached thumbnails for persistence (clip id -> sorted
    /// (timestamp, rgba) pairs). Cheap: clones the `Arc`s, not the pixel data.
    /// Partial sets are persisted too — pair with [`complete_thumbnail_clips`] so
    /// the load knows which clips still need generation resumed.
    pub fn snapshot_all_thumbnails(&self) -> HashMap<Uuid, Vec<(f64, Arc<Vec<u8>>)>> {
        self.thumbnail_cache.clone()
    }

    /// The set of clips whose thumbnail generation has finished (a full keyframe
    /// pass). A persisted set flagged incomplete is resumed on load.
    pub fn complete_thumbnail_clips(&self) -> std::collections::HashSet<Uuid> {
        self.thumbnails_complete.clone()
    }

    /// Mark a clip's thumbnail generation as complete (called when the background
    /// generator finishes the full keyframe pass).
    pub fn mark_thumbnails_complete(&mut self, clip_id: &Uuid) {
        self.thumbnails_complete.insert(*clip_id);
    }

    /// Whether the clip already has a thumbnail within `tol` seconds of `ts`.
    /// Lets the generator skip keyframes already covered (resume / dedup).
    pub fn has_thumbnail_near(&self, clip_id: &Uuid, ts: f64, tol: f64) -> bool {
        self.thumbnail_cache
            .get(clip_id)
            .map_or(false, |v| v.iter().any(|(t, _)| (t - ts).abs() < tol))
    }

    /// Insert a thumbnail into the cache, keeping it **sorted by timestamp** and
    /// **deduped** (an existing entry at the same timestamp is replaced). Sorted
    /// order is required by `get_thumbnail_at`'s binary search, and dedup makes
    /// concurrent restore + resumed generation idempotent (no double inserts).
    pub fn insert_thumbnail(&mut self, clip_id: &Uuid, timestamp: f64, data: Arc<Vec<u8>>) {
        let vec = self.thumbnail_cache.entry(*clip_id).or_default();
        match vec.binary_search_by(|(t, _)| {
            t.partial_cmp(&timestamp).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(i) => vec[i] = (timestamp, data),
            Err(i) => vec.insert(i, (timestamp, data)),
        }
    }

    /// Get the thumbnail closest to the specified timestamp.
    ///
    /// Returns `(actual_timestamp, width, height, data)` — `actual_timestamp` is
    /// the time of the thumbnail actually chosen (which may differ from the
    /// requested `timestamp`, and changes as closer thumbnails finish generating).
    /// Callers key their GPU texture cache on it so the on-clip strip refreshes as
    /// better thumbnails load instead of freezing on the first one.
    /// Returns None if no thumbnails have been generated for this clip.
    pub fn get_thumbnail_at(&self, clip_id: &Uuid, timestamp: f64) -> Option<(f64, u32, u32, Arc<Vec<u8>>)> {
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

        let (actual_ts, rgba_data) = &thumbnails[idx];

        // Return (actual_timestamp, width, height, data)
        // Thumbnails are always 128px width
        let thumb_width = 128;
        let thumb_height = (rgba_data.len() / (thumb_width * 4)) as u32;

        Some((*actual_ts, thumb_width as u32, thumb_height, Arc::clone(rgba_data)))
    }

    /// Remove a video clip and its cached data
    pub fn unload_video(&mut self, clip_id: &Uuid) {
        self.decoders.remove(clip_id);

        // Remove all cached frames for this clip (LruCache has no retain; collect
        // matching keys, then pop each, keeping the byte total in sync).
        let keys: Vec<(Uuid, i64)> = self
            .frame_cache
            .iter()
            .filter(|((id, _), _)| id == clip_id)
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            if let Some(frame) = self.frame_cache.pop(&key) {
                self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(frame.rgba_data.len());
            }
        }

        // Remove thumbnails
        self.thumbnail_cache.remove(clip_id);
        self.thumbnails_complete.remove(clip_id);
    }

    /// Clear all frame caches (useful for memory management)
    pub fn clear_frame_cache(&mut self) {
        self.frame_cache.clear();
        self.frame_cache_bytes = 0;
    }
}

impl Default for VideoManager {
    fn default() -> Self {
        Self::new()
    }
}
