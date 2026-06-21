/// Theme rendering helpers for painting CSS backgrounds
///
/// Handles solid colors, linear gradients (via egui Mesh), and image backgrounds.

use eframe::egui;

use crate::theme::Background;

/// Paint a background into the given rect
#[allow(dead_code)]
pub fn paint_background(
    painter: &egui::Painter,
    rect: egui::Rect,
    background: &Background,
    rounding: f32,
) {
    match background {
        Background::Solid(color) => {
            painter.rect_filled(rect, rounding, *color);
        }
        Background::LinearGradient { angle_degrees, stops } => {
            paint_linear_gradient(painter, rect, *angle_degrees, stops, rounding);
        }
        Background::Image { .. } => {
            // Image backgrounds require a TextureHandle loaded externally.
            // For now, fall back to transparent (no-op).
            // TODO: image cache integration
        }
    }
}

/// Paint a linear gradient using an egui Mesh with colored vertices
///
/// Supports arbitrary angles. The gradient direction follows CSS conventions:
/// - 0deg = bottom to top
/// - 90deg = left to right
/// - 180deg = top to bottom (default)
/// - 270deg = right to left
#[allow(dead_code)]
pub fn paint_linear_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    angle_degrees: f32,
    stops: &[(f32, egui::Color32)],
    rounding: f32,
) {
    if stops.len() < 2 {
        if let Some((_, color)) = stops.first() {
            painter.rect_filled(rect, rounding, *color);
        }
        return;
    }

    // Convert CSS angle to a direction vector
    // CSS: 0deg = to top, 90deg = to right, 180deg = to bottom
    let angle_rad = (angle_degrees - 90.0).to_radians();
    let dir = egui::vec2(angle_rad.cos(), angle_rad.sin());

    // Project rect corners onto gradient direction to find start/end
    let center = rect.center();
    let half_size = rect.size() / 2.0;

    // The gradient line length is the projection of the rect diagonal onto the direction
    let gradient_half_len = (half_size.x * dir.x.abs()) + (half_size.y * dir.y.abs());

    // For simple horizontal/vertical gradients with no rounding, use a mesh directly
    if rounding <= 0.0 {
        let mut mesh = egui::Mesh::default();
        mesh.texture_id = egui::TextureId::default();

        // For each consecutive pair of stops, add a quad
        for i in 0..stops.len() - 1 {
            let (t0, c0) = stops[i];
            let (t1, c1) = stops[i + 1];

            // Map t to positions along the gradient line
            let p0_along = -gradient_half_len + t0 * 2.0 * gradient_half_len;
            let p1_along = -gradient_half_len + t1 * 2.0 * gradient_half_len;

            // Perpendicular direction for quad width
            let perp = egui::vec2(-dir.y, dir.x);
            let perp_extent = (half_size.x * perp.x.abs()) + (half_size.y * perp.y.abs());

            let base0 = center + dir * p0_along;
            let base1 = center + dir * p1_along;

            let v0 = base0 - perp * perp_extent;
            let v1 = base0 + perp * perp_extent;
            let v2 = base1 + perp * perp_extent;
            let v3 = base1 - perp * perp_extent;

            let uv = egui::pos2(0.0, 0.0);
            let idx = mesh.vertices.len() as u32;
            mesh.vertices.push(egui::epaint::Vertex { pos: v0, uv, color: c0 });
            mesh.vertices.push(egui::epaint::Vertex { pos: v1, uv, color: c0 });
            mesh.vertices.push(egui::epaint::Vertex { pos: v2, uv, color: c1 });
            mesh.vertices.push(egui::epaint::Vertex { pos: v3, uv, color: c1 });

            mesh.indices.extend_from_slice(&[idx, idx + 1, idx + 2, idx, idx + 2, idx + 3]);
        }

        painter.add(egui::Shape::mesh(mesh));
    } else {
        // For rounded rects, paint without rounding for now.
        // TODO: proper rounded gradient with tessellation or clip mask
        paint_linear_gradient(painter, rect, angle_degrees, stops, 0.0);
    }
}
