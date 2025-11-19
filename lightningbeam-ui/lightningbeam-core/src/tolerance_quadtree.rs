//! Tolerance-based quadtree for proximity detection
//!
//! This quadtree subdivides until cells reach a minimum size (tolerance),
//! enabling efficient spatial queries for curves that are within tolerance
//! distance of each other.

use crate::quadtree::BoundingBox;
use crate::shape::{Shape, ShapeColor, StrokeStyle};
use std::collections::HashSet;
use vello::kurbo::{BezPath, CubicBez, ParamCurve, Shape as KurboShape};

/// Tolerance-based quadtree for spatial proximity detection
pub struct ToleranceQuadtree {
    root: QuadtreeNode,
    min_cell_size: f64,
}

impl ToleranceQuadtree {
    /// Create a new tolerance quadtree
    ///
    /// # Arguments
    ///
    /// * `bounds` - The bounding box of the entire space
    /// * `min_cell_size` - Minimum cell size (tolerance) - cells won't subdivide smaller than this
    pub fn new(bounds: BoundingBox, min_cell_size: f64) -> Self {
        Self {
            root: QuadtreeNode::new(bounds),
            min_cell_size,
        }
    }

    /// Insert a curve into the quadtree
    ///
    /// The curve will be added to all cells it overlaps with.
    pub fn insert_curve(&mut self, curve_id: usize, curve: &CubicBez) {
        let bbox = BoundingBox::from_rect(curve.bounding_box());
        self.root
            .insert(curve_id, &bbox, curve, self.min_cell_size);
    }

    /// Finalize the quadtree after all curves have been inserted (Step 4)
    ///
    /// This removes curves from all non-leaf nodes, keeping them only in minimum-size cells.
    /// Call this after inserting all curves.
    pub fn finalize(&mut self) {
        self.root.remove_curves_from_non_leaf_nodes(self.min_cell_size);
    }

    /// Get all curves that share cells with the given curve
    ///
    /// Returns a set of unique curve IDs that are spatially nearby.
    pub fn get_nearby_curves(&self, curve: &CubicBez) -> HashSet<usize> {
        let bbox = BoundingBox::from_rect(curve.bounding_box());
        let mut nearby = HashSet::new();
        self.root.query(&bbox, &mut nearby);
        nearby
    }

    /// Get all curves in cells that overlap with the given bounding box
    pub fn get_curves_in_region(&self, bbox: &BoundingBox) -> HashSet<usize> {
        let mut curves = HashSet::new();
        self.root.query(bbox, &mut curves);
        curves
    }

    /// Render debug visualization of the quadtree
    ///
    /// Returns two shapes: one for non-leaf nodes (blue) and one for leaf nodes (green).
    pub fn render_debug(&self) -> (Shape, Shape) {
        let mut non_leaf_path = BezPath::new();
        let mut leaf_path = BezPath::new();
        self.root.render_debug(&mut non_leaf_path, &mut leaf_path, 0);

        let stroke_style = StrokeStyle {
            width: 0.5,
            ..Default::default()
        };

        let non_leaf_shape = Shape::new(non_leaf_path).with_stroke(ShapeColor::rgb(100, 100, 255), stroke_style.clone());
        let leaf_shape = Shape::new(leaf_path).with_stroke(ShapeColor::rgb(0, 200, 0), stroke_style);

        (non_leaf_shape, leaf_shape)
    }
}

/// A node in the tolerance quadtree
struct QuadtreeNode {
    bounds: BoundingBox,
    curves: Vec<(usize, BoundingBox)>,  // (curve_id, bbox)
    children: Option<Box<[QuadtreeNode; 4]>>,
}

impl QuadtreeNode {
    fn new(bounds: BoundingBox) -> Self {
        Self {
            bounds,
            curves: Vec::new(),
            children: None,
        }
    }

    fn is_subdividable(&self, min_size: f64) -> bool {
        self.bounds.width() >= min_size * 2.0 && self.bounds.height() >= min_size * 2.0
    }

    fn subdivide(&mut self) {
        let x_mid = (self.bounds.x_min + self.bounds.x_max) / 2.0;
        let y_mid = (self.bounds.y_min + self.bounds.y_max) / 2.0;

        self.children = Some(Box::new([
            // Northwest (top-left)
            QuadtreeNode::new(BoundingBox::new(
                self.bounds.x_min,
                x_mid,
                self.bounds.y_min,
                y_mid,
            )),
            // Northeast (top-right)
            QuadtreeNode::new(BoundingBox::new(
                x_mid,
                self.bounds.x_max,
                self.bounds.y_min,
                y_mid,
            )),
            // Southwest (bottom-left)
            QuadtreeNode::new(BoundingBox::new(
                self.bounds.x_min,
                x_mid,
                y_mid,
                self.bounds.y_max,
            )),
            // Southeast (bottom-right)
            QuadtreeNode::new(BoundingBox::new(
                x_mid,
                self.bounds.x_max,
                y_mid,
                self.bounds.y_max,
            )),
        ]));
    }

    fn insert(
        &mut self,
        curve_id: usize,
        curve_bbox: &BoundingBox,
        curve: &CubicBez,
        min_size: f64,
    ) {
        // Step 2: Check if curve actually intersects this cell (not just bounding box)
        if !self.curve_intersects_cell(curve) {
            return;
        }

        // Add curve to this cell
        if !self.curves.iter().any(|(id, _)| *id == curve_id) {
            self.curves.push((curve_id, curve_bbox.clone()));
        }

        // Step 3: If this cell has at least one curve AND size > tolerance, subdivide
        if self.is_subdividable(min_size) && !self.curves.is_empty() && self.children.is_none() {
            self.subdivide();
        }

        // Recursively insert into children if they exist
        // Each child only gets curves that actually intersect it (checked by curve_intersects_cell)
        if let Some(ref mut children) = self.children {
            for child in children.iter_mut() {
                child.insert(curve_id, curve_bbox, curve, min_size);
            }
        }
    }

    /// Check if a curve actually passes through this cell by sampling it
    fn curve_intersects_cell(&self, curve: &CubicBez) -> bool {
        // Sample the curve at multiple points to see if any fall within this cell
        const SAMPLES: usize = 20;
        for i in 0..=SAMPLES {
            let t = i as f64 / SAMPLES as f64;
            let point = curve.eval(t);
            if self.bounds.contains_point(point) {
                return true;
            }
        }
        false
    }

    /// Remove curves from all non-minimum-size cells (Step 4)
    fn remove_curves_from_non_leaf_nodes(&mut self, min_size: f64) {
        // If this cell has children, clear its curves and recurse
        if self.children.is_some() {
            self.curves.clear();

            if let Some(ref mut children) = self.children {
                for child in children.iter_mut() {
                    child.remove_curves_from_non_leaf_nodes(min_size);
                }
            }
        }
        // If no children, this is a leaf node - keep its curves
    }

    fn query(&self, bbox: &BoundingBox, result: &mut HashSet<usize>) {
        // If query bbox doesn't overlap this cell, skip
        if !self.bounds.intersects(bbox) {
            return;
        }

        // Add all curves in this cell
        for &(curve_id, _) in &self.curves {
            result.insert(curve_id);
        }

        // Query children
        if let Some(ref children) = self.children {
            for child in children.iter() {
                child.query(bbox, result);
            }
        }
    }

    fn render_debug(&self, non_leaf_path: &mut BezPath, leaf_path: &mut BezPath, depth: usize) {
        use vello::kurbo::PathEl;

        // Choose which path to draw to based on whether this is a leaf node
        let is_leaf = self.children.is_none();

        // Draw cell boundary as outline only (not filled)
        // Draw the four edges of the rectangle without closing the path

        // Helper closure to add rectangle to the appropriate path
        let add_rect = |path: &mut BezPath| {
            // Top edge
            path.push(PathEl::MoveTo(vello::kurbo::Point::new(
                self.bounds.x_min,
                self.bounds.y_min,
            )));
            path.push(PathEl::LineTo(vello::kurbo::Point::new(
                self.bounds.x_max,
                self.bounds.y_min,
            )));

            // Right edge
            path.push(PathEl::MoveTo(vello::kurbo::Point::new(
                self.bounds.x_max,
                self.bounds.y_min,
            )));
            path.push(PathEl::LineTo(vello::kurbo::Point::new(
                self.bounds.x_max,
                self.bounds.y_max,
            )));

            // Bottom edge
            path.push(PathEl::MoveTo(vello::kurbo::Point::new(
                self.bounds.x_max,
                self.bounds.y_max,
            )));
            path.push(PathEl::LineTo(vello::kurbo::Point::new(
                self.bounds.x_min,
                self.bounds.y_max,
            )));

            // Left edge
            path.push(PathEl::MoveTo(vello::kurbo::Point::new(
                self.bounds.x_min,
                self.bounds.y_max,
            )));
            path.push(PathEl::LineTo(vello::kurbo::Point::new(
                self.bounds.x_min,
                self.bounds.y_min,
            )));
        };

        if is_leaf {
            add_rect(leaf_path);
        } else {
            add_rect(non_leaf_path);
        }

        // Recursively render children
        if let Some(ref children) = self.children {
            for child in children.iter() {
                child.render_debug(non_leaf_path, leaf_path, depth + 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::kurbo::Point;

    #[test]
    fn test_create_tolerance_quadtree() {
        let bounds = BoundingBox::new(0.0, 1000.0, 0.0, 1000.0);
        let tolerance = 2.0;
        let quadtree = ToleranceQuadtree::new(bounds, tolerance);
        assert!(quadtree.root.is_subdividable(tolerance));
    }

    #[test]
    fn test_insert_and_query() {
        let bounds = BoundingBox::new(0.0, 1000.0, 0.0, 1000.0);
        let mut quadtree = ToleranceQuadtree::new(bounds, 2.0);

        // Create a simple curve
        let curve = CubicBez::new(
            Point::new(100.0, 100.0),
            Point::new(200.0, 100.0),
            Point::new(200.0, 200.0),
            Point::new(100.0, 200.0),
        );

        quadtree.insert_curve(0, &curve);

        // Query with the same curve should find it
        let nearby = quadtree.get_nearby_curves(&curve);
        assert!(nearby.contains(&0));
    }

    #[test]
    fn test_nearby_curves() {
        let bounds = BoundingBox::new(0.0, 1000.0, 0.0, 1000.0);
        let mut quadtree = ToleranceQuadtree::new(bounds, 2.0);

        // Create two close curves
        let curve1 = CubicBez::new(
            Point::new(100.0, 100.0),
            Point::new(200.0, 100.0),
            Point::new(200.0, 200.0),
            Point::new(100.0, 200.0),
        );

        let curve2 = CubicBez::new(
            Point::new(150.0, 150.0),
            Point::new(250.0, 150.0),
            Point::new(250.0, 250.0),
            Point::new(150.0, 250.0),
        );

        quadtree.insert_curve(0, &curve1);
        quadtree.insert_curve(1, &curve2);

        // Both curves should find each other
        let nearby1 = quadtree.get_nearby_curves(&curve1);
        assert!(nearby1.contains(&0));
        assert!(nearby1.contains(&1));

        let nearby2 = quadtree.get_nearby_curves(&curve2);
        assert!(nearby2.contains(&0));
        assert!(nearby2.contains(&1));
    }
}
