//! Hit testing for selection and interaction
//!
//! Provides functions for testing if points or rectangles intersect with
//! shapes and objects, taking into account transform hierarchies.

use crate::layer::VectorLayer;
use crate::object::Object;
use crate::shape::Shape;
use uuid::Uuid;
use vello::kurbo::{Affine, Point, Rect, Shape as KurboShape};

/// Hit test a layer at a specific point
///
/// Tests objects in reverse order (front to back) and returns the first hit.
/// Combines parent_transform with object transforms for hierarchical testing.
///
/// # Arguments
///
/// * `layer` - The vector layer to test
/// * `point` - The point to test in screen/canvas space
/// * `tolerance` - Additional tolerance in pixels for stroke hit testing
/// * `parent_transform` - Transform from parent GraphicsObject(s)
///
/// # Returns
///
/// The UUID of the first object hit, or None if no hit
pub fn hit_test_layer(
    layer: &VectorLayer,
    point: Point,
    tolerance: f64,
    parent_transform: Affine,
) -> Option<Uuid> {
    // Test objects in reverse order (back to front in Vec = front to back for hit testing)
    for object in layer.objects.iter().rev() {
        // Get the shape for this object
        let shape = layer.get_shape(&object.shape_id)?;

        // Combine parent transform with object transform
        let combined_transform = parent_transform * object.to_affine();

        if hit_test_shape(shape, point, tolerance, combined_transform) {
            return Some(object.id);
        }
    }

    None
}

/// Hit test a single shape with a given transform
///
/// Tests if a point hits the shape, considering both fill and stroke.
///
/// # Arguments
///
/// * `shape` - The shape to test
/// * `point` - The point to test in screen/canvas space
/// * `tolerance` - Additional tolerance in pixels for stroke hit testing
/// * `transform` - The combined transform to apply to the shape
///
/// # Returns
///
/// true if the point hits the shape, false otherwise
pub fn hit_test_shape(
    shape: &Shape,
    point: Point,
    tolerance: f64,
    transform: Affine,
) -> bool {
    // Transform point to shape's local space
    // We need the inverse transform to go from screen space to shape space
    let inverse_transform = transform.inverse();
    let local_point = inverse_transform * point;

    // Check if point is inside filled path
    if shape.fill_color.is_some() {
        if shape.path().contains(local_point) {
            return true;
        }
    }

    // Check stroke bounds if has stroke
    if let Some(stroke_style) = &shape.stroke_style {
        let stroke_tolerance = stroke_style.width / 2.0 + tolerance;

        // For stroke hit testing, we need to check if the point is within
        // stroke_tolerance distance of the path
        // kurbo's winding() method can be used, or we can check bounding box first

        // Quick bounding box check with stroke tolerance
        let bbox = shape.path().bounding_box();
        let expanded_bbox = bbox.inflate(stroke_tolerance, stroke_tolerance);

        if !expanded_bbox.contains(local_point) {
            return false;
        }

        // For more accurate stroke hit testing, we would need to:
        // 1. Stroke the path with the stroke width
        // 2. Check if the point is contained in the stroked outline
        // For now, we do a simpler bounding box check
        // TODO: Implement accurate stroke hit testing using kurbo's stroke functionality

        // Simple approach: if within expanded bbox, consider it a hit for now
        return true;
    }

    false
}

/// Hit test objects within a rectangle (for marquee selection)
///
/// Returns all objects whose bounding boxes intersect with the given rectangle.
///
/// # Arguments
///
/// * `layer` - The vector layer to test
/// * `rect` - The selection rectangle in screen/canvas space
/// * `parent_transform` - Transform from parent GraphicsObject(s)
///
/// # Returns
///
/// Vector of UUIDs for all objects that intersect the rectangle
pub fn hit_test_objects_in_rect(
    layer: &VectorLayer,
    rect: Rect,
    parent_transform: Affine,
) -> Vec<Uuid> {
    let mut hits = Vec::new();

    for object in &layer.objects {
        // Get the shape for this object
        if let Some(shape) = layer.get_shape(&object.shape_id) {
            // Combine parent transform with object transform
            let combined_transform = parent_transform * object.to_affine();

            // Get shape bounding box in local space
            let bbox = shape.path().bounding_box();

            // Transform bounding box to screen space
            let transformed_bbox = combined_transform.transform_rect_bbox(bbox);

            // Check if rectangles intersect
            if rect.intersect(transformed_bbox).area() > 0.0 {
                hits.push(object.id);
            }
        }
    }

    hits
}

/// Get the bounding box of an object in screen space
///
/// # Arguments
///
/// * `object` - The object to get bounds for
/// * `shape` - The shape definition
/// * `parent_transform` - Transform from parent GraphicsObject(s)
///
/// # Returns
///
/// The bounding box in screen/canvas space
pub fn get_object_bounds(
    object: &Object,
    shape: &Shape,
    parent_transform: Affine,
) -> Rect {
    let combined_transform = parent_transform * object.to_affine();
    let local_bbox = shape.path().bounding_box();
    combined_transform.transform_rect_bbox(local_bbox)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::ShapeColor;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_hit_test_simple_circle() {
        let mut layer = VectorLayer::new("Test Layer");

        // Create a circle at (100, 100) with radius 50
        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));
        let object = Object::new(shape.id);

        layer.add_shape(shape);
        layer.add_object(object);

        // Test hit inside circle
        let hit = hit_test_layer(&layer, Point::new(100.0, 100.0), 0.0, Affine::IDENTITY);
        assert!(hit.is_some());

        // Test miss outside circle
        let miss = hit_test_layer(&layer, Point::new(200.0, 200.0), 0.0, Affine::IDENTITY);
        assert!(miss.is_none());
    }

    #[test]
    fn test_hit_test_with_transform() {
        let mut layer = VectorLayer::new("Test Layer");

        // Create a circle at origin
        let circle = Circle::new((0.0, 0.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));

        // Create object with translation
        let object = Object::new(shape.id).with_position(100.0, 100.0);

        layer.add_shape(shape);
        layer.add_object(object);

        // Test hit at translated position
        let hit = hit_test_layer(&layer, Point::new(100.0, 100.0), 0.0, Affine::IDENTITY);
        assert!(hit.is_some());

        // Test miss at origin (where shape is defined, but object is translated)
        let miss = hit_test_layer(&layer, Point::new(0.0, 0.0), 0.0, Affine::IDENTITY);
        assert!(miss.is_none());
    }

    #[test]
    fn test_marquee_selection() {
        let mut layer = VectorLayer::new("Test Layer");

        // Create two circles
        let circle1 = Circle::new((50.0, 50.0), 20.0);
        let path1 = circle1.to_path(0.1);
        let shape1 = Shape::new(path1).with_fill(ShapeColor::rgb(255, 0, 0));
        let object1 = Object::new(shape1.id);

        let circle2 = Circle::new((150.0, 150.0), 20.0);
        let path2 = circle2.to_path(0.1);
        let shape2 = Shape::new(path2).with_fill(ShapeColor::rgb(0, 255, 0));
        let object2 = Object::new(shape2.id);

        layer.add_shape(shape1);
        layer.add_object(object1);
        layer.add_shape(shape2);
        layer.add_object(object2);

        // Marquee that contains both circles
        let rect = Rect::new(0.0, 0.0, 200.0, 200.0);
        let hits = hit_test_objects_in_rect(&layer, rect, Affine::IDENTITY);
        assert_eq!(hits.len(), 2);

        // Marquee that contains only first circle
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let hits = hit_test_objects_in_rect(&layer, rect, Affine::IDENTITY);
        assert_eq!(hits.len(), 1);
    }
}
