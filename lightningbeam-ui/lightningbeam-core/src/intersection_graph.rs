//! Intersection graph for paint bucket fill
//!
//! This module implements an incremental graph-building approach for finding
//! closed regions to fill. Instead of flood-filling, we:
//! 1. Start at a curve found via raycast from the click point
//! 2. Find all intersections on that curve (with other curves and itself)
//! 3. Walk the graph, choosing the "most clockwise" turn at each junction
//! 4. Incrementally add nearby curves as we encounter them
//! 5. Track visited segments to detect when we've completed a loop

use crate::curve_intersections::{find_closest_approach, find_curve_intersections, find_self_intersections};
use crate::curve_segment::CurveSegment;
use crate::gap_handling::GapHandlingMode;
use crate::tolerance_quadtree::ToleranceQuadtree;
use std::collections::HashSet;
use vello::kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveDeriv, ParamCurveNearest, Point};

/// A node in the intersection graph representing a point where curves meet
#[derive(Debug, Clone)]
pub struct IntersectionNode {
    /// Location of this node
    pub point: Point,

    /// Edges connected to this node
    pub edges: Vec<EdgeRef>,
}

/// Reference to an edge in the graph
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeRef {
    /// Index of the curve this edge follows
    pub curve_id: usize,

    /// Parameter value where this edge starts [0, 1]
    pub t_start: f64,

    /// Parameter value where this edge ends [0, 1]
    pub t_end: f64,

    /// Direction at the start of this edge (for angle calculations)
    pub start_tangent: Point,
}

/// A visited segment (for loop detection)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct VisitedSegment {
    curve_id: usize,
    /// t_start quantized to 0.01 precision for hashing
    t_start_quantized: i32,
    /// t_end quantized to 0.01 precision for hashing
    t_end_quantized: i32,
}

impl VisitedSegment {
    fn new(curve_id: usize, t_start: f64, t_end: f64) -> Self {
        Self {
            curve_id,
            t_start_quantized: (t_start * 100.0).round() as i32,
            t_end_quantized: (t_end * 100.0).round() as i32,
        }
    }
}

/// Result of walking the intersection graph
pub struct WalkResult {
    /// The closed path found by walking the graph
    pub path: Option<BezPath>,

    /// Debug information about the walk
    pub debug_info: WalkDebugInfo,
}

/// Debug information about the walk process
#[derive(Default)]
pub struct WalkDebugInfo {
    /// Number of segments walked
    pub segments_walked: usize,

    /// Number of intersections found
    pub intersections_found: usize,

    /// Number of gaps bridged
    pub gaps_bridged: usize,

    /// Whether the walk completed successfully
    pub completed: bool,

    /// Points visited during the walk (for visualization)
    pub visited_points: Vec<Point>,

    /// Segments walked during the graph traversal (curve_id, t_start, t_end)
    pub walked_segments: Vec<(usize, f64, f64)>,
}

/// Configuration for the intersection graph walk
pub struct WalkConfig {
    /// Gap tolerance in pixels
    pub tolerance: f64,

    /// Gap handling mode
    pub gap_mode: GapHandlingMode,

    /// Maximum number of segments to walk before giving up
    pub max_segments: usize,
}

impl Default for WalkConfig {
    fn default() -> Self {
        Self {
            tolerance: 2.0,
            gap_mode: GapHandlingMode::default(),
            max_segments: 10000,
        }
    }
}

/// Walk the intersection graph to find a closed path
///
/// # Arguments
///
/// * `start_point` - Point to start the walk (click point)
/// * `curves` - All curves in the scene
/// * `quadtree` - Spatial index for finding nearby curves
/// * `config` - Walk configuration
///
/// # Returns
///
/// A `WalkResult` with the closed path if one was found
pub fn walk_intersection_graph(
    start_point: Point,
    curves: &[CurveSegment],
    quadtree: &ToleranceQuadtree,
    config: &WalkConfig,
) -> WalkResult {
    let mut debug_info = WalkDebugInfo::default();

    // Step 1: Find the first curve via raycast
    let first_curve_id = match find_curve_at_point(start_point, curves) {
        Some(id) => id,
        None => {
            println!("No curve found at start point");
            return WalkResult {
                path: None,
                debug_info,
            };
        }
    };

    println!("Starting walk from curve {}", first_curve_id);

    // Step 2: Find a starting point on that curve
    let first_curve = &curves[first_curve_id];
    let nearest = first_curve.to_cubic_bez().nearest(start_point, 1e-6);
    let start_t = nearest.t;
    let start_pos = first_curve.to_cubic_bez().eval(start_t);

    debug_info.visited_points.push(start_pos);

    println!("Start position: ({:.1}, {:.1}) at t={:.3}", start_pos.x, start_pos.y, start_t);

    // Step 3: Walk the graph
    let mut path = BezPath::new();
    path.move_to(start_pos);

    let mut current_curve_id = first_curve_id;
    let mut current_t = start_t;
    let mut visited_segments = HashSet::new();
    let mut processed_curves = HashSet::new();
    processed_curves.insert(first_curve_id);

    // Convert CurveSegments to CubicBez for easier processing
    let cubic_curves: Vec<CubicBez> = curves.iter().map(|seg| seg.to_cubic_bez()).collect();

    for _iteration in 0..config.max_segments {
        debug_info.segments_walked += 1;

        // Find all intersections on current curve
        let intersections = find_intersections_on_curve(
            current_curve_id,
            &cubic_curves,
            &processed_curves,
            quadtree,
            config.tolerance,
            &mut debug_info,
        );

        println!("Found {} intersections on curve {}", intersections.len(), current_curve_id);

        // Find the next intersection point in the forward direction (just to get to an intersection)
        let next_intersection_point = intersections
            .iter()
            .filter(|i| i.t_on_current > current_t + 0.01) // Small epsilon to avoid same point
            .min_by(|a, b| a.t_on_current.partial_cmp(&b.t_on_current).unwrap());

        let next_intersection_point = match next_intersection_point {
            Some(i) => i,
            None => {
                // Try wrapping around (for closed curves)
                let wrapped = intersections
                    .iter()
                    .filter(|i| i.t_on_current < current_t - 0.01)
                    .min_by(|a, b| a.t_on_current.partial_cmp(&b.t_on_current).unwrap());

                match wrapped {
                    Some(i) => i,
                    None => {
                        println!("No next intersection found, walk failed");
                        break;
                    }
                }
            }
        };

        println!("Reached intersection at t={:.3} on curve {}, point: ({:.1}, {:.1})",
                 next_intersection_point.t_on_current,
                 current_curve_id,
                 next_intersection_point.point.x,
                 next_intersection_point.point.y);

        // Add segment from current position to intersection
        let segment = extract_curve_segment(
            &cubic_curves[current_curve_id],
            current_t,
            next_intersection_point.t_on_current,
        );
        add_segment_to_path(&mut path, &segment, config.gap_mode);

        // Record this segment for debug visualization
        debug_info.walked_segments.push((
            current_curve_id,
            current_t,
            next_intersection_point.t_on_current,
        ));

        // Mark this segment as visited
        let visited = VisitedSegment::new(
            current_curve_id,
            current_t,
            next_intersection_point.t_on_current,
        );

        // Check if we've completed a loop
        if visited_segments.contains(&visited) {
            println!("Loop detected! Walk complete");
            debug_info.completed = true;
            path.close_path();
            break;
        }

        visited_segments.insert(visited);
        debug_info.visited_points.push(next_intersection_point.point);

        // Now at the intersection point, we need to choose which curve to follow next
        // by finding all curves at this point and choosing the rightmost turn

        // Calculate incoming direction (tangent at the end of the segment we just walked)
        let incoming_deriv = cubic_curves[current_curve_id].deriv().eval(next_intersection_point.t_on_current);
        let incoming_angle = incoming_deriv.y.atan2(incoming_deriv.x);

        // For boundary walking, we measure angles from the REVERSE of the incoming direction
        // (i.e., where we came FROM, not where we're going)
        let reverse_incoming_angle = (incoming_angle + std::f64::consts::PI) % (2.0 * std::f64::consts::PI);

        println!("Incoming angle: {:.2} rad ({:.1} deg), reverse: {:.2} rad ({:.1} deg)",
                 incoming_angle, incoming_angle.to_degrees(),
                 reverse_incoming_angle, reverse_incoming_angle.to_degrees());

        // Find ALL intersections at this point (within tolerance)
        let intersection_point = next_intersection_point.point;
        let mut candidates: Vec<(usize, f64, f64, bool)> = Vec::new(); // (curve_id, t, angle_from_incoming, is_gap)

        // Query the quadtree to find ALL curves at this intersection point
        // Create a small bounding box around the point
        use crate::quadtree::BoundingBox;
        let search_bbox = BoundingBox {
            x_min: intersection_point.x - config.tolerance,
            x_max: intersection_point.x + config.tolerance,
            y_min: intersection_point.y - config.tolerance,
            y_max: intersection_point.y + config.tolerance,
        };
        let nearby_curves = quadtree.get_curves_in_region(&search_bbox);

        println!("Querying quadtree at ({:.1}, {:.1}) found {} nearby curves",
                 intersection_point.x, intersection_point.y, nearby_curves.len());

        // ALSO check ALL curves to see if any pass through this intersection
        // (in case quadtree isn't finding everything)
        let mut all_curves_at_point = nearby_curves.clone();
        for curve_id in 0..cubic_curves.len() {
            if !nearby_curves.contains(&curve_id) {
                let curve_bez = &cubic_curves[curve_id];
                let nearest = curve_bez.nearest(intersection_point, 1e-6);
                let point_on_curve = curve_bez.eval(nearest.t);
                let dist = (point_on_curve - intersection_point).hypot();
                if dist < config.tolerance {
                    println!("  EXTRA: Curve {} found by brute-force check at t={:.3}, dist={:.4}", curve_id, nearest.t, dist);
                    all_curves_at_point.insert(curve_id);
                }
            }
        }

        let nearby_curves: Vec<usize> = all_curves_at_point.into_iter().collect();

        for &curve_id in &nearby_curves {
            // Find the t value on this curve closest to the intersection point
            let curve_bez = &cubic_curves[curve_id];
            let nearest = curve_bez.nearest(intersection_point, 1e-6);
            let t_on_curve = nearest.t;
            let point_on_curve = curve_bez.eval(t_on_curve);
            let dist = (point_on_curve - intersection_point).hypot();

            println!("  Curve {} at t={:.3}, dist={:.4}", curve_id, t_on_curve, dist);

            if dist < config.tolerance {
                // This curve passes through (or very near) the intersection point
                let is_gap = dist > config.tolerance * 0.1; // Consider it a gap if not very close

                // Forward direction (increasing t)
                let forward_deriv = curve_bez.deriv().eval(t_on_curve);
                let forward_angle = forward_deriv.y.atan2(forward_deriv.x);
                let forward_angle_diff = normalize_angle(forward_angle - reverse_incoming_angle);

                // Don't add this candidate if it's going back exactly where we came from
                // (same curve, same t, same direction)
                let is_reverse_on_current = curve_id == current_curve_id &&
                                           (t_on_curve - next_intersection_point.t_on_current).abs() < 0.01 &&
                                           forward_angle_diff < 0.1;

                if !is_reverse_on_current {
                    candidates.push((curve_id, t_on_curve, forward_angle_diff, is_gap));
                }

                // Backward direction (decreasing t) - reverse the tangent
                let backward_angle = (forward_angle + std::f64::consts::PI) % (2.0 * std::f64::consts::PI);
                let backward_angle_diff = normalize_angle(backward_angle - reverse_incoming_angle);

                let is_reverse_on_current_backward = curve_id == current_curve_id &&
                                                     (t_on_curve - next_intersection_point.t_on_current).abs() < 0.01 &&
                                                     backward_angle_diff < 0.1;

                if !is_reverse_on_current_backward {
                    candidates.push((curve_id, t_on_curve, backward_angle_diff, is_gap));
                }
            }
        }

        println!("Found {} candidate outgoing edges", candidates.len());
        for (i, (cid, t, angle, is_gap)) in candidates.iter().enumerate() {
            println!("  Candidate {}: curve={}, t={:.3}, angle_diff={:.2} rad ({:.1} deg), gap={}",
                     i, cid, t, angle, angle.to_degrees(), is_gap);
        }

        // Choose the edge with the smallest positive angle (sharpest right turn for clockwise)
        // Now that we measure from reverse_incoming_angle:
        // - 0° = going back the way we came (filter out)
        // - Small angles like 30°-90° = sharp right turn (what we want)
        // - 180° = continuing straight (valid - don't filter)
        // IMPORTANT:
        // 1. Prefer non-gap edges over gap edges
        // 2. When angles are equal, prefer switching to a different curve
        let best_edge = candidates
            .iter()
            .filter(|(_cid, _, angle_diff, _)| {
                // Don't go back the way we came (angle near 0)
                let is_reverse = *angle_diff < 0.1;
                !is_reverse
            })
            .min_by(|a, b| {
                // First, prefer non-gap edges over gap edges
                match (a.3, b.3) {
                    (false, true) => std::cmp::Ordering::Less,  // a is non-gap, b is gap -> prefer a
                    (true, false) => std::cmp::Ordering::Greater, // a is gap, b is non-gap -> prefer b
                    _ => {
                        // Both same gap status -> compare angles
                        let angle_diff = (a.2 - b.2).abs();
                        const ANGLE_EPSILON: f64 = 0.01; // ~0.57 degrees tolerance for "equal" angles

                        if angle_diff < ANGLE_EPSILON {
                            // Angles are effectively equal - prefer different curve over same curve
                            let a_is_current = a.0 == current_curve_id;
                            let b_is_current = b.0 == current_curve_id;
                            match (a_is_current, b_is_current) {
                                (true, false) => std::cmp::Ordering::Greater,  // a is current, b is different -> prefer b
                                (false, true) => std::cmp::Ordering::Less,    // a is different, b is current -> prefer a
                                _ => a.2.partial_cmp(&b.2).unwrap(),          // Both same or both different -> fall back to angle
                            }
                        } else {
                            a.2.partial_cmp(&b.2).unwrap()
                        }
                    }
                }
            });

        let (next_curve_id, next_t, chosen_angle, is_gap) = match best_edge {
            Some(&(cid, t, angle, gap)) => (cid, t, angle, gap),
            None => {
                println!("No valid outgoing edge found!");
                break;
            }
        };

        println!("Chose: curve={}, t={:.3}, angle={:.2} rad, gap={}",
                 next_curve_id, next_t, chosen_angle, is_gap);

        // Handle gap if needed
        if is_gap {
            debug_info.gaps_bridged += 1;

            match config.gap_mode {
                GapHandlingMode::BridgeSegment => {
                    // Add a line segment to bridge the gap
                    let current_end = intersection_point;
                    let next_start = cubic_curves[next_curve_id].eval(next_t);
                    path.line_to(next_start);
                    println!("Bridged gap: ({:.1}, {:.1}) -> ({:.1}, {:.1})",
                             current_end.x, current_end.y, next_start.x, next_start.y);
                }
                GapHandlingMode::SnapAndSplit => {
                    // Snap to midpoint (geometry modification handled in segment extraction)
                    println!("Snapped to gap midpoint");
                }
            }
        }

        // Move to next curve
        processed_curves.insert(next_curve_id);
        current_curve_id = next_curve_id;
        current_t = next_t;

        // Check if we've returned to start
        let current_pos = cubic_curves[current_curve_id].eval(current_t);
        let dist_to_start = (current_pos - start_pos).hypot();

        if dist_to_start < config.tolerance && current_curve_id == first_curve_id {
            println!("Returned to start! Walk complete");
            debug_info.completed = true;
            path.close_path();
            break;
        }
    }

    println!("Walk finished: {} segments, {} intersections, {} gaps",
             debug_info.segments_walked, debug_info.intersections_found, debug_info.gaps_bridged);

    WalkResult {
        path: if debug_info.completed { Some(path) } else { None },
        debug_info,
    }
}

/// Information about an intersection found on a curve
#[derive(Debug, Clone)]
struct CurveIntersection {
    /// Parameter on current curve
    t_on_current: f64,

    /// Parameter on other curve
    t_on_other: f64,

    /// ID of the other curve
    other_curve_id: usize,

    /// Intersection point
    point: Point,

    /// Whether this is a gap (within tolerance but not exact intersection)
    is_gap: bool,
}

/// Find all intersections on a given curve
fn find_intersections_on_curve(
    curve_id: usize,
    curves: &[CubicBez],
    processed_curves: &HashSet<usize>,
    quadtree: &ToleranceQuadtree,
    tolerance: f64,
    debug_info: &mut WalkDebugInfo,
) -> Vec<CurveIntersection> {
    let mut intersections = Vec::new();
    let current_curve = &curves[curve_id];

    // Find nearby curves using quadtree
    let nearby = quadtree.get_nearby_curves(current_curve);

    for &other_id in &nearby {
        if other_id == curve_id {
            // Check for self-intersections
            let self_ints = find_self_intersections(current_curve);
            for int in self_ints {
                intersections.push(CurveIntersection {
                    t_on_current: int.t1,
                    t_on_other: int.t2.unwrap_or(int.t1),
                    other_curve_id: curve_id,
                    point: int.point,
                    is_gap: false,
                });
                debug_info.intersections_found += 1;
            }
        } else {
            let other_curve = &curves[other_id];

            // Find exact intersections
            let exact_ints = find_curve_intersections(current_curve, other_curve);
            for int in exact_ints {
                intersections.push(CurveIntersection {
                    t_on_current: int.t1,
                    t_on_other: int.t2.unwrap_or(0.0),
                    other_curve_id: other_id,
                    point: int.point,
                    is_gap: false,
                });
                debug_info.intersections_found += 1;
            }

            // Find close approaches (gaps within tolerance)
            if let Some(approach) = find_closest_approach(current_curve, other_curve, tolerance) {
                intersections.push(CurveIntersection {
                    t_on_current: approach.t1,
                    t_on_other: approach.t2,
                    other_curve_id: other_id,
                    point: approach.p1,
                    is_gap: true,
                });
            }
        }
    }

    // Sort by t_on_current for easier processing
    intersections.sort_by(|a, b| a.t_on_current.partial_cmp(&b.t_on_current).unwrap());

    intersections
}

/// Find a curve at the given point via raycast
fn find_curve_at_point(point: Point, curves: &[CurveSegment]) -> Option<usize> {
    let mut min_dist = f64::MAX;
    let mut closest_id = None;

    for (i, curve) in curves.iter().enumerate() {
        let cubic = curve.to_cubic_bez();
        let nearest = cubic.nearest(point, 1e-6);
        let dist = (cubic.eval(nearest.t) - point).hypot();

        if dist < min_dist {
            min_dist = dist;
            closest_id = Some(i);
        }
    }

    // Only accept if within reasonable distance
    if min_dist < 50.0 {
        closest_id
    } else {
        None
    }
}

/// Extract a subsegment of a curve between two t parameters
fn extract_curve_segment(curve: &CubicBez, t_start: f64, t_end: f64) -> CubicBez {
    // Clamp parameters
    let t_start = t_start.clamp(0.0, 1.0);
    let t_end = t_end.clamp(0.0, 1.0);

    if t_start >= t_end {
        // Degenerate segment, return a point
        let p = curve.eval(t_start);
        return CubicBez::new(p, p, p, p);
    }

    // Use de Casteljau's algorithm to extract subsegment
    curve.subsegment(t_start..t_end)
}

/// Add a curve segment to the path
fn add_segment_to_path(path: &mut BezPath, segment: &CubicBez, _gap_mode: GapHandlingMode) {
    // Add as cubic bezier curve
    path.curve_to(segment.p1, segment.p2, segment.p3);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::kurbo::Circle;

    #[test]
    fn test_visited_segment_quantization() {
        // VisitedSegment quantizes to 0.01 precision (t * 100 rounded)
        // Values differing by less than 0.005 will round to the same value
        let seg1 = VisitedSegment::new(0, 0.123, 0.456);
        let seg2 = VisitedSegment::new(0, 0.135, 0.467); // Differs by > 0.01 to ensure different quantization
        let seg3 = VisitedSegment::new(0, 0.123, 0.456);
        let seg4 = VisitedSegment::new(0, 0.124, 0.457); // Within 0.01 - should be same as seg1

        // Different quantized values
        assert_ne!(seg1, seg2);

        // Same exact values
        assert_eq!(seg1, seg3);

        // seg4 has values within 0.01 of seg1, so they quantize to the same
        // 0.123 * 100 = 12.3 -> 12, 0.124 * 100 = 12.4 -> 12
        // 0.456 * 100 = 45.6 -> 46, 0.457 * 100 = 45.7 -> 46
        assert_eq!(seg1, seg4);
    }

    #[test]
    fn test_extract_curve_segment() {
        let curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        );

        let segment = extract_curve_segment(&curve, 0.25, 0.75);

        // Segment should start and end at expected points
        let start = segment.eval(0.0);
        let end = segment.eval(1.0);

        let expected_start = curve.eval(0.25);
        let expected_end = curve.eval(0.75);

        assert!((start - expected_start).hypot() < 1.0);
        assert!((end - expected_end).hypot() < 1.0);
    }

    #[test]
    fn test_find_curve_at_point() {
        use crate::curve_segment::CurveType;

        let curves = vec![
            CurveSegment::new(
                0,
                0,
                CurveType::Cubic,
                0.0,
                1.0,
                vec![
                    Point::new(0.0, 0.0),
                    Point::new(100.0, 0.0),
                    Point::new(100.0, 100.0),
                    Point::new(0.0, 100.0),
                ],
            ),
        ];

        // Point on the curve should find it
        let found = find_curve_at_point(Point::new(50.0, 25.0), &curves);
        assert!(found.is_some());

        // Point far away should not find it
        let not_found = find_curve_at_point(Point::new(1000.0, 1000.0), &curves);
        assert!(not_found.is_none());
    }
}

/// Normalize an angle difference to the range [0, 2*PI)
/// This is used to calculate the clockwise angle from one direction to another
fn normalize_angle(angle: f64) -> f64 {
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut result = angle % two_pi;
    if result < 0.0 {
        result += two_pi;
    }
    // Handle floating point precision: if very close to 2π, wrap to 0
    if result > two_pi - 0.01 {
        result = 0.0;
    }
    result
}
