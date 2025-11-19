//! Quadtree spatial indexing for efficient curve queries
//!
//! This module provides a quadtree data structure optimized for storing
//! bounding boxes of Bezier curve segments. It supports:
//! - Fast spatial queries (which curves intersect a region?)
//! - Auto-expanding boundary (grows to accommodate new curves)
//! - Efficient insertion and querying

use vello::kurbo::{Point, Rect};

/// Axis-aligned bounding box
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
}

impl BoundingBox {
    /// Create a new bounding box
    pub fn new(x_min: f64, x_max: f64, y_min: f64, y_max: f64) -> Self {
        Self {
            x_min,
            x_max,
            y_min,
            y_max,
        }
    }

    /// Create a bounding box from a vello Rect
    pub fn from_rect(rect: Rect) -> Self {
        Self {
            x_min: rect.x0,
            x_max: rect.x1,
            y_min: rect.y0,
            y_max: rect.y1,
        }
    }

    /// Create a bounding box around a point with padding
    pub fn around_point(point: Point, padding: f64) -> Self {
        Self {
            x_min: point.x - padding,
            x_max: point.x + padding,
            y_min: point.y - padding,
            y_max: point.y + padding,
        }
    }

    /// Get the width of this bounding box
    pub fn width(&self) -> f64 {
        self.x_max - self.x_min
    }

    /// Get the height of this bounding box
    pub fn height(&self) -> f64 {
        self.y_max - self.y_min
    }

    /// Get the combined size (width + height) for threshold checks
    pub fn size(&self) -> f64 {
        self.width() + self.height()
    }

    /// Check if this bounding box intersects with another
    pub fn intersects(&self, other: &BoundingBox) -> bool {
        !(other.x_max < self.x_min
            || other.x_min > self.x_max
            || other.y_max < self.y_min
            || other.y_min > self.y_max)
    }

    /// Check if this bounding box contains a point
    pub fn contains_point(&self, point: Point) -> bool {
        point.x >= self.x_min
            && point.x <= self.x_max
            && point.y >= self.y_min
            && point.y <= self.y_max
    }

    /// Check if this bounding box fully contains another bounding box
    pub fn contains_bbox(&self, other: &BoundingBox) -> bool {
        other.x_min >= self.x_min
            && other.x_max <= self.x_max
            && other.y_min >= self.y_min
            && other.y_max <= self.y_max
    }

    /// Get the center point of this bounding box
    pub fn center(&self) -> Point {
        Point::new(
            (self.x_min + self.x_max) / 2.0,
            (self.y_min + self.y_max) / 2.0,
        )
    }

    /// Expand this bounding box to include another
    pub fn expand_to_include(&mut self, other: &BoundingBox) {
        self.x_min = self.x_min.min(other.x_min);
        self.x_max = self.x_max.max(other.x_max);
        self.y_min = self.y_min.min(other.y_min);
        self.y_max = self.y_max.max(other.y_max);
    }
}

/// Quadtree for spatial indexing of curve segments
pub struct Quadtree {
    /// Boundary of this quadtree node
    boundary: BoundingBox,
    /// Maximum number of items before subdivision
    capacity: usize,
    /// Curve indices and their bounding boxes stored in this node
    items: Vec<(usize, BoundingBox)>,
    /// Whether this node has been subdivided
    divided: bool,

    // Child quadrants (only exist after subdivision)
    nw: Option<Box<Quadtree>>, // Northwest (top-left)
    ne: Option<Box<Quadtree>>, // Northeast (top-right)
    sw: Option<Box<Quadtree>>, // Southwest (bottom-left)
    se: Option<Box<Quadtree>>, // Southeast (bottom-right)
}

impl Quadtree {
    /// Create a new quadtree with the given boundary and capacity
    pub fn new(boundary: BoundingBox, capacity: usize) -> Self {
        Self {
            boundary,
            capacity,
            items: Vec::new(),
            divided: false,
            nw: None,
            ne: None,
            sw: None,
            se: None,
        }
    }

    /// Insert a curve's bounding box into the quadtree
    ///
    /// If the bbox doesn't fit in current boundary, the tree will expand.
    /// Returns true if inserted successfully.
    pub fn insert(&mut self, bbox: &BoundingBox, curve_idx: usize) -> bool {
        // If bbox is outside our boundary, we need to expand
        if !self.boundary.contains_bbox(bbox) {
            self.expand_to_contain(bbox);
        }

        self.insert_internal(bbox, curve_idx)
    }

    /// Internal insertion that assumes bbox fits within boundary
    fn insert_internal(&mut self, bbox: &BoundingBox, curve_idx: usize) -> bool {
        // Early exit if bbox doesn't intersect this node at all
        if !self.boundary.intersects(bbox) {
            return false;
        }

        // If we have space and haven't subdivided, store it here
        if !self.divided && self.items.len() < self.capacity {
            self.items.push((curve_idx, *bbox));
            return true;
        }

        // Otherwise, subdivide if needed
        if !self.divided {
            self.subdivide();
        }

        // Try to insert into children (might go into multiple quadrants)
        let mut inserted = false;
        if let Some(ref mut nw) = self.nw {
            inserted |= nw.insert_internal(bbox, curve_idx);
        }
        if let Some(ref mut ne) = self.ne {
            inserted |= ne.insert_internal(bbox, curve_idx);
        }
        if let Some(ref mut sw) = self.sw {
            inserted |= sw.insert_internal(bbox, curve_idx);
        }
        if let Some(ref mut se) = self.se {
            inserted |= se.insert_internal(bbox, curve_idx);
        }

        inserted
    }

    /// Subdivide this node into 4 quadrants
    fn subdivide(&mut self) {
        let x_mid = (self.boundary.x_min + self.boundary.x_max) / 2.0;
        let y_mid = (self.boundary.y_min + self.boundary.y_max) / 2.0;

        // Northwest (top-left)
        self.nw = Some(Box::new(Quadtree::new(
            BoundingBox::new(
                self.boundary.x_min,
                x_mid,
                self.boundary.y_min,
                y_mid,
            ),
            self.capacity,
        )));

        // Northeast (top-right)
        self.ne = Some(Box::new(Quadtree::new(
            BoundingBox::new(x_mid, self.boundary.x_max, self.boundary.y_min, y_mid),
            self.capacity,
        )));

        // Southwest (bottom-left)
        self.sw = Some(Box::new(Quadtree::new(
            BoundingBox::new(self.boundary.x_min, x_mid, y_mid, self.boundary.y_max),
            self.capacity,
        )));

        // Southeast (bottom-right)
        self.se = Some(Box::new(Quadtree::new(
            BoundingBox::new(x_mid, self.boundary.x_max, y_mid, self.boundary.y_max),
            self.capacity,
        )));

        self.divided = true;

        // Re-insert existing items into children
        let items_to_redistribute = std::mem::take(&mut self.items);
        for (idx, bbox) in items_to_redistribute {
            // Insert into all children that intersect with the bbox
            if let Some(ref mut nw) = self.nw {
                nw.insert_internal(&bbox, idx);
            }
            if let Some(ref mut ne) = self.ne {
                ne.insert_internal(&bbox, idx);
            }
            if let Some(ref mut sw) = self.sw {
                sw.insert_internal(&bbox, idx);
            }
            if let Some(ref mut se) = self.se {
                se.insert_internal(&bbox, idx);
            }
        }
    }

    /// Expand the quadtree to contain a bounding box that's outside current boundary
    ///
    /// This is the complex auto-expanding logic from the JS implementation.
    fn expand_to_contain(&mut self, bbox: &BoundingBox) {
        // Determine which direction we need to expand
        let needs_expand_left = bbox.x_min < self.boundary.x_min;
        let needs_expand_right = bbox.x_max > self.boundary.x_max;
        let needs_expand_top = bbox.y_min < self.boundary.y_min;
        let needs_expand_bottom = bbox.y_max > self.boundary.y_max;

        // Calculate the current width and height
        let width = self.boundary.width();
        let height = self.boundary.height();

        // Create a new root that's twice as large in the necessary direction(s)
        let new_boundary = if needs_expand_left && needs_expand_top {
            // Expand northwest
            BoundingBox::new(
                self.boundary.x_min - width,
                self.boundary.x_max,
                self.boundary.y_min - height,
                self.boundary.y_max,
            )
        } else if needs_expand_right && needs_expand_top {
            // Expand northeast
            BoundingBox::new(
                self.boundary.x_min,
                self.boundary.x_max + width,
                self.boundary.y_min - height,
                self.boundary.y_max,
            )
        } else if needs_expand_left && needs_expand_bottom {
            // Expand southwest
            BoundingBox::new(
                self.boundary.x_min - width,
                self.boundary.x_max,
                self.boundary.y_min,
                self.boundary.y_max + height,
            )
        } else if needs_expand_right && needs_expand_bottom {
            // Expand southeast
            BoundingBox::new(
                self.boundary.x_min,
                self.boundary.x_max + width,
                self.boundary.y_min,
                self.boundary.y_max + height,
            )
        } else if needs_expand_left {
            // Expand west
            BoundingBox::new(
                self.boundary.x_min - width,
                self.boundary.x_max,
                self.boundary.y_min,
                self.boundary.y_max,
            )
        } else if needs_expand_right {
            // Expand east
            BoundingBox::new(
                self.boundary.x_min,
                self.boundary.x_max + width,
                self.boundary.y_min,
                self.boundary.y_max,
            )
        } else if needs_expand_top {
            // Expand north
            BoundingBox::new(
                self.boundary.x_min,
                self.boundary.x_max,
                self.boundary.y_min - height,
                self.boundary.y_max,
            )
        } else {
            // Expand south
            BoundingBox::new(
                self.boundary.x_min,
                self.boundary.x_max,
                self.boundary.y_min,
                self.boundary.y_max + height,
            )
        };

        // Clone current tree to become a child of new root
        let old_tree = Quadtree {
            boundary: self.boundary,
            capacity: self.capacity,
            items: std::mem::take(&mut self.items),
            divided: self.divided,
            nw: self.nw.take(),
            ne: self.ne.take(),
            sw: self.sw.take(),
            se: self.se.take(),
        };

        // Update self to be the new larger root
        self.boundary = new_boundary;
        self.items.clear();
        self.divided = true;

        // Create quadrants and place old tree in appropriate position
        self.subdivide();

        // Move old tree to appropriate quadrant
        // When expanding diagonally, old tree goes in opposite corner
        // When expanding in one direction, old tree takes up half the space
        if needs_expand_left && needs_expand_top {
            // Old tree was in bottom-right, new space is top-left
            self.se = Some(Box::new(old_tree));
        } else if needs_expand_right && needs_expand_top {
            // Old tree was in bottom-left, new space is top-right
            self.sw = Some(Box::new(old_tree));
        } else if needs_expand_left && needs_expand_bottom {
            // Old tree was in top-right, new space is bottom-left
            self.ne = Some(Box::new(old_tree));
        } else if needs_expand_right && needs_expand_bottom {
            // Old tree was in top-left, new space is bottom-right
            self.nw = Some(Box::new(old_tree));
        } else {
            // For single-direction expansion, just place the old tree
            // We'll let it naturally distribute when items are inserted
            // Place it in a quadrant that makes sense for the expansion direction
            if needs_expand_left {
                self.ne = Some(Box::new(old_tree));
            } else if needs_expand_right {
                self.nw = Some(Box::new(old_tree));
            } else if needs_expand_top {
                self.sw = Some(Box::new(old_tree));
            } else {
                // needs_expand_bottom
                self.nw = Some(Box::new(old_tree));
            }
        }
    }

    /// Query the quadtree for all curve indices that intersect with the given range
    pub fn query(&self, range: &BoundingBox) -> Vec<usize> {
        let mut found = Vec::new();
        self.query_internal(range, &mut found);

        // Remove duplicates
        found.sort_unstable();
        found.dedup();

        found
    }

    /// Internal recursive query
    fn query_internal(&self, range: &BoundingBox, found: &mut Vec<usize>) {
        // If range doesn't intersect this node, nothing to do
        if !self.boundary.intersects(range) {
            return;
        }

        // Add items from this node that actually intersect the query range
        for (idx, bbox) in &self.items {
            if bbox.intersects(range) {
                found.push(*idx);
            }
        }

        // Recursively query children
        if self.divided {
            if let Some(ref nw) = self.nw {
                nw.query_internal(range, found);
            }
            if let Some(ref ne) = self.ne {
                ne.query_internal(range, found);
            }
            if let Some(ref sw) = self.sw {
                sw.query_internal(range, found);
            }
            if let Some(ref se) = self.se {
                se.query_internal(range, found);
            }
        }
    }

    /// Clear all items from the quadtree
    pub fn clear(&mut self) {
        self.items.clear();
        self.divided = false;
        self.nw = None;
        self.ne = None;
        self.sw = None;
        self.se = None;
    }

    /// Get the boundary of this quadtree
    pub fn boundary(&self) -> &BoundingBox {
        &self.boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bounding_box_creation() {
        let bbox = BoundingBox::new(0.0, 100.0, 0.0, 50.0);
        assert_eq!(bbox.width(), 100.0);
        assert_eq!(bbox.height(), 50.0);
        assert_eq!(bbox.size(), 150.0);
    }

    #[test]
    fn test_bounding_box_intersects() {
        let bbox1 = BoundingBox::new(0.0, 100.0, 0.0, 100.0);
        let bbox2 = BoundingBox::new(50.0, 150.0, 50.0, 150.0);
        let bbox3 = BoundingBox::new(200.0, 300.0, 200.0, 300.0);

        assert!(bbox1.intersects(&bbox2));
        assert!(bbox2.intersects(&bbox1));
        assert!(!bbox1.intersects(&bbox3));
        assert!(!bbox3.intersects(&bbox1));
    }

    #[test]
    fn test_bounding_box_contains_point() {
        let bbox = BoundingBox::new(0.0, 100.0, 0.0, 100.0);

        assert!(bbox.contains_point(Point::new(50.0, 50.0)));
        assert!(bbox.contains_point(Point::new(0.0, 0.0)));
        assert!(bbox.contains_point(Point::new(100.0, 100.0)));
        assert!(!bbox.contains_point(Point::new(150.0, 50.0)));
        assert!(!bbox.contains_point(Point::new(50.0, 150.0)));
    }

    #[test]
    fn test_quadtree_insert_and_query() {
        let mut qt = Quadtree::new(BoundingBox::new(0.0, 100.0, 0.0, 100.0), 4);

        // Insert some curves
        qt.insert(&BoundingBox::new(10.0, 20.0, 10.0, 20.0), 0);
        qt.insert(&BoundingBox::new(30.0, 40.0, 30.0, 40.0), 1);
        qt.insert(&BoundingBox::new(60.0, 70.0, 60.0, 70.0), 2);

        // Query overlapping region
        let results = qt.query(&BoundingBox::new(15.0, 35.0, 15.0, 35.0));

        assert!(results.contains(&0));
        assert!(results.contains(&1));
        assert!(!results.contains(&2));
    }

    #[test]
    fn test_quadtree_subdivision() {
        let mut qt = Quadtree::new(BoundingBox::new(0.0, 100.0, 0.0, 100.0), 2);

        // Insert enough items to force subdivision
        qt.insert(&BoundingBox::new(10.0, 20.0, 10.0, 20.0), 0);
        qt.insert(&BoundingBox::new(30.0, 40.0, 30.0, 40.0), 1);
        qt.insert(&BoundingBox::new(60.0, 70.0, 60.0, 70.0), 2);
        qt.insert(&BoundingBox::new(80.0, 90.0, 80.0, 90.0), 3);

        assert!(qt.divided);

        // Should still be able to query
        let results = qt.query(&BoundingBox::new(0.0, 100.0, 0.0, 100.0));
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn test_quadtree_clear() {
        let mut qt = Quadtree::new(BoundingBox::new(0.0, 100.0, 0.0, 100.0), 4);

        qt.insert(&BoundingBox::new(10.0, 20.0, 10.0, 20.0), 0);
        qt.insert(&BoundingBox::new(30.0, 40.0, 30.0, 40.0), 1);

        qt.clear();

        let results = qt.query(&BoundingBox::new(0.0, 100.0, 0.0, 100.0));
        assert_eq!(results.len(), 0);
        assert!(!qt.divided);
    }

    #[test]
    fn test_quadtree_auto_expand() {
        let mut qt = Quadtree::new(BoundingBox::new(0.0, 100.0, 0.0, 100.0), 4);

        // Insert bbox outside current boundary
        qt.insert(&BoundingBox::new(150.0, 200.0, 150.0, 200.0), 0);

        // Boundary should have expanded
        assert!(qt.boundary().x_max >= 200.0 || qt.boundary().y_max >= 200.0);

        // Should be able to query the item
        let results = qt.query(&BoundingBox::new(150.0, 200.0, 150.0, 200.0));
        assert!(results.contains(&0));
    }
}
