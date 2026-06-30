//! Resizable timeline ribbon. A grabber strip drags the ribbon through three snap tiers
//! (Peek / Half / Full); the ribbon body reuses the existing `TimelinePane` full-bleed.
//! The transport floor below it is always present, so the ribbon never collapses to zero.

use eframe::egui;
use lightningbeam_core::pane::PaneType;

use super::{surface, MobileState, RibbonTier, MOBILE_NS};
use crate::panes::NodePath;
use crate::RenderContext;

const C_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x24, 0x2c);
const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);

/// Stable `pane_instances` key for the ribbon's timeline — distinct from the Time surface so
/// their zoom/scroll state don't fight.
fn ribbon_path() -> NodePath {
    vec![MOBILE_NS, 100]
}

pub fn render(
    ui: &mut egui::Ui,
    grabber_rect: egui::Rect,
    ribbon_body: egui::Rect,
    region_h: f32,
    state: &mut MobileState,
    rc: &mut RenderContext,
) {
    // Grabber strip.
    let painter = ui.painter_at(grabber_rect);
    painter.rect_filled(grabber_rect, 0.0, C_PANEL);
    painter.hline(grabber_rect.x_range(), grabber_rect.top(), egui::Stroke::new(1.0, C_LINE));
    // Handle pill.
    let pill = egui::Rect::from_center_size(grabber_rect.center(), egui::vec2(34.0, 4.0));
    painter.rect_filled(pill, 2.0, C_LINE);

    let resp = ui.interact(
        grabber_rect,
        ui.id().with("mobile_ribbon_grabber"),
        egui::Sense::drag(),
    );
    if resp.dragged() {
        // Dragging up (negative y) expands the ribbon.
        state.ribbon_drag -= resp.drag_delta().y;
    }
    if resp.drag_stopped() {
        let target_h = state.ribbon_tier.height(region_h) + state.ribbon_drag;
        state.ribbon_tier = RibbonTier::snap_from(region_h, target_h);
        state.ribbon_drag = 0.0;
    }
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    // Ribbon body = the timeline, full-bleed.
    surface::render_surface_fullbleed(ui, ribbon_body, &ribbon_path(), PaneType::Timeline, rc);
}
