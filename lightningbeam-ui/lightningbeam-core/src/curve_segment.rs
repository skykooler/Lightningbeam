//! Curve segment representation for paint bucket fill algorithm
//!
//! This module provides types for representing segments of Bezier curves
//! with parameter ranges. These segments are used to build filled paths
//! from the exact geometry of curves that bound a filled region.

use vello::kurbo::{
    CubicBez, Line, ParamCurve, ParamCurveNearest, PathEl, Point, QuadBez, Shape,
};

/// Type of Bezier curve segment
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CurveType {
    /// Straight line segment
    Line,
    /// Quadratic Bezier curve
    Quadratic,
    /// Cubic Bezier curve
    Cubic,
}

/// A segment of a Bezier curve with parameter range
///
/// Represents a portion of a curve from parameter t_start to t_end.
/// The curve is identified by its index in a document's path list,
/// and the segment within that path.
#[derive(Debug, Clone)]
pub struct CurveSegment {
    /// Index of the shape/path in the document
    pub shape_index: usize,
    /// Index of the segment within the path
    pub segment_index: usize,
    /// Type of curve
    pub curve_type: CurveType,
    /// Start parameter (0.0 to 1.0)
    pub t_start: f64,
    /// End parameter (0.0 to 1.0)
    pub t_end: f64,
    /// Cached control points for this segment
    pub control_points: Vec<Point>,
}

impl CurveSegment {
    /// Create a new curve segment
    pub fn new(
        shape_index: usize,
        segment_index: usize,
        curve_type: CurveType,
        t_start: f64,
        t_end: f64,
        control_points: Vec<Point>,
    ) -> Self {
        Self {
            shape_index,
            segment_index,
            curve_type,
            t_start,
            t_end,
            control_points,
        }
    }

    /// Create a curve segment from a full curve (t_start=0, t_end=1)
    pub fn from_path_element(
        shape_index: usize,
        segment_index: usize,
        element: &PathEl,
        start_point: Point,
    ) -> Option<Self> {
        match element {
            PathEl::LineTo(p) => Some(Self::new(
                shape_index,
                segment_index,
                CurveType::Line,
                0.0,
                1.0,
                vec![start_point, *p],
            )),
            PathEl::QuadTo(p1, p2) => Some(Self::new(
                shape_index,
                segment_index,
                CurveType::Quadratic,
                0.0,
                1.0,
                vec![start_point, *p1, *p2],
            )),
            PathEl::CurveTo(p1, p2, p3) => Some(Self::new(
                shape_index,
                segment_index,
                CurveType::Cubic,
                0.0,
                1.0,
                vec![start_point, *p1, *p2, *p3],
            )),
            PathEl::MoveTo(_) | PathEl::ClosePath => None,
        }
    }

    /// Evaluate the curve at parameter t (in segment's local [t_start, t_end] range)
    pub fn eval_at(&self, t: f64) -> Point {
        // Map t from segment range to curve range
        let curve_t = self.t_start + t * (self.t_end - self.t_start);

        match self.curve_type {
            CurveType::Line => {
                let line = Line::new(self.control_points[0], self.control_points[1]);
                line.eval(curve_t)
            }
            CurveType::Quadratic => {
                let quad = QuadBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                );
                quad.eval(curve_t)
            }
            CurveType::Cubic => {
                let cubic = CubicBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                    self.control_points[3],
                );
                cubic.eval(curve_t)
            }
        }
    }

    /// Get the start point of this segment
    pub fn start_point(&self) -> Point {
        self.eval_at(0.0)
    }

    /// Get the end point of this segment
    pub fn end_point(&self) -> Point {
        self.eval_at(1.0)
    }

    /// Split this segment at parameter t (in local [0, 1] range)
    ///
    /// Returns (left_segment, right_segment)
    pub fn split_at(&self, t: f64) -> (Self, Self) {
        match self.curve_type {
            CurveType::Line => {
                let line = Line::new(self.control_points[0], self.control_points[1]);
                let split_point = line.eval(t);

                let left = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Line,
                    0.0,
                    1.0,
                    vec![self.control_points[0], split_point],
                );

                let right = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Line,
                    0.0,
                    1.0,
                    vec![split_point, self.control_points[1]],
                );

                (left, right)
            }
            CurveType::Quadratic => {
                let quad = QuadBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                );
                let (q1, q2) = quad.subdivide();

                let left = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Quadratic,
                    0.0,
                    1.0,
                    vec![q1.p0, q1.p1, q1.p2],
                );

                let right = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Quadratic,
                    0.0,
                    1.0,
                    vec![q2.p0, q2.p1, q2.p2],
                );

                (left, right)
            }
            CurveType::Cubic => {
                let cubic = CubicBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                    self.control_points[3],
                );
                let (c1, c2) = cubic.subdivide();

                let left = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Cubic,
                    0.0,
                    1.0,
                    vec![c1.p0, c1.p1, c1.p2, c1.p3],
                );

                let right = Self::new(
                    self.shape_index,
                    self.segment_index,
                    CurveType::Cubic,
                    0.0,
                    1.0,
                    vec![c2.p0, c2.p1, c2.p2, c2.p3],
                );

                (left, right)
            }
        }
    }

    /// Get the bounding box of this curve segment
    pub fn bounding_box(&self) -> crate::quadtree::BoundingBox {
        match self.curve_type {
            CurveType::Line => {
                let line = Line::new(self.control_points[0], self.control_points[1]);
                let rect = line.bounding_box();
                crate::quadtree::BoundingBox::from_rect(rect)
            }
            CurveType::Quadratic => {
                let quad = QuadBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                );
                let rect = quad.bounding_box();
                crate::quadtree::BoundingBox::from_rect(rect)
            }
            CurveType::Cubic => {
                let cubic = CubicBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                    self.control_points[3],
                );
                let rect = cubic.bounding_box();
                crate::quadtree::BoundingBox::from_rect(rect)
            }
        }
    }

    /// Get the nearest point on this curve to a given point
    ///
    /// Returns (parameter t, nearest point, distance squared)
    pub fn nearest_point(&self, point: Point) -> (f64, Point, f64) {
        match self.curve_type {
            CurveType::Line => {
                let line = Line::new(self.control_points[0], self.control_points[1]);
                let t = line.nearest(point, 1e-6).t;
                let nearest = line.eval(t);
                let dist_sq = (nearest - point).hypot2();
                (t, nearest, dist_sq)
            }
            CurveType::Quadratic => {
                let quad = QuadBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                );
                let t = quad.nearest(point, 1e-6).t;
                let nearest = quad.eval(t);
                let dist_sq = (nearest - point).hypot2();
                (t, nearest, dist_sq)
            }
            CurveType::Cubic => {
                let cubic = CubicBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                    self.control_points[3],
                );
                let t = cubic.nearest(point, 1e-6).t;
                let nearest = cubic.eval(t);
                let dist_sq = (nearest - point).hypot2();
                (t, nearest, dist_sq)
            }
        }
    }

    /// Convert this segment to a path element
    pub fn to_path_element(&self) -> PathEl {
        match self.curve_type {
            CurveType::Line => PathEl::LineTo(self.control_points[1]),
            CurveType::Quadratic => {
                PathEl::QuadTo(self.control_points[1], self.control_points[2])
            }
            CurveType::Cubic => PathEl::CurveTo(
                self.control_points[1],
                self.control_points[2],
                self.control_points[3],
            ),
        }
    }

    /// Convert this segment to a cubic Bezier curve
    ///
    /// Lines and quadratic curves are converted to their cubic equivalents.
    pub fn to_cubic_bez(&self) -> CubicBez {
        match self.curve_type {
            CurveType::Line => {
                // Convert line to cubic: p0, p0 + 1/3(p1-p0), p0 + 2/3(p1-p0), p1
                let p0 = self.control_points[0];
                let p1 = self.control_points[1];
                let c1 = Point::new(
                    p0.x + (p1.x - p0.x) / 3.0,
                    p0.y + (p1.y - p0.y) / 3.0,
                );
                let c2 = Point::new(
                    p0.x + 2.0 * (p1.x - p0.x) / 3.0,
                    p0.y + 2.0 * (p1.y - p0.y) / 3.0,
                );
                CubicBez::new(p0, c1, c2, p1)
            }
            CurveType::Quadratic => {
                // Convert quadratic to cubic using standard formula
                // Cubic control points: p0, p0 + 2/3(p1-p0), p2 + 2/3(p1-p2), p2
                let p0 = self.control_points[0];
                let p1 = self.control_points[1];
                let p2 = self.control_points[2];
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
            CurveType::Cubic => {
                // Already cubic, just create from control points
                CubicBez::new(
                    self.control_points[0],
                    self.control_points[1],
                    self.control_points[2],
                    self.control_points[3],
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_segment_creation() {
        let seg = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0)],
        );

        assert_eq!(seg.curve_type, CurveType::Line);
        assert_eq!(seg.start_point(), Point::new(0.0, 0.0));
        assert_eq!(seg.end_point(), Point::new(100.0, 100.0));
    }

    #[test]
    fn test_line_segment_eval() {
        let seg = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0)],
        );

        let mid = seg.eval_at(0.5);
        assert!((mid.x - 50.0).abs() < 1e-6);
        assert!((mid.y - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_line_segment_split() {
        let seg = CurveSegment::new(
            0,
            0,
            CurveType::Line,
            0.0,
            1.0,
            vec![Point::new(0.0, 0.0), Point::new(100.0, 100.0)],
        );

        let (left, right) = seg.split_at(0.5);

        // After splitting, both segments have full parameter range
        assert_eq!(left.t_start, 0.0);
        assert_eq!(left.t_end, 1.0);
        assert_eq!(right.t_start, 0.0);
        assert_eq!(right.t_end, 1.0);

        // End of left should match start of right
        assert_eq!(left.end_point(), right.start_point());

        // Check that split happened at the midpoint
        let expected_mid = Point::new(50.0, 50.0);
        assert!((left.end_point().x - expected_mid.x).abs() < 1e-6);
        assert!((left.end_point().y - expected_mid.y).abs() < 1e-6);
    }

    #[test]
    fn test_quadratic_segment_creation() {
        let seg = CurveSegment::new(
            0,
            0,
            CurveType::Quadratic,
            0.0,
            1.0,
            vec![
                Point::new(0.0, 0.0),
                Point::new(50.0, 100.0),
                Point::new(100.0, 0.0),
            ],
        );

        assert_eq!(seg.curve_type, CurveType::Quadratic);
        assert_eq!(seg.control_points.len(), 3);
    }

    #[test]
    fn test_cubic_segment_creation() {
        let seg = CurveSegment::new(
            0,
            0,
            CurveType::Cubic,
            0.0,
            1.0,
            vec![
                Point::new(0.0, 0.0),
                Point::new(33.0, 100.0),
                Point::new(66.0, 100.0),
                Point::new(100.0, 0.0),
            ],
        );

        assert_eq!(seg.curve_type, CurveType::Cubic);
        assert_eq!(seg.control_points.len(), 4);
    }

    #[test]
    fn test_from_path_element_line() {
        let start = Point::new(0.0, 0.0);
        let end = Point::new(100.0, 100.0);
        let element = PathEl::LineTo(end);

        let seg = CurveSegment::from_path_element(0, 0, &element, start).unwrap();

        assert_eq!(seg.curve_type, CurveType::Line);
        assert_eq!(seg.control_points.len(), 2);
        assert_eq!(seg.start_point(), start);
        assert_eq!(seg.end_point(), end);
    }

    #[test]
    fn test_from_path_element_quad() {
        let start = Point::new(0.0, 0.0);
        let element = PathEl::QuadTo(Point::new(50.0, 100.0), Point::new(100.0, 0.0));

        let seg = CurveSegment::from_path_element(0, 0, &element, start).unwrap();

        assert_eq!(seg.curve_type, CurveType::Quadratic);
        assert_eq!(seg.control_points.len(), 3);
    }

    #[test]
    fn test_from_path_element_cubic() {
        let start = Point::new(0.0, 0.0);
        let element = PathEl::CurveTo(
            Point::new(33.0, 100.0),
            Point::new(66.0, 100.0),
            Point::new(100.0, 0.0),
        );

        let seg = CurveSegment::from_path_element(0, 0, &element, start).unwrap();

        assert_eq!(seg.curve_type, CurveType::Cubic);
        assert_eq!(seg.control_points.len(), 4);
    }
}
