//! Gap tolerance fill tracing: invisible edges bridging small gaps,
//! mid-curve gap closing, and area-limiting behavior.

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

// ── Endpoint gap closing ─────────────────────────────────────────────────

#[test]
fn gap_close_bridges_small_endpoint_gap() {
    // Three sides of a rectangle with a small gap at the fourth corner.
    // Without gap tolerance: no enclosed region.
    // With gap tolerance: the gap is bridged by an invisible edge.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    let tl = Point::new(0.0, 100.0);
    let tr = Point::new(100.0, 100.0);
    let br = Point::new(100.0, 0.0);
    let bl = Point::new(0.0, 0.0);
    let bl_gap = Point::new(3.0, 0.0); // 3px gap from true bottom-left

    // Three complete sides + one side that stops 3px short
    g.insert_stroke(&[line(tl, tr)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(tr, br)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(br, bl_gap)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(bl, tl)], style, color, 0.5);

    // Without gap tolerance: should fail to find an enclosed region
    let no_gap = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        0.0,
    );
    assert!(no_gap.is_none(), "should not find enclosed region with zero gap tolerance");

    // With gap tolerance of 5px: should bridge the 3px gap
    let with_gap = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        5.0,
    );
    assert!(with_gap.is_some(), "should bridge the 3px gap with 5px tolerance");

    // The bridge should be a real invisible edge in the graph
    let fid = with_gap.unwrap();
    let fill = g.fill(fid);
    let has_invisible_boundary_edge = fill.boundary.iter().any(|(eid, _)| {
        !g.edge_is_visible(*eid)
    });
    assert!(has_invisible_boundary_edge, "gap-close should create an invisible edge");
}

#[test]
fn gap_close_does_not_bridge_large_gap() {
    // Same as above but with a 20px gap — should not bridge with 5px tolerance.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    let tl = Point::new(0.0, 100.0);
    let tr = Point::new(100.0, 100.0);
    let br = Point::new(100.0, 0.0);
    let bl = Point::new(0.0, 0.0);
    let bl_gap = Point::new(20.0, 0.0); // 20px gap

    g.insert_stroke(&[line(tl, tr)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(tr, br)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(br, bl_gap)], style.clone(), color, 0.5);
    g.insert_stroke(&[line(bl, tl)], style, color, 0.5);

    let result = g.paint_bucket(
        Point::new(50.0, 50.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        5.0,
    );
    assert!(result.is_none(), "should not bridge a 20px gap with 5px tolerance");
}

// ── Mid-curve gap closing: )( pattern ────────────────────────────────────

#[test]
fn gap_close_mid_curve_parentheses() {
    // Two opposing arcs forming a )( shape, with caps at top and bottom
    // so the ends are closed. The closest approach is at the midpoints
    // of the arcs (~5px gap). Gap-close should bridge there, and filling
    // on one side should only fill that half — not the full eye shape.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Left arc: ) shape — endpoints at (40, 0) and (40, 100), bowing right to x≈55
    g.insert_stroke(
        &[CubicBez::new(
            Point::new(40.0, 0.0),
            Point::new(60.0, 0.0),
            Point::new(60.0, 100.0),
            Point::new(40.0, 100.0),
        )],
        style.clone(), color, 0.5,
    );

    // Right arc: ( shape — endpoints at (70, 0) and (70, 100), bowing left to x≈59
    // (must stay right of left arc's max x≈55 to avoid crossing)
    g.insert_stroke(
        &[CubicBez::new(
            Point::new(70.0, 0.0),
            Point::new(55.0, 0.0),
            Point::new(55.0, 100.0),
            Point::new(70.0, 100.0),
        )],
        style.clone(), color, 0.5,
    );

    // Cap the top: (40, 0) → (70, 0)
    g.insert_stroke(&[line(Point::new(40.0, 0.0), Point::new(70.0, 0.0))], style.clone(), color, 0.5);

    // Cap the bottom: (40, 100) → (70, 100)
    g.insert_stroke(&[line(Point::new(40.0, 100.0), Point::new(70.0, 100.0))], style, color, 0.5);

    // The full shape is an eye/lens. The mid-curve gap (~3.75px at y≈50)
    // divides it into left and right halves.
    // With gap tolerance of 10px, clicking in the LEFT half should only
    // fill the left half — the gap-close bridge at the midpoints acts
    // as a dividing edge.
    // The bridge divides the eye horizontally at y≈50.
    // The eye interior is between the two arcs (at y=25, roughly x=53..60).
    // Click between the arcs in the upper half.
    let fid = g.paint_bucket(
        Point::new(57.0, 25.0),  // between arcs, upper half
        ShapeColor::rgb(0, 0, 255),
        FillRule::NonZero,
        10.0,
    );
    assert!(fid.is_some(), "should bridge mid-curve gap and fill one half");

    let fid = fid.unwrap();
    let path = g.fill_to_bezpath(fid);
    let bbox = kurbo::Shape::bounding_box(&path);

    // The fill should cover only one half of the eye shape.
    // The full eye spans y=0..100, so one half should be roughly y=0..50.
    assert!(
        bbox.height() < 60.0,
        "fill should only cover one half of the eye, got height={:.1}",
        bbox.height()
    );

    // The other half should NOT be filled
    assert_eq!(
        kurbo::Shape::winding(&path, Point::new(57.0, 75.0)),
        0,
        "other half of the eye should not be filled"
    );
}

// ── Acute corner: gap-close should NOT cut across ────────────────────────

#[test]
fn gap_close_does_not_shortcut_acute_corner() {
    // Two edges meeting at a sharp acute angle at vertex (50, 0).
    // Near the vertex, the edges are close together, but they are connected —
    // gap-close should NOT bridge between them.
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Two lines meeting at a sharp angle at (50, 0)
    // Left arm: (0, 50) → (50, 0)
    // Right arm: (50, 0) → (100, 50)
    g.insert_stroke(
        &[
            line(Point::new(0.0, 50.0), Point::new(50.0, 0.0)),
            line(Point::new(50.0, 0.0), Point::new(100.0, 50.0)),
        ],
        style.clone(), color, 0.5,
    );

    // Close off the bottom to form a triangle
    g.insert_stroke(
        &[line(Point::new(0.0, 50.0), Point::new(100.0, 50.0))],
        style, color, 0.5,
    );

    // Fill the triangle with generous gap tolerance — the fill should go all
    // the way into the acute corner at (50, 0), not shortcut across it.
    let fid = g.paint_bucket(
        Point::new(50.0, 30.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        10.0,
    ).expect("should fill the triangle");

    // The fill should reach the apex at (50, 0)
    let path = g.fill_to_bezpath(fid);
    let bbox = kurbo::Shape::bounding_box(&path);
    assert!(
        bbox.min_y() < 2.0,
        "fill should reach the apex near y=0, got min_y={:.1}",
        bbox.min_y()
    );
}

// ── Gap tolerance as area limiter ────────────────────────────────────────

#[test]
fn gap_close_prefers_smallest_enclosing_region() {
    // A large rectangle with a small rectangle inside it.
    // The small rectangle has a gap. With gap tolerance, the user clicks
    // inside the small rectangle — should fill the small rectangle,
    // NOT the large one (even though the large one is also reachable
    // through the gap).
    let mut g = VectorGraph::new();
    let (style, color) = black_stroke();

    // Large rectangle: (0, 0) → (200, 200)
    g.insert_stroke(
        &[
            line(Point::new(0.0, 0.0), Point::new(200.0, 0.0)),
            line(Point::new(200.0, 0.0), Point::new(200.0, 200.0)),
            line(Point::new(200.0, 200.0), Point::new(0.0, 200.0)),
            line(Point::new(0.0, 200.0), Point::new(0.0, 0.0)),
        ],
        style.clone(), color, 0.5,
    );

    // Small rectangle inside: (80, 80) → (120, 120), with a 3px gap
    g.insert_stroke(
        &[
            line(Point::new(80.0, 80.0), Point::new(120.0, 80.0)),
            line(Point::new(120.0, 80.0), Point::new(120.0, 120.0)),
            line(Point::new(120.0, 120.0), Point::new(83.0, 120.0)), // stops 3px short
        ],
        style.clone(), color, 0.5,
    );
    g.insert_stroke(
        &[line(Point::new(80.0, 120.0), Point::new(80.0, 80.0))],
        style, color, 0.5,
    );

    // Click inside the small rectangle with gap tolerance
    let fid = g.paint_bucket(
        Point::new(100.0, 100.0),
        ShapeColor::rgb(255, 0, 0),
        FillRule::NonZero,
        5.0,
    ).expect("should fill with gap tolerance");

    // The fill should be the small rectangle, not the large one
    let path = g.fill_to_bezpath(fid);
    let bbox = kurbo::Shape::bounding_box(&path);
    assert!(
        bbox.width() < 60.0 && bbox.height() < 60.0,
        "should fill the small rectangle (~40x40), not the large one (~200x200), got {:.0}x{:.0}",
        bbox.width(),
        bbox.height()
    );
}
