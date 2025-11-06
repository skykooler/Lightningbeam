use std::sync::{Arc, Mutex};
use std::num::NonZeroUsize;
use ffmpeg_next as ffmpeg;
use lru::LruCache;
use daw_backend::WaveformPeak;

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

        let t_after_cache = Instant::now();

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

pub struct VideoState {
    pool: Vec<Arc<Mutex<VideoDecoder>>>,
    next_pool_index: usize,
    cache_size: usize,
}

impl Default for VideoState {
    fn default() -> Self {
        Self {
            pool: Vec::new(),
            next_pool_index: 0,
            cache_size: 20, // Default cache size
        }
    }
}

#[tauri::command]
pub async fn video_load_file(
    video_state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    audio_state: tauri::State<'_, Arc<Mutex<crate::audio::AudioState>>>,
    path: String,
) -> Result<VideoFileMetadata, String> {
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

    // Create video decoder with max dimensions for playback (800x600)
    // This scales down high-res videos to reduce data transfer
    let mut video_state_guard = video_state.lock().unwrap();
    let pool_index = video_state_guard.next_pool_index;
    video_state_guard.next_pool_index += 1;

    let decoder = VideoDecoder::new(path, video_state_guard.cache_size, Some(800), Some(600))?;

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
    };

    video_state_guard.pool.push(Arc::new(Mutex::new(decoder)));

    Ok(metadata)
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

// Use a custom serializer wrapper for efficient binary transfer
#[derive(serde::Serialize)]
struct BinaryFrame(#[serde(with = "serde_bytes")] Vec<u8>);

#[tauri::command]
pub async fn video_get_frame(
    state: tauri::State<'_, Arc<Mutex<VideoState>>>,
    pool_index: usize,
    timestamp: f64,
) -> Result<Vec<u8>, String> {
    let video_state = state.lock().unwrap();

    let decoder = video_state.pool.get(pool_index)
        .ok_or("Invalid pool index")?
        .clone();

    drop(video_state);

    let mut decoder = decoder.lock().unwrap();
    decoder.get_frame(timestamp)
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
