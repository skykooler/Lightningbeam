//! Export functionality for audio and video
//!
//! This module provides the export orchestrator and progress tracking
//! for exporting audio and video from the timeline.

pub mod audio_exporter;
pub mod dialog;

use lightningbeam_core::export::{AudioExportSettings, ExportProgress};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Export orchestrator that manages the export process
pub struct ExportOrchestrator {
    /// Channel for receiving progress updates
    progress_rx: Option<Receiver<ExportProgress>>,

    /// Handle to the export thread
    thread_handle: Option<std::thread::JoinHandle<()>>,

    /// Cancel flag
    cancel_flag: Arc<AtomicBool>,
}

impl ExportOrchestrator {
    /// Create a new export orchestrator
    pub fn new() -> Self {
        Self {
            progress_rx: None,
            thread_handle: None,
            cancel_flag: Arc::new(AtomicBool::new(false)),
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
    pub fn poll_progress(&mut self) -> Option<ExportProgress> {
        if let Some(rx) = &self.progress_rx {
            match rx.try_recv() {
                Ok(progress) => {
                    println!("📨 [ORCHESTRATOR] Received progress: {:?}", std::mem::discriminant(&progress));
                    Some(progress)
                }
                Err(e) => {
                    // Only log occasionally to avoid spam
                    None
                }
            }
        } else {
            None
        }
    }

    /// Cancel the current export
    pub fn cancel(&mut self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// Check if an export is in progress
    pub fn is_exporting(&self) -> bool {
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

        // Convert settings to DAW backend format
        let daw_settings = daw_backend::audio::ExportSettings {
            format: match settings.format {
                lightningbeam_core::export::AudioFormat::Wav => daw_backend::audio::ExportFormat::Wav,
                lightningbeam_core::export::AudioFormat::Flac => daw_backend::audio::ExportFormat::Flac,
                lightningbeam_core::export::AudioFormat::Mp3 |
                lightningbeam_core::export::AudioFormat::Aac => {
                    // MP3/AAC not supported yet
                    progress_tx
                        .send(ExportProgress::Error {
                            message: format!("{} export not yet implemented. Please use WAV or FLAC format.", settings.format.name()),
                        })
                        .ok();
                    return;
                }
            },
            sample_rate: settings.sample_rate,
            channels: settings.channels,
            bit_depth: settings.bit_depth,
            mp3_bitrate: 320, // Not used for WAV/FLAC
            start_time: settings.start_time,
            end_time: settings.end_time,
        };

        println!("🧵 [EXPORT THREAD] Starting non-blocking export...");

        // Start the export (non-blocking - just sends the query)
        {
            let mut controller = audio_controller.lock().unwrap();
            println!("🧵 [EXPORT THREAD] Sending export query...");
            if let Err(e) = controller.start_export_audio(&daw_settings, &output_path) {
                println!("🧵 [EXPORT THREAD] Failed to start export: {}", e);
                progress_tx.send(ExportProgress::Error { message: e }).ok();
                return;
            }
            println!("🧵 [EXPORT THREAD] Export query sent, lock released");
        }

        // Poll for completion without holding the lock for extended periods
        let duration = settings.end_time - settings.start_time;
        let start_time = std::time::Instant::now();
        let result = loop {
            if cancel_flag.load(Ordering::Relaxed) {
                break Err("Export cancelled by user".to_string());
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
                    // Export completed
                    println!("🧵 [EXPORT THREAD] Export completed: {:?}", result.is_ok());
                    break result;
                }
                Ok(None) => {
                    // Still in progress - actual progress comes via AudioEvent::ExportProgress
                    // No need to send progress here
                }
                Err(e) => {
                    // Polling error (shouldn't happen)
                    println!("🧵 [EXPORT THREAD] Poll error: {}", e);
                    break Err(e);
                }
            }
        };
        println!("🧵 [EXPORT THREAD] Export loop finished");

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
}

impl Default for ExportOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}
