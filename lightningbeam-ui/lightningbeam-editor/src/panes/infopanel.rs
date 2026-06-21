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
use lightningbeam_core::brush_settings::{bundled_brushes, BrushSettings};
use lightningbeam_core::actions::{SetDocumentPropertiesAction, SetShapePropertiesAction, SetFillPaintAction};
use lightningbeam_core::gradient::ShapeGradient;
use lightningbeam_core::layer::{AnyLayer, LayerTrait};
use lightningbeam_core::selection::FocusSelection;
use lightningbeam_core::shape::ShapeColor;
use lightningbeam_core::tool::{SimplifyMode, Tool};
use super::{NodePath, PaneRenderer, SharedPaneState};
use super::gradient_editor::gradient_stop_editor;
use uuid::Uuid;

/// Info panel pane state
pub struct InfopanelPane {
    /// Whether the tool options section is expanded
    tool_section_open: bool,
    /// Whether the shape properties section is expanded
    shape_section_open: bool,
    /// Index of the selected paint brush preset (None = custom / unset)
    selected_brush_preset: Option<usize>,
    /// Whether the paint brush picker is expanded
    brush_picker_expanded: bool,
    /// Index of the selected eraser brush preset
    selected_eraser_preset: Option<usize>,
    /// Whether the eraser brush picker is expanded
    eraser_picker_expanded: bool,
    /// Cached preview textures, one per preset (populated lazily).
    brush_preview_textures: Vec<egui::TextureHandle>,
    /// Selected stop index for gradient editor in shape section.
    selected_shape_gradient_stop: Option<usize>,
    /// Selected stop index for gradient editor in tool section (gradient tool).
    selected_tool_gradient_stop: Option<usize>,
    /// FPS value captured when a drag/focus-in starts (for single-undo-action on commit)
    fps_drag_start: Option<f64>,
}

impl InfopanelPane {
    pub fn new() -> Self {
        let presets = bundled_brushes();
        let default_eraser_idx = presets.iter().position(|p| p.name == "Brush");
        Self {
            tool_section_open: true,
            shape_section_open: true,
            selected_brush_preset: None,
            brush_picker_expanded: false,
            selected_eraser_preset: default_eraser_idx,
            eraser_picker_expanded: false,
            brush_preview_textures: Vec::new(),
            selected_shape_gradient_stop: None,
            selected_tool_gradient_stop: None,
            fps_drag_start: None,
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
    /// None = mixed across selection; Some(None) = no gradient; Some(Some(g)) = all same gradient
    fill_gradient: Option<Option<ShapeGradient>>,
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
            fill_gradient: None,
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
        let face_count = shared.selection.selected_fills().len();
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
                    if let Some(graph) = vector_layer.graph_at_time(*shared.playback_time) {
                        // Gather stroke properties from selected edges
                        let mut first_stroke_color: Option<Option<ShapeColor>> = None;
                        let mut first_stroke_width: Option<f64> = None;
                        let mut stroke_color_mixed = false;
                        let mut stroke_width_mixed = false;

                        for &eid in shared.selection.selected_edges() {
                            let edge = graph.edge(lightningbeam_core::vector_graph::EdgeId(eid.0));
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
                        let mut first_fill_gradient: Option<Option<ShapeGradient>> = None;
                        let mut fill_gradient_mixed = false;

                        for &fid in shared.selection.selected_fills() {
                            let fill = graph.fill(fid);
                            let fc = fill.color;
                            let fg = fill.gradient_fill.clone();

                            match first_fill_color {
                                None => first_fill_color = Some(fc),
                                Some(prev) if prev != fc => fill_color_mixed = true,
                                _ => {}
                            }
                            match &first_fill_gradient {
                                None => first_fill_gradient = Some(fg),
                                Some(prev) if *prev != fg => fill_gradient_mixed = true,
                                _ => {}
                            }
                        }

                        if !fill_color_mixed {
                            info.fill_color = first_fill_color;
                        }
                        if !fill_gradient_mixed {
                            info.fill_gradient = first_fill_gradient;
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

        let active_is_raster = shared.active_layer_id
            .and_then(|id| shared.action_executor.document().get_layer(&id))
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));

        let raster_tool_def = active_is_raster.then(|| crate::tools::raster_tool_def(&tool)).flatten();
        let is_raster_paint_tool = raster_tool_def.is_some();

        // Only show tool options for tools that have options
        let is_vector_tool = !active_is_raster && matches!(
            tool,
            Tool::Select | Tool::BezierEdit | Tool::Draw | Tool::Rectangle
            | Tool::Ellipse | Tool::Line | Tool::Polygon
        );
        let is_raster_transform = active_is_raster
            && matches!(tool, Tool::Transform)
            && shared.selection.raster_floating.is_some();

        let is_raster_select = active_is_raster && matches!(tool, Tool::Select);
        let is_raster_shape = active_is_raster && matches!(
            tool,
            Tool::Rectangle | Tool::Ellipse | Tool::Line | Tool::Polygon
        );
        let has_options = is_vector_tool || is_raster_paint_tool || is_raster_transform
            || is_raster_select || is_raster_shape || matches!(
            tool,
            Tool::PaintBucket | Tool::RegionSelect | Tool::MagicWand | Tool::QuickSelect
            | Tool::Warp | Tool::Liquify | Tool::Gradient
        );

        if !has_options {
            return;
        }

        let header_label = if is_raster_transform {
            "Raster Transform"
        } else {
            raster_tool_def.map(|d| d.header_label()).unwrap_or("Tool Options")
        };

        egui::CollapsingHeader::new(header_label)
            .id_salt(("tool_options", path))
            .default_open(self.tool_section_open)
            .show(ui, |ui| {
                self.tool_section_open = true;
                ui.add_space(4.0);

                if is_vector_tool {
                    ui.checkbox(shared.snap_enabled, "Snap to Geometry");
                    ui.add_space(2.0);
                }

                // Raster transform tool hint.
                if is_raster_transform {
                    ui.label("Drag handles to move, scale, or rotate.");
                    ui.add_space(4.0);
                    ui.label("Enter — apply    Esc — cancel");
                    ui.add_space(4.0);
                    return;
                }

                // Raster paint tool: delegate to per-tool impl.
                if let Some(def) = raster_tool_def {
                    def.render_ui(ui, shared.raster_settings);
                    if def.show_brush_preset_picker() {
                        self.render_raster_tool_options(ui, shared, def.is_eraser());
                    }
                }

                match tool {
                    Tool::Draw if !is_raster_paint_tool => {
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
                        if active_is_raster {
                            use crate::tools::FillThresholdMode;
                            ui.horizontal(|ui| {
                                ui.label("Threshold:");
                                ui.add(
                                    egui::Slider::new(
                                        &mut shared.raster_settings.fill_threshold,
                                        0.0_f32..=255.0,
                                    )
                                    .step_by(1.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Softness:");
                                ui.add(
                                    egui::Slider::new(
                                        &mut shared.raster_settings.fill_softness,
                                        0.0_f32..=100.0,
                                    )
                                    .custom_formatter(|v, _| format!("{:.0}%", v)),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label("Mode:");
                                ui.selectable_value(
                                    &mut shared.raster_settings.fill_threshold_mode,
                                    FillThresholdMode::Absolute,
                                    "Absolute",
                                );
                                ui.selectable_value(
                                    &mut shared.raster_settings.fill_threshold_mode,
                                    FillThresholdMode::Relative,
                                    "Relative",
                                );
                            });
                        } else {
                            // Vector: gap tolerance
                            ui.horizontal(|ui| {
                                ui.label("Gap Tolerance:");
                                ui.add(
                                    DragValue::new(shared.paint_bucket_gap_tolerance)
                                        .speed(0.1)
                                        .range(0.0..=50.0),
                                );
                            });
                        }
                    }

                    Tool::Select if is_raster_select => {
                        use crate::tools::SelectionShape;
                        ui.horizontal(|ui| {
                            ui.label("Shape:");
                            ui.selectable_value(
                                &mut shared.raster_settings.select_shape,
                                SelectionShape::Rect,
                                "Rectangle",
                            );
                            ui.selectable_value(
                                &mut shared.raster_settings.select_shape,
                                SelectionShape::Ellipse,
                                "Ellipse",
                            );
                        });
                    }

                    Tool::MagicWand => {
                        use crate::tools::FillThresholdMode;
                        ui.horizontal(|ui| {
                            ui.label("Threshold:");
                            ui.add(
                                egui::Slider::new(
                                    &mut shared.raster_settings.wand_threshold,
                                    0.0_f32..=255.0,
                                )
                                .step_by(1.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            ui.selectable_value(
                                &mut shared.raster_settings.wand_mode,
                                FillThresholdMode::Absolute,
                                "Absolute",
                            );
                            ui.selectable_value(
                                &mut shared.raster_settings.wand_mode,
                                FillThresholdMode::Relative,
                                "Relative",
                            );
                        });
                        ui.checkbox(&mut shared.raster_settings.wand_contiguous, "Contiguous");
                    }

                    Tool::QuickSelect => {
                        use crate::tools::FillThresholdMode;
                        ui.horizontal(|ui| {
                            ui.label("Radius:");
                            ui.add(
                                egui::Slider::new(
                                    &mut shared.raster_settings.quick_select_radius,
                                    1.0_f32..=200.0,
                                )
                                .step_by(1.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Threshold:");
                            ui.add(
                                egui::Slider::new(
                                    &mut shared.raster_settings.wand_threshold,
                                    0.0_f32..=255.0,
                                )
                                .step_by(1.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            ui.selectable_value(
                                &mut shared.raster_settings.wand_mode,
                                FillThresholdMode::Absolute,
                                "Absolute",
                            );
                            ui.selectable_value(
                                &mut shared.raster_settings.wand_mode,
                                FillThresholdMode::Relative,
                                "Relative",
                            );
                        });
                    }

                    Tool::Warp => {
                        ui.horizontal(|ui| {
                            ui.label("Grid:");
                            let cols = shared.raster_settings.warp_grid_cols;
                            let rows = shared.raster_settings.warp_grid_rows;
                            for (label, c, r) in [("3×3", 3u32, 3u32), ("4×4", 4, 4), ("5×5", 5, 5), ("8×8", 8, 8)] {
                                let selected = cols == c && rows == r;
                                if ui.selectable_label(selected, label).clicked() {
                                    shared.raster_settings.warp_grid_cols = c;
                                    shared.raster_settings.warp_grid_rows = r;
                                }
                            }
                        });
                        ui.small("Enter to commit · Escape to cancel");
                    }

                    Tool::Liquify => {
                        use crate::tools::LiquifyMode;
                        ui.horizontal(|ui| {
                            ui.label("Mode:");
                            for (label, mode) in [
                                ("Push",        LiquifyMode::Push),
                                ("Pucker",       LiquifyMode::Pucker),
                                ("Bloat",        LiquifyMode::Bloat),
                                ("Smooth",       LiquifyMode::Smooth),
                                ("Reconstruct",  LiquifyMode::Reconstruct),
                            ] {
                                let selected = shared.raster_settings.liquify_mode == mode;
                                if ui.selectable_label(selected, label).clicked() {
                                    shared.raster_settings.liquify_mode = mode;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label("Radius:");
                            ui.add(
                                egui::Slider::new(
                                    &mut shared.raster_settings.liquify_radius,
                                    5.0_f32..=500.0,
                                )
                                .step_by(1.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label("Strength:");
                            ui.add(
                                egui::Slider::new(
                                    &mut shared.raster_settings.liquify_strength,
                                    0.01_f32..=1.0,
                                )
                                .step_by(0.01),
                            );
                        });
                        ui.small("Enter to commit · Escape to cancel");
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

                    Tool::Gradient => {
                        if active_is_raster {
                            ui.horizontal(|ui| {
                                ui.label("Opacity:");
                                ui.add(egui::Slider::new(
                                    &mut shared.raster_settings.gradient_opacity,
                                    0.0_f32..=1.0,
                                ).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                            });
                            ui.add_space(4.0);
                        }
                        gradient_stop_editor(
                            ui,
                            &mut shared.raster_settings.gradient,
                            &mut self.selected_tool_gradient_stop,
                        );
                    }

                    _ => {}
                }

                ui.add_space(4.0);
            });
    }

    /// Render all options for a raster paint tool (brush picker + sliders).
    /// `is_eraser` drives which shared state is read/written.
    fn render_raster_tool_options(
        &mut self,
        ui: &mut Ui,
        shared: &mut SharedPaneState,
        is_eraser: bool,
    ) {
        self.render_brush_preset_grid(ui, shared, is_eraser);
        ui.add_space(2.0);

        let rs = &mut shared.raster_settings;

        if !is_eraser {
            ui.horizontal(|ui| {
                ui.label("Color:");
                ui.selectable_value(&mut rs.brush_use_fg, true, "FG");
                ui.selectable_value(&mut rs.brush_use_fg, false, "BG");
            });
        }

        macro_rules! field {
            ($eraser:ident, $brush:ident) => {
                if is_eraser { &mut rs.$eraser } else { &mut rs.$brush }
            }
        }

        ui.horizontal(|ui| {
            ui.label("Size:");
            ui.add(egui::Slider::new(field!(eraser_radius, brush_radius), 1.0_f32..=200.0).logarithmic(true).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Opacity:");
            ui.add(egui::Slider::new(field!(eraser_opacity, brush_opacity), 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Hardness:");
            ui.add(egui::Slider::new(field!(eraser_hardness, brush_hardness), 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Spacing:");
            ui.add(egui::Slider::new(field!(eraser_spacing, brush_spacing), 0.01_f32..=1.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });

        let bs = if is_eraser { &rs.active_eraser_settings } else { &rs.active_brush_settings };
        if bs.elliptical_dab_ratio > 1.001 {
            ui.horizontal(|ui| {
                ui.label("Angle:");
                ui.add(egui::Slider::new(&mut rs.brush_angle_offset, -180.0_f32..=180.0)
                    .suffix("°")
                    .custom_formatter(|v, _| format!("{:.0}°", v)));
            });
        }
    }

    /// Render the brush preset thumbnail grid (collapsible).
    /// `is_eraser` drives which picker state and which shared settings are updated.
    fn render_brush_preset_grid(&mut self, ui: &mut Ui, shared: &mut SharedPaneState, is_eraser: bool) {
        let presets = bundled_brushes();
        if presets.is_empty() { return; }

        // Build preview TextureHandles from GPU-rendered pixel data when available.
        if self.brush_preview_textures.len() != presets.len() {
            if let Ok(previews) = shared.brush_preview_pixels.try_lock() {
                if previews.len() == presets.len() {
                    self.brush_preview_textures.clear();
                    for (idx, (w, h, pixels)) in previews.iter().enumerate() {
                        let image = egui::ColorImage::from_rgba_premultiplied(
                            [*w as usize, *h as usize],
                            pixels,
                        );
                        let handle = ui.ctx().load_texture(
                            format!("brush_preview_{}", presets[idx].name),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        self.brush_preview_textures.push(handle);
                    }
                }
            }
        }

        // Read picker state into locals to avoid multiple &mut self borrows.
        let mut expanded = if is_eraser { self.eraser_picker_expanded } else { self.brush_picker_expanded };
        let mut selected = if is_eraser { self.selected_eraser_preset } else { self.selected_brush_preset };

        let gap = 3.0;
        let cols = 2usize;
        let avail_w = ui.available_width();
        let cell_w = ((avail_w - gap * (cols as f32 - 1.0)) / cols as f32).max(50.0);
        let cell_h = 80.0;

        if !expanded {
            // Collapsed: show just the currently selected preset as a single wide cell.
            let show_idx = selected.unwrap_or(0);
            if let Some(preset) = presets.get(show_idx) {
                let full_w = avail_w.max(50.0);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(full_w, cell_h), egui::Sense::click());
                let painter = ui.painter();
                let bg = if resp.hovered() {
                    egui::Color32::from_rgb(50, 56, 70)
                } else {
                    egui::Color32::from_rgb(45, 65, 95)
                };
                painter.rect_filled(rect, 4.0, bg);
                painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 140, 220)), egui::StrokeKind::Middle);
                let preview_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(4.0, 4.0),
                    egui::vec2(rect.width() - 8.0, cell_h - 22.0),
                );
                if let Some(tex) = self.brush_preview_textures.get(show_idx) {
                    painter.image(tex.id(), preview_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE);
                }
                painter.text(egui::pos2(rect.center().x, rect.max.y - 9.0),
                    egui::Align2::CENTER_CENTER, preset.name,
                    egui::FontId::proportional(9.5), egui::Color32::from_rgb(140, 190, 255));
                if resp.clicked() { expanded = true; }
            }
        } else {
            // Expanded: full grid; clicking a preset selects it and collapses.
            for (row_idx, chunk) in presets.chunks(cols).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for (col_idx, preset) in chunk.iter().enumerate() {
                        let idx = row_idx * cols + col_idx;
                        let is_sel = selected == Some(idx);
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(cell_w, cell_h), egui::Sense::click());
                        let painter = ui.painter();
                        let bg = if is_sel {
                            egui::Color32::from_rgb(45, 65, 95)
                        } else if resp.hovered() {
                            egui::Color32::from_rgb(45, 50, 62)
                        } else {
                            egui::Color32::from_rgb(32, 36, 44)
                        };
                        painter.rect_filled(rect, 4.0, bg);
                        if is_sel {
                            painter.rect_stroke(rect, 4.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(80, 140, 220)), egui::StrokeKind::Middle);
                        }
                        let preview_rect = egui::Rect::from_min_size(
                            rect.min + egui::vec2(4.0, 4.0),
                            egui::vec2(cell_w - 8.0, cell_h - 22.0),
                        );
                        if let Some(tex) = self.brush_preview_textures.get(idx) {
                            painter.image(tex.id(), preview_rect,
                                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                egui::Color32::WHITE);
                        }
                        painter.text(egui::pos2(rect.center().x, rect.max.y - 9.0),
                            egui::Align2::CENTER_CENTER, preset.name,
                            egui::FontId::proportional(9.5),
                            if is_sel { egui::Color32::from_rgb(140, 190, 255) } else { egui::Color32::from_gray(160) });
                        if resp.clicked() {
                            selected = Some(idx);
                            expanded = false;
                            let s = &preset.settings;
                            let rs = &mut shared.raster_settings;
                            if is_eraser {
                                rs.eraser_opacity  = s.opaque.clamp(0.0, 1.0);
                                rs.eraser_hardness = s.hardness.clamp(0.0, 1.0);
                                rs.eraser_spacing  = s.dabs_per_radius;
                                rs.active_eraser_settings = s.clone();
                            } else {
                                rs.brush_opacity  = s.opaque.clamp(0.0, 1.0);
                                rs.brush_hardness = s.hardness.clamp(0.0, 1.0);
                                rs.brush_spacing  = s.dabs_per_radius;
                                rs.active_brush_settings = s.clone();
                                // If the user was on a preset-backed tool (Pencil/Pen/Airbrush)
                                // and manually picked a different brush, revert to the generic tool.
                                if matches!(*shared.selected_tool, Tool::Pencil | Tool::Pen | Tool::Airbrush) {
                                    *shared.selected_tool = Tool::Draw;
                                }
                            }
                        }
                    }
                });
                ui.add_space(gap);
            }
        }

        // Write back picker state.
        if is_eraser {
            self.eraser_picker_expanded = expanded;
            self.selected_eraser_preset = selected;
        } else {
            self.brush_picker_expanded = expanded;
            self.selected_brush_preset = selected;
        }
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
        let face_ids: Vec<lightningbeam_core::vector_graph::FillId> = shared.selection.selected_fills()
            .iter().map(|fid| lightningbeam_core::vector_graph::FillId(fid.0)).collect();
        let edge_ids: Vec<lightningbeam_core::vector_graph::EdgeId> = shared.selection.selected_edges()
            .iter().map(|eid| lightningbeam_core::vector_graph::EdgeId(eid.0)).collect();

        egui::CollapsingHeader::new("Shape")
            .id_salt(("shape", path))
            .default_open(self.shape_section_open)
            .show(ui, |ui| {
                self.shape_section_open = true;
                ui.add_space(4.0);

                // Fill — determine current fill type
                let has_gradient = matches!(&info.fill_gradient, Some(Some(_)));
                let has_solid = matches!(&info.fill_color, Some(Some(_)));
                let fill_is_none = matches!(&info.fill_color, Some(None))
                    && matches!(&info.fill_gradient, Some(None));
                let fill_mixed = info.fill_color.is_none() && info.fill_gradient.is_none();

                // Fill type toggle row
                ui.horizontal(|ui| {
                    ui.label("Fill:");
                    if fill_mixed {
                        ui.label("--");
                    } else {
                        if ui.selectable_label(fill_is_none, "None").clicked() && !fill_is_none {
                            let action = SetFillPaintAction::solid(
                                layer_id, time, face_ids.clone(), None,
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                        if ui.selectable_label(has_solid || (!has_gradient && !fill_is_none), "Solid").clicked() && !has_solid {
                            // Switch to solid: use existing color or default to black
                            let color = info.fill_color.flatten()
                                .unwrap_or(ShapeColor::rgba(0, 0, 0, 255));
                            let action = SetFillPaintAction::solid(
                                layer_id, time, face_ids.clone(), Some(color),
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                        if ui.selectable_label(has_gradient, "Gradient").clicked() && !has_gradient {
                            let grad = info.fill_gradient.clone().flatten()
                                .unwrap_or_default();
                            let action = SetFillPaintAction::gradient(
                                layer_id, time, face_ids.clone(), Some(grad),
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                    }
                });

                // Solid fill color editor
                if !fill_mixed && has_solid {
                    if let Some(Some(color)) = info.fill_color {
                        ui.horizontal(|ui| {
                            let mut rgba = [color.r, color.g, color.b, color.a];
                            if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                                let new_color = ShapeColor::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
                                let action = SetFillPaintAction::solid(
                                    layer_id, time, face_ids.clone(), Some(new_color),
                                );
                                shared.pending_actions.push(Box::new(action));
                            }
                        });
                    }
                }

                // Gradient fill editor
                if !fill_mixed && has_gradient {
                    if let Some(Some(mut grad)) = info.fill_gradient.clone() {
                        if gradient_stop_editor(ui, &mut grad, &mut self.selected_shape_gradient_stop) {
                            let action = SetFillPaintAction::gradient(
                                layer_id, time, face_ids.clone(), Some(grad),
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                    }
                }

                // Stroke color
                ui.horizontal(|ui| {
                    ui.label("Stroke:");
                    match info.stroke_color {
                        Some(Some(color)) => {
                            let mut rgba = [color.r, color.g, color.b, color.a];
                            if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                                let new_color = ShapeColor::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
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
    fn render_document_section(&mut self, ui: &mut Ui, path: &NodePath, shared: &mut SharedPaneState) {
        egui::CollapsingHeader::new("Document")
            .id_salt(("document", path))
            .default_open(true)
            .show(ui, |ui| {
                ui.add_space(4.0);

                // Extract all needed values up front, then drop the borrow before closures
                // that need mutable access to shared or self.
                let (mut width, mut height, mut duration, mut framerate, layer_count, background_color) = {
                    let document = shared.action_executor.document();
                    (document.width, document.height, document.duration, document.framerate,
                     document.root.children.len(), document.background_color)
                };

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
                    let fps_response = ui.add(
                        DragValue::new(&mut framerate)
                            .speed(1.0)
                            .range(1.0..=120.0)
                            .suffix(" fps"),
                    );

                    if fps_response.gained_focus() || fps_response.drag_started() {
                        if self.fps_drag_start.is_none() {
                            self.fps_drag_start = Some(framerate);
                        }
                    }

                    if fps_response.changed() {
                        // Live preview: update document directly
                        shared.action_executor.document_mut().framerate = framerate;
                    }

                    if fps_response.drag_stopped() || fps_response.lost_focus() {
                        if let Some(start_fps) = self.fps_drag_start.take() {
                            let new_fps = shared.action_executor.document().framerate;
                            let timeline_mode = shared.action_executor.document().timeline_mode;
                            if (start_fps - new_fps).abs() > 1e-9
                                && timeline_mode == lightningbeam_core::document::TimelineMode::Frames
                            {
                                use lightningbeam_core::actions::ChangeFpsAction;
                                // Revert live-preview so the action owns it
                                shared.action_executor.document_mut().framerate = start_fps;
                                let action = ChangeFpsAction::new(
                                    start_fps,
                                    new_fps,
                                    shared.action_executor.document(),
                                );
                                shared.pending_actions.push(Box::new(action));
                            } else if (start_fps - new_fps).abs() > 1e-9 {
                                // Not in Frames mode — use simple property action (no stretching)
                                shared.action_executor.document_mut().framerate = start_fps;
                                let action = SetDocumentPropertiesAction::set_framerate(new_fps);
                                shared.pending_actions.push(Box::new(action));
                            }
                        }
                    }
                });

                // Background color (with alpha)
                ui.horizontal(|ui| {
                    ui.label("Background:");
                    let bg = background_color;
                    let mut color = [bg.r, bg.g, bg.b, bg.a];
                    if ui.color_edit_button_srgba_unmultiplied(&mut color).changed() {
                        let action = SetDocumentPropertiesAction::set_background_color(
                            ShapeColor::rgba(color[0], color[1], color[2], color[3]),
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

    /// Render the onion-skinning view settings (global; not tied to selection).
    fn render_onion_section(&mut self, ui: &mut Ui, path: &NodePath, shared: &mut SharedPaneState) {
        egui::CollapsingHeader::new("Onion Skin")
            .id_salt(("onion", path))
            .default_open(shared.onion_skin.enabled)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.checkbox(&mut shared.onion_skin.enabled, "Enabled");
                ui.add_enabled_ui(shared.onion_skin.enabled, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Frames before:");
                        ui.add(DragValue::new(&mut shared.onion_skin.frames_before).range(0..=5));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Frames after:");
                        ui.add(DragValue::new(&mut shared.onion_skin.frames_after).range(0..=5));
                    });
                    ui.add(egui::Slider::new(&mut shared.onion_skin.opacity, 0.0..=1.0).text("Opacity"));
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
                            AnyLayer::Raster(_) => "Raster",
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
                            AnyLayer::Raster(_) => &[],
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
                            let total_dur = ci.total_duration(clip_dur, document.tempo_map());
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
                        // Resolve note-on/off pairs to (time, note, vel, dur) tuples
                        let mut notes: Vec<(f64, u8, u8, f64)> = Vec::new();
                        let mut pending: std::collections::HashMap<u8, (f64, u8)> = std::collections::HashMap::new();
                        for event in events {
                            if event.is_note_on() {
                                pending.insert(event.data1, (event.timestamp.beats_to_f64(), event.data2));
                            } else if event.is_note_off() {
                                if let Some((start, v)) = pending.remove(&event.data1) {
                                    notes.push((start, event.data1, v, event.timestamp.beats_to_f64() - start));
                                }
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

/// Draw a brush dab preview into `rect` approximating the brush falloff shape.
///
/// Renders N concentric filled circles from outermost to innermost.  Because each
/// inner circle overwrites the pixels of all outer circles beneath it, the visible
/// alpha at distance `d` from the centre equals the alpha of the innermost circle
/// whose radius ≥ `d`.  This step-approximates the actual brush falloff formula:
/// `opa = ((1 − r) / (1 − hardness))²` for `r > hardness`, 1 inside the hard core.
fn paint_brush_dab(painter: &egui::Painter, rect: egui::Rect, s: &BrushSettings) {
    let center = rect.center();
    let max_r = (rect.width().min(rect.height()) / 2.0 - 2.0).max(1.0);
    let h = s.hardness;
    let a = s.opaque;

    const N: usize = 12;
    for i in 0..N {
        // t: normalized radial position of this ring, 1.0 = outermost edge
        let t = 1.0 - i as f32 / N as f32;
        let r = max_r * t;

        let opa_weight = if h >= 1.0 || t <= h {
            1.0f32
        } else {
            let x = (1.0 - t) / (1.0 - h).max(1e-4);
            (x * x).min(1.0)
        };

        let alpha = (opa_weight * a * 220.0).min(220.0) as u8;
        painter.circle_filled(
            center, r,
            egui::Color32::from_rgba_unmultiplied(200, 200, 220, alpha),
        );
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
        let bg = shared.theme.bg_color(&["#infopanel", ".pane-content"], ui.ctx(), egui::Color32::from_rgb(30, 35, 40));
        ui.painter().rect_filled(rect, 0.0, bg);

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

                // Onion-skinning view settings — always available, regardless of selection.
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                self.render_onion_section(ui, path, shared);
            });
    }

    fn name(&self) -> &str {
        "Info Panel"
    }
}
