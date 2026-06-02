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

    // Pixel-space convergence threshold: stop subdividing when both
    // subsegments span less than this many pixels.
    const PIXEL_TOL: f64 = 0.25;

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

    // Evaluate subsegment endpoints for convergence check and line-line solve
    let a0 = orig_curve1.eval(t1_start);
    let a1 = orig_curve1.eval(t1_end);
    let b0 = orig_curve2.eval(t2_start);
    let b1 = orig_curve2.eval(t2_end);

    // Check convergence in pixel space: both subsegment spans must be
    // below the tolerance. This ensures the linear approximation error
    // is always well within the vertex snap threshold regardless of
    // curve length.
    let a_span = (a1 - a0).hypot();
    let b_span = (b1 - b0).hypot();

    if depth >= MAX_DEPTH || (a_span < PIXEL_TOL && b_span < PIXEL_TOL) {

        let (t1, t2, point) = if let Some((s, u)) = line_line_intersect(a0, a1, b0, b1) {
            let s = s.clamp(0.0, 1.0);
            let u = u.clamp(0.0, 1.0);
            let mut t1 = t1_start + s * (t1_end - t1_start);
            let mut t2 = t2_start + u * (t2_end - t2_start);

            // Newton refinement: converge t1, t2 so that
            // curve1.eval(t1) == curve2.eval(t2) to sub-pixel accuracy.
            // We solve F(t1,t2) = curve1(t1) - curve2(t2) = 0 via the
            // Jacobian [d1, -d2] where d1/d2 are the curve tangents.
            let t1_orig = t1;
            let t2_orig = t2;
            for _ in 0..8 {
                let p1 = orig_curve1.eval(t1);
                let p2 = orig_curve2.eval(t2);
                let err = Point::new(p1.x - p2.x, p1.y - p2.y);
                if err.x * err.x + err.y * err.y < 1e-6 {
                    break;
                }
                // Tangent vectors (derivative of cubic bezier)
                let d1 = cubic_deriv(orig_curve1, t1);
                let d2 = cubic_deriv(orig_curve2, t2);
                // Solve [d1.x, -d2.x; d1.y, -d2.y] * [dt1; dt2] = -[err.x; err.y]
                let det = d1.x * (-d2.y) - d1.y * (-d2.x);
                if det.abs() < 1e-12 {
                    break; // tangents parallel, can't refine
                }
                let dt1 = (-d2.y * (-err.x) - (-d2.x) * (-err.y)) / det;
                let dt2 = (d1.x * (-err.y) - d1.y * (-err.x)) / det;
                t1 = (t1 + dt1).clamp(0.0, 1.0);
                t2 = (t2 + dt2).clamp(0.0, 1.0);
            }
            // If Newton diverged far from the initial estimate, it may have
            // jumped to a different crossing. Check if the refined result is
            // actually better than the original before rejecting.
            let p1_refined = orig_curve1.eval(t1);
            let p2_refined = orig_curve2.eval(t2);
            let err_refined = (p1_refined.x - p2_refined.x).powi(2)
                + (p1_refined.y - p2_refined.y).powi(2);

            if (t1 - t1_orig).abs() > (t1_end - t1_start) * 2.0
                || (t2 - t2_orig).abs() > (t2_end - t2_start) * 2.0
            {
                let p1_orig = orig_curve1.eval(t1_orig);
                let p2_orig = orig_curve2.eval(t2_orig);
                let err_orig = (p1_orig.x - p2_orig.x).powi(2)
                    + (p1_orig.y - p2_orig.y).powi(2);
                // Only fall back if the original is actually closer
                if err_orig < err_refined {
                    t1 = t1_orig;
                    t2 = t2_orig;
                }
            }

            let p1 = orig_curve1.eval(t1);
            let p2 = orig_curve2.eval(t2);
            (t1, t2, Point::new((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5))
        } else {
            // Lines are parallel/degenerate — fall back to midpoint
            let t1 = (t1_start + t1_end) / 2.0;
            let t2 = (t2_start + t2_end) / 2.0;
            (t1, t2, orig_curve1.eval(t1))
        };

        intersections.push(Intersection {
            t1,
            t2: Some(t2),
            point,
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

/// Remove duplicate intersections by clustering on parameter proximity.
///
/// Raw hits from subdivision can produce chains of near-duplicates spaced
/// just over the spatial tolerance (e.g. 4 hits at 1.02 px apart for a
/// single crossing of shallow-angle curves). Pairwise spatial dedup fails
/// on these chains. Instead, we sort by t1, cluster consecutive hits whose
/// t1 values are within `param_tol`, and keep the median of each cluster.
fn dedup_intersections(intersections: &mut Vec<Intersection>, _tolerance: f64) {
    if intersections.is_empty() {
        return;
    }

    const PARAM_TOL: f64 = 0.05;

    // Sort by t1 (primary) then t2 (secondary)
    intersections.sort_by(|a, b| {
        a.t1.partial_cmp(&b.t1)
            .unwrap()
            .then_with(|| {
                let at2 = a.t2.unwrap_or(0.0);
                let bt2 = b.t2.unwrap_or(0.0);
                at2.partial_cmp(&bt2).unwrap()
            })
    });

    // Cluster consecutive intersections that are close in both t1 and t2
    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut current_cluster = vec![0usize];

    for i in 1..intersections.len() {
        let prev = &intersections[*current_cluster.last().unwrap()];
        let curr = &intersections[i];
        let t1_close = (curr.t1 - prev.t1).abs() < PARAM_TOL;
        let t2_close = match (curr.t2, prev.t2) {
            (Some(a), Some(b)) => (a - b).abs() < PARAM_TOL,
            _ => true,
        };
        if t1_close && t2_close {
            current_cluster.push(i);
        } else {
            clusters.push(std::mem::take(&mut current_cluster));
            current_cluster = vec![i];
        }
    }
    clusters.push(current_cluster);

    // Keep the median of each cluster
    let mut result = Vec::with_capacity(clusters.len());
    for cluster in &clusters {
        let median_idx = cluster[cluster.len() / 2];
        result.push(intersections[median_idx].clone());
    }

    *intersections = result;
}

/// Derivative (tangent vector) of a cubic Bezier at parameter t.
///
/// B'(t) = 3[(1-t)²(P1-P0) + 2(1-t)t(P2-P1) + t²(P3-P2)]
fn cubic_deriv(c: &CubicBez, t: f64) -> Point {
    let u = 1.0 - t;
    let d0 = Point::new(c.p1.x - c.p0.x, c.p1.y - c.p0.y);
    let d1 = Point::new(c.p2.x - c.p1.x, c.p2.y - c.p1.y);
    let d2 = Point::new(c.p3.x - c.p2.x, c.p3.y - c.p2.y);
    Point::new(
        3.0 * (u * u * d0.x + 2.0 * u * t * d1.x + t * t * d2.x),
        3.0 * (u * u * d0.y + 2.0 * u * t * d1.y + t * t * d2.y),
    )
}

/// 2D line-line intersection.
///
/// Given line segment A (a0→a1) and line segment B (b0→b1),
/// returns `Some((s, u))` where `s` is the parameter on A and
/// `u` is the parameter on B at the intersection point.
/// Returns `None` if the lines are parallel or degenerate.
fn line_line_intersect(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<(f64, f64)> {
    let dx_a = a1.x - a0.x;
    let dy_a = a1.y - a0.y;
    let dx_b = b1.x - b0.x;
    let dy_b = b1.y - b0.y;

    let denom = dx_a * dy_b - dy_a * dx_b;
    if denom.abs() < 1e-12 {
        return None; // parallel or degenerate
    }

    let dx_ab = b0.x - a0.x;
    let dy_ab = b0.y - a0.y;

    let s = (dx_ab * dy_b - dy_ab * dx_b) / denom;
    let u = (dx_ab * dy_a - dy_ab * dx_a) / denom;

    Some((s, u))
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
