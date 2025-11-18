//! Path fitting algorithms for converting raw points to smooth curves
//!
//! Provides two main algorithms:
//! - Ramer-Douglas-Peucker (RDP) simplification for corner detection
//! - Schneider curve fitting for smooth Bezier curves
//!
//! Based on:
//! - simplify.js by Vladimir Agafonkin
//! - fit-curve by Philip J. Schneider (Graphics Gems, 1990)

use kurbo::{BezPath, Point, Vec2};

/// Configuration for RDP simplification
#[derive(Debug, Clone, Copy)]
pub struct RdpConfig {
    /// Tolerance for simplification (default: 10.0)
    /// Higher values = more simplification (fewer points)
    pub tolerance: f64,
    /// Whether to use highest quality (skip radial distance filter)
    pub highest_quality: bool,
}

impl Default for RdpConfig {
    fn default() -> Self {
        Self {
            tolerance: 10.0,
            highest_quality: false,
        }
    }
}

/// Configuration for Schneider curve fitting
#[derive(Debug, Clone, Copy)]
pub struct SchneiderConfig {
    /// Maximum error tolerance (default: 30.0)
    /// Lower values = more accurate curves (more segments)
    pub max_error: f64,
}

impl Default for SchneiderConfig {
    fn default() -> Self {
        Self { max_error: 30.0 }
    }
}

/// Simplify a polyline using Ramer-Douglas-Peucker algorithm
///
/// This is a two-stage process:
/// 1. Radial distance filter (unless highest_quality is true)
/// 2. Douglas-Peucker recursive simplification
pub fn simplify_rdp(points: &[Point], config: RdpConfig) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }

    let sq_tolerance = config.tolerance * config.tolerance;

    let mut simplified = if config.highest_quality {
        points.to_vec()
    } else {
        simplify_radial_dist(points, sq_tolerance)
    };

    simplified = simplify_douglas_peucker(&simplified, sq_tolerance);

    simplified
}

/// First stage: Remove points that are too close to the previous point
fn simplify_radial_dist(points: &[Point], sq_tolerance: f64) -> Vec<Point> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut result = vec![points[0]];
    let mut prev_point = points[0];

    for &point in &points[1..] {
        if sq_dist(point, prev_point) > sq_tolerance {
            result.push(point);
            prev_point = point;
        }
    }

    // Always include the last point if it's different from the previous one
    if let Some(&last) = points.last() {
        if last != prev_point {
            result.push(last);
        }
    }

    result
}

/// Second stage: Douglas-Peucker recursive simplification
fn simplify_douglas_peucker(points: &[Point], sq_tolerance: f64) -> Vec<Point> {
    if points.len() < 2 {
        return points.to_vec();
    }

    let last = points.len() - 1;
    let mut simplified = vec![points[0]];
    simplify_dp_step(points, 0, last, sq_tolerance, &mut simplified);
    simplified.push(points[last]);

    simplified
}

/// Recursive Douglas-Peucker step
fn simplify_dp_step(
    points: &[Point],
    first: usize,
    last: usize,
    sq_tolerance: f64,
    simplified: &mut Vec<Point>,
) {
    let mut max_sq_dist = sq_tolerance;
    let mut index = 0;

    for i in first + 1..last {
        let sq_dist = sq_seg_dist(points[i], points[first], points[last]);

        if sq_dist > max_sq_dist {
            index = i;
            max_sq_dist = sq_dist;
        }
    }

    if max_sq_dist > sq_tolerance {
        if index - first > 1 {
            simplify_dp_step(points, first, index, sq_tolerance, simplified);
        }
        simplified.push(points[index]);
        if last - index > 1 {
            simplify_dp_step(points, index, last, sq_tolerance, simplified);
        }
    }
}

/// Square distance between two points
#[inline]
fn sq_dist(p1: Point, p2: Point) -> f64 {
    let dx = p1.x - p2.x;
    let dy = p1.y - p2.y;
    dx * dx + dy * dy
}

/// Square distance from a point to a line segment
fn sq_seg_dist(p: Point, p1: Point, p2: Point) -> f64 {
    let mut x = p1.x;
    let mut y = p1.y;
    let dx = p2.x - x;
    let dy = p2.y - y;

    if dx != 0.0 || dy != 0.0 {
        let t = ((p.x - x) * dx + (p.y - y) * dy) / (dx * dx + dy * dy);

        if t > 1.0 {
            x = p2.x;
            y = p2.y;
        } else if t > 0.0 {
            x += dx * t;
            y += dy * t;
        }
    }

    let dx = p.x - x;
    let dy = p.y - y;
    dx * dx + dy * dy
}

/// Fit Bezier curves to a set of points using Schneider's algorithm
///
/// Returns a BezPath containing the fitted cubic Bezier curves
pub fn fit_bezier_curves(points: &[Point], config: SchneiderConfig) -> BezPath {
    if points.len() < 2 {
        return BezPath::new();
    }

    // Remove duplicate points
    let mut unique_points = Vec::new();
    unique_points.push(points[0]);
    for i in 1..points.len() {
        if points[i] != points[i - 1] {
            unique_points.push(points[i]);
        }
    }

    if unique_points.len() < 2 {
        return BezPath::new();
    }

    let len = unique_points.len();
    let left_tangent = create_tangent(unique_points[1], unique_points[0]);
    let right_tangent = create_tangent(unique_points[len - 2], unique_points[len - 1]);

    let curves = fit_cubic(&unique_points, left_tangent, right_tangent, config.max_error);

    // Convert curves to BezPath
    let mut path = BezPath::new();
    if curves.is_empty() {
        return path;
    }

    // Start at the first point
    path.move_to(curves[0][0]);

    // Add all the curves
    for curve in curves {
        path.curve_to(curve[1], curve[2], curve[3]);
    }

    path
}

/// Fit a cubic Bezier curve to a set of points
///
/// Returns an array of Bezier curves, where each curve is [p0, p1, p2, p3]
fn fit_cubic(
    points: &[Point],
    left_tangent: Vec2,
    right_tangent: Vec2,
    error: f64,
) -> Vec<[Point; 4]> {
    const MAX_ITERATIONS: usize = 20;

    // Use heuristic if region only has two points
    if points.len() == 2 {
        let dist = (points[1] - points[0]).hypot() / 3.0;
        let bez_curve = [
            points[0],
            points[0] + left_tangent * dist,
            points[1] + right_tangent * dist,
            points[1],
        ];
        return vec![bez_curve];
    }

    // Parameterize points and attempt to fit curve
    let u = chord_length_parameterize(points);
    let (mut bez_curve, mut max_error, mut split_point) =
        generate_and_report(points, &u, &u, left_tangent, right_tangent);

    if max_error < error {
        return vec![bez_curve];
    }

    // If error not too large, try reparameterization and iteration
    if max_error < error * error {
        let mut u_prime = u.clone();
        let mut prev_err = max_error;
        let mut prev_split = split_point;

        for _ in 0..MAX_ITERATIONS {
            u_prime = reparameterize(&bez_curve, points, &u_prime);

            let result = generate_and_report(points, &u, &u_prime, left_tangent, right_tangent);
            bez_curve = result.0;
            max_error = result.1;
            split_point = result.2;

            if max_error < error {
                return vec![bez_curve];
            }

            // If development grinds to a halt, abort
            if split_point == prev_split {
                let err_change = max_error / prev_err;
                if err_change > 0.9999 && err_change < 1.0001 {
                    break;
                }
            }

            prev_err = max_error;
            prev_split = split_point;
        }
    }

    // Fitting failed -- split at max error point and fit recursively
    let mut beziers = Vec::new();

    // Calculate tangent at split point
    let mut center_vector = points[split_point - 1] - points[split_point + 1];

    // Handle case where points are the same
    if center_vector.hypot() == 0.0 {
        center_vector = points[split_point - 1] - points[split_point];
        center_vector = Vec2::new(-center_vector.y, center_vector.x);
    }

    let to_center_tangent = normalize(center_vector);
    let from_center_tangent = -to_center_tangent;

    // Recursively fit curves
    beziers.extend(fit_cubic(
        &points[0..=split_point],
        left_tangent,
        to_center_tangent,
        error,
    ));
    beziers.extend(fit_cubic(
        &points[split_point..],
        from_center_tangent,
        right_tangent,
        error,
    ));

    beziers
}

/// Generate a Bezier curve and compute its error
fn generate_and_report(
    points: &[Point],
    params_orig: &[f64],
    params_prime: &[f64],
    left_tangent: Vec2,
    right_tangent: Vec2,
) -> ([Point; 4], f64, usize) {
    let bez_curve = generate_bezier(points, params_prime, left_tangent, right_tangent);
    let (max_error, split_point) = compute_max_error(points, &bez_curve, params_orig);

    (bez_curve, max_error, split_point)
}

/// Use least-squares method to find Bezier control points
fn generate_bezier(
    points: &[Point],
    parameters: &[f64],
    left_tangent: Vec2,
    right_tangent: Vec2,
) -> [Point; 4] {
    let first_point = points[0];
    let last_point = points[points.len() - 1];

    // Compute the A matrix
    let mut a = Vec::new();
    for &u in parameters {
        let ux = 1.0 - u;
        let a0 = left_tangent * (3.0 * u * ux * ux);
        let a1 = right_tangent * (3.0 * ux * u * u);
        a.push([a0, a1]);
    }

    // Create C and X matrices
    let mut c = [[0.0, 0.0], [0.0, 0.0]];
    let mut x = [0.0, 0.0];

    for i in 0..points.len() {
        let u = parameters[i];
        let ai = a[i];

        c[0][0] += dot(ai[0], ai[0]);
        c[0][1] += dot(ai[0], ai[1]);
        c[1][0] += dot(ai[0], ai[1]);
        c[1][1] += dot(ai[1], ai[1]);

        let tmp = points[i] - bezier_q(&[first_point, first_point, last_point, last_point], u);

        x[0] += dot(ai[0], tmp);
        x[1] += dot(ai[1], tmp);
    }

    // Compute determinants
    let det_c0_c1 = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det_c0_x = c[0][0] * x[1] - c[1][0] * x[0];
    let det_x_c1 = x[0] * c[1][1] - x[1] * c[0][1];

    // Derive alpha values
    let alpha_l = if det_c0_c1 == 0.0 {
        0.0
    } else {
        det_x_c1 / det_c0_c1
    };
    let alpha_r = if det_c0_c1 == 0.0 {
        0.0
    } else {
        det_c0_x / det_c0_c1
    };

    // If alpha is negative or too small, use heuristic
    let seg_length = (last_point - first_point).hypot();
    let epsilon = 1.0e-6 * seg_length;

    let (p1, p2) = if alpha_l < epsilon || alpha_r < epsilon {
        // Fall back on standard formula
        (
            first_point + left_tangent * (seg_length / 3.0),
            last_point + right_tangent * (seg_length / 3.0),
        )
    } else {
        (
            first_point + left_tangent * alpha_l,
            last_point + right_tangent * alpha_r,
        )
    };

    [first_point, p1, p2, last_point]
}

/// Reparameterize points using Newton-Raphson
fn reparameterize(bezier: &[Point; 4], points: &[Point], parameters: &[f64]) -> Vec<f64> {
    parameters
        .iter()
        .zip(points.iter())
        .map(|(&p, &point)| newton_raphson_root_find(bezier, point, p))
        .collect()
}

/// Use Newton-Raphson iteration to find better root
fn newton_raphson_root_find(bez: &[Point; 4], point: Point, u: f64) -> f64 {
    let d = bezier_q(bez, u) - point;
    let qprime = bezier_qprime(bez, u);
    let numerator = dot(d, qprime);
    let qprimeprime = bezier_qprimeprime(bez, u);
    let denominator = dot(qprime, qprime) + 2.0 * dot(d, qprimeprime);

    if denominator == 0.0 {
        u
    } else {
        u - numerator / denominator
    }
}

/// Assign parameter values using chord length
fn chord_length_parameterize(points: &[Point]) -> Vec<f64> {
    let mut u = Vec::new();
    let mut curr_u = 0.0;

    u.push(0.0);

    for i in 1..points.len() {
        curr_u += (points[i] - points[i - 1]).hypot();
        u.push(curr_u);
    }

    let total_length = u[u.len() - 1];
    u.iter().map(|&x| x / total_length).collect()
}

/// Find maximum squared distance of points to fitted curve
fn compute_max_error(points: &[Point], bez: &[Point; 4], parameters: &[f64]) -> (f64, usize) {
    let mut max_dist = 0.0;
    let mut split_point = points.len() / 2;

    let t_dist_map = map_t_to_relative_distances(bez, 10);

    for i in 0..points.len() {
        let point = points[i];
        let t = find_t(bez, parameters[i], &t_dist_map, 10);

        let v = bezier_q(bez, t) - point;
        let dist = v.x * v.x + v.y * v.y;

        if dist > max_dist {
            max_dist = dist;
            split_point = i;
        }
    }

    (max_dist, split_point)
}

/// Sample t values and map to relative distances along curve
fn map_t_to_relative_distances(bez: &[Point; 4], b_parts: usize) -> Vec<f64> {
    let mut b_t_dist = vec![0.0];
    let mut b_t_prev = bez[0];
    let mut sum_len = 0.0;

    for i in 1..=b_parts {
        let b_t_curr = bezier_q(bez, i as f64 / b_parts as f64);
        sum_len += (b_t_curr - b_t_prev).hypot();
        b_t_dist.push(sum_len);
        b_t_prev = b_t_curr;
    }

    // Normalize to 0..1
    b_t_dist.iter().map(|&x| x / sum_len).collect()
}

/// Find t value for a given parameter distance
fn find_t(bez: &[Point; 4], param: f64, t_dist_map: &[f64], b_parts: usize) -> f64 {
    if param < 0.0 {
        return 0.0;
    }
    if param > 1.0 {
        return 1.0;
    }

    for i in 1..=b_parts {
        if param <= t_dist_map[i] {
            let t_min = (i - 1) as f64 / b_parts as f64;
            let t_max = i as f64 / b_parts as f64;
            let len_min = t_dist_map[i - 1];
            let len_max = t_dist_map[i];

            let t = (param - len_min) / (len_max - len_min) * (t_max - t_min) + t_min;
            return t;
        }
    }

    1.0
}

/// Evaluate cubic Bezier at parameter t
fn bezier_q(ctrl_poly: &[Point; 4], t: f64) -> Point {
    let tx = 1.0 - t;
    let p_a = ctrl_poly[0].to_vec2() * (tx * tx * tx);
    let p_b = ctrl_poly[1].to_vec2() * (3.0 * tx * tx * t);
    let p_c = ctrl_poly[2].to_vec2() * (3.0 * tx * t * t);
    let p_d = ctrl_poly[3].to_vec2() * (t * t * t);

    (p_a + p_b + p_c + p_d).to_point()
}

/// Evaluate first derivative of cubic Bezier at parameter t
fn bezier_qprime(ctrl_poly: &[Point; 4], t: f64) -> Vec2 {
    let tx = 1.0 - t;
    let p_a = (ctrl_poly[1] - ctrl_poly[0]) * (3.0 * tx * tx);
    let p_b = (ctrl_poly[2] - ctrl_poly[1]) * (6.0 * tx * t);
    let p_c = (ctrl_poly[3] - ctrl_poly[2]) * (3.0 * t * t);

    p_a + p_b + p_c
}

/// Evaluate second derivative of cubic Bezier at parameter t
fn bezier_qprimeprime(ctrl_poly: &[Point; 4], t: f64) -> Vec2 {
    let v0 = ctrl_poly[2].to_vec2() - ctrl_poly[1].to_vec2() * 2.0 + ctrl_poly[0].to_vec2();
    let v1 = ctrl_poly[3].to_vec2() - ctrl_poly[2].to_vec2() * 2.0 + ctrl_poly[1].to_vec2();
    v0 * (6.0 * (1.0 - t)) + v1 * (6.0 * t)
}

/// Create a unit tangent vector from A to B
fn create_tangent(point_a: Point, point_b: Point) -> Vec2 {
    normalize(point_a - point_b)
}

/// Normalize a vector to unit length
fn normalize(v: Vec2) -> Vec2 {
    let len = v.hypot();
    if len == 0.0 {
        Vec2::ZERO
    } else {
        v / len
    }
}

/// Dot product of two vectors
fn dot(v1: Vec2, v2: Vec2) -> f64 {
    v1.x * v2.x + v1.y * v2.y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rdp_simplification() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.1),
            Point::new(2.0, 0.0),
            Point::new(3.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(5.0, 0.0),
        ];

        let config = RdpConfig {
            tolerance: 0.5,
            highest_quality: false,
        };

        let simplified = simplify_rdp(&points, config);

        // Should simplify the nearly-straight line
        assert!(simplified.len() < points.len());
        assert_eq!(simplified[0], points[0]);
        assert_eq!(simplified[simplified.len() - 1], points[points.len() - 1]);
    }

    #[test]
    fn test_schneider_curve_fitting() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(50.0, 100.0),
            Point::new(100.0, 50.0),
            Point::new(150.0, 100.0),
        ];

        let config = SchneiderConfig { max_error: 30.0 };

        let path = fit_bezier_curves(&points, config);

        // Should create a valid BezPath
        assert!(!path.is_empty());
    }

    #[test]
    fn test_chord_length_parameterization() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(2.0, 0.0),
        ];

        let params = chord_length_parameterize(&points);

        // Should start at 0 and end at 1
        assert_eq!(params[0], 0.0);
        assert_eq!(params[params.len() - 1], 1.0);
        // Should be evenly spaced for uniform spacing
        assert!((params[1] - 0.5).abs() < 0.01);
    }
}
