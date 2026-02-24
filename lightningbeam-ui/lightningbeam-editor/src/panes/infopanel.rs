/// Info Panel pane - displays and edits properties of selected objects
///
/// Shows context-sensitive property editors based on current selection:
/// - Tool options (when a tool is active)
/// - Transform properties (when shapes are selected)
/// - Shape properties (fill/stroke for selected shapes)
/// - Document settings (when nothing is selected)

use eframe::egui::{self, DragValue, Ui};
use lightningbeam_core::actions::{SetDocumentPropertiesAction, SetShapePropertiesAction};
use lightningbeam_core::layer::AnyLayer;
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

/// Aggregated info about the current selection
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
    /// Gather info about the current selection
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
        let has_options = matches!(
            tool,
            Tool::Draw | Tool::Rectangle | Tool::Ellipse | Tool::PaintBucket | Tool::Polygon | Tool::Line | Tool::RegionSelect
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

    /// Render document settings section (shown when nothing is selected)
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

                // Layer count (read-only)
                ui.horizontal(|ui| {
                    ui.label("Layers:");
                    ui.label(format!("{}", layer_count));
                });

                ui.add_space(4.0);
            });
    }
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

                // 2. Gather selection info
                let info = self.gather_selection_info(shared);

                // 3. Shape properties section (if DCEL elements selected)
                if info.dcel_count > 0 {
                    self.render_shape_section(ui, path, shared, &info);
                }

                // 5. Document settings (if nothing selected)
                if info.is_empty {
                    self.render_document_section(ui, path, shared);
                }

                // Show selection count at bottom
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
            });
    }

    fn name(&self) -> &str {
        "Info Panel"
    }
}
