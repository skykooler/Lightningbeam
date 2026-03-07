//! Flood fill algorithms for paint bucket tool
//!
//! This module contains two fill implementations:
//! - `flood_fill` — vector curve-boundary fill (used by vector paint bucket)
//! - `raster_flood_fill` — pixel BFS fill with configurable threshold, soft
//!   edge, and optional selection clipping (used by raster paint bucket)

// ── Raster flood fill ─────────────────────────────────────────────────────────

/// Which pixel to compare against when deciding if a neighbor should be filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillThresholdMode {
    /// Compare each candidate pixel to the original seed pixel (Photoshop default).
    Absolute,
    /// Compare each candidate pixel to the pixel it was reached from (spreads
    /// through gradients without a global seed-color reference).
    Relative,
}

/// Pixel flood fill for the raster paint bucket tool.
///
/// Operates on a flat RGBA `Vec<u8>` (4 bytes per pixel, row-major).
/// Pixels are alpha-composited with `fill_color` — the existing canvas content
/// shows through wherever `fill_color` has partial transparency or softness
/// reduces its effective alpha.
///
/// # Parameters
/// - `pixels`     – raw RGBA buffer, modified in place
/// - `width/height` – canvas dimensions
/// - `seed_x/y`  – click coordinates (canvas pixel indices, 0-based)
/// - `fill_color` – RGBA fill \[r, g, b, a\], 0–255 each
/// - `threshold`  – max color distance (Euclidean RGBA) to include in fill (0–255·√4 ≈ 510)
/// - `softness`   – how much of the threshold is a soft fade (0 = hard edge, 100 = full fade)
/// - `mode`       – compare candidates to seed (Absolute) or to their BFS parent (Relative)
/// - `selection`  – optional clip mask; pixels outside are never filled
pub fn raster_flood_fill(
    pixels: &mut Vec<u8>,
    width: u32,
    height: u32,
    seed_x: i32,
    seed_y: i32,
    fill_color: [u8; 4],
    threshold: f32,
    softness: f32,
    mode: FillThresholdMode,
    selection: Option<&crate::selection::RasterSelection>,
) {
    use std::collections::VecDeque;

    let w = width as i32;
    let h = height as i32;
    let n = (width * height) as usize;

    if seed_x < 0 || seed_y < 0 || seed_x >= w || seed_y >= h { return; }

    let seed_idx = (seed_y * w + seed_x) as usize;
    let seed_color = [
        pixels[seed_idx * 4],
        pixels[seed_idx * 4 + 1],
        pixels[seed_idx * 4 + 2],
        pixels[seed_idx * 4 + 3],
    ];

    // dist_map[i] = Some(d) when pixel i is within the fill region with
    // color-distance `d` from its comparison color.
    let mut dist_map: Vec<Option<f32>> = vec![None; n];
    let mut queue: VecDeque<(i32, i32)> = VecDeque::new();

    // Track the pixel that each BFS node was reached from (for Relative mode).
    // In Absolute mode this is ignored (we always compare to seed_color).
    let mut parent_color: Vec<[u8; 4]> = vec![[0; 4]; n];

    dist_map[seed_idx] = Some(0.0);
    parent_color[seed_idx] = seed_color;
    queue.push_back((seed_x, seed_y));

    let dirs: [(i32, i32); 4] = [(0, -1), (0, 1), (-1, 0), (1, 0)];

    while let Some((cx, cy)) = queue.pop_front() {
        let ci = (cy * w + cx) as usize;

        let compare_color = match mode {
            FillThresholdMode::Absolute => seed_color,
            FillThresholdMode::Relative => parent_color[ci],
        };

        for (dx, dy) in dirs {
            let nx = cx + dx;
            let ny = cy + dy;
            if nx < 0 || ny < 0 || nx >= w || ny >= h { continue; }
            let ni = (ny * w + nx) as usize;
            if dist_map[ni].is_some() { continue; }
            if let Some(sel) = selection {
                if !sel.contains_pixel(nx, ny) { continue; }
            }
            let npx = [
                pixels[ni * 4],
                pixels[ni * 4 + 1],
                pixels[ni * 4 + 2],
                pixels[ni * 4 + 3],
            ];
            let d = color_distance(npx, compare_color);
            if d <= threshold {
                dist_map[ni] = Some(d);
                parent_color[ni] = npx;
                queue.push_back((nx, ny));
            }
        }
    }

    // Write pass: alpha-composite fill_color over existing pixels.
    let fr = fill_color[0] as f32 / 255.0;
    let fg = fill_color[1] as f32 / 255.0;
    let fb = fill_color[2] as f32 / 255.0;
    let fa_base = fill_color[3] as f32 / 255.0;

    // Softness defines what fraction of [0..threshold] uses full alpha vs fade.
    // falloff_start_ratio: distance ratio at which the fade begins (0→full fade, 1→hard).
    let falloff_start = if softness <= 0.0 || threshold <= 0.0 {
        1.0_f32 // hard edge: never start fading
    } else {
        1.0 - softness / 100.0
    };

    for i in 0..n {
        if let Some(d) = dist_map[i] {
            let alpha = if threshold <= 0.0 {
                fa_base
            } else {
                let t = d / threshold; // 0.0 at seed, 1.0 at boundary
                if t <= falloff_start {
                    fa_base
                } else {
                    let frac = (t - falloff_start) / (1.0 - falloff_start).max(1e-6);
                    fa_base * (1.0 - frac)
                }
            };

            if alpha <= 0.0 { continue; }

            // Porter-Duff "src over dst" with straight alpha.
            let dst_r = pixels[i * 4    ] as f32 / 255.0;
            let dst_g = pixels[i * 4 + 1] as f32 / 255.0;
            let dst_b = pixels[i * 4 + 2] as f32 / 255.0;
            let dst_a = pixels[i * 4 + 3] as f32 / 255.0;

            let inv_a = 1.0 - alpha;
            let out_a = alpha + dst_a * inv_a;
            if out_a > 0.0 {
                pixels[i * 4    ] = ((fr * alpha + dst_r * dst_a * inv_a) / out_a * 255.0).round() as u8;
                pixels[i * 4 + 1] = ((fg * alpha + dst_g * dst_a * inv_a) / out_a * 255.0).round() as u8;
                pixels[i * 4 + 2] = ((fb * alpha + dst_b * dst_a * inv_a) / out_a * 255.0).round() as u8;
                pixels[i * 4 + 3] = (out_a * 255.0).round() as u8;
            }
        }
    }
}

fn color_distance(a: [u8; 4], b: [u8; 4]) -> f32 {
    let dr = a[0] as f32 - b[0] as f32;
    let dg = a[1] as f32 - b[1] as f32;
    let db = a[2] as f32 - b[2] as f32;
    let da = a[3] as f32 - b[3] as f32;
    (dr * dr + dg * dg + db * db + da * da).sqrt()
}

// ── Vector (curve-boundary) flood fill ───────────────────────────────────────
// The following is the original vector-layer flood fill, kept for the vector
// paint bucket tool.

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
