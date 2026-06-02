//! Full-width selectable list for use inside popups and dropdowns.
//!
//! Solves the recurring issue where `selectable_label` inside `ScrollArea`
//! inside a `Popup` doesn't fill the available width, making only the text
//! portion clickable.

use eframe::egui;
use egui::Ui;

/// Render a full-width selectable list item.
///
/// Unlike `ui.selectable_label()`, this allocates the full available width
/// for the clickable area, matching native menu item behavior.
pub fn list_item(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let desired_width = ui.available_width();
    let height = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(desired_width, height),
        egui::Sense::click(),
    );

    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if selected {
            ui.painter().rect_filled(rect, 2.0, visuals.selection.bg_fill);
        } else if response.hovered() {
            ui.painter().rect_filled(rect, 2.0, visuals.widgets.hovered.bg_fill);
        }

        let text_color = if selected {
            visuals.selection.stroke.color
        } else if response.hovered() {
            visuals.widgets.hovered.text_color()
        } else {
            visuals.widgets.inactive.text_color()
        };

        let text_pos = rect.min + egui::vec2(4.0, (rect.height() - 14.0) / 2.0);
        ui.painter().text(
            text_pos,
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(14.0),
            text_color,
        );
    }

    response.clicked()
}

/// Render a scrollable list of items inside a popup, ensuring full-width
/// clickable areas and proper ScrollArea sizing.
///
/// Returns the index of the clicked item, if any.
pub fn scrollable_list<'a>(
    ui: &mut Ui,
    max_height: f32,
    items: impl Iterator<Item = (bool, &'a str)>,
) -> Option<usize> {
    let mut clicked_index = None;

    // Force the ScrollArea to use the full width set by the parent
    let width = ui.available_width();

    egui::ScrollArea::vertical()
        .max_height(max_height)
        .show(ui, |ui| {
            ui.set_min_width(width);
            for (i, (selected, label)) in items.enumerate() {
                if list_item(ui, selected, label) {
                    clicked_index = Some(i);
                }
            }
        });

    clicked_index
}
