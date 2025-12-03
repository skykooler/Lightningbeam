//! Export dialog UI
//!
//! Provides a user interface for configuring and starting audio/video exports.

use eframe::egui;
use lightningbeam_core::export::{AudioExportSettings, AudioFormat};
use std::path::PathBuf;

/// Export dialog state
pub struct ExportDialog {
    /// Is the dialog open?
    pub open: bool,

    /// Export settings
    pub settings: AudioExportSettings,

    /// Output file path
    pub output_path: Option<PathBuf>,

    /// Selected preset index (for UI)
    pub selected_preset: usize,

    /// Error message (if any)
    pub error_message: Option<String>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        Self {
            open: false,
            settings: AudioExportSettings::default(),
            output_path: None,
            selected_preset: 0,
            error_message: None,
        }
    }
}

impl ExportDialog {
    /// Open the dialog with default settings
    pub fn open(&mut self, timeline_duration: f64) {
        self.open = true;
        self.settings.end_time = timeline_duration;
        self.error_message = None;
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.open = false;
        self.error_message = None;
    }

    /// Render the export dialog
    ///
    /// Returns Some(settings, output_path) if the user clicked Export,
    /// None otherwise.
    pub fn render(&mut self, ctx: &egui::Context) -> Option<(AudioExportSettings, PathBuf)> {
        if !self.open {
            return None;
        }

        let mut should_export = false;
        let mut should_close = false;

        egui::Window::new("Export Audio")
            .open(&mut self.open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(500.0);

                // Error message (if any)
                if let Some(error) = &self.error_message {
                    ui.colored_label(egui::Color32::RED, error);
                    ui.add_space(8.0);
                }

                // Preset selection
                ui.heading("Preset");
                ui.horizontal(|ui| {
                    let presets = [
                        ("High Quality WAV", AudioExportSettings::high_quality_wav()),
                        ("High Quality FLAC", AudioExportSettings::high_quality_flac()),
                        ("Standard MP3", AudioExportSettings::standard_mp3()),
                        ("Standard AAC", AudioExportSettings::standard_aac()),
                        ("High Quality MP3", AudioExportSettings::high_quality_mp3()),
                        ("High Quality AAC", AudioExportSettings::high_quality_aac()),
                        ("Podcast MP3", AudioExportSettings::podcast_mp3()),
                        ("Podcast AAC", AudioExportSettings::podcast_aac()),
                    ];

                    egui::ComboBox::from_id_source("export_preset")
                        .selected_text(presets[self.selected_preset].0)
                        .show_ui(ui, |ui| {
                            for (i, (name, _)) in presets.iter().enumerate() {
                                if ui.selectable_value(&mut self.selected_preset, i, *name).clicked() {
                                    // Save current time range before applying preset
                                    let saved_start = self.settings.start_time;
                                    let saved_end = self.settings.end_time;
                                    self.settings = presets[i].1.clone();
                                    // Restore time range
                                    self.settings.start_time = saved_start;
                                    self.settings.end_time = saved_end;
                                }
                            }
                        });
                });

                ui.add_space(12.0);

                // Format settings
                ui.heading("Format");
                ui.horizontal(|ui| {
                    ui.label("Format:");
                    egui::ComboBox::from_id_source("audio_format")
                        .selected_text(self.settings.format.name())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.format, AudioFormat::Wav, "WAV (Uncompressed)");
                            ui.selectable_value(&mut self.settings.format, AudioFormat::Flac, "FLAC (Lossless)");
                            ui.selectable_value(&mut self.settings.format, AudioFormat::Mp3, "MP3");
                            ui.selectable_value(&mut self.settings.format, AudioFormat::Aac, "AAC");
                        });
                });

                ui.add_space(8.0);

                // Audio settings
                ui.horizontal(|ui| {
                    ui.label("Sample Rate:");
                    egui::ComboBox::from_id_source("sample_rate")
                        .selected_text(format!("{} Hz", self.settings.sample_rate))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.sample_rate, 44100, "44100 Hz");
                            ui.selectable_value(&mut self.settings.sample_rate, 48000, "48000 Hz");
                            ui.selectable_value(&mut self.settings.sample_rate, 96000, "96000 Hz");
                        });
                });

                ui.horizontal(|ui| {
                    ui.label("Channels:");
                    ui.radio_value(&mut self.settings.channels, 1, "Mono");
                    ui.radio_value(&mut self.settings.channels, 2, "Stereo");
                });

                ui.add_space(8.0);

                // Format-specific settings
                if self.settings.format.supports_bit_depth() {
                    ui.horizontal(|ui| {
                        ui.label("Bit Depth:");
                        ui.radio_value(&mut self.settings.bit_depth, 16, "16-bit");
                        ui.radio_value(&mut self.settings.bit_depth, 24, "24-bit");
                    });
                }

                if self.settings.format.uses_bitrate() {
                    ui.horizontal(|ui| {
                        ui.label("Bitrate:");
                        egui::ComboBox::from_id_source("bitrate")
                            .selected_text(format!("{} kbps", self.settings.bitrate_kbps))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.settings.bitrate_kbps, 128, "128 kbps");
                                ui.selectable_value(&mut self.settings.bitrate_kbps, 192, "192 kbps");
                                ui.selectable_value(&mut self.settings.bitrate_kbps, 256, "256 kbps");
                                ui.selectable_value(&mut self.settings.bitrate_kbps, 320, "320 kbps");
                            });
                    });
                }

                ui.add_space(12.0);

                // Time range
                ui.heading("Time Range");
                ui.horizontal(|ui| {
                    ui.label("Start:");
                    ui.add(egui::DragValue::new(&mut self.settings.start_time)
                        .speed(0.1)
                        .clamp_range(0.0..=self.settings.end_time)
                        .suffix(" s"));

                    ui.label("End:");
                    ui.add(egui::DragValue::new(&mut self.settings.end_time)
                        .speed(0.1)
                        .clamp_range(self.settings.start_time..=f64::MAX)
                        .suffix(" s"));
                });

                let duration = self.settings.duration();
                ui.label(format!("Duration: {:.2} seconds", duration));

                ui.add_space(12.0);

                // Output file path
                ui.heading("Output");
                ui.horizontal(|ui| {
                    let path_text = self.output_path.as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "No file selected".to_string());

                    ui.label("File:");
                    ui.text_edit_singleline(&mut path_text.clone());

                    if ui.button("Browse...").clicked() {
                        // Open file dialog
                        let default_name = format!("audio.{}", self.settings.format.extension());
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name(&default_name)
                            .add_filter("Audio", &[self.settings.format.extension()])
                            .save_file()
                        {
                            self.output_path = Some(path);
                        }
                    }
                });

                ui.add_space(12.0);

                // Estimated file size
                if duration > 0.0 {
                    let estimated_mb = if self.settings.format.uses_bitrate() {
                        // Lossy: bitrate * duration / 8 / 1024
                        (self.settings.bitrate_kbps as f64 * duration) / 8.0 / 1024.0
                    } else {
                        // Lossless: sample_rate * channels * bit_depth * duration / 8 / 1024 / 1024
                        let compression_factor = if self.settings.format == AudioFormat::Flac { 0.6 } else { 1.0 };
                        (self.settings.sample_rate as f64 * self.settings.channels as f64 *
                         self.settings.bit_depth as f64 * duration * compression_factor) / 8.0 / 1024.0 / 1024.0
                    };
                    ui.label(format!("Estimated size: ~{:.1} MB", estimated_mb));
                }

                ui.add_space(16.0);

                // Buttons
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        should_close = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Export").clicked() {
                            should_export = true;
                        }
                    });
                });
            });

        if should_close {
            self.close();
            return None;
        }

        if should_export {
            // Validate settings
            if let Err(err) = self.settings.validate() {
                self.error_message = Some(err);
                return None;
            }

            // Check if output path is set
            if self.output_path.is_none() {
                self.error_message = Some("Please select an output file".to_string());
                return None;
            }

            // Return settings and path
            let result = Some((self.settings.clone(), self.output_path.clone().unwrap()));
            self.close();
            return result;
        }

        None
    }
}

/// Export progress dialog state
pub struct ExportProgressDialog {
    /// Is the dialog open?
    pub open: bool,

    /// Current progress message
    pub message: String,

    /// Progress (0.0 to 1.0)
    pub progress: f32,

    /// Start time for elapsed time calculation
    pub start_time: Option<std::time::Instant>,

    /// Was cancel requested?
    pub cancel_requested: bool,
}

impl Default for ExportProgressDialog {
    fn default() -> Self {
        Self {
            open: false,
            message: String::new(),
            progress: 0.0,
            start_time: None,
            cancel_requested: false,
        }
    }
}

impl ExportProgressDialog {
    /// Open the progress dialog
    pub fn open(&mut self) {
        self.open = true;
        self.message = "Starting export...".to_string();
        self.progress = 0.0;
        self.start_time = Some(std::time::Instant::now());
        self.cancel_requested = false;
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.open = false;
        self.start_time = None;
        self.cancel_requested = false;
    }

    /// Update progress
    pub fn update_progress(&mut self, message: String, progress: f32) {
        self.message = message;
        self.progress = progress.clamp(0.0, 1.0);
    }

    /// Render the export progress dialog
    ///
    /// Returns true if the user clicked Cancel
    pub fn render(&mut self, ctx: &egui::Context) -> bool {
        if !self.open {
            return false;
        }

        let mut should_cancel = false;

        egui::Window::new("Exporting...")
            .open(&mut self.open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(400.0);

                // Status message
                ui.label(&self.message);
                ui.add_space(8.0);

                // Progress bar
                let progress_text = format!("{:.0}%", self.progress * 100.0);
                ui.add(egui::ProgressBar::new(self.progress).text(progress_text));
                ui.add_space(8.0);

                // Elapsed time and estimate
                if let Some(start_time) = self.start_time {
                    let elapsed = start_time.elapsed();
                    let elapsed_secs = elapsed.as_secs();

                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Elapsed: {}:{:02}",
                            elapsed_secs / 60,
                            elapsed_secs % 60
                        ));

                        // Estimate remaining time if we have progress
                        if self.progress > 0.01 {
                            let total_estimated = elapsed.as_secs_f32() / self.progress;
                            let remaining = total_estimated - elapsed.as_secs_f32();
                            if remaining > 0.0 {
                                ui.label(format!(
                                    "  |  Remaining: ~{}:{:02}",
                                    (remaining as u64) / 60,
                                    (remaining as u64) % 60
                                ));
                            }
                        }
                    });
                }

                ui.add_space(12.0);

                // Cancel button
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            should_cancel = true;
                        }
                    });
                });
            });

        if should_cancel {
            self.cancel_requested = true;
        }

        should_cancel
    }
}
