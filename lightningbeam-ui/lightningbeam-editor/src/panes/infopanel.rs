/// Info Panel pane - displays and edits properties of selected objects
///
/// Shows context-sensitive property editors based on current focus:
/// - Tool options (when a tool is active)
/// - Layer properties (when layers are focused)
/// - Clip instance properties (when clip instances are focused)
/// - Shape properties (fill/stroke for selected geometry)
/// - Note info (when piano roll notes are focused)
/// - Node info (when node graph nodes are focused)
/// - Asset info (when asset library items are focused)
/// - Document settings (when nothing is focused)

use eframe::egui::{self, DragValue, Ui};
use lightningbeam_core::actions::{SetDocumentPropertiesAction, SetShapePropertiesAction};
use lightningbeam_core::layer::{AnyLayer, LayerTrait};
use lightningbeam_core::selection::FocusSelection;
use lightningbeam_core::shape::ShapeColor;
use lightningbeam_core::tool::{SimplifyMode, Tool};
use super::{NodePath, PaneRenderer, SharedPaneState};
use uuid::Uuid;

/// Info panel pane state
pub struct InfopanelPane {
    /// Whether the tool options section is expanded
    tool_section_open: bool,
    /// Whether the shape properties section is expanded
    shape_section_open: bool,
}

impl InfopanelPane {
    pub fn new() -> Self {
        Self {
            tool_section_open: true,
            shape_section_open: true,
        }
    }
}

/// Aggregated info about the current DCEL selection
struct SelectionInfo {
    /// True if nothing is selected
    is_empty: bool,
    /// Number of selected DCEL elements (edges + faces)
    dcel_count: usize,
    /// Layer ID of selected elements (assumes single layer selection for now)
    layer_id: Option<Uuid>,

    // Shape property values (None = mixed)
    fill_color: Option<Option<ShapeColor>>,
    stroke_color: Option<Option<ShapeColor>>,
    stroke_width: Option<f64>,
}

impl Default for SelectionInfo {
    fn default() -> Self {
        Self {
            is_empty: true,
            dcel_count: 0,
            layer_id: None,
            fill_color: None,
            stroke_color: None,
            stroke_width: None,
        }
    }
}

impl InfopanelPane {
    /// Gather info about the current DCEL selection
    fn gather_selection_info(&self, shared: &SharedPaneState) -> SelectionInfo {
        let mut info = SelectionInfo::default();

        let edge_count = shared.selection.selected_edges().len();
        let face_count = shared.selection.selected_faces().len();
        info.dcel_count = edge_count + face_count;
        info.is_empty = info.dcel_count == 0;

        if info.is_empty {
            return info;
        }

        let document = shared.action_executor.document();
        let active_layer_id = *shared.active_layer_id;

        if let Some(layer_id) = active_layer_id {
            info.layer_id = Some(layer_id);

            if let Some(layer) = document.get_layer(&layer_id) {
                if let AnyLayer::Vector(vector_layer) = layer {
                    if let Some(dcel) = vector_layer.dcel_at_time(*shared.playback_time) {
                        // Gather stroke properties from selected edges
                        let mut first_stroke_color: Option<Option<ShapeColor>> = None;
                        let mut first_stroke_width: Option<f64> = None;
                        let mut stroke_color_mixed = false;
                        let mut stroke_width_mixed = false;

                        for &eid in shared.selection.selected_edges() {
                            let edge = dcel.edge(eid);
                            let sc = edge.stroke_color;
                            let sw = edge.stroke_style.as_ref().map(|s| s.width);

                            match first_stroke_color {
                                None => first_stroke_color = Some(sc),
                                Some(prev) if prev != sc => stroke_color_mixed = true,
                                _ => {}
                            }
                            match (first_stroke_width, sw) {
                                (None, _) => first_stroke_width = sw,
                                (Some(prev), Some(cur)) if (prev - cur).abs() > 0.01 => stroke_width_mixed = true,
                                _ => {}
                            }
                        }

                        if !stroke_color_mixed {
                            info.stroke_color = first_stroke_color;
                        }
                        if !stroke_width_mixed {
                            info.stroke_width = first_stroke_width;
                        }

                        // Gather fill properties from selected faces
                        let mut first_fill_color: Option<Option<ShapeColor>> = None;
                        let mut fill_color_mixed = false;

                        for &fid in shared.selection.selected_faces() {
                            let face = dcel.face(fid);
                            let fc = face.fill_color;

                            match first_fill_color {
                                None => first_fill_color = Some(fc),
                                Some(prev) if prev != fc => fill_color_mixed = true,
                                _ => {}
                            }
                        }

                        if !fill_color_mixed {
                            info.fill_color = first_fill_color;
                        }
                    }
                }
            }
        }

        info
    }

    /// Render tool-specific options section
    fn render_tool_section(&mut self, ui: &mut Ui, path: &NodePath, shared: &mut SharedPaneState) {
        let tool = *shared.selected_tool;

        // Only show tool options for tools that have options
        let is_vector_tool = matches!(
            tool,
            Tool::Select | Tool::BezierEdit | Tool::Draw | Tool::Rectangle
            | Tool::Ellipse | Tool::Line | Tool::Polygon
        );
        let has_options = is_vector_tool || matches!(
            tool,
            Tool::PaintBucket | Tool::RegionSelect
        );

        if !has_options {
            return;
        }

        egui::CollapsingHeader::new("Tool Options")
            .id_salt(("tool_options", path))
            .default_open(self.tool_section_open)
            .show(ui, |ui| {
                self.tool_section_open = true;
                ui.add_space(4.0);

                if is_vector_tool {
                    ui.checkbox(shared.snap_enabled, "Snap to Geometry");
                    ui.add_space(2.0);
                }

                match tool {
                    Tool::Draw => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Simplify mode
                        ui.horizontal(|ui| {
                            ui.label("Simplify:");
                            egui::ComboBox::from_id_salt(("draw_simplify", path))
                                .selected_text(match shared.draw_simplify_mode {
                                    SimplifyMode::Corners => "Corners",
                                    SimplifyMode::Smooth => "Smooth",
                                    SimplifyMode::Verbatim => "Verbatim",
                                })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Corners,
                                        "Corners",
                                    );
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Smooth,
                                        "Smooth",
                                    );
                                    ui.selectable_value(
                                        shared.draw_simplify_mode,
                                        SimplifyMode::Verbatim,
                                        "Verbatim",
                                    );
                                });
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::Rectangle | Tool::Ellipse => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::PaintBucket => {
                        // Gap tolerance
                        ui.horizontal(|ui| {
                            ui.label("Gap Tolerance:");
                            ui.add(
                                DragValue::new(shared.paint_bucket_gap_tolerance)
                                    .speed(0.1)
                                    .range(0.0..=50.0),
                            );
                        });
                    }

                    Tool::Polygon => {
                        // Number of sides
                        ui.horizontal(|ui| {
                            ui.label("Sides:");
                            let mut sides = *shared.polygon_sides as i32;
                            if ui.add(DragValue::new(&mut sides).range(3..=20)).changed() {
                                *shared.polygon_sides = sides.max(3) as u32;
                            }
                        });

                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });

                        // Fill shape toggle
                        ui.checkbox(shared.fill_enabled, "Fill Shape");
                    }

                    Tool::Line => {
                        // Stroke width
                        ui.horizontal(|ui| {
                            ui.label("Stroke Width:");
                            ui.add(DragValue::new(shared.stroke_width).speed(0.1).range(0.1..=100.0));
                        });
                    }

                    Tool::RegionSelect => {
                        use lightningbeam_core::tool::RegionSelectMode;
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            if ui.selectable_label(
                                *shared.region_select_mode == RegionSelectMode::Rectangle,
                                "Rectangle",
                            ).clicked() {
                                *shared.region_select_mode = RegionSelectMode::Rectangle;
                            }
                            if ui.selectable_label(
                                *shared.region_select_mode == RegionSelectMode::Lasso,
                                "Lasso",
                            ).clicked() {
                                *shared.region_select_mode = RegionSelectMode::Lasso;
                            }
                        });
                    }

                    _ => {}
                }

                ui.add_space(4.0);
            });
    }

    // Transform section: deferred to Phase 2 (DCEL elements don't have instance transforms)

    /// Render shape properties section (fill/stroke)
    fn render_shape_section(
        &mut self,
        ui: &mut Ui,
        path: &NodePath,
        shared: &mut SharedPaneState,
        info: &SelectionInfo,
    ) {
        // Clone IDs and values we need before borrowing shared mutably
        let layer_id = match info.layer_id {
            Some(id) => id,
            None => return,
        };
        let time = *shared.playback_time;
        let face_ids: Vec<_> = shared.selection.selected_faces().iter().copied().collect();
        let edge_ids: Vec<_> = shared.selection.selected_edges().iter().copied().collect();

        egui::CollapsingHeader::new("Shape")
            .id_salt(("shape", path))
            .default_open(self.shape_section_open)
            .show(ui, |ui| {
                self.shape_section_open = true;
                ui.add_space(4.0);

                // Fill color
                ui.horizontal(|ui| {
                    ui.label("Fill:");
                    match info.fill_color {
                        Some(Some(color)) => {
                            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                                color.r, color.g, color.b, color.a,
                            );
                            if egui::color_picker::color_edit_button_srgba(
                                ui,
                                &mut egui_color,
                                egui::color_picker::Alpha::OnlyBlend,
                            ).changed() {
                                let new_color = ShapeColor {
                                    r: egui_color.r(),
                                    g: egui_color.g(),
                                    b: egui_color.b(),
                                    a: egui_color.a(),
                                };
                                let action = SetShapePropertiesAction::set_fill_color(
                                    layer_id, time, face_ids.clone(), Some(new_color),
                                );
                                shared.pending_actions.push(Box::new(action));
                            }
                        }
                        Some(None) => {
                            ui.label("None");
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                // Stroke color
                ui.horizontal(|ui| {
                    ui.label("Stroke:");
                    match info.stroke_color {
                        Some(Some(color)) => {
                            let mut egui_color = egui::Color32::from_rgba_unmultiplied(
                                color.r, color.g, color.b, color.a,
                            );
                            if egui::color_picker::color_edit_button_srgba(
                                ui,
                                &mut egui_color,
                                egui::color_picker::Alpha::OnlyBlend,
                            ).changed() {
                                let new_color = ShapeColor {
                                    r: egui_color.r(),
                                    g: egui_color.g(),
                                    b: egui_color.b(),
                                    a: egui_color.a(),
                                };
                                let action = SetShapePropertiesAction::set_stroke_color(
                                    layer_id, time, edge_ids.clone(), Some(new_color),
                                );
                                shared.pending_actions.push(Box::new(action));
                            }
                        }
                        Some(None) => {
                            ui.label("None");
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                // Stroke width
                ui.horizontal(|ui| {
                    ui.label("Stroke Width:");
                    match info.stroke_width {
                        Some(mut width) => {
                            if ui.add(
                                DragValue::new(&mut width)
                                    .speed(0.1)
                                    .range(0.1..=100.0),
                            ).changed() {
                                let action = SetShapePropertiesAction::set_stroke_width(
                                    layer_id, time, edge_ids.clone(), width,
                                );
                                shared.pending_actions.push(Box::new(action));
                            }
                        }
                        None => {
                            ui.label("--");
                        }
                    }
                });

                ui.add_space(4.0);
            });
    }

    /// Render document settings section (shown when nothing is focused)
    fn render_document_section(&self, ui: &mut Ui, path: &NodePath, shared: &mut SharedPaneState) {
        egui::CollapsingHeader::new("Document")
            .id_salt(("document", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                let document = shared.action_executor.document();

                // Get current values for editing
                let mut width = document.width;
                let mut height = document.height;
                let mut duration = document.duration;
                let mut framerate = document.framerate;
                let layer_count = document.root.children.len();

                // Canvas width
                ui.horizontal(|ui| {
                    ui.label("Width:");
                    if ui
                        .add(DragValue::new(&mut width).speed(1.0).range(1.0..=10000.0))
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_width(width);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Canvas height
                ui.horizontal(|ui| {
                    ui.label("Height:");
                    if ui
                        .add(DragValue::new(&mut height).speed(1.0).range(1.0..=10000.0))
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_height(height);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Duration
                ui.horizontal(|ui| {
                    ui.label("Duration:");
                    if ui
                        .add(
                            DragValue::new(&mut duration)
                                .speed(0.1)
                                .range(0.1..=3600.0)
                                .suffix("s"),
                        )
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_duration(duration);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Framerate
                ui.horizontal(|ui| {
                    ui.label("Framerate:");
                    if ui
                        .add(
                            DragValue::new(&mut framerate)
                                .speed(1.0)
                                .range(1.0..=120.0)
                                .suffix(" fps"),
                        )
                        .changed()
                    {
                        let action = SetDocumentPropertiesAction::set_framerate(framerate);
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Background color
                ui.horizontal(|ui| {
                    ui.label("Background:");
                    let bg = document.background_color;
                    let mut color = [bg.r, bg.g, bg.b];
                    if ui.color_edit_button_srgb(&mut color).changed() {
                        let action = SetDocumentPropertiesAction::set_background_color(
                            ShapeColor::rgb(color[0], color[1], color[2]),
                        );
                        shared.pending_actions.push(Box::new(action));
                    }
                });

                // Layer count (read-only)
                ui.horizontal(|ui| {
                    ui.label("Layers:");
                    ui.label(format!("{}", layer_count));
                });

                ui.add_space(4.0);
            });
    }

    /// Render layer info section
    fn render_layer_section(&self, ui: &mut Ui, path: &NodePath, shared: &SharedPaneState, layer_ids: &[Uuid]) {
        let document = shared.action_executor.document();

        egui::CollapsingHeader::new("Layer")
            .id_salt(("layer_info", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                if layer_ids.len() == 1 {
                    if let Some(layer) = document.get_layer(&layer_ids[0]) {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.label(layer.name());
                        });

                        let type_name = match layer {
                            AnyLayer::Vector(_) => "Vector",
                            AnyLayer::Audio(a) => match a.audio_layer_type {
                                lightningbeam_core::layer::AudioLayerType::Midi => "MIDI",
                                lightningbeam_core::layer::AudioLayerType::Sampled => "Audio",
                            },
                            AnyLayer::Video(_) => "Video",
                            AnyLayer::Effect(_) => "Effect",
                            AnyLayer::Group(_) => "Group",
                        };
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            ui.label(type_name);
                        });

                        ui.horizontal(|ui| {
                            ui.label("Opacity:");
                            ui.label(format!("{:.0}%", layer.opacity() * 100.0));
                        });

                        if matches!(layer, AnyLayer::Audio(_)) {
                            ui.horizontal(|ui| {
                                ui.label("Volume:");
                                ui.label(format!("{:.0}%", layer.volume() * 100.0));
                            });
                        }

                        if layer.muted() {
                            ui.label("Muted");
                        }
                        if layer.locked() {
                            ui.label("Locked");
                        }
                    }
                } else {
                    ui.label(format!("{} layers selected", layer_ids.len()));
                }

                ui.add_space(4.0);
            });
    }

    /// Render clip instance info section
    fn render_clip_instance_section(&self, ui: &mut Ui, path: &NodePath, shared: &SharedPaneState, clip_ids: &[Uuid]) {
        let document = shared.action_executor.document();

        egui::CollapsingHeader::new("Clip Instance")
            .id_salt(("clip_instance_info", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                if clip_ids.len() == 1 {
                    // Find the clip instance across all layers
                    let ci_id = clip_ids[0];
                    let mut found = false;

                    for layer in document.all_layers() {
                        let instances: &[lightningbeam_core::clip::ClipInstance] = match layer {
                            AnyLayer::Vector(l) => &l.clip_instances,
                            AnyLayer::Audio(l) => &l.clip_instances,
                            AnyLayer::Video(l) => &l.clip_instances,
                            AnyLayer::Effect(l) => &l.clip_instances,
                            AnyLayer::Group(_) => &[],
                        };
                        if let Some(ci) = instances.iter().find(|c| c.id == ci_id) {
                            found = true;

                            if let Some(name) = &ci.name {
                                ui.horizontal(|ui| {
                                    ui.label("Name:");
                                    ui.label(name.as_str());
                                });
                            }

                            // Show clip name based on type
                            let clip_name = document.get_vector_clip(&ci.clip_id).map(|c| c.name.as_str())
                                .or_else(|| document.get_video_clip(&ci.clip_id).map(|c| c.name.as_str()))
                                .or_else(|| document.get_audio_clip(&ci.clip_id).map(|c| c.name.as_str()));
                            if let Some(name) = clip_name {
                                ui.horizontal(|ui| {
                                    ui.label("Clip:");
                                    ui.label(name);
                                });
                            }

                            ui.horizontal(|ui| {
                                ui.label("Start:");
                                ui.label(format!("{:.2}s", ci.effective_start()));
                            });

                            let clip_dur = document.get_clip_duration(&ci.clip_id)
                                .unwrap_or_else(|| ci.trim_end.unwrap_or(1.0) - ci.trim_start);
                            let total_dur = ci.total_duration(clip_dur);
                            ui.horizontal(|ui| {
                                ui.label("Duration:");
                                ui.label(format!("{:.2}s", total_dur));
                            });

                            if ci.trim_start > 0.0 {
                                ui.horizontal(|ui| {
                                    ui.label("Trim Start:");
                                    ui.label(format!("{:.2}s", ci.trim_start));
                                });
                            }

                            if ci.playback_speed != 1.0 {
                                ui.horizontal(|ui| {
                                    ui.label("Speed:");
                                    ui.label(format!("{:.2}x", ci.playback_speed));
                                });
                            }

                            break;
                        }
                    }

                    if !found {
                        ui.label("Clip instance not found");
                    }
                } else {
                    ui.label(format!("{} clip instances selected", clip_ids.len()));
                }

                ui.add_space(4.0);
            });
    }

    /// Render MIDI note info section
    fn render_notes_section(
        &self,
        ui: &mut Ui,
        path: &NodePath,
        shared: &SharedPaneState,
        layer_id: Uuid,
        midi_clip_id: u32,
        indices: &[usize],
    ) {
        egui::CollapsingHeader::new("Notes")
            .id_salt(("notes_info", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                // Show layer name
                let document = shared.action_executor.document();
                if let Some(layer) = document.get_layer(&layer_id) {
                    ui.horizontal(|ui| {
                        ui.label("Layer:");
                        ui.label(layer.name());
                    });
                }

                if indices.len() == 1 {
                    // Single note — show details if we can resolve from the event cache
                    if let Some(events) = shared.midi_event_cache.get(&midi_clip_id) {
                        // Events are (time, note, velocity, is_on) — resolve to notes
                        let mut notes: Vec<(f64, u8, u8, f64)> = Vec::new(); // (time, note, vel, dur)
                        let mut pending: std::collections::HashMap<u8, (f64, u8)> = std::collections::HashMap::new();
                        for &(time, note, vel, is_on) in events {
                            if is_on {
                                pending.insert(note, (time, vel));
                            } else if let Some((start, v)) = pending.remove(&note) {
                                notes.push((start, note, v, time - start));
                            }
                        }
                        notes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

                        let idx = indices[0];
                        if idx < notes.len() {
                            let (time, note, vel, dur) = notes[idx];
                            let note_name = midi_note_name(note);
                            ui.horizontal(|ui| {
                                ui.label("Note:");
                                ui.label(format!("{} ({})", note_name, note));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Time:");
                                ui.label(format!("{:.3}s", time));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Duration:");
                                ui.label(format!("{:.3}s", dur));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Velocity:");
                                ui.label(format!("{}", vel));
                            });
                        }
                    }
                } else {
                    ui.label(format!("{} notes selected", indices.len()));
                }

                ui.add_space(4.0);
            });
    }

    /// Render node graph info section
    fn render_nodes_section(&self, ui: &mut Ui, path: &NodePath, node_indices: &[u32]) {
        egui::CollapsingHeader::new("Nodes")
            .id_salt(("nodes_info", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                ui.label(format!(
                    "{} node{} selected",
                    node_indices.len(),
                    if node_indices.len() == 1 { "" } else { "s" }
                ));

                ui.add_space(4.0);
            });
    }

    /// Render asset info section
    fn render_asset_section(&self, ui: &mut Ui, path: &NodePath, shared: &SharedPaneState, asset_ids: &[Uuid]) {
        let document = shared.action_executor.document();

        egui::CollapsingHeader::new("Asset")
            .id_salt(("asset_info", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                if asset_ids.len() == 1 {
                    let id = asset_ids[0];

                    if let Some(clip) = document.get_vector_clip(&id) {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.label(&clip.name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            ui.label("Vector");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Size:");
                            ui.label(format!("{:.0} x {:.0}", clip.width, clip.height));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Duration:");
                            ui.label(format!("{:.2}s", clip.duration));
                        });
                    } else if let Some(clip) = document.get_video_clip(&id) {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.label(&clip.name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            ui.label("Video");
                        });
                        ui.horizontal(|ui| {
                            ui.label("Size:");
                            ui.label(format!("{:.0} x {:.0}", clip.width, clip.height));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Duration:");
                            ui.label(format!("{:.2}s", clip.duration));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Frame Rate:");
                            ui.label(format!("{:.1} fps", clip.frame_rate));
                        });
                    } else if let Some(clip) = document.get_audio_clip(&id) {
                        ui.horizontal(|ui| {
                            ui.label("Name:");
                            ui.label(&clip.name);
                        });
                        let type_name = match &clip.clip_type {
                            lightningbeam_core::clip::AudioClipType::Sampled { .. } => "Audio (Sampled)",
                            lightningbeam_core::clip::AudioClipType::Midi { .. } => "Audio (MIDI)",
                            lightningbeam_core::clip::AudioClipType::Recording => "Audio (Recording)",
                        };
                        ui.horizontal(|ui| {
                            ui.label("Type:");
                            ui.label(type_name);
                        });
                        ui.horizontal(|ui| {
                            ui.label("Duration:");
                            ui.label(format!("{:.2}s", clip.duration));
                        });
                    } else {
                        // Could be an image asset or effect — show ID
                        ui.label(format!("Asset {}", id));
                    }
                } else {
                    ui.label(format!("{} assets selected", asset_ids.len()));
                }

                ui.add_space(4.0);
            });
    }
}

/// Convert MIDI note number to note name (e.g. 60 -> "C4")
fn midi_note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (note as i32 / 12) - 1;
    let name = NAMES[note as usize % 12];
    format!("{}{}", name, octave)
}

impl PaneRenderer for InfopanelPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Background
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgb(30, 35, 40),
        );

        // Create scrollable area for content
        let content_rect = rect.shrink(8.0);
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );

        egui::ScrollArea::vertical()
            .id_salt(("infopanel_scroll", path))
            .show(&mut content_ui, |ui| {
                ui.set_min_width(content_rect.width() - 16.0);

                // 1. Tool options section (always shown if tool has options)
                self.render_tool_section(ui, path, shared);

                // 2. Focus-driven content
                // Clone focus to avoid borrow issues with shared
                let focus = shared.focus.clone();
                match &focus {
                    FocusSelection::Layers(ids) => {
                        self.render_layer_section(ui, path, shared, ids);
                    }
                    FocusSelection::ClipInstances(ids) => {
                        self.render_clip_instance_section(ui, path, shared, ids);
                    }
                    FocusSelection::Geometry { .. } => {
                        let info = self.gather_selection_info(shared);
                        if info.dcel_count > 0 {
                            self.render_shape_section(ui, path, shared, &info);
                        }
                        // Selection count
                        if info.dcel_count > 0 {
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                            ui.label(format!(
                                "{} object{} selected",
                                info.dcel_count,
                                if info.dcel_count == 1 { "" } else { "s" }
                            ));
                        }
                    }
                    FocusSelection::Notes { layer_id, midi_clip_id, indices } => {
                        self.render_notes_section(ui, path, shared, *layer_id, *midi_clip_id, indices);
                    }
                    FocusSelection::Nodes(indices) => {
                        self.render_nodes_section(ui, path, indices);
                    }
                    FocusSelection::Assets(ids) => {
                        self.render_asset_section(ui, path, shared, ids);
                    }
                    FocusSelection::None => {
                        // Fallback: check if there's a DCEL selection even without focus
                        let info = self.gather_selection_info(shared);
                        if info.dcel_count > 0 {
                            self.render_shape_section(ui, path, shared, &info);
                            ui.add_space(8.0);
                            ui.separator();
                            ui.add_space(4.0);
                            ui.label(format!(
                                "{} object{} selected",
                                info.dcel_count,
                                if info.dcel_count == 1 { "" } else { "s" }
                            ));
                        } else {
                            self.render_document_section(ui, path, shared);
                        }
                    }
                }
            });
    }

    fn name(&self) -> &str {
        "Info Panel"
    }
}
