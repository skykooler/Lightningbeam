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
        // `ui` spans the whole window, not this pane — bind a child Ui to the pane's content rect
        // first, or the ScrollArea would start at the window's top (under the pane header) and
        // size itself against the window's height (so it would never need to scroll).
        // Salt by path: widget ids inside are auto-generated from the Ui's id, so two toolbar
        // panes would otherwise fight over the same ids.
        let mut content_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("toolbar", path))
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );

        egui::ScrollArea::vertical()
            .id_salt(("toolbar_scroll", path))
            .auto_shrink([false; 2])
            .show(&mut content_ui, |ui| {
                self.render_toolbar(ui, path, shared);
            });
    }

    fn name(&self) -> &str {
        "Toolbar"
    }
}

const BUTTON_SIZE: f32 = 60.0;
const BUTTON_PADDING: f32 = 8.0;
const BUTTON_SPACING: f32 = 4.0;
const COLOR_BUTTON_SIZE: f32 = 50.0;
const COLOR_LABEL_WIDTH: f32 = 40.0;

impl ToolbarPane {
    /// Laid out with real egui widgets (`horizontal_wrapped` for the tool grid) rather than
    /// absolute rect math, so the ScrollArea above can measure the content and scroll it when the
    /// pane is too short. Buttons are still painted by hand — `allocate_exact_size` reserves the
    /// space and gives us the Response, and we draw into the rect egui hands back.
    fn render_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
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

        let is_raster = matches!(active_layer_type, Some(LayerType::Raster));
        let show_colors = matches!(active_layer_type, None | Some(LayerType::Vector) | Some(LayerType::Raster));

        ui.spacing_mut().item_spacing = egui::vec2(BUTTON_SPACING, BUTTON_SPACING);
        ui.add_space(BUTTON_PADDING);

        // Centre the grid as a block. Work out how many columns fit, then lay the buttons out in
        // a band of exactly that width, positioned to centre it. The band has to be an explicit
        // max_rect on the child Ui — a `horizontal_wrapped` nested inside a `horizontal` inherits
        // the parent's available width and wraps against that, not against the width we set.
        let full_width = ui.available_width();
        let avail = full_width - BUTTON_PADDING * 2.0;
        let columns = (((avail + BUTTON_SPACING) / (BUTTON_SIZE + BUTTON_SPACING)).floor() as usize)
            .max(1)
            .min(tools.len().max(1));
        let grid_width =
            columns as f32 * BUTTON_SIZE + (columns.saturating_sub(1)) as f32 * BUTTON_SPACING;
        let indent = ((full_width - grid_width) / 2.0).max(0.0);
        let rows = (tools.len() + columns - 1) / columns;

        let cursor = ui.cursor().min;
        let band = egui::Rect::from_min_size(
            egui::pos2(cursor.x + indent, cursor.y),
            egui::vec2(grid_width, rows as f32 * (BUTTON_SIZE + BUTTON_SPACING)),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(band).layout(
                egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true),
            ),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(BUTTON_SPACING, BUTTON_SPACING);
                for tool in tools.iter() {
                    self.render_tool_button(ui, tool, shared);
                }
            },
        );

        // Colour swatches below the tools.
        // Stroke/FG always on top, Fill/BG always on bottom.
        // Raster layers label them "FG" / "BG"; vector layers label them "Stroke" / "Fill".
        if show_colors {
            ui.add_space(BUTTON_SPACING * 2.0);

            let row_width = COLOR_LABEL_WIDTH + COLOR_BUTTON_SIZE + BUTTON_SPACING;
            let color_indent = ((ui.available_width() - row_width) / 2.0).max(0.0);

            for (label, is_stroke) in [
                (if is_raster { "FG" } else { "Stroke" }, true),
                (if is_raster { "BG" } else { "Fill" }, false),
            ] {
                ui.horizontal(|ui| {
                    ui.add_space(color_indent);

                    let (label_rect, _) = ui.allocate_exact_size(
                        egui::vec2(COLOR_LABEL_WIDTH, COLOR_BUTTON_SIZE),
                        egui::Sense::hover(),
                    );
                    let label_color = shared.theme.text_color(
                        &["#toolbar", ".text-secondary"],
                        ui.ctx(),
                        egui::Color32::from_gray(200),
                    );
                    ui.painter().text(
                        label_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(14.0),
                        label_color,
                    );

                    let (swatch_rect, _) = ui.allocate_exact_size(
                        egui::vec2(COLOR_BUTTON_SIZE, COLOR_BUTTON_SIZE),
                        egui::Sense::hover(),
                    );
                    let (id_key, color) = if is_stroke {
                        ("stroke_color_button", &mut *shared.stroke_color)
                    } else {
                        ("fill_color_button", &mut *shared.fill_color)
                    };
                    let button_id = ui.id().with((id_key, path));
                    crate::widgets::color_swatch::color_swatch(ui, button_id, swatch_rect, color);
                });
            }
        }

        ui.add_space(BUTTON_PADDING);
    }

    fn render_tool_button(
        &mut self,
        ui: &mut egui::Ui,
        tool: &Tool,
        shared: &mut SharedPaneState,
    ) {
        let (button_rect, response) = ui.allocate_exact_size(
            egui::vec2(BUTTON_SIZE, BUTTON_SIZE),
            egui::Sense::click(),
        );

        let is_selected = *shared.selected_tool == *tool;

        // Button background
        let bg_color = if is_selected {
            shared.theme.bg_color(&["#toolbar", ".tool-button", ".selected"], ui.ctx(), egui::Color32::from_rgb(70, 100, 150))
        } else {
            shared.theme.bg_color(&["#toolbar", ".tool-button"], ui.ctx(), egui::Color32::from_rgb(50, 50, 50))
        };
        ui.painter().rect_filled(button_rect, 4.0, bg_color);

        // Tool icon: tools without a bundled SVG fall back to a Lucide glyph rather than the
        // shared TODO placeholder.
        if let Some(glyph) = crate::mobile::icons::tool_glyph(*tool) {
            ui.painter().text(
                button_rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                crate::mobile::icons::font(26.0),
                shared.theme.text_color(
                    &["#toolbar", ".tool-button"],
                    ui.ctx(),
                    egui::Color32::from_gray(220),
                ),
            );
        } else if let Some(icon) = shared.tool_icon_cache.get_or_load(*tool, ui.ctx()) {
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
                if let Some(idx) = bundled_brushes().iter().position(|p| p.name == name) {
                    let settings = bundled_brushes()[idx].settings.clone();
                    shared
                        .raster_settings
                        .brush_mut(crate::tools::BrushKind::Paint)
                        .apply_preset(idx, &settings);
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

        // Draw selection border
        if is_selected {
            ui.painter().rect_stroke(
                button_rect,
                4.0,
                egui::Stroke::new(2.0, shared.theme.border_color(&["#toolbar", ".tool-button", ".selected"], ui.ctx(), egui::Color32::from_rgb(100, 150, 255))),
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
    }
}
