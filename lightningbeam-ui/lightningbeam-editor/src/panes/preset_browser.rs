/// Instrument Browser pane — browse, search, load, and save instrument presets
///
/// Scans factory presets from `src/assets/instruments/` organized by category.
/// Presets are loaded into the currently selected track's audio graph.

use eframe::egui;
use std::path::PathBuf;
use super::{NodePath, PaneRenderer, SharedPaneState};

/// Metadata extracted from a preset file
struct PresetInfo {
    name: String,
    path: PathBuf,
    category: String,
    description: String,
    author: String,
    tags: Vec<String>,
    is_factory: bool,
}

/// State for the save-preset dialog
struct SaveDialogState {
    name: String,
    description: String,
    tags_str: String,
}

impl Default for SaveDialogState {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            tags_str: String::new(),
        }
    }
}

pub struct PresetBrowserPane {
    presets: Vec<PresetInfo>,
    search_query: String,
    /// Index into `self.presets` of the currently selected preset
    selected_index: Option<usize>,
    selected_category: Option<String>,
    needs_reload: bool,
    save_dialog: Option<SaveDialogState>,
    /// Sorted unique category names extracted from presets
    categories: Vec<String>,
}

impl PresetBrowserPane {
    pub fn new() -> Self {
        Self {
            presets: Vec::new(),
            search_query: String::new(),
            selected_index: None,
            selected_category: None,
            needs_reload: true,
            save_dialog: None,
            categories: Vec::new(),
        }
    }

    /// Scan preset directories and populate the preset list
    fn scan_presets(&mut self) {
        self.presets.clear();
        self.categories.clear();

        // Factory presets: check installed location first, fall back to dev source tree
        let factory_dirs = [
            // Installed location (Linux packages / AppImage)
            PathBuf::from("/usr/share/lightningbeam-editor/presets"),
            // Next to the binary (AppImage / portable)
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("presets")))
                .unwrap_or_default(),
            // Development: relative to CARGO_MANIFEST_DIR
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/assets/instruments"),
        ];

        for dir in &factory_dirs {
            if let Ok(factory_dir) = dir.canonicalize() {
                if factory_dir.is_dir() {
                    self.scan_directory(&factory_dir, &factory_dir, true);
                    break;
                }
            }
        }

        // User presets
        let user_dir = user_presets_dir();
        if user_dir.is_dir() {
            self.scan_directory(&user_dir, &user_dir, false);
        }

        // Sort presets alphabetically by name within each category
        self.presets.sort_by(|a, b| {
            a.category.cmp(&b.category).then(a.name.cmp(&b.name))
        });

        // Extract unique categories
        let mut cats: Vec<String> = self.presets.iter()
            .map(|p| p.category.clone())
            .collect();
        cats.sort();
        cats.dedup();
        self.categories = cats;

        self.needs_reload = false;
    }

    /// Recursively scan a directory for .json preset files
    fn scan_directory(&mut self, dir: &std::path::Path, base_dir: &std::path::Path, is_factory: bool) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_directory(&path, base_dir, is_factory);
            } else if path.extension().is_some_and(|e| e == "json") {
                if let Some(info) = self.load_preset_info(&path, base_dir, is_factory) {
                    self.presets.push(info);
                }
            }
        }
    }

    /// Load metadata from a preset JSON file
    fn load_preset_info(&self, path: &std::path::Path, base_dir: &std::path::Path, is_factory: bool) -> Option<PresetInfo> {
        let contents = std::fs::read_to_string(path).ok()?;
        let preset: daw_backend::audio::node_graph::GraphPreset =
            serde_json::from_str(&contents).ok()?;

        // Category = first directory component relative to base_dir
        let relative = path.strip_prefix(base_dir).ok()?;
        let category = relative.components().next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("other")
            .to_string();

        Some(PresetInfo {
            name: preset.metadata.name,
            path: path.to_path_buf(),
            category,
            description: preset.metadata.description,
            author: preset.metadata.author,
            tags: preset.metadata.tags,
            is_factory,
        })
    }

    /// Get indices of presets matching the current search query and category filter
    fn filtered_indices(&self) -> Vec<usize> {
        let query = self.search_query.to_lowercase();
        self.presets.iter().enumerate()
            .filter(|(_, p)| {
                // Category filter
                if let Some(ref cat) = self.selected_category {
                    if &p.category != cat {
                        return false;
                    }
                }
                // Search filter
                if !query.is_empty() {
                    let name_match = p.name.to_lowercase().contains(&query);
                    let desc_match = p.description.to_lowercase().contains(&query);
                    let tag_match = p.tags.iter().any(|t| t.to_lowercase().contains(&query));
                    if !name_match && !desc_match && !tag_match {
                        return false;
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Load the selected preset into the current track
    fn load_preset(&self, preset_index: usize, shared: &mut SharedPaneState) {
        let preset = &self.presets[preset_index];

        let track_id = match shared.active_layer_id.and_then(|lid| shared.layer_to_track_map.get(&lid)) {
            Some(&tid) => tid,
            None => return,
        };

        if let Some(audio_controller) = &shared.audio_controller {
            let mut controller = audio_controller.lock().unwrap();
            controller.graph_load_preset(track_id, preset.path.to_string_lossy().to_string());
        }
        // Note: project_generation is incremented by the GraphPresetLoaded event handler
        // in main.rs, which fires after the audio thread has actually processed the load.
        // This avoids a race where the node graph queries stale backend state.
    }

    /// Render the save preset dialog
    fn render_save_dialog(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) {
        let dialog = match &mut self.save_dialog {
            Some(d) => d,
            None => return,
        };

        ui.add_space(8.0);
        ui.heading("Save Preset");
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.text_edit_singleline(&mut dialog.name);
        });

        ui.add_space(4.0);
        ui.label("Description:");
        ui.add(egui::TextEdit::multiline(&mut dialog.description)
            .desired_rows(3)
            .desired_width(f32::INFINITY));

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Tags:");
            ui.text_edit_singleline(&mut dialog.tags_str);
        });
        ui.label(egui::RichText::new("Comma-separated, e.g. bass, synth, warm")
            .small()
            .color(ui.visuals().weak_text_color()));

        ui.add_space(8.0);
        let name_valid = !dialog.name.trim().is_empty();
        let mut do_save = false;
        let mut do_cancel = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(name_valid, egui::Button::new("Save")).clicked() {
                do_save = true;
            }
            if ui.button("Cancel").clicked() {
                do_cancel = true;
            }
        });

        // Act after dialog borrow is released
        if do_save {
            self.do_save_preset(shared);
        } else if do_cancel {
            self.save_dialog = None;
        }
    }

    /// Execute the save action
    fn do_save_preset(&mut self, shared: &mut SharedPaneState) {
        let dialog = match self.save_dialog.take() {
            Some(d) => d,
            None => return,
        };

        let track_id = match shared.active_layer_id.and_then(|lid| shared.layer_to_track_map.get(&lid)) {
            Some(&tid) => tid,
            None => return,
        };

        let name = dialog.name.trim().to_string();
        let description = dialog.description.trim().to_string();
        let tags: Vec<String> = dialog.tags_str.split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();

        // Save to user presets directory
        let save_dir = user_presets_dir();
        if let Err(e) = std::fs::create_dir_all(&save_dir) {
            eprintln!("Failed to create presets directory: {}", e);
            return;
        }

        let filename = sanitize_filename(&name);
        let save_path = save_dir.join(format!("{}.json", filename));

        if let Some(audio_controller) = &shared.audio_controller {
            let mut controller = audio_controller.lock().unwrap();
            controller.graph_save_preset(
                track_id,
                save_path.to_string_lossy().to_string(),
                name,
                description,
                tags,
            );
        }

        self.needs_reload = true;
    }
}

/// Get the user presets directory ($XDG_DATA_HOME/lightningbeam/presets or ~/.local/share/lightningbeam/presets)
fn user_presets_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg).join("lightningbeam").join("presets")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/lightningbeam/presets")
    } else {
        PathBuf::from("presets")
    }
}

/// Sanitize a string for use as a filename
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect::<String>()
        .trim()
        .to_string()
}

impl PaneRenderer for PresetBrowserPane {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let has_track = shared.active_layer_id
                .and_then(|lid| shared.layer_to_track_map.get(&lid))
                .is_some();
            if ui.add_enabled(has_track, egui::Button::new("Save")).clicked() {
                self.save_dialog = Some(SaveDialogState::default());
            }
        });
        true
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        if self.needs_reload {
            self.scan_presets();
        }

        // Background
        let bg_style = shared.theme.style(".pane-content", ui.ctx());
        let bg_color = bg_style.background_color().unwrap_or(egui::Color32::from_rgb(47, 47, 47));
        ui.painter().rect_filled(rect, 0.0, bg_color);

        let text_color = shared.theme.style(".text-primary", ui.ctx())
            .text_color.unwrap_or(egui::Color32::from_gray(246));
        let text_secondary = shared.theme.style(".text-secondary", ui.ctx())
            .text_color.unwrap_or(egui::Color32::from_gray(170));

        let content_rect = rect.shrink(4.0);
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        let ui = &mut content_ui;

        // Save dialog takes over the content area
        if self.save_dialog.is_some() {
            self.render_save_dialog(ui, shared);
            return;
        }

        // Search bar
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.search_query);
        });

        ui.add_space(4.0);

        // Category chips
        ui.horizontal_wrapped(|ui| {
            let all_selected = self.selected_category.is_none();
            if ui.selectable_label(all_selected, "All").clicked() {
                self.selected_category = None;
                self.selected_index = None;
            }
            for cat in &self.categories.clone() {
                let is_selected = self.selected_category.as_ref() == Some(cat);
                let display = capitalize_first(cat);
                if ui.selectable_label(is_selected, &display).clicked() {
                    if is_selected {
                        self.selected_category = None;
                    } else {
                        self.selected_category = Some(cat.clone());
                    }
                    self.selected_index = None;
                }
            }
        });

        ui.separator();

        // Preset list
        let filtered = self.filtered_indices();

        if filtered.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(egui::RichText::new("No presets found")
                    .color(text_secondary));
            });
            return;
        }

        let mut load_index = None;
        let mut delete_path = None;

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let mut new_selection = self.selected_index;

            for &idx in &filtered {
                let preset = &self.presets[idx];
                let is_selected = self.selected_index == Some(idx);

                let response = ui.push_id(idx, |ui| {
                    let frame = egui::Frame::NONE
                        .inner_margin(egui::Margin::same(6))
                        .corner_radius(4.0);

                    let mut button_clicked = false;

                    let frame_response = frame.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());

                        ui.label(
                            egui::RichText::new(&preset.name).strong().color(text_color)
                        );

                        if is_selected {
                            if !preset.description.is_empty() {
                                ui.label(egui::RichText::new(&preset.description)
                                    .color(text_secondary)
                                    .small());
                            }

                            if !preset.tags.is_empty() {
                                ui.horizontal_wrapped(|ui| {
                                    for tag in &preset.tags {
                                        let tag_frame = egui::Frame::NONE
                                            .inner_margin(egui::Margin::symmetric(6, 2))
                                            .corner_radius(8.0)
                                            .fill(ui.visuals().selection.bg_fill.linear_multiply(0.3));
                                        tag_frame.show(ui, |ui| {
                                            ui.label(egui::RichText::new(tag).small().color(text_color));
                                        });
                                    }
                                });
                            }

                            ui.horizontal(|ui| {
                                if !preset.author.is_empty() {
                                    ui.label(egui::RichText::new(format!("by {}", preset.author))
                                        .small()
                                        .color(text_secondary));
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if !preset.is_factory {
                                        if ui.small_button("Delete").clicked() {
                                            delete_path = Some(preset.path.clone());
                                            button_clicked = true;
                                        }
                                    }

                                    let has_track = shared.active_layer_id
                                        .and_then(|lid| shared.layer_to_track_map.get(&lid))
                                        .is_some();
                                    if ui.add_enabled(has_track, egui::Button::new("Load")).clicked() {
                                        load_index = Some(idx);
                                        button_clicked = true;
                                    }
                                });
                            });
                        }
                    });

                    // Hover highlight and click-to-select (no ui.interact overlay)
                    let frame_rect = frame_response.response.rect;
                    let is_hovered = ui.rect_contains_pointer(frame_rect);

                    let fill = if is_selected {
                        ui.visuals().selection.bg_fill.linear_multiply(0.3)
                    } else if is_hovered {
                        ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.3)
                    } else {
                        egui::Color32::TRANSPARENT
                    };
                    if fill != egui::Color32::TRANSPARENT {
                        ui.painter().rect_filled(frame_rect, 4.0, fill);
                    }

                    if is_hovered {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if is_hovered && !button_clicked && ui.input(|i| i.pointer.any_released()) {
                        new_selection = if is_selected { None } else { Some(idx) };
                    }
                });

                let rect = response.response.rect;
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    egui::Stroke::new(0.5, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
            }

            self.selected_index = new_selection;
        });

        // Deferred actions after ScrollArea borrow is released
        if let Some(idx) = load_index {
            self.load_preset(idx, shared);
            // Signal that we're expecting a GraphPresetLoaded event so the
            // repaint loop stays alive until the audio thread responds.
            shared.pending_graph_loads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(path) = delete_path {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("Failed to delete preset: {e}");
            }
            self.needs_reload = true;
        }
    }

    fn name(&self) -> &str {
        "Instrument Browser"
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}
