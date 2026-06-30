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

use super::{icons, slot_path, MobileState, SnapAnim, StackDrag, StackPane, STACK};
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

/// Intermediate divider snap fractions (pane k's share of the {k, k+1} span).
const SNAP_FRACS: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];
/// Below/above these local fractions, a divider release collapses a pane → membership change.
const COLLAPSE_LO: f32 = 0.12;
const COLLAPSE_HI: f32 = 0.88;
/// Minimum normalized size a pane may be squeezed to during a live divider drag.
const MIN_FRAC: f32 = 0.05;
/// Duration (seconds) of the ease into a snapped divider position.
const SNAP_ANIM_SECS: f64 = 0.10;

fn ease_out(p: f32) -> f32 {
    1.0 - (1.0 - p).powi(3)
}

/// Normalized pane weights (the first `count` stored weights, summing to 1).
fn nweights(weights: &[f32; 3], count: usize) -> Vec<f32> {
    let mut w: Vec<f32> = weights[..count].iter().map(|x| x.max(0.0001)).collect();
    let s: f32 = w.iter().sum();
    if s > 0.0 {
        for x in &mut w {
            *x /= s;
        }
    } else {
        let e = 1.0 / count as f32;
        w.iter_mut().for_each(|x| *x = e);
    }
    w
}

/// Lay out `count` panes in `rect` using normalized weights `nw`.
fn config_rects(top: usize, count: usize, rect: egui::Rect, nw: &[f32]) -> Vec<(usize, egui::Rect)> {
    let h = rect.height();
    let mut out = Vec::with_capacity(count);
    let mut y = rect.top();
    for i in 0..count {
        let ph = nw[i] * h;
        out.push((
            top + i,
            egui::Rect::from_min_max(egui::pos2(rect.left(), y), egui::pos2(rect.right(), y + ph)),
        ));
        y += ph;
    }
    out
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

/// The local boundary fraction (pane k's share of the {k, k+1} span) after applying a normalized
/// `offset_frac` to divider `k`, clamped so neither pane is squeezed past MIN_FRAC.
fn divider_local(nw: &[f32], k: usize, offset_frac: f32) -> f32 {
    let span_lo: f32 = nw[..k].iter().sum();
    let span = nw[k] + nw[k + 1];
    if span <= 0.0 {
        return 0.5;
    }
    let base_b = span_lo + nw[k];
    let b = (base_b + offset_frac).clamp(span_lo + MIN_FRAC * span, span_lo + span - MIN_FRAC * span);
    ((b - span_lo) / span).clamp(0.0, 1.0)
}

/// Live pane rects while dragging divider `k` (resize-only; membership is unchanged until release).
fn divider_live(
    top: usize,
    count: usize,
    rect: egui::Rect,
    nw: &[f32],
    k: usize,
    offset_frac: f32,
) -> Vec<(usize, egui::Rect)> {
    let f = divider_local(nw, k, offset_frac);
    let span = nw[k] + nw[k + 1];
    let mut w = nw.to_vec();
    w[k] = f * span;
    w[k + 1] = (1.0 - f) * span;
    config_rects(top, count, rect, &w)
}

/// Interpolate between two precomputed rect lists by `t` (used for edge membership transitions).
fn interp_layout(
    c: &[(usize, egui::Rect)],
    tt: &[(usize, egui::Rect)],
    top_c: usize,
    top_t: usize,
    t: f32,
    rect: egui::Rect,
) -> Vec<(usize, egui::Rect)> {
    let find = |v: &[(usize, egui::Rect)], slot: usize| v.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r);
    let lo = c.first().map(|(s, _)| *s).unwrap_or(0).min(tt.first().map(|(s, _)| *s).unwrap_or(0));
    let hi = c.last().map(|(s, _)| *s + 1).unwrap_or(0).max(tt.last().map(|(s, _)| *s + 1).unwrap_or(0));
    let mut out = Vec::new();
    for slot in lo..hi {
        let r = match (find(c, slot), find(tt, slot)) {
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

    // Resting band rects (weighted; used for interaction so drags don't chase the animated layout).
    // While a snap ease is in flight (and we're not dragging), override the weights with the eased
    // values so the boundary glides into place.
    let now = ui.input(|i| i.time);
    let mut nw = nweights(&state.weights, count);
    if state.drag.is_none() {
        if let Some(a) = state.snap_anim {
            let p = ((now - a.start) / SNAP_ANIM_SECS).clamp(0.0, 1.0) as f32;
            if p >= 1.0 {
                state.snap_anim = None;
            } else {
                let e = ease_out(p);
                nw = (0..count).map(|i| lerp_f(a.from[i], a.to[i], e)).collect();
                ui.ctx().request_repaint();
            }
        }
    }
    let rest_bands = config_rects(top, count, content_area, &nw);

    // The draw layout. A divider drag resizes its two panes live; an edge drag animates toward the
    // membership transition.
    let draw_layout = match state.drag {
        Some(d) => match d.handle {
            Handle::Divider(k) if k + 1 < count => {
                let off_frac = d.offset / content_area.height().max(1.0);
                divider_live(top, count, content_area, &nw, k, off_frac)
            }
            _ => resolve(d.handle, d.offset, top, count, trigger)
                .map(|(tt, tc, t)| {
                    let even = vec![1.0 / tc as f32; tc];
                    let target = config_rects(tt, tc, content_area, &even);
                    interp_layout(&rest_bands, &target, top, tt, t, content_area)
                })
                .unwrap_or_else(|| rest_bands.clone()),
        },
        None => rest_bands.clone(),
    };

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
    handle_interactions(ui, &rest_bands, content_area, footer_rect, trigger, now, state);
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
            set_window(state, t, c);
        }
    } else {
        set_window(state, slot, 1);
    }
}

fn handle_interactions(
    ui: &mut egui::Ui,
    rest_bands: &[(usize, egui::Rect)],
    content_area: egui::Rect,
    footer_rect: egui::Rect,
    trigger: f32,
    now: f64,
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
            state.snap_anim = None; // a fresh drag cancels any in-flight snap ease
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
                commit_drag(d, state, content_area, trigger, now);
            }
        }
        if resp.hovered() || state.drag.map(|d| d.handle == handle).unwrap_or(false) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
    }
}

fn commit_drag(d: StackDrag, state: &mut MobileState, content_area: egui::Rect, trigger: f32, now: f64) {
    match d.handle {
        Handle::Divider(k) if k + 1 < state.window_count => {
            commit_divider(state, k, d.offset / content_area.height().max(1.0), now);
        }
        _ => {
            // Edges (and degenerate dividers): membership transition, then reset to even sizing.
            if let Some((tt, tc, t)) =
                resolve(d.handle, d.offset, state.window_top, state.window_count, trigger)
            {
                if t >= 0.5 {
                    set_window(state, tt, tc);
                }
            }
        }
    }
}

/// Apply a divider release: collapse a pane (→ membership change) past the thresholds, otherwise
/// snap the boundary to the nearest intermediate fraction.
fn commit_divider(state: &mut MobileState, k: usize, offset_frac: f32, now: f64) {
    let top = state.window_top;
    let count = state.window_count;
    let nw = nweights(&state.weights, count);
    let f = divider_local(&nw, k, offset_frac);

    if f <= COLLAPSE_LO {
        // Pane k squeezed out at the top.
        let op = if k == 0 { op_r1(top, count) } else { op_r3(top, count) };
        if let Some((tt, tc)) = op {
            set_window(state, tt, tc);
        }
    } else if f >= COLLAPSE_HI {
        // Pane k+1 squeezed out at the bottom.
        let op = if k == count - 2 { op_r2(top, count) } else { op_r4(top, count) };
        if let Some((tt, tc)) = op {
            set_window(state, tt, tc);
        }
    } else {
        // Snap the boundary to the nearest intermediate fraction, preserving the {k, k+1} span and
        // the other panes' sizes — and ease into it.
        let sf = *SNAP_FRACS
            .iter()
            .min_by(|a, b| (**a - f).abs().total_cmp(&(**b - f).abs()))
            .unwrap();
        let span = nw[k] + nw[k + 1];

        // `from` = where the finger released; `to` = the snapped target.
        let mut from = [0.0_f32; 3];
        let mut to = [0.0_f32; 3];
        for i in 0..count {
            from[i] = nw[i];
            to[i] = nw[i];
        }
        from[k] = f * span;
        from[k + 1] = (1.0 - f) * span;
        to[k] = sf * span;
        to[k + 1] = (1.0 - sf) * span;

        state.weights = to;
        state.snap_anim = Some(SnapAnim { from, to, start: now });
    }
}

/// Switch the visible window and reset to even pane sizing.
fn set_window(state: &mut MobileState, top: usize, count: usize) {
    state.window_top = top;
    state.window_count = count;
    state.weights = [1.0, 1.0, 1.0];
    state.snap_anim = None;
}
