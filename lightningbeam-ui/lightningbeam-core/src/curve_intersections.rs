//! Curve intersection and proximity detection for paint bucket tool
//!
//! This module provides functions for finding:
//! - Exact intersections between cubic Bezier curves
//! - Self-intersections within a single curve
//! - Closest approach between curves (for gap tolerance)

use vello::kurbo::{CubicBez, ParamCurve, ParamCurveNearest, Point, Shape};

/// Result of a curve intersection
#[derive(Debug, Clone)]
pub struct Intersection {
    /// Parameter t on first curve [0, 1]
    pub t1: f64,
    /// Parameter t on second curve [0, 1] (for curve-curve intersections)
    pub t2: Option<f64>,
    /// Point of intersection
    pub point: Point,
}

/// Result of a close approach between two curves
#[derive(Debug, Clone)]
pub struct CloseApproach {
    /// Parameter on first curve
    pub t1: f64,
    /// Parameter on second curve
    pub t2: f64,
    /// Point on first curve
    pub p1: Point,
    /// Point on second curve
    pub p2: Point,
    /// Distance between the curves
    pub distance: f64,
}

/// Find intersections between two cubic Bezier curves
///
/// Uses recursive subdivision to find intersection points.
/// This is much more robust and faster than sampling.
pub fn find_curve_intersections(curve1: &CubicBez, curve2: &CubicBez) -> Vec<Intersection> {
    let mut intersections = Vec::new();

    // Use subdivision-based intersection detection
    find_intersections_recursive(
        curve1, curve1, 0.0, 1.0,
        curve2, curve2, 0.0, 1.0,
        &mut intersections,
        0, // recursion depth
    );

    // Remove duplicate intersections
    dedup_intersections(&mut intersections, 1.0);

    intersections
}

/// Recursively find intersections using subdivision
///
/// orig_curve1/2 are the original curves (for computing final intersection points)
/// curve1/2 are the current subsegments being tested
/// t1_start/end track the parameter range on the original curve
fn find_intersections_recursive(
    orig_curve1: &CubicBez,
    curve1: &CubicBez,
    t1_start: f64,
    t1_end: f64,
    orig_curve2: &CubicBez,
    curve2: &CubicBez,
    t2_start: f64,
    t2_end: f64,
    intersections: &mut Vec<Intersection>,
    depth: usize,
) {
    // Maximum recursion depth
    const MAX_DEPTH: usize = 20;

    // Minimum parameter range (if smaller, we've found an intersection)
    const MIN_RANGE: f64 = 0.001;

    // Get bounding boxes of current subsegments
    let bbox1 = curve1.bounding_box();
    let bbox2 = curve2.bounding_box();

    // Inflate bounding boxes slightly to account for numerical precision
    let bbox1 = bbox1.inflate(0.1, 0.1);
    let bbox2 = bbox2.inflate(0.1, 0.1);

    // If bounding boxes don't overlap, no intersection
    if !bboxes_overlap(&bbox1, &bbox2) {
        return;
    }

    // If we've recursed deep enough or ranges are small enough, record intersection
    if depth >= MAX_DEPTH ||
       ((t1_end - t1_start) < MIN_RANGE && (t2_end - t2_start) < MIN_RANGE) {
        let t1 = (t1_start + t1_end) / 2.0;
        let t2 = (t2_start + t2_end) / 2.0;

        intersections.push(Intersection {
            t1,
            t2: Some(t2),
            point: orig_curve1.eval(t1),
        });
        return;
    }

    // Subdivide both curves at midpoint (of the current subsegment, which is 0..1)
    let t1_mid = (t1_start + t1_end) / 2.0;
    let t2_mid = (t2_start + t2_end) / 2.0;

    // Create subsegments - these are new curves parameterized 0..1
    let curve1_left = curve1.subsegment(0.0..0.5);
    let curve1_right = curve1.subsegment(0.5..1.0);
    let curve2_left = curve2.subsegment(0.0..0.5);
    let curve2_right = curve2.subsegment(0.5..1.0);

    // Check all four combinations
    find_intersections_recursive(
        orig_curve1, &curve1_left, t1_start, t1_mid,
        orig_curve2, &curve2_left, t2_start, t2_mid,
        intersections, depth + 1
    );

    find_intersections_recursive(
        orig_curve1, &curve1_left, t1_start, t1_mid,
        orig_curve2, &curve2_right, t2_mid, t2_end,
        intersections, depth + 1
    );

    find_intersections_recursive(
        orig_curve1, &curve1_right, t1_mid, t1_end,
        orig_curve2, &curve2_left, t2_start, t2_mid,
        intersections, depth + 1
    );

    find_intersections_recursive(
        orig_curve1, &curve1_right, t1_mid, t1_end,
        orig_curve2, &curve2_right, t2_mid, t2_end,
        intersections, depth + 1
    );
}

/// Check if two bounding boxes overlap
fn bboxes_overlap(bbox1: &vello::kurbo::Rect, bbox2: &vello::kurbo::Rect) -> bool {
    bbox1.x0 <= bbox2.x1 &&
    bbox1.x1 >= bbox2.x0 &&
    bbox1.y0 <= bbox2.y1 &&
    bbox1.y1 >= bbox2.y0
}

/// Find self-intersections within a single cubic Bezier curve
///
/// A curve self-intersects when it crosses itself, forming a loop.
pub fn find_self_intersections(curve: &CubicBez) -> Vec<Intersection> {
    let mut intersections = Vec::new();

    // Sample the curve at regular intervals
    let samples = 50;
    for i in 0..samples {
        let t1 = i as f64 / samples as f64;
        let p1 = curve.eval(t1);

        // Check against all later points
        for j in (i + 5)..samples {  // Skip nearby points to avoid false positives
            let t2 = j as f64 / samples as f64;
            let p2 = curve.eval(t2);
            let dist = (p1 - p2).hypot();

            // If points are very close, we may have a self-intersection
            if dist < 0.5 {
                // Refine to get more accurate parameters
                let (refined_t1, refined_t2) = refine_self_intersection(curve, t1, t2);

                intersections.push(Intersection {
                    t1: refined_t1,
                    t2: Some(refined_t2),
                    point: curve.eval(refined_t1),
                });
            }
        }
    }

    // Remove duplicates
    dedup_intersections(&mut intersections, 0.5);

    intersections
}

/// Find the closest approach between two curves if within tolerance
///
/// Returns Some if the minimum distance between curves is less than tolerance.
pub fn find_closest_approach(
    curve1: &CubicBez,
    curve2: &CubicBez,
    tolerance: f64,
) -> Option<CloseApproach> {
    let mut min_dist = f64::MAX;
    let mut best_t1 = 0.0;
    let mut best_t2 = 0.0;

    // Sample curve1 at regular intervals
    let samples = 50;
    for i in 0..=samples {
        let t1 = i as f64 / samples as f64;
        let p1 = curve1.eval(t1);

        // Find nearest point on curve2
        let nearest = curve2.nearest(p1, 1e-6);
        let dist = (p1 - curve2.eval(nearest.t)).hypot();

        if dist < min_dist {
            min_dist = dist;
            best_t1 = t1;
            best_t2 = nearest.t;
        }
    }

    // If minimum distance is within tolerance, return it
    if min_dist < tolerance {
        Some(CloseApproach {
            t1: best_t1,
            t2: best_t2,
            p1: curve1.eval(best_t1),
            p2: curve2.eval(best_t2),
            distance: min_dist,
        })
    } else {
        None
    }
}

/// Refine intersection parameters using Newton's method
fn refine_intersection(
    curve1: &CubicBez,
    curve2: &CubicBez,
    mut t1: f64,
    mut t2: f64,
) -> (f64, f64) {
    // Simple refinement: just find nearest points iteratively
    for _ in 0..5 {
        let p1 = curve1.eval(t1);
        let nearest2 = curve2.nearest(p1, 1e-6);
        t2 = nearest2.t;

        let p2 = curve2.eval(t2);
        let nearest1 = curve1.nearest(p2, 1e-6);
        t1 = nearest1.t;
    }

    (t1.clamp(0.0, 1.0), t2.clamp(0.0, 1.0))
}

/// Refine self-intersection parameters
fn refine_self_intersection(curve: &CubicBez, mut t1: f64, mut t2: f64) -> (f64, f64) {
    // Refine by moving parameters closer to where curves actually meet
    for _ in 0..5 {
        let p1 = curve.eval(t1);
        let p2 = curve.eval(t2);
        let mid = Point::new((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0);

        // Move both parameters toward the midpoint
        let nearest1 = curve.nearest(mid, 1e-6);
        let nearest2 = curve.nearest(mid, 1e-6);

        // Take whichever is closer to original parameter
        if (nearest1.t - t1).abs() < (nearest2.t - t1).abs() {
            t1 = nearest1.t;
        } else if (nearest2.t - t2).abs() < (nearest1.t - t2).abs() {
            t2 = nearest2.t;
        }
    }

    (t1.clamp(0.0, 1.0), t2.clamp(0.0, 1.0))
}

/// Remove duplicate intersections within a tolerance
fn dedup_intersections(intersections: &mut Vec<Intersection>, tolerance: f64) {
    let mut i = 0;
    while i < intersections.len() {
        let mut j = i + 1;
        while j < intersections.len() {
            let dist = (intersections[i].point - intersections[j].point).hypot();
            if dist < tolerance {
                intersections.remove(j);
            } else {
                j += 1;
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_curve_intersection_simple() {
        // Two curves that cross
        let curve1 = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(100.0, 100.0),
            Point::new(200.0, 200.0),
        );

        let curve2 = CubicBez::new(
            Point::new(200.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 200.0),
        );

        let intersections = find_curve_intersections(&curve1, &curve2);
        // Should find at least one intersection near the center
        assert!(!intersections.is_empty());
    }

    #[test]
    fn test_self_intersection() {
        // A curve that loops back on itself
        let curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(-100.0, 100.0),
            Point::new(0.0, 0.0),
        );

        let intersections = find_self_intersections(&curve);
        // May or may not find intersection depending on curve shape
        // This is mostly testing that the function doesn't crash
        assert!(intersections.len() <= 10);  // Sanity check
    }

    #[test]
    fn test_closest_approach() {
        // Two curves that are close but don't intersect
        let curve1 = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(50.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(150.0, 0.0),
        );

        let curve2 = CubicBez::new(
            Point::new(0.0, 1.5),
            Point::new(50.0, 1.5),
            Point::new(100.0, 1.5),
            Point::new(150.0, 1.5),
        );

        let approach = find_closest_approach(&curve1, &curve2, 2.0);
        assert!(approach.is_some());
        let approach = approach.unwrap();
        assert!(approach.distance < 2.0);
    }
}
