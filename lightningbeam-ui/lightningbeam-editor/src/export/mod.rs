//! Export functionality for audio and video
//!
//! This module provides the export orchestrator and progress tracking
//! for exporting audio and video from the timeline.

pub mod audio_exporter;
pub mod dialog;
pub mod image_exporter;
pub mod video_exporter;
pub mod readback_pipeline;
pub mod perf_metrics;
pub mod cpu_yuv_converter;

use lightningbeam_core::export::{AudioExportSettings, ImageExportSettings, VideoExportSettings, ExportProgress};
use lightningbeam_core::document::Document;
use lightningbeam_core::renderer::ImageCache;
use lightningbeam_core::video::VideoManager;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Message sent from main thread to video encoder thread
enum VideoFrameMessage {
    /// YUV420p frame data with frame number and timestamp (GPU-converted)
    Frame {
        frame_num: usize,
        timestamp: f64,
        y_plane: Vec<u8>,
        u_plane: Vec<u8>,
        v_plane: Vec<u8>,
    },
    /// Signal that all frames have been sent
    Done,
}

/// Video export state for incremental rendering
pub struct VideoExportState {
    /// Current frame number being rendered
    current_frame: usize,
    /// Total number of frames to export
    total_frames: usize,
    /// Start time in seconds
    start_time: f64,
    /// End time in seconds
    #[allow(dead_code)]
    end_time: f64,
    /// Frames per second
    framerate: f64,
    /// Export width in pixels
    width: u32,
    /// Export height in pixels
    height: u32,
    /// Channel to send rendered frames to encoder thread
    frame_tx: Option<Sender<VideoFrameMessage>>,
    /// HDR GPU resources for compositing pipeline (effects, color conversion)
    gpu_resources: Option<video_exporter::ExportGpuResources>,
    /// Async triple-buffered readback pipeline for GPU RGBA frames
    readback_pipeline: Option<readback_pipeline::ReadbackPipeline>,
    /// CPU YUV converter for RGBA→YUV420p conversion
    cpu_yuv_converter: Option<cpu_yuv_converter::CpuYuvConverter>,
    /// Frames that have been submitted to GPU but not yet encoded
    frames_in_flight: usize,
    /// Next frame number to send to encoder (for ordering)
    next_frame_to_encode: usize,
    /// Performance metrics for instrumentation
    perf_metrics: Option<perf_metrics::ExportMetrics>,
}

/// State for a single-frame image export (runs on the GPU render thread, one frame per update).
pub struct ImageExportState {
    pub settings: ImageExportSettings,
    pub output_path: PathBuf,
    /// Resolved pixel dimensions (after applying any width/height overrides).
    pub width: u32,
    pub height: u32,
    /// True once rendering has been submitted; the next call reads back and encodes.
    pub rendered: bool,
    /// GPU resources allocated on the first render call.
    pub gpu_resources: Option<video_exporter::ExportGpuResources>,
    /// Output RGBA texture — kept separate from gpu_resources to avoid split-borrow issues.
    pub output_texture: Option<wgpu::Texture>,
    /// View for output_texture.
    pub output_texture_view: Option<wgpu::TextureView>,
    /// Staging buffer for synchronous GPU→CPU readback.
    pub staging_buffer: Option<wgpu::Buffer>,
}

/// Export orchestrator that manages the export process
pub struct ExportOrchestrator {
    /// Channel for receiving progress updates (video or audio-only export)
    progress_rx: Option<Receiver<ExportProgress>>,

    /// Handle to the export thread (video or audio-only export)
    thread_handle: Option<std::thread::JoinHandle<()>>,

    /// Cancel flag
    cancel_flag: Arc<AtomicBool>,

    /// Video export state (if video export is in progress)
    video_state: Option<VideoExportState>,

    /// Parallel audio+video export state
    parallel_export: Option<ParallelExportState>,

    /// Single-frame image export state
    image_state: Option<ImageExportState>,
}

/// State for parallel audio+video export
struct ParallelExportState {
    /// Video progress channel
    video_progress_rx: Receiver<ExportProgress>,
    /// Audio progress channel
    audio_progress_rx: Receiver<ExportProgress>,
    /// Video encoder thread handle (taken when the mux thread is spawned).
    video_thread: Option<std::thread::JoinHandle<()>>,
    /// Audio export thread handle (taken when the mux thread is spawned).
    audio_thread: Option<std::thread::JoinHandle<()>>,
    /// Temporary video file path
    temp_video_path: PathBuf,
    /// Temporary audio file path
    temp_audio_path: PathBuf,
    /// Final output path
    final_output_path: PathBuf,
    /// Latest video progress
    video_progress: Option<ExportProgress>,
    /// Latest audio progress
    audio_progress: Option<ExportProgress>,
    /// Result channel for the background mux. `Some` once muxing has started; the
    /// mux runs off the UI thread so the app stays responsive during finalization.
    mux_rx: Option<Receiver<Result<(), String>>>,
}

impl ExportOrchestrator {
    /// Create a new export orchestrator
    pub fn new() -> Self {
        Self {
            progress_rx: None,
            thread_handle: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            video_state: None,
            parallel_export: None,
            image_state: None,
        }
    }

    /// Start an audio export in the background
    ///
    /// Returns immediately, spawning a background thread for the export.
    /// Use `poll_progress()` to check the export progress.
    pub fn start_audio_export(
        &mut self,
        settings: AudioExportSettings,
        output_path: PathBuf,
        audio_controller: Arc<std::sync::Mutex<daw_backend::EngineController>>,
    ) {
        println!("🔄 [ORCHESTRATOR] start_audio_export called");

        // Create progress channel
        let (tx, rx) = channel();
        self.progress_rx = Some(rx);

        // Reset cancel flag
        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel_flag = Arc::clone(&self.cancel_flag);

        println!("🔄 [ORCHESTRATOR] Spawning background thread...");
        // Spawn background thread
        let handle = std::thread::spawn(move || {
            println!("🧵 [EXPORT THREAD] Background thread started!");
            Self::run_audio_export(
                settings,
                output_path,
                audio_controller,
                tx,
                cancel_flag,
            );
            println!("🧵 [EXPORT THREAD] Background thread finished!");
        });

        self.thread_handle = Some(handle);
        println!("🔄 [ORCHESTRATOR] Thread spawned, returning");
    }

    /// Poll for progress updates
    ///
    /// Returns None if no updates are available.
    /// Returns Some(progress) if an update is available.
    ///
    /// For parallel video+audio exports, returns combined progress.
    pub fn poll_progress(&mut self) -> Option<ExportProgress> {
        // Handle parallel video+audio export
        if let Some(ref mut _parallel) = self.parallel_export {
            return self.poll_parallel_progress();
        }

        // Handle single export (audio-only or video-only)
        if let Some(rx) = &self.progress_rx {
            match rx.try_recv() {
                Ok(progress) => {
                    println!("📨 [ORCHESTRATOR] Received progress: {:?}", std::mem::discriminant(&progress));
                    Some(progress)
                }
                Err(_) => None,
            }
        } else {
            None
        }
    }

    /// Poll progress for parallel video+audio export
    fn poll_parallel_progress(&mut self) -> Option<ExportProgress> {
        let parallel = self.parallel_export.as_mut()?;

        // Poll video progress
        while let Ok(progress) = parallel.video_progress_rx.try_recv() {
            parallel.video_progress = Some(progress);
        }

        // Poll audio progress
        while let Ok(progress) = parallel.audio_progress_rx.try_recv() {
            parallel.audio_progress = Some(progress);
        }

        // If a background mux is already running, poll it without blocking the UI.
        if parallel.mux_rx.is_some() {
            match parallel.mux_rx.as_ref().unwrap().try_recv() {
                Ok(Ok(())) => {
                    println!("✅ [MUX] Muxing complete, cleaning up temp files");
                    let state = self.parallel_export.take().unwrap();
                    std::fs::remove_file(&state.temp_video_path).ok();
                    std::fs::remove_file(&state.temp_audio_path).ok();
                    return Some(ExportProgress::Complete { output_path: state.final_output_path });
                }
                Ok(Err(err)) => {
                    println!("❌ [MUX] Muxing failed: {}", err);
                    self.parallel_export = None;
                    return Some(ExportProgress::Error { message: format!("Muxing failed: {}", err) });
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Still muxing — keep the UI responsive and show finalizing state.
                    return Some(ExportProgress::Finalizing);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.parallel_export = None;
                    return Some(ExportProgress::Error { message: "Mux thread terminated unexpectedly".to_string() });
                }
            }
        }

        // Check for errors before completion.
        if let Some(ExportProgress::Error { ref message }) = parallel.video_progress {
            return Some(ExportProgress::Error { message: format!("Video: {}", message) });
        }
        if let Some(ExportProgress::Error { ref message }) = parallel.audio_progress {
            return Some(ExportProgress::Error { message: format!("Audio: {}", message) });
        }

        // Both streams done → spawn the mux on a background thread (the previous
        // implementation muxed synchronously here on the UI thread, which froze the
        // app for the whole re-mux pass after progress already hit 100%).
        let video_complete = matches!(parallel.video_progress, Some(ExportProgress::Complete { .. }));
        let audio_complete = matches!(parallel.audio_progress, Some(ExportProgress::Complete { .. }));
        if video_complete && audio_complete {
            println!("🎬🎵 [PARALLEL] Both video and audio complete, starting background mux");
            let video_thread = parallel.video_thread.take();
            let audio_thread = parallel.audio_thread.take();
            let video_path = parallel.temp_video_path.clone();
            let audio_path = parallel.temp_audio_path.clone();
            let output_path = parallel.final_output_path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                // The export threads have signalled Complete; join is near-instant.
                if let Some(t) = video_thread { t.join().ok(); }
                if let Some(t) = audio_thread { t.join().ok(); }
                let result = Self::mux_video_and_audio(&video_path, &audio_path, &output_path);
                tx.send(result).ok();
            });
            parallel.mux_rx = Some(rx);
            return Some(ExportProgress::Finalizing);
        }

        // Return combined progress
        match (&parallel.video_progress, &parallel.audio_progress) {
            (Some(ExportProgress::FrameRendered { frame, total }), _) => {
                Some(ExportProgress::FrameRendered { frame: *frame, total: *total })
            }
            (_, Some(ExportProgress::Started { .. })) |
            (Some(ExportProgress::Started { .. }), _) => {
                Some(ExportProgress::Started { total_frames: 0 })
            }
            _ => None,
        }
    }

    /// Mux video and audio files together using FFmpeg CLI
    ///
    /// # Arguments
    /// * `video_path` - Path to video file (no audio)
    /// * `audio_path` - Path to audio file
    /// * `output_path` - Path for final output file
    ///
    /// # Returns
    /// Ok(()) on success, Err with message on failure
    fn mux_video_and_audio(
        video_path: &PathBuf,
        audio_path: &PathBuf,
        output_path: &PathBuf,
    ) -> Result<(), String> {
        use ffmpeg_next as ffmpeg;

        println!("🎬🎵 [MUX] Muxing video and audio using ffmpeg-next");
        println!("  Video: {:?}", video_path);
        println!("  Audio: {:?}", audio_path);
        println!("  Output: {:?}", output_path);

        // Initialize FFmpeg
        ffmpeg::init().map_err(|e| format!("FFmpeg init failed: {}", e))?;

        // Open input video
        let mut video_input = ffmpeg::format::input(&video_path)
            .map_err(|e| format!("Failed to open video file: {}", e))?;

        // Open input audio
        let mut audio_input = ffmpeg::format::input(&audio_path)
            .map_err(|e| format!("Failed to open audio file: {}", e))?;

        // Create output
        let mut output = ffmpeg::format::output(&output_path)
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        // Find video stream
        let video_stream_index = video_input.streams().best(ffmpeg::media::Type::Video)
            .ok_or("No video stream found")?.index();

        // Find audio stream
        let audio_stream_index = audio_input.streams().best(ffmpeg::media::Type::Audio)
            .ok_or("No audio stream found")?.index();

        // Extract video stream info (do this before adding output streams)
        let (video_input_tb, video_output_tb) = {
            let video_stream = video_input.stream(video_stream_index)
                .ok_or("Failed to get video stream")?;
            let input_tb = video_stream.time_base();
            let codec_id = video_stream.parameters().id();
            let params = video_stream.parameters();

            // Add video stream to output and extract time_base before dropping
            let mut video_out_stream = output.add_stream(ffmpeg::encoder::find(codec_id))
                .map_err(|e| format!("Failed to add video stream: {}", e))?;
            video_out_stream.set_parameters(params);
            // Set time base explicitly (params might not include it, resulting in 0/0)
            video_out_stream.set_time_base(input_tb);
            let output_tb = video_out_stream.time_base();

            (input_tb, output_tb)
        }; // video_out_stream drops here

        // Extract audio stream info (after video stream is dropped)
        let (audio_input_tb, audio_output_tb) = {
            let audio_stream = audio_input.stream(audio_stream_index)
                .ok_or("Failed to get audio stream")?;
            let input_tb = audio_stream.time_base();
            let codec_id = audio_stream.parameters().id();
            let params = audio_stream.parameters();

            // Add audio stream to output and extract time_base before dropping
            let mut audio_out_stream = output.add_stream(ffmpeg::encoder::find(codec_id))
                .map_err(|e| format!("Failed to add audio stream: {}", e))?;
            audio_out_stream.set_parameters(params);
            // Set time base explicitly (params might not include it, resulting in 0/0)
            audio_out_stream.set_time_base(input_tb);
            let output_tb = audio_out_stream.time_base();

            (input_tb, output_tb)
        }; // audio_out_stream drops here

        // Write header
        output.write_header().map_err(|e| format!("Failed to write header: {}", e))?;

        println!("🎬 [MUX] Video stream - Input TB: {}/{}, Output TB: {}/{}",
                 video_input_tb.0, video_input_tb.1, video_output_tb.0, video_output_tb.1);
        println!("🎵 [MUX] Audio stream - Input TB: {}/{}, Output TB: {}/{}",
                 audio_input_tb.0, audio_input_tb.1, audio_output_tb.0, audio_output_tb.1);

        // Stream-merge the two inputs by PTS, writing each packet as it's read —
        // O(1) memory (one pending packet per stream) instead of collecting every
        // packet first, so muxing a long export never grows unbounded.
        let video_idx = video_stream_index;
        let audio_idx = audio_stream_index;
        let mut v_iter = video_input.packets();
        let mut a_iter = audio_input.packets();

        // Pull the next packet belonging to the desired stream from each input.
        let mut next_video = move || -> Option<ffmpeg::Packet> {
            loop {
                match v_iter.next() {
                    Some((stream, packet)) => {
                        if stream.index() == video_idx {
                            return Some(packet);
                        }
                    }
                    None => return None,
                }
            }
        };
        let mut next_audio = move || -> Option<ffmpeg::Packet> {
            loop {
                match a_iter.next() {
                    Some((stream, packet)) => {
                        if stream.index() == audio_idx {
                            return Some(packet);
                        }
                    }
                    None => return None,
                }
            }
        };

        let mut pending_v = next_video();
        let mut pending_a = next_audio();
        let mut v_count = 0usize;
        let mut a_count = 0usize;
        let mut log_count = 0;

        loop {
            // Write whichever pending packet has the earlier PTS (in a common
            // microsecond base); when one stream is exhausted, drain the other.
            let write_video = match (&pending_v, &pending_a) {
                (None, None) => break,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (Some(v), Some(a)) => {
                    let v_us = v.pts().unwrap_or(0) * 1_000_000 * video_input_tb.0 as i64
                        / video_input_tb.1 as i64;
                    let a_us = a.pts().unwrap_or(0) * 1_000_000 * audio_input_tb.0 as i64
                        / audio_input_tb.1 as i64;
                    v_us <= a_us
                }
            };

            if write_video {
                let mut packet = pending_v.take().unwrap();
                packet.set_stream(0);
                packet.rescale_ts(video_input_tb, video_output_tb);
                if log_count < 10 {
                    println!("🎬 [MUX] Writing V packet - PTS={:?}, DTS={:?}", packet.pts(), packet.dts());
                    log_count += 1;
                }
                packet.write_interleaved(&mut output)
                    .map_err(|e| format!("Failed to write video packet: {}", e))?;
                v_count += 1;
                pending_v = next_video();
            } else {
                let mut packet = pending_a.take().unwrap();
                packet.set_stream(1);
                packet.rescale_ts(audio_input_tb, audio_output_tb);
                if log_count < 10 {
                    println!("🎵 [MUX] Writing A packet - PTS={:?}, DTS={:?}", packet.pts(), packet.dts());
                    log_count += 1;
                }
                packet.write_interleaved(&mut output)
                    .map_err(|e| format!("Failed to write audio packet: {}", e))?;
                a_count += 1;
                pending_a = next_audio();
            }
        }

        println!("🎬 [MUX] Wrote {} video packets, {} audio packets", v_count, a_count);

        // Write trailer
        output.write_trailer().map_err(|e| format!("Failed to write trailer: {}", e))?;

        println!("✅ [MUX] Muxing completed successfully");
        Ok(())
    }

    /// Cancel the current export
    pub fn cancel(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// Check if an export is in progress
    pub fn is_exporting(&self) -> bool {
        if self.parallel_export.is_some() { return true; }
        if self.image_state.is_some()     { return true; }
        if let Some(handle) = &self.thread_handle {
            !handle.is_finished()
        } else {
            false
        }
    }

    /// Enqueue a single-frame image export.  Call `render_image_frame()` from the
    /// egui update loop (where the wgpu device/queue are available) to complete it.
    pub fn start_image_export(
        &mut self,
        settings: ImageExportSettings,
        output_path: PathBuf,
        doc_width: u32,
        doc_height: u32,
    ) {
        self.cancel_flag.store(false, Ordering::Relaxed);
        let width  = settings.width.unwrap_or(doc_width).max(1);
        let height = settings.height.unwrap_or(doc_height).max(1);
        self.image_state = Some(ImageExportState {
            settings,
            output_path,
            width,
            height,
            rendered: false,
            gpu_resources: None,
            output_texture: None,
            output_texture_view: None,
            staging_buffer: None,
        });
    }

    /// Drive the single-frame image export.  Returns `Ok(true)` when done (success or
    /// cancelled), `Ok(false)` if another call is needed next frame.
    pub fn render_image_frame(
        &mut self,
        document: &mut Document,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        image_cache: &mut ImageCache,
        video_manager: &Arc<std::sync::Mutex<VideoManager>>,
        floating_selection: Option<&lightningbeam_core::selection::RasterFloatingSelection>,
    ) -> Result<bool, String> {
        if self.cancel_flag.load(Ordering::Relaxed) {
            self.image_state = None;
            return Ok(true);
        }

        let state = match self.image_state.as_mut() {
            Some(s) => s,
            None    => return Ok(true),
        };

        if !state.rendered {
            // ── First call: render the frame to the GPU output texture ────────
            let w = state.width;
            let h = state.height;

            if state.gpu_resources.is_none() {
                state.gpu_resources = Some(video_exporter::ExportGpuResources::new(device, w, h));
            }
            if state.output_texture.is_none() {
                let tex = device.create_texture(&wgpu::TextureDescriptor {
                    label:              Some("image_export_output"),
                    size:               wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count:    1,
                    sample_count:       1,
                    dimension:          wgpu::TextureDimension::D2,
                    format:             wgpu::TextureFormat::Rgba8Unorm,
                    usage:              wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats:       &[],
                });
                state.output_texture_view = Some(tex.create_view(&wgpu::TextureViewDescriptor::default()));
                state.output_texture = Some(tex);
            }

            // Borrow separately to avoid a split-borrow conflict (gpu mutably, view immutably).
            let gpu = state.gpu_resources.as_mut().unwrap();
            let output_view = state.output_texture_view.as_ref().unwrap();

            let mut encoder = video_exporter::render_frame_to_gpu_rgba(
                document,
                state.settings.time,
                w, h,
                device, queue, renderer, image_cache, video_manager,
                gpu,
                output_view,
                floating_selection,
                state.settings.allow_transparency,
            )?;
            queue.submit(Some(encoder.finish()));

            // Create a staging buffer for synchronous readback.
            // wgpu requires bytes_per_row to be a multiple of 256.
            let align        = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
            let bytes_per_row = (w * 4 + align - 1) / align * align;
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("image_export_staging"),
                size:               (bytes_per_row * h) as u64,
                usage:              wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            let mut copy_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("image_export_copy"),
            });
            let output_tex = state.output_texture.as_ref().unwrap();
            copy_enc.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture:   output_tex,
                    mip_level: 0,
                    origin:    wgpu::Origin3d::ZERO,
                    aspect:    wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset:         0,
                        bytes_per_row:  Some(bytes_per_row),
                        rows_per_image: Some(h),
                    },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            queue.submit(Some(copy_enc.finish()));

            state.staging_buffer = Some(staging);
            state.rendered       = true;
            return Ok(false); // Come back next frame to read the result.
        }

        // ── Second call: map the staging buffer, encode, and save ─────────────
        let staging = match state.staging_buffer.as_ref() {
            Some(b) => b,
            None    => { self.image_state = None; return Ok(true); }
        };

        // Map synchronously.
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        let w = state.width;
        let h = state.height;
        let align        = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let bytes_per_row = (w * 4 + align - 1) / align * align;

        let pixels: Vec<u8> = {
            let mapped = slice.get_mapped_range();
            // Strip row padding: copy only w*4 bytes from each bytes_per_row-wide row.
            let mut out = Vec::with_capacity((w * h * 4) as usize);
            for row in 0..h {
                let start = (row * bytes_per_row) as usize;
                out.extend_from_slice(&mapped[start..start + (w * 4) as usize]);
            }
            out
        };
        staging.unmap();

        let result = image_exporter::save_rgba_image(
            &pixels, w, h,
            state.settings.format,
            state.settings.quality,
            state.settings.allow_transparency,
            &state.output_path,
        );

        self.image_state = None;
        result.map(|_| true)
    }

    /// Wait for the export to complete
    ///
    /// This blocks until the export thread finishes.
    #[allow(dead_code)]
    pub fn wait_for_completion(&mut self) {
        if let Some(handle) = self.thread_handle.take() {
            handle.join().ok();
        }
    }

    /// Run audio export in background thread
    fn run_audio_export(
        settings: AudioExportSettings,
        output_path: PathBuf,
        audio_controller: Arc<std::sync::Mutex<daw_backend::EngineController>>,
        progress_tx: Sender<ExportProgress>,
        cancel_flag: Arc<AtomicBool>,
    ) {
        println!("🧵 [EXPORT THREAD] run_audio_export started");

        // Send start notification with calculated total frames
        let duration = settings.end_time - settings.start_time;
        let total_frames = (duration * settings.sample_rate as f64).round() as usize;
        progress_tx
            .send(ExportProgress::Started { total_frames })
            .ok();
        println!("🧵 [EXPORT THREAD] Sent Started progress");

        // Check for cancellation
        if cancel_flag.load(Ordering::Relaxed) {
            progress_tx
                .send(ExportProgress::Error {
                    message: "Export cancelled by user".to_string(),
                })
                .ok();
            return;
        }

        println!("🧵 [EXPORT THREAD] Starting export for format: {:?}", settings.format);

        // Convert settings to DAW backend format
        let daw_settings = daw_backend::audio::ExportSettings {
            format: match settings.format {
                lightningbeam_core::export::AudioFormat::Wav => daw_backend::audio::ExportFormat::Wav,
                lightningbeam_core::export::AudioFormat::Flac => daw_backend::audio::ExportFormat::Flac,
                lightningbeam_core::export::AudioFormat::Mp3 => daw_backend::audio::ExportFormat::Mp3,
                lightningbeam_core::export::AudioFormat::Aac => daw_backend::audio::ExportFormat::Aac,
            },
            sample_rate: settings.sample_rate,
            channels: settings.channels,
            bit_depth: settings.bit_depth,
            mp3_bitrate: settings.bitrate_kbps,
            start_time: daw_backend::Seconds(settings.start_time),
            end_time: daw_backend::Seconds(settings.end_time),
            tempo_map: daw_backend::TempoMap::constant(settings.bpm),
        };

        // Use DAW backend export for all formats
        let result = Self::run_daw_backend_export(
            &daw_settings,
            &output_path,
            &audio_controller,
            &cancel_flag,
        );

        println!("🧵 [EXPORT THREAD] Export finished");

        // Send completion or error
        match result {
            Ok(_) => {
                println!("📤 [EXPORT THREAD] Sending Complete event");
                let send_result = progress_tx.send(ExportProgress::Complete {
                    output_path: output_path.clone(),
                });
                println!("📤 [EXPORT THREAD] Complete event sent: {:?}", send_result.is_ok());
            }
            Err(err) => {
                println!("📤 [EXPORT THREAD] Sending Error event: {}", err);
                let send_result = progress_tx.send(ExportProgress::Error { message: err });
                println!("📤 [EXPORT THREAD] Error event sent: {:?}", send_result.is_ok());
            }
        }
    }

    /// Run export using DAW backend (for all formats)
    fn run_daw_backend_export(
        settings: &daw_backend::audio::ExportSettings,
        output_path: &PathBuf,
        audio_controller: &Arc<std::sync::Mutex<daw_backend::EngineController>>,
        cancel_flag: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        println!("🧵 [EXPORT THREAD] Starting DAW backend export...");

        // Start the export (non-blocking - just sends the query)
        {
            let mut controller = audio_controller.lock().unwrap();
            println!("🧵 [EXPORT THREAD] Sending export query...");
            controller.start_export_audio(settings, output_path)?;
            println!("🧵 [EXPORT THREAD] Export query sent, lock released");
        }

        // Poll for completion without holding the lock for extended periods
        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err("Export cancelled by user".to_string());
            }

            // Sleep before polling to avoid spinning
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Brief lock to poll for completion
            let poll_result = {
                let mut controller = audio_controller.lock().unwrap();
                controller.poll_export_completion()
            };

            match poll_result {
                Ok(Some(result)) => {
                    println!("🧵 [EXPORT THREAD] DAW backend export completed: {:?}", result.is_ok());
                    return result;
                }
                Ok(None) => {
                    // Still in progress
                }
                Err(e) => {
                    println!("🧵 [EXPORT THREAD] Poll error: {}", e);
                    return Err(e);
                }
            }
        }
    }

    /// Start a video export in the background (encoder thread)
    ///
    /// Returns immediately after spawning encoder thread. Caller must call
    /// `render_next_video_frame()` repeatedly from the main thread to feed frames.
    ///
    /// # Arguments
    /// * `settings` - Video export settings
    /// * `output_path` - Output file path
    ///
    /// # Returns
    /// Ok(()) on success, Err on failure
    pub fn start_video_export(
        &mut self,
        settings: VideoExportSettings,
        output_path: PathBuf,
    ) -> Result<(), String> {
        println!("🎬 [VIDEO EXPORT] Starting video export");

        // Extract values we need before moving settings to thread
        let start_time = settings.start_time;
        let end_time = settings.end_time;
        let framerate = settings.framerate;
        let width = settings.width.unwrap_or(1920);
        let height = settings.height.unwrap_or(1080);
        let duration = end_time - start_time;
        let total_frames = (duration * framerate).ceil() as usize;

        // Create channels
        let (progress_tx, progress_rx) = channel();
        let (frame_tx, frame_rx) = channel();

        self.progress_rx = Some(progress_rx);

        // Reset cancel flag
        self.cancel_flag.store(false, Ordering::Relaxed);
        let cancel_flag = Arc::clone(&self.cancel_flag);

        // Spawn encoder thread
        let handle = std::thread::spawn(move || {
            Self::run_video_encoder(
                settings,
                output_path,
                frame_rx,
                progress_tx,
                cancel_flag,
                total_frames,
            );
        });

        self.thread_handle = Some(handle);

        // Initialize video export state
        // GPU resources and readback pipeline will be initialized lazily on first frame (needs device)
        self.video_state = Some(VideoExportState {
            current_frame: 0,
            total_frames,
            start_time,
            end_time,
            framerate,
            width,
            height,
            frame_tx: Some(frame_tx),
            gpu_resources: None,
            readback_pipeline: None,
            cpu_yuv_converter: None,
            frames_in_flight: 0,
            next_frame_to_encode: 0,
            perf_metrics: Some(perf_metrics::ExportMetrics::new()),
        });

        println!("🎬 [VIDEO EXPORT] Encoder thread spawned, ready for frames");
        Ok(())
    }

    /// Start a video+audio export in parallel
    ///
    /// Exports video and audio simultaneously to temporary files, then muxes them together.
    /// Returns immediately after spawning both threads. Caller must call
    /// `render_next_video_frame()` repeatedly for video rendering.
    ///
    /// # Arguments
    /// * `video_settings` - Video export settings
    /// * `audio_settings` - Audio export settings
    /// * `output_path` - Final output file path
    /// * `audio_controller` - DAW audio controller for audio export
    ///
    /// # Returns
    /// Ok(()) on success, Err on failure
    pub fn start_video_with_audio_export(
        &mut self,
        video_settings: VideoExportSettings,
        mut audio_settings: AudioExportSettings,
        output_path: PathBuf,
        audio_controller: Arc<std::sync::Mutex<daw_backend::EngineController>>,
    ) -> Result<(), String> {
        println!("🎬🎵 [PARALLEL EXPORT] Starting parallel video+audio export");

        // Force AAC if format is incompatible with MP4 (WAV/FLAC/MP3)
        // AAC is the standard audio codec for MP4 containers
        // Allow user-selected AAC to pass through
        match audio_settings.format {
            lightningbeam_core::export::AudioFormat::Wav |
            lightningbeam_core::export::AudioFormat::Flac |
            lightningbeam_core::export::AudioFormat::Mp3 => {
                audio_settings.format = lightningbeam_core::export::AudioFormat::Aac;
                println!("🎵 [PARALLEL EXPORT] Audio format forced to AAC for MP4 compatibility");
            }
            lightningbeam_core::export::AudioFormat::Aac => {
                println!("🎵 [PARALLEL EXPORT] Using user-selected audio format: AAC");
            }
        }

        // Generate temporary file paths
        let temp_dir = std::env::temp_dir();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let temp_video_path = temp_dir.join(format!("lightningbeam_video_{}.mp4", timestamp));
        let temp_audio_path = temp_dir.join(format!("lightningbeam_audio_{}.{}",
            timestamp,
            match audio_settings.format {
                lightningbeam_core::export::AudioFormat::Wav => "wav",
                lightningbeam_core::export::AudioFormat::Flac => "flac",
                lightningbeam_core::export::AudioFormat::Mp3 => "mp3",
                lightningbeam_core::export::AudioFormat::Aac => "m4a",
            }
        ));

        println!("🎬 [PARALLEL EXPORT] Temp video: {:?}", temp_video_path);
        println!("🎵 [PARALLEL EXPORT] Temp audio: {:?}", temp_audio_path);

        // Extract values we need before moving settings
        let video_start_time = video_settings.start_time;
        let video_end_time = video_settings.end_time;
        let video_framerate = video_settings.framerate;
        let video_width = video_settings.width.unwrap_or(1920);
        let video_height = video_settings.height.unwrap_or(1080);
        let video_duration = video_end_time - video_start_time;
        let total_frames = (video_duration * video_framerate).ceil() as usize;

        // Create channels for video export
        let (video_progress_tx, video_progress_rx) = channel();
        let (frame_tx, frame_rx) = channel();

        // Create channel for audio export
        let (audio_progress_tx, audio_progress_rx) = channel();

        // Reset cancel flag
        self.cancel_flag.store(false, Ordering::Relaxed);
        let video_cancel_flag = Arc::clone(&self.cancel_flag);
        let audio_cancel_flag = Arc::clone(&self.cancel_flag);

        // Spawn video encoder thread
        let video_settings_clone = video_settings.clone();
        let temp_video_path_clone = temp_video_path.clone();
        let video_thread = std::thread::spawn(move || {
            Self::run_video_encoder(
                video_settings_clone,
                temp_video_path_clone,
                frame_rx,
                video_progress_tx,
                video_cancel_flag,
                total_frames,
            );
        });

        // Spawn audio export thread
        let temp_audio_path_clone = temp_audio_path.clone();
        let audio_thread = std::thread::spawn(move || {
            Self::run_audio_export(
                audio_settings,
                temp_audio_path_clone,
                audio_controller,
                audio_progress_tx,
                audio_cancel_flag,
            );
        });

        // Initialize video export state for incremental rendering
        // GPU resources and readback pipeline will be initialized lazily on first frame (needs device)
        self.video_state = Some(VideoExportState {
            current_frame: 0,
            total_frames,
            start_time: video_start_time,
            end_time: video_end_time,
            framerate: video_framerate,
            width: video_width,
            height: video_height,
            frame_tx: Some(frame_tx),
            gpu_resources: None,
            readback_pipeline: None,
            cpu_yuv_converter: None,
            frames_in_flight: 0,
            next_frame_to_encode: 0,
            perf_metrics: Some(perf_metrics::ExportMetrics::new()),
        });

        // Initialize parallel export state
        self.parallel_export = Some(ParallelExportState {
            video_progress_rx,
            audio_progress_rx,
            video_thread: Some(video_thread),
            audio_thread: Some(audio_thread),
            temp_video_path,
            temp_audio_path,
            final_output_path: output_path,
            video_progress: None,
            audio_progress: None,
            mux_rx: None,
        });

        println!("🎬🎵 [PARALLEL EXPORT] Both threads spawned, ready for frames");
        Ok(())
    }

    /// Render and send the next video frame (call from main thread)
    ///
    /// Uses async triple-buffered pipeline for maximum throughput.
    /// Returns true if there are more frames to render, false if done.
    ///
    /// # Arguments
    /// * `document` - Document to render
    /// * `device` - wgpu device
    /// * `queue` - wgpu queue
    /// * `renderer` - Vello renderer
    /// * `image_cache` - Image cache
    /// * `video_manager` - Video manager
    ///
    /// # Returns
    /// Ok(true) if more frames remain, Ok(false) if done, Err on failure
    pub fn render_next_video_frame(
        &mut self,
        document: &mut Document,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut vello::Renderer,
        image_cache: &mut ImageCache,
        video_manager: &Arc<std::sync::Mutex<VideoManager>>,
    ) -> Result<bool, String> {
        use std::time::Instant;

        let state = self.video_state.as_mut()
            .ok_or("No video export in progress")?;

        let width = state.width;
        let height = state.height;

        // Initialize GPU resources and readback pipeline on first frame
        if state.gpu_resources.is_none() {
            println!("🎬 [VIDEO EXPORT] Initializing HDR GPU + async pipeline {}x{}", width, height);
            state.gpu_resources = Some(video_exporter::ExportGpuResources::new(device, width, height));
            state.readback_pipeline = Some(readback_pipeline::ReadbackPipeline::new(device, queue, width, height));
            state.cpu_yuv_converter = Some(cpu_yuv_converter::CpuYuvConverter::new(width, height)?);
            println!("🚀 [ASYNC PIPELINE] Triple-buffered pipeline initialized");
            println!("🚀 [CPU YUV] swscale converter initialized");
        }

        let pipeline = state.readback_pipeline.as_mut().unwrap();
        let gpu_resources = state.gpu_resources.as_mut().unwrap();
        let cpu_converter = state.cpu_yuv_converter.as_mut().unwrap();
        let mut metrics = state.perf_metrics.as_mut();

        // Poll for completed async readbacks (non-blocking)
        if let Some(m) = metrics.as_mut() {
            m.poll_count += 1;
        }
        let completed_frames = pipeline.poll_nonblocking();
        if let Some(m) = metrics.as_mut() {
            m.completions_per_poll.push(completed_frames.len());
        }

        // Process completed frames IN ORDER
        for result in completed_frames {
            if result.frame_num == state.next_frame_to_encode {
                // Record readback completion time
                if let Some(m) = metrics.as_mut() {
                    if let Some(frame_metrics) = m.frames.get_mut(result.frame_num) {
                        frame_metrics.readback_complete = Some(Instant::now());
                    }
                }

                // Extract RGBA data (timed)
                let extraction_start = Instant::now();
                let rgba_data = pipeline.extract_rgba_data(result.buffer_id);
                let extraction_end = Instant::now();

                // CPU YUV conversion (timed)
                let conversion_start = Instant::now();
                let (y, u, v) = cpu_converter.convert(&rgba_data)?;
                let conversion_end = Instant::now();

                if let Some(m) = metrics.as_mut() {
                    if let Some(frame_metrics) = m.frames.get_mut(result.frame_num) {
                        frame_metrics.extraction_start = Some(extraction_start);
                        frame_metrics.extraction_end = Some(extraction_end);
                        frame_metrics.conversion_start = Some(conversion_start);
                        frame_metrics.conversion_end = Some(conversion_end);
                    }
                }

                // Send to encoder
                if let Some(tx) = &state.frame_tx {
                    tx.send(VideoFrameMessage::Frame {
                        frame_num: result.frame_num,
                        timestamp: result.timestamp,
                        y_plane: y,
                        u_plane: u,
                        v_plane: v,
                    }).map_err(|_| "Failed to send frame")?;
                }

                pipeline.release(result.buffer_id);
                state.frames_in_flight -= 1;
                state.next_frame_to_encode += 1;
            }
        }

        // Submit new frames (up to 3 in flight)
        while state.current_frame < state.total_frames && state.frames_in_flight < 3 {
            let timestamp = state.start_time + (state.current_frame as f64 / state.framerate);

            if let Some(acquired) = pipeline.acquire(state.current_frame, timestamp) {
                // Create frame metrics entry
                if let Some(m) = metrics.as_mut() {
                    m.frames.push(perf_metrics::FrameMetrics::new(state.current_frame));
                }

                // Render to GPU (timed)
                let _render_start = Instant::now();
                let encoder = video_exporter::render_frame_to_gpu_rgba(
                    document, timestamp, width, height,
                    device, queue, renderer, image_cache, video_manager,
                    gpu_resources, &acquired.rgba_texture_view,
                    None,  // No floating selection during video export
                    false, // Video export is never transparent
                )?;
                let render_end = Instant::now();

                // Record render timing
                if let Some(m) = metrics.as_mut() {
                    if let Some(frame_metrics) = m.frames.get_mut(state.current_frame) {
                        frame_metrics.render_end = Some(render_end);
                        frame_metrics.submit_time = Some(Instant::now());
                    }
                }

                // Submit for async readback
                pipeline.submit_and_readback(acquired.id, encoder);

                state.current_frame += 1;
                state.frames_in_flight += 1;
            } else {
                break; // All buffers in use
            }
        }

        // Done when all submitted AND all completed
        if state.current_frame >= state.total_frames && state.frames_in_flight == 0 {
            println!("🎬 [VIDEO EXPORT] Complete: {} frames", state.total_frames);

            // Print performance summary
            if let Some(m) = &state.perf_metrics {
                m.print_summary();
                m.print_per_frame_details(10);
            }

            if let Some(tx) = state.frame_tx.take() {
                tx.send(VideoFrameMessage::Done).ok();
            }

            state.gpu_resources = None;
            state.readback_pipeline = None;
            state.cpu_yuv_converter = None;
            state.perf_metrics = None;
            return Ok(false);
        }

        Ok(true) // More work to do
    }

    /// Background thread that receives frames and encodes them
    fn run_video_encoder(
        settings: VideoExportSettings,
        output_path: PathBuf,
        frame_rx: Receiver<VideoFrameMessage>,
        progress_tx: Sender<ExportProgress>,
        cancel_flag: Arc<AtomicBool>,
        total_frames: usize,
    ) {
        println!("🧵 [ENCODER THREAD] Video encoder thread started");

        // Send started progress
        progress_tx.send(ExportProgress::Started {
            total_frames,
        }).ok();

        // Delegate to inner function for better error handling
        match Self::run_video_encoder_inner(
            &settings,
            &output_path,
            frame_rx,
            &progress_tx,
            &cancel_flag,
            total_frames,
        ) {
            Ok(()) => {
                println!("🧵 [ENCODER] Export completed successfully");
                progress_tx.send(ExportProgress::Complete {
                    output_path: output_path.clone(),
                }).ok();
            }
            Err(err) => {
                println!("🧵 [ENCODER] Export failed: {}", err);
                progress_tx.send(ExportProgress::Error {
                    message: err,
                }).ok();
            }
        }
    }

    /// Inner encoder function with proper error handling
    fn run_video_encoder_inner(
        settings: &VideoExportSettings,
        output_path: &PathBuf,
        frame_rx: Receiver<VideoFrameMessage>,
        progress_tx: &Sender<ExportProgress>,
        cancel_flag: &Arc<AtomicBool>,
        total_frames: usize,
    ) -> Result<(), String> {
        use lightningbeam_core::export::VideoCodec;

        // Initialize FFmpeg
        ffmpeg_next::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

        // Convert codec enum to FFmpeg codec ID
        let codec_id = match settings.codec {
            VideoCodec::H264 => ffmpeg_next::codec::Id::H264,
            VideoCodec::H265 => ffmpeg_next::codec::Id::HEVC,
            VideoCodec::VP8 => ffmpeg_next::codec::Id::VP8,
            VideoCodec::VP9 => ffmpeg_next::codec::Id::VP9,
            VideoCodec::ProRes422 => ffmpeg_next::codec::Id::PRORES,
        };

        // Get bitrate from quality settings
        let bitrate_kbps = settings.quality.bitrate_kbps();
        let framerate = settings.framerate;

        // Wait for first frame to determine dimensions
        let first_frame = match frame_rx.recv() {
            Ok(VideoFrameMessage::Frame { frame_num, timestamp, y_plane, u_plane, v_plane }) => {
                println!("🧵 [ENCODER] Received first YUV frame (Y: {} bytes)", y_plane.len());
                Some((frame_num, timestamp, y_plane, u_plane, v_plane))
            }
            Ok(VideoFrameMessage::Done) => {
                return Err("No frames to encode".to_string());
            }
            Err(_) => {
                return Err("Frame channel disconnected before first frame".to_string());
            }
        };

        // Determine dimensions from first frame
        let (width, height) = if let Some((_, _, ref y_plane, _, _)) = first_frame {
            // Calculate dimensions from Y plane size (full resolution, 1 byte per pixel)
            let _pixel_count = y_plane.len();
            // Use settings dimensions if provided, otherwise infer from buffer
            let w = settings.width.unwrap_or(1920); // Default to 1920 if not specified
            let h = settings.height.unwrap_or(1080); // Default to 1080 if not specified
            (w, h)
        } else {
            return Err("Failed to determine dimensions".to_string());
        };

        println!("🧵 [ENCODER] Setting up encoder: {}×{} @ {} fps, {} kbps",
            width, height, framerate, bitrate_kbps);

        // Setup encoder
        let (mut encoder, encoder_codec) = video_exporter::setup_video_encoder(
            codec_id,
            width,
            height,
            framerate,
            bitrate_kbps,
        )?;

        // Create output file
        let mut output = ffmpeg_next::format::output(&output_path)
            .map_err(|e| format!("Failed to create output file: {}", e))?;

        // Add stream AFTER opening encoder (critical order!)
        {
            let mut stream = output.add_stream(encoder_codec)
                .map_err(|e| format!("Failed to add stream: {}", e))?;
            stream.set_parameters(&encoder);
        }

        // Write header
        output.write_header()
            .map_err(|e| format!("Failed to write header: {}", e))?;

        println!("🧵 [ENCODER] Encoder initialized, ready to encode frames");

        // Process first frame
        if let Some((_frame_num, timestamp, y_plane, u_plane, v_plane)) = first_frame {
            Self::encode_frame(
                &mut encoder,
                &mut output,
                &y_plane,
                &u_plane,
                &v_plane,
                width,
                height,
                timestamp,
            )?;

            // Send progress update for first frame
            progress_tx.send(ExportProgress::FrameRendered {
                frame: 1,
                total: total_frames,
            }).ok();
        }

        // Process remaining frames
        let mut frames_encoded = 1;
        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err("Export cancelled by user".to_string());
            }

            match frame_rx.recv() {
                Ok(VideoFrameMessage::Frame { frame_num: _, timestamp, y_plane, u_plane, v_plane }) => {
                    Self::encode_frame(
                        &mut encoder,
                        &mut output,
                        &y_plane,
                        &u_plane,
                        &v_plane,
                        width,
                        height,
                        timestamp,
                    )?;

                    frames_encoded += 1;

                    // Send progress update
                    progress_tx.send(ExportProgress::FrameRendered {
                        frame: frames_encoded,
                        total: total_frames,
                    }).ok();
                }
                Ok(VideoFrameMessage::Done) => {
                    println!("🧵 [ENCODER] All frames received, flushing encoder");
                    break;
                }
                Err(_) => {
                    return Err("Frame channel disconnected".to_string());
                }
            }
        }

        // Flush encoder
        encoder.send_eof()
            .map_err(|e| format!("Failed to send EOF to encoder: {}", e))?;

        video_exporter::receive_and_write_packets(&mut encoder, &mut output)?;

        // Write trailer
        output.write_trailer()
            .map_err(|e| format!("Failed to write trailer: {}", e))?;

        println!("🧵 [ENCODER] Video export completed: {} frames", frames_encoded);
        Ok(())
    }

    /// Encode a single YUV420p frame (already converted by GPU)
    fn encode_frame(
        encoder: &mut ffmpeg_next::encoder::Video,
        output: &mut ffmpeg_next::format::context::Output,
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        width: u32,
        height: u32,
        timestamp: f64,
    ) -> Result<(), String> {
        // YUV planes already converted by GPU (no CPU conversion needed)

        // Create FFmpeg video frame
        let mut video_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
        );

        // Copy YUV planes to frame
        // Use safe slice copy - LLVM optimizes this to memcpy, same performance as copy_nonoverlapping
        let y_dest = video_frame.data_mut(0);
        let y_len = y_plane.len().min(y_dest.len());
        y_dest[..y_len].copy_from_slice(&y_plane[..y_len]);

        let u_dest = video_frame.data_mut(1);
        let u_len = u_plane.len().min(u_dest.len());
        u_dest[..u_len].copy_from_slice(&u_plane[..u_len]);

        let v_dest = video_frame.data_mut(2);
        let v_len = v_plane.len().min(v_dest.len());
        v_dest[..v_len].copy_from_slice(&v_plane[..v_len]);

        // Set PTS (presentation timestamp) in encoder's time base
        // Encoder time base is 1/(framerate * 1000), so PTS = timestamp * (framerate * 1000)
        let encoder_tb = encoder.time_base();
        let pts = (timestamp * encoder_tb.1 as f64) as i64;
        video_frame.set_pts(Some(pts));

        // Send frame to encoder
        encoder.send_frame(&video_frame)
            .map_err(|e| format!("Failed to send frame to encoder: {}", e))?;

        // Receive and write packets
        video_exporter::receive_and_write_packets(encoder, output)?;

        Ok(())
    }
}

impl Default for ExportOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
