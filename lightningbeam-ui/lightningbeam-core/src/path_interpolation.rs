//! Path interpolation using the d3-interpolate-path algorithm
//!
//! This module implements path morphing by normalizing two paths to have
//! the same number of segments and then interpolating between them.
//!
//! Based on: https://github.com/pbeshai/d3-interpolate-path

use kurbo::{BezPath, PathEl, Point};

/// de Casteljau's algorithm for splitting bezier curves
///
/// Takes a list of control points and a parameter t, and returns
/// the two curves (left and right) that result from splitting at t.
fn decasteljau(points: &[Point], t: f64) -> (Vec<Point>, Vec<Point>) {
    let mut left = Vec::new();
    let mut right = Vec::new();

    fn recurse(points: &[Point], t: f64, left: &mut Vec<Point>, right: &mut Vec<Point>) {
        if points.len() == 1 {
            left.push(points[0]);
            right.push(points[0]);
        } else {
            let mut new_points = Vec::with_capacity(points.len() - 1);

            for i in 0..points.len() - 1 {
                if i == 0 {
                    left.push(points[0]);
                }
                if i == points.len() - 2 {
                    right.push(points[i + 1]);
                }

                // Linear interpolation between consecutive points
                let x = (1.0 - t) * points[i].x + t * points[i + 1].x;
                let y = (1.0 - t) * points[i].y + t * points[i + 1].y;
                new_points.push(Point::new(x, y));
            }

            recurse(&new_points, t, left, right);
        }
    }

    if !points.is_empty() {
        recurse(points, t, &mut left, &mut right);
        right.reverse();
    }

    (left, right)
}

/// A simplified path command representation for interpolation
#[derive(Clone, Debug)]
enum PathCommand {
    MoveTo { x: f64, y: f64 },
    LineTo { x: f64, y: f64 },
    QuadTo { x1: f64, y1: f64, x: f64, y: f64 },
    CurveTo { x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64 },
    Close,
}

impl PathCommand {
    /// Get the end point of this command
    fn end_point(&self) -> Point {
        match self {
            PathCommand::MoveTo { x, y }
            | PathCommand::LineTo { x, y }
            | PathCommand::QuadTo { x, y, .. }
            | PathCommand::CurveTo { x, y, .. } => Point::new(*x, *y),
            PathCommand::Close => Point::new(0.0, 0.0), // Will be handled specially
        }
    }

    /// Get all control points for this command (from start point)
    fn to_points(&self, start: Point) -> Vec<Point> {
        match self {
            PathCommand::LineTo { x, y } => {
                vec![start, Point::new(*x, *y)]
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                vec![start, Point::new(*x1, *y1), Point::new(*x, *y)]
            }
            PathCommand::CurveTo { x1, y1, x2, y2, x, y } => {
                vec![
                    start,
                    Point::new(*x1, *y1),
                    Point::new(*x2, *y2),
                    Point::new(*x, *y),
                ]
            }
            _ => vec![start],
        }
    }

    /// Convert command type to match another command
    fn convert_to_type(&self, target: &PathCommand) -> PathCommand {
        match target {
            PathCommand::CurveTo { .. } => {
                // Convert to cubic curve
                let end = self.end_point();
                match self {
                    PathCommand::LineTo { .. } | PathCommand::MoveTo { .. } => {
                        PathCommand::CurveTo {
                            x1: end.x,
                            y1: end.y,
                            x2: end.x,
                            y2: end.y,
                            x: end.x,
                            y: end.y,
                        }
                    }
                    PathCommand::QuadTo { x1, y1, x, y } => {
                        // Convert quadratic to cubic
                        PathCommand::CurveTo {
                            x1: *x1,
                            y1: *y1,
                            x2: *x1,
                            y2: *y1,
                            x: *x,
                            y: *y,
                        }
                    }
                    PathCommand::CurveTo { .. } => self.clone(),
                    PathCommand::Close => self.clone(),
                }
            }
            PathCommand::QuadTo { .. } => {
                // Convert to quadratic curve
                let end = self.end_point();
                match self {
                    PathCommand::LineTo { .. } | PathCommand::MoveTo { .. } => {
                        PathCommand::QuadTo {
                            x1: end.x,
                            y1: end.y,
                            x: end.x,
                            y: end.y,
                        }
                    }
                    PathCommand::QuadTo { .. } => self.clone(),
                    PathCommand::CurveTo { x1, y1, x, y, .. } => {
                        // Use first control point for quad
                        PathCommand::QuadTo {
                            x1: *x1,
                            y1: *y1,
                            x: *x,
                            y: *y,
                        }
                    }
                    PathCommand::Close => self.clone(),
                }
            }
            PathCommand::LineTo { .. } => {
                let end = self.end_point();
                PathCommand::LineTo { x: end.x, y: end.y }
            }
            _ => self.clone(),
        }
    }
}

/// Convert points back to a command
fn points_to_command(points: &[Point]) -> PathCommand {
    match points.len() {
        2 => PathCommand::LineTo {
            x: points[1].x,
            y: points[1].y,
        },
        3 => PathCommand::QuadTo {
            x1: points[1].x,
            y1: points[1].y,
            x: points[2].x,
            y: points[2].y,
        },
        4 => PathCommand::CurveTo {
            x1: points[1].x,
            y1: points[1].y,
            x2: points[2].x,
            y2: points[2].y,
            x: points[3].x,
            y: points[3].y,
        },
        _ => PathCommand::LineTo {
            x: points.last().map(|p| p.x).unwrap_or(0.0),
            y: points.last().map(|p| p.y).unwrap_or(0.0),
        },
    }
}

/// Split a curve segment into multiple segments using de Casteljau
fn split_segment(start: Point, command: &PathCommand, count: usize) -> Vec<PathCommand> {
    if count == 0 {
        return vec![];
    }
    if count == 1 {
        return vec![command.clone()];
    }

    // For splittable curves (L, Q, C), use de Casteljau
    match command {
        PathCommand::LineTo { .. }
        | PathCommand::QuadTo { .. }
        | PathCommand::CurveTo { .. } => {
            let points = command.to_points(start);
            split_curve_as_points(&points, count)
                .into_iter()
                .map(|pts| points_to_command(&pts))
                .collect()
        }
        _ => {
            // For other commands, just repeat
            vec![command.clone(); count]
        }
    }
}

/// Split a curve (represented as points) into segment_count segments
fn split_curve_as_points(points: &[Point], segment_count: usize) -> Vec<Vec<Point>> {
    let mut segments = Vec::new();
    let mut remaining_curve = points.to_vec();
    let t_increment = 1.0 / segment_count as f64;

    for i in 0..segment_count - 1 {
        let t_relative = t_increment / (1.0 - t_increment * i as f64);
        let (left, right) = decasteljau(&remaining_curve, t_relative);
        segments.push(left);
        remaining_curve = right;
    }

    segments.push(remaining_curve);
    segments
}

/// Extend a path to match the length of a reference path
fn extend_commands(
    commands_to_extend: &[PathCommand],
    reference_commands: &[PathCommand],
) -> Vec<PathCommand> {
    if commands_to_extend.is_empty() || reference_commands.is_empty() {
        return commands_to_extend.to_vec();
    }

    let num_segments_to_extend = commands_to_extend.len() - 1;
    let num_reference_segments = reference_commands.len() - 1;

    if num_reference_segments == 0 {
        return commands_to_extend.to_vec();
    }

    let segment_ratio = num_segments_to_extend as f64 / num_reference_segments as f64;

    // Count how many points should be in each segment
    let mut count_per_segment = vec![0; num_segments_to_extend];
    for i in 0..num_reference_segments {
        let insert_index = ((segment_ratio * i as f64).floor() as usize)
            .min(num_segments_to_extend.saturating_sub(1));
        count_per_segment[insert_index] += 1;
    }

    // Start with first command
    let mut extended = vec![commands_to_extend[0].clone()];
    let mut current_point = commands_to_extend[0].end_point();

    // Extend each segment
    for (i, &count) in count_per_segment.iter().enumerate() {
        if i >= commands_to_extend.len() - 1 {
            // Handle last command
            for _ in 0..count {
                extended.push(commands_to_extend[commands_to_extend.len() - 1].clone());
            }
        } else {
            // Split this segment
            let split_commands =
                split_segment(current_point, &commands_to_extend[i + 1], count.max(1));
            extended.extend(split_commands);
            current_point = commands_to_extend[i + 1].end_point();
        }
    }

    extended
}

/// Convert a BezPath to our internal command representation
fn bezpath_to_commands(path: &BezPath) -> Vec<PathCommand> {
    let mut commands = Vec::new();

    for el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                commands.push(PathCommand::MoveTo { x: p.x, y: p.y });
            }
            PathEl::LineTo(p) => {
                commands.push(PathCommand::LineTo { x: p.x, y: p.y });
            }
            PathEl::QuadTo(p1, p2) => {
                commands.push(PathCommand::QuadTo {
                    x1: p1.x,
                    y1: p1.y,
                    x: p2.x,
                    y: p2.y,
                });
            }
            PathEl::CurveTo(p1, p2, p3) => {
                commands.push(PathCommand::CurveTo {
                    x1: p1.x,
                    y1: p1.y,
                    x2: p2.x,
                    y2: p2.y,
                    x: p3.x,
                    y: p3.y,
                });
            }
            PathEl::ClosePath => {
                commands.push(PathCommand::Close);
            }
        }
    }

    commands
}

/// Convert our internal commands back to a BezPath
fn commands_to_bezpath(commands: &[PathCommand]) -> BezPath {
    let mut path = BezPath::new();

    for cmd in commands {
        match cmd {
            PathCommand::MoveTo { x, y } => {
                path.move_to((*x, *y));
            }
            PathCommand::LineTo { x, y } => {
                path.line_to((*x, *y));
            }
            PathCommand::QuadTo { x1, y1, x, y } => {
                path.quad_to((*x1, *y1), (*x, *y));
            }
            PathCommand::CurveTo { x1, y1, x2, y2, x, y } => {
                path.curve_to((*x1, *y1), (*x2, *y2), (*x, *y));
            }
            PathCommand::Close => {
                path.close_path();
            }
        }
    }

    path
}

/// Interpolate between two paths at parameter t (0.0 to 1.0)
///
/// Uses the d3-interpolate-path algorithm:
/// 1. Normalize paths to same length by splitting segments
/// 2. Convert commands to matching types
/// 3. Linearly interpolate all parameters
pub fn interpolate_paths(path_a: &BezPath, path_b: &BezPath, t: f64) -> BezPath {
    let mut commands_a = bezpath_to_commands(path_a);
    let mut commands_b = bezpath_to_commands(path_b);

    // Handle Z (close path) - remove temporarily, add back if both have it
    let add_z = commands_a.last().map_or(false, |c| matches!(c, PathCommand::Close))
        && commands_b.last().map_or(false, |c| matches!(c, PathCommand::Close));

    if commands_a.last().map_or(false, |c| matches!(c, PathCommand::Close)) {
        commands_a.pop();
    }
    if commands_b.last().map_or(false, |c| matches!(c, PathCommand::Close)) {
        commands_b.pop();
    }

    // Handle empty paths
    if commands_a.is_empty() && !commands_b.is_empty() {
        commands_a.push(commands_b[0].clone());
    } else if commands_b.is_empty() && !commands_a.is_empty() {
        commands_b.push(commands_a[0].clone());
    } else if commands_a.is_empty() && commands_b.is_empty() {
        return BezPath::new();
    }

    // Extend paths to match length
    if commands_a.len() < commands_b.len() {
        commands_a = extend_commands(&commands_a, &commands_b);
    } else if commands_b.len() < commands_a.len() {
        commands_b = extend_commands(&commands_b, &commands_a);
    }

    // Convert A commands to match B types
    commands_a = commands_a
        .iter()
        .zip(commands_b.iter())
        .map(|(a, b)| a.convert_to_type(b))
        .collect();

    // Interpolate
    let mut interpolated = Vec::new();
    for (cmd_a, cmd_b) in commands_a.iter().zip(commands_b.iter()) {
        let interpolated_cmd = match (cmd_a, cmd_b) {
            (PathCommand::MoveTo { x: x1, y: y1 }, PathCommand::MoveTo { x: x2, y: y2 }) => {
                PathCommand::MoveTo {
                    x: x1 + t * (x2 - x1),
                    y: y1 + t * (y2 - y1),
                }
            }
            (PathCommand::LineTo { x: x1, y: y1 }, PathCommand::LineTo { x: x2, y: y2 }) => {
                PathCommand::LineTo {
                    x: x1 + t * (x2 - x1),
                    y: y1 + t * (y2 - y1),
                }
            }
            (
                PathCommand::QuadTo { x1: xa1, y1: ya1, x: x1, y: y1 },
                PathCommand::QuadTo { x1: xa2, y1: ya2, x: x2, y: y2 },
            ) => PathCommand::QuadTo {
                x1: xa1 + t * (xa2 - xa1),
                y1: ya1 + t * (ya2 - ya1),
                x: x1 + t * (x2 - x1),
                y: y1 + t * (y2 - y1),
            },
            (
                PathCommand::CurveTo { x1: xa1, y1: ya1, x2: xb1, y2: yb1, x: x1, y: y1 },
                PathCommand::CurveTo { x1: xa2, y1: ya2, x2: xb2, y2: yb2, x: x2, y: y2 },
            ) => PathCommand::CurveTo {
                x1: xa1 + t * (xa2 - xa1),
                y1: ya1 + t * (ya2 - ya1),
                x2: xb1 + t * (xb2 - xb1),
                y2: yb1 + t * (yb2 - yb1),
                x: x1 + t * (x2 - x1),
                y: y1 + t * (y2 - y1),
            },
            (PathCommand::Close, PathCommand::Close) => PathCommand::Close,
            _ => cmd_a.clone(), // Fallback
        };
        interpolated.push(interpolated_cmd);
    }

    if add_z {
        interpolated.push(PathCommand::Close);
    }

    commands_to_bezpath(&interpolated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Circle, Shape};

    #[test]
    fn test_decasteljau() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(50.0, 0.0),
            Point::new(50.0, 50.0),
            Point::new(100.0, 50.0),
        ];

        let (left, right) = decasteljau(&points, 0.5);
        assert_eq!(left.len(), 4);
        assert_eq!(right.len(), 4);
    }

    #[test]
    fn test_interpolate_circles() {
        let circle1 = Circle::new((100.0, 100.0), 50.0);
        let circle2 = Circle::new((100.0, 100.0), 100.0);

        let path1 = circle1.to_path(0.1);
        let path2 = circle2.to_path(0.1);

        let interpolated = interpolate_paths(&path1, &path2, 0.5);
        assert!(!interpolated.elements().is_empty());
    }
}
