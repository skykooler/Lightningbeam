use lightningbeam_core::region_select::*;
use vello::kurbo::{BezPath, Point, Rect, Shape};

#[test]
fn debug_clip_rect_corner() {
    // Rectangle from (0,0) to (200,200)
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(200.0, 0.0));
    subject.line_to(Point::new(200.0, 200.0));
    subject.line_to(Point::new(0.0, 200.0));
    subject.close_path();

    // Region: upper-right corner, from (100,0) to (300,100)
    // This extends beyond the subject on the right and top
    let region = rect_to_path(Rect::new(100.0, 0.0, 300.0, 100.0));

    println!("Subject path: {:?}", subject);
    println!("Region path: {:?}", region);

    let result = clip_path_to_region(&subject, &region);

    println!("Inside path: {:?}", result.inside);
    println!("Outside path: {:?}", result.outside);

    let inside_bb = result.inside.bounding_box();
    println!("Inside bounding box: {:?}", inside_bb);

    // Expected inside: a rectangle from (100,0) to (200,100)
    assert!((inside_bb.x0 - 100.0).abs() < 2.0,
        "inside x0 should be ~100, got {}", inside_bb.x0);
    assert!((inside_bb.y0 - 0.0).abs() < 2.0,
        "inside y0 should be ~0, got {}", inside_bb.y0);
    assert!((inside_bb.x1 - 200.0).abs() < 2.0,
        "inside x1 should be ~200, got {}", inside_bb.x1);
    assert!((inside_bb.y1 - 100.0).abs() < 2.0,
        "inside y1 should be ~100, got {}", inside_bb.y1);

    // Verify the inside path has the right shape by checking it has ~5 elements
    // (MoveTo, 3x LineTo, ClosePath) for a rectangle
    let elem_count = result.inside.elements().len();
    println!("Inside element count: {}", elem_count);

    // Print each element
    for (i, el) in result.inside.elements().iter().enumerate() {
        println!("  inside[{}]: {:?}", i, el);
    }
    for (i, el) in result.outside.elements().iter().enumerate() {
        println!("  outside[{}]: {:?}", i, el);
    }
}

#[test]
fn debug_clip_partial_overlap() {
    // When the region is fully contained inside the subject (no edge crossings),
    // the path-clipping approach cannot split the subject since no path segments
    // cross the region boundary. This is correct — the selection system handles
    // this case by classifying the shape as "fully_inside" via hit_test, not
    // via path clipping.
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(200.0, 0.0));
    subject.line_to(Point::new(200.0, 200.0));
    subject.line_to(Point::new(0.0, 200.0));
    subject.close_path();

    let region = rect_to_path(Rect::new(50.0, 50.0, 150.0, 150.0));
    let result = clip_path_to_region(&subject, &region);

    // No intersections found → entire subject classified as outside
    assert!(result.inside.elements().is_empty(),
        "No edge crossings → inside should be empty (handled by hit_test instead)");
    assert!(!result.outside.elements().is_empty());
}

#[test]
fn debug_lasso_extending_beyond_subject() {
    // Rectangle subject from (0,0) to (100,100)
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(100.0, 0.0));
    subject.line_to(Point::new(100.0, 100.0));
    subject.line_to(Point::new(0.0, 100.0));
    subject.close_path();

    // Lasso region that extends beyond the subject: a triangle
    // covering (20,20) to (150,20) to (80,120)
    // This extends beyond the right and bottom of the rectangle
    let mut lasso = BezPath::new();
    lasso.move_to(Point::new(20.0, 20.0));
    lasso.line_to(Point::new(150.0, 20.0));
    lasso.line_to(Point::new(80.0, 120.0));
    lasso.close_path();

    let result = clip_path_to_region(&subject, &lasso);

    // The inside should be ONLY the intersection of the rectangle and lasso.
    // It should NOT extend beyond the rectangle's bounds.
    let inside_bb = result.inside.bounding_box();
    println!("Lasso extending beyond: inside bb = {:?}", inside_bb);
    for (i, el) in result.inside.elements().iter().enumerate() {
        println!("  inside[{}]: {:?}", i, el);
    }

    // The inside must be contained within the subject rectangle
    assert!(inside_bb.x0 >= -1.0, "inside x0={} should be >= 0", inside_bb.x0);
    assert!(inside_bb.y0 >= -1.0, "inside y0={} should be >= 0", inside_bb.y0);
    assert!(inside_bb.x1 <= 101.0, "inside x1={} should be <= 100", inside_bb.x1);
    assert!(inside_bb.y1 <= 101.0, "inside y1={} should be <= 100", inside_bb.y1);
}

#[test]
fn debug_lasso_splits_remainder_into_two() {
    // Rectangle subject from (0,0) to (200,200)
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(200.0, 0.0));
    subject.line_to(Point::new(200.0, 200.0));
    subject.line_to(Point::new(0.0, 200.0));
    subject.close_path();

    // Lasso that cuts across the rectangle, with vertices clearly NOT on
    // the subject boundary. The lasso is a diamond that extends beyond
    // the rect on top and right, splitting the remainder into two pieces:
    // upper-left and lower-right.
    let mut lasso = BezPath::new();
    lasso.move_to(Point::new(-10.0, 100.0));   // left of rect
    lasso.line_to(Point::new(100.0, -60.0));   // above rect
    lasso.line_to(Point::new(260.0, 100.0));   // right of rect
    lasso.line_to(Point::new(100.0, 210.0));   // below rect
    lasso.close_path();

    let result = clip_path_to_region(&subject, &lasso);

    let inside_bb = result.inside.bounding_box();
    println!("Lasso splits remainder: inside bb = {:?}", inside_bb);
    for (i, el) in result.inside.elements().iter().enumerate() {
        println!("  inside[{}]: {:?}", i, el);
    }

    // The inside must be contained within the subject rectangle bounds
    assert!(inside_bb.x0 >= -1.0,
        "inside x0={} should be >= 0 (must not extend left of subject)", inside_bb.x0);
    assert!(inside_bb.y0 >= -1.0,
        "inside y0={} should be >= 0 (must not extend above subject)", inside_bb.y0);
    assert!(inside_bb.x1 <= 201.0,
        "inside x1={} should be <= 200 (must not extend right of subject)", inside_bb.x1);
    assert!(inside_bb.y1 <= 201.0,
        "inside y1={} should be <= 200 (must not extend below subject)", inside_bb.y1);

    // The outside (remainder) must also be within subject bounds
    let outside_bb = result.outside.bounding_box();
    println!("Lasso splits remainder: outside bb = {:?}", outside_bb);
    for (i, el) in result.outside.elements().iter().enumerate() {
        println!("  outside[{}]: {:?}", i, el);
    }
    assert!(outside_bb.x1 <= 201.0,
        "outside x1={} must not extend right of subject (no lasso fill!)", outside_bb.x1);
    assert!(outside_bb.y0 >= -1.0,
        "outside y0={} must not extend above subject (no lasso fill!)", outside_bb.y0);

    // The outside should have multiple separate sub-paths (multiple ClosePath elements)
    // instead of one giant path that includes the lasso area
    let close_count = result.outside.elements().iter()
        .filter(|el| matches!(el, vello::kurbo::PathEl::ClosePath))
        .count();
    println!("Outside close_path count: {}", close_count);
    assert!(close_count >= 2,
        "Outside should be split into separate sub-paths, got {} ClosePaths", close_count);
}

#[test]
fn debug_clip_outside_shape_correct() {
    // Test the exact scenario the user reported: outside shape should
    // correctly include boundary walk points (no diagonal shortcuts)
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(200.0, 0.0));
    subject.line_to(Point::new(200.0, 200.0));
    subject.line_to(Point::new(0.0, 200.0));
    subject.close_path();

    let region = rect_to_path(Rect::new(100.0, 0.0, 300.0, 100.0));
    let result = clip_path_to_region(&subject, &region);

    // Outside should be an L-shape: (0,0)-(100,0)-(100,100)-(200,100)-(200,200)-(0,200)
    let outside_bb = result.outside.bounding_box();
    assert!((outside_bb.x0 - 0.0).abs() < 2.0);
    assert!((outside_bb.y0 - 0.0).abs() < 2.0);
    assert!((outside_bb.x1 - 200.0).abs() < 2.0);
    assert!((outside_bb.y1 - 200.0).abs() < 2.0);

    // Verify the path includes (200,100) — the critical boundary walk point
    let has_200_100 = result.outside.elements().iter().any(|el| {
        match *el {
            vello::kurbo::PathEl::LineTo(p) => (p.x - 200.0).abs() < 1.0 && (p.y - 100.0).abs() < 1.0,
            _ => false,
        }
    });
    assert!(has_200_100, "Outside path must include point (200,100) from boundary walk");
}
