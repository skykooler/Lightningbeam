//! Stroke insertion with intersection detection and curve splitting.

use super::super::*;
use kurbo::{CubicBez, Point};

fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
        Point::new(a.x + 2.0 * (b.x - a.x) / 3.0, a.y + 2.0 * (b.y - a.y) / 3.0),
        b,
    )
}

fn black_stroke() -> (Option<StrokeStyle>, Option<ShapeColor>) {
    (Some(StrokeStyle { width: 2.0, ..Default::default() }), Some(ShapeColor::rgb(0, 0, 0)))
}

// ── Single stroke ────────────────────────────────────────────────────────

#[test]
fn insert_single_line_creates_edge() {
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();
    let edges = g.insert_stroke(
        &[line(Point::new(0.0, 0.0), Point::new(100.0, 0.0))],
        style,
        color,
        0.5,
    );
    assert_eq!(edges.len(), 1);
    assert!(g.edge_is_visible(edges[0]));

    // Should have 2 vertices (endpoints)
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    assert_eq!(live_verts, 2);
}

#[test]
fn insert_multi_segment_stroke_creates_chain() {
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();
    let segments = vec![
        line(Point::new(0.0, 0.0), Point::new(50.0, 0.0)),
        line(Point::new(50.0, 0.0), Point::new(100.0, 50.0)),
        line(Point::new(100.0, 50.0), Point::new(100.0, 100.0)),
    ];
    let edges = g.insert_stroke(&segments, style, color, 0.5);
    assert_eq!(edges.len(), 3);

    // Should have 4 vertices (start + 2 intermediate + end)
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    assert_eq!(live_verts, 4);
}

#[test]
fn insert_stroke_snaps_to_existing_vertex() {
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // First stroke: (0,0) → (100,0)
    g.insert_stroke(
        &[line(Point::new(0.0, 0.0), Point::new(100.0, 0.0))],
        style.clone(),
        color,
        0.5,
    );

    // Second stroke starts very close to (100,0) — should snap, not create new vertex
    g.insert_stroke(
        &[line(Point::new(100.2, 0.1), Point::new(100.0, 100.0))],
        style,
        color,
        0.5,
    );

    // Should have 3 vertices, not 4 (the near-endpoint was snapped)
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    assert_eq!(live_verts, 3);
}

// ── Intersection splitting ───────────────────────────────────────────────

#[test]
fn crossing_strokes_creates_intersection_vertex() {
    // Two perpendicular lines forming a +
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Horizontal: (0, 50) → (100, 50)
    g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(100.0, 50.0))],
        style.clone(),
        color,
        0.5,
    );

    // Vertical: (50, 0) → (50, 100) — crosses the horizontal at (50, 50)
    g.insert_stroke(
        &[line(Point::new(50.0, 0.0), Point::new(50.0, 100.0))],
        style,
        color,
        0.5,
    );

    // The horizontal should have been split into 2 edges
    // The vertical should be split into 2 edges
    // Total: 4 edges, 5 vertices (4 endpoints + 1 intersection)
    let live_edges = g.edges.iter().filter(|e| !e.deleted).count();
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    assert_eq!(live_edges, 4, "two lines crossing = 4 sub-edges");
    assert_eq!(live_verts, 5, "4 endpoints + 1 intersection vertex");

    // The intersection vertex should be near (50, 50)
    let intersection_v = g.vertices.iter().find(|v| {
        !v.deleted
            && (v.position.x - 50.0).abs() < 1.0
            && (v.position.y - 50.0).abs() < 1.0
    });
    assert!(intersection_v.is_some(), "should have a vertex near (50, 50)");
}

#[test]
fn hash_structure_four_crossing_edges() {
    // Four lines creating a # pattern:
    //   Two horizontal, two vertical — 4 intersection points
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Horizontal lines
    g.insert_stroke(
        &[line(Point::new(0.0, 30.0), Point::new(100.0, 30.0))],
        style.clone(), color, 0.5,
    );
    g.insert_stroke(
        &[line(Point::new(0.0, 70.0), Point::new(100.0, 70.0))],
        style.clone(), color, 0.5,
    );

    // Vertical lines — each crosses both horizontals
    g.insert_stroke(
        &[line(Point::new(30.0, 0.0), Point::new(30.0, 100.0))],
        style.clone(), color, 0.5,
    );
    g.insert_stroke(
        &[line(Point::new(70.0, 0.0), Point::new(70.0, 100.0))],
        style, color, 0.5,
    );

    // 4 intersection vertices + 8 endpoints = 12 vertices
    // Each of the 4 original lines is split into 3 sub-edges = 12 edges
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    let live_edges = g.edges.iter().filter(|e| !e.deleted).count();
    assert_eq!(live_verts, 12);
    assert_eq!(live_edges, 12);
}

#[test]
fn self_intersecting_stroke_splits() {
    // A curve that crosses itself (figure-8 like).
    // We approximate with line segments forming an X.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    let segments = vec![
        line(Point::new(0.0, 0.0), Point::new(100.0, 100.0)),
        line(Point::new(100.0, 100.0), Point::new(100.0, 0.0)),
        line(Point::new(100.0, 0.0), Point::new(0.0, 100.0)),
    ];

    let edges = g.insert_stroke(&segments, style, color, 0.5);

    // The first segment (0,0)→(100,100) and third segment (100,0)→(0,100)
    // cross near (50, 50). This splits both into 2 sub-edges each,
    // plus the middle segment (100,100)→(100,0) is untouched.
    // Total: 2 + 1 + 2 = 5 edges, 4 corners + 1 self-intersection = 5 vertices.
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    let live_edges = g.edges.iter().filter(|e| !e.deleted).count();
    assert_eq!(
        live_verts, 5,
        "should have 5 vertices (4 corners + 1 self-intersection), got {live_verts}"
    );
    assert_eq!(
        live_edges, 5,
        "should have 5 edges (2 split + 1 unsplit + 2 split), got {live_edges}"
    );
}

// ── Edge splitting preserves fills ───────────────────────────────────────

#[test]
fn split_edge_updates_fill_boundary() {
    let mut g = VectorGraph::new();

    // Build a square manually
    let tl = Point::new(0.0, 0.0);
    let tr = Point::new(100.0, 0.0);
    let br = Point::new(100.0, 100.0);
    let bl = Point::new(0.0, 100.0);

    let v_tl = g.alloc_vertex(tl);
    let v_tr = g.alloc_vertex(tr);
    let v_br = g.alloc_vertex(br);
    let v_bl = g.alloc_vertex(bl);

    let e_top = g.alloc_edge(line(tl, tr), v_tl, v_tr, None, None);
    let e_right = g.alloc_edge(line(tr, br), v_tr, v_br, None, None);
    let e_bottom = g.alloc_edge(line(br, bl), v_br, v_bl, None, None);
    let e_left = g.alloc_edge(line(bl, tl), v_bl, v_tl, None, None);

    let boundary = vec![
        (e_top, Direction::Forward),
        (e_right, Direction::Forward),
        (e_bottom, Direction::Forward),
        (e_left, Direction::Forward),
    ];
    let fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    // Split the top edge at t=0.5
    let (_mid_v, sub_a, sub_b) = g.split_edge(e_top, 0.5);

    // The fill should now reference sub_a and sub_b instead of e_top
    let fill = g.fill(fid);
    assert_eq!(fill.boundary.len(), 5, "boundary should grow from 4 to 5 edges");
    assert!(
        fill.boundary.iter().any(|(eid, _)| *eid == sub_a),
        "fill should reference first sub-edge"
    );
    assert!(
        fill.boundary.iter().any(|(eid, _)| *eid == sub_b),
        "fill should reference second sub-edge"
    );
    assert!(
        !fill.boundary.iter().any(|(eid, _)| *eid == e_top),
        "fill should no longer reference the original edge"
    );
}
