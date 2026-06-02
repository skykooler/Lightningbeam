//! Basic graph construction, vertex/edge/fill CRUD, adjacency, BezPath generation.

use super::super::*;
use kurbo::{CubicBez, Point};

/// Helper: create a straight-line cubic Bézier from a to b.
fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
        Point::new(a.x + 2.0 * (b.x - a.x) / 3.0, a.y + 2.0 * (b.y - a.y) / 3.0),
        b,
    )
}

// ── Vertex CRUD ──────────────────────────────────────────────────────────

#[test]
fn alloc_vertex() {
    let mut g = VectorGraph::new();
    let v = g.alloc_vertex(Point::new(10.0, 20.0));
    assert_eq!(g.vertex(v).position, Point::new(10.0, 20.0));
    assert!(!g.vertex(v).deleted);
}

#[test]
fn free_and_reuse_vertex() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(1.0, 2.0));
    g.free_vertex(v0);
    assert!(g.vertex(v0).deleted);

    // Next alloc should reuse the freed slot
    let v1 = g.alloc_vertex(Point::new(3.0, 4.0));
    assert_eq!(v0, v1);
    assert_eq!(g.vertex(v1).position, Point::new(3.0, 4.0));
    assert!(!g.vertex(v1).deleted);
}

// ── Edge CRUD ────────────────────────────────────────────────────────────

#[test]
fn alloc_edge_with_stroke() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(100.0, 0.0));
    let style = StrokeStyle { width: 2.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);

    let e = g.alloc_edge(line(Point::ZERO, Point::new(100.0, 0.0)), v0, v1, Some(style), Some(color));
    assert_eq!(g.edge(e).vertices, [v0, v1]);
    assert!(g.edge_is_visible(e));
}

#[test]
fn alloc_invisible_edge() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(50.0, 0.0));
    let e = g.alloc_edge(line(Point::ZERO, Point::new(50.0, 0.0)), v0, v1, None, None);
    assert!(!g.edge_is_visible(e));
}

// ── Fill CRUD ────────────────────────────────────────────────────────────

#[test]
fn alloc_fill_with_boundary() {
    let mut g = VectorGraph::new();

    // Build a triangle: 3 vertices, 3 edges
    let p0 = Point::new(0.0, 0.0);
    let p1 = Point::new(100.0, 0.0);
    let p2 = Point::new(50.0, 100.0);

    let v0 = g.alloc_vertex(p0);
    let v1 = g.alloc_vertex(p1);
    let v2 = g.alloc_vertex(p2);

    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(p0, p1), v0, v1, Some(style.clone()), Some(color));
    let e1 = g.alloc_edge(line(p1, p2), v1, v2, Some(style.clone()), Some(color));
    let e2 = g.alloc_edge(line(p2, p0), v2, v0, Some(style), Some(color));

    let boundary = vec![
        (e0, Direction::Forward),
        (e1, Direction::Forward),
        (e2, Direction::Forward),
    ];
    let fill_color = ShapeColor::rgb(255, 0, 0);
    let fid = g.alloc_fill(boundary, fill_color, FillRule::NonZero);

    assert_eq!(g.fill(fid).boundary.len(), 3);
    assert_eq!(g.fill(fid).color, Some(fill_color));
}

// ── Adjacency ────────────────────────────────────────────────────────────

#[test]
fn edges_at_vertex_finds_incident() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(50.0, 50.0));
    let v1 = g.alloc_vertex(Point::new(100.0, 50.0));
    let v2 = g.alloc_vertex(Point::new(50.0, 100.0));
    let v3 = g.alloc_vertex(Point::new(0.0, 50.0));

    let e0 = g.alloc_edge(line(Point::new(50.0, 50.0), Point::new(100.0, 50.0)), v0, v1, None, None);
    let e1 = g.alloc_edge(line(Point::new(50.0, 50.0), Point::new(50.0, 100.0)), v0, v2, None, None);
    let _e2 = g.alloc_edge(line(Point::new(0.0, 50.0), Point::new(100.0, 50.0)), v3, v1, None, None);

    let incident = g.edges_at_vertex(v0);
    assert_eq!(incident.len(), 2);
    assert!(incident.contains(&e0));
    assert!(incident.contains(&e1));
}

#[test]
fn vertices_share_edge_check() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(10.0, 0.0));
    let v2 = g.alloc_vertex(Point::new(20.0, 0.0));

    g.alloc_edge(line(Point::ZERO, Point::new(10.0, 0.0)), v0, v1, None, None);

    assert!(g.vertices_share_edge(v0, v1));
    assert!(g.vertices_share_edge(v1, v0)); // symmetric
    assert!(!g.vertices_share_edge(v0, v2));
}

// ── Edge visibility + deletion ───────────────────────────────────────────

#[test]
fn delete_visible_edge_without_fill_removes_it() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::ZERO);
    let v1 = g.alloc_vertex(Point::new(10.0, 0.0));
    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let e = g.alloc_edge(line(Point::ZERO, Point::new(10.0, 0.0)), v0, v1, Some(style), Some(ShapeColor::rgb(0, 0, 0)));

    g.delete_edge_by_user(e);
    assert!(g.edge(e).deleted);
}

#[test]
fn delete_edge_with_fill_makes_invisible() {
    let mut g = VectorGraph::new();

    // Triangle with a fill
    let p0 = Point::new(0.0, 0.0);
    let p1 = Point::new(100.0, 0.0);
    let p2 = Point::new(50.0, 100.0);
    let v0 = g.alloc_vertex(p0);
    let v1 = g.alloc_vertex(p1);
    let v2 = g.alloc_vertex(p2);
    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(p0, p1), v0, v1, Some(style.clone()), Some(color));
    let e1 = g.alloc_edge(line(p1, p2), v1, v2, Some(style.clone()), Some(color));
    let e2 = g.alloc_edge(line(p2, p0), v2, v0, Some(style), Some(color));

    let boundary = vec![
        (e0, Direction::Forward),
        (e1, Direction::Forward),
        (e2, Direction::Forward),
    ];
    let _fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    // Delete one edge — should become invisible, not deleted
    g.delete_edge_by_user(e0);
    assert!(!g.edge(e0).deleted, "edge should not be deleted while fill references it");
    assert!(!g.edge_is_visible(e0), "edge should be invisible");
}

#[test]
fn gc_removes_invisible_unreferenced_edges() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::ZERO);
    let v1 = g.alloc_vertex(Point::new(10.0, 0.0));

    // Invisible edge with no fill referencing it
    let e = g.alloc_edge(line(Point::ZERO, Point::new(10.0, 0.0)), v0, v1, None, None);
    assert!(!g.edge(e).deleted);

    g.gc_invisible_edges();
    assert!(g.edge(e).deleted, "invisible unreferenced edge should be garbage collected");
}

// ── BezPath generation ───────────────────────────────────────────────────

#[test]
fn fill_to_bezpath_generates_closed_path() {
    let mut g = VectorGraph::new();

    // Square
    let tl = Point::new(0.0, 0.0);
    let tr = Point::new(100.0, 0.0);
    let br = Point::new(100.0, 100.0);
    let bl = Point::new(0.0, 100.0);

    let v_tl = g.alloc_vertex(tl);
    let v_tr = g.alloc_vertex(tr);
    let v_br = g.alloc_vertex(br);
    let v_bl = g.alloc_vertex(bl);

    let e0 = g.alloc_edge(line(tl, tr), v_tl, v_tr, None, None);
    let e1 = g.alloc_edge(line(tr, br), v_tr, v_br, None, None);
    let e2 = g.alloc_edge(line(br, bl), v_br, v_bl, None, None);
    let e3 = g.alloc_edge(line(bl, tl), v_bl, v_tl, None, None);

    let boundary = vec![
        (e0, Direction::Forward),
        (e1, Direction::Forward),
        (e2, Direction::Forward),
        (e3, Direction::Forward),
    ];
    let fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    let path = g.fill_to_bezpath(fid);
    let elements: Vec<_> = path.elements().to_vec();

    // Should be: MoveTo, CurveTo x4, ClosePath
    assert_eq!(elements.len(), 6);
    assert!(matches!(elements[0], kurbo::PathEl::MoveTo(_)));
    assert!(matches!(elements[5], kurbo::PathEl::ClosePath));
}

#[test]
fn fill_to_bezpath_respects_direction() {
    let mut g = VectorGraph::new();

    let p0 = Point::new(0.0, 0.0);
    let p1 = Point::new(100.0, 0.0);
    let v0 = g.alloc_vertex(p0);
    let v1 = g.alloc_vertex(p1);
    let e = g.alloc_edge(line(p0, p1), v0, v1, None, None);

    // Forward: start at p0
    let fwd_boundary = vec![(e, Direction::Forward)];
    let fid_fwd = g.alloc_fill(fwd_boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);
    let path_fwd = g.fill_to_bezpath(fid_fwd);
    if let kurbo::PathEl::MoveTo(start) = path_fwd.elements()[0] {
        assert!((start.x - p0.x).abs() < 0.01);
    }

    // Backward: start at p1
    let bwd_boundary = vec![(e, Direction::Backward)];
    let fid_bwd = g.alloc_fill(bwd_boundary, ShapeColor::rgb(0, 255, 0), FillRule::NonZero);
    let path_bwd = g.fill_to_bezpath(fid_bwd);
    if let kurbo::PathEl::MoveTo(start) = path_bwd.elements()[0] {
        assert!((start.x - p1.x).abs() < 0.01);
    }
}
