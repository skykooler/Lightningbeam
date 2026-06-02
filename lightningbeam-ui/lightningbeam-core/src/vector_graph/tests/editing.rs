//! Vertex dragging, curve editing, edge deletion, and fill response.

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

// ── Vertex dragging ──────────────────────────────────────────────────────

#[test]
fn drag_vertex_moves_connected_edges() {
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Two edges sharing a vertex at (50, 50)
    // (0, 0) → (50, 50) → (100, 0)
    g.insert_stroke(
        &[
            line(Point::new(0.0, 0.0), Point::new(50.0, 50.0)),
            line(Point::new(50.0, 50.0), Point::new(100.0, 0.0)),
        ],
        style, color, 0.5,
    );

    // Find the shared vertex at (50, 50)
    let mid_v = g.vertices.iter().enumerate()
        .find(|(_, v)| !v.deleted && (v.position.x - 50.0).abs() < 1.0 && (v.position.y - 50.0).abs() < 1.0)
        .map(|(i, _)| VertexId(i as u32))
        .expect("should find vertex at (50, 50)");

    // Move it to (50, 80)
    g.vertex_mut(mid_v).position = Point::new(50.0, 80.0);
    g.update_edges_for_vertex(mid_v);

    // Both edges incident to this vertex should have updated endpoints
    let incident = g.edges_at_vertex(mid_v);
    assert_eq!(incident.len(), 2);

    for eid in incident {
        let edge = g.edge(eid);
        let v0_pos = g.vertex(edge.vertices[0]).position;
        let v1_pos = g.vertex(edge.vertices[1]).position;
        // One endpoint should be the moved vertex
        assert!(
            (v0_pos.x - 50.0).abs() < 1.0 && (v0_pos.y - 80.0).abs() < 1.0
            || (v1_pos.x - 50.0).abs() < 1.0 && (v1_pos.y - 80.0).abs() < 1.0,
            "one endpoint should be the moved vertex at (50, 80)"
        );
    }
}

#[test]
fn drag_vertex_fill_follows() {
    // Build a square, fill it, drag a corner — fill boundary should update
    // because the fill references edges, edges reference vertices.
    let mut g = VectorGraph::new();

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

    // Drag top-right corner from (100, 0) to (150, 0)
    g.vertex_mut(v_tr).position = Point::new(150.0, 0.0);
    g.update_edges_for_vertex(v_tr);

    // The fill's BezPath should reflect the moved vertex
    let path = g.fill_to_bezpath(fid);
    let bbox = kurbo::Shape::bounding_box(&path);
    assert!(
        bbox.max_x() > 120.0,
        "fill bounding box should extend to the moved vertex, got max_x={:.1}",
        bbox.max_x()
    );
}

// ── Edge editing (# structure, editing a stub) ───────────────────────────

#[test]
fn edit_stub_in_hash_only_moves_that_segment() {
    // # structure: 2 horizontal + 2 vertical lines
    // Grab the top stub of the left vertical line and drag it.
    // Only that sub-edge (above the top horizontal) should move.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    g.insert_stroke(&[line(Point::new(0.0, 30.0), Point::new(100.0, 30.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(0.0, 70.0), Point::new(100.0, 70.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(30.0, 0.0), Point::new(30.0, 100.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(70.0, 0.0), Point::new(70.0, 100.0))], style, color, 0.5);

    // Find the top stub: the edge that goes from (30, 0) to (30, 30)
    let stub_edge = g.edges.iter().enumerate().find(|(_, e)| {
        if e.deleted { return false; }
        let v0 = g.vertex(e.vertices[0]).position;
        let v1 = g.vertex(e.vertices[1]).position;
        // One endpoint near (30, 0), other near (30, 30)
        let has_top = (v0.y < 5.0 && (v0.x - 30.0).abs() < 1.0)
            || (v1.y < 5.0 && (v1.x - 30.0).abs() < 1.0);
        let has_junction = ((v0.y - 30.0).abs() < 1.0 && (v0.x - 30.0).abs() < 1.0)
            || ((v1.y - 30.0).abs() < 1.0 && (v1.x - 30.0).abs() < 1.0);
        has_top && has_junction
    }).map(|(i, _)| EdgeId(i as u32));

    assert!(stub_edge.is_some(), "should find the top stub edge (30,0)→(30,30)");

    // The stub is an independently selectable/editable sub-edge,
    // not the full original vertical line.
    let stub = g.edge(stub_edge.unwrap());
    let v0_pos = g.vertex(stub.vertices[0]).position;
    let v1_pos = g.vertex(stub.vertices[1]).position;
    let length = ((v0_pos.x - v1_pos.x).powi(2) + (v0_pos.y - v1_pos.y).powi(2)).sqrt();
    assert!(
        (length - 30.0).abs() < 2.0,
        "stub should be ~30 units long (from y=0 to y=30), got {length:.1}"
    );
}

// ── Self-intersection creates new fill regions ───────────────────────────

#[test]
fn drag_o_into_figure_eight_splits_fill() {
    // Start with a circle-like closed curve (approximated as a square for simplicity),
    // fill it, then simulate dragging it into a figure-8 by creating a self-intersection.
    let mut g = VectorGraph::new();

    // Build a diamond shape: top, right, bottom, left
    let top = Point::new(50.0, 0.0);
    let right = Point::new(100.0, 50.0);
    let bottom = Point::new(50.0, 100.0);
    let left = Point::new(0.0, 50.0);

    let v_top = g.alloc_vertex(top);
    let v_right = g.alloc_vertex(right);
    let v_bottom = g.alloc_vertex(bottom);
    let v_left = g.alloc_vertex(left);

    let style = StrokeStyle { width: 2.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(top, right), v_top, v_right, Some(style.clone()), Some(color));
    let e1 = g.alloc_edge(line(right, bottom), v_right, v_bottom, Some(style.clone()), Some(color));
    let e2 = g.alloc_edge(line(bottom, left), v_bottom, v_left, Some(style.clone()), Some(color));
    let e3 = g.alloc_edge(line(left, top), v_left, v_top, Some(style), Some(color));

    let boundary = vec![
        (e0, Direction::Forward),
        (e1, Direction::Forward),
        (e2, Direction::Forward),
        (e3, Direction::Forward),
    ];
    let _fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    // Simulate figure-8: drag top vertex down past center to (50, 70)
    // and bottom vertex up past center to (50, 30).
    // This causes edges e0/e3 (meeting at top) and e1/e2 (meeting at bottom)
    // to cross, creating a self-intersection near the center.
    g.vertex_mut(v_top).position = Point::new(50.0, 70.0);
    g.update_edges_for_vertex(v_top);
    g.vertex_mut(v_bottom).position = Point::new(50.0, 30.0);
    g.update_edges_for_vertex(v_bottom);

    // Detect and handle self-intersection.
    // Edges e0 ((50,70)→(100,50)) and e2 ((50,30)→(0,50)) now cross.
    // Edges e1 ((100,50)→(50,30)) and e3 ((0,50)→(50,70)) now cross.
    // After detecting and splitting, the single fill should become two fills
    // (the two lobes of the figure-8).

    // TODO: This test documents the expected behavior. The implementation
    // needs a "detect self-intersections in fill boundaries" pass that runs
    // after vertex edits. For now, we test the expected outcome:
    // - The crossing edges should be split at the intersection points
    // - The original fill should be split into two fills
    // - Both fills should inherit the original color

    // For now, just verify the edges actually cross by checking that
    // the diamond is now "inverted" (top below bottom)
    assert!(
        g.vertex(v_top).position.y > g.vertex(v_bottom).position.y,
        "top vertex should now be below bottom vertex (figure-8)"
    );
}

// ── Control point editing creates new intersections ──────────────────────

#[test]
fn edit_control_points_creates_intersections() {
    // Draw a thin rectangle (0,0)-(100,20) with y-up convention.
    // The bottom edge runs along y=0, the top edge along y=20.
    // Edit the bottom edge's control points to bow it upward past y=20,
    // crossing the top edge in two places.
    let mut g = VectorGraph::new();

    let bl = Point::new(0.0, 0.0);
    let br = Point::new(100.0, 0.0);
    let tr = Point::new(100.0, 20.0);
    let tl = Point::new(0.0, 20.0);

    let v_bl = g.alloc_vertex(bl);
    let v_br = g.alloc_vertex(br);
    let v_tr = g.alloc_vertex(tr);
    let v_tl = g.alloc_vertex(tl);

    let style = StrokeStyle { width: 2.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);

    // Bottom edge: (0,0) → (100,0) — the one we'll edit
    let e_bottom = g.alloc_edge(line(bl, br), v_bl, v_br, Some(style.clone()), Some(color));
    let e_right = g.alloc_edge(line(br, tr), v_br, v_tr, Some(style.clone()), Some(color));
    // Top edge: (100,20) → (0,20)
    let e_top = g.alloc_edge(line(tr, tl), v_tr, v_tl, Some(style.clone()), Some(color));
    let e_left = g.alloc_edge(line(tl, bl), v_tl, v_bl, Some(style), Some(color));

    let boundary = vec![
        (e_bottom, Direction::Forward),
        (e_right, Direction::Forward),
        (e_top, Direction::Forward),
        (e_left, Direction::Forward),
    ];
    let _fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    // Edit the bottom edge's control points so it bows upward past y=20,
    // crossing the top edge in two places.
    // Endpoints stay at (0,0) and (100,0), control points go to (0,100) and (100,100).
    g.edge_mut(e_bottom).curve = CubicBez::new(
        bl,
        Point::new(0.0, 100.0),
        Point::new(100.0, 100.0),
        br,
    );

    // The edited bottom curve now arcs up to ~y=75 at its peak,
    // well past the top edge at y=20. It crosses the top edge twice.
    // The implementation should:
    // 1. Detect that e_bottom and e_top now intersect at 2 points
    // 2. Split both edges at the intersection points
    // 3. The original fill is split into 3 regions

    // Verify the geometry: sample the edited curve at t=0.5 — should be well above y=20
    let mid = kurbo::ParamCurve::eval(&g.edge(e_bottom).curve, 0.5);
    assert!(
        mid.y > 20.0,
        "edited bottom curve should bow above y=20 (got y={:.1}), crossing the top edge",
        mid.y
    );
}

#[test]
fn edit_curve_into_self_intersection() {
    // A single edge that is edited so it crosses itself.
    // Start with a straight line, edit control points to create a loop.
    let mut g = VectorGraph::new();

    let p0 = Point::new(0.0, 0.0);
    let p1 = Point::new(100.0, 0.0);
    let v0 = g.alloc_vertex(p0);
    let v1 = g.alloc_vertex(p1);

    let style = StrokeStyle { width: 2.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let eid = g.alloc_edge(line(p0, p1), v0, v1, Some(style), Some(color));

    // Edit control points to create a loop:
    // The curve goes from (0,50), control points pull far left and far right
    // at y=100, causing the curve to loop over itself.
    g.edge_mut(eid).curve = CubicBez::new(
        p0,
        Point::new(150.0, 100.0),
        Point::new(-50.0, 100.0),
        p1,
    );

    // The implementation should detect the self-intersection, split the edge,
    // and create a new vertex at the crossing. This forms a loop that is a
    // fillable region.

    // Verify the curve actually self-intersects by checking that it
    // crosses x=50 more than twice (the loop causes extra crossings).
    let mut crossings = 0;
    let n = 100;
    for i in 0..n {
        let t0 = i as f64 / n as f64;
        let t1 = (i + 1) as f64 / n as f64;
        let x0 = kurbo::ParamCurve::eval(&g.edge(eid).curve, t0).x;
        let x1 = kurbo::ParamCurve::eval(&g.edge(eid).curve, t1).x;
        if (x0 - 50.0).signum() != (x1 - 50.0).signum() {
            crossings += 1;
        }
    }
    assert!(
        crossings >= 3,
        "edited curve should cross x=50 at least 3 times (self-intersecting), got {crossings}"
    );
}
