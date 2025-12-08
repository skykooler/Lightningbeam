//! Export functionality for audio and video
//!
//! This module provides the export orchestrator and progress tracking
//! for exporting audio and video from the timeline.

pub mod audio_exporter;
pub mod dialog;
pub mod video_exporter;

use lightningbeam_core::export::{AudioExportSettings, VideoExportSettings, ExportProgress};
use lightningbeam_core::document::Document;
use lightningbeam_core::renderer::ImageCache;
use lightningbeam_core::video::VideoManager;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Message sent from main thread to video encoder thread
enum VideoFrameMessage {
    /// RGBA frame data with frame number and timestamp
    Frame { frame_num: usize, timestamp: f64, rgba_data: Vec<u8> },
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
}

/// State for parallel audio+video export
struct ParallelExportState {
    /// Video progress channel
    video_progress_rx: Receiver<ExportProgress>,
    /// Audio progress channel
    audio_progress_rx: Receiver<ExportProgress>,
    /// Video encoder thread handle
    video_thread: std::thread::JoinHandle<()>,
    /// Audio export thread handle
    audio_thread: std::thread::JoinHandle<()>,
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
        if let Some(ref mut parallel) = self.parallel_export {
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
            println!("📨 [PARALLEL] Video progress: {:?}", std::mem::discriminant(&progress));
            parallel.video_progress = Some(progress);
        }

        // Poll audio progress
        while let Ok(progress) = parallel.audio_progress_rx.try_recv() {
            println!("📨 [PARALLEL] Audio progress: {:?}", std::mem::discriminant(&progress));
            parallel.audio_progress = Some(progress);
        }

        // Check if both are complete
        let video_complete = matches!(parallel.video_progress, Some(ExportProgress::Complete { .. }));
        let audio_complete = matches!(parallel.audio_progress, Some(ExportProgress::Complete { .. }));

        if video_complete && audio_complete {
            println!("🎬🎵 [PARALLEL] Both video and audio complete, starting mux");

            // Take parallel state to extract file paths
            let parallel_state = self.parallel_export.take().unwrap();

            // Wait for threads to finish
            parallel_state.video_thread.join().ok();
            parallel_state.audio_thread.join().ok();

            // Start muxing
            match Self::mux_video_and_audio(
                &parallel_state.temp_video_path,
                &parallel_state.temp_audio_path,
                &parallel_state.final_output_path,
            ) {
                Ok(()) => {
                    println!("✅ [MUX] Muxing complete, cleaning up temp files");

                    // Clean up temp files
                    std::fs::remove_file(&parallel_state.temp_video_path).ok();
                    std::fs::remove_file(&parallel_state.temp_audio_path).ok();

                    return Some(ExportProgress::Complete {
                        output_path: parallel_state.final_output_path,
                    });
                }
                Err(err) => {
                    println!("❌ [MUX] Muxing failed: {}", err);
                    return Some(ExportProgress::Error {
                        message: format!("Muxing failed: {}", err),
                    });
                }
            }
        }

        // Check for errors
        if let Some(ExportProgress::Error { ref message }) = parallel.video_progress {
            return Some(ExportProgress::Error { message: format!("Video: {}", message) });
        }
        if let Some(ExportProgress::Error { ref message }) = parallel.audio_progress {
            return Some(ExportProgress::Error { message: format!("Audio: {}", message) });
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

        // Collect all packets with their stream info and timestamps
        let mut video_packets = Vec::new();
        for (stream, packet) in video_input.packets() {
            if stream.index() == video_stream_index {
                video_packets.push(packet);
            }
        }

        let mut audio_packets = Vec::new();
        for (stream, packet) in audio_input.packets() {
            if stream.index() == audio_stream_index {
                audio_packets.push(packet);
            }
        }

        println!("🎬 [MUX] Collected {} video packets, {} audio packets",
                 video_packets.len(), audio_packets.len());

        // Report first and last timestamps
        if !video_packets.is_empty() {
            println!("🎬 [MUX] Video PTS range: {} to {}",
                     video_packets[0].pts().unwrap_or(0),
                     video_packets[video_packets.len()-1].pts().unwrap_or(0));
        }
        if !audio_packets.is_empty() {
            println!("🎵 [MUX] Audio PTS range: {} to {}",
                     audio_packets[0].pts().unwrap_or(0),
                     audio_packets[audio_packets.len()-1].pts().unwrap_or(0));
        }

        // Interleave packets by comparing timestamps in a common time base (use microseconds)
        let mut v_idx = 0;
        let mut a_idx = 0;
        let mut interleave_log_count = 0;

        while v_idx < video_packets.len() || a_idx < audio_packets.len() {
            let write_video = if v_idx >= video_packets.len() {
                false // No more video
            } else if a_idx >= audio_packets.len() {
                true // No more audio, write video
            } else {
                // Compare timestamps - convert both to microseconds
                let v_pts = video_packets[v_idx].pts().unwrap_or(0);
                let a_pts = audio_packets[a_idx].pts().unwrap_or(0);

                // Convert to microseconds: pts * 1000000 * tb.num / tb.den
                let v_us = v_pts * 1_000_000 * video_input_tb.0 as i64 / video_input_tb.1 as i64;
                let a_us = a_pts * 1_000_000 * audio_input_tb.0 as i64 / audio_input_tb.1 as i64;

                v_us <= a_us // Write video if it comes before or at same time as audio
            };

            if write_video {
                let mut packet = video_packets[v_idx].clone();
                packet.set_stream(0);
                packet.rescale_ts(video_input_tb, video_output_tb);

                if interleave_log_count < 10 {
                    println!("🎬 [MUX] Writing V packet {} - PTS={:?}, DTS={:?}, Duration={:?}",
                             v_idx, packet.pts(), packet.dts(), packet.duration());
                    interleave_log_count += 1;
                }

                packet.write_interleaved(&mut output)
                    .map_err(|e| format!("Failed to write video packet: {}", e))?;
                v_idx += 1;
            } else {
                let mut packet = audio_packets[a_idx].clone();
                packet.set_stream(1);
                packet.rescale_ts(audio_input_tb, audio_output_tb);

                if interleave_log_count < 10 {
                    println!("🎵 [MUX] Writing A packet {} - PTS={:?}, DTS={:?}, Duration={:?}",
                             a_idx, packet.pts(), packet.dts(), packet.duration());
                    interleave_log_count += 1;
                }

                packet.write_interleaved(&mut output)
                    .map_err(|e| format!("Failed to write audio packet: {}", e))?;
                a_idx += 1;
            }
        }

        println!("🎬 [MUX] Wrote {} video packets, {} audio packets", v_idx, a_idx);

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
        // Check parallel export first
        if self.parallel_export.is_some() {
            return true;
        }

        // Check single export
        if let Some(handle) = &self.thread_handle {
            !handle.is_finished()
        } else {
            false
        }
    }

    /// Wait for the export to complete
    ///
    /// This blocks until the export thread finishes.
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

        // Send start notification
        progress_tx
            .send(ExportProgress::Started { total_frames: 0 })
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
            start_time: settings.start_time,
            end_time: settings.end_time,
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
        // GPU resources will be initialized lazily on first frame (needs device)
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
        // GPU resources will be initialized lazily on first frame (needs device)
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
        });

        // Initialize parallel export state
        self.parallel_export = Some(ParallelExportState {
            video_progress_rx,
            audio_progress_rx,
            video_thread,
            audio_thread,
            temp_video_path,
            temp_audio_path,
            final_output_path: output_path,
            video_progress: None,
            audio_progress: None,
        });

        println!("🎬🎵 [PARALLEL EXPORT] Both threads spawned, ready for frames");
        Ok(())
    }

    /// Render and send the next video frame (call from main thread)
    ///
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
        let state = self.video_state.as_mut()
            .ok_or("No video export in progress")?;

        if state.current_frame >= state.total_frames {
            // All frames rendered, signal encoder thread
            if let Some(tx) = state.frame_tx.take() {
                tx.send(VideoFrameMessage::Done).ok();
            }
            // Clean up GPU resources
            state.gpu_resources = None;
            return Ok(false);
        }

        // Calculate timestamp for this frame
        let timestamp = state.start_time + (state.current_frame as f64 / state.framerate);

        // Get frame dimensions from export settings
        let width = state.width;
        let height = state.height;

        // Initialize GPU resources on first frame (needs device)
        if state.gpu_resources.is_none() {
            println!("🎬 [VIDEO EXPORT] Initializing HDR GPU resources for {}x{}", width, height);
            state.gpu_resources = Some(video_exporter::ExportGpuResources::new(device, width, height));
        }

        // Render frame to RGBA buffer using HDR pipeline (with effects)
        let mut rgba_buffer = vec![0u8; (width * height * 4) as usize];
        let gpu_resources = state.gpu_resources.as_mut().unwrap();
        video_exporter::render_frame_to_rgba_hdr(
            document,
            timestamp,
            width,
            height,
            device,
            queue,
            renderer,
            image_cache,
            video_manager,
            gpu_resources,
            &mut rgba_buffer,
        )?;

        // Send frame to encoder thread
        if let Some(tx) = &state.frame_tx {
            tx.send(VideoFrameMessage::Frame {
                frame_num: state.current_frame,
                timestamp,
                rgba_data: rgba_buffer,
            }).map_err(|_| "Failed to send frame to encoder")?;
        }

        state.current_frame += 1;

        // Return true if more frames remain
        Ok(state.current_frame < state.total_frames)
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
            Ok(VideoFrameMessage::Frame { frame_num, timestamp, rgba_data }) => {
                println!("🧵 [ENCODER] Received first frame ({} bytes)", rgba_data.len());
                Some((frame_num, timestamp, rgba_data))
            }
            Ok(VideoFrameMessage::Done) => {
                return Err("No frames to encode".to_string());
            }
            Err(_) => {
                return Err("Frame channel disconnected before first frame".to_string());
            }
        };

        // Determine dimensions from first frame
        let (width, height) = if let Some((_, _, ref rgba_data)) = first_frame {
            // Calculate dimensions from buffer size (RGBA = 4 bytes per pixel)
            let pixel_count = rgba_data.len() / 4;
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
        if let Some((frame_num, timestamp, rgba_data)) = first_frame {
            Self::encode_frame(
                &mut encoder,
                &mut output,
                &rgba_data,
                width,
                height,
                timestamp,
            )?;

            // Send progress update for first frame
            progress_tx.send(ExportProgress::FrameRendered {
                frame: 1,
                total: total_frames,
            }).ok();

            println!("🧵 [ENCODER] Encoded frame {}", frame_num);
        }

        // Process remaining frames
        let mut frames_encoded = 1;
        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err("Export cancelled by user".to_string());
            }

            match frame_rx.recv() {
                Ok(VideoFrameMessage::Frame { frame_num, timestamp, rgba_data }) => {
                    Self::encode_frame(
                        &mut encoder,
                        &mut output,
                        &rgba_data,
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

                    if frames_encoded % 30 == 0 || frames_encoded == frame_num + 1 {
                        println!("🧵 [ENCODER] Encoded frame {}/{}", frames_encoded, total_frames);
                    }
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

    /// Encode a single RGBA frame
    fn encode_frame(
        encoder: &mut ffmpeg_next::encoder::Video,
        output: &mut ffmpeg_next::format::context::Output,
        rgba_data: &[u8],
        width: u32,
        height: u32,
        timestamp: f64,
    ) -> Result<(), String> {
        // Convert RGBA to YUV420p
        let (y_plane, u_plane, v_plane) = video_exporter::rgba_to_yuv420p(rgba_data, width, height);

        // Create FFmpeg video frame
        let mut video_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
        );

        // Copy YUV planes to frame
        unsafe {
            let y_dest = video_frame.data_mut(0);
            std::ptr::copy_nonoverlapping(y_plane.as_ptr(), y_dest.as_mut_ptr(), y_plane.len());

            let u_dest = video_frame.data_mut(1);
            std::ptr::copy_nonoverlapping(u_plane.as_ptr(), u_dest.as_mut_ptr(), u_plane.len());

            let v_dest = video_frame.data_mut(2);
            std::ptr::copy_nonoverlapping(v_plane.as_ptr(), v_dest.as_mut_ptr(), v_plane.len());
        }

        // Set PTS (presentation timestamp) in encoder's time base
        // Encoder time base is 1/(framerate * 1000), so PTS = timestamp * (framerate * 1000)
        let encoder_tb = encoder.time_base();
        let pts = (timestamp * encoder_tb.1 as f64) as i64;
        println!("🎬 [ENCODE] Frame timestamp={:.3}s, encoder_tb={}/{}, calculated PTS={}",
                 timestamp, encoder_tb.0, encoder_tb.1, pts);
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
