//! Paint bucket, fill splitting, fill persistence, and fill merging.

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

/// Helper: insert a rectangle as 4 stroke segments, returning the graph.
fn make_rect(x0: f64, y0: f64, x1: f64, y1: f64) -> VectorGraph {
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();
    let tl = Point::new(x0, y0);
    let tr = Point::new(x1, y0);
    let br = Point::new(x1, y1);
    let bl = Point::new(x0, y1);
    g.insert_stroke(
        &[line(tl, tr), line(tr, br), line(br, bl), line(bl, tl)],
        style,
        color,
        0.5,
    );
    g
}

// ── Paint bucket traces boundary and creates fill ────────────────────────

#[test]
fn paint_bucket_fills_rectangle() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    // Click inside the rectangle
    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    );
    assert!(fid.is_some(), "paint bucket should find and fill the rectangle");

    let fid = fid.unwrap();
    let fill = g.fill(fid);
    assert_eq!(fill.boundary.len(), 4, "rectangle boundary should have 4 edges");
    assert_eq!(fill.color, ShapeColor::rgb(255, 0, 0));
}

#[test]
fn paint_bucket_outside_rectangle_returns_none() {
    let mut g = make_rect(100.0, 100.0, 200.0, 200.0);

    // Click outside — no enclosed region
    let fid = g.paint_bucket(
        Point::new(0.0, 0.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    );
    assert!(fid.is_none(), "paint bucket outside all curves should return None");
}

#[test]
fn paint_bucket_hash_fills_center_only() {
    // # structure: the center square should be fillable without including the stubs
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    g.insert_stroke(&[line(Point::new(0.0, 30.0), Point::new(100.0, 30.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(0.0, 70.0), Point::new(100.0, 70.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(30.0, 0.0), Point::new(30.0, 100.0))], style.clone(), color, 0.5);
    g.insert_stroke(&[line(Point::new(70.0, 0.0), Point::new(70.0, 100.0))], style, color, 0.5);

    // Click in the center square (50, 50)
    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(0, 0, 255),
        FillRule::NonZero,
        0.0,
    );
    assert!(fid.is_some(), "should fill the center square of the # pattern");

    let fid = fid.unwrap();
    let fill = g.fill(fid);
    assert_eq!(fill.boundary.len(), 4, "center square should have exactly 4 boundary edges");

    // Verify the fill region is small (the center square, not the whole #)
    let path = g.fill_to_bezpath(fid);
    let bbox = kurbo::Shape::bounding_box(&path);
    assert!(bbox.width() < 50.0, "fill should be the center square, not the whole structure");
    assert!(bbox.height() < 50.0);
}

// ── Fill splitting ───────────────────────────────────────────────────────

#[test]
fn draw_line_across_fill_splits_it() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    // Fill the rectangle
    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Draw a horizontal line through the middle, splitting the rectangle
    let (style, color) = black_stroke();
    let new_edges = g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(100.0, 50.0))],
        style,
        color,
        0.5,
    );

    // The new line's endpoints should be at (0, 50) and (100, 50),
    // where it intersects the left and right edges of the rectangle.
    assert!(!new_edges.is_empty(), "insert_stroke should create at least one edge");
    let first_edge = g.edge(*new_edges.first().unwrap());
    let last_edge = g.edge(*new_edges.last().unwrap());
    let start_pos = g.vertex(first_edge.vertices[0]).position;
    let end_pos = g.vertex(last_edge.vertices[1]).position;
    assert!(
        (start_pos.x - 0.0).abs() < 1.0 && (start_pos.y - 50.0).abs() < 1.0,
        "new line should start at (0, 50), got ({:.1}, {:.1})",
        start_pos.x, start_pos.y,
    );
    assert!(
        (end_pos.x - 100.0).abs() < 1.0 && (end_pos.y - 50.0).abs() < 1.0,
        "new line should end at (100, 50), got ({:.1}, {:.1})",
        end_pos.x, end_pos.y,
    );

    // The original fill should have been split into two fills
    let live_fills: Vec<_> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted)
        .collect();
    assert_eq!(live_fills.len(), 2, "drawing a line across a fill should split it into 2");

    // Both fills should inherit the original color
    for (_, fill) in &live_fills {
        assert_eq!(fill.color, ShapeColor::rgb(255, 0, 0));
    }
}

#[test]
fn draw_line_not_through_fill_does_not_split() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    let _fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Draw a line outside the rectangle — should not affect the fill
    let (style, color) = black_stroke();
    g.insert_stroke(
        &[line(Point::new(200.0, 0.0), Point::new(200.0, 100.0))],
        style,
        color,
        0.5,
    );

    let live_fills = g.fills.iter().filter(|f| !f.deleted).count();
    assert_eq!(live_fills, 1, "line outside fill should not split it");
}

#[test]
fn draw_line_partially_across_fill_does_not_split() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Draw a line that enters the fill but doesn't reach the other side:
    // (0, 50) → (50, 50) — starts on the left edge, ends in the middle
    let (style, color) = black_stroke();
    g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(50.0, 50.0))],
        style,
        color,
        0.5,
    );

    // The fill should NOT be split — the line only touches one boundary edge,
    // not two. It's a spur (dead end) inside the fill.
    let live_fills = g.fills.iter().filter(|f| !f.deleted).count();
    assert_eq!(live_fills, 1, "line partially across fill should not split it");

    // The fill should still reference a valid closed boundary
    let fill = g.fill(fid);
    assert!(!fill.deleted);
}

// ── Shared edges and concentric fills ────────────────────────────────────

#[test]
fn inner_square_reusing_edge_not_filled() {
    // Outer square (0,0)-(100,100), a spur from (0,50)→(50,50),
    // then an inner square (50,50)-(75,75). The spur connects the inner
    // square to the outer boundary. Fill the outer square — the inner
    // square should NOT be filled (it's a separate enclosed region).
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);
    let (style, color) = black_stroke();

    // Spur: (0,50) → (50,50)
    g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(50.0, 50.0))],
        style.clone(), color, 0.5,
    );

    // Inner square: (50,50) → (75,50) → (75,75) → (50,75) → (50,50)
    g.insert_stroke(
        &[
            line(Point::new(50.0, 50.0), Point::new(75.0, 50.0)),
            line(Point::new(75.0, 50.0), Point::new(75.0, 75.0)),
            line(Point::new(75.0, 75.0), Point::new(50.0, 75.0)),
            line(Point::new(50.0, 75.0), Point::new(50.0, 50.0)),
        ],
        style, color, 0.5,
    );

    // Fill outside the inner square but inside the outer square
    let fid = g.paint_bucket(
        Point::new(25.0, 25.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill the outer region");

    // The fill should NOT cover the inner square's interior
    let path = g.fill_to_bezpath(fid);
    // Point inside the inner square should be outside the fill path
    assert_eq!(
        kurbo::Shape::winding(&path, Point::new(62.0, 62.0)),
        0,
        "inner square interior should not be included in the outer fill"
    );
    // Point in the outer region should be inside the fill path
    assert_ne!(
        kurbo::Shape::winding(&path, Point::new(25.0, 25.0)),
        0,
        "outer region should be inside the fill"
    );
}

#[test]
fn concentric_squares_fill_has_hole() {
    // Outer square (0,0)-(100,100), inner square (25,25)-(75,75).
    // No connecting edge — the two squares share no vertices.
    // Filling the outer region should produce a fill with a hole
    // (the inner square subtracts from the outer).
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);
    let (style, color) = black_stroke();

    // Inner square, entirely inside the outer one
    g.insert_stroke(
        &[
            line(Point::new(25.0, 25.0), Point::new(75.0, 25.0)),
            line(Point::new(75.0, 25.0), Point::new(75.0, 75.0)),
            line(Point::new(75.0, 75.0), Point::new(25.0, 75.0)),
            line(Point::new(25.0, 75.0), Point::new(25.0, 25.0)),
        ],
        style, color, 0.5,
    );

    // Fill between the two squares (click in the gap between them)
    let fid = g.paint_bucket(
        Point::new(10.0, 10.0),
        ShapeColor::rgb(0, 255, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill the annular region");

    let path = g.fill_to_bezpath(fid);

    // Point in the gap (between squares) should be inside the fill
    assert_ne!(
        kurbo::Shape::winding(&path, Point::new(10.0, 10.0)),
        0,
        "gap between squares should be filled"
    );

    // Point inside the inner square should NOT be filled
    assert_eq!(
        kurbo::Shape::winding(&path, Point::new(50.0, 50.0)),
        0,
        "inner square interior should be a hole in the fill"
    );
}

// ── Fill persistence through edge deletion ───────────────────────────────

#[test]
fn fill_persists_when_edge_deleted() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Delete one edge of the rectangle
    let boundary_edge = g.fill(fid).boundary[0].0;
    g.delete_edge_by_user(boundary_edge);

    // Fill should still exist and still have the same boundary
    assert!(!g.fill(fid).deleted, "fill should persist when its boundary edge is deleted");
    assert_eq!(g.fill(fid).boundary.len(), 4, "fill boundary should be unchanged");

    // The deleted edge should now be invisible but still exist
    assert!(!g.edge(boundary_edge).deleted);
    assert!(!g.edge_is_visible(boundary_edge));
}

#[test]
fn deleting_fill_then_gc_removes_invisible_edges() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    let fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Make all edges invisible (user deleted the strokes)
    let boundary_edges: Vec<EdgeId> = g.fill(fid).boundary.iter().map(|(e, _)| *e).collect();
    for &eid in &boundary_edges {
        g.make_edge_invisible(eid);
    }

    // Edges should still exist because fill references them
    for &eid in &boundary_edges {
        assert!(!g.edge(eid).deleted);
    }

    // Now delete the fill
    g.free_fill(fid);

    // GC should remove the invisible, now-unreferenced edges
    g.gc_invisible_edges();
    for &eid in &boundary_edges {
        assert!(g.edge(eid).deleted, "invisible edge should be GC'd after fill deleted");
    }
}

// ── Fill merging ─────────────────────────────────────────────────────────

#[test]
fn deleting_dividing_edge_merges_fills() {
    let mut g = make_rect(0.0, 0.0, 100.0, 100.0);

    // Fill the rectangle
    let _fid = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    ).expect("should fill");

    // Draw a horizontal line through the middle to split the fill
    let (style, color) = black_stroke();
    let new_edges = g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(100.0, 50.0))],
        style,
        color,
        0.5,
    );

    let live_fills_before = g.fills.iter().filter(|f| !f.deleted).count();
    assert_eq!(live_fills_before, 2);

    // Delete the dividing line — the two fills should merge back into one
    // (The dividing edge endpoints are on the fill boundaries, making this detectable)
    for eid in new_edges {
        g.delete_edge_by_user(eid);
    }

    let live_fills_after = g.fills.iter().filter(|f| !f.deleted).count();
    assert_eq!(live_fills_after, 1, "deleting the dividing edge should merge the two fills");
}
