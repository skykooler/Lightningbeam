use lightningbeam_core::region_select::*;
use vello::kurbo::{BezPath, Point, Rect, Shape};

#[test]
fn test_rect_to_path() {
    let rect = Rect::new(10.0, 20.0, 100.0, 200.0);
    let path = rect_to_path(rect);
    assert!(path.elements().len() >= 5);
}

#[test]
fn test_lasso_to_path() {
    let points = vec![
        Point::new(0.0, 0.0),
        Point::new(100.0, 0.0),
        Point::new(100.0, 100.0),
        Point::new(0.0, 100.0),
    ];
    let path = lasso_to_path(&points);
    assert!(path.elements().len() >= 5);
}

#[test]
fn test_clip_rect_corner() {
    // Rectangle from (0,0) to (100,100)
    let mut subject = BezPath::new();
    subject.move_to(Point::new(0.0, 0.0));
    subject.line_to(Point::new(100.0, 0.0));
    subject.line_to(Point::new(100.0, 100.0));
    subject.line_to(Point::new(0.0, 100.0));
    subject.close_path();

    // Clip to upper-right corner: region covers (50,0) to (150,50)
    let region = rect_to_path(Rect::new(50.0, 0.0, 150.0, 50.0));
    let result = clip_path_to_region(&subject, &region);

    // Inside should have elements (the upper-right portion)
    assert!(
        !result.inside.elements().is_empty(),
        "inside path should not be empty"
    );
    // Outside should have elements (the rest of the rectangle)
    assert!(
        !result.outside.elements().is_empty(),
        "outside path should not be empty"
    );

    // The inside portion should be a roughly rectangular region
    // Its bounding box should be approximately (50,0)-(100,50)
    let inside_bb = result.inside.bounding_box();
    assert!(
        (inside_bb.x0 - 50.0).abs() < 2.0,
        "inside x0 should be ~50, got {}",
        inside_bb.x0
    );
    assert!(
        (inside_bb.y0 - 0.0).abs() < 2.0,
        "inside y0 should be ~0, got {}",
        inside_bb.y0
    );
    assert!(
        (inside_bb.x1 - 100.0).abs() < 2.0,
        "inside x1 should be ~100, got {}",
        inside_bb.x1
    );
    assert!(
        (inside_bb.y1 - 50.0).abs() < 2.0,
        "inside y1 should be ~50, got {}",
        inside_bb.y1
    );
}

#[test]
fn test_clip_fully_inside() {
    let mut path = BezPath::new();
    path.move_to(Point::new(20.0, 20.0));
    path.line_to(Point::new(80.0, 20.0));
    path.line_to(Point::new(80.0, 80.0));
    path.line_to(Point::new(20.0, 80.0));
    path.close_path();

    let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
    let result = clip_path_to_region(&path, &region);

    assert!(!result.inside.elements().is_empty());
    assert!(result.outside.elements().is_empty());
}

#[test]
fn test_clip_fully_outside() {
    let mut path = BezPath::new();
    path.move_to(Point::new(200.0, 200.0));
    path.line_to(Point::new(300.0, 200.0));
    path.line_to(Point::new(300.0, 300.0));
    path.close_path();

    let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
    let result = clip_path_to_region(&path, &region);

    assert!(result.inside.elements().is_empty());
    assert!(!result.outside.elements().is_empty());
}

#[test]
fn test_path_intersects_region() {
    let mut path = BezPath::new();
    path.move_to(Point::new(-50.0, 50.0));
    path.line_to(Point::new(150.0, 50.0));

    let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(path_intersects_region(&path, &region));
}

#[test]
fn test_path_fully_inside() {
    let mut path = BezPath::new();
    path.move_to(Point::new(20.0, 20.0));
    path.line_to(Point::new(80.0, 20.0));
    path.line_to(Point::new(80.0, 80.0));
    path.close_path();

    let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(path_fully_inside_region(&path, &region));
    assert!(!path_intersects_region(&path, &region));
}

#[test]
fn test_clip_horizontal_line_crossing() {
    // A horizontal line crossing through a region
    let mut subject = BezPath::new();
    subject.move_to(Point::new(-50.0, 50.0));
    subject.line_to(Point::new(150.0, 50.0));

    let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
    let result = clip_path_to_region(&subject, &region);

    // Inside should be the segment from x=0 to x=100 at y=50
    let inside_bb = result.inside.bounding_box();
    assert!(
        (inside_bb.x0 - 0.0).abs() < 2.0,
        "inside x0 should be ~0, got {}",
        inside_bb.x0
    );
    assert!(
        (inside_bb.x1 - 100.0).abs() < 2.0,
        "inside x1 should be ~100, got {}",
        inside_bb.x1
    );
}
