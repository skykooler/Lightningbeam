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

use super::{icons, slot_path, LayoutAnim, MobileState, Palette, StackDrag, StackPane, STACK};
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

/// Below/above these boundary positions, a divider release collapses a group → membership change.
const COLLAPSE_LO: f32 = 0.12;
const COLLAPSE_HI: f32 = 0.88;

// The only allowed pane-weight distributions per window size. Snapping to these keeps the minimum
// pane height manageable (no pane below ~1/4).
const PRESETS_1: [[f32; 3]; 1] = [[1.0, 0.0, 0.0]];
const PRESETS_2: [[f32; 3]; 3] = [[0.25, 0.75, 0.0], [0.5, 0.5, 0.0], [0.75, 0.25, 0.0]];
const PRESETS_3: [[f32; 3]; 3] = [
    [0.5, 0.25, 0.25],
    [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0],
    [0.25, 0.25, 0.5],
];

fn presets(count: usize) -> &'static [[f32; 3]] {
    match count {
        2 => &PRESETS_2,
        3 => &PRESETS_3,
        _ => &PRESETS_1,
    }
}

/// The allowed preset nearest (L2) to the current weights.
fn nearest_preset(cur: &[f32], count: usize) -> [f32; 3] {
    let dist = |p: &[f32; 3]| (0..count).map(|i| (p[i] - cur[i]).powi(2)).sum::<f32>();
    presets(count)
        .iter()
        .min_by(|a, b| dist(a).total_cmp(&dist(b)))
        .copied()
        .unwrap_or([1.0, 0.0, 0.0])
}

// 3-pane snapping rides a single path parameter s in [0, 2] through the three presets, so the two
// dividers move together and reach their snaps simultaneously. The dragged divider's boundary
// position (pane0 for the upper divider; pane0+pane1 for the lower) maps onto s.
const PATH3_UPPER: [f32; 3] = [0.5, 1.0 / 3.0, 0.25]; // pane0 at s = 0, 1, 2
const PATH3_LOWER: [f32; 3] = [0.75, 2.0 / 3.0, 0.5]; // pane0+pane1 at s = 0, 1, 2
/// A pane grown beyond this collapses the 3-pane window to 2 panes on release.
const COLLAPSE_GROW: f32 = 0.66;

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp_f(a[0], b[0], t),
        lerp_f(a[1], b[1], t),
        lerp_f(a[2], b[2], t),
    ]
}

/// Weights along the 3-pane preset path at parameter `s` ∈ [0, 2].
fn path3(s: f32) -> [f32; 3] {
    let s = s.clamp(0.0, 2.0);
    if s <= 1.0 {
        lerp3(PRESETS_3[0], PRESETS_3[1], s)
    } else {
        lerp3(PRESETS_3[1], PRESETS_3[2], s - 1.0)
    }
}

/// Path parameter for divider `k` whose boundary position is `b` (within the path's range).
fn s_from_boundary(k: usize, b: f32) -> f32 {
    let pts = if k == 0 { PATH3_UPPER } else { PATH3_LOWER };
    if b >= pts[1] {
        (pts[0] - b) / (pts[0] - pts[1]) // 0 at pts[0] … 1 at pts[1]
    } else {
        1.0 + (pts[1] - b) / (pts[1] - pts[2]) // 1 at pts[1] … 2 at pts[2]
    }
}

/// The 3-pane path boundary range for divider `k`: (min, max) of its position across the presets.
fn path3_range(k: usize) -> (f32, f32) {
    let pts = if k == 0 { PATH3_UPPER } else { PATH3_LOWER };
    (pts[2], pts[0])
}

/// Live weights while dragging divider `k` to boundary `b` in a 3-pane window: follow the linked
/// path within range, and beyond it group-resize from the nearer path endpoint (so the motion is
/// continuous across the extreme presets — no jump in the other divider).
fn live_weights3(k: usize, b_unclamped: f32) -> [f32; 3] {
    let (lo, hi) = path3_range(k);
    if b_unclamped >= lo && b_unclamped <= hi {
        path3(s_from_boundary(k, b_unclamped))
    } else {
        let anchor = if b_unclamped > hi { path3(0.0) } else { path3(2.0) };
        let (bmin, bmax) = boundary_bounds(3, k);
        to_arr(&weights_for_boundary(&anchor, k, b_unclamped.clamp(bmin, bmax)))
    }
}
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

/// The bottom Y of the given stack slot's band within `region` (using the current window +
/// weights), or None if that slot isn't in the visible window. Used to decide whether the
/// inspector sheet would cover the selected pane.
#[allow(dead_code)] // kept for pane-coverage heuristics
pub fn pane_bottom_in(state: &MobileState, region: egui::Rect, slot: usize) -> Option<f32> {
    let (top, count) = (state.window_top, state.window_count);
    if slot < top || slot >= top + count {
        return None;
    }
    let nw = nweights(&state.weights, count);
    let cum: f32 = nw[..=(slot - top)].iter().sum();
    Some(region.top() + cum * region.height())
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

/// Allowed range of divider `k`'s boundary position (normalized) given MIN_FRAC per pane.
/// The boundary splits the window into the group above (panes 0..=k) and below (panes k+1..).
fn boundary_bounds(count: usize, k: usize) -> (f32, f32) {
    let above = (k + 1) as f32;
    let below = (count - 1 - k) as f32;
    (above * MIN_FRAC, 1.0 - below * MIN_FRAC)
}

/// New normalized weights when divider `k` is moved to boundary position `b`: the panes above the
/// divider are scaled to fill `[0, b]` and those below to fill `[b, 1]`, each group keeping its
/// internal proportions. This makes the *other* dividers in the group move with the dragged one.
fn weights_for_boundary(nw: &[f32], k: usize, b: f32) -> Vec<f32> {
    let count = nw.len();
    let above_sum: f32 = nw[..=k].iter().sum();
    let below_sum: f32 = nw[k + 1..].iter().sum();
    let mut w = nw.to_vec();
    if above_sum > 0.0 {
        let s = b / above_sum;
        for x in &mut w[..=k] {
            *x *= s;
        }
    }
    if below_sum > 0.0 {
        let s = (1.0 - b) / below_sum;
        for x in &mut w[k + 1..count] {
            *x *= s;
        }
    }
    w
}

fn to_arr(v: &[f32]) -> [f32; 3] {
    let mut a = [0.0; 3];
    for (i, x) in v.iter().take(3).enumerate() {
        a[i] = *x;
    }
    a
}

fn even_arr(count: usize) -> [f32; 3] {
    let mut a = [0.0; 3];
    let e = 1.0 / count as f32;
    for x in a.iter_mut().take(count) {
        *x = e;
    }
    a
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
pub fn render(ui: &mut egui::Ui, rect: egui::Rect, rc: &mut RenderContext, state: &mut MobileState, pal: &Palette) {
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

    let now = ui.input(|i| i.time);
    // Resting band rects from the committed weights — used for interaction so handles sit at their
    // final positions even while the visuals animate.
    let nw = nweights(&state.weights, count);
    let rest_bands = config_rects(top, count, content_area, &nw);

    // The draw layout: a live drag, an in-flight layout ease, or the resting config.
    let draw_layout = if let Some(d) = state.drag {
        match d.handle {
            Handle::Divider(k) if k + 1 < count => {
                // Live resize: move divider k's boundary. In 3-pane the dividers are linked along the
                // preset path; in 2-pane it's a simple group resize.
                let off_frac = d.offset / content_area.height().max(1.0);
                let b0: f32 = nw[..=k].iter().sum();
                let b = b0 + off_frac;
                let w = if count == 3 {
                    live_weights3(k, b)
                } else {
                    let (bmin, bmax) = boundary_bounds(count, k);
                    to_arr(&weights_for_boundary(&nw, k, b.clamp(bmin, bmax)))
                };
                config_rects(top, count, content_area, &nweights(&w, count))
            }
            _ => resolve(d.handle, d.offset, top, count, trigger)
                .map(|(tt, tc, t)| {
                    let target = config_rects(tt, tc, content_area, &nweights(&even_arr(tc), tc));
                    interp_layout(&rest_bands, &target, top, tt, t, content_area)
                })
                .unwrap_or_else(|| rest_bands.clone()),
        }
    } else if let Some(a) = state.anim {
        let p = (((now - a.start) / SNAP_ANIM_SECS) as f32).clamp(0.0, 1.0);
        if p >= 1.0 {
            state.anim = None;
            rest_bands.clone()
        } else {
            ui.ctx().request_repaint();
            let e = ease_out(p);
            let from = config_rects(a.from_top, a.from_count, content_area, &nweights(&a.from_w, a.from_count));
            let to = config_rects(a.to_top, a.to_count, content_area, &nweights(&a.to_w, a.to_count));
            interp_layout(&from, &to, a.from_top, a.to_top, e, content_area)
        }
    } else {
        rest_bands.clone()
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
        draw_header(ui, hr, STACK[*slot], state.show_instruments, fullscreen, pal);
    }
    draw_footer(ui, footer_rect, top + count >= N, pal);

    // 3) Interactions on the resting header/footer rects (added last → they win the press).
    handle_interactions(ui, &rest_bands, content_area, footer_rect, trigger, now, state);
}

fn draw_header(ui: &egui::Ui, hr: egui::Rect, sp: StackPane, show_instruments: bool, fullscreen: bool, pal: &Palette) {
    let p = ui.painter();
    // Rounded top corners so the header reads as a tab atop the pane; square at the bottom where
    // it meets the pane content.
    p.rect_filled(
        hr,
        egui::CornerRadius { nw: HEADER_RADIUS, ne: HEADER_RADIUS, sw: 0, se: 0 },
        pal.header,
    );
    p.hline(hr.x_range(), hr.bottom(), egui::Stroke::new(1.0, pal.line));
    let cy = hr.center().y;
    // Grip glyph on the left (Lucide).
    p.text(
        egui::pos2(hr.left() + 15.0, cy),
        egui::Align2::CENTER_CENTER,
        icons::GRIP_HORIZONTAL,
        icons::font(16.0),
        pal.text_dim,
    );
    p.text(
        egui::pos2(hr.left() + 32.0, cy),
        egui::Align2::LEFT_CENTER,
        sp.label(show_instruments),
        egui::FontId::proportional(15.0),
        pal.text,
    );
    // Right-side buttons: fullscreen / restore rightmost, Node/Instrument toggle just left of it.
    p.text(
        egui::pos2(hr.right() - BTN_W * 0.5, cy),
        egui::Align2::CENTER_CENTER,
        if fullscreen { icons::MINIMIZE } else { icons::MAXIMIZE },
        icons::font(17.0),
        pal.text_dim,
    );
    if sp == StackPane::NodeInstrument {
        p.text(
            egui::pos2(hr.right() - BTN_W * 1.5, cy),
            egui::Align2::CENTER_CENTER,
            icons::ARROW_LEFT_RIGHT,
            icons::font(17.0),
            pal.accent,
        );
    }
}

fn draw_footer(ui: &egui::Ui, fr: egui::Rect, at_end: bool, pal: &Palette) {
    let p = ui.painter();
    p.rect_filled(fr, 0.0, pal.header);
    p.hline(fr.x_range(), fr.top(), egui::Stroke::new(1.0, pal.line));
    let col = if at_end { pal.line } else { pal.text_dim };
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
fn toggle_fullscreen(state: &mut MobileState, slot: usize, now: f64) {
    if state.window_count == 1 && state.window_top == slot {
        // Restore: split with the pane below if possible, else above.
        if let Some((t, c)) = op_r5(slot, 1).or_else(|| op_r6(slot, 1)) {
            set_window(state, t, c, now);
        }
    } else {
        set_window(state, slot, 1, now);
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
                toggle_fullscreen(state, slot, now);
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
            state.anim = None; // a fresh drag cancels any in-flight ease
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
    let h = content_area.height().max(1.0);
    match d.handle {
        Handle::Divider(k) if k + 1 < state.window_count => {
            commit_divider(state, k, d.offset / h, now);
        }
        _ => {
            // Edges (and degenerate dividers): membership transition. Animate from the current
            // config to the new one, continuing from where the drag's interp left off (~progress t).
            let top = state.window_top;
            let count = state.window_count;
            if let Some((tt, tc, t)) = resolve(d.handle, d.offset, top, count, trigger) {
                if t >= 0.5 {
                    let from_w = to_arr(&nweights(&state.weights, count));
                    begin_anim(state, top, count, from_w, tt, tc, even_arr(tc), t, now);
                }
            }
        }
    }
}

/// Apply a divider release. 3-pane and 2-pane have different snap rules (see below). Either way the
/// result is eased in from whatever was on screen at release.
fn commit_divider(state: &mut MobileState, k: usize, offset_frac: f32, now: f64) {
    let top = state.window_top;
    let count = state.window_count;
    let nw = nweights(&state.weights, count);
    let b0: f32 = nw[..=k].iter().sum();
    let b = b0 + offset_frac; // unclamped boundary position of divider k

    if count == 3 {
        // `from` = what's on screen at release.
        let from_w = live_weights3(k, b);
        // Sizes of the two extreme panes after this drag, to test the "grown past 66%" rule.
        let pane_top = from_w[0]; // grown when dragging the upper divider down
        let pane_bot = from_w[2]; // grown when dragging the lower divider up

        if k == 0 && pane_top >= COLLAPSE_GROW {
            // Upper pane grown out → drop the bottom pane; the surviving {0,1} divider snaps to
            // where it was dropped.
            if let Some((tt, tc)) = op_r4(top, 3) {
                let to_w = collapse_2pane(from_w[0], from_w[1]);
                begin_anim(state, top, 3, from_w, tt, tc, to_w, 0.0, now);
            }
        } else if k == 1 && pane_bot >= COLLAPSE_GROW {
            // Bottom pane grown out → drop the top pane; surviving {1,2} divider snaps to drop.
            if let Some((tt, tc)) = op_r3(top, 3) {
                let to_w = collapse_2pane(from_w[1], from_w[2]);
                begin_anim(state, top, 3, from_w, tt, tc, to_w, 0.0, now);
            }
        } else if b <= COLLAPSE_LO {
            if let Some((tt, tc)) = op_r1(top, 3) {
                begin_anim(state, top, 3, from_w, tt, tc, even_arr(tc), 0.0, now);
            }
        } else if b >= COLLAPSE_HI {
            if let Some((tt, tc)) = op_r2(top, 3) {
                begin_anim(state, top, 3, from_w, tt, tc, even_arr(tc), 0.0, now);
            }
        } else {
            // Snap both dividers together: round the path parameter to the nearest preset.
            let (lo, hi) = path3_range(k);
            let s = s_from_boundary(k, b.clamp(lo, hi)).round().clamp(0.0, 2.0);
            let to_w = PRESETS_3[s as usize];
            begin_anim(state, top, 3, from_w, top, 3, to_w, 0.0, now);
        }
        return;
    }

    // 2-pane: group resize, snap to nearest 2-pane preset, slide off at the extremes.
    let (bmin, bmax) = boundary_bounds(count, k);
    let from_w = to_arr(&weights_for_boundary(&nw, k, b.clamp(bmin, bmax)));
    if b <= COLLAPSE_LO {
        if let Some((tt, tc)) = op_r1(top, count) {
            begin_anim(state, top, count, from_w, tt, tc, even_arr(tc), 0.0, now);
        }
    } else if b >= COLLAPSE_HI {
        if let Some((tt, tc)) = op_r2(top, count) {
            begin_anim(state, top, count, from_w, tt, tc, even_arr(tc), 0.0, now);
        }
    } else {
        let to_w = nearest_preset(&from_w, count);
        begin_anim(state, top, count, from_w, top, count, to_w, 0.0, now);
    }
}

/// Given the two surviving panes' (unnormalized) sizes after a 3→2 collapse, snap to the nearest
/// 2-pane preset by the first pane's proportion.
fn collapse_2pane(a: f32, b: f32) -> [f32; 3] {
    let total = (a + b).max(1e-4);
    let p0 = a / total;
    nearest_preset(&[p0, 1.0 - p0, 0.0], 2)
}

/// Switch to a new window config, easing from `from_*` to the new state over the snap duration.
/// `start_p` back-dates the animation so it can continue an in-progress drag.
fn begin_anim(
    state: &mut MobileState,
    from_top: usize,
    from_count: usize,
    from_w: [f32; 3],
    to_top: usize,
    to_count: usize,
    to_w: [f32; 3],
    start_p: f32,
    now: f64,
) {
    state.window_top = to_top;
    state.window_count = to_count;
    state.weights = to_w;
    state.anim = Some(LayoutAnim {
        from_top,
        from_count,
        from_w,
        to_top,
        to_count,
        to_w,
        start: now - start_p as f64 * SNAP_ANIM_SECS,
    });
}

/// Switch the visible window and reset to even pane sizing, easing the transition.
fn set_window(state: &mut MobileState, top: usize, count: usize, now: f64) {
    let from_w = to_arr(&nweights(&state.weights, state.window_count));
    begin_anim(state, state.window_top, state.window_count, from_w, top, count, even_arr(count), 0.0, now);
}
