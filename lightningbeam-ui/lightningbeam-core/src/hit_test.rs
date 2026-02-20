//! Hit testing for selection and interaction
//!
//! Provides functions for testing if points or rectangles intersect with
//! shapes and objects, taking into account transform hierarchies.

use crate::clip::ClipInstance;
use crate::layer::VectorLayer;
use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vello::kurbo::{Affine, Point, Rect, Shape as KurboShape};

/// Result of a hit test operation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitResult {
    /// Hit a shape instance
    ShapeInstance(Uuid),
    /// Hit a clip instance
    ClipInstance(Uuid),
}

/// Hit test a layer at a specific point
///
/// Tests shapes in the active keyframe in reverse order (front to back) and returns the first hit.
///
/// # Arguments
///
/// * `layer` - The vector layer to test
/// * `time` - The current time (for keyframe lookup)
/// * `point` - The point to test in screen/canvas space
/// * `tolerance` - Additional tolerance in pixels for stroke hit testing
/// * `parent_transform` - Transform from parent GraphicsObject(s)
///
/// # Returns
///
/// The UUID of the first shape hit, or None if no hit
pub fn hit_test_layer(
    layer: &VectorLayer,
    time: f64,
    point: Point,
    tolerance: f64,
    parent_transform: Affine,
) -> Option<Uuid> {
    // Test shapes in reverse order (front to back for hit testing)
    for shape in layer.shapes_at_time(time).iter().rev() {
        let combined_transform = parent_transform * shape.transform.to_affine();

        if hit_test_shape(shape, point, tolerance, combined_transform) {
            return Some(shape.id);
        }
    }

    None
}

/// Hit test a single shape with a given transform
///
/// Tests if a point hits the shape, considering both fill and stroke.
pub fn hit_test_shape(
    shape: &Shape,
    point: Point,
    tolerance: f64,
    transform: Affine,
) -> bool {
    // Transform point to shape's local space
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

        let bbox = shape.path().bounding_box();
        let expanded_bbox = bbox.inflate(stroke_tolerance, stroke_tolerance);

        if !expanded_bbox.contains(local_point) {
            return false;
        }

        return true;
    }

    false
}

/// Hit test objects within a rectangle (for marquee selection)
///
/// Returns all shapes in the active keyframe whose bounding boxes intersect with the given rectangle.
pub fn hit_test_objects_in_rect(
    layer: &VectorLayer,
    time: f64,
    rect: Rect,
    parent_transform: Affine,
) -> Vec<Uuid> {
    let mut hits = Vec::new();

    for shape in layer.shapes_at_time(time) {
        let combined_transform = parent_transform * shape.transform.to_affine();

        // Get shape bounding box in local space
        let bbox = shape.path().bounding_box();

        // Transform bounding box to screen space
        let transformed_bbox = combined_transform.transform_rect_bbox(bbox);

        // Check if rectangles intersect
        if rect.intersect(transformed_bbox).area() > 0.0 {
            hits.push(shape.id);
        }
    }

    hits
}

/// Get the bounding box of a shape in screen space
pub fn get_shape_bounds(
    shape: &Shape,
    parent_transform: Affine,
) -> Rect {
    let combined_transform = parent_transform * shape.transform.to_affine();
    let local_bbox = shape.path().bounding_box();
    combined_transform.transform_rect_bbox(local_bbox)
}

/// Hit test a single clip instance with a given clip bounds
pub fn hit_test_clip_instance(
    clip_instance: &ClipInstance,
    clip_width: f64,
    clip_height: f64,
    point: Point,
    parent_transform: Affine,
) -> bool {
    let clip_rect = Rect::new(0.0, 0.0, clip_width, clip_height);
    let combined_transform = parent_transform * clip_instance.transform.to_affine();
    let transformed_rect = combined_transform.transform_rect_bbox(clip_rect);
    transformed_rect.contains(point)
}

/// Get the bounding box of a clip instance in screen space
pub fn get_clip_instance_bounds(
    clip_instance: &ClipInstance,
    clip_width: f64,
    clip_height: f64,
    parent_transform: Affine,
) -> Rect {
    let clip_rect = Rect::new(0.0, 0.0, clip_width, clip_height);
    let combined_transform = parent_transform * clip_instance.transform.to_affine();
    combined_transform.transform_rect_bbox(clip_rect)
}

/// Hit test clip instances at a specific point
pub fn hit_test_clip_instances(
    clip_instances: &[ClipInstance],
    document: &crate::document::Document,
    point: Point,
    parent_transform: Affine,
    timeline_time: f64,
) -> Option<Uuid> {
    for clip_instance in clip_instances.iter().rev() {
        let clip_time = ((timeline_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

        let content_bounds = if let Some(vector_clip) = document.get_vector_clip(&clip_instance.clip_id) {
            vector_clip.calculate_content_bounds(document, clip_time)
        } else if let Some(video_clip) = document.get_video_clip(&clip_instance.clip_id) {
            Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
        } else {
            continue;
        };

        let clip_transform = parent_transform * clip_instance.transform.to_affine();
        let clip_bbox = clip_transform.transform_rect_bbox(content_bounds);

        if clip_bbox.contains(point) {
            return Some(clip_instance.id);
        }
    }

    None
}

/// Hit test clip instances within a rectangle (for marquee selection)
pub fn hit_test_clip_instances_in_rect(
    clip_instances: &[ClipInstance],
    document: &crate::document::Document,
    rect: Rect,
    parent_transform: Affine,
    timeline_time: f64,
) -> Vec<Uuid> {
    let mut hits = Vec::new();

    for clip_instance in clip_instances {
        let clip_time = ((timeline_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

        let content_bounds = if let Some(vector_clip) = document.get_vector_clip(&clip_instance.clip_id) {
            vector_clip.calculate_content_bounds(document, clip_time)
        } else if let Some(video_clip) = document.get_video_clip(&clip_instance.clip_id) {
            Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
        } else {
            continue;
        };

        let clip_transform = parent_transform * clip_instance.transform.to_affine();
        let clip_bbox = clip_transform.transform_rect_bbox(content_bounds);

        if rect.intersect(clip_bbox).area() > 0.0 {
            hits.push(clip_instance.id);
        }
    }

    hits
}

/// Result of a vector editing hit test
///
/// Represents different types of hits in order of priority:
/// ControlPoint > Vertex > Curve > Fill
#[derive(Debug, Clone, Copy)]
pub enum VectorEditHit {
    /// Hit a control point (BezierEdit tool only)
    ControlPoint {
        shape_instance_id: Uuid,
        curve_index: usize,
        point_index: u8,
    },
    /// Hit a vertex (anchor point)
    Vertex {
        shape_instance_id: Uuid,
        vertex_index: usize,
    },
    /// Hit a curve segment
    Curve {
        shape_instance_id: Uuid,
        curve_index: usize,
        parameter_t: f64,
    },
    /// Hit shape fill
    Fill { shape_instance_id: Uuid },
}

/// Tolerances for vector editing hit testing (in screen pixels)
#[derive(Debug, Clone, Copy)]
pub struct EditingHitTolerance {
    pub control_point: f64,
    pub vertex: f64,
    pub curve: f64,
    pub fill: f64,
}

impl Default for EditingHitTolerance {
    fn default() -> Self {
        Self {
            control_point: 10.0,
            vertex: 15.0,
            curve: 15.0,
            fill: 0.0,
        }
    }
}

impl EditingHitTolerance {
    pub fn scaled_by_zoom(zoom: f64) -> Self {
        Self {
            control_point: 10.0 / zoom,
            vertex: 15.0 / zoom,
            curve: 15.0 / zoom,
            fill: 0.0,
        }
    }
}

/// Hit test for vector editing with priority-based detection
pub fn hit_test_vector_editing(
    layer: &VectorLayer,
    time: f64,
    point: Point,
    tolerance: &EditingHitTolerance,
    parent_transform: Affine,
    show_control_points: bool,
) -> Option<VectorEditHit> {
    use crate::bezpath_editing::extract_editable_curves;
    use vello::kurbo::{ParamCurve, ParamCurveNearest};

    // Test shapes in reverse order (front to back for hit testing)
    for shape in layer.shapes_at_time(time).iter().rev() {
        let combined_transform = parent_transform * shape.transform.to_affine();
        let inverse_transform = combined_transform.inverse();
        let local_point = inverse_transform * point;

        // Calculate the scale factor to transform screen-space tolerances to local space
        let coeffs = combined_transform.as_coeffs();
        let scale_x = (coeffs[0].powi(2) + coeffs[1].powi(2)).sqrt();
        let scale_y = (coeffs[2].powi(2) + coeffs[3].powi(2)).sqrt();
        let avg_scale = (scale_x + scale_y) / 2.0;
        let local_tolerance_factor = 1.0 / avg_scale.max(0.001);

        let editable = extract_editable_curves(shape.path());

        // Priority 1: Control points (only in BezierEdit mode)
        if show_control_points {
            let local_cp_tolerance = tolerance.control_point * local_tolerance_factor;
            for (i, curve) in editable.curves.iter().enumerate() {
                let dist_p1 = (curve.p1 - local_point).hypot();
                if dist_p1 < local_cp_tolerance {
                    return Some(VectorEditHit::ControlPoint {
                        shape_instance_id: shape.id,
                        curve_index: i,
                        point_index: 1,
                    });
                }

                let dist_p2 = (curve.p2 - local_point).hypot();
                if dist_p2 < local_cp_tolerance {
                    return Some(VectorEditHit::ControlPoint {
                        shape_instance_id: shape.id,
                        curve_index: i,
                        point_index: 2,
                    });
                }
            }
        }

        // Priority 2: Vertices (anchor points)
        let local_vertex_tolerance = tolerance.vertex * local_tolerance_factor;
        for (i, vertex) in editable.vertices.iter().enumerate() {
            let dist = (vertex.point - local_point).hypot();
            if dist < local_vertex_tolerance {
                return Some(VectorEditHit::Vertex {
                    shape_instance_id: shape.id,
                    vertex_index: i,
                });
            }
        }

        // Priority 3: Curves
        let local_curve_tolerance = tolerance.curve * local_tolerance_factor;
        for (i, curve) in editable.curves.iter().enumerate() {
            let nearest = curve.nearest(local_point, 1e-6);
            let nearest_point = curve.eval(nearest.t);
            let dist = (nearest_point - local_point).hypot();
            if dist < local_curve_tolerance {
                return Some(VectorEditHit::Curve {
                    shape_instance_id: shape.id,
                    curve_index: i,
                    parameter_t: nearest.t,
                });
            }
        }

        // Priority 4: Fill
        if shape.fill_color.is_some() && shape.path().contains(local_point) {
            return Some(VectorEditHit::Fill {
                shape_instance_id: shape.id,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::ShapeColor;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_hit_test_simple_circle() {
        let mut layer = VectorLayer::new("Test Layer");

        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));

        layer.add_shape_to_keyframe(shape, 0.0);

        // Test hit inside circle
        let hit = hit_test_layer(&layer, 0.0, Point::new(100.0, 100.0), 0.0, Affine::IDENTITY);
        assert!(hit.is_some());

        // Test miss outside circle
        let miss = hit_test_layer(&layer, 0.0, Point::new(200.0, 200.0), 0.0, Affine::IDENTITY);
        assert!(miss.is_none());
    }

    #[test]
    fn test_hit_test_with_transform() {
        let mut layer = VectorLayer::new("Test Layer");

        let circle = Circle::new((0.0, 0.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path)
            .with_fill(ShapeColor::rgb(255, 0, 0))
            .with_position(100.0, 100.0);

        layer.add_shape_to_keyframe(shape, 0.0);

        // Test hit at translated position
        let hit = hit_test_layer(&layer, 0.0, Point::new(100.0, 100.0), 0.0, Affine::IDENTITY);
        assert!(hit.is_some());

        // Test miss at origin (where shape is defined, but transform moves it)
        let miss = hit_test_layer(&layer, 0.0, Point::new(0.0, 0.0), 0.0, Affine::IDENTITY);
        assert!(miss.is_none());
    }

    #[test]
    fn test_marquee_selection() {
        let mut layer = VectorLayer::new("Test Layer");

        let circle1 = Circle::new((50.0, 50.0), 20.0);
        let shape1 = Shape::new(circle1.to_path(0.1)).with_fill(ShapeColor::rgb(255, 0, 0));

        let circle2 = Circle::new((150.0, 150.0), 20.0);
        let shape2 = Shape::new(circle2.to_path(0.1)).with_fill(ShapeColor::rgb(0, 255, 0));

        layer.add_shape_to_keyframe(shape1, 0.0);
        layer.add_shape_to_keyframe(shape2, 0.0);

        // Marquee that contains both circles
        let rect = Rect::new(0.0, 0.0, 200.0, 200.0);
        let hits = hit_test_objects_in_rect(&layer, 0.0, rect, Affine::IDENTITY);
        assert_eq!(hits.len(), 2);

        // Marquee that contains only first circle
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let hits = hit_test_objects_in_rect(&layer, 0.0, rect, Affine::IDENTITY);
        assert_eq!(hits.len(), 1);
    }
}
