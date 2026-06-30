//! Top surface-tab bar. One tap switches the hero surface. Mirrors the wireframe's top tabs
//! (Stage / Time / Nodes / Mixer / Tree); the active tab is marked with an amber underline.

use eframe::egui;

use super::{MobileState, MobileSurface};
use crate::RenderContext;

// Wireframe palette.
const C_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x24, 0x2c);
const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);
const C_AMBER: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xa3, 0x40);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(0x7c, 0x86, 0x93);
const C_BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xea, 0xee, 0xf3);

pub fn render(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &mut MobileState,
    _rc: &mut RenderContext,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, C_PANEL);
    // Bottom rule.
    painter.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, C_LINE));

    let tabs = MobileSurface::TABS;
    let tab_w = rect.width() / tabs.len() as f32;

    for (i, surface) in tabs.iter().enumerate() {
        let tab_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + i as f32 * tab_w, rect.top()),
            egui::pos2(rect.left() + (i as f32 + 1.0) * tab_w, rect.bottom()),
        );
        let id = ui.id().with(("mobile_tab", i));
        let resp = ui.interact(tab_rect, id, egui::Sense::click());
        if resp.clicked() {
            state.active_surface = *surface;
        }

        let active = state.active_surface == *surface;
        let color = if active {
            C_AMBER
        } else if resp.hovered() {
            C_BRIGHT
        } else {
            C_DIM
        };
        painter.text(
            tab_rect.center(),
            egui::Align2::CENTER_CENTER,
            surface.label(),
            egui::FontId::proportional(13.0),
            color,
        );
        if active {
            // Underline marking the active surface.
            let y = tab_rect.bottom() - 1.5;
            painter.hline(
                (tab_rect.left() + 10.0)..=(tab_rect.right() - 10.0),
                y,
                egui::Stroke::new(2.0, C_AMBER),
            );
        }
        // Vertical separators between tabs.
        if i > 0 {
            painter.vline(tab_rect.left(), rect.y_range(), egui::Stroke::new(1.0, C_LINE));
        }
    }
}
