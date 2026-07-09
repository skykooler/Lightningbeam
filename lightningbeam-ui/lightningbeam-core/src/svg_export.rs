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
/// Vector layers, groups of them, and text layers (as real glyph outlines) — raster/video/audio/
/// effect layers are skipped (a later pass can rasterize them to `<image>`). Animation is a single
/// static frame at `time`.
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
            if !vl.layer.visible {
                return; // hidden layers are not rendered, so don't export them
            }
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
            if !g.layer.visible {
                return;
            }
            // Render children first; only emit the <g> wrapper if it has exportable content
            // (avoids empty groups when every child is a non-vector/hidden layer).
            let mut inner = String::new();
            for child in &g.children {
                layer_to_svg(child, time, 1.0, &mut inner, defs, grad_n);
            }
            if !inner.is_empty() {
                let opacity = parent_opacity * g.layer.opacity;
                body.push_str(&format!(r#"<g opacity="{opacity:.4}">"#));
                body.push_str(&inner);
                body.push_str("</g>");
            }
        }
        AnyLayer::Text(tl) => text_layer_to_svg(tl, time, parent_opacity, body),
        // Raster/Video/Audio/Effect have no lossless vector representation — skipped this pass.
        _ => {}
    }
}

/// A skrifa outline pen that appends transformed glyph contours to an SVG path `d` string.
///
/// skrifa emits outline points in y-up pixel space (origin at the glyph baseline); this maps each
/// point into document space: `x = gx + px + skew·py`, `y = gy − py` (Y flips, `skew` applies any
/// synthetic-italic slant), where `(gx, gy)` is the glyph's document-space pen position.
struct SvgOutlinePen<'a> {
    gx: f64,
    gy: f64,
    skew: f64,
    d: &'a mut String,
}

impl<'a> SvgOutlinePen<'a> {
    fn map(&self, px: f32, py: f32) -> (f64, f64) {
        let (px, py) = (px as f64, py as f64);
        (self.gx + px + self.skew * py, self.gy - py)
    }
}

impl skrifa::outline::OutlinePen for SvgOutlinePen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.d.push_str(&format!("M{x:.2} {y:.2}"));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.d.push_str(&format!("L{x:.2} {y:.2}"));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (cx, cy) = self.map(cx, cy);
        let (x, y) = self.map(x, y);
        self.d.push_str(&format!("Q{cx:.2} {cy:.2} {x:.2} {y:.2}"));
    }
    fn curve_to(&mut self, c0x: f32, c0y: f32, c1x: f32, c1y: f32, x: f32, y: f32) {
        let (c0x, c0y) = self.map(c0x, c0y);
        let (c1x, c1y) = self.map(c1x, c1y);
        let (x, y) = self.map(x, y);
        self.d.push_str(&format!("C{c0x:.2} {c0y:.2} {c1x:.2} {c1y:.2} {x:.2} {y:.2}"));
    }
    fn close(&mut self) {
        self.d.push('Z');
    }
}

/// Append a text layer's glyphs to `body` as a single filled `<path>` of real glyph outlines
/// (lossless — no font dependency in the SVG). Lays the text out with the same parley path the
/// renderer uses, then extracts each glyph's outline with skrifa. Variable-font axis positions and
/// synthetic-italic skew are honored; synthetic bold is not (rare).
fn text_layer_to_svg(
    tl: &crate::text_layer::TextLayer,
    time: f64,
    parent_opacity: f64,
    body: &mut String,
) {
    use skrifa::MetadataProvider;

    if !tl.layer.visible {
        return;
    }
    let content = tl.content_at(time);
    if content.text.is_empty() {
        return;
    }

    let (ox, oy) = (tl.box_origin.x, tl.box_origin.y);
    let mut d = String::new();

    crate::fonts::with_layout(content, tl.box_width as f32, |layout| {
        for line in layout.lines() {
            for item in line.items() {
                let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else { continue };
                let run = glyph_run.run();
                let font = run.font();
                let font_size = run.font_size();
                let skew = run
                    .synthesis()
                    .skew()
                    .map(|angle| (angle as f64).to_radians().tan())
                    .unwrap_or(0.0);

                let Ok(font_ref) = skrifa::FontRef::from_index(font.data.data(), font.index) else {
                    continue;
                };
                let outlines = font_ref.outline_glyphs();

                // Variable-font axis position for this run (empty for static fonts).
                let coords: Vec<skrifa::instance::NormalizedCoord> = run
                    .normalized_coords()
                    .iter()
                    .map(|&c| skrifa::instance::NormalizedCoord::from_bits(c))
                    .collect();
                let location = skrifa::instance::LocationRef::new(&coords);
                let size = skrifa::instance::Size::new(font_size);

                for g in glyph_run.positioned_glyphs() {
                    let Some(glyph) = outlines.get(skrifa::GlyphId::new(g.id as u32)) else {
                        continue;
                    };
                    let mut pen = SvgOutlinePen {
                        gx: ox + g.x as f64,
                        gy: oy + g.y as f64,
                        skew,
                        d: &mut d,
                    };
                    let settings = skrifa::outline::DrawSettings::unhinted(size, location);
                    let _ = glyph.draw(settings, &mut pen);
                }
            }
        }
    });

    if d.is_empty() {
        return;
    }

    let [r, g, b, a] = content.color;
    let to_u8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    let fill_opacity = (a as f64 * parent_opacity * tl.layer.opacity).clamp(0.0, 1.0);
    body.push_str(&format!(
        r#"<path fill="rgb({},{},{})" fill-opacity="{:.4}" fill-rule="nonzero" d="{}"/>"#,
        to_u8(r), to_u8(g), to_u8(b), fill_opacity, d
    ));
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

    #[test]
    fn outline_pen_maps_yflip_and_skew() {
        use skrifa::outline::OutlinePen;
        let mut d = String::new();
        {
            let mut pen = SvgOutlinePen { gx: 10.0, gy: 100.0, skew: 0.0, d: &mut d };
            pen.move_to(0.0, 0.0); // baseline origin → (10, 100)
            pen.line_to(5.0, 20.0); // 20 up → y = 100 − 20 = 80
            pen.close();
        }
        assert!(d.contains("M10.00 100.00"), "d={d}");
        assert!(d.contains("L15.00 80.00"), "d={d}");
        assert!(d.ends_with('Z'));

        // Synthetic-italic skew shifts x right in proportion to height.
        let mut d2 = String::new();
        {
            let mut pen = SvgOutlinePen { gx: 0.0, gy: 0.0, skew: 0.5, d: &mut d2 };
            pen.move_to(0.0, 10.0); // x = 0 + 0.5·10 = 5, y = −10
        }
        assert!(d2.contains("M5.00 -10.00"), "d={d2}");
    }

    #[test]
    fn text_layer_emits_real_glyph_outlines() {
        use crate::text_layer::TextLayer;

        let mut tl = TextLayer::new("t", Point::new(20.0, 60.0));
        tl.content.text = "Hi".to_string();
        tl.content.font_size = 48.0;
        tl.content.color = [1.0, 0.0, 0.0, 1.0];

        let mut body = String::new();
        text_layer_to_svg(&tl, 0.0, 1.0, &mut body);

        // Bundled fonts guarantee glyphs → a filled path with actual outline segments.
        assert!(body.contains("<path"), "no path emitted: {body}");
        assert!(body.contains(r#"fill="rgb(255,0,0)""#), "wrong fill: {body}");
        assert!(
            body.contains('C') || body.contains('Q') || body.contains('L'),
            "path has no outline segments: {body}"
        );
        assert!(body.len() > 80, "suspiciously short path: {body}");
    }

    #[test]
    fn empty_text_layer_emits_nothing() {
        use crate::text_layer::TextLayer;
        let tl = TextLayer::new("t", Point::new(0.0, 0.0)); // no text set
        let mut body = String::new();
        text_layer_to_svg(&tl, 0.0, 1.0, &mut body);
        assert!(body.is_empty(), "empty text should emit nothing: {body}");
    }
}
