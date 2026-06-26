//! SVG import → `VectorGraph`.
//!
//! Parses an `.svg` with usvg (which resolves CSS, converts shapes/rects/circles to
//! paths, and computes absolute transforms), then bakes each path's absolute transform
//! into geometry and builds a single [`VectorGraph`] that becomes one new vector layer.
//!
//! Scope (matches the export pass): paths with solid/gradient fills and strokes. `<image>`
//! and `<text>` nodes are skipped, and nested groups are flattened (their transforms are
//! already baked into each path's `abs_transform`).
//!
//! Known limitation: imported edges are NOT intersection-split, so the paint-bucket tool
//! may need to re-process imported art. Display, transform, and round-trip are fine.

use kurbo::{CubicBez, Point as KPoint};
use lightningbeam_core::gradient::{GradientExtend, GradientStop, GradientType, ShapeGradient};
use lightningbeam_core::shape::{Cap, FillRule, Join, ShapeColor, StrokeStyle};
use lightningbeam_core::vector_graph::{Direction, EdgeId, VectorGraph, VertexId};
use resvg::usvg;
use usvg::tiny_skia_path::{PathSegment, Point as SkPoint};

/// Parse SVG bytes into a single flattened [`VectorGraph`] in document (canvas) space.
pub fn import_svg(bytes: &[u8]) -> Result<VectorGraph, String> {
    let tree = usvg::Tree::from_data(bytes, &usvg::Options::default())
        .map_err(|e| format!("Failed to parse SVG: {e}"))?;
    let mut graph = VectorGraph::new();
    walk_group(tree.root(), &mut graph);
    if graph.edges.is_empty() {
        return Err("SVG contained no importable vector paths".to_string());
    }
    Ok(graph)
}

fn walk_group(group: &usvg::Group, graph: &mut VectorGraph) {
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => walk_group(g, graph),
            usvg::Node::Path(p) => convert_path(p, graph),
            usvg::Node::Image(_) | usvg::Node::Text(_) => {} // skipped this pass
        }
    }
}

fn convert_path(path: &usvg::Path, graph: &mut VectorGraph) {
    if !path.is_visible() {
        return;
    }
    let ts = path.abs_transform();
    // Bake the absolute transform into the geometry so everything lives in canvas space.
    let Some(data) = path.data().clone().transform(ts) else {
        return;
    };

    // One stroke style/colour shared by every edge of this path.
    let stroke = path.stroke().map(|s| stroke_to_style(s, ts));

    // Walk the (transformed) segments, allocating vertices/edges and recording the
    // boundary cycle. `EdgeId::NONE` separates subpaths (outer contour + holes).
    let mut boundary: Vec<(EdgeId, Direction)> = Vec::new();
    let mut have_subpath = false;
    let mut cur_v = VertexId(0);
    let mut cur_p = SkPoint::from_xy(0.0, 0.0);
    let mut start_v = VertexId(0);
    let mut start_p = SkPoint::from_xy(0.0, 0.0);

    for seg in data.segments() {
        match seg {
            PathSegment::MoveTo(p) => {
                if have_subpath {
                    boundary.push((EdgeId::NONE, Direction::Forward));
                }
                let v = graph.alloc_vertex(kp(p));
                cur_v = v;
                cur_p = p;
                start_v = v;
                start_p = p;
                have_subpath = true;
            }
            PathSegment::LineTo(p) => {
                let (c1, c2) = line_ctrls(cur_p, p);
                cur_v = add_edge(graph, &mut boundary, cur_v, cur_p, c1, c2, p, &stroke);
                cur_p = p;
            }
            PathSegment::QuadTo(c, p) => {
                let (c1, c2) = quad_to_cubic(cur_p, c, p);
                cur_v = add_edge(graph, &mut boundary, cur_v, cur_p, c1, c2, p, &stroke);
                cur_p = p;
            }
            PathSegment::CubicTo(c1, c2, p) => {
                cur_v = add_edge(graph, &mut boundary, cur_v, cur_p, c1, c2, p, &stroke);
                cur_p = p;
            }
            PathSegment::Close => {
                // Close back to the subpath start (reusing its vertex) unless already there.
                if cur_p != start_p {
                    let (c1, c2) = line_ctrls(cur_p, start_p);
                    let curve = CubicBez::new(kp(cur_p), kp(c1), kp(c2), kp(start_p));
                    let (style, color) = split_stroke(&stroke);
                    let e = graph.alloc_edge(curve, cur_v, start_v, style, color);
                    boundary.push((e, Direction::Forward));
                }
                cur_v = start_v;
                cur_p = start_p;
            }
        }
    }

    // Fill (if any) references the whole boundary cycle.
    if let Some(fill) = path.fill() {
        if !boundary.is_empty() {
            let rule = match fill.rule() {
                usvg::FillRule::NonZero => FillRule::NonZero,
                usvg::FillRule::EvenOdd => FillRule::EvenOdd,
            };
            let fid = graph.alloc_fill(boundary, None, rule);
            let slot = &mut graph.fills[fid.idx()];
            match fill.paint() {
                usvg::Paint::Color(c) => {
                    slot.color = Some(ShapeColor::rgba(c.red, c.green, c.blue, opacity_u8(fill.opacity())));
                }
                usvg::Paint::LinearGradient(g) => {
                    slot.gradient_fill = Some(linear_gradient(g, ts));
                }
                usvg::Paint::RadialGradient(g) => {
                    slot.gradient_fill = Some(radial_gradient(g, ts));
                }
                usvg::Paint::Pattern(_) => {
                    // Patterns aren't representable yet — neutral gray so the shape stays visible.
                    slot.color = Some(ShapeColor::rgba(128, 128, 128, opacity_u8(fill.opacity())));
                }
            }
        }
    }
}

/// Allocate the end vertex + a cubic edge from `av`/`ap` to `bp`, recording it on the boundary.
fn add_edge(
    graph: &mut VectorGraph,
    boundary: &mut Vec<(EdgeId, Direction)>,
    av: VertexId,
    ap: SkPoint,
    c1: SkPoint,
    c2: SkPoint,
    bp: SkPoint,
    stroke: &Option<(StrokeStyle, ShapeColor)>,
) -> VertexId {
    let bv = graph.alloc_vertex(kp(bp));
    let curve = CubicBez::new(kp(ap), kp(c1), kp(c2), kp(bp));
    let (style, color) = split_stroke(stroke);
    let e = graph.alloc_edge(curve, av, bv, style, color);
    boundary.push((e, Direction::Forward));
    bv
}

fn split_stroke(stroke: &Option<(StrokeStyle, ShapeColor)>) -> (Option<StrokeStyle>, Option<ShapeColor>) {
    match stroke {
        Some((s, c)) => (Some(s.clone()), Some(*c)),
        None => (None, None),
    }
}

fn stroke_to_style(s: &usvg::Stroke, ts: usvg::Transform) -> (StrokeStyle, ShapeColor) {
    let scale = transform_scale(ts) as f64;
    let style = StrokeStyle {
        width: s.width().get() as f64 * scale,
        cap: match s.linecap() {
            usvg::LineCap::Butt => Cap::Butt,
            usvg::LineCap::Round => Cap::Round,
            usvg::LineCap::Square => Cap::Square,
        },
        join: match s.linejoin() {
            usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => Join::Miter,
            usvg::LineJoin::Round => Join::Round,
            usvg::LineJoin::Bevel => Join::Bevel,
        },
        miter_limit: s.miterlimit().get() as f64,
    };
    let color = match s.paint() {
        usvg::Paint::Color(c) => ShapeColor::rgba(c.red, c.green, c.blue, opacity_u8(s.opacity())),
        // Gradient/pattern strokes aren't representable per-edge — fall back to opaque black.
        _ => ShapeColor::rgba(0, 0, 0, opacity_u8(s.opacity())),
    };
    (style, color)
}

/// Geometric-mean scale of the transform's linear part (for stroke-width baking).
fn transform_scale(ts: usvg::Transform) -> f32 {
    (ts.sx * ts.sy - ts.kx * ts.ky).abs().sqrt()
}

fn linear_gradient(g: &usvg::LinearGradient, abs: usvg::Transform) -> ShapeGradient {
    let ct = abs.pre_concat(g.transform());
    let start = map_pt(ct, g.x1(), g.y1());
    let end = map_pt(ct, g.x2(), g.y2());
    let angle = (end.1 - start.1).atan2(end.0 - start.0).to_degrees() as f32;
    ShapeGradient {
        kind: GradientType::Linear,
        stops: gradient_stops(g),
        angle,
        extend: spread(g),
        start_world: Some(start),
        end_world: Some(end),
    }
}

fn radial_gradient(g: &usvg::RadialGradient, abs: usvg::Transform) -> ShapeGradient {
    let ct = abs.pre_concat(g.transform());
    // Our model stores center as start_world and a rim point (defining the radius) as end_world.
    let center = map_pt(ct, g.cx(), g.cy());
    let rim = map_pt(ct, g.cx() + g.r().get(), g.cy());
    ShapeGradient {
        kind: GradientType::Radial,
        stops: gradient_stops(g),
        angle: 0.0,
        extend: spread(g),
        start_world: Some(center),
        end_world: Some(rim),
    }
}

fn gradient_stops(base: &usvg::BaseGradient) -> Vec<GradientStop> {
    base.stops()
        .iter()
        .map(|s| GradientStop {
            position: s.offset().get(),
            color: ShapeColor::rgba(s.color().red, s.color().green, s.color().blue, opacity_u8(s.opacity())),
        })
        .collect()
}

fn spread(base: &usvg::BaseGradient) -> GradientExtend {
    match base.spread_method() {
        usvg::SpreadMethod::Pad => GradientExtend::Pad,
        usvg::SpreadMethod::Reflect => GradientExtend::Reflect,
        usvg::SpreadMethod::Repeat => GradientExtend::Repeat,
    }
}

// ── small geometry helpers ──────────────────────────────────────────────────

fn kp(p: SkPoint) -> KPoint {
    KPoint::new(p.x as f64, p.y as f64)
}

fn map_pt(ts: usvg::Transform, x: f32, y: f32) -> (f64, f64) {
    let mut p = SkPoint::from_xy(x, y);
    ts.map_point(&mut p);
    (p.x as f64, p.y as f64)
}

fn lerp(a: SkPoint, b: SkPoint, t: f32) -> SkPoint {
    SkPoint::from_xy(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Degenerate cubic control points for a straight segment (matches our edge model).
fn line_ctrls(a: SkPoint, b: SkPoint) -> (SkPoint, SkPoint) {
    (lerp(a, b, 1.0 / 3.0), lerp(a, b, 2.0 / 3.0))
}

/// Elevate a quadratic Bézier to a cubic.
fn quad_to_cubic(a: SkPoint, c: SkPoint, b: SkPoint) -> (SkPoint, SkPoint) {
    let c1 = SkPoint::from_xy(a.x + 2.0 / 3.0 * (c.x - a.x), a.y + 2.0 / 3.0 * (c.y - a.y));
    let c2 = SkPoint::from_xy(b.x + 2.0 / 3.0 * (c.x - b.x), b.y + 2.0 / 3.0 * (c.y - b.y));
    (c1, c2)
}

fn opacity_u8(o: usvg::Opacity) -> u8 {
    (o.get() * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_solid_rect_fill() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><rect x="10" y="10" width="80" height="80" fill="#ff0000"/></svg>"##;
        let g = import_svg(svg).expect("import");
        assert!(!g.edges.is_empty(), "expected edges from the rect");
        let fills: Vec<_> = g.fills.iter().filter(|f| !f.deleted).collect();
        assert_eq!(fills.len(), 1, "one fill expected");
        let c = fills[0].color.expect("solid color");
        assert_eq!((c.r, c.g, c.b), (255, 0, 0), "red fill");
    }

    #[test]
    fn imports_stroke_only() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="M0 0 L50 50" fill="none" stroke="#00ff00" stroke-width="3"/></svg>"##;
        let g = import_svg(svg).expect("import");
        let stroked = g.edges.iter().filter(|e| !e.deleted && e.stroke_color.is_some()).count();
        assert!(stroked >= 1, "expected at least one stroked edge");
        let c = g.edges.iter().find_map(|e| e.stroke_color).unwrap();
        assert_eq!((c.r, c.g, c.b), (0, 255, 0), "green stroke");
    }

    #[test]
    fn imports_linear_gradient() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <defs><linearGradient id="g" x1="0" y1="0" x2="100" y2="0">
              <stop offset="0" stop-color="#ff0000"/><stop offset="1" stop-color="#0000ff"/>
            </linearGradient></defs>
            <rect x="0" y="0" width="100" height="100" fill="url(#g)"/></svg>"##;
        let g = import_svg(svg).expect("import");
        let fills: Vec<_> = g.fills.iter().filter(|f| !f.deleted).collect();
        assert_eq!(fills.len(), 1);
        let grad = fills[0].gradient_fill.as_ref().expect("gradient");
        assert_eq!(grad.stops.len(), 2);
        assert!(grad.start_world.is_some() && grad.end_world.is_some());
    }

    #[test]
    fn empty_svg_errors() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"></svg>"#;
        assert!(import_svg(svg).is_err());
    }
}
