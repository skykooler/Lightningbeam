//! Flood fill algorithm for paint bucket tool
//!
//! This module implements a flood fill that tracks which curves each point
//! touches. Instead of filling with pixels, it returns boundary points that
//! can be used to construct a filled shape from exact curve geometry.

use crate::curve_segment::CurveSegment;
use crate::quadtree::{BoundingBox, Quadtree};
use std::collections::{HashSet, VecDeque};
use vello::kurbo::Point;

/// A point on the boundary of the filled region
#[derive(Debug, Clone)]
pub struct BoundaryPoint {
    /// The sampled point location
    pub point: Point,
    /// Index of the nearest curve segment
    pub curve_index: usize,
    /// Parameter t on the nearest curve (0.0 to 1.0)
    pub t: f64,
    /// Nearest point on the curve
    pub nearest_point: Point,
    /// Distance to the nearest curve
    pub distance: f64,
}

/// Result of a flood fill operation
#[derive(Debug)]
pub struct FloodFillResult {
    /// All boundary points found during flood fill
    pub boundary_points: Vec<BoundaryPoint>,
    /// All interior points that were filled
    pub interior_points: Vec<Point>,
}

/// Flood fill configuration
pub struct FloodFillConfig {
    /// Distance threshold - points closer than this to a curve are boundary points
    pub epsilon: f64,
    /// Step size for sampling (distance between sampled points)
    pub step_size: f64,
    /// Maximum number of points to sample (prevents infinite loops)
    pub max_points: usize,
    /// Bounding box to constrain the fill
    pub bounds: Option<BoundingBox>,
}

impl Default for FloodFillConfig {
    fn default() -> Self {
        Self {
            epsilon: 2.0,
            step_size: 5.0,
            max_points: 10000,
            bounds: None,
        }
    }
}

/// Perform flood fill starting from a point
///
/// This function expands outward from the start point, stopping when it
/// encounters curves (within epsilon distance). It returns all boundary
/// points along with information about which curve each point is near.
///
/// # Parameters
/// - `start`: Starting point for the flood fill
/// - `curves`: All curve segments in the scene
/// - `quadtree`: Spatial index for efficient curve queries
/// - `config`: Flood fill configuration
///
/// # Returns
/// FloodFillResult with boundary and interior points
pub fn flood_fill(
    start: Point,
    curves: &[CurveSegment],
    quadtree: &Quadtree,
    config: &FloodFillConfig,
) -> FloodFillResult {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    let mut boundary_points = Vec::new();
    let mut interior_points = Vec::new();

    // Quantize start point to grid
    let start_grid = point_to_grid(start, config.step_size);
    queue.push_back(start_grid);
    visited.insert(start_grid);

    while let Some(grid_point) = queue.pop_front() {
        // Check max points limit
        if visited.len() >= config.max_points {
            break;
        }

        // Convert grid point back to actual coordinates
        let point = grid_to_point(grid_point, config.step_size);

        // Check bounds if specified
        if let Some(ref bounds) = config.bounds {
            if !bounds.contains_point(point) {
                continue;
            }
        }

        // Query quadtree for nearby curves
        let query_bbox = BoundingBox::around_point(point, config.epsilon * 2.0);
        let nearby_curve_indices = quadtree.query(&query_bbox);

        // Find the nearest curve
        let nearest = find_nearest_curve(point, curves, &nearby_curve_indices);

        if let Some((curve_idx, t, nearest_point, distance)) = nearest {
            // If we're within epsilon, this is a boundary point
            if distance < config.epsilon {
                boundary_points.push(BoundaryPoint {
                    point,
                    curve_index: curve_idx,
                    t,
                    nearest_point,
                    distance,
                });
                continue; // Don't expand from boundary points
            }
        }

        // This is an interior point - add to interior and expand
        interior_points.push(point);

        // Add neighbors to queue (4-directional)
        let neighbors = [
            (grid_point.0 + 1, grid_point.1),     // Right
            (grid_point.0 - 1, grid_point.1),     // Left
            (grid_point.0, grid_point.1 + 1),     // Down
            (grid_point.0, grid_point.1 - 1),     // Up
            (grid_point.0 + 1, grid_point.1 + 1), // Diagonal: down-right
            (grid_point.0 + 1, grid_point.1 - 1), // Diagonal: up-right
            (grid_point.0 - 1, grid_point.1 + 1), // Diagonal: down-left
            (grid_point.0 - 1, grid_point.1 - 1), // Diagonal: up-left
        ];

        for neighbor in neighbors {
            if !visited.contains(&neighbor) {
                visited.insert(neighbor);
                queue.push_back(neighbor);
            }
        }
    }

    FloodFillResult {
        boundary_points,
        interior_points,
    }
}

/// Convert a point to grid coordinates
fn point_to_grid(point: Point, step_size: f64) -> (i32, i32) {
    let x = (point.x / step_size).round() as i32;
    let y = (point.y / step_size).round() as i32;
    (x, y)
}

/// Convert grid coordinates back to a point
fn grid_to_point(grid: (i32, i32), step_size: f64) -> Point {
    Point::new(grid.0 as f64 * step_size, grid.1 as f64 * step_size)
}

/// Find the nearest curve to a point from a set of candidate curves
///
/// Returns (curve_index, parameter_t, nearest_point, distance)
fn find_nearest_curve(
    point: Point,
    all_curves: &[CurveSegment],
    candidate_indices: &[usize],
) -> Option<(usize, f64, Point, f64)> {
    let mut best: Option<(usize, f64, Point, f64)> = None;

    for &curve_idx in candidate_indices {
        if curve_idx >= all_curves.len() {
            continue;
        }

        let curve = &all_curves[curve_idx];
        let (t, nearest_point, dist_sq) = curve.nearest_point(point);
        let distance = dist_sq.sqrt();

        match best {
            None => {
                best = Some((curve_idx, t, nearest_point, distance));
            }
            Some((_, _, _, best_dist)) if distance < best_dist => {
                best = Some((curve_idx, t, nearest_point, distance));
            }
            _ => {}
        }
    }

    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve_segment::{CurveSegment, CurveType};

    #[test]
    fn test_point_to_grid_conversion() {
        let point = Point::new(10.0, 20.0);
        let step_size = 5.0;

        let grid = point_to_grid(point, step_size);
        assert_eq!(grid, (2, 4));

        let back = grid_to_point(grid, step_size);
        assert!((back.x - 10.0).abs() < 0.1);
        assert!((back.y - 20.0).abs() < 0.1);
    }

    #[test]
    fn test_find_nearest_curve() {
        let curves = vec![
            CurveSegment::new(
                0,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
            ),
            CurveSegment::new(
                1,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 50.0), Point::new(100.0, 50.0)],
            ),
        ];

        let point = Point::new(50.0, 10.0);
        let candidates = vec![0, 1];

        let result = find_nearest_curve(point, &curves, &candidates);
        assert!(result.is_some());

        let (curve_idx, _t, _nearest, distance) = result.unwrap();
        assert_eq!(curve_idx, 0); // Should be nearest to first curve
        assert!((distance - 10.0).abs() < 1.0);
    }

    #[test]
    fn test_flood_fill_simple_box() {
        // Create a simple box with 4 lines
        let curves = vec![
            // Bottom
            CurveSegment::new(
                0,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
            ),
            // Right
            CurveSegment::new(
                1,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(100.0, 0.0), Point::new(100.0, 100.0)],
            ),
            // Top
            CurveSegment::new(
                2,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(100.0, 100.0), Point::new(0.0, 100.0)],
            ),
            // Left
            CurveSegment::new(
                3,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 100.0), Point::new(0.0, 0.0)],
            ),
        ];

        // Build quadtree
        let mut quadtree = Quadtree::new(BoundingBox::new(-10.0, 110.0, -10.0, 110.0), 4);
        for (i, curve) in curves.iter().enumerate() {
            let bbox = curve.bounding_box();
            quadtree.insert(&bbox, i);
        }

        // Fill from center
        let config = FloodFillConfig {
            epsilon: 2.0,
            step_size: 5.0,
            max_points: 10000,
            bounds: Some(BoundingBox::new(-10.0, 110.0, -10.0, 110.0)),
        };

        let result = flood_fill(Point::new(50.0, 50.0), &curves, &quadtree, &config);

        // Should have boundary points
        assert!(!result.boundary_points.is_empty());
        // Should have interior points
        assert!(!result.interior_points.is_empty());

        // All boundary points should be within epsilon of a curve
        for bp in &result.boundary_points {
            assert!(bp.distance < config.epsilon);
        }
    }

    #[test]
    fn test_flood_fill_respects_bounds() {
        let curves = vec![CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
        )];

        let mut quadtree = Quadtree::new(BoundingBox::new(-10.0, 110.0, -10.0, 110.0), 4);
        for (i, curve) in curves.iter().enumerate() {
            let bbox = curve.bounding_box();
            quadtree.insert(&bbox, i);
        }

        let config = FloodFillConfig {
            epsilon: 2.0,
            step_size: 5.0,
            max_points: 1000,
            bounds: Some(BoundingBox::new(0.0, 50.0, 0.0, 50.0)),
        };

        let result = flood_fill(Point::new(25.0, 25.0), &curves, &quadtree, &config);

        // All points should be within bounds
        for point in &result.interior_points {
            assert!(point.x >= 0.0 && point.x <= 50.0);
            assert!(point.y >= 0.0 && point.y <= 50.0);
        }
    }
}
