//! Preferences dialog UI
//!
//! Provides a user interface for configuring application preferences,
//! including a Keyboard Shortcuts tab with click-to-rebind support.

use std::collections::HashMap;
use eframe::egui;
use crate::config::{AppConfig, TabletButtonAction};
use crate::keymap::{self, AppAction, KeymapManager};
use crate::menu::{MenuSystem, Shortcut, ShortcutKey};
use crate::theme::{Theme, ThemeMode};
use lightningbeam_core::file_io::LargeMediaMode;

/// Which tab is selected in the preferences dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreferencesTab {
    General,
    Shortcuts,
}

/// Preferences dialog state
pub struct PreferencesDialog {
    /// Is the dialog open?
    pub open: bool,

    /// Currently selected tab
    tab: PreferencesTab,

    /// Working copy of preferences (allows cancel to discard changes)
    working_prefs: PreferencesState,

    /// Original audio buffer size (to detect changes that need restart)
    original_buffer_size: u32,

    /// Error message (if validation fails)
    error_message: Option<String>,

    // --- Shortcuts tab state ---
    /// Working copy of keybindings (for live editing before save)
    working_keybindings: HashMap<AppAction, Option<Shortcut>>,

    /// Which action is currently being rebound (waiting for key press)
    rebinding: Option<AppAction>,

    /// Search/filter text for shortcuts list
    shortcut_filter: String,
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
    waveform_stereo: bool,
    theme_mode: ThemeMode,
    large_media_default: LargeMediaMode,
    tablet_button_lower: TabletButtonAction,
    tablet_button_upper: TabletButtonAction,
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
            waveform_stereo: config.waveform_stereo,
            theme_mode: theme.mode(),
            large_media_default: config.large_media_default,
            tablet_button_lower: config.tablet_button_lower,
            tablet_button_upper: config.tablet_button_upper,
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
            waveform_stereo: false,
            theme_mode: ThemeMode::System,
            large_media_default: LargeMediaMode::default(),
            tablet_button_lower: TabletButtonAction::Pan,
            tablet_button_upper: TabletButtonAction::Eyedropper,
        }
    }
}

/// Result returned when preferences are saved
pub struct PreferencesSaveResult {
    /// Whether audio buffer size changed (requires restart)
    pub buffer_size_changed: bool,
    /// New keymap manager if keybindings changed (caller must replace their keymap and call apply_keybindings)
    pub new_keymap: Option<KeymapManager>,
}

impl Default for PreferencesDialog {
    fn default() -> Self {
        Self {
            open: false,
            tab: PreferencesTab::General,
            working_prefs: PreferencesState::default(),
            original_buffer_size: 256,
            error_message: None,
            working_keybindings: HashMap::new(),
            rebinding: None,
            shortcut_filter: String::new(),
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
        self.working_keybindings = config.keybindings.effective_bindings();
        self.rebinding = None;
        self.shortcut_filter.clear();
    }

    /// Close the dialog
    pub fn close(&mut self) {
        self.open = false;
        self.error_message = None;
        self.rebinding = None;
    }

    /// Render the preferences dialog
    ///
    /// Returns Some(PreferencesSaveResult) if user clicked Save, None otherwise.
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        config: &mut AppConfig,
        theme: &mut Theme,
        mobile: bool,
    ) -> Option<PreferencesSaveResult> {
        if !self.open {
            return None;
        }

        let mut should_save = false;
        let mut should_cancel = false;
        let mut open = self.open;

        // On mobile, render as a screen-fitting modal sheet (dim backdrop, centered) like the other
        // mobile modals; on desktop, the familiar draggable window.
        let width = crate::mobile::dialog_width(ctx, 550.0);
        let scroll_h = if mobile {
            (ctx.content_rect().height() - 220.0).clamp(160.0, 400.0)
        } else {
            400.0
        };

        if mobile {
            let resp = egui::Modal::new(egui::Id::new("preferences_modal")).show(ctx, |ui| {
                self.render_body(ui, width, scroll_h, &mut should_save, &mut should_cancel);
            });
            if resp.backdrop_response.clicked() {
                should_cancel = true;
            }
        } else {
            egui::Window::new("Preferences")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    self.render_body(ui, width, scroll_h, &mut should_save, &mut should_cancel);
                });
        }

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

    #[allow(clippy::too_many_arguments)]
    fn render_body(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        scroll_h: f32,
        should_save: &mut bool,
        should_cancel: &mut bool,
    ) {
        ui.set_width(width);

        // Error message
        if let Some(error) = &self.error_message {
            ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error);
            ui.add_space(8.0);
        }

        // Tab bar
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, PreferencesTab::General, "General");
            ui.selectable_value(&mut self.tab, PreferencesTab::Shortcuts, "Keyboard Shortcuts");
        });
        ui.separator();

        // Tab content
        match self.tab {
            PreferencesTab::General => {
                egui::ScrollArea::vertical().max_height(scroll_h).show(ui, |ui| {
                    self.render_general_section(ui);
                    ui.add_space(8.0);
                    self.render_audio_section(ui);
                    ui.add_space(8.0);
                    self.render_appearance_section(ui);
                    ui.add_space(8.0);
                    self.render_startup_section(ui);
                    ui.add_space(8.0);
                    self.render_tablet_section(ui);
                    ui.add_space(8.0);
                    self.render_advanced_section(ui);
                });
            }
            PreferencesTab::Shortcuts => {
                self.render_shortcuts_tab(ui);
            }
        }

        ui.add_space(16.0);

        // Buttons
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                *should_cancel = true;
            }

            if ui.button("Reset to Defaults").clicked() {
                self.reset_to_defaults();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save").clicked() {
                    *should_save = true;
                }
            });
        });
    }

    fn render_shortcuts_tab(&mut self, ui: &mut egui::Ui) {
        // Capture key events for rebinding BEFORE rendering the rest
        if let Some(rebind_action) = self.rebinding {
            // Intercept key presses for rebinding
            let captured = ui.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                        // Escape clears the binding
                        if *key == egui::Key::Escape && !modifiers.ctrl && !modifiers.shift && !modifiers.alt {
                            return Some(None); // Clear binding
                        }
                        // Any other key: set as new binding
                        if let Some(shortcut_key) = ShortcutKey::from_egui_key(*key) {
                            return Some(Some(Shortcut::new(
                                shortcut_key,
                                modifiers.ctrl || modifiers.command,
                                modifiers.shift,
                                modifiers.alt,
                            )));
                        }
                    }
                }
                None
            });

            if let Some(new_binding) = captured {
                self.working_keybindings.insert(rebind_action, new_binding);
                self.rebinding = None;
            }
        }

        // Search/filter
        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut self.shortcut_filter);
        });
        ui.add_space(4.0);

        // Conflict detection
        let conflicts = self.detect_conflicts();
        if !conflicts.is_empty() {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 180, 50),
                    format!("{} conflict(s) detected", conflicts.len()));
            });
            ui.add_space(4.0);
        }

        // Scrollable list of actions grouped by category
        egui::ScrollArea::vertical()
            .max_height(350.0)
            .show(ui, |ui| {
                let filter_lower = self.shortcut_filter.to_lowercase();

                // Collect categories in display order
                let category_order = [
                    "File", "Edit", "Modify", "Layer", "Timeline", "View",
                    "Help", "Window", "Tools", "Global", "Pane",
                ];

                for category in &category_order {
                    let actions_in_category: Vec<AppAction> = AppAction::all().iter()
                        .filter(|a| a.category() == *category)
                        .filter(|a| {
                            if filter_lower.is_empty() {
                                true
                            } else {
                                a.display_name().to_lowercase().contains(&filter_lower)
                                    || a.category().to_lowercase().contains(&filter_lower)
                            }
                        })
                        .copied()
                        .collect();

                    if actions_in_category.is_empty() {
                        continue;
                    }

                    egui::CollapsingHeader::new(*category)
                        .default_open(!filter_lower.is_empty() || *category == "Tools" || *category == "Global")
                        .show(ui, |ui| {
                            for action in &actions_in_category {
                                self.render_shortcut_row(ui, *action, &conflicts);
                            }
                        });
                }
            });
    }

    fn render_shortcut_row(
        &mut self,
        ui: &mut egui::Ui,
        action: AppAction,
        conflicts: &HashMap<Shortcut, Vec<AppAction>>,
    ) {
        let binding = self.working_keybindings.get(&action).copied().flatten();
        let is_rebinding = self.rebinding == Some(action);
        let has_conflict = binding
            .as_ref()
            .and_then(|s| conflicts.get(s))
            .map(|actions| actions.len() > 1)
            .unwrap_or(false);

        ui.horizontal(|ui| {
            // Action name (fixed width)
            ui.add_sized([200.0, 20.0], egui::Label::new(action.display_name()));

            // Binding button (click to rebind)
            let button_text = if is_rebinding {
                "Press a key...".to_string()
            } else if let Some(s) = &binding {
                MenuSystem::format_shortcut(s)
            } else {
                "None".to_string()
            };

            let button_color = if is_rebinding {
                egui::Color32::from_rgb(100, 150, 255)
            } else if has_conflict {
                egui::Color32::from_rgb(255, 180, 50)
            } else {
                ui.visuals().widgets.inactive.text_color()
            };

            let response = ui.add_sized(
                [140.0, 20.0],
                egui::Button::new(egui::RichText::new(&button_text).color(button_color)),
            );

            if response.clicked() && !is_rebinding {
                self.rebinding = Some(action);
            }

            // Show conflict tooltip
            if has_conflict {
                if let Some(s) = &binding {
                    if let Some(conflicting) = conflicts.get(s) {
                        let others: Vec<&str> = conflicting.iter()
                            .filter(|a| **a != action)
                            .map(|a| a.display_name())
                            .collect();
                        response.on_hover_text(format!("Conflicts with: {}", others.join(", ")));
                    }
                }
            }

            // Clear button
            if ui.small_button("x").clicked() {
                self.working_keybindings.insert(action, None);
                if self.rebinding == Some(action) {
                    self.rebinding = None;
                }
            }
        });
    }

    /// Detect all shortcut conflicts (shortcut -> list of actions sharing it).
    /// Only actions within the same conflict scope can conflict — pane-local actions
    /// are isolated to their pane and never conflict with each other or global actions.
    fn detect_conflicts(&self) -> HashMap<Shortcut, Vec<AppAction>> {
        // Group by (shortcut, conflict_scope)
        let mut by_scope: HashMap<(&str, Shortcut), Vec<AppAction>> = HashMap::new();
        for (&action, &shortcut) in &self.working_keybindings {
            if let Some(s) = shortcut {
                by_scope.entry((action.conflict_scope(), s)).or_default().push(action);
            }
        }

        // Flatten into shortcut -> conflicting actions (only where there are actual conflicts)
        let mut result: HashMap<Shortcut, Vec<AppAction>> = HashMap::new();
        for ((_, shortcut), actions) in by_scope {
            if actions.len() > 1 {
                result.entry(shortcut).or_default().extend(actions);
            }
        }
        result
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

                ui.label("Requires app restart to take effect");
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

    fn render_tablet_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Tablet")
            .default_open(false)
            .show(ui, |ui| {
                ui.label("What each barrel button on the stylus does while held:");
                ui.add_space(4.0);

                let button_row = |ui: &mut egui::Ui, label: &str, id: &str, value: &mut TabletButtonAction| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        egui::ComboBox::from_id_salt(id)
                            .selected_text(value.label())
                            .show_ui(ui, |ui| {
                                for action in TabletButtonAction::ALL {
                                    ui.selectable_value(value, action, action.label());
                                }
                            });
                    });
                };

                button_row(
                    ui,
                    "Lower button:",
                    "tablet_button_lower",
                    &mut self.working_prefs.tablet_button_lower,
                );
                button_row(
                    ui,
                    "Upper button:",
                    "tablet_button_upper",
                    &mut self.working_prefs.tablet_button_upper,
                );
            });
    }

    fn render_advanced_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Advanced")
            .default_open(false)
            .show(ui, |ui| {
                ui.checkbox(&mut self.working_prefs.debug, "Enable debug mode");
                ui.checkbox(
                    &mut self.working_prefs.waveform_stereo,
                    "Show waveforms as stacked stereo",
                );
                ui.horizontal(|ui| {
                    let threshold_gb = lightningbeam_core::beam_archive::LARGE_MEDIA_THRESHOLD
                        as f64
                        / (1024.0 * 1024.0 * 1024.0);
                    ui.label(format!("Large media (>{:.0} GB):", threshold_gb));
                    let label = |m: LargeMediaMode| match m {
                        LargeMediaMode::Ask => "Ask each time",
                        LargeMediaMode::Pack => "Pack into project",
                        LargeMediaMode::Reference => "Reference external file",
                    };
                    egui::ComboBox::from_id_salt("large_media_default")
                        .selected_text(label(self.working_prefs.large_media_default))
                        .show_ui(ui, |ui| {
                            for mode in [LargeMediaMode::Ask, LargeMediaMode::Pack, LargeMediaMode::Reference] {
                                ui.selectable_value(&mut self.working_prefs.large_media_default, mode, label(mode));
                            }
                        });
                });
            });
    }

    fn reset_to_defaults(&mut self) {
        self.working_prefs = PreferencesState::default();
        self.working_keybindings = keymap::all_defaults();
        self.rebinding = None;
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
        temp_config.waveform_stereo = self.working_prefs.waveform_stereo;
        temp_config.theme_mode = self.working_prefs.theme_mode.to_string_lower();
        temp_config.tablet_button_lower = self.working_prefs.tablet_button_lower;
        temp_config.tablet_button_upper = self.working_prefs.tablet_button_upper;

        // Validate
        if let Err(err) = temp_config.validate() {
            self.error_message = Some(err);
            return None;
        }

        // Check if buffer size changed
        let buffer_size_changed = self.working_prefs.audio_buffer_size != self.original_buffer_size;

        // Build new keymap from working keybindings to compute sparse overrides
        let defaults = keymap::all_defaults();
        let mut overrides = HashMap::new();
        for (&action, &shortcut) in &self.working_keybindings {
            let default = defaults.get(&action).copied().flatten();
            if shortcut != default {
                overrides.insert(action, shortcut);
            }
        }
        let keybinding_config = keymap::KeybindingConfig { overrides };
        let new_keymap = KeymapManager::new(&keybinding_config);

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
        config.waveform_stereo = self.working_prefs.waveform_stereo;
        config.theme_mode = self.working_prefs.theme_mode.to_string_lower();
        config.large_media_default = self.working_prefs.large_media_default;
        config.tablet_button_lower = self.working_prefs.tablet_button_lower;
        config.tablet_button_upper = self.working_prefs.tablet_button_upper;
        config.keybindings = keybinding_config;

        // Apply theme immediately
        theme.set_mode(self.working_prefs.theme_mode);
        crate::tablet::set_button_actions(
            self.working_prefs.tablet_button_lower,
            self.working_prefs.tablet_button_upper,
        );

        // Save to disk
        config.save();

        // Close dialog
        self.close();

        Some(PreferencesSaveResult {
            buffer_size_changed,
            new_keymap: Some(new_keymap),
        })
    }
}
