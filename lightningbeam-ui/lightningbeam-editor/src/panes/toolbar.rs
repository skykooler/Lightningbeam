/// Toolbar pane - displays drawing tool buttons
///
/// The toolbar shows all available drawing tools in a responsive grid layout.
/// Users can click to select tools, which updates the global selected_tool state.

use eframe::egui;
use lightningbeam_core::tool::Tool;
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

        // Calculate how many columns we can fit
        let available_width = rect.width() - (button_padding * 2.0);
        let columns =
            ((available_width + button_spacing) / (button_size + button_spacing)).floor() as usize;
        let columns = columns.max(1); // At least 1 column

        // Calculate total number of tools and rows
        let tools = Tool::all();
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
                egui::Color32::from_rgb(70, 100, 150) // Highlighted blue
            } else {
                egui::Color32::from_rgb(50, 50, 50)
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

            // Make button interactive (include path to ensure unique IDs across panes)
            let button_id = ui.id().with(("tool_button", path, *tool as usize));
            let response = ui.interact(button_rect, button_id, egui::Sense::click());

            // Check for click first
            if response.clicked() {
                *shared.selected_tool = *tool;
            }

            if response.hovered() {
                ui.painter().rect_stroke(
                    button_rect,
                    4.0,
                    egui::Stroke::new(2.0, egui::Color32::from_gray(180)),
                    egui::StrokeKind::Middle,
                );
            }

            // Show tooltip with tool name and shortcut (consumes response)
            response.on_hover_text(format!("{} ({})", tool.display_name(), tool.shortcut_hint()));

            // Draw selection border
            if is_selected {
                ui.painter().rect_stroke(
                    button_rect,
                    4.0,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                    egui::StrokeKind::Middle,
                );
            }

                // Move to next column in this row
                x += button_size + button_spacing;
            }

            // Move to next row
            y += button_size + button_spacing;
        }

        // Add color pickers below the tool buttons
        y += button_spacing * 2.0; // Extra spacing

        // Fill Color
        let fill_label_width = 40.0;
        let color_button_size = 50.0;
        let color_row_width = fill_label_width + color_button_size + button_spacing;
        let color_x = rect.left() + (rect.width() - color_row_width) / 2.0;

        // Fill color label
        ui.painter().text(
            egui::pos2(color_x + fill_label_width / 2.0, y + color_button_size / 2.0),
            egui::Align2::CENTER_CENTER,
            "Fill",
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(200),
        );

        // Fill color button
        let fill_button_rect = egui::Rect::from_min_size(
            egui::pos2(color_x + fill_label_width + button_spacing, y),
            egui::vec2(color_button_size, color_button_size),
        );
        let fill_button_id = ui.id().with(("fill_color_button", path));
        let fill_response = ui.interact(fill_button_rect, fill_button_id, egui::Sense::click());

        // Draw fill color button with checkerboard for alpha
        draw_color_button(ui, fill_button_rect, *shared.fill_color);

        if fill_response.clicked() {
            // Open color picker popup
            ui.memory_mut(|mem| mem.toggle_popup(fill_button_id));
        }

        // Show fill color picker popup
        egui::popup::popup_below_widget(ui, fill_button_id, &fill_response, egui::popup::PopupCloseBehavior::CloseOnClickOutside, |ui: &mut egui::Ui| {
            let changed = egui::color_picker::color_picker_color32(ui, shared.fill_color, egui::color_picker::Alpha::OnlyBlend);
            // Track that the user interacted with the fill color
            if changed {
                *shared.active_color_mode = super::ColorMode::Fill;
            }
        });

        y += color_button_size + button_spacing;

        // Stroke color label
        ui.painter().text(
            egui::pos2(color_x + fill_label_width / 2.0, y + color_button_size / 2.0),
            egui::Align2::CENTER_CENTER,
            "Stroke",
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(200),
        );

        // Stroke color button
        let stroke_button_rect = egui::Rect::from_min_size(
            egui::pos2(color_x + fill_label_width + button_spacing, y),
            egui::vec2(color_button_size, color_button_size),
        );
        let stroke_button_id = ui.id().with(("stroke_color_button", path));
        let stroke_response = ui.interact(stroke_button_rect, stroke_button_id, egui::Sense::click());

        // Draw stroke color button with checkerboard for alpha
        draw_color_button(ui, stroke_button_rect, *shared.stroke_color);

        if stroke_response.clicked() {
            // Open color picker popup
            ui.memory_mut(|mem| mem.toggle_popup(stroke_button_id));
        }

        // Show stroke color picker popup
        egui::popup::popup_below_widget(ui, stroke_button_id, &stroke_response, egui::popup::PopupCloseBehavior::CloseOnClickOutside, |ui: &mut egui::Ui| {
            let changed = egui::color_picker::color_picker_color32(ui, shared.stroke_color, egui::color_picker::Alpha::OnlyBlend);
            // Track that the user interacted with the stroke color
            if changed {
                *shared.active_color_mode = super::ColorMode::Stroke;
            }
        });
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
