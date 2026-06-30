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

use super::{icons, slot_path, MobileState, StackDrag, StackPane, STACK};
use crate::RenderContext;

const N: usize = STACK.len();

/// Height of each band's drag header (and the bottom-edge footer). The whole header is the grab
/// target — there's no thin divider bar.
const HEADER_H: f32 = 52.0;
/// Corner radius (px) for the rounded top of each header.
const HEADER_RADIUS: u8 = 9;
const FOOTER_H: f32 = 28.0;
/// Width of a header right-side button (fullscreen, node toggle).
const BTN_W: f32 = 44.0;
/// Max pixels of drag to complete a window transition (so you don't drag half the screen).
const TRIGGER_MAX: f32 = 150.0;

const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);
const C_AMBER: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xa3, 0x40);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(0x7c, 0x86, 0x93);
const C_BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xea, 0xee, 0xf3);
const C_HEADER: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x24, 0x2c);

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
    (top + count < N).then(|| (top + 1, count)) // slide down
}
fn op_r2(top: usize, count: usize) -> Option<(usize, usize)> {
    (top > 0).then(|| (top - 1, count)) // slide up
}
fn op_r3(top: usize, count: usize) -> Option<(usize, usize)> {
    (count == 3).then(|| (top + 1, 2)) // drop top
}
fn op_r4(top: usize, count: usize) -> Option<(usize, usize)> {
    (count == 3).then(|| (top, 2)) // drop bottom
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
    trigger: f32,
) -> Option<(usize, usize, f32)> {
    let going_up = offset < 0.0;
    let t = (offset.abs() / trigger.max(1.0)).clamp(0.0, 1.0);
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

/// The interaction (drag) target for a band's boundary is its full-width header bar. Bands are
/// laid out in the area above a reserved bottom footer (the BottomEdge handle). Band 0's header is
/// the TopEdge handle; band i's header (i≥1) is the divider above it; the footer is the BottomEdge.
pub fn render(ui: &mut egui::Ui, rect: egui::Rect, rc: &mut RenderContext, state: &mut MobileState) {
    let top = state.window_top;
    let count = state.window_count;

    // Reserve a footer bar at the very bottom for the BottomEdge handle.
    let footer_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left(), rect.bottom() - FOOTER_H),
        rect.max,
    );
    let content_area = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.right(), footer_rect.top()),
    );
    let pane_h = content_area.height() / count as f32;
    let trigger = pane_h.min(TRIGGER_MAX);

    // Resting band rects (used for interaction so drags don't chase the animated layout) and the
    // possibly-animated draw layout.
    let rest_bands = config_rects(top, count, content_area);
    let draw_layout = state
        .drag
        .and_then(|d| resolve(d.handle, d.offset, top, count, trigger))
        .map(|(tt, tc, t)| interp_rects((top, count), (tt, tc), t, content_area))
        .unwrap_or_else(|| rest_bands.clone());

    // 1) Pane content, carving the header off the top of each band.
    for (slot, brect) in &draw_layout {
        let header_h = HEADER_H.min(brect.height());
        let content_rect = egui::Rect::from_min_max(
            egui::pos2(brect.left(), brect.top() + header_h),
            brect.max,
        );
        if content_rect.height() > 1.0 {
            let sp = STACK[*slot];
            super::surface::render_surface_fullbleed(
                ui,
                content_rect,
                &slot_path(*slot),
                sp.pane_type(state.show_instruments),
                rc,
            );
        }
    }

    // 2) Header visuals (animated positions). The fullscreen icon shows "restore" when a single
    // pane fills the stack.
    let fullscreen = count == 1;
    for (slot, brect) in &draw_layout {
        if brect.height() < 6.0 {
            continue;
        }
        let hr = egui::Rect::from_min_max(
            brect.left_top(),
            egui::pos2(brect.right(), brect.top() + HEADER_H.min(brect.height())),
        );
        draw_header(ui, hr, STACK[*slot], state.show_instruments, fullscreen);
    }
    draw_footer(ui, footer_rect, top + count >= N);

    // 3) Interactions on the resting header/footer rects (added last → they win the press).
    handle_interactions(ui, &rest_bands, footer_rect, trigger, state);
}

fn draw_header(ui: &egui::Ui, hr: egui::Rect, sp: StackPane, show_instruments: bool, fullscreen: bool) {
    let p = ui.painter();
    // Rounded top corners so the header reads as a tab atop the pane; square at the bottom where
    // it meets the pane content.
    p.rect_filled(
        hr,
        egui::CornerRadius { nw: HEADER_RADIUS, ne: HEADER_RADIUS, sw: 0, se: 0 },
        C_HEADER,
    );
    p.hline(hr.x_range(), hr.bottom(), egui::Stroke::new(1.0, C_LINE));
    let cy = hr.center().y;
    // Grip glyph on the left (Lucide).
    p.text(
        egui::pos2(hr.left() + 15.0, cy),
        egui::Align2::CENTER_CENTER,
        icons::GRIP_HORIZONTAL,
        icons::font(16.0),
        C_DIM,
    );
    p.text(
        egui::pos2(hr.left() + 32.0, cy),
        egui::Align2::LEFT_CENTER,
        sp.label(show_instruments),
        egui::FontId::proportional(15.0),
        C_BRIGHT,
    );
    // Right-side buttons: fullscreen / restore rightmost, Node/Instrument toggle just left of it.
    p.text(
        egui::pos2(hr.right() - BTN_W * 0.5, cy),
        egui::Align2::CENTER_CENTER,
        if fullscreen { icons::MINIMIZE } else { icons::MAXIMIZE },
        icons::font(17.0),
        C_DIM,
    );
    if sp == StackPane::NodeInstrument {
        p.text(
            egui::pos2(hr.right() - BTN_W * 1.5, cy),
            egui::Align2::CENTER_CENTER,
            icons::ARROW_LEFT_RIGHT,
            icons::font(17.0),
            C_AMBER,
        );
    }
}

fn draw_footer(ui: &egui::Ui, fr: egui::Rect, at_end: bool) {
    let p = ui.painter();
    p.rect_filled(fr, 0.0, C_HEADER);
    p.hline(fr.x_range(), fr.top(), egui::Stroke::new(1.0, C_LINE));
    let col = if at_end { C_LINE } else { C_DIM };
    let cy = fr.center().y;
    let galley = p.layout_no_wrap(
        "pull up for more".to_string(),
        egui::FontId::proportional(11.0),
        col,
    );
    let total_w = 22.0 + galley.size().x;
    let start_x = fr.center().x - total_w * 0.5;
    p.text(
        egui::pos2(start_x + 8.0, cy),
        egui::Align2::CENTER_CENTER,
        icons::CHEVRONS_UP,
        icons::font(14.0),
        col,
    );
    let gy = cy - galley.size().y * 0.5;
    p.galley(egui::pos2(start_x + 22.0, gy), galley, col);
}

fn handle_key(h: Handle) -> (usize, usize) {
    match h {
        Handle::TopEdge => (0, 0),
        Handle::Divider(k) => (1, k + 1),
        Handle::BottomEdge => (2, 0),
    }
}

/// Toggle a slot between filling the stack (count==1) and a 2-pane split with an adjacent pane.
fn toggle_fullscreen(state: &mut MobileState, slot: usize) {
    if state.window_count == 1 && state.window_top == slot {
        // Restore: split with the pane below if possible, else above.
        if let Some((t, c)) = op_r5(slot, 1).or_else(|| op_r6(slot, 1)) {
            state.window_top = t;
            state.window_count = c;
        }
    } else {
        state.window_top = slot;
        state.window_count = 1;
    }
}

fn handle_interactions(
    ui: &mut egui::Ui,
    rest_bands: &[(usize, egui::Rect)],
    footer_rect: egui::Rect,
    trigger: f32,
    state: &mut MobileState,
) {
    // (handle, header_rect, slot) — slot is Some for band headers, None for the footer.
    let mut handles: Vec<(Handle, egui::Rect, Option<usize>)> = Vec::new();
    for (i, (slot, brect)) in rest_bands.iter().enumerate() {
        let hr = egui::Rect::from_min_max(
            brect.left_top(),
            egui::pos2(brect.right(), brect.top() + HEADER_H.min(brect.height())),
        );
        let handle = if i == 0 { Handle::TopEdge } else { Handle::Divider(i - 1) };
        handles.push((handle, hr, Some(*slot)));
    }
    handles.push((Handle::BottomEdge, footer_rect, None));

    for (handle, hrect, slot_opt) in handles {
        let id = ui.id().with(("mobile_stack_handle", handle_key(handle)));
        let resp = ui.interact(hrect, id, egui::Sense::click_and_drag());

        // Right-side header buttons are interacted AFTER the header (so they're on top and win the
        // press there); the rest of the header drives the drag.
        if let Some(slot) = slot_opt {
            let fs = egui::Rect::from_min_max(
                egui::pos2(hrect.right() - BTN_W, hrect.top()),
                hrect.max,
            );
            let fsresp = ui.interact(fs, ui.id().with(("mobile_fs", slot)), egui::Sense::click());
            if fsresp.clicked() {
                toggle_fullscreen(state, slot);
            }
            if STACK[slot] == StackPane::NodeInstrument {
                let nt = egui::Rect::from_min_max(
                    egui::pos2(hrect.right() - 2.0 * BTN_W, hrect.top()),
                    egui::pos2(hrect.right() - BTN_W, hrect.bottom()),
                );
                let ntresp = ui.interact(nt, ui.id().with("mobile_node_toggle"), egui::Sense::click());
                if ntresp.clicked() {
                    state.show_instruments = !state.show_instruments;
                }
            }
        }

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
                commit_drag(d, state, trigger);
            }
        }
        if resp.hovered() || state.drag.map(|d| d.handle == handle).unwrap_or(false) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
}

fn commit_drag(d: StackDrag, state: &mut MobileState, trigger: f32) {
    if let Some((tt, tc, t)) = resolve(d.handle, d.offset, state.window_top, state.window_count, trigger) {
        if t >= 0.5 {
            state.window_top = tt;
            state.window_count = tc;
        }
    }
}
