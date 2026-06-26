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

// ===========================================================================
// Document / VectorGraph → SVG (the current model). The functions above target
// the legacy DCEL and are kept only for the clipboard stub.
// ===========================================================================

use crate::document::Document;
use crate::gradient::{GradientExtend, GradientType, ShapeGradient};
use crate::layer::AnyLayer;
use crate::shape::{Cap, FillRule, Join, ShapeColor};
use crate::vector_graph::{FillId, VectorGraph};
use kurbo::{BezPath, PathEl, Rect, Shape};

/// Serialize the document's **vector** content to a standalone SVG string, at document time `time`.
/// Vector layers (and groups of them) only — raster/video/audio/effect layers are skipped (a later
/// pass can rasterize them to `<image>`). Animation is a single static frame at `time`.
pub fn document_to_svg(document: &Document, time: f64) -> String {
    let (w, h) = (document.width, document.height);
    let mut defs = String::new();
    let mut body = String::new();
    let mut grad_n = 0usize;

    // Opaque background rect (skip if the document background is transparent).
    let bg = document.background_color;
    if bg.a > 0 {
        body.push_str(&format!(
            r#"<rect x="0" y="0" width="{w:.3}" height="{h:.3}" {}/>"#,
            fill_attrs(&bg)
        ));
    }

    for layer in &document.root.children {
        layer_to_svg(layer, time, 1.0, &mut body, &mut defs, &mut grad_n);
    }

    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" "#,
            r#"viewBox="0 0 {:.3} {:.3}"><defs>{}</defs>{}</svg>"#
        ),
        w, h, w, h, defs, body
    )
}

/// Append one layer's SVG. Recurses into groups (`<g>`); other non-vector layer types are skipped.
fn layer_to_svg(layer: &AnyLayer, time: f64, parent_opacity: f64, body: &mut String, defs: &mut String, grad_n: &mut usize) {
    match layer {
        AnyLayer::Vector(vl) => {
            let opacity = parent_opacity * vl.layer.opacity;
            if let Some(graph) = vl.tweened_graph_at(time) {
                let wrap = opacity < 0.999;
                if wrap {
                    body.push_str(&format!(r#"<g opacity="{opacity:.4}">"#));
                }
                vector_graph_to_svg(&graph, body, defs, grad_n);
                if wrap {
                    body.push_str("</g>");
                }
            }
            // NOTE: placed clip instances (nested clips with their own transform) are not yet
            // exported — a refinement once loose-geometry export is verified.
        }
        AnyLayer::Group(g) => {
            let opacity = parent_opacity * g.layer.opacity;
            body.push_str(&format!(r#"<g opacity="{opacity:.4}">"#));
            for child in &g.children {
                layer_to_svg(child, time, 1.0, body, defs, grad_n);
            }
            body.push_str("</g>");
        }
        // Raster/Video/Audio/Effect have no lossless vector representation — skipped this pass.
        _ => {}
    }
}

/// Emit a vector graph's fills (`<path fill>`) and stroked edges (`<path stroke>`) into `body`,
/// accumulating any gradients into `defs`. Geometry is in document space (no per-layer transform).
fn vector_graph_to_svg(graph: &VectorGraph, body: &mut String, defs: &mut String, grad_n: &mut usize) {
    // Fills first (drawn under strokes, matching the renderer).
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue;
        }
        let path = graph.fill_to_bezpath(FillId(i as u32));
        let d = bezpath_to_d(&path);
        if d.is_empty() {
            continue;
        }
        let rule = match fill.fill_rule {
            FillRule::NonZero => "nonzero",
            FillRule::EvenOdd => "evenodd",
        };

        if let Some(grad) = &fill.gradient_fill {
            let id = format!("grad{}", *grad_n);
            *grad_n += 1;
            defs.push_str(&gradient_to_svg(grad, &id, path.bounding_box()));
            body.push_str(&format!(r#"<path fill="url(#{id})" fill-rule="{rule}" d="{d}"/>"#));
        } else if fill.image_fill.is_some() {
            // Image fills need <image>/<pattern> + asset embedding — skipped this (vector-only) pass.
            continue;
        } else if let Some(c) = &fill.color {
            body.push_str(&format!(r#"<path {} fill-rule="{rule}" d="{d}"/>"#, fill_attrs(c)));
        }
    }

    // Strokes: one <path> per stroked edge (each edge may carry its own style).
    for edge in &graph.edges {
        if edge.deleted {
            continue;
        }
        if let (Some(style), Some(color)) = (&edge.stroke_style, &edge.stroke_color) {
            let d = cubic_to_svg_path(&edge.curve);
            body.push_str(&format!(
                r#"<path fill="none" {} stroke-width="{:.3}" stroke-linecap="{}" stroke-linejoin="{}" stroke-miterlimit="{:.3}" d="{d}"/>"#,
                stroke_attrs(color), style.width, cap_str(style.cap), join_str(style.join), style.miter_limit
            ));
        }
    }
}

/// `<linearGradient>` / `<radialGradient>` definition matching the renderer's start/end semantics.
fn gradient_to_svg(grad: &ShapeGradient, id: &str, bbox: Rect) -> String {
    use kurbo::Point;
    // Mirror renderer.rs: explicit world endpoints if present (radial reflects the edge through the
    // center so midpoint(start,end) == center), else derive from angle + bbox.
    let (start, end) = match (grad.start_world, grad.end_world) {
        (Some((sx, sy)), Some((ex, ey))) => match grad.kind {
            GradientType::Linear => (Point::new(sx, sy), Point::new(ex, ey)),
            GradientType::Radial => (Point::new(2.0 * sx - ex, 2.0 * sy - ey), Point::new(ex, ey)),
        },
        _ => crate::renderer::gradient_bbox_endpoints(grad.angle, bbox),
    };

    let stops: String = grad
        .stops
        .iter()
        .map(|s| {
            format!(
                r##"<stop offset="{:.4}" stop-color="#{:02x}{:02x}{:02x}" stop-opacity="{:.4}"/>"##,
                s.position, s.color.r, s.color.g, s.color.b, s.color.a as f32 / 255.0
            )
        })
        .collect();
    let spread = match grad.extend {
        GradientExtend::Pad => "pad",
        GradientExtend::Reflect => "reflect",
        GradientExtend::Repeat => "repeat",
    };

    match grad.kind {
        GradientType::Linear => format!(
            r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" x1="{:.3}" y1="{:.3}" x2="{:.3}" y2="{:.3}" spreadMethod="{spread}">{stops}</linearGradient>"#,
            start.x, start.y, end.x, end.y
        ),
        GradientType::Radial => {
            let (cx, cy) = ((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
            let r = (((end.x - start.x).powi(2) + (end.y - start.y).powi(2)).sqrt()) * 0.5;
            format!(
                r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" cx="{cx:.3}" cy="{cy:.3}" r="{r:.3}" spreadMethod="{spread}">{stops}</radialGradient>"#
            )
        }
    }
}

/// kurbo `BezPath` → SVG path-data string (`M/L/Q/C/Z`).
fn bezpath_to_d(path: &BezPath) -> String {
    let mut d = String::new();
    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => d.push_str(&format!("M{:.3} {:.3} ", p.x, p.y)),
            PathEl::LineTo(p) => d.push_str(&format!("L{:.3} {:.3} ", p.x, p.y)),
            PathEl::QuadTo(p1, p) => d.push_str(&format!("Q{:.3} {:.3} {:.3} {:.3} ", p1.x, p1.y, p.x, p.y)),
            PathEl::CurveTo(p1, p2, p) => d.push_str(&format!(
                "C{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} ",
                p1.x, p1.y, p2.x, p2.y, p.x, p.y
            )),
            PathEl::ClosePath => d.push_str("Z "),
        }
    }
    d.trim_end().to_string()
}

// sRGB color → SVG attributes. Hex color + a separate `*-opacity` for max compatibility (Inkscape).
fn fill_attrs(c: &ShapeColor) -> String {
    if c.a == 255 {
        format!(r##"fill="#{:02x}{:02x}{:02x}""##, c.r, c.g, c.b)
    } else {
        format!(r##"fill="#{:02x}{:02x}{:02x}" fill-opacity="{:.4}""##, c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}
fn stroke_attrs(c: &ShapeColor) -> String {
    if c.a == 255 {
        format!(r##"stroke="#{:02x}{:02x}{:02x}""##, c.r, c.g, c.b)
    } else {
        format!(r##"stroke="#{:02x}{:02x}{:02x}" stroke-opacity="{:.4}""##, c.r, c.g, c.b, c.a as f32 / 255.0)
    }
}
fn cap_str(cap: Cap) -> &'static str {
    match cap {
        Cap::Butt => "butt",
        Cap::Round => "round",
        Cap::Square => "square",
    }
}
fn join_str(join: Join) -> &'static str {
    match join {
        Join::Miter => "miter",
        Join::Round => "round",
        Join::Bevel => "bevel",
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use crate::shape::{ShapeColor, StrokeStyle};
    use crate::vector_graph::{Direction, VectorGraph};
    use kurbo::{CubicBez, Point};

    fn line(a: Point, b: Point) -> CubicBez {
        // Degenerate cubic representing a straight segment (matches our model).
        CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b)
    }

    #[test]
    fn solid_triangle_fill_and_stroke() {
        let mut g = VectorGraph::new();
        let p0 = Point::new(10.0, 10.0);
        let p1 = Point::new(90.0, 10.0);
        let p2 = Point::new(50.0, 80.0);
        let v0 = g.alloc_vertex(p0);
        let v1 = g.alloc_vertex(p1);
        let v2 = g.alloc_vertex(p2);
        let stroke = Some(StrokeStyle { width: 2.0, ..Default::default() });
        let scol = Some(ShapeColor::rgb(0, 0, 0));
        let e0 = g.alloc_edge(line(p0, p1), v0, v1, stroke.clone(), scol);
        let e1 = g.alloc_edge(line(p1, p2), v1, v2, stroke.clone(), scol);
        let e2 = g.alloc_edge(line(p2, p0), v2, v0, stroke.clone(), scol);
        g.alloc_fill(
            vec![(e0, Direction::Forward), (e1, Direction::Forward), (e2, Direction::Forward)],
            ShapeColor::rgb(255, 0, 0),
            crate::shape::FillRule::NonZero,
        );

        let mut body = String::new();
        let mut defs = String::new();
        let mut n = 0;
        vector_graph_to_svg(&g, &mut body, &mut defs, &mut n);

        assert!(body.contains(r##"fill="#ff0000""##), "fill color missing: {body}");
        assert!(body.contains(r#"fill-rule="nonzero""#), "fill-rule missing: {body}");
        assert!(body.contains(r#"fill="none""#), "stroke path missing: {body}");
        assert!(body.contains(r#"stroke-width="2.000""#), "stroke width missing: {body}");
        assert!(defs.is_empty(), "no gradients expected: {defs}");
        // 1 fill path + 3 stroked edges = 4 <path> elements.
        assert_eq!(body.matches("<path").count(), 4, "{body}");
    }
}
