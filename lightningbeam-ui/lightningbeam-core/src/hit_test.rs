//! Hit testing for selection and interaction
//!
//! Provides functions for testing if points or rectangles intersect with
//! shapes and objects, taking into account transform hierarchies.

use crate::clip::ClipInstance;
use crate::dcel::{VertexId, EdgeId, FaceId};
use crate::layer::VectorLayer;
use crate::shape::Shape; // TODO: remove after DCEL migration complete
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vello::kurbo::{Affine, BezPath, Point, Rect, Shape as KurboShape};

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
    _layer: &VectorLayer,
    _time: f64,
    _point: Point,
    _tolerance: f64,
    _parent_transform: Affine,
) -> Option<Uuid> {
    // TODO: Implement DCEL-based hit testing (faces, edges, vertices)
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
    _layer: &VectorLayer,
    _time: f64,
    _rect: Rect,
    _parent_transform: Affine,
) -> Vec<Uuid> {
    // TODO: Implement DCEL-based marquee selection
    Vec::new()
}

/// Classification of shapes relative to a clipping region
#[derive(Debug, Clone)]
pub struct ShapeRegionClassification {
    /// Shapes entirely inside the region
    pub fully_inside: Vec<Uuid>,
    /// Shapes whose paths cross the region boundary
    pub intersecting: Vec<Uuid>,
    /// Shapes with no overlap with the region
    pub fully_outside: Vec<Uuid>,
}

/// Classify shapes in a layer relative to a clipping region.
///
/// Uses bounding box fast-rejection, then checks path-region intersection
/// and containment for accurate classification.
pub fn classify_shapes_by_region(
    layer: &VectorLayer,
    time: f64,
    region: &BezPath,
    parent_transform: Affine,
) -> ShapeRegionClassification {
    let result = ShapeRegionClassification {
        fully_inside: Vec::new(),
        intersecting: Vec::new(),
        fully_outside: Vec::new(),
    };

    let region_bbox = region.bounding_box();

    // TODO: Implement DCEL-based region classification
    let _ = (layer, time, parent_transform, region_bbox);

    result
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
        // Check time bounds: skip clip instances not active at this time
        let clip_duration = document.get_clip_duration(&clip_instance.clip_id).unwrap_or(0.0);
        let instance_end = clip_instance.timeline_start + clip_instance.effective_duration(clip_duration);
        if timeline_time < clip_instance.timeline_start || timeline_time >= instance_end {
            continue;
        }

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
        // Check time bounds: skip clip instances not active at this time
        let clip_duration = document.get_clip_duration(&clip_instance.clip_id).unwrap_or(0.0);
        let instance_end = clip_instance.timeline_start + clip_instance.effective_duration(clip_duration);
        if timeline_time < clip_instance.timeline_start || timeline_time >= instance_end {
            continue;
        }

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
        edge_id: EdgeId,
        point_index: u8,  // 1 = p1, 2 = p2
    },
    /// Hit a vertex (anchor point)
    Vertex {
        vertex_id: VertexId,
    },
    /// Hit a curve segment
    Curve {
        edge_id: EdgeId,
        parameter_t: f64,
    },
    /// Hit shape fill
    Fill {
        face_id: FaceId,
    },
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
    use kurbo::ParamCurveNearest;

    let dcel = layer.dcel_at_time(time)?;

    // Transform point into layer-local space
    let local_point = parent_transform.inverse() * point;

    // Priority: ControlPoint > Vertex > Curve

    // 1. Control points (only when show_control_points is true, e.g. BezierEdit tool)
    if show_control_points {
        let mut best_cp: Option<(EdgeId, u8, f64)> = None;
        for (i, edge) in dcel.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let edge_id = EdgeId(i as u32);
            // Check p1
            let d1 = local_point.distance(edge.curve.p1);
            if d1 < tolerance.control_point {
                if best_cp.is_none() || d1 < best_cp.unwrap().2 {
                    best_cp = Some((edge_id, 1, d1));
                }
            }
            // Check p2
            let d2 = local_point.distance(edge.curve.p2);
            if d2 < tolerance.control_point {
                if best_cp.is_none() || d2 < best_cp.unwrap().2 {
                    best_cp = Some((edge_id, 2, d2));
                }
            }
        }
        if let Some((edge_id, point_index, _)) = best_cp {
            return Some(VectorEditHit::ControlPoint { edge_id, point_index });
        }
    }

    // 2. Vertices
    let mut best_vertex: Option<(VertexId, f64)> = None;
    for (i, vertex) in dcel.vertices.iter().enumerate() {
        if vertex.deleted {
            continue;
        }
        let dist = local_point.distance(vertex.position);
        if dist < tolerance.vertex {
            if best_vertex.is_none() || dist < best_vertex.unwrap().1 {
                best_vertex = Some((VertexId(i as u32), dist));
            }
        }
    }
    if let Some((vertex_id, _)) = best_vertex {
        return Some(VectorEditHit::Vertex { vertex_id });
    }

    // 3. Curves (edges)
    let mut best_curve: Option<(EdgeId, f64, f64)> = None; // (edge_id, t, dist)
    for (i, edge) in dcel.edges.iter().enumerate() {
        if edge.deleted {
            continue;
        }
        let nearest = edge.curve.nearest(local_point, 0.5);
        let dist = nearest.distance_sq.sqrt();
        if dist < tolerance.curve {
            if best_curve.is_none() || dist < best_curve.unwrap().2 {
                best_curve = Some((EdgeId(i as u32), nearest.t, dist));
            }
        }
    }
    if let Some((edge_id, parameter_t, _)) = best_curve {
        return Some(VectorEditHit::Curve { edge_id, parameter_t });
    }

    // 4. Face hit testing skipped for now
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::ShapeColor;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_hit_test_simple_circle() {
        // TODO: DCEL - rewrite test
    }

    #[test]
    fn test_hit_test_with_transform() {
        // TODO: DCEL - rewrite test
    }

    #[test]
    fn test_marquee_selection() {
        // TODO: DCEL - rewrite test
    }
}
