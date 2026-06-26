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
    native_width: u32,    // Original (decoded) video width
    native_height: u32,   // Original (decoded) video height
    fps: f64,
    _duration: f64,
    time_base: f64,
    stream_index: usize,
    // Decoded RGBA keyed by (frame timestamp, output width, output height): the same source
    // frame may be requested at different sizes (preview res vs export res).
    frame_cache: LruCache<(i64, u32, u32), Vec<u8>>,
    input: Option<OwnedInput>,
    decoder: Option<ffmpeg::decoder::Video>,
    last_decoded_ts: i64, // Track the last decoded frame timestamp
    keyframe_positions: Vec<i64>, // Index of keyframe timestamps for fast seeking
    /// Reused RGBA scaler, keyed by `(input format, input w, input h, output w, output h)`.
    /// Building an swscale context isn't free; a stream's frames share one input format/size and a
    /// consumer keeps one output size, so it's built once and rebuilt only when either changes.
    scaler: Option<(ffmpeg::format::Pixel, u32, u32, u32, u32, SendScaler)>,
    /// When set (and `hw_failed` is false), decode on the GPU: attach `hw_device` as the decoder's
    /// `hw_device_ctx`, decode into VAAPI surfaces, and hand each surface to `importer` to import as
    /// wgpu NV12 textures (no CPU copy). `None`/failure → the software swscale path.
    hw_device: Option<HwDeviceHandle>,
    importer: Option<Arc<dyn HwVideoImporter>>,
    /// Set if hardware decode init failed for this clip — fall back to software permanently.
    hw_failed: bool,
}

/// A decoded frame: CPU RGBA (software) or GPU NV12 textures (hardware).
enum DecodedFrame {
    Cpu { rgba: Vec<u8>, width: u32, height: u32 },
    Gpu(GpuVideoFrame),
}

/// `get_format` callback for hardware decode: select VAAPI surfaces. With `hw_device_ctx` set,
/// FFmpeg auto-allocates the frames context.
unsafe extern "C" fn get_vaapi_format(
    _ctx: *mut ffmpeg::ffi::AVCodecContext,
    mut fmts: *const ffmpeg::ffi::AVPixelFormat,
) -> ffmpeg::ffi::AVPixelFormat {
    while *fmts != ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *fmts == ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI {
            return ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
        }
        fmts = fmts.add(1);
    }
    ffmpeg::ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

/// `SwsContext` is `!Send` in ffmpeg-next, but a `VideoDecoder` (like its decoder/input) is only
/// ever accessed under the `VideoManager` mutex — never concurrently — so moving it between
/// threads is sound. The decoder/input fields rely on the same invariant.
struct SendScaler(ffmpeg::software::scaling::context::Context);
unsafe impl Send for SendScaler {}

/// Per-frame video decode tracing, gated behind `LB_VIDEO_DEBUG` (checked once). Off by
/// default — at export frame rates these prints are a lot of locked stderr writes.
fn video_debug() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("LB_VIDEO_DEBUG").is_ok())
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

        // Output dimensions are now chosen per `get_frame` call (the caller's target res, capped to
        // native) rather than frozen here — so the same clip can be decoded at preview res for the
        // canvas and at full export res, and exporting above document res no longer upscales.
        // `max_width`/`max_height` are retained as an upper bound for callers that want a fixed cap
        // (e.g. thumbnails pass their thumb width per call instead).
        let _ = (max_width, max_height);

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
            native_width: width,
            native_height: height,
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
            scaler: None,
            hw_device: None,
            importer: None,
            hw_failed: false,
        })
    }

    /// Configure hardware (VAAPI) decode for this clip. The next decoder open attaches `hw_device`
    /// and decodes into VAAPI surfaces imported by `importer`. Resets any prior decoder so the new
    /// mode takes effect on the next `get_frame`.
    fn set_hardware(&mut self, hw_device: HwDeviceHandle, importer: Arc<dyn HwVideoImporter>) {
        self.hw_device = Some(hw_device);
        self.importer = Some(importer);
        self.hw_failed = false;
        self.decoder = None; // force a rebuild with hw_device_ctx
        self.input = None;
    }

    /// Whether this decoder will hardware-decode (configured + not failed).
    fn hw_active(&self) -> bool {
        self.hw_device.is_some() && self.importer.is_some() && !self.hw_failed
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

    /// The output size for a requested target: the target capped to native resolution, preserving
    /// aspect ratio (never upscale beyond native — there's no detail to invent).
    fn capped_output(&self, target_w: u32, target_h: u32) -> (u32, u32) {
        let (nw, nh) = (self.native_width as f32, self.native_height as f32);
        if nw <= 0.0 || nh <= 0.0 { return (self.native_width.max(1), self.native_height.max(1)); }
        let scale = (target_w as f32 / nw).min(target_h as f32 / nh).min(1.0);
        (((nw * scale) as u32).max(1), ((nh * scale) as u32).max(1))
    }

    /// Decode a frame at the specified timestamp, at native resolution (public wrapper).
    pub fn decode_frame(&mut self, timestamp: f64) -> Result<Vec<u8>, String> {
        // Software-only helper; request CPU output.
        match self.get_frame(timestamp, self.native_width, self.native_height, false)? {
            DecodedFrame::Cpu { rgba, .. } => Ok(rgba),
            DecodedFrame::Gpu(_) => Err("decode_frame: unexpected GPU frame".into()),
        }
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

    /// Decode the frame at `timestamp`, scaled to `capped_output(target_w, target_h)`. Returns GPU
    /// NV12 textures when hardware-decoding and `want_gpu` (the consumer is on the shared device,
    /// i.e. the preview); otherwise CPU RGBA. A hardware decoder serving a CPU consumer (export)
    /// downloads the surface via `av_hwframe_transfer_data` then swscales. The `VideoManager` caches
    /// the result, so the inner RGBA cache here is for CPU output only.
    fn get_frame(&mut self, timestamp: f64, target_w: u32, target_h: u32, want_gpu: bool) -> Result<DecodedFrame, String> {
        use std::time::Instant;
        let t_start = Instant::now();

        // `hw` = decoder is opened in hardware mode (produces VAAPI surfaces).
        // `gpu_out` = return GPU textures (hw + the consumer can use them).
        let hw = self.hw_active();
        let gpu_out = hw && want_gpu;
        let (out_w, out_h) = self.capped_output(target_w, target_h);

        // Round timestamp to nearest frame boundary to improve cache hits
        // This ensures that timestamps like 1.0001s and 0.9999s both map to frame 1.0s
        let frame_duration = 1.0 / self.fps;
        let rounded_timestamp = (timestamp / frame_duration).round() * frame_duration;

        // Convert timestamp to frame timestamp
        let frame_ts = (rounded_timestamp / self.time_base) as i64;
        let cache_key = (frame_ts, out_w, out_h);

        // Check the inner RGBA cache (CPU output only; GPU frames are cached by VideoManager).
        if !gpu_out {
            if let Some(cached_frame) = self.frame_cache.get(&cache_key) {
                if video_debug() {
                    eprintln!("[Video Timing] Cache hit for ts={:.3}s ({}ms)", timestamp, t_start.elapsed().as_millis());
                }
                return Ok(DecodedFrame::Cpu { rgba: cached_frame.clone(), width: out_w, height: out_h });
            }
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

            if video_debug() {
                eprintln!("[Video Seek] Target: {} | Keyframe(stream): {} | Keyframe(AV): {} | Index size: {}",
                    frame_ts, keyframe_ts_stream, keyframe_ts_av, self.keyframe_positions.len());
            }

            // Reopen input (a fresh BlobReader for packed sources).
            let mut owned = self.source.open()
                .map_err(|e| format!("Failed to reopen video: {}", e))?;

            {
                let input = owned.get();
                // Seek directly to the keyframe with a 1-unit window
                // Can't use keyframe_ts..keyframe_ts (empty) or ..= (not supported)
                input.seek(keyframe_ts_av, keyframe_ts_av..(keyframe_ts_av + 1))
                    .map_err(|e| format!("Seek failed: {}", e))?;

                if video_debug() {
                    eprintln!("[Video Timing] Seek call took {}ms", t_seek_start.elapsed().as_millis());
                }

                let context_decoder = ffmpeg::codec::context::Context::from_parameters(
                    input.streams().best(ffmpeg::media::Type::Video).unwrap().parameters()
                ).map_err(|e| e.to_string())?;

                let mut dec_ctx = context_decoder.decoder();
                if hw {
                    // Attach the VAAPI device + format selector before opening so the decoder
                    // produces hardware surfaces.
                    unsafe {
                        let ctx = dec_ctx.as_mut_ptr();
                        let hwdev = self.hw_device.unwrap().0 as *mut ffmpeg::ffi::AVBufferRef;
                        (*ctx).hw_device_ctx = ffmpeg::ffi::av_buffer_ref(hwdev);
                        (*ctx).get_format = Some(get_vaapi_format);
                    }
                }
                match dec_ctx.video() {
                    Ok(decoder) => self.decoder = Some(decoder),
                    Err(e) if hw => {
                        // Hardware decode unavailable for this clip — fall back to software. This
                        // frame fails; the next call rebuilds a software decoder.
                        eprintln!("[Video] hardware decode unavailable ({e}); falling back to software");
                        self.hw_failed = true;
                        self.decoder = None;
                        return Err(format!("hw decode init failed: {e}"));
                    }
                    Err(e) => return Err(e.to_string()),
                }
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
        let mut best_gpu: Option<GpuVideoFrame> = None;
        let mut best_frame_ts: Option<i64> = None;
        let t_decode_start = Instant::now();
        let mut decode_count = 0;
        let mut scale_time_ms = 0u128;
        let mut hw_import_failed = false;

        'decode: for (stream, packet) in input.packets() {
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
                        if gpu_out {
                            // Hardware + GPU consumer: import the VAAPI surface as wgpu NV12 textures
                            // (no CPU copy).
                            // VAAPI hw frames often don't carry the stream's colour tags, so the
                            // importer (which only sees the frame) would mis-detect transfer/gamut.
                            // Copy the authoritative values from the codec context (parsed from the
                            // bitstream) onto the frame when it left them unspecified.
                            unsafe {
                                use ffmpeg::ffi::*;
                                let fp = frame.as_mut_ptr();
                                let cp = decoder.as_ptr();
                                if (*fp).color_trc == AVColorTransferCharacteristic::AVCOL_TRC_UNSPECIFIED {
                                    (*fp).color_trc = (*cp).color_trc;
                                }
                                if (*fp).color_primaries == AVColorPrimaries::AVCOL_PRI_UNSPECIFIED {
                                    (*fp).color_primaries = (*cp).color_primaries;
                                }
                                if (*fp).colorspace == AVColorSpace::AVCOL_SPC_UNSPECIFIED {
                                    (*fp).colorspace = (*cp).colorspace;
                                }
                                if (*fp).color_range == AVColorRange::AVCOL_RANGE_UNSPECIFIED {
                                    (*fp).color_range = (*cp).color_range;
                                }
                            }
                            let importer = self.importer.as_ref().unwrap();
                            match unsafe { importer.import(frame.as_mut_ptr() as *mut std::ffi::c_void) } {
                                Some(gpu) => {
                                    best_gpu = Some(gpu);
                                    best_frame_ts = Some(current_frame_ts);
                                }
                                None => {
                                    // Import failed → fall back to software for this clip.
                                    self.hw_failed = true;
                                    hw_import_failed = true;
                                    break 'decode;
                                }
                            }
                        } else {
                            let t_scale_start = Instant::now();

                            // A hardware decoder produces VAAPI surfaces; a CPU consumer (export)
                            // downloads to system memory first, then swscales like the software path.
                            let downloaded;
                            let src: &ffmpeg::util::frame::Video = if hw {
                                let mut dl = ffmpeg::util::frame::Video::empty();
                                let r = unsafe {
                                    ffmpeg::ffi::av_hwframe_transfer_data(dl.as_mut_ptr(), frame.as_ptr(), 0)
                                };
                                if r < 0 {
                                    return Err(format!("av_hwframe_transfer_data failed: {r}"));
                                }
                                downloaded = dl;
                                &downloaded
                            } else {
                                &frame
                            };

                            // Reuse the RGBA scaler across frames; rebuild only if the input
                            // format/size or the requested output size changes.
                            let need_new = match &self.scaler {
                                Some((fmt, w, h, ow, oh, _)) => {
                                    *fmt != src.format() || *w != src.width() || *h != src.height()
                                        || *ow != out_w || *oh != out_h
                                }
                                None => true,
                            };
                            if need_new {
                                let ctx = ffmpeg::software::scaling::context::Context::get(
                                    src.format(),
                                    src.width(),
                                    src.height(),
                                    ffmpeg::format::Pixel::RGBA,
                                    out_w,
                                    out_h,
                                    ffmpeg::software::scaling::flag::Flags::BILINEAR,
                                ).map_err(|e| e.to_string())?;
                                self.scaler = Some((src.format(), src.width(), src.height(), out_w, out_h, SendScaler(ctx)));
                            }
                            let scaler = &mut self.scaler.as_mut().unwrap().5.0;

                            let mut rgb_frame = ffmpeg::util::frame::Video::empty();
                            scaler.run(src, &mut rgb_frame)
                                .map_err(|e| e.to_string())?;

                            // Remove stride padding to create tightly packed RGBA data
                            let width = out_w as usize;
                            let height = out_h as usize;
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
                    }

                    // If we've reached or passed the target timestamp, we can stop
                    if current_frame_ts >= frame_ts {
                        if video_debug() {
                            let total_time = t_start.elapsed().as_millis();
                            let decode_time = t_decode_start.elapsed().as_millis();
                            eprintln!("[Video Timing] ts={:.3}s | Decoded {} frames in {}ms | Scale: {}ms | Total: {}ms | {}",
                                timestamp, decode_count, decode_time, scale_time_ms, total_time, if hw { "hw" } else { "sw" });
                        }
                        if gpu_out {
                            if let Some(gpu) = best_gpu.take() {
                                return Ok(DecodedFrame::Gpu(gpu));
                            }
                        } else if let Some(data) = best_frame_data {
                            self.frame_cache.put(cache_key, data.clone());
                            return Ok(DecodedFrame::Cpu { rgba: data, width: out_w, height: out_h });
                        }
                        break 'decode;
                    }
                }
            }
        }

        // Reached EOF without hitting the target, or HW import failed mid-stream.
        if hw_import_failed {
            self.decoder = None; // force a software rebuild next call (decoder borrow ended here)
            self.input = None;
            return Err("hardware frame import failed; retrying software".to_string());
        }
        // EOF: return the closest frame we found, if any.
        if gpu_out {
            if let Some(gpu) = best_gpu.take() {
                return Ok(DecodedFrame::Gpu(gpu));
            }
        } else if let Some(data) = best_frame_data {
            self.frame_cache.put(cache_key, data.clone());
            return Ok(DecodedFrame::Cpu { rgba: data, width: out_w, height: out_h });
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
        // Decode at the thumbnail width (large height so width is the constraint), capped to native.
        // Thumbnail decoders are always software (no hardware importer).
        if let Ok(DecodedFrame::Cpu { rgba, .. }) = decoder.get_frame(ks, thumb_width, 100_000, false) {
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
    /// CPU-decoded sRGB RGBA8 (software path). Empty when `gpu` is `Some`.
    pub rgba_data: Arc<Vec<u8>>,
    /// Hardware-decoded frame living on the GPU (NV12 plane textures). When `Some`, the compositor
    /// samples it directly and `rgba_data` is empty.
    pub gpu: Option<GpuVideoFrame>,
    pub timestamp: f64,
}

/// A hardware-decoded video frame on the GPU: two NV12 plane textures (Y = R8, UV = RG8) imported
/// from a VAAPI DMA-BUF on the editor's shared wgpu device. The compositor samples these directly
/// (NV12→RGB), no CPU copy.
#[derive(Clone, Debug)]
pub struct GpuVideoFrame {
    pub y: Arc<wgpu::Texture>,
    pub uv: Arc<wgpu::Texture>,
    pub width: u32,
    pub height: u32,
    /// Source YUV range: true = full/PC (0–255), false = limited/TV (16–235). Drives the NV12→RGB
    /// offset/scale in the compositor.
    pub full_range: bool,
    /// Y'CbCr→R'G'B' matrix coefficients derived from the frame's colorspace (BT.709/601/2020),
    /// so SD (BT.601) and HD/UHD clips each convert correctly: `[Cr→R, Cb→G, Cr→G, Cb→B]`.
    ///   R = Y + c[0]·Cr,  G = Y + c[1]·Cb + c[2]·Cr,  B = Y + c[3]·Cb
    pub coeffs: [f32; 4],
    /// Opto-electronic transfer of the encoded R'G'B' — the compositor applies the matching EOTF to
    /// reach scene-linear (graphics white = 1.0). HDR (PQ/HLG) values exceed 1.0.
    pub transfer: VideoTransfer,
    /// Colour primaries; BT.2020 is gamut-mapped to the compositor's BT.709 space in linear light.
    pub primaries: VideoPrimaries,
}

/// Transfer characteristic of a decoded video frame (selects the EOTF in the NV12→linear pass).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoTransfer {
    /// SDR gamma (BT.709/sRGB/601/gamma22) — approximated by the sRGB EOTF.
    Gamma,
    /// SMPTE ST 2084 (PQ) — absolute, normalized so 203 nits (graphics white) = 1.0.
    Pq,
    /// ARIB STD-B67 (HLG) — scene-referred, normalized so reference white ≈ 1.0.
    Hlg,
}

/// Colour primaries of a decoded video frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoPrimaries {
    /// BT.709 / sRGB (also used for BT.601, whose primaries differ only slightly).
    Bt709,
    /// BT.2020 (wide gamut) — converted to BT.709 in linear light by the compositor.
    Bt2020,
}

/// Y'CbCr→R'G'B' matrix coefficients (`[Cr→R, Cb→G, Cr→G, Cb→B]`) from the luma weights `kr`/`kb`
/// (`kg = 1−kr−kb`). BT.709 → `[1.5748, −0.1873, −0.4681, 1.8556]`.
pub fn ycbcr_coeffs(kr: f32, kb: f32) -> [f32; 4] {
    let kg = 1.0 - kr - kb;
    [
        2.0 * (1.0 - kr),
        -2.0 * kb * (1.0 - kb) / kg,
        -2.0 * kr * (1.0 - kr) / kg,
        2.0 * (1.0 - kb),
    ]
}

/// Imports a decoded VAAPI surface (a `*mut AVFrame`, passed as an opaque pointer so core needn't
/// reference the GPU crate's ffmpeg-sys types) into [`GpuVideoFrame`] textures on the shared device.
/// Implemented by the editor; `gpu-video-encoder` does the actual DMA-BUF import.
pub trait HwVideoImporter: Send + Sync {
    /// # Safety
    /// `av_frame` must be a valid `*mut ffmpeg_sys_next::AVFrame` holding a VAAPI surface.
    unsafe fn import(&self, av_frame: *mut std::ffi::c_void) -> Option<GpuVideoFrame>;
}

/// Opaque handle to the FFmpeg VAAPI hardware device (`*mut AVBufferRef`), created by the editor and
/// handed to core so decoders can attach it as `hw_device_ctx`. Core never frees it (the editor owns
/// it for the app's lifetime).
#[derive(Clone, Copy)]
pub struct HwDeviceHandle(pub *mut std::ffi::c_void);
// SAFETY: the pointer is an AVBufferRef whose refcount is managed by FFmpeg; we only `av_buffer_ref`
// it (atomic) and never free it, so sharing the handle across threads is sound.
unsafe impl Send for HwDeviceHandle {}
unsafe impl Sync for HwDeviceHandle {}

/// Approximate resident bytes of a cached frame for the byte budget: the CPU RGBA buffer, or for a
/// GPU (NV12) frame ~`w*h*3/2` of VRAM, so GPU frames stay bounded too.
fn frame_cache_bytes(frame: &VideoFrame) -> usize {
    if frame.gpu.is_some() {
        (frame.width as usize * frame.height as usize * 3) / 2
    } else {
        frame.rgba_data.len()
    }
}

/// Manages video decoders and frame caching for multiple video clips
pub struct VideoManager {
    /// Pool of video decoders, one per clip
    decoders: HashMap<Uuid, Arc<Mutex<VideoDecoder>>>,

    /// Frame cache: (clip_id, timestamp_ms) -> frame. Stores decoded RGBA for
    /// zero-copy rendering. Bounded by a **byte budget** (not a frame count, which
    /// would be unsafe across resolutions — a 4K frame is ~33MB vs ~2MB at 800x600)
    /// so playback of arbitrarily long video never grows unbounded.
    frame_cache: LruCache<(Uuid, i64, u32, u32, bool), Arc<VideoFrame>>,
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

    /// Hardware (VAAPI) decode, injected by the editor once the shared device is up. When set, each
    /// decoder attaches the VAAPI device and imports frames as GPU textures via `hw_importer`.
    hw_device: Option<HwDeviceHandle>,
    hw_importer: Option<Arc<dyn HwVideoImporter>>,
    /// Whether the current render pass can consume GPU textures (preview = true; export = false,
    /// since it composites on a different device → a hardware decoder downloads to CPU instead).
    /// Set by the render caller before each pass.
    render_hardware_ok: bool,
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
            hw_device: None,
            hw_importer: None,
            render_hardware_ok: true,
        }
    }

    /// Set whether the upcoming render pass can consume GPU video textures (preview = true; export =
    /// false). Call before `render_document_for_compositing`.
    pub fn set_render_hardware_ok(&mut self, ok: bool) {
        self.render_hardware_ok = ok;
    }

    /// Enable hardware (VAAPI) decode for all clips. Injected by the editor once the shared wgpu
    /// device is active; `hw_device` is the FFmpeg VAAPI device and `importer` imports decoded
    /// surfaces as GPU textures on that device. Applies to existing and future decoders. Clears the
    /// frame cache (cached CPU frames would otherwise hide the new GPU frames).
    pub fn set_hardware_decode(&mut self, hw_device: HwDeviceHandle, importer: Arc<dyn HwVideoImporter>) {
        self.hw_device = Some(hw_device);
        self.hw_importer = Some(Arc::clone(&importer));
        for dec in self.decoders.values() {
            if let Ok(mut d) = dec.lock() {
                d.set_hardware(hw_device, Arc::clone(&importer));
            }
        }
        self.frame_cache.clear();
        self.frame_cache_bytes = 0;
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
        let mut decoder = VideoDecoder::new(
            source,
            self.cache_size,
            Some(target_width),
            Some(target_height),
            false, // Don't build keyframe index synchronously
        )?;

        // Inherit hardware decode if the manager has it configured.
        if let (Some(hw), Some(imp)) = (self.hw_device, &self.hw_importer) {
            decoder.set_hardware(hw, Arc::clone(imp));
        }

        // Store decoder in pool
        self.decoders.insert(clip_id, Arc::new(Mutex::new(decoder)));

        Ok(metadata)
    }

    /// Get a decoded frame for a specific clip at a specific timestamp
    ///
    /// Returns None if the clip is not loaded or decoding fails. Frames are cached.
    /// Whether a hardware decoder returns a GPU texture or downloads to CPU RGBA depends on
    /// [`set_render_hardware_ok`](Self::set_render_hardware_ok), set per render pass (true for the
    /// preview, false for export, which composites on a different device).
    pub fn get_frame(&mut self, clip_id: &Uuid, timestamp: f64, target_w: u32, target_h: u32) -> Option<Arc<VideoFrame>> {
        // Whether this pass wants (and can produce) a GPU frame. Gated on HW being configured at all
        // so that with software-only decode preview and export share one cache entry (no double-cache).
        let want_gpu = self.render_hardware_ok && self.hw_device.is_some();
        self.get_frame_inner(clip_id, timestamp, target_w, target_h, want_gpu)
    }

    /// Like [`get_frame`](Self::get_frame) but always returns a CPU (RGBA) frame, ignoring the
    /// render-pass hardware flag. For consumers that need pixel bytes (thumbnails, image readback)
    /// regardless of whether a render pass last enabled GPU frames.
    pub fn get_frame_cpu(&mut self, clip_id: &Uuid, timestamp: f64, target_w: u32, target_h: u32) -> Option<Arc<VideoFrame>> {
        self.get_frame_inner(clip_id, timestamp, target_w, target_h, false)
    }

    fn get_frame_inner(&mut self, clip_id: &Uuid, timestamp: f64, target_w: u32, target_h: u32, want_gpu: bool) -> Option<Arc<VideoFrame>> {
        // The cache key includes (target size, want_gpu): preview (GPU, preview res) and export
        // (CPU, export res) request the same clip/time and must not collide or cross representation.
        let timestamp_ms = (timestamp * 1000.0) as i64;
        let cache_key = (*clip_id, timestamp_ms, target_w, target_h, want_gpu);

        // Check frame cache first
        if let Some(cached_frame) = self.frame_cache.get(&cache_key) {
            return Some(Arc::clone(cached_frame));
        }

        // Get decoder for this clip. Clone the Arc so we don't hold a borrow of
        // `self.decoders` across the `&mut self` cache insert below.
        let decoder_arc = Arc::clone(self.decoders.get(clip_id)?);
        let mut decoder = decoder_arc.lock().ok()?;

        // Decode the frame at the requested target (capped to native by the decoder).
        let decoded = decoder.get_frame(timestamp, target_w, target_h, want_gpu).ok()?;
        drop(decoder); // release the lock before touching `self`

        // Create VideoFrame and cache it.
        let frame = Arc::new(match decoded {
            DecodedFrame::Cpu { rgba, width, height } => VideoFrame {
                width,
                height,
                rgba_data: Arc::new(rgba),
                gpu: None,
                timestamp,
            },
            DecodedFrame::Gpu(gpu) => VideoFrame {
                width: gpu.width,
                height: gpu.height,
                rgba_data: Arc::new(Vec::new()),
                gpu: Some(gpu),
                timestamp,
            },
        });

        self.cache_frame(cache_key, Arc::clone(&frame));

        Some(frame)
    }

    /// Insert a frame into the byte-budgeted cache, evicting least-recently-used
    /// frames until the total is within [`FRAME_CACHE_BYTE_BUDGET`].
    fn cache_frame(&mut self, key: (Uuid, i64, u32, u32, bool), frame: Arc<VideoFrame>) {
        let bytes = frame_cache_bytes(&frame);
        if let Some(old) = self.frame_cache.put(key, frame) {
            self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(frame_cache_bytes(&old));
        }
        self.frame_cache_bytes += bytes;
        // Keep at least one frame resident even if it alone exceeds the budget.
        while self.frame_cache_bytes > FRAME_CACHE_BYTE_BUDGET && self.frame_cache.len() > 1 {
            if let Some((_, evicted)) = self.frame_cache.pop_lru() {
                self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(frame_cache_bytes(&evicted));
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
        let keys: Vec<(Uuid, i64, u32, u32, bool)> = self
            .frame_cache
            .iter()
            .filter(|((id, _, _, _, _), _)| id == clip_id)
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            if let Some(frame) = self.frame_cache.pop(&key) {
                self.frame_cache_bytes = self.frame_cache_bytes.saturating_sub(frame_cache_bytes(&frame));
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
