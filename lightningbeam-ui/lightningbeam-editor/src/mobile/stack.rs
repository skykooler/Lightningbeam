//! The vertical sliding-window stack engine.
//!
//! A window of 2–3 consecutive panes (from [`super::STACK`]) is visible. Dragging the dividers
//! between visible panes and the top/bottom screen edges grows, shrinks, or slides the window via
//! the operations below (verified against the user's transition table):
//!
//! ```text
//! R1  upper divider, UP    -> slide window DOWN one   (top+=1)
//! R4  upper divider, DOWN  -> drop BOTTOM pane         (count 3->2)
//! R3  lower divider, UP    -> drop TOP pane            (top+=1, count 3->2)
//! R2  lower divider, DOWN  -> slide window UP one       (top-=1)
//! R5  bottom edge,  UP     -> grow at bottom; if already 3, slide down
//! R6  top edge,     DOWN   -> grow at top;    if already 3, slide up
//! ```
//!
//! At count==2 the single divider is "upper" when dragged up (R1) and "lower" when dragged
//! down (R2). The window content interpolates continuously during a drag and snaps on release.

use eframe::egui;

use super::{slot_path, MobileState, StackDrag, StackPane, STACK};
use crate::RenderContext;

const N: usize = STACK.len();

const EDGE_GRAB_H: f32 = 16.0;
const DIV_GRAB_H: f32 = 18.0;

const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);
const C_AMBER: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xa3, 0x40);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(0x7c, 0x86, 0x93);
const C_CHIP_BG: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x1b, 0x1f, 0x27, 0xcc);

/// A draggable boundary of the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    TopEdge,
    /// Divider below visible pane `k` (between visible pane `k` and `k+1`).
    Divider(usize),
    BottomEdge,
}

// --- R1..R6 as pure ops on (top, count) -> Option<(top, count)> ---

fn op_r1(top: usize, count: usize) -> Option<(usize, usize)> {
    (top + count < N).then_some((top + 1, count)) // slide down
}
fn op_r2(top: usize, count: usize) -> Option<(usize, usize)> {
    (top > 0).then_some((top - 1, count)) // slide up
}
fn op_r3(top: usize, count: usize) -> Option<(usize, usize)> {
    (count == 3).then_some((top + 1, 2)) // drop top
}
fn op_r4(top: usize, count: usize) -> Option<(usize, usize)> {
    (count == 3).then_some((top, 2)) // drop bottom
}
fn op_r5(top: usize, count: usize) -> Option<(usize, usize)> {
    if count < 3 && top + count < N {
        Some((top, count + 1)) // grow bottom
    } else if count == 3 && top + count < N {
        Some((top + 1, count)) // slide down
    } else {
        None
    }
}
fn op_r6(top: usize, count: usize) -> Option<(usize, usize)> {
    if count < 3 && top > 0 {
        Some((top - 1, count + 1)) // grow top
    } else if count == 3 && top > 0 {
        Some((top - 1, count)) // slide up
    } else {
        None
    }
}

/// Given a handle and signed drag offset, resolve the target window config and progress `t`.
/// Returns None if the drag direction has no valid operation (e.g. at a list boundary).
fn resolve(
    handle: Handle,
    offset: f32,
    top: usize,
    count: usize,
    pane_h: f32,
) -> Option<(usize, usize, f32)> {
    let going_up = offset < 0.0;
    let t = (offset.abs() / pane_h.max(1.0)).clamp(0.0, 1.0);
    let target = match handle {
        Handle::TopEdge => (!going_up).then(|| op_r6(top, count)).flatten(),
        Handle::BottomEdge => going_up.then(|| op_r5(top, count)).flatten(),
        Handle::Divider(k) => {
            if count == 2 {
                if going_up { op_r1(top, count) } else { op_r2(top, count) }
            } else if k == 0 {
                // upper divider
                if going_up { op_r1(top, count) } else { op_r4(top, count) }
            } else {
                // lower divider
                if going_up { op_r3(top, count) } else { op_r2(top, count) }
            }
        }
    };
    target.map(|(tt, tc)| (tt, tc, t))
}

// --- rect layout ---

fn config_rects(top: usize, count: usize, rect: egui::Rect) -> Vec<(usize, egui::Rect)> {
    let h = rect.height() / count as f32;
    (0..count)
        .map(|i| {
            let y0 = rect.top() + i as f32 * h;
            (
                top + i,
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), y0),
                    egui::pos2(rect.right(), y0 + h),
                ),
            )
        })
        .collect()
}

fn lerp_f(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn lerp_rect(a: egui::Rect, b: egui::Rect, t: f32) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(lerp_f(a.min.x, b.min.x, t), lerp_f(a.min.y, b.min.y, t)),
        egui::pos2(lerp_f(a.max.x, b.max.x, t), lerp_f(a.max.y, b.max.y, t)),
    )
}
fn collapsed(rect: egui::Rect, at_top: bool) -> egui::Rect {
    let y = if at_top { rect.top() } else { rect.bottom() };
    egui::Rect::from_min_max(egui::pos2(rect.left(), y), egui::pos2(rect.right(), y))
}

/// Interpolate the visible-pane rects from config C toward config T by progress `t`.
fn interp_rects(
    (top_c, count_c): (usize, usize),
    (top_t, count_t): (usize, usize),
    t: f32,
    rect: egui::Rect,
) -> Vec<(usize, egui::Rect)> {
    let c = config_rects(top_c, count_c, rect);
    let tt = config_rects(top_t, count_t, rect);
    let find = |v: &[(usize, egui::Rect)], slot: usize| v.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r);

    let lo = top_c.min(top_t);
    let hi = (top_c + count_c).max(top_t + count_t);
    let mut out = Vec::new();
    for slot in lo..hi {
        let in_c = find(&c, slot);
        let in_t = find(&tt, slot);
        let r = match (in_c, in_t) {
            (Some(rc), Some(rt)) => lerp_rect(rc, rt, t),
            (Some(rc), None) => lerp_rect(rc, collapsed(rect, slot < top_t), t), // exiting
            (None, Some(rt)) => lerp_rect(collapsed(rect, slot < top_c), rt, t), // entering
            (None, None) => continue,
        };
        out.push((slot, r));
    }
    out
}

// --- rendering ---

pub fn render(ui: &mut egui::Ui, rect: egui::Rect, rc: &mut RenderContext, state: &mut MobileState) {
    let top = state.window_top;
    let count = state.window_count;
    let pane_h = rect.height() / count as f32;

    // Layout from the (previous frame's) drag, if any has a valid op; else the resting config.
    let layout = state
        .drag
        .and_then(|d| resolve(d.handle, d.offset, top, count, pane_h))
        .map(|(tt, tc, t)| interp_rects((top, count), (tt, tc), t, rect))
        .unwrap_or_else(|| config_rects(top, count, rect));

    // 1) Pane content (top to bottom).
    for (slot, prect) in &layout {
        if prect.height() < 1.0 {
            continue;
        }
        let sp = STACK[*slot];
        super::surface::render_surface_fullbleed(
            ui,
            *prect,
            &slot_path(*slot),
            sp.pane_type(state.show_instruments),
            rc,
        );
    }

    // 2) Per-band label chips + the Node/Instrument toggle (drawn over content).
    for (slot, prect) in &layout {
        if prect.height() < 24.0 {
            continue;
        }
        draw_band_chip(ui, *prect, STACK[*slot], state);
    }

    // 3) Handles (interacted last so they win the initial press on their thin strips).
    handle_interactions(ui, rect, state);
}

fn draw_band_chip(ui: &mut egui::Ui, prect: egui::Rect, sp: StackPane, state: &mut MobileState) {
    let label = sp.label(state.show_instruments);
    let pos = prect.left_top() + egui::vec2(8.0, 6.0);
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(11.0),
        C_DIM,
    );
    let chip = egui::Rect::from_min_size(pos, galley.size() + egui::vec2(12.0, 5.0));
    ui.painter().rect_filled(chip, 4.0, C_CHIP_BG);
    ui.painter()
        .galley(chip.min + egui::vec2(6.0, 2.0), galley, C_DIM);

    // Node/Instrument toggle sits just to the right of the label chip.
    if sp == StackPane::NodeInstrument {
        let tog = egui::Rect::from_min_size(
            egui::pos2(chip.right() + 6.0, chip.top()),
            egui::vec2(24.0, chip.height()),
        );
        let resp = ui.interact(tog, ui.id().with("mobile_node_toggle"), egui::Sense::click());
        ui.painter().rect_filled(tog, 4.0, C_CHIP_BG);
        ui.painter().text(
            tog.center(),
            egui::Align2::CENTER_CENTER,
            "⇄",
            egui::FontId::proportional(13.0),
            if resp.hovered() { C_AMBER } else { C_DIM },
        );
        if resp.clicked() {
            state.show_instruments = !state.show_instruments;
        }
    }
}

fn handle_key(h: Handle) -> (usize, usize) {
    match h {
        Handle::TopEdge => (0, 0),
        Handle::Divider(k) => (1, k + 1),
        Handle::BottomEdge => (2, 0),
    }
}

fn handle_interactions(ui: &mut egui::Ui, rect: egui::Rect, state: &mut MobileState) {
    let count = state.window_count;
    let pane_h = rect.height() / count as f32;

    let mut handles: Vec<(Handle, egui::Rect)> = Vec::new();
    handles.push((
        Handle::TopEdge,
        egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + EDGE_GRAB_H)),
    ));
    for k in 0..count.saturating_sub(1) {
        let y = rect.top() + (k + 1) as f32 * pane_h;
        handles.push((
            Handle::Divider(k),
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y - DIV_GRAB_H * 0.5),
                egui::pos2(rect.right(), y + DIV_GRAB_H * 0.5),
            ),
        ));
    }
    handles.push((
        Handle::BottomEdge,
        egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - EDGE_GRAB_H), rect.max),
    ));

    for (handle, hrect) in handles {
        let id = ui.id().with(("mobile_stack_handle", handle_key(handle)));
        let resp = ui.interact(hrect, id, egui::Sense::drag());

        let active = state.drag.map(|d| d.handle == handle).unwrap_or(false);
        // Grab pill.
        let pill = egui::Rect::from_center_size(hrect.center(), egui::vec2(40.0, 4.0));
        let pill_col = if resp.hovered() || active { C_AMBER } else { C_LINE };
        ui.painter().rect_filled(pill, 2.0, pill_col);

        if resp.drag_started() {
            state.drag = Some(StackDrag { handle, offset: 0.0 });
        }
        if resp.dragged() {
            if let Some(d) = &mut state.drag {
                if d.handle == handle {
                    d.offset += resp.drag_delta().y;
                }
            }
        }
        if resp.drag_stopped() {
            if let Some(d) = state.drag.take() {
                commit_drag(d, state, pane_h);
            }
        }
        if resp.hovered() || active {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
}

fn commit_drag(d: StackDrag, state: &mut MobileState, pane_h: f32) {
    if let Some((tt, tc, t)) = resolve(d.handle, d.offset, state.window_top, state.window_count, pane_h) {
        if t >= 0.5 {
            state.window_top = tt;
            state.window_count = tc;
        }
    }
}
