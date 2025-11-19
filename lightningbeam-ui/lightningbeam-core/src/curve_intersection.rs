//! Curve intersection algorithm using recursive subdivision
//!
//! This module implements intersection finding between Bezier curve segments
//! using a recursive subdivision algorithm similar to the one in bezier.js.
//! The algorithm is based on the paper "Intersection of Two Bezier Curves"
//! and uses bounding box tests to prune the search space.

use crate::curve_segment::CurveSegment;
use vello::kurbo::Point;

/// Result of a curve intersection test
#[derive(Debug, Clone)]
pub struct CurveIntersection {
    /// Parameter t on the first curve (in range [0, 1])
    pub t1: f64,
    /// Parameter t on the second curve (in range [0, 1])
    pub t2: f64,
    /// Point of intersection
    pub point: Point,
}

/// Find all intersections between two curve segments
///
/// Uses recursive subdivision with bounding box pruning.
/// The threshold determines when curves are considered "small enough"
/// to return an intersection point.
///
/// # Parameters
/// - `curve1`: First curve segment
/// - `curve2`: Second curve segment
/// - `threshold`: Size threshold for convergence (sum of bbox widths + heights)
///
/// # Returns
/// Vector of intersection points with parameters on both curves
pub fn find_intersections(
    curve1: &CurveSegment,
    curve2: &CurveSegment,
    threshold: f64,
) -> Vec<CurveIntersection> {
    let mut results = Vec::new();
    pair_iteration(curve1, curve2, threshold, &mut results);
    results
}

/// Recursive subdivision algorithm for finding curve intersections
///
/// This is the core algorithm that mirrors the JavaScript bezier.js implementation.
fn pair_iteration(
    c1: &CurveSegment,
    c2: &CurveSegment,
    threshold: f64,
    results: &mut Vec<CurveIntersection>,
) {
    // 1. Check if bounding boxes overlap - early exit if not
    let bbox1 = c1.bounding_box();
    let bbox2 = c2.bounding_box();

    if !bbox1.intersects(&bbox2) {
        return;
    }

    // 2. Base case: curves are small enough
    let combined_size = bbox1.size() + bbox2.size();
    if combined_size < threshold {
        // Found an intersection - compute the midpoint parameters
        let t1_mid = (c1.t_start + c1.t_end) / 2.0;
        let t2_mid = (c2.t_start + c2.t_end) / 2.0;

        // Evaluate at midpoints to get intersection point
        // Average the two points for better accuracy
        let p1 = c1.eval_at(0.5);
        let p2 = c2.eval_at(0.5);
        let point = Point::new((p1.x + p2.x) / 2.0, (p1.y + p2.y) / 2.0);

        results.push(CurveIntersection {
            t1: t1_mid,
            t2: t2_mid,
            point,
        });
        return;
    }

    // 3. Recursive case: split both curves and test all 4 pairs
    let (c1_left, c1_right) = c1.split_at(0.5);
    let (c2_left, c2_right) = c2.split_at(0.5);

    // Test all 4 combinations:
    // (c1_left, c2_left), (c1_left, c2_right), (c1_right, c2_left), (c1_right, c2_right)
    pair_iteration(&c1_left, &c2_left, threshold, results);
    pair_iteration(&c1_left, &c2_right, threshold, results);
    pair_iteration(&c1_right, &c2_left, threshold, results);
    pair_iteration(&c1_right, &c2_right, threshold, results);
}

/// Find intersection between a curve and a line segment
///
/// This is a specialized version for line-curve intersections which can be
/// more efficient than the general curve-curve intersection.
pub fn find_line_curve_intersections(
    line: &CurveSegment,
    curve: &CurveSegment,
    threshold: f64,
) -> Vec<CurveIntersection> {
    // For now, just use the general algorithm
    // TODO: Optimize with line-specific tests
    find_intersections(line, curve, threshold)
}

/// Check if two curves intersect (without computing exact intersection points)
///
/// This is faster than find_intersections when you only need to know
/// whether curves intersect, not where.
pub fn curves_intersect(c1: &CurveSegment, c2: &CurveSegment, threshold: f64) -> bool {
    curves_intersect_internal(c1, c2, threshold)
}

fn curves_intersect_internal(c1: &CurveSegment, c2: &CurveSegment, threshold: f64) -> bool {
    // Check if bounding boxes overlap
    let bbox1 = c1.bounding_box();
    let bbox2 = c2.bounding_box();

    if !bbox1.intersects(&bbox2) {
        return false;
    }

    // Base case: curves are small enough
    let combined_size = bbox1.size() + bbox2.size();
    if combined_size < threshold {
        return true;
    }

    // Recursive case: split and test
    let (c1_left, c1_right) = c1.split_at(0.5);
    let (c2_left, c2_right) = c2.split_at(0.5);

    curves_intersect_internal(&c1_left, &c2_left, threshold)
        || curves_intersect_internal(&c1_left, &c2_right, threshold)
        || curves_intersect_internal(&c1_right, &c2_left, threshold)
        || curves_intersect_internal(&c1_right, &c2_right, threshold)
}

/// Remove duplicate intersections that are very close to each other
///
/// The recursive subdivision algorithm can find the same intersection
/// multiple times from different branches. This function deduplicates
/// intersections that are within `epsilon` distance of each other.
pub fn deduplicate_intersections(
    intersections: &[CurveIntersection],
    epsilon: f64,
) -> Vec<CurveIntersection> {
    let mut unique = Vec::new();
    let epsilon_sq = epsilon * epsilon;

    for intersection in intersections {
        // Check if this intersection is close to any existing one
        let is_duplicate = unique.iter().any(|existing: &CurveIntersection| {
            let dx = intersection.point.x - existing.point.x;
            let dy = intersection.point.y - existing.point.y;
            dx * dx + dy * dy < epsilon_sq
        });

        if !is_duplicate {
            unique.push(intersection.clone());
        }
    }

    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve_segment::{CurveSegment, CurveType};

    #[test]
    fn test_line_line_intersection() {
        // Two lines that cross at (50, 50)
        let line1 = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0)],
        );

        let line2 = CurveSegment::new(
            1,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 100.0), Point::new(100.0, 0.0)],
        );

        let intersections = find_intersections(&line1, &line2, 1.0);

        assert!(!intersections.is_empty());

        // Should find intersection near (50, 50)
        let intersection = &intersections[0];
        assert!((intersection.point.x - 50.0).abs() < 5.0);
        assert!((intersection.point.y - 50.0).abs() < 5.0);
    }

    #[test]
    fn test_parallel_lines_no_intersection() {
        // Two parallel lines that don't intersect
        let line1 = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
        );

        let line2 = CurveSegment::new(
            1,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 10.0), Point::new(100.0, 10.0)],
        );

        let intersections = find_intersections(&line1, &line2, 1.0);

        assert!(intersections.is_empty());
    }

    #[test]
    fn test_curves_intersect_check() {
        // Two lines that cross
        let line1 = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0)],
        );

        let line2 = CurveSegment::new(
            1,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 100.0), Point::new(100.0, 0.0)],
        );

        assert!(curves_intersect(&line1, &line2, 1.0));
    }

    #[test]
    fn test_no_intersection_check() {
        // Two lines that don't intersect
        let line1 = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(10.0, 0.0)],
        );

        let line2 = CurveSegment::new(
            1,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(20.0, 0.0), Point::new(30.0, 0.0)],
        );

        assert!(!curves_intersect(&line1, &line2, 1.0));
    }

    #[test]
    fn test_deduplicate_intersections() {
        let intersections = vec![
            CurveIntersection {
                t1: 0.5,
                t2: 0.5,
                point: Point::new(50.0, 50.0),
            },
            CurveIntersection {
                t1: 0.50001,
                t2: 0.50001,
                point: Point::new(50.001, 50.001),
            },
            CurveIntersection {
                t1: 0.7,
                t2: 0.3,
                point: Point::new(70.0, 30.0),
            },
        ];

        let unique = deduplicate_intersections(&intersections, 0.1);

        // First two should be deduplicated, third should remain
        assert_eq!(unique.len(), 2);
    }

    #[test]
    fn test_quadratic_curve_intersection() {
        // Line from (0, 50) to (100, 50)
        let line = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 50.0), Point::new(100.0, 50.0)],
        );

        // Quadratic curve that crosses the line
        let quad = CurveSegment::new(
            1,
            0,
            CurveType::Quadratic,
            0.0,
            1.0,
            vec![
                Point::new(50.0, 0.0),
                Point::new(50.0, 100.0),
                Point::new(50.0, 100.0),
            ],
        );

        let intersections = find_intersections(&line, &quad, 1.0);

        // Should find at least one intersection
        assert!(!intersections.is_empty());
    }
}
