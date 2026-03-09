//! Gradient stop editor widget.
//!
//! Call [`gradient_stop_editor`] inside any egui layout; it returns `true` when
//! `gradient` was modified.

use eframe::egui::{self, Color32, DragValue, Painter, Rect, Sense, Stroke, Vec2};
use lightningbeam_core::gradient::{GradientExtend, GradientStop, GradientType, ShapeGradient};
use lightningbeam_core::shape::ShapeColor;

// ── Public entry point ───────────────────────────────────────────────────────

/// Render an inline gradient editor.
///
/// * `gradient`      – the gradient being edited (mutated in place).
/// * `selected_stop` – index of the currently selected stop (persisted by caller).
///
/// Returns `true` if anything changed.
pub fn gradient_stop_editor(
    ui: &mut egui::Ui,
    gradient: &mut ShapeGradient,
    selected_stop: &mut Option<usize>,
) -> bool {
    let mut changed = false;

    // ── Row 1: Kind + angle ───────────────────────────────────────────────
    ui.horizontal(|ui| {
        let was_linear = gradient.kind == GradientType::Linear;
        if ui.selectable_label(was_linear,  "Linear").clicked() && !was_linear {
            gradient.kind = GradientType::Linear;
            changed = true;
        }
        if ui.selectable_label(!was_linear, "Radial").clicked() && was_linear {
            gradient.kind = GradientType::Radial;
            changed = true;
        }
        if gradient.kind == GradientType::Linear {
            ui.add_space(8.0);
            ui.label("Angle:");
            if ui.add(
                DragValue::new(&mut gradient.angle)
                    .speed(1.0)
                    .range(-360.0..=360.0)
                    .suffix("°"),
            ).changed() {
                changed = true;
            }
        }
    });

    // ── Gradient bar + handles ────────────────────────────────────────────
    let bar_height   = 22.0_f32;
    let handle_h     = 14.0_f32;
    let total_height = bar_height + handle_h + 4.0;
    let avail_w      = ui.available_width();

    let (bar_rect, bar_resp) = ui.allocate_exact_size(
        Vec2::new(avail_w, total_height),
        Sense::click(),
    );
    let painter = ui.painter_at(bar_rect);

    let bar   = Rect::from_min_size(bar_rect.min, Vec2::new(avail_w, bar_height));
    let track = Rect::from_min_size(
        egui::pos2(bar_rect.min.x, bar_rect.min.y + bar_height + 2.0),
        Vec2::new(avail_w, handle_h),
    );

    // Draw checkerboard background (transparent indicator).
    draw_checker(&painter, bar);

    // Draw gradient bar as N segments.
    let seg = 128_usize;
    for i in 0..seg {
        let t0 = i as f32 / seg as f32;
        let t1 = (i + 1) as f32 / seg as f32;
        let t  = (t0 + t1) * 0.5;
        let [r, g, b, a] = gradient.eval(t);
        let col = Color32::from_rgba_unmultiplied(r, g, b, a);
        let x0  = bar.min.x + t0 * bar.width();
        let x1  = bar.min.x + t1 * bar.width();
        let seg_rect = Rect::from_min_max(
            egui::pos2(x0, bar.min.y),
            egui::pos2(x1, bar.max.y),
        );
        painter.rect_filled(seg_rect, 0.0, col);
    }
    // Outline.
    painter.rect_stroke(bar, 2.0, Stroke::new(1.0, Color32::from_gray(60)), eframe::egui::StrokeKind::Middle);

    // Click on bar → add stop.
    if bar_resp.clicked() {
        if let Some(pos) = bar_resp.interact_pointer_pos() {
            if bar.contains(pos) {
                let t = ((pos.x - bar.min.x) / bar.width()).clamp(0.0, 1.0);
                let [r, g, b, a] = gradient.eval(t);
                gradient.stops.push(GradientStop {
                    position: t,
                    color:    ShapeColor::rgba(r, g, b, a),
                });
                gradient.stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                *selected_stop = gradient.stops.iter().position(|s| s.position == t);
                changed = true;
            }
        }
    }

    // Draw stop handles.
    // We need to detect drags per-handle, so allocate individual rects with the
    // regular egui input model. To avoid borrow conflicts we collect interactions
    // before mutating.
    let handle_w  = 10.0_f32;
    let n_stops   = gradient.stops.len();

    let mut drag_idx:    Option<usize>  = None;
    let mut drag_delta:  f32            = 0.0;
    let mut click_idx:   Option<usize>  = None;

    // To render handles after collecting, remember their rects.
    let handle_rects: Vec<Rect> = (0..n_stops).map(|i| {
        let cx = track.min.x + gradient.stops[i].position * track.width();
        Rect::from_center_size(
            egui::pos2(cx, track.center().y),
            Vec2::new(handle_w, handle_h),
        )
    }).collect();

    for (i, &h_rect) in handle_rects.iter().enumerate() {
        let resp = ui.interact(h_rect, ui.id().with(("grad_handle", i)), Sense::click_and_drag());
        if resp.dragged() {
            drag_idx = Some(i);
            drag_delta = resp.drag_delta().x / track.width();
        }
        if resp.clicked() {
            click_idx = Some(i);
        }
    }

    // Apply drag.
    if let (Some(i), delta) = (drag_idx, drag_delta) {
        if delta != 0.0 {
            let new_pos = (gradient.stops[i].position + delta).clamp(0.0, 1.0);
            gradient.stops[i].position = new_pos;
            // Re-sort and track the moved stop.
            gradient.stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
            // Find new index of the moved stop (closest position match).
            if let Some(ref mut sel) = *selected_stop {
                // Re-find by position proximity.
                *sel = gradient.stops.iter().enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let pa = (a.position - (gradient.stops.get(i).map_or(0.0, |s| s.position))).abs();
                        let pb = (b.position - (gradient.stops.get(i).map_or(0.0, |s| s.position))).abs();
                        pa.partial_cmp(&pb).unwrap()
                    })
                    .map(|(idx, _)| idx)
                    .unwrap_or(0);
            }
            changed = true;
        }
    }

    if let Some(i) = click_idx {
        *selected_stop = Some(i);
    }

    // Paint handles on top (after interaction so they visually react).
    for (i, h_rect) in handle_rects.iter().enumerate() {
        let col = ShapeColor_to_Color32(gradient.stops[i].color);
        let is_selected = *selected_stop == Some(i);

        // Draw a downward-pointing triangle.
        let cx = h_rect.center().x;
        let top = h_rect.min.y;
        let bot = h_rect.max.y;
        let hw  = h_rect.width() * 0.5;
        let tri = vec![
            egui::pos2(cx,       bot),
            egui::pos2(cx - hw,  top),
            egui::pos2(cx + hw,  top),
        ];
        painter.add(egui::Shape::convex_polygon(
            tri,
            col,
            Stroke::new(if is_selected { 2.0 } else { 1.0 },
                        if is_selected { Color32::WHITE } else { Color32::from_gray(100) }),
        ));
    }

    // ── Selected stop detail ──────────────────────────────────────────────
    if let Some(i) = *selected_stop {
        if i < gradient.stops.len() {
            ui.separator();
            ui.horizontal(|ui| {
                let stop = &mut gradient.stops[i];
                let mut rgba = [stop.color.r, stop.color.g, stop.color.b, stop.color.a];
                if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
                    stop.color = ShapeColor::rgba(rgba[0], rgba[1], rgba[2], rgba[3]);
                    changed = true;
                }
                ui.label("Position:");
                if ui.add(
                    DragValue::new(&mut stop.position)
                        .speed(0.005)
                        .range(0.0..=1.0),
                ).changed() {
                    gradient.stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                    changed = true;
                }
                let can_remove = gradient.stops.len() > 2;
                if ui.add_enabled(can_remove, egui::Button::new("− Remove")).clicked() {
                    gradient.stops.remove(i);
                    *selected_stop = None;
                    changed = true;
                }
            });
        } else {
            *selected_stop = None;
        }
    }

    // ── Extend mode ───────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        ui.label("Extend:");
        if ui.selectable_label(gradient.extend == GradientExtend::Pad,     "Pad").clicked() {
            gradient.extend = GradientExtend::Pad;     changed = true;
        }
        if ui.selectable_label(gradient.extend == GradientExtend::Reflect,  "Reflect").clicked() {
            gradient.extend = GradientExtend::Reflect;  changed = true;
        }
        if ui.selectable_label(gradient.extend == GradientExtend::Repeat,   "Repeat").clicked() {
            gradient.extend = GradientExtend::Repeat;   changed = true;
        }
    });

    changed
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ShapeColor_to_Color32(c: ShapeColor) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r, c.g, c.b, c.a)
}

/// Draw a small grey/white checkerboard inside `rect`.
fn draw_checker(painter: &Painter, rect: Rect) {
    let cell = 6.0_f32;
    let cols = ((rect.width()  / cell).ceil() as u32).max(1);
    let rows = ((rect.height() / cell).ceil() as u32).max(1);
    for row in 0..rows {
        for col in 0..cols {
            let light = (row + col) % 2 == 0;
            let col32 = if light { Color32::from_gray(200) } else { Color32::from_gray(140) };
            let x = rect.min.x + col as f32 * cell;
            let y = rect.min.y + row as f32 * cell;
            let r = Rect::from_min_size(
                egui::pos2(x, y),
                Vec2::splat(cell),
            ).intersect(rect);
            painter.rect_filled(r, 0.0, col32);
        }
    }
}
