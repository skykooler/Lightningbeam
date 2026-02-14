//! Preferences dialog UI
//!
//! Provides a user interface for configuring application preferences

use eframe::egui;
use crate::config::AppConfig;
use crate::theme::{Theme, ThemeMode};

/// Preferences dialog state
pub struct PreferencesDialog {
    /// Is the dialog open?
    pub open: bool,

    /// Working copy of preferences (allows cancel to discard changes)
    working_prefs: PreferencesState,

    /// Original audio buffer size (to detect changes that need restart)
    original_buffer_size: u32,

    /// Error message (if validation fails)
    error_message: Option<String>,
}

/// Editable preferences state (working copy)
#[derive(Debug, Clone)]
struct PreferencesState {
    bpm: u32,
    framerate: u32,
    file_width: u32,
    file_height: u32,
    scroll_speed: f64,
    audio_buffer_size: u32,
    reopen_last_session: bool,
    restore_layout_from_file: bool,
    debug: bool,
    theme_mode: ThemeMode,
}

impl From<(&AppConfig, &Theme)> for PreferencesState {
    fn from((config, theme): (&AppConfig, &Theme)) -> Self {
        Self {
            bpm: config.bpm,
            framerate: config.framerate,
            file_width: config.file_width,
            file_height: config.file_height,
            scroll_speed: config.scroll_speed,
            audio_buffer_size: config.audio_buffer_size,
            reopen_last_session: config.reopen_last_session,
            restore_layout_from_file: config.restore_layout_from_file,
            debug: config.debug,
            theme_mode: theme.mode(),
        }
    }
}

impl Default for PreferencesState {
    fn default() -> Self {
        Self {
            bpm: 120,
            framerate: 24,
            file_width: 800,
            file_height: 600,
            scroll_speed: 1.0,
            audio_buffer_size: 256,
            reopen_last_session: false,
            restore_layout_from_file: true,
            debug: false,
            theme_mode: ThemeMode::System,
        }
    }
}

/// Result returned when preferences are saved
#[derive(Debug, Clone)]
pub struct PreferencesSaveResult {
    /// Whether audio buffer size changed (requires restart)
    pub buffer_size_changed: bool,
}

impl Default for PreferencesDialog {
    fn default() -> Self {
        Self {
            open: false,
            working_prefs: PreferencesState::default(),
            original_buffer_size: 256,
            error_message: None,
        }
    }
}

impl PreferencesDialog {
    /// Open the dialog with current config and theme
    pub fn open(&mut self, config: &AppConfig, theme: &Theme) {
        self.open = true;
        self.working_prefs = PreferencesState::from((config, theme));
        self.original_buffer_size = config.audio_buffer_size;
        self.error_message = None;
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.open = false;
        self.error_message = None;
    }

    /// Render the preferences dialog
    ///
    /// Returns Some(PreferencesSaveResult) if user clicked Save, None otherwise.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        config: &mut AppConfig,
        theme: &mut Theme,
    ) -> Option<PreferencesSaveResult> {
        if !self.open {
            return None;
        }

        let mut should_save = false;
        let mut should_cancel = false;
        let mut open = self.open;

        egui::Window::new("Preferences")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_width(500.0);

                // Error message
                if let Some(error) = &self.error_message {
                    ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
                    ui.add_space(8.0);
                }

                // Scrollable area for preferences sections
                egui::ScrollArea::vertical()
                    .max_height(400.0)
                    .show(ui, |ui| {
                        self.render_general_section(ui);
                        ui.add_space(8.0);
                        self.render_audio_section(ui);
                        ui.add_space(8.0);
                        self.render_appearance_section(ui);
                        ui.add_space(8.0);
                        self.render_startup_section(ui);
                        ui.add_space(8.0);
                        self.render_advanced_section(ui);
                    });

                ui.add_space(16.0);

                // Buttons
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        should_cancel = true;
                    }

                    if ui.button("Reset to Defaults").clicked() {
                        self.reset_to_defaults();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Save").clicked() {
                            should_save = true;
                        }
                    });
                });
            });

        // Update open state
        self.open = open;

        if should_cancel {
            self.close();
            return None;
        }

        if should_save {
            return self.handle_save(config, theme);
        }

        None
    }

    fn render_general_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("General")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Default BPM:");
                    ui.add(
                        egui::DragValue::new(&mut self.working_prefs.bpm)
                            .range(20..=300)
                            .speed(1.0),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Default Framerate:");
                    ui.add(
                        egui::DragValue::new(&mut self.working_prefs.framerate)
                            .range(1..=120)
                            .speed(1.0)
                            .suffix(" fps"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Default File Width:");
                    ui.add(
                        egui::DragValue::new(&mut self.working_prefs.file_width)
                            .range(100..=10000)
                            .speed(10.0)
                            .suffix(" px"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Default File Height:");
                    ui.add(
                        egui::DragValue::new(&mut self.working_prefs.file_height)
                            .range(100..=10000)
                            .speed(10.0)
                            .suffix(" px"),
                    );
                });

                ui.horizontal(|ui| {
                    ui.label("Scroll Speed:");
                    ui.add(
                        egui::DragValue::new(&mut self.working_prefs.scroll_speed)
                            .range(0.1..=10.0)
                            .speed(0.1),
                    );
                });
            });
    }

    fn render_audio_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Audio")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Audio Buffer Size:");

                    egui::ComboBox::from_id_salt("audio_buffer_size")
                        .selected_text(format!("{} samples", self.working_prefs.audio_buffer_size))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                128,
                                "128 samples (~3ms - Low latency)",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                256,
                                "256 samples (~6ms - Balanced)",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                512,
                                "512 samples (~12ms - Stable)",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                1024,
                                "1024 samples (~23ms - Very stable)",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                2048,
                                "2048 samples (~46ms - Low-end systems)",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.audio_buffer_size,
                                4096,
                                "4096 samples (~93ms - Very low-end systems)",
                            );
                        });
                });

                ui.label("⚠ Requires app restart to take effect");
            });
    }

    fn render_appearance_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Appearance")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Theme:");

                    egui::ComboBox::from_id_salt("theme_mode")
                        .selected_text(format!("{:?}", self.working_prefs.theme_mode))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.working_prefs.theme_mode,
                                ThemeMode::Light,
                                "Light",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.theme_mode,
                                ThemeMode::Dark,
                                "Dark",
                            );
                            ui.selectable_value(
                                &mut self.working_prefs.theme_mode,
                                ThemeMode::System,
                                "System",
                            );
                        });
                });
            });
    }

    fn render_startup_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Startup")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(
                    &mut self.working_prefs.reopen_last_session,
                    "Reopen last session on startup",
                );
                ui.checkbox(
                    &mut self.working_prefs.restore_layout_from_file,
                    "Restore layout when opening files",
                );
            });
    }

    fn render_advanced_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(&mut self.working_prefs.debug, "Enable debug mode");
            });
    }

    fn reset_to_defaults(&mut self) {
        self.working_prefs = PreferencesState::default();
        self.error_message = None;
    }

    fn handle_save(
        &mut self,
        config: &mut AppConfig,
        theme: &mut Theme,
    ) -> Option<PreferencesSaveResult> {
        // Create temp config for validation
        let mut temp_config = config.clone();
        temp_config.bpm = self.working_prefs.bpm;
        temp_config.framerate = self.working_prefs.framerate;
        temp_config.file_width = self.working_prefs.file_width;
        temp_config.file_height = self.working_prefs.file_height;
        temp_config.scroll_speed = self.working_prefs.scroll_speed;
        temp_config.audio_buffer_size = self.working_prefs.audio_buffer_size;
        temp_config.reopen_last_session = self.working_prefs.reopen_last_session;
        temp_config.restore_layout_from_file = self.working_prefs.restore_layout_from_file;
        temp_config.debug = self.working_prefs.debug;
        temp_config.theme_mode = self.working_prefs.theme_mode.to_string_lower();

        // Validate
        if let Err(err) = temp_config.validate() {
            self.error_message = Some(err);
            return None;
        }

        // Check if buffer size changed
        let buffer_size_changed = self.working_prefs.audio_buffer_size != self.original_buffer_size;

        // Apply changes to config
        config.bpm = self.working_prefs.bpm;
        config.framerate = self.working_prefs.framerate;
        config.file_width = self.working_prefs.file_width;
        config.file_height = self.working_prefs.file_height;
        config.scroll_speed = self.working_prefs.scroll_speed;
        config.audio_buffer_size = self.working_prefs.audio_buffer_size;
        config.reopen_last_session = self.working_prefs.reopen_last_session;
        config.restore_layout_from_file = self.working_prefs.restore_layout_from_file;
        config.debug = self.working_prefs.debug;
        config.theme_mode = self.working_prefs.theme_mode.to_string_lower();

        // Apply theme immediately
        theme.set_mode(self.working_prefs.theme_mode);

        // Save to disk
        config.save();

        // Close dialog
        self.close();

        Some(PreferencesSaveResult {
            buffer_size_changed,
        })
    }
}
