//! BezPath editing utilities for vector shape manipulation
//!
//! Provides functions to convert BezPath to/from editable bezier curves,
//! generate vertices, and implement curve manipulation algorithms like moldCurve.

use crate::bezier_vertex::{BezierVertex, EditableBezierCurves};
use vello::kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveNearest, PathEl, Point};

/// Tolerance for merging nearby vertices (in pixels)
pub const VERTEX_MERGE_EPSILON: f64 = 1.5;

/// Default epsilon for moldCurve numerical differentiation
const MOLD_CURVE_EPSILON: f64 = 0.01;

/// Extract editable curves and vertices from a BezPath
///
/// Converts all path elements to cubic bezier curves and generates vertices
/// by merging nearby endpoints. This creates a structure suitable for
/// vertex and curve editing operations.
///
/// # Arguments
///
/// * `path` - The BezPath to extract from
///
/// # Returns
///
/// EditableBezierCurves containing curves, vertices, and closure status
pub fn extract_editable_curves(path: &BezPath) -> EditableBezierCurves {
    let mut curves = Vec::new();
    let mut current_point = Point::ZERO;
    let mut start_point = Point::ZERO;
    let mut first_point_set = false;

    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                current_point = *p;
                start_point = *p;
                first_point_set = true;
            }
            PathEl::LineTo(p) => {
                if first_point_set {
                    curves.push(line_to_cubic(current_point, *p));
                    current_point = *p;
                }
            }
            PathEl::QuadTo(p1, p2) => {
                if first_point_set {
                    curves.push(quad_to_cubic(current_point, *p1, *p2));
                    current_point = *p2;
                }
            }
            PathEl::CurveTo(p1, p2, p3) => {
                if first_point_set {
                    curves.push(CubicBez::new(current_point, *p1, *p2, *p3));
                    current_point = *p3;
                }
            }
            PathEl::ClosePath => {
                // Add closing line if needed
                if first_point_set && (current_point - start_point).hypot() > 1e-6 {
                    curves.push(line_to_cubic(current_point, start_point));
                    current_point = start_point;
                }
            }
        }
    }

    let vertices = generate_vertices(&curves);
    let is_closed = !curves.is_empty()
        && (curves[0].p0 - curves.last().unwrap().p3).hypot() < VERTEX_MERGE_EPSILON;

    EditableBezierCurves {
        curves,
        vertices,
        is_closed,
    }
}

/// Rebuild a BezPath from editable curves
///
/// Converts the editable curve structure back into a BezPath for rendering.
///
/// # Arguments
///
/// * `editable` - The editable curves structure
///
/// # Returns
///
/// A BezPath ready for rendering
pub fn rebuild_bezpath(editable: &EditableBezierCurves) -> BezPath {
    let mut path = BezPath::new();

    if editable.curves.is_empty() {
        return path;
    }

    path.move_to(editable.curves[0].p0);

    for curve in &editable.curves {
        path.curve_to(curve.p1, curve.p2, curve.p3);
    }

    if editable.is_closed {
        path.close_path();
    }

    path
}

/// Convert a line segment to a cubic bezier curve
///
/// Places control points at 1/3 and 2/3 along the line so the cubic
/// bezier exactly represents the straight line.
fn line_to_cubic(p0: Point, p3: Point) -> CubicBez {
    let p1 = Point::new(p0.x + (p3.x - p0.x) / 3.0, p0.y + (p3.y - p0.y) / 3.0);
    let p2 = Point::new(
        p0.x + 2.0 * (p3.x - p0.x) / 3.0,
        p0.y + 2.0 * (p3.y - p0.y) / 3.0,
    );
    CubicBez::new(p0, p1, p2, p3)
}

/// Convert a quadratic bezier to a cubic bezier
///
/// Uses the standard quadratic-to-cubic conversion formula.
fn quad_to_cubic(p0: Point, p1: Point, p2: Point) -> CubicBez {
    // Standard quadratic to cubic conversion formula
    let c1 = Point::new(
        p0.x + 2.0 * (p1.x - p0.x) / 3.0,
        p0.y + 2.0 * (p1.y - p0.y) / 3.0,
    );
    let c2 = Point::new(
        p2.x + 2.0 * (p1.x - p2.x) / 3.0,
        p2.y + 2.0 * (p1.y - p2.y) / 3.0,
    );
    CubicBez::new(p0, c1, c2, p2)
}

/// Generate vertices from curve endpoints
///
/// Creates vertices by merging nearby endpoints (within VERTEX_MERGE_EPSILON).
/// Each vertex tracks which curves start and end at that point.
///
/// # Arguments
///
/// * `curves` - The array of cubic bezier curves
///
/// # Returns
///
/// A vector of BezierVertex structs with connection information
fn generate_vertices(curves: &[CubicBez]) -> Vec<BezierVertex> {
    let mut vertices = Vec::new();

    for (i, curve) in curves.iter().enumerate() {
        // Process start point (p0)
        add_or_merge_vertex(&mut vertices, curve.p0, i, true);

        // Process end point (p3)
        add_or_merge_vertex(&mut vertices, curve.p3, i, false);
    }

    vertices
}

/// Add a point as a new vertex or merge with existing nearby vertex
///
/// If a vertex already exists within VERTEX_MERGE_EPSILON, the curve
/// is added to that vertex's connection list. Otherwise, a new vertex
/// is created.
fn add_or_merge_vertex(
    vertices: &mut Vec<BezierVertex>,
    point: Point,
    curve_index: usize,
    is_start: bool,
) {
    // Check if a vertex already exists at this point (within epsilon)
    for vertex in vertices.iter_mut() {
        let dist = (vertex.point - point).hypot();
        if dist < VERTEX_MERGE_EPSILON {
            // Merge with existing vertex
            if is_start {
                if !vertex.start_curves.contains(&curve_index) {
                    vertex.start_curves.push(curve_index);
                }
            } else {
                if !vertex.end_curves.contains(&curve_index) {
                    vertex.end_curves.push(curve_index);
                }
            }
            return;
        }
    }

    // Create new vertex
    let mut vertex = BezierVertex::new(point);
    if is_start {
        vertex.start_curves.push(curve_index);
    } else {
        vertex.end_curves.push(curve_index);
    }

    vertices.push(vertex);
}

/// Reshape a cubic bezier curve by dragging a point on it (moldCurve algorithm)
///
/// This uses numerical differentiation to calculate how the control points
/// should move to make the curve pass through the mouse position while keeping
/// endpoints fixed. The algorithm is based on the JavaScript UI implementation.
///
/// # Algorithm
///
/// 1. Project old_mouse onto the curve to find the grab parameter t
/// 2. Create offset curves by nudging each control point by epsilon
/// 3. Evaluate offset curves at parameter t to get derivatives
/// 4. Calculate control point adjustments weighted by t
/// 5. Return curve with adjusted control points and same endpoints
///
/// # Arguments
///
/// * `curve` - The original curve
/// * `mouse` - The target position (where we want the curve to go)
/// * `old_mouse` - The starting position (where the drag started)
/// * `epsilon` - Step size for numerical differentiation (optional)
///
/// # Returns
///
/// A new CubicBez with adjusted control points
///
/// # Reference
///
/// Based on `src/main.js` lines 551-602 in the JavaScript UI
pub fn mold_curve(curve: &CubicBez, mouse: &Point, old_mouse: &Point) -> CubicBez {
    mold_curve_with_epsilon(curve, mouse, old_mouse, MOLD_CURVE_EPSILON)
}

/// Mold curve with custom epsilon (for testing or fine-tuning)
pub fn mold_curve_with_epsilon(
    curve: &CubicBez,
    mouse: &Point,
    old_mouse: &Point,
    epsilon: f64,
) -> CubicBez {
    // Step 1: Find the closest point on the curve to old_mouse
    let nearest = curve.nearest(*old_mouse, 1e-6);
    let t = nearest.t;
    let projection = curve.eval(t);

    // Step 2: Create offset curves by moving each control point by epsilon
    let offset_p1 = Point::new(curve.p1.x + epsilon, curve.p1.y + epsilon);
    let offset_p2 = Point::new(curve.p2.x + epsilon, curve.p2.y + epsilon);

    let offset_curve_p1 = CubicBez::new(curve.p0, offset_p1, curve.p2, curve.p3);
    let offset_curve_p2 = CubicBez::new(curve.p0, curve.p1, offset_p2, curve.p3);

    // Step 3: Evaluate offset curves at parameter t
    let offset1 = offset_curve_p1.eval(t);
    let offset2 = offset_curve_p2.eval(t);

    // Step 4: Calculate derivatives (numerical differentiation)
    let derivative_p1_x = (offset1.x - projection.x) / epsilon;
    let derivative_p1_y = (offset1.y - projection.y) / epsilon;
    let derivative_p2_x = (offset2.x - projection.x) / epsilon;
    let derivative_p2_y = (offset2.y - projection.y) / epsilon;

    // Step 5: Calculate how much to move control points
    let delta_x = mouse.x - projection.x;
    let delta_y = mouse.y - projection.y;

    // Weight by parameter t: p1 affects curve more at t=0, p2 more at t=1
    let weight_p1 = 1.0 - t * t; // Stronger near start
    let weight_p2 = t * t; // Stronger near end

    // Avoid division by zero
    let adjust_p1_x = if derivative_p1_x.abs() > 1e-10 {
        (delta_x / derivative_p1_x) * weight_p1
    } else {
        0.0
    };
    let adjust_p1_y = if derivative_p1_y.abs() > 1e-10 {
        (delta_y / derivative_p1_y) * weight_p1
    } else {
        0.0
    };
    let adjust_p2_x = if derivative_p2_x.abs() > 1e-10 {
        (delta_x / derivative_p2_x) * weight_p2
    } else {
        0.0
    };
    let adjust_p2_y = if derivative_p2_y.abs() > 1e-10 {
        (delta_y / derivative_p2_y) * weight_p2
    } else {
        0.0
    };

    let new_p1 = Point::new(curve.p1.x + adjust_p1_x, curve.p1.y + adjust_p1_y);
    let new_p2 = Point::new(curve.p2.x + adjust_p2_x, curve.p2.y + adjust_p2_y);

    // Return updated curve with same endpoints
    CubicBez::new(curve.p0, new_p1, new_p2, curve.p3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_to_cubic() {
        let p0 = Point::new(0.0, 0.0);
        let p3 = Point::new(100.0, 100.0);
        let cubic = line_to_cubic(p0, p3);

        // Check endpoints
        assert_eq!(cubic.p0, p0);
        assert_eq!(cubic.p3, p3);

        // Check that control points are collinear (on the line)
        // Middle of line should be at (50, 50)
        let mid = cubic.eval(0.5);
        assert!((mid.x - 50.0).abs() < 0.01);
        assert!((mid.y - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_extract_and_rebuild_bezpath() {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        path.line_to((100.0, 100.0));
        path.line_to((0.0, 100.0));
        path.close_path();

        let editable = extract_editable_curves(&path);
        assert_eq!(editable.curves.len(), 4); // 4 line segments
        assert!(editable.is_closed);

        let rebuilt = rebuild_bezpath(&editable);
        // Rebuilt path should have same shape
        assert!(!rebuilt.is_empty());
    }

    #[test]
    fn test_vertex_generation() {
        let curves = vec![
            CubicBez::new(
                Point::new(0.0, 0.0),
                Point::new(33.0, 0.0),
                Point::new(66.0, 0.0),
                Point::new(100.0, 0.0),
            ),
            CubicBez::new(
                Point::new(100.0, 0.0),
                Point::new(100.0, 33.0),
                Point::new(100.0, 66.0),
                Point::new(100.0, 100.0),
            ),
        ];

        let vertices = generate_vertices(&curves);

        // Should have 3 vertices: start of curve 0, junction, end of curve 1
        assert_eq!(vertices.len(), 3);

        // Middle vertex should connect both curves
        let middle_vertex = vertices.iter().find(|v| {
            let dist = (v.point - Point::new(100.0, 0.0)).hypot();
            dist < 1.0
        });
        assert!(middle_vertex.is_some());
        let middle = middle_vertex.unwrap();
        assert_eq!(middle.end_curves.len(), 1); // End of curve 0
        assert_eq!(middle.start_curves.len(), 1); // Start of curve 1
    }
}
