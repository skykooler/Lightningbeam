//! SVG export from a DCEL subgraph.
//!
//! Generates a minimal SVG string containing one `<path>` per filled face,
//! plus stroked edges. Used as the secondary clipboard format for cross-app paste.

use crate::dcel2::{Dcel, FaceId, HalfEdgeId};
use kurbo::CubicBez;

/// Convert a DCEL to an SVG string.
///
/// Each non-unbounded face with a fill color becomes a `<path fill="..."/>`.
/// Each edge with a stroke becomes a `<path stroke="..."/>`.
/// Coordinates are document-space (no transform applied).
pub fn dcel_to_svg(dcel: &Dcel) -> String {
    // Compute bounding box from vertex positions.
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for v in &dcel.vertices {
        if !v.deleted {
            min_x = min_x.min(v.position.x);
            min_y = min_y.min(v.position.y);
            max_x = max_x.max(v.position.x);
            max_y = max_y.max(v.position.y);
        }
    }

    if min_x == f64::MAX {
        return r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_string();
    }

    // Add a small margin.
    let margin = 2.0;
    let vx = min_x - margin;
    let vy = min_y - margin;
    let vw = (max_x - min_x) + margin * 2.0;
    let vh = (max_y - min_y) + margin * 2.0;

    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{vx:.3} {vy:.3} {vw:.3} {vh:.3}">"#
    );

    // Emit filled faces.
    for (face_idx, face) in dcel.faces.iter().enumerate() {
        if face.deleted || face_idx == 0 {
            continue;
        }
        let fill_color = match &face.fill_color {
            Some(c) => format!("rgba({},{},{},{})", c.r, c.g, c.b, c.a as f32 / 255.0),
            None => continue,
        };

        let face_id = FaceId(face_idx as u32);
        let path_d = face_boundary_to_svg_path(dcel, face_id);
        if path_d.is_empty() {
            continue;
        }

        svg.push_str(&format!(r#"<path fill="{fill_color}" d="{path_d}"/>"#));
    }

    // Emit stroked edges.
    for edge in &dcel.edges {
        if edge.deleted {
            continue;
        }
        let (stroke_color, stroke_width) = match (&edge.stroke_color, &edge.stroke_style) {
            (Some(c), Some(s)) => (
                format!("rgba({},{},{},{})", c.r, c.g, c.b, c.a as f32 / 255.0),
                s.width,
            ),
            _ => continue,
        };

        let path_d = cubic_to_svg_path(&edge.curve);
        svg.push_str(&format!(
            r#"<path fill="none" stroke="{stroke_color}" stroke-width="{stroke_width:.3}" d="{path_d}"/>"#
        ));
    }

    svg.push_str("</svg>");
    svg
}

/// Walk a face's outer boundary half-edges and build an SVG path string.
fn face_boundary_to_svg_path(dcel: &Dcel, face_id: FaceId) -> String {
    let face = dcel.face(face_id);
    let start_he = face.outer_half_edge;
    if start_he.is_none() {
        return String::new();
    }

    let mut path = String::new();
    let mut first = true;
    let mut he_id = start_he;

    // Safety limit to prevent infinite loops on malformed DCELs.
    let limit = dcel.half_edges.len() + 1;
    let mut count = 0;

    loop {
        if count > limit {
            break;
        }
        count += 1;

        let he = dcel.half_edge(he_id);
        if he.deleted {
            break;
        }

        let edge = dcel.edge(he.edge);
        // Determine curve direction: forward half-edge is half_edges[0].
        let curve = if edge.half_edges[0] == he_id {
            edge.curve
        } else {
            // Reverse the cubic bezier.
            let c = edge.curve;
            CubicBez::new(c.p3, c.p2, c.p1, c.p0)
        };

        if first {
            path.push_str(&format!("M {:.3} {:.3} ", curve.p0.x, curve.p0.y));
            first = false;
        }

        path.push_str(&format!(
            "C {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} ",
            curve.p1.x, curve.p1.y,
            curve.p2.x, curve.p2.y,
            curve.p3.x, curve.p3.y,
        ));

        he_id = he.next;
        if he_id == start_he {
            break;
        }
    }

    if !path.is_empty() {
        path.push('Z');
    }

    // Also handle inner boundaries (holes).
    for &inner_he_start in &face.inner_half_edges {
        if inner_he_start.is_none() {
            continue;
        }
        let inner = inner_boundary_to_svg_path(dcel, inner_he_start);
        if !inner.is_empty() {
            path.push(' ');
            path.push_str(&inner);
        }
    }

    path
}

fn inner_boundary_to_svg_path(dcel: &Dcel, start_he: HalfEdgeId) -> String {
    let mut path = String::new();
    let mut first = true;
    let mut he_id = start_he;
    let limit = dcel.half_edges.len() + 1;
    let mut count = 0;

    loop {
        if count > limit {
            break;
        }
        count += 1;

        let he = dcel.half_edge(he_id);
        if he.deleted {
            break;
        }

        let edge = dcel.edge(he.edge);
        let curve = if edge.half_edges[0] == he_id {
            edge.curve
        } else {
            let c = edge.curve;
            CubicBez::new(c.p3, c.p2, c.p1, c.p0)
        };

        if first {
            path.push_str(&format!("M {:.3} {:.3} ", curve.p0.x, curve.p0.y));
            first = false;
        }

        path.push_str(&format!(
            "C {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} ",
            curve.p1.x, curve.p1.y,
            curve.p2.x, curve.p2.y,
            curve.p3.x, curve.p3.y,
        ));

        he_id = he.next;
        if he_id == start_he {
            break;
        }
    }

    if !path.is_empty() {
        path.push('Z');
    }
    path
}

/// Convert a single cubic bezier to an SVG path string.
fn cubic_to_svg_path(curve: &CubicBez) -> String {
    format!(
        "M {:.3} {:.3} C {:.3} {:.3} {:.3} {:.3} {:.3} {:.3}",
        curve.p0.x, curve.p0.y,
        curve.p1.x, curve.p1.y,
        curve.p2.x, curve.p2.y,
        curve.p3.x, curve.p3.y,
    )
}
