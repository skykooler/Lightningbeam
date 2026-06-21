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
    let bar_height  = 22.0_f32;
    let peak_h      = 7.0_f32;   // triangular roof height
    let body_h      = 12.0_f32;  // rectangular body height
    let handle_h    = peak_h + body_h;
    let body_half_w = 6.0_f32;
    let right_pad   = 10.0_f32;  // keep rightmost stop clear of infopanel scrollbar
    let total_height = bar_height + handle_h + 4.0;
    let avail_w     = ui.available_width() - right_pad;

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

    // Draw gradient bar as a mesh: one quad per stop-pair with vertex colours
    // so the GPU interpolates linearly — no segmentation artefacts.
    {
        use egui::epaint::{Mesh, Vertex};
        let mut mesh = Mesh::default();
        let stops = &gradient.stops;
        let color_at = |t: f32| -> Color32 {
            let [r, g, b, a] = gradient.eval(t);
            Color32::from_rgba_unmultiplied(r, g, b, a)
        };
        // One quad for each consecutive stop pair.
        for pair in stops.windows(2) {
            let t0 = pair[0].position;
            let t1 = pair[1].position;
            let c0 = color_at(t0);
            let c1 = color_at(t1);
            let x0 = bar.min.x + t0 * bar.width();
            let x1 = bar.min.x + t1 * bar.width();
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&[
                Vertex { pos: egui::pos2(x0, bar.min.y), uv: egui::Pos2::ZERO, color: c0 },
                Vertex { pos: egui::pos2(x1, bar.min.y), uv: egui::Pos2::ZERO, color: c1 },
                Vertex { pos: egui::pos2(x1, bar.max.y), uv: egui::Pos2::ZERO, color: c1 },
                Vertex { pos: egui::pos2(x0, bar.max.y), uv: egui::Pos2::ZERO, color: c0 },
            ]);
            mesh.indices.extend_from_slice(&[base, base+1, base+2, base, base+2, base+3]);
        }
        painter.add(egui::Shape::mesh(mesh));
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
                *selected_stop = gradient.stops.iter().position(|s| (s.position - t).abs() < 1e-5);
                changed = true;
            }
        }
    }

    // ── Stop handles: interact + popup ───────────────────────────────────
    let n_stops = gradient.stops.len();

    // Top-anchored hit rects (peak touches track.min.y).
    let handle_rects: Vec<Rect> = (0..n_stops).map(|i| {
        let cx = track.min.x + gradient.stops[i].position * track.width();
        Rect::from_min_size(
            egui::pos2(cx - body_half_w, track.min.y),
            Vec2::new(body_half_w * 2.0, handle_h),
        )
    }).collect();

    let mut drag_delta : f32  = 0.0;
    let mut drag_active: bool = false;
    let mut drag_ended : bool = false;
    let mut delete_idx : Option<usize> = None;

    for (i, &h_rect) in handle_rects.iter().enumerate() {
        let resp = ui.interact(h_rect, ui.id().with(("grad_handle", i)), Sense::click_and_drag());

        // Anchor the dragged stop at drag-start time, before any sort can change indices.
        if resp.drag_started() {
            *selected_stop = Some(i);
        }
        if resp.dragged() {
            drag_delta = resp.drag_delta().x / track.width();
            drag_active = true;
        }
        if resp.drag_stopped() {
            drag_ended = true;
        }
        if resp.clicked() {
            *selected_stop = Some(i);
        }
        // Right-click on an interior stop (not the first or last) deletes it.
        if resp.secondary_clicked() && i > 0 && i < n_stops - 1 {
            delete_idx = Some(i);
        }

        // Color picker popup — opens on click, closes on click-outside.
        egui::containers::Popup::from_toggle_button_response(&resp)
            .show(|ui| {
                ui.spacing_mut().slider_width = 200.0;
                let stop = &mut gradient.stops[i];
                let mut c32 = Color32::from_rgba_unmultiplied(
                    stop.color.r, stop.color.g, stop.color.b, stop.color.a,
                );
                if egui::color_picker::color_picker_color32(
                    ui, &mut c32, egui::color_picker::Alpha::OnlyBlend,
                ) {
                    // Color32 stores premultiplied RGB; unmultiply before storing
                    // as straight-alpha ShapeColor to avoid darkening on round-trip.
                    let [pr, pg, pb, a] = c32.to_array();
                    let unpm = |c: u8| -> u8 {
                        if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 }
                    };
                    stop.color = ShapeColor::rgba(unpm(pr), unpm(pg), unpm(pb), a);
                    changed = true;
                }
            });
    }

    // Apply drag to whichever stop selected_stop points at.
    // Using selected_stop (anchored at drag_started) instead of the widget index
    // means sorting never causes a different stop to be dragged when the dragged
    // stop passes over a neighbour.
    if drag_active {
        if let Some(cur) = *selected_stop {
            if drag_delta != 0.0 {
                let new_pos = (gradient.stops[cur].position + drag_delta).clamp(0.0, 1.0);
                gradient.stops[cur].position = new_pos;
                gradient.stops.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap());
                // Re-find the moved stop by its new position so selected_stop stays correct.
                *selected_stop = gradient.stops.iter()
                    .position(|s| (s.position - new_pos).abs() < 1e-5);
                changed = true;
            }
        }
    }

    // Merge-on-drop: if the dragged stop was released within one handle-width of
    // another stop, delete that other stop (provided ≥ 3 stops remain).
    if drag_ended {
        if let Some(cur) = *selected_stop {
            if gradient.stops.len() > 2 {
                let my_pos = gradient.stops[cur].position;
                let merge_thresh = body_half_w / track.width();
                if let Some(victim) = gradient.stops.iter().enumerate()
                    .find(|&(j, s)| j != cur && (s.position - my_pos).abs() < merge_thresh)
                    .map(|(j, _)| j)
                {
                    gradient.stops.remove(victim);
                    if victim < cur {
                        *selected_stop = Some(cur - 1);
                    }
                    changed = true;
                }
            }
        }
    }

    // Apply right-click delete (after loop to avoid borrow conflicts).
    if let Some(i) = delete_idx {
        gradient.stops.remove(i);
        if *selected_stop == Some(i) {
            *selected_stop = None;
        } else if let Some(sel) = *selected_stop {
            if sel > i {
                *selected_stop = Some(sel - 1);
            }
        }
        changed = true;
    }

    // ── Paint handles ─────────────────────────────────────────────────────
    // handle_rects was built before any deletions this frame; guard against OOB.
    for (i, h_rect) in handle_rects.iter().enumerate().take(gradient.stops.len()) {
        let col = shape_color_to_color32(gradient.stops[i].color);
        let is_selected = *selected_stop == Some(i);
        let stroke = Stroke::new(
            if is_selected { 2.0 } else { 1.0 },
            if is_selected { Color32::WHITE } else { Color32::from_gray(80) },
        );
        let cx         = h_rect.center().x;
        let apex       = egui::pos2(cx, track.min.y);
        let shoulder_y = track.min.y + peak_h;
        let bottom_y   = track.min.y + handle_h;
        // Convex pentagon: apex → upper-right → lower-right → lower-left → upper-left
        painter.add(egui::Shape::convex_polygon(
            vec![
                apex,
                egui::pos2(cx + body_half_w, shoulder_y),
                egui::pos2(cx + body_half_w, bottom_y),
                egui::pos2(cx - body_half_w, bottom_y),
                egui::pos2(cx - body_half_w, shoulder_y),
            ],
            col,
            stroke,
        ));
    }

    // ── Selected stop detail (position + remove) ──────────────────────────
    if let Some(i) = *selected_stop {
        if i < gradient.stops.len() {
            ui.separator();
            ui.horizontal(|ui| {
                let stop = &mut gradient.stops[i];
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

fn shape_color_to_color32(c: ShapeColor) -> Color32 {
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
