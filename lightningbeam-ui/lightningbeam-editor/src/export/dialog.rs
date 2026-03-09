//! Export dialog UI
//!
//! Provides a user interface for configuring and starting audio/video exports.

use eframe::egui;
use lightningbeam_core::export::{
    AudioExportSettings, AudioFormat,
    ImageExportSettings, ImageFormat,
    VideoExportSettings, VideoCodec, VideoQuality,
};
use std::path::PathBuf;

/// Hint about document content, used to pick a smart default export type.
pub struct DocumentHint {
    pub has_video: bool,
    pub has_audio: bool,
    pub has_raster: bool,
    pub has_vector: bool,
    pub current_time: f64,
    pub doc_width: u32,
    pub doc_height: u32,
}

/// Export type selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportType {
    Audio,
    Image,
    Video,
}

/// Export result from dialog
#[derive(Debug, Clone)]
pub enum ExportResult {
    AudioOnly(AudioExportSettings, PathBuf),
    Image(ImageExportSettings, PathBuf),
    VideoOnly(VideoExportSettings, PathBuf),
    VideoWithAudio(VideoExportSettings, AudioExportSettings, PathBuf),
}

/// Export dialog state
pub struct ExportDialog {
    /// Is the dialog open?
    pub open: bool,

    /// Export type (Audio or Video)
    pub export_type: ExportType,

    /// Audio export settings
    pub audio_settings: AudioExportSettings,

    /// Image export settings
    pub image_settings: ImageExportSettings,

    /// Video export settings
    pub video_settings: VideoExportSettings,

    /// Include audio with video?
    pub include_audio: bool,

    /// Output file path
    pub output_path: Option<PathBuf>,

    /// Error message (if any)
    pub error_message: Option<String>,

    /// Whether advanced settings are shown
    pub show_advanced: bool,

    /// Selected video preset index
    pub selected_video_preset: usize,

    /// Output filename (editable text, without directory)
    pub output_filename: String,

    /// Output directory
    pub output_dir: PathBuf,

    /// Project name from the last `open()` call — used to detect file switches.
    current_project: String,

    /// Export type used the last time the user actually clicked Export for `current_project`.
    last_export_type: Option<ExportType>,
}

impl Default for ExportDialog {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        let music_dir = {
            let m = home.join("Music");
            if m.is_dir() { m } else { home }
        };

        Self {
            open: false,
            export_type: ExportType::Audio,
            audio_settings: AudioExportSettings::standard_mp3(),
            image_settings: ImageExportSettings::default(),
            video_settings: VideoExportSettings::default(),
            include_audio: true,
            output_path: None,
            error_message: None,
            show_advanced: false,
            selected_video_preset: 0,
            output_filename: String::new(),
            current_project: String::new(),
            last_export_type: None,
            output_dir: music_dir,
        }
    }
}

impl ExportDialog {
    /// Open the dialog with default settings, using `hint` to pick a smart default tab.
    pub fn open(&mut self, timeline_duration: f64, project_name: &str, hint: &DocumentHint) {
        self.open = true;
        self.audio_settings.end_time = timeline_duration;
        self.video_settings.end_time = timeline_duration;
        self.image_settings.time     = hint.current_time;
        // Propagate document dimensions as defaults (None means "use doc size").
        self.image_settings.width  = None;
        self.image_settings.height = None;
        self.error_message = None;

        // Determine export type: prefer the type used last time for this file,
        // then fall back to document-content hints.
        let same_project = self.current_project == project_name;
        self.export_type = if same_project && self.last_export_type.is_some() {
            self.last_export_type.unwrap()
        } else {
            let only_audio  = hint.has_audio  && !hint.has_video && !hint.has_raster && !hint.has_vector;
            let only_raster = hint.has_raster && !hint.has_video && !hint.has_audio  && !hint.has_vector;
            if hint.has_video        { ExportType::Video }
            else if only_audio       { ExportType::Audio }
            else if only_raster      { ExportType::Image }
            else                     { self.export_type  } // keep current as fallback
        };
        self.current_project = project_name.to_owned();

        // Pre-populate filename from project name if not already set.
        if self.output_filename.is_empty() || !self.output_filename.contains(project_name) {
            self.output_filename = format!("{}.{}", project_name, self.current_extension());
        }
    }

    /// Extension for the currently selected export type.
    fn current_extension(&self) -> &'static str {
        match self.export_type {
            ExportType::Audio => self.audio_settings.format.extension(),
            ExportType::Image => self.image_settings.format.extension(),
            ExportType::Video => self.video_settings.codec.container_format(),
        }
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.open = false;
        self.error_message = None;
    }

    /// Update the filename extension to match the current format
    fn update_filename_extension(&mut self) {
        let ext = self.current_extension();
        // Replace extension in filename
        if let Some(dot_pos) = self.output_filename.rfind('.') {
            self.output_filename.truncate(dot_pos + 1);
            self.output_filename.push_str(ext);
        } else if !self.output_filename.is_empty() {
            self.output_filename.push('.');
            self.output_filename.push_str(ext);
        }
    }

    /// Build the full output path from directory + filename
    fn build_output_path(&self) -> PathBuf {
        self.output_dir.join(&self.output_filename)
    }

    /// Render the export dialog
    ///
    /// Returns Some(ExportResult) if the user clicked Export, None otherwise.
    pub fn render(&mut self, ctx: &egui::Context) -> Option<ExportResult> {
        if !self.open {
            return None;
        }

        let mut should_export = false;
        let mut should_close = false;

        let window_title = match self.export_type {
            ExportType::Audio => "Export Audio",
            ExportType::Image => "Export Image",
            ExportType::Video => "Export Video",
        };

        let modal_response = egui::Modal::new(egui::Id::new("export_dialog_modal"))
            .show(ctx, |ui| {
                ui.set_width(500.0);

                ui.heading(window_title);
                ui.add_space(8.0);

                // Error message (if any)
                if let Some(error) = &self.error_message {
                    ui.colored_label(egui::Color32::RED, error);
                    ui.add_space(8.0);
                }

                // Export type selection (tabs)
                ui.horizontal(|ui| {
                    for (variant, label) in [
                        (ExportType::Audio, "Audio"),
                        (ExportType::Image, "Image"),
                        (ExportType::Video, "Video"),
                    ] {
                        if ui.selectable_value(&mut self.export_type, variant, label).clicked() {
                            self.update_filename_extension();
                        }
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);

                // Basic settings
                match self.export_type {
                    ExportType::Audio => self.render_audio_basic(ui),
                    ExportType::Image => self.render_image_settings(ui),
                    ExportType::Video => self.render_video_basic(ui),
                }

                ui.add_space(12.0);

                // Output file
                self.render_output_selection(ui);

                ui.add_space(4.0);

                // Advanced toggle
                ui.toggle_value(&mut self.show_advanced, "Advanced settings");

                if self.show_advanced {
                    ui.add_space(8.0);
                    match self.export_type {
                        ExportType::Audio => self.render_audio_advanced(ui),
                        ExportType::Image => self.render_image_advanced(ui),
                        ExportType::Video => self.render_video_advanced(ui),
                    }
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

        // Close on backdrop click or escape
        if modal_response.backdrop_response.clicked() {
            should_close = true;
        }

        if should_close {
            self.close();
            return None;
        }

        if should_export {
            self.output_path = Some(self.build_output_path());
            return self.handle_export();
        }

        None
    }

    /// Render basic audio settings (format + filename)
    fn render_audio_basic(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Format:");
            let prev_format = self.audio_settings.format;
            egui::ComboBox::from_id_salt("audio_format")
                .selected_text(self.audio_settings.format.name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.audio_settings.format, AudioFormat::Mp3, "MP3");
                    ui.selectable_value(&mut self.audio_settings.format, AudioFormat::Aac, "AAC");
                    ui.selectable_value(&mut self.audio_settings.format, AudioFormat::Flac, "FLAC (Lossless)");
                    ui.selectable_value(&mut self.audio_settings.format, AudioFormat::Wav, "WAV (Uncompressed)");
                });
            if self.audio_settings.format != prev_format {
                self.update_filename_extension();
                // Apply sensible defaults when switching formats
                match self.audio_settings.format {
                    AudioFormat::Mp3 => {
                        self.audio_settings.sample_rate = 44100;
                        self.audio_settings.bitrate_kbps = 192;
                    }
                    AudioFormat::Aac => {
                        self.audio_settings.sample_rate = 44100;
                        self.audio_settings.bitrate_kbps = 256;
                    }
                    AudioFormat::Flac | AudioFormat::Wav => {
                        self.audio_settings.sample_rate = 48000;
                        self.audio_settings.bit_depth = 24;
                    }
                }
            }
        });
    }

    /// Render basic image export settings (format, quality, transparency).
    fn render_image_settings(&mut self, ui: &mut egui::Ui) {
        // Format
        ui.horizontal(|ui| {
            ui.label("Format:");
            let prev = self.image_settings.format;
            egui::ComboBox::from_id_salt("image_format")
                .selected_text(self.image_settings.format.name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.image_settings.format, ImageFormat::Png,  "PNG");
                    ui.selectable_value(&mut self.image_settings.format, ImageFormat::Jpeg, "JPEG");
                    ui.selectable_value(&mut self.image_settings.format, ImageFormat::WebP, "WebP");
                });
            if self.image_settings.format != prev {
                self.update_filename_extension();
            }
        });

        // Quality (JPEG / WebP only)
        if self.image_settings.format.has_quality() {
            ui.horizontal(|ui| {
                ui.label("Quality:");
                ui.add(egui::Slider::new(&mut self.image_settings.quality, 1..=100));
            });
        }

        // Transparency (PNG / WebP only — JPEG has no alpha)
        if self.image_settings.format != ImageFormat::Jpeg {
            ui.checkbox(&mut self.image_settings.allow_transparency, "Allow transparency");
        }
    }

    /// Render advanced image export settings (time, resolution override).
    fn render_image_advanced(&mut self, ui: &mut egui::Ui) {
        // Time (which frame to export)
        ui.horizontal(|ui| {
            ui.label("Time:");
            ui.add(egui::DragValue::new(&mut self.image_settings.time)
                .speed(0.01)
                .range(0.0..=f64::MAX)
                .suffix(" s"));
        });

        // Resolution override (None = use document size; 0 means "use doc size")
        ui.horizontal(|ui| {
            ui.label("Size:");
            let mut w = self.image_settings.width.unwrap_or(0);
            let mut h = self.image_settings.height.unwrap_or(0);
            let changed_w = ui.add(egui::DragValue::new(&mut w).range(0..=u32::MAX).prefix("W ")).changed();
            let changed_h = ui.add(egui::DragValue::new(&mut h).range(0..=u32::MAX).prefix("H ")).changed();
            if changed_w { self.image_settings.width  = if w == 0 { None } else { Some(w) }; }
            if changed_h { self.image_settings.height = if h == 0 { None } else { Some(h) }; }
            ui.weak("(0 = document size)");
        });
    }

    /// Render advanced audio settings (sample rate, channels, bit depth, bitrate, time range)
    fn render_audio_advanced(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Sample Rate:");
            egui::ComboBox::from_id_salt("sample_rate")
                .selected_text(format!("{} Hz", self.audio_settings.sample_rate))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.audio_settings.sample_rate, 44100, "44100 Hz");
                    ui.selectable_value(&mut self.audio_settings.sample_rate, 48000, "48000 Hz");
                    ui.selectable_value(&mut self.audio_settings.sample_rate, 96000, "96000 Hz");
                });
        });

        ui.horizontal(|ui| {
            ui.label("Channels:");
            ui.radio_value(&mut self.audio_settings.channels, 1, "Mono");
            ui.radio_value(&mut self.audio_settings.channels, 2, "Stereo");
        });

        // Format-specific settings
        if self.audio_settings.format.supports_bit_depth() {
            ui.horizontal(|ui| {
                ui.label("Bit Depth:");
                ui.radio_value(&mut self.audio_settings.bit_depth, 16, "16-bit");
                ui.radio_value(&mut self.audio_settings.bit_depth, 24, "24-bit");
            });
        }

        if self.audio_settings.format.uses_bitrate() {
            ui.horizontal(|ui| {
                ui.label("Bitrate:");
                egui::ComboBox::from_id_salt("bitrate")
                    .selected_text(format!("{} kbps", self.audio_settings.bitrate_kbps))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.audio_settings.bitrate_kbps, 128, "128 kbps");
                        ui.selectable_value(&mut self.audio_settings.bitrate_kbps, 192, "192 kbps");
                        ui.selectable_value(&mut self.audio_settings.bitrate_kbps, 256, "256 kbps");
                        ui.selectable_value(&mut self.audio_settings.bitrate_kbps, 320, "320 kbps");
                    });
            });
        }

        ui.add_space(8.0);

        // Time range
        self.render_time_range(ui);
    }

    /// Video presets: (name, codec, quality, width, height, fps)
    const VIDEO_PRESETS: &'static [(&'static str, VideoCodec, VideoQuality, u32, u32, f64)] = &[
        ("1080p H.264 (Standard)",  VideoCodec::H264, VideoQuality::High,     1920, 1080, 30.0),
        ("1080p H.264 60fps",       VideoCodec::H264, VideoQuality::High,     1920, 1080, 60.0),
        ("4K H.264",                VideoCodec::H264, VideoQuality::VeryHigh, 3840, 2160, 30.0),
        ("720p H.264 (Small)",      VideoCodec::H264, VideoQuality::Medium,   1280,  720, 30.0),
        ("1080p H.265 (Smaller)",   VideoCodec::H265, VideoQuality::High,     1920, 1080, 30.0),
        ("1080p VP9 (WebM)",        VideoCodec::VP9,  VideoQuality::High,     1920, 1080, 30.0),
        ("1080p ProRes 422",        VideoCodec::ProRes422, VideoQuality::VeryHigh, 1920, 1080, 30.0),
    ];

    /// Render basic video settings (preset dropdown)
    fn render_video_basic(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Preset:");
            egui::ComboBox::from_id_salt("video_preset")
                .selected_text(Self::VIDEO_PRESETS[self.selected_video_preset].0)
                .show_ui(ui, |ui| {
                    for (i, preset) in Self::VIDEO_PRESETS.iter().enumerate() {
                        if ui.selectable_value(&mut self.selected_video_preset, i, preset.0).clicked() {
                            let (_, codec, quality, w, h, fps) = *preset;
                            self.video_settings.codec = codec;
                            self.video_settings.quality = quality;
                            self.video_settings.width = Some(w);
                            self.video_settings.height = Some(h);
                            self.video_settings.framerate = fps;
                            self.update_filename_extension();
                        }
                    }
                });
        });
    }

    /// Render advanced video settings (codec, resolution, framerate, quality, time range)
    fn render_video_advanced(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Codec:");
            egui::ComboBox::from_id_salt("video_codec")
                .selected_text(format!("{:?}", self.video_settings.codec))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.video_settings.codec, VideoCodec::H264, "H.264 (Most Compatible)");
                    ui.selectable_value(&mut self.video_settings.codec, VideoCodec::H265, "H.265 (Better Compression)");
                    ui.selectable_value(&mut self.video_settings.codec, VideoCodec::VP8, "VP8 (WebM)");
                    ui.selectable_value(&mut self.video_settings.codec, VideoCodec::VP9, "VP9 (WebM)");
                    ui.selectable_value(&mut self.video_settings.codec, VideoCodec::ProRes422, "ProRes 422 (Professional)");
                });
        });

        ui.horizontal(|ui| {
            ui.label("Resolution:");
            let mut custom_width = self.video_settings.width.unwrap_or(1920);
            if ui.add(egui::DragValue::new(&mut custom_width).range(1..=7680)).changed() {
                self.video_settings.width = Some(custom_width);
            }
            ui.label("x");
            let mut custom_height = self.video_settings.height.unwrap_or(1080);
            if ui.add(egui::DragValue::new(&mut custom_height).range(1..=4320)).changed() {
                self.video_settings.height = Some(custom_height);
            }
        });

        ui.horizontal(|ui| {
            if ui.small_button("1080p").clicked() {
                self.video_settings.width = Some(1920);
                self.video_settings.height = Some(1080);
            }
            if ui.small_button("4K").clicked() {
                self.video_settings.width = Some(3840);
                self.video_settings.height = Some(2160);
            }
            if ui.small_button("720p").clicked() {
                self.video_settings.width = Some(1280);
                self.video_settings.height = Some(720);
            }
        });

        ui.horizontal(|ui| {
            ui.label("FPS:");
            egui::ComboBox::from_id_salt("framerate")
                .selected_text(format!("{}", self.video_settings.framerate as u32))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.video_settings.framerate, 24.0, "24");
                    ui.selectable_value(&mut self.video_settings.framerate, 30.0, "30");
                    ui.selectable_value(&mut self.video_settings.framerate, 60.0, "60");
                });
        });

        ui.horizontal(|ui| {
            ui.label("Quality:");
            egui::ComboBox::from_id_salt("video_quality")
                .selected_text(self.video_settings.quality.name())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.video_settings.quality, VideoQuality::Low, VideoQuality::Low.name());
                    ui.selectable_value(&mut self.video_settings.quality, VideoQuality::Medium, VideoQuality::Medium.name());
                    ui.selectable_value(&mut self.video_settings.quality, VideoQuality::High, VideoQuality::High.name());
                    ui.selectable_value(&mut self.video_settings.quality, VideoQuality::VeryHigh, VideoQuality::VeryHigh.name());
                });
        });

        ui.checkbox(&mut self.include_audio, "Include Audio");

        ui.add_space(8.0);

        // Time range
        self.render_time_range(ui);
    }

    /// Render time range UI (common to both audio and video)
    fn render_time_range(&mut self, ui: &mut egui::Ui) {
        let (start_time, end_time) = match self.export_type {
            ExportType::Audio => (&mut self.audio_settings.start_time, &mut self.audio_settings.end_time),
            ExportType::Image => return, // image uses a single time field, not a range
            ExportType::Video => (&mut self.video_settings.start_time, &mut self.video_settings.end_time),
        };

        ui.horizontal(|ui| {
            ui.label("Start:");
            ui.add(egui::DragValue::new(start_time)
                .speed(0.1)
                .range(0.0..=*end_time)
                .suffix(" s"));

            ui.label("End:");
            ui.add(egui::DragValue::new(end_time)
                .speed(0.1)
                .range(*start_time..=f64::MAX)
                .suffix(" s"));
        });

        let duration = *end_time - *start_time;
        ui.label(format!("Duration: {:.2} seconds", duration));
    }

    /// Render output file selection UI — single OS save-file dialog.
    fn render_output_selection(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Show the current path (truncated if long).
            let full_path = self.build_output_path();
            let path_str = full_path.display().to_string();
            ui.label("Save to:");
            ui.add(egui::Label::new(
                egui::RichText::new(&path_str).weak()
            ).truncate());
        });

        if ui.button("Choose location...").clicked() {
            let ext = self.current_extension();
            let mut dialog = rfd::FileDialog::new()
                .set_directory(&self.output_dir)
                .set_file_name(&self.output_filename)
                .add_filter(ext.to_uppercase(), &[ext]);
            if let Some(path) = dialog.save_file() {
                if let Some(dir) = path.parent() {
                    self.output_dir = dir.to_path_buf();
                }
                if let Some(name) = path.file_name() {
                    self.output_filename = name.to_string_lossy().into_owned();
                    // Ensure the extension matches the selected format.
                    self.update_filename_extension();
                }
            }
        }
    }

    /// Handle export button click
    fn handle_export(&mut self) -> Option<ExportResult> {
        if self.output_filename.trim().is_empty() {
            self.error_message = Some("Please enter a filename".to_string());
            return None;
        }

        let output_path = self.output_path.clone().unwrap();

        // Remember this export type for next time this file is opened.
        self.last_export_type = Some(self.export_type);

        let result = match self.export_type {
            ExportType::Image => {
                if let Err(err) = self.image_settings.validate() {
                    self.error_message = Some(err);
                    return None;
                }
                Some(ExportResult::Image(self.image_settings.clone(), output_path))
            }
            ExportType::Audio => {
                // Validate audio settings
                if let Err(err) = self.audio_settings.validate() {
                    self.error_message = Some(err);
                    return None;
                }

                Some(ExportResult::AudioOnly(self.audio_settings.clone(), output_path))
            }
            ExportType::Video => {
                // Validate video settings
                if let Err(err) = self.video_settings.validate() {
                    self.error_message = Some(err);
                    return None;
                }

                if self.include_audio {
                    // Validate audio settings too
                    if let Err(err) = self.audio_settings.validate() {
                        self.error_message = Some(err);
                        return None;
                    }

                    // Sync time range from video to audio
                    self.audio_settings.start_time = self.video_settings.start_time;
                    self.audio_settings.end_time = self.video_settings.end_time;

                    Some(ExportResult::VideoWithAudio(
                        self.video_settings.clone(),
                        self.audio_settings.clone(),
                        output_path,
                    ))
                } else {
                    Some(ExportResult::VideoOnly(self.video_settings.clone(), output_path))
                }
            }
        };

        self.close();
        result
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

        egui::Modal::new(egui::Id::new("export_progress_modal"))
            .show(ctx, |ui| {
                ui.set_width(400.0);

                ui.heading("Exporting...");
                ui.add_space(8.0);

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
