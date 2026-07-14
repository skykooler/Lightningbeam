//! Shared color swatch + picker popup used by the toolbar FG/BG swatches and the
//! gradient-stop editor.
//!
//! egui's default popup close behavior is `CloseOnClick`, which closes on a click *anywhere* —
//! including on the picker's own hue/alpha bars. And `CloseOnClickOutside` only reacts to a
//! click (press+release without movement), so painting a stroke on the stage — a drag — would
//! leave the popup open. Both call sites want the same thing, so it lives here once.

use eframe::egui;

/// Show a color-picker popup anchored to `toggle_response`.
///
/// `swatch_rect` is excluded from the close-on-press check so the press that opens the popup
/// doesn't immediately close it again.
///
/// Returns true if the color changed this frame.
pub fn color_picker_popup(
    ui: &mut egui::Ui,
    popup_id: egui::Id,
    toggle_response: &egui::Response,
    color: &mut egui::Color32,
    swatch_rect: egui::Rect,
) -> bool {
    let mut changed = false;

    let popup_response = egui::containers::Popup::from_toggle_button_response(toggle_response)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = 275.0;
            changed = egui::color_picker::color_picker_color32(
                ui,
                color,
                egui::color_picker::Alpha::OnlyBlend,
            );
        });

    // Close on any pointer *press* outside the popup, not just a click, so that starting a
    // brush stroke on the stage dismisses the picker.
    if let Some(popup) = &popup_response {
        let popup_rect = popup.response.rect;
        let pressed_outside = ui.ctx().input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| !popup_rect.contains(p) && !swatch_rect.contains(p))
        });
        if pressed_outside {
            egui::Popup::close_id(ui.ctx(), popup_id);
        }
    }

    changed
}

/// A color button (checkerboard under the color, so alpha reads correctly) that opens a color
/// picker popup when clicked. Returns true if the color changed this frame.
pub fn color_swatch(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    color: &mut egui::Color32,
) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    draw_color_button(ui, rect, *color);
    color_picker_popup(ui, id.with("picker_popup"), &response, color, rect)
}

/// Draw a color button with a checkerboard background so the alpha channel is visible.
pub fn draw_color_button(ui: &mut egui::Ui, rect: egui::Rect, color: egui::Color32) {
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
            )
            .intersect(rect);
            ui.painter().rect_filled(checker_rect, 0.0, checker_color);
        }
    }

    ui.painter().rect_filled(rect, 2.0, color);
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
        egui::StrokeKind::Middle,
    );
}
