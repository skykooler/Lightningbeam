/// Generic curve lane widget — renders a keyframe curve and handles editing interactions.
///
/// Used for audio automation lanes (AutomationInput nodes) and, in future, for visual
/// property animation lanes on vector/raster layers.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, Vec2};

// ─── Data types ──────────────────────────────────────────────────────────────

/// A single keyframe. Values are in the caller's raw unit space (not normalised).
/// Convert from `AutomationKeyframeData` or `lightningbeam_core::animation::Keyframe`
/// before passing in.
#[derive(Clone, Debug)]
pub struct CurvePoint {
    pub time: f64,
    pub value: f32,
    pub interpolation: CurveInterpolation,
    /// Outgoing Bezier tangent (x, y) relative to this keyframe, range 0–1
    pub ease_out: (f32, f32),
    /// Incoming Bezier tangent (x, y) relative to next keyframe, range 0–1
    pub ease_in: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurveInterpolation {
    Linear,
    Bezier,
    Step,
    Hold,
}

/// Edit action the user performed during one frame, returned from [`render_curve_lane`].
#[derive(Debug)]
pub enum CurveEditAction {
    None,
    AddKeyframe { time: f64, value: f32 },
    MoveKeyframe { index: usize, new_time: f64, new_value: f32 },
    DeleteKeyframe { index: usize },
}

/// Drag state for an in-progress keyframe move.
/// Stored by the caller alongside the lane's cached keyframe list.
#[derive(Clone, Debug)]
pub struct CurveDragState {
    pub keyframe_index: usize,
    pub current_time: f64,
    pub current_value: f32,
}

// ─── Curve evaluation ────────────────────────────────────────────────────────

/// Evaluate the curve defined by `keyframes` at the given `time`.
///
/// Matches the interpolation logic of `AutomationInputNode::evaluate_at_time()`.
pub fn evaluate_curve(keyframes: &[CurvePoint], time: f64) -> f32 {
    if keyframes.is_empty() {
        return 0.0;
    }
    if keyframes.len() == 1 || time <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = &keyframes[keyframes.len() - 1];
    if time >= last.time {
        return last.value;
    }

    // Find the pair that brackets `time`
    let right = keyframes.partition_point(|kf| kf.time <= time);
    let kf1 = &keyframes[right - 1];
    let kf2 = &keyframes[right];

    let t = if kf2.time == kf1.time {
        0.0f32
    } else {
        ((time - kf1.time) / (kf2.time - kf1.time)) as f32
    };

    match kf1.interpolation {
        CurveInterpolation::Linear => kf1.value + (kf2.value - kf1.value) * t,
        CurveInterpolation::Bezier => {
            let eased = cubic_bezier_ease(t, kf1.ease_out, kf2.ease_in);
            kf1.value + (kf2.value - kf1.value) * eased
        }
        CurveInterpolation::Step | CurveInterpolation::Hold => kf1.value,
    }
}

/// Simplified cubic Bezier easing (0,0 → ease_out → ease_in → 1,1).
/// Identical to `AutomationInputNode::cubic_bezier_ease`.
fn cubic_bezier_ease(t: f32, ease_out: (f32, f32), ease_in: (f32, f32)) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * ease_out.1 + 3.0 * u * t * t * ease_in.1 + t * t * t
}

// ─── Rendering ───────────────────────────────────────────────────────────────

const DIAMOND_RADIUS: f32 = 5.0;

/// Render a curve lane within `rect` and return any edit action the user performed.
///
/// `drag_state` is an in/out reference; the caller is responsible for storing it between
/// frames alongside the lane's keyframe list.
///
/// `value_min` and `value_max` define the displayed value range (bottom to top of rect).
/// Keyframe values outside this range are clamped visually.
///
/// `time_to_x` maps a project time (seconds) to an **absolute** screen X coordinate.
/// `x_to_time` maps an **absolute** screen X coordinate to project time.
pub fn render_curve_lane(
    ui: &mut egui::Ui,
    rect: Rect,
    keyframes: &[CurvePoint],
    drag_state: &mut Option<CurveDragState>,
    playback_time: f64,
    accent_color: Color32,
    id: egui::Id,
    value_min: f32,
    value_max: f32,
    time_to_x: impl Fn(f64) -> f32,
    x_to_time: impl Fn(f32) -> f64,
) -> CurveEditAction {
    let painter = ui.painter_at(rect);

    // Helper: raw value → normalised [0,1] for screen-Y mapping
    let normalize = |v: f32| -> f32 {
        if (value_max - value_min).abs() < f32::EPSILON {
            0.5
        } else {
            (v - value_min) / (value_max - value_min)
        }
    };
    // Helper: normalised [0,1] → raw value
    let denormalize = |n: f32| -> f32 {
        value_min + n * (value_max - value_min)
    };

    // ── Background ──────────────────────────────────────────────────────────
    painter.rect_filled(rect, 0.0, Color32::from_rgba_premultiplied(20, 20, 25, 230));

    // Inset shadow: dark line at top, light line at bottom
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0, Color32::from_black_alpha(60)),
    );
    let bottom_y = rect.max.y - 1.0;
    painter.line_segment(
        [Pos2::new(rect.min.x, bottom_y), Pos2::new(rect.max.x, bottom_y)],
        Stroke::new(1.0, Color32::from_white_alpha(18)),
    );

    // Zero-line (value = 0, or mid-line if range doesn't include 0)
    let zero_norm = normalize(0.0).clamp(0.0, 1.0);
    let zero_y = value_to_y(zero_norm, rect);
    painter.line_segment(
        [Pos2::new(rect.min.x, zero_y), Pos2::new(rect.max.x, zero_y)],
        Stroke::new(1.0, Color32::from_rgba_premultiplied(80, 80, 80, 120)),
    );

    // ── Curve polyline ───────────────────────────────────────────────────────
    // Build a working keyframe list with any in-progress drag preview applied
    let display_keyframes: Vec<CurvePoint> = if let Some(ref ds) = drag_state {
        let mut kfs = keyframes.to_vec();
        if ds.keyframe_index < kfs.len() {
            kfs[ds.keyframe_index].time = ds.current_time;
            kfs[ds.keyframe_index].value = ds.current_value;
            kfs.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        }
        kfs
    } else {
        keyframes.to_vec()
    };

    if !display_keyframes.is_empty() {
        let step = 2.0f32;  // sample every 2 screen pixels
        let num_steps = ((rect.width() / step) as usize).max(1);
        let mut points: Vec<Pos2> = Vec::with_capacity(num_steps + 1);

        for i in 0..=num_steps {
            let x = rect.min.x + i as f32 * step;
            let t = x_to_time(x.min(rect.max.x));
            let v = evaluate_curve(&display_keyframes, t);
            let y = value_to_y(normalize(v), rect);
            points.push(Pos2::new(x.min(rect.max.x), y));
        }

        let curve_color = accent_color.linear_multiply(0.8);
        painter.add(Shape::line(points, Stroke::new(1.5, curve_color)));
    }

    // ── Playhead ─────────────────────────────────────────────────────────────
    let ph_x = time_to_x(playback_time);
    if ph_x >= rect.min.x && ph_x <= rect.max.x {
        painter.line_segment(
            [Pos2::new(ph_x, rect.min.y), Pos2::new(ph_x, rect.max.y)],
            Stroke::new(1.0, Color32::from_rgb(255, 80, 80)),
        );
    }

    // ── Interaction ──────────────────────────────────────────────────────────
    let sense = egui::Sense::click_and_drag();
    let response = ui.interact(rect, id, sense);

    // latest_pos() works whether the pointer button is up or down (unlike interact_pos).
    let pointer_pos: Option<Pos2> = ui.input(|i| i.pointer.latest_pos());

    // Find which keyframe (if any) the pointer is near
    let hovered_kf: Option<usize> = pointer_pos.and_then(|pos| {
        keyframes.iter().enumerate().find(|(_, kf)| {
            let kx = time_to_x(kf.time);
            let ky = value_to_y(normalize(kf.value), rect);
            let d = Vec2::new(pos.x - kx, pos.y - ky).length();
            d <= DIAMOND_RADIUS * 1.5
        }).map(|(i, _)| i)
    });

    // Draw keyframe diamonds (after interaction setup so hover color works)
    for (idx, kf) in keyframes.iter().enumerate() {
        let kx = time_to_x(kf.time);
        if kx < rect.min.x - DIAMOND_RADIUS || kx > rect.max.x + DIAMOND_RADIUS {
            continue;
        }
        let ky = value_to_y(normalize(kf.value), rect);

        // During drag, show this diamond at its preview position
        let (draw_x, draw_y) = if let Some(ref ds) = drag_state {
            if ds.keyframe_index == idx {
                (time_to_x(ds.current_time), value_to_y(normalize(ds.current_value), rect))
            } else {
                (kx, ky)
            }
        } else {
            (kx, ky)
        };

        let is_hovered = hovered_kf == Some(idx);
        let is_dragging = drag_state.as_ref().map_or(false, |d| d.keyframe_index == idx);

        let fill = if is_dragging {
            Color32::WHITE
        } else if is_hovered {
            accent_color
        } else {
            accent_color.linear_multiply(0.7)
        };

        draw_diamond(&painter, Pos2::new(draw_x, draw_y), DIAMOND_RADIUS, fill);
    }

    // ── Interaction logic ────────────────────────────────────────────────────

    // Right-click → delete keyframe
    if response.secondary_clicked() {
        if let Some(idx) = hovered_kf {
            return CurveEditAction::DeleteKeyframe { index: idx };
        }
    }

    // Left drag start → begin dragging a keyframe
    if response.drag_started() {
        if let Some(idx) = hovered_kf {
            let kf = &keyframes[idx];
            *drag_state = Some(CurveDragState {
                keyframe_index: idx,
                current_time: kf.time,
                current_value: kf.value,
            });
        }
    }

    // Drag in progress → update preview position
    if let Some(ref mut ds) = drag_state {
        if response.dragged() {
            if let Some(pos) = pointer_pos {
                let clamped_x = pos.x.clamp(rect.min.x, rect.max.x);
                let clamped_y = pos.y.clamp(rect.min.y, rect.max.y);
                ds.current_time = x_to_time(clamped_x);
                ds.current_value = denormalize(y_to_value(clamped_y, rect));
            }
        }
        // Drag released → commit
        if response.drag_stopped() {
            let ds = drag_state.take().unwrap();
            return CurveEditAction::MoveKeyframe {
                index: ds.keyframe_index,
                new_time: ds.current_time,
                new_value: ds.current_value,
            };
        }
    }

    // Left click on empty space → add keyframe
    // Use interact_pointer_pos() here: it captures the click position even after button release.
    if response.clicked() && hovered_kf.is_none() && drag_state.is_none() {
        if let Some(pos) = response.interact_pointer_pos() {
            let t = x_to_time(pos.x);
            let v = denormalize(y_to_value(pos.y, rect));
            return CurveEditAction::AddKeyframe { time: t, value: v };
        }
    }

    CurveEditAction::None
}

// ─── Coordinate helpers ───────────────────────────────────────────────────────

/// Map a normalised value (0=bottom, 1=top) to a Y screen coordinate within `rect`.
pub fn value_to_y(value: f32, rect: Rect) -> f32 {
    rect.max.y - value.clamp(0.0, 1.0) * rect.height()
}

/// Map a screen Y coordinate within `rect` to a normalised value (0=bottom, 1=top).
pub fn y_to_value(y: f32, rect: Rect) -> f32 {
    ((rect.max.y - y) / rect.height()).clamp(0.0, 1.0)
}

// ─── Drawing utilities ────────────────────────────────────────────────────────

fn draw_diamond(painter: &egui::Painter, center: Pos2, radius: f32, fill: Color32) {
    let points = vec![
        Pos2::new(center.x, center.y - radius),  // top
        Pos2::new(center.x + radius, center.y),  // right
        Pos2::new(center.x, center.y + radius),  // bottom
        Pos2::new(center.x - radius, center.y),  // left
    ];
    painter.add(Shape::convex_polygon(
        points,
        fill,
        Stroke::new(1.0, Color32::from_rgba_premultiplied(0, 0, 0, 180)),
    ));
}
