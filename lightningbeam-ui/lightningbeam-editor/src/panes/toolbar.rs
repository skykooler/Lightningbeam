/// Toolbar pane - displays drawing tool buttons
///
/// The toolbar shows all available drawing tools in a responsive grid layout.
/// Users can click to select tools, which updates the global selected_tool state.

use eframe::egui;
use lightningbeam_core::layer::{AnyLayer, LayerType};
use lightningbeam_core::tool::{Tool, RegionSelectMode, LassoMode};
use lightningbeam_core::brush_settings::bundled_brushes;
use crate::keymap::tool_app_action;
use super::{NodePath, PaneRenderer, SharedPaneState};

/// Toolbar pane state
pub struct ToolbarPane {
    // No local state needed for toolbar
}

impl ToolbarPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for ToolbarPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        let button_size = 60.0;
        let button_padding = 8.0;
        let button_spacing = 4.0;

        // Determine which tools to show based on the active layer type
        let active_layer_type: Option<LayerType> = shared.active_layer_id
            .and_then(|id| shared.action_executor.document().get_layer(&id))
            .map(|layer| match layer {
                AnyLayer::Vector(_) => LayerType::Vector,
                AnyLayer::Audio(_)  => LayerType::Audio,
                AnyLayer::Video(_)  => LayerType::Video,
                AnyLayer::Effect(_) => LayerType::Effect,
                AnyLayer::Group(_)  => LayerType::Group,
                AnyLayer::Raster(_) => LayerType::Raster,
                AnyLayer::Text(_)   => LayerType::Text,
            });

        // Auto-switch to Select if the current tool isn't available for this layer type
        let tools = Tool::for_layer_type(active_layer_type);
        if !tools.contains(shared.selected_tool) {
            *shared.selected_tool = Tool::Select;
        }

        // Calculate how many columns we can fit
        let available_width = rect.width() - (button_padding * 2.0);
        let columns =
            ((available_width + button_spacing) / (button_size + button_spacing)).floor() as usize;
        let columns = columns.max(1); // At least 1 column
        let total_tools = tools.len();
        let total_rows = (total_tools + columns - 1) / columns;

        let mut y = rect.top() + button_padding;

        // Process tools row by row for centered layout
        for row in 0..total_rows {
            let start_idx = row * columns;
            let end_idx = (start_idx + columns).min(total_tools);
            let buttons_in_row = end_idx - start_idx;

            // Calculate the total width of buttons in this row
            let row_width = (buttons_in_row as f32 * button_size)
                          + ((buttons_in_row.saturating_sub(1)) as f32 * button_spacing);

            // Center the row
            let mut x = rect.left() + (rect.width() - row_width) / 2.0;

            for tool_idx in start_idx..end_idx {
                let tool = &tools[tool_idx];
            let button_rect =
                egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(button_size, button_size));

            // Check if this is the selected tool
            let is_selected = *shared.selected_tool == *tool;

            // Button background
            let bg_color = if is_selected {
                shared.theme.bg_color(&["#toolbar", ".tool-button", ".selected"], ui.ctx(), egui::Color32::from_rgb(70, 100, 150))
            } else {
                shared.theme.bg_color(&["#toolbar", ".tool-button"], ui.ctx(), egui::Color32::from_rgb(50, 50, 50))
            };
            ui.painter().rect_filled(button_rect, 4.0, bg_color);

            // Load and render tool icon
            if let Some(icon) = shared.tool_icon_cache.get_or_load(*tool, ui.ctx()) {
                let icon_rect = button_rect.shrink(8.0); // Padding inside button
                ui.painter().image(
                    icon.id(),
                    icon_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            // Draw sub-tool arrow indicator for tools with modes
            let has_sub_tools = matches!(tool, Tool::RegionSelect | Tool::SelectLasso);
            if has_sub_tools {
                let arrow_size = 6.0;
                let margin = 4.0;
                let corner = button_rect.right_bottom() - egui::vec2(margin, margin);
                let tri = [
                    corner,
                    corner - egui::vec2(arrow_size, 0.0),
                    corner - egui::vec2(0.0, arrow_size),
                ];
                ui.painter().add(egui::Shape::convex_polygon(
                    tri.to_vec(),
                    shared.theme.text_color(&["#toolbar", ".tool-button"], ui.ctx(), egui::Color32::from_gray(200)),
                    egui::Stroke::NONE,
                ));
            }

            // Make button interactive (include path to ensure unique IDs across panes)
            let button_id = ui.id().with(("tool_button", path, *tool as usize));
            let response = ui.interact(button_rect, button_id, egui::Sense::click());

            // Check for click first
            if response.clicked() {
                *shared.selected_tool = *tool;
                // Preset-backed tools: auto-select the matching bundled brush.
                let preset_name = match tool {
                    Tool::Pencil   => Some("Pencil"),
                    Tool::Pen      => Some("Pen"),
                    Tool::Airbrush => Some("Airbrush"),
                    _ => None,
                };
                if let Some(name) = preset_name {
                    if let Some(preset) = bundled_brushes().iter().find(|p| p.name == name) {
                        let s = &preset.settings;
                        shared.raster_settings.brush_opacity  = s.opaque.clamp(0.0, 1.0);
                        shared.raster_settings.brush_hardness = s.hardness.clamp(0.0, 1.0);
                        shared.raster_settings.brush_spacing  = s.dabs_per_radius;
                        shared.raster_settings.active_brush_settings = s.clone();
                    }
                }
            }

            // Right-click context menu for tools with sub-options
            if has_sub_tools {
                response.context_menu(|ui| {
                    match tool {
                        Tool::RegionSelect => {
                            ui.set_min_width(120.0);
                            if ui.selectable_label(
                                *shared.region_select_mode == RegionSelectMode::Rectangle,
                                "Rectangle",
                            ).clicked() {
                                *shared.region_select_mode = RegionSelectMode::Rectangle;
                                *shared.selected_tool = Tool::RegionSelect;
                                ui.close();
                            }
                            if ui.selectable_label(
                                *shared.region_select_mode == RegionSelectMode::Lasso,
                                "Lasso",
                            ).clicked() {
                                *shared.region_select_mode = RegionSelectMode::Lasso;
                                *shared.selected_tool = Tool::RegionSelect;
                                ui.close();
                            }
                        }
                        Tool::SelectLasso => {
                            ui.set_min_width(130.0);
                            if ui.selectable_label(
                                *shared.lasso_mode == LassoMode::Freehand,
                                "Freehand",
                            ).clicked() {
                                *shared.lasso_mode = LassoMode::Freehand;
                                *shared.selected_tool = Tool::SelectLasso;
                                ui.close();
                            }
                            if ui.selectable_label(
                                *shared.lasso_mode == LassoMode::Polygonal,
                                "Polygonal",
                            ).clicked() {
                                *shared.lasso_mode = LassoMode::Polygonal;
                                *shared.selected_tool = Tool::SelectLasso;
                                ui.close();
                            }
                            if ui.selectable_label(
                                *shared.lasso_mode == LassoMode::Magnetic,
                                "Magnetic",
                            ).clicked() {
                                *shared.lasso_mode = LassoMode::Magnetic;
                                *shared.selected_tool = Tool::SelectLasso;
                                ui.close();
                            }
                        }
                        _ => {}
                    }
                });
            }

            if response.hovered() {
                ui.painter().rect_stroke(
                    button_rect,
                    4.0,
                    egui::Stroke::new(2.0, shared.theme.border_color(&["#toolbar", ".tool-button", ".hover"], ui.ctx(), egui::Color32::from_gray(180))),
                    egui::StrokeKind::Middle,
                );
            }

            // Show tooltip with tool name and shortcut (consumes response).
            // Hint text is pulled from the live keymap so it reflects user remappings.
            let hint = tool_app_action(*tool)
                .and_then(|action| shared.keymap.get(action))
                .map(|s| format!(" ({})", s.hint_text()))
                .unwrap_or_default();
            let tooltip = if *tool == Tool::RegionSelect {
                let mode = match *shared.region_select_mode {
                    RegionSelectMode::Rectangle => "Rectangle",
                    RegionSelectMode::Lasso => "Lasso",
                };
                format!("{} - {}{}\nRight-click for options", tool.display_name(), mode, hint)
            } else if *tool == Tool::SelectLasso {
                let mode = match *shared.lasso_mode {
                    LassoMode::Freehand  => "Freehand",
                    LassoMode::Polygonal => "Polygonal",
                    LassoMode::Magnetic  => "Magnetic",
                };
                format!("{} - {}{}\nRight-click for options", tool.display_name(), mode, hint)
            } else {
                format!("{}{}", tool.display_name(), hint)
            };
            response.on_hover_text(tooltip);

            // Draw selection border
            if is_selected {
                ui.painter().rect_stroke(
                    button_rect,
                    4.0,
                    egui::Stroke::new(2.0, shared.theme.border_color(&["#toolbar", ".tool-button", ".selected"], ui.ctx(), egui::Color32::from_rgb(100, 150, 255))),
                    egui::StrokeKind::Middle,
                );
            }

                // Move to next column in this row
                x += button_size + button_spacing;
            }

            // Move to next row
            y += button_size + button_spacing;
        }

        let is_raster = matches!(active_layer_type, Some(LayerType::Raster));
        let show_colors = matches!(active_layer_type, None | Some(LayerType::Vector) | Some(LayerType::Raster));

        // Add color pickers below the tool buttons
        if show_colors {
        y += button_spacing * 2.0; // Extra spacing

        let fill_label_width = 40.0;
        let color_button_size = 50.0;
        let color_row_width = fill_label_width + color_button_size + button_spacing;
        let color_x = rect.left() + (rect.width() - color_row_width) / 2.0;

        // Two color swatches:
        // Stroke/FG always on top, Fill/BG always on bottom.
        // Raster layers label them "FG" / "BG"; vector layers label them "Stroke" / "Fill".
        {
            let stroke_label = if is_raster { "FG" } else { "Stroke" };
            let label_color = shared.theme.text_color(&["#toolbar", ".text-secondary"], ui.ctx(), egui::Color32::from_gray(200));
            ui.painter().text(
                egui::pos2(color_x + fill_label_width / 2.0, y + color_button_size / 2.0),
                egui::Align2::CENTER_CENTER,
                stroke_label,
                egui::FontId::proportional(14.0),
                label_color,
            );

            let stroke_button_rect = egui::Rect::from_min_size(
                egui::pos2(color_x + fill_label_width + button_spacing, y),
                egui::vec2(color_button_size, color_button_size),
            );
            let stroke_button_id = ui.id().with(("stroke_color_button", path));
            let stroke_response = ui.interact(stroke_button_rect, stroke_button_id, egui::Sense::click());
            draw_color_button(ui, stroke_button_rect, *shared.stroke_color);
            egui::containers::Popup::from_toggle_button_response(&stroke_response)
                .show(|ui| {
                    ui.spacing_mut().slider_width = 275.0;
                    let changed = egui::color_picker::color_picker_color32(ui, shared.stroke_color, egui::color_picker::Alpha::OnlyBlend);
                    if changed {
                        *shared.active_color_mode = super::ColorMode::Stroke;
                    }
                });

            y += color_button_size + button_spacing;
        }

        // Fill/BG color swatch
        {
            let fill_label = if is_raster { "BG" } else { "Fill" };
            let label_color = shared.theme.text_color(&["#toolbar", ".text-secondary"], ui.ctx(), egui::Color32::from_gray(200));
            ui.painter().text(
                egui::pos2(color_x + fill_label_width / 2.0, y + color_button_size / 2.0),
                egui::Align2::CENTER_CENTER,
                fill_label,
                egui::FontId::proportional(14.0),
                label_color,
            );

            let fill_button_rect = egui::Rect::from_min_size(
                egui::pos2(color_x + fill_label_width + button_spacing, y),
                egui::vec2(color_button_size, color_button_size),
            );
            let fill_button_id = ui.id().with(("fill_color_button", path));
            let fill_response = ui.interact(fill_button_rect, fill_button_id, egui::Sense::click());
            draw_color_button(ui, fill_button_rect, *shared.fill_color);
            egui::containers::Popup::from_toggle_button_response(&fill_response)
                .show(|ui| {
                    ui.spacing_mut().slider_width = 275.0;
                    let changed = egui::color_picker::color_picker_color32(ui, shared.fill_color, egui::color_picker::Alpha::OnlyBlend);
                    if changed {
                        *shared.active_color_mode = super::ColorMode::Fill;
                    }
                });
        }
        } // end color pickers
    }

    fn name(&self) -> &str {
        "Toolbar"
    }
}

/// Draw a color button with checkerboard background for alpha channel
fn draw_color_button(ui: &mut egui::Ui, rect: egui::Rect, color: egui::Color32) {
    // Draw checkerboard background
    let checker_size = 5.0;
    let cols = (rect.width() / checker_size).ceil() as usize;
    let rows = (rect.height() / checker_size).ceil() as usize;

    for row in 0..rows {
        for col in 0..cols {
            let is_light = (row + col) % 2 == 0;
            let checker_color = if is_light {
                egui::Color32::from_gray(180)
            } else {
                egui::Color32::from_gray(120)
            };
            let checker_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.min.x + col as f32 * checker_size,
                    rect.min.y + row as f32 * checker_size,
                ),
                egui::vec2(checker_size, checker_size),
            ).intersect(rect);
            ui.painter().rect_filled(checker_rect, 0.0, checker_color);
        }
    }

    // Draw color on top
    ui.painter().rect_filled(rect, 2.0, color);

    // Draw border
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
        egui::StrokeKind::Middle,
    );
}
