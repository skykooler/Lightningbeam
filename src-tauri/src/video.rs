use std::sync::{Arc, Mutex};
use std::num::NonZeroUsize;
use ffmpeg_next as ffmpeg;
use lru::LruCache;
use daw_backend::WaveformPeak;
use image::RgbaImage;
use tauri::Manager;

#[derive(serde::Serialize, Clone)]
pub struct VideoFileMetadata {
    pub pool_index: usize,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration: f64,
    pub has_audio: bool,
    pub audio_pool_index: Option<usize>,
    pub audio_duration: Option<f64>,
    pub audio_sample_rate: Option<u32>,
    pub audio_channels: Option<u32>,
    pub audio_waveform: Option<Vec<WaveformPeak>>,
    pub codec_name: String,
    pub is_browser_compatible: bool,
    pub http_url: Option<String>,  // HTTP URL to stream video (if compatible or transcode complete)
    pub transcoding: bool,  // True if currently transcoding
}

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
}

impl VideoDecoder {
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
        })
    }

    fn get_frame(&mut self, timestamp: f64) -> Result<Vec<u8>, String> {
        use std::time::Instant;
        let t_start = Instant::now();

        // Convert timestamp to frame timestamp
        let frame_ts = (timestamp / self.time_base) as i64;

        // Check cache
        if let Some(cached_frame) = self.frame_cache.get(&frame_ts) {
            eprintln!("[Video Timing] Cache hit for ts={:.3}s ({}ms)", timestamp, t_start.elapsed().as_millis());
            return Ok(cached_frame.clone());
        }

        let _t_after_cache = Instant::now();

        // Determine if we need to seek
        // Seek if: no decoder open, going backwards, or jumping forward more than 2 seconds
        let need_seek = self.decoder.is_none()
            || frame_ts < self.last_decoded_ts
            || frame_ts > self.last_decoded_ts + (2.0 / self.time_base) as i64;

        if need_seek {
            let t_seek_start = Instant::now();

            // Reopen input
            let mut input = ffmpeg::format::input(&self.path)
                .map_err(|e| format!("Failed to reopen video: {}", e))?;

            // Seek to timestamp
            input.seek(frame_ts, ..frame_ts)
                .map_err(|e| format!("Seek failed: {}", e))?;

            let context_decoder = ffmpeg::codec::context::Context::from_parameters(
                input.streams().best(ffmpeg::media::Type::Video).unwrap().parameters()
            ).map_err(|e| e.to_string())?;

            let decoder = context_decoder.decoder().video()
                .map_err(|e| e.to_string())?;

            self.input = Some(input);
            self.decoder = Some(decoder);
            self.last_decoded_ts = -1; // Reset since we seeked

            eprintln!("[Video Timing] Seek took {}ms", t_seek_start.elapsed().as_millis());
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

use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone)]
pub struct TranscodeJob {
    pub pool_index: usize,
    pub input_path: String,
    pub output_path: String,
    pub http_url: Option<String>,  // HTTP URL when transcode completes
    pub progress: f32,  // 0.0 to 1.0
    pub completed: bool,
}

pub struct VideoState {
    pool: Vec<Arc<Mutex<VideoDecoder>>>,
    next_pool_index: usize,
    cache_size: usize,
    transcode_jobs: Arc<Mutex<HashMap<usize, TranscodeJob>>>,  // pool_index -> job
}

impl Default for VideoState {
    fn default() -> Self {
        Self {
            pool: Vec::new(),
            next_pool_index: 0,
            cache_size: 20, // Default cache size
            transcode_jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[tauri::command]
pub async fn video_load_file(
    video_state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    audio_state: tauri::State<'_, Arc<Mutex<crate::audio::AudioState>>>,
    video_server: tauri::State<'_, Arc<Mutex<crate::video_server::VideoServer>>>,
    path: String,
) -> Result<VideoFileMetadata, String> {
    eprintln!("[Video] Loading file: {}", path);

    ffmpeg::init().map_err(|e| e.to_string())?;

    // Open input to check for audio stream
    let mut input = ffmpeg::format::input(&path)
        .map_err(|e| format!("Failed to open video: {}", e))?;

    let audio_stream_opt = input.streams()
        .best(ffmpeg::media::Type::Audio);

    let has_audio = audio_stream_opt.is_some();

    // Extract audio if present
    let (audio_pool_index, audio_duration, audio_sample_rate, audio_channels, audio_waveform) = if has_audio {
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
                    // Convert audio to f32 planar format
                    let format = audio_frame.format();
                    let frame_channels = audio_frame.channels() as usize;

                    // Create resampler to convert to f32 planar
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

        // Calculate audio duration
        let total_samples_per_channel = audio_samples.len() / channels as usize;
        let audio_duration = total_samples_per_channel as f64 / sample_rate as f64;

        // Generate waveform
        let target_peaks = ((audio_duration * 300.0) as usize).clamp(1000, 20000);
        let waveform = generate_waveform(&audio_samples, channels, target_peaks);

        // Send audio to DAW backend
        let mut audio_state_guard = audio_state.lock().unwrap();
        let audio_pool_index = audio_state_guard.next_pool_index;
        audio_state_guard.next_pool_index += 1;

        if let Some(controller) = &mut audio_state_guard.controller {
            controller.add_audio_file(
                path.clone(),
                audio_samples,
                channels,
                sample_rate,
            );
        }
        drop(audio_state_guard);

        (Some(audio_pool_index), Some(audio_duration), Some(sample_rate), Some(channels), Some(waveform))
    } else {
        (None, None, None, None, None)
    };

    // Detect video codec
    let video_stream = input.streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or("No video stream found")?;

    let codec_id = video_stream.parameters().id();
    let codec_name = ffmpeg::codec::Id::name(&codec_id).to_string();

    // Check if codec is browser-compatible (can play directly)
    // Browsers support: H.264/AVC, VP8, VP9, AV1 (limited)
    let is_browser_compatible = matches!(
        codec_id,
        ffmpeg::codec::Id::H264 |
        ffmpeg::codec::Id::VP8 |
        ffmpeg::codec::Id::VP9 |
        ffmpeg::codec::Id::AV1
    );

    eprintln!("[Video Codec] {} - Browser compatible: {}", codec_name, is_browser_compatible);

    // Create video decoder with max dimensions for playback (800x600)
    // This scales down high-res videos to reduce data transfer
    let mut video_state_guard = video_state.lock().unwrap();
    let pool_index = video_state_guard.next_pool_index;
    video_state_guard.next_pool_index += 1;

    let decoder = VideoDecoder::new(path.clone(), video_state_guard.cache_size, Some(800), Some(600))?;

    // Add file to HTTP server if browser-compatible
    let http_url = if is_browser_compatible {
        let server = video_server.lock().unwrap();
        let url_path = format!("/video/{}", pool_index);
        server.add_file(url_path.clone(), PathBuf::from(&path));
        let http_url = server.get_url(&url_path);
        eprintln!("[Video] Browser-compatible, serving at: {}", http_url);
        Some(http_url)
    } else {
        None
    };

    let metadata = VideoFileMetadata {
        pool_index,
        width: decoder.output_width,  // Return scaled dimensions to JS
        height: decoder.output_height,
        fps: decoder.fps,
        duration: decoder.duration,
        has_audio,
        audio_pool_index,
        audio_duration,
        audio_sample_rate,
        audio_channels,
        audio_waveform,
        codec_name,
        is_browser_compatible,
        http_url,
        transcoding: !is_browser_compatible,
    };

    video_state_guard.pool.push(Arc::new(Mutex::new(decoder)));

    // Start background transcoding if not browser-compatible
    if !is_browser_compatible {
        eprintln!("[Video Transcode] Starting background transcode for pool_index {}", pool_index);
        let jobs = video_state_guard.transcode_jobs.clone();
        let input_path = path.clone();
        let pool_idx = pool_index;
        let server = video_server.inner().clone();

        tauri::async_runtime::spawn(async move {
            if let Err(e) = start_transcode(jobs, pool_idx, input_path, server).await {
                eprintln!("[Video Transcode] Failed: {}", e);
            }
        });
    }

    Ok(metadata)
}

// Background transcode to WebM/VP9 for browser compatibility
async fn start_transcode(
    jobs: Arc<Mutex<HashMap<usize, TranscodeJob>>>,
    pool_index: usize,
    input_path: String,
    video_server: Arc<Mutex<crate::video_server::VideoServer>>,
) -> Result<(), String> {
    use std::process::Command;

    // Generate output path in system cache directory
    let cache_dir = std::env::temp_dir().join("lightningbeam_transcoded");
    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;

    let input_file = PathBuf::from(&input_path);
    let file_stem = input_file.file_stem()
        .ok_or("Invalid input path")?
        .to_string_lossy();
    let output_path = cache_dir.join(format!("{}_{}.webm", file_stem, pool_index));

    // Create job entry
    {
        let mut jobs_guard = jobs.lock().unwrap();
        jobs_guard.insert(pool_index, TranscodeJob {
            pool_index,
            input_path: input_path.clone(),
            output_path: output_path.to_string_lossy().to_string(),
            http_url: None,
            progress: 0.0,
            completed: false,
        });
    }

    eprintln!("[Video Transcode] Output: {}", output_path.display());

    // Run FFmpeg transcode command
    // Using VP9 codec with CRF 30 (good quality/size balance) and fast encoding
    let output = Command::new("ffmpeg")
        .args(&[
            "-i", &input_path,
            "-c:v", "libvpx-vp9",  // VP9 video codec
            "-crf", "30",           // Quality (lower = better, 23-32 recommended)
            "-b:v", "0",            // Use CRF mode
            "-threads", "4",        // Use 4 threads
            "-row-mt", "1",         // Enable row-based multithreading
            "-speed", "4",          // Encoding speed (0=slowest/best, 4=good balance)
            "-c:a", "libopus",      // Opus audio codec (best for WebM)
            "-b:a", "128k",         // Audio bitrate
            "-y",                   // Overwrite output
            output_path.to_str().ok_or("Invalid output path")?,
        ])
        .output()
        .map_err(|e| format!("Failed to spawn ffmpeg: {}", e))?;

    if output.status.success() {
        eprintln!("[Video Transcode] Completed: {}", output_path.display());

        // Add transcoded file to HTTP server
        let server = video_server.lock().unwrap();
        let url_path = format!("/video/{}", pool_index);
        server.add_file(url_path.clone(), output_path.clone());
        let http_url = server.get_url(&url_path);
        eprintln!("[Video Transcode] Serving at: {}", http_url);
        drop(server);

        // Mark as completed and store HTTP URL
        let mut jobs_guard = jobs.lock().unwrap();
        if let Some(job) = jobs_guard.get_mut(&pool_index) {
            job.progress = 1.0;
            job.completed = true;
            job.http_url = Some(http_url);
        }
        eprintln!("[Video Transcode] Job completed for pool_index {}", pool_index);
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("[Video Transcode] FFmpeg error: {}", stderr);
        Err(format!("FFmpeg failed: {}", stderr))
    }
}

// Get transcode status for a pool index
#[tauri::command]
pub async fn video_get_transcode_status(
    video_state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    pool_index: usize,
) -> Result<Option<(String, f32, bool, Option<String>)>, String> {
    let state = video_state.lock().unwrap();
    let jobs = state.transcode_jobs.lock().unwrap();

    if let Some(job) = jobs.get(&pool_index) {
        Ok(Some((job.output_path.clone(), job.progress, job.completed, job.http_url.clone())))
    } else {
        Ok(None)
    }
}

// Add a video file to asset protocol scope so browser can access it
#[tauri::command]
pub async fn video_allow_asset(
    app: tauri::AppHandle,
    path: String,
) -> Result<(), String> {
    use tauri_plugin_fs::FsExt;

    let file_path = PathBuf::from(&path);

    // Add to FS scope
    let fs_scope = app.fs_scope();
    fs_scope.allow_file(&file_path)
        .map_err(|e| format!("Failed to allow file in fs scope: {}", e))?;

    // Add to asset protocol scope
    let asset_scope = app.asset_protocol_scope();
    asset_scope.allow_file(&file_path)
        .map_err(|e| format!("Failed to allow file in asset scope: {}", e))?;

    eprintln!("[Video] Added to asset scope: {}", path);
    Ok(())
}

fn generate_waveform(audio_data: &[f32], channels: u32, target_peaks: usize) -> Vec<WaveformPeak> {
    let total_samples = audio_data.len();
    let samples_per_channel = total_samples / channels as usize;
    let samples_per_peak = (samples_per_channel / target_peaks).max(1);

    let mut waveform = Vec::new();

    for peak_idx in 0..target_peaks {
        let start_sample = peak_idx * samples_per_peak;
        let end_sample = ((peak_idx + 1) * samples_per_peak).min(samples_per_channel);

        if start_sample >= samples_per_channel {
            break;
        }

        let mut min_val = 0.0f32;
        let mut max_val = 0.0f32;

        for sample_idx in start_sample..end_sample {
            // Average across channels
            let mut channel_sum = 0.0f32;
            for ch in 0..channels as usize {
                let idx = sample_idx * channels as usize + ch;
                if idx < total_samples {
                    channel_sum += audio_data[idx];
                }
            }
            let avg_sample = channel_sum / channels as f32;

            min_val = min_val.min(avg_sample);
            max_val = max_val.max(avg_sample);
        }

        waveform.push(WaveformPeak {
            min: min_val,
            max: max_val,
        });
    }

    waveform
}

#[tauri::command]
pub async fn video_get_frame(
    state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    pool_index: usize,
    timestamp: f64,
    use_jpeg: bool,
    channel: tauri::ipc::Channel,
) -> Result<(), String> {
    use std::time::Instant;

    let t_total_start = Instant::now();

    let t_lock_start = Instant::now();
    let video_state = state.lock().unwrap();

    let decoder = video_state.pool.get(pool_index)
        .ok_or("Invalid pool index")?
        .clone();

    drop(video_state);

    let mut decoder = decoder.lock().unwrap();
    let t_lock_end = Instant::now();

    let t_decode_start = Instant::now();
    let frame_data = decoder.get_frame(timestamp)?;
    let t_decode_end = Instant::now();

    let t_compress_start = Instant::now();
    let data_to_send = if use_jpeg {
        // Get frame dimensions from decoder
        let width = decoder.output_width;
        let height = decoder.output_height;

        // Create image from raw RGBA data
        let img = RgbaImage::from_raw(width, height, frame_data)
            .ok_or("Failed to create image from frame data")?;

        // Convert RGBA to RGB (JPEG doesn't support alpha)
        let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();

        // Encode to JPEG with quality 85 (good balance of size/quality)
        let mut jpeg_data = Vec::new();
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 85);
        encoder.encode(
            rgb_img.as_raw(),
            rgb_img.width(),
            rgb_img.height(),
            image::ColorType::Rgb8
        ).map_err(|e| format!("JPEG encoding failed: {}", e))?;

        jpeg_data
    } else {
        frame_data
    };
    let t_compress_end = Instant::now();

    // Drop decoder lock before sending to avoid blocking
    drop(decoder);

    let t_send_start = Instant::now();
    // Send binary data through channel (bypasses JSON serialization)
    // InvokeResponseBody::Raw sends raw binary data without JSON encoding
    channel.send(tauri::ipc::InvokeResponseBody::Raw(data_to_send.clone()))
        .map_err(|e| format!("Channel send error: {}", e))?;
    let t_send_end = Instant::now();

    let t_total_end = Instant::now();

    // Detailed profiling
    let lock_time = t_lock_end.duration_since(t_lock_start).as_micros();
    let decode_time = t_decode_end.duration_since(t_decode_start).as_micros();
    let compress_time = t_compress_end.duration_since(t_compress_start).as_micros();
    let send_time = t_send_end.duration_since(t_send_start).as_micros();
    let total_time = t_total_end.duration_since(t_total_start).as_micros();

    let size_kb = data_to_send.len() / 1024;
    let mode = if use_jpeg { "JPEG" } else { "RAW" };

    eprintln!("[Video Profile {}] Size: {}KB | Lock: {}μs | Decode: {}μs | Compress: {}μs | Send: {}μs | Total: {}μs",
        mode, size_kb, lock_time, decode_time, compress_time, send_time, total_time);

    Ok(())
}

#[tauri::command]
pub async fn video_set_cache_size(
    state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    cache_size: usize,
) -> Result<(), String> {
    let mut video_state = state.lock().unwrap();
    video_state.cache_size = cache_size;
    Ok(())
}

#[tauri::command]
pub async fn video_get_pool_info(
    state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    pool_index: usize,
) -> Result<(u32, u32, f64), String> {
    let video_state = state.lock().unwrap();
    let decoder = video_state.pool.get(pool_index)
        .ok_or("Invalid pool index")?
        .lock().unwrap();

    Ok((
        decoder.output_width,   // Return scaled dimensions
        decoder.output_height,
        decoder.fps
    ))
}

// Benchmark command to test IPC performance with various payload sizes
#[tauri::command]
pub async fn video_ipc_benchmark(
    size_bytes: usize,
    channel: tauri::ipc::Channel,
) -> Result<(), String> {
    use std::time::Instant;

    let t_start = Instant::now();

    // Create dummy data of requested size
    let data = vec![0u8; size_bytes];

    let t_after_alloc = Instant::now();

    // Send through channel
    channel.send(tauri::ipc::InvokeResponseBody::Raw(data))
        .map_err(|e| format!("Channel send error: {}", e))?;

    let t_after_send = Instant::now();

    let alloc_time = t_after_alloc.duration_since(t_start).as_micros();
    let send_time = t_after_send.duration_since(t_after_alloc).as_micros();
    let total_time = t_after_send.duration_since(t_start).as_micros();

    eprintln!("[IPC Benchmark Rust] Size: {}KB | Alloc: {}μs | Send: {}μs | Total: {}μs",
        size_bytes / 1024, alloc_time, send_time, total_time);

    Ok(())
}

// Batch frame request - get multiple frames in one IPC call
#[tauri::command]
pub async fn video_get_frames_batch(
    state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    pool_index: usize,
    timestamps: Vec<f64>,
    use_jpeg: bool,
    channel: tauri::ipc::Channel,
) -> Result<(), String> {
    use std::time::Instant;

    let t_total_start = Instant::now();

    let video_state = state.lock().unwrap();
    let decoder = video_state.pool.get(pool_index)
        .ok_or("Invalid pool index")?
        .clone();
    drop(video_state);

    let mut decoder = decoder.lock().unwrap();

    // Decode all frames
    let mut all_frames = Vec::new();
    let mut total_decode_time = 0u128;
    let mut total_compress_time = 0u128;

    for timestamp in &timestamps {
        let t_decode_start = Instant::now();
        let frame_data = decoder.get_frame(*timestamp)?;
        let t_decode_end = Instant::now();
        total_decode_time += t_decode_end.duration_since(t_decode_start).as_micros();

        let t_compress_start = Instant::now();
        let data = if use_jpeg {
            let width = decoder.output_width;
            let height = decoder.output_height;
            let img = RgbaImage::from_raw(width, height, frame_data)
                .ok_or("Failed to create image from frame data")?;
            let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();
            let mut jpeg_data = Vec::new();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_data, 85);
            encoder.encode(
                rgb_img.as_raw(),
                rgb_img.width(),
                rgb_img.height(),
                image::ColorType::Rgb8
            ).map_err(|e| format!("JPEG encoding failed: {}", e))?;
            jpeg_data
        } else {
            frame_data
        };
        let t_compress_end = Instant::now();
        total_compress_time += t_compress_end.duration_since(t_compress_start).as_micros();

        all_frames.push(data);
    }

    drop(decoder);

    // Pack all frames into one buffer with metadata
    // Format: [frame_count: u32][frame1_size: u32][frame1_data...][frame2_size: u32][frame2_data...]
    let mut packed_data = Vec::new();
    packed_data.extend_from_slice(&(all_frames.len() as u32).to_le_bytes());

    for frame in &all_frames {
        packed_data.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        packed_data.extend_from_slice(frame);
    }

    let total_size_kb = packed_data.len() / 1024;

    let t_send_start = Instant::now();
    channel.send(tauri::ipc::InvokeResponseBody::Raw(packed_data))
        .map_err(|e| format!("Channel send error: {}", e))?;
    let t_send_end = Instant::now();

    let send_time = t_send_end.duration_since(t_send_start).as_micros();
    let total_time = t_send_end.duration_since(t_total_start).as_micros();

    let mode = if use_jpeg { "JPEG" } else { "RAW" };
    eprintln!("[Video Batch {}] Frames: {} | Size: {}KB | Decode: {}μs | Compress: {}μs | Send: {}μs | Total: {}μs",
        mode, timestamps.len(), total_size_kb, total_decode_time, total_compress_time, send_time, total_time);

    Ok(())
}

/// Stream a decoded video frame over WebSocket (zero-copy performance testing)
#[tauri::command]
pub async fn video_stream_frame(
    video_state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    frame_streamer: tauri::State<'_, Arc<Mutex<crate::frame_streamer::FrameStreamer>>>,
    pool_index: usize,
    timestamp: f64,
) -> Result<(), String> {
    use std::time::Instant;
    let t_start = Instant::now();

    // Get decoder
    let state = video_state.lock().unwrap();
    let decoder = state.pool.get(pool_index)
        .ok_or("Invalid pool index")?
        .clone();
    drop(state);

    // Decode frame
    let mut decoder = decoder.lock().unwrap();
    let width = decoder.output_width;
    let height = decoder.output_height;

    let t_decode_start = Instant::now();
    let rgba_data = decoder.get_frame(timestamp)?;  // Note: get_frame returns RGBA, not RGB
    let t_decode = t_decode_start.elapsed().as_micros();
    drop(decoder);

    // Stream over WebSocket
    let t_stream_start = Instant::now();
    let streamer = frame_streamer.lock().unwrap();
    streamer.send_frame(pool_index, timestamp, width, height, &rgba_data);
    let t_stream = t_stream_start.elapsed().as_micros();
    drop(streamer);

    // Commented out per-frame logging
    // let t_total = t_start.elapsed().as_micros();
    // eprintln!("[Video Stream] Frame {}x{} @ {:.2}s | Decode: {}μs | Stream: {}μs | Total: {}μs",
    //     width, height, timestamp, t_decode, t_stream, t_total);

    Ok(())
}
