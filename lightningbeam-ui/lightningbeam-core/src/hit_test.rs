//! Hit testing for selection and interaction
//!
//! Provides functions for testing if points or rectangles intersect with
//! vector graph elements and clip instances, taking into account transform hierarchies.

use crate::clip::{ClipDuration, ClipInstance};
use crate::vector_graph::{VertexId, EdgeId, FillId};
use crate::layer::VectorLayer;
use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use vello::kurbo::{Affine, Point, Rect, Shape as KurboShape};

/// Result of a hit test operation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitResult {
    /// Hit an edge (stroke)
    Edge(EdgeId),
    /// Hit a fill
    Fill(FillId),
    /// Hit a clip instance
    ClipInstance(Uuid),
}

/// Result of a graph-only hit test (no clip instances)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphHitResult {
    Edge(EdgeId),
    Fill(FillId),
}

/// Hit test a layer at a specific point, returning edge or fill hits.
///
/// Tests edges (strokes) and fills in the active keyframe.
/// Edge hits take priority over fill hits.
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
/// The first element hit, or None if no hit
pub fn hit_test_layer(
    layer: &VectorLayer,
    time: f64,
    point: Point,
    tolerance: f64,
    parent_transform: Affine,
) -> Option<GraphHitResult> {
    let graph = layer.graph_at_time(time)?;

    // Transform point to local space
    let local_point = parent_transform.inverse() * point;

    // 1. Check edges (strokes) — priority over fills
    let mut best_edge: Option<(EdgeId, f64)> = None;
    for (i, edge) in graph.edges.iter().enumerate() {
        if edge.deleted {
            continue;
        }
        // Only hit-test edges that have a visible stroke
        if edge.stroke_color.is_none() && edge.stroke_style.is_none() {
            continue;
        }

        use kurbo::ParamCurveNearest;
        let nearest = edge.curve.nearest(local_point, 0.5);
        let dist = nearest.distance_sq.sqrt();

        let hit_radius = edge
            .stroke_style
            .as_ref()
            .map(|s| s.width / 2.0)
            .unwrap_or(0.0)
            + tolerance;

        if dist < hit_radius {
            if best_edge.is_none() || dist < best_edge.unwrap().1 {
                best_edge = Some((EdgeId(i as u32), dist));
            }
        }
    }
    if let Some((edge_id, _)) = best_edge {
        return Some(GraphHitResult::Edge(edge_id));
    }

    // 2. Check fills
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue;
        }
        if fill.color.is_none() && fill.image_fill.is_none() && fill.gradient_fill.is_none() {
            continue;
        }
        if fill.boundary.is_empty() {
            continue;
        }

        let path = graph.fill_to_bezpath(FillId(i as u32));
        if path.winding(local_point) != 0 {
            return Some(GraphHitResult::Fill(FillId(i as u32)));
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

/// Result of graph marquee selection
#[derive(Debug, Default)]
pub struct GraphMarqueeResult {
    pub edges: Vec<EdgeId>,
    pub fills: Vec<FillId>,
}

/// Hit test graph elements within a rectangle (for marquee selection).
///
/// Selects edges whose both endpoints are inside the rect,
/// and fills whose all boundary vertices are inside the rect.
pub fn hit_test_graph_in_rect(
    layer: &VectorLayer,
    time: f64,
    rect: Rect,
    parent_transform: Affine,
) -> GraphMarqueeResult {
    let mut result = GraphMarqueeResult::default();

    let graph = match layer.graph_at_time(time) {
        Some(d) => d,
        None => return result,
    };

    let inv = parent_transform.inverse();
    let local_rect = inv.transform_rect_bbox(rect);

    // Check edges: both endpoints inside rect
    for (i, edge) in graph.edges.iter().enumerate() {
        if edge.deleted {
            continue;
        }
        let v1 = edge.vertices[0];
        let v2 = edge.vertices[1];
        if v1.is_none() || v2.is_none() {
            continue;
        }
        let p1 = graph.vertex(v1).position;
        let p2 = graph.vertex(v2).position;
        if local_rect.contains(p1) && local_rect.contains(p2) {
            result.edges.push(EdgeId(i as u32));
        }
    }

    // Check fills: all boundary vertices inside rect
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue;
        }
        if fill.boundary.is_empty() {
            continue;
        }
        let boundary_verts = graph.fill_boundary_vertices(FillId(i as u32));
        let all_inside = boundary_verts.iter().all(|&v| {
            !v.is_none() && local_rect.contains(graph.vertex(v).position)
        });
        if all_inside && !boundary_verts.is_empty() {
            result.fills.push(FillId(i as u32));
        }
    }

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
    let tempo_map = document.tempo_map();
    for clip_instance in clip_instances.iter().rev() {
        // Check time bounds: skip clip instances not active at this time
        // timeline_start/instance_end are in beats; convert timeline_time (seconds) to beats.
        // Hit-testing runs on vector/raster content, which is wall-clock seconds.
        let clip_duration = ClipDuration::Seconds(
            document.get_clip_duration(&clip_instance.clip_id).unwrap_or(daw_backend::Seconds::ZERO),
        );
        let instance_end = clip_instance.timeline_start + clip_instance.effective_duration(clip_duration, tempo_map);
        let timeline_beats = tempo_map.seconds_to_beats(daw_backend::Seconds(timeline_time));
        if timeline_beats < clip_instance.timeline_start || timeline_beats >= instance_end {
            continue;
        }

        // clip_time is in seconds; offset from clip start (in seconds) + trim_start (seconds)
        let start_secs = tempo_map.beats_to_seconds(clip_instance.timeline_start).seconds_to_f64();
        let clip_time =
            ((timeline_time - start_secs) * clip_instance.playback_speed) + clip_instance.trim_start.raw();

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
    let tempo_map = document.tempo_map();

    for clip_instance in clip_instances {
        // Check time bounds: skip clip instances not active at this time
        // timeline_start/instance_end are in beats; convert timeline_time (seconds) to beats.
        // Hit-testing runs on vector/raster content, which is wall-clock seconds.
        let clip_duration = ClipDuration::Seconds(
            document.get_clip_duration(&clip_instance.clip_id).unwrap_or(daw_backend::Seconds::ZERO),
        );
        let instance_end = clip_instance.timeline_start + clip_instance.effective_duration(clip_duration, tempo_map);
        let timeline_beats = tempo_map.seconds_to_beats(daw_backend::Seconds(timeline_time));
        if timeline_beats < clip_instance.timeline_start || timeline_beats >= instance_end {
            continue;
        }

        let start_secs = tempo_map.beats_to_seconds(clip_instance.timeline_start).seconds_to_f64();
        let clip_time =
            ((timeline_time - start_secs) * clip_instance.playback_speed) + clip_instance.trim_start.raw();

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
        fill_id: FillId,
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

    let graph = layer.graph_at_time(time)?;

    // Transform point into layer-local space
    let local_point = parent_transform.inverse() * point;

    // Priority: ControlPoint > Vertex > Curve > Fill

    // 1. Control points (only when show_control_points is true, e.g. BezierEdit tool)
    if show_control_points {
        let mut best_cp: Option<(EdgeId, u8, f64)> = None;
        for (i, edge) in graph.edges.iter().enumerate() {
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
    for (i, vertex) in graph.vertices.iter().enumerate() {
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
    for (i, edge) in graph.edges.iter().enumerate() {
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

    // 4. Fill testing
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue;
        }
        if fill.color.is_none() && fill.image_fill.is_none() && fill.gradient_fill.is_none() {
            continue;
        }
        if fill.boundary.is_empty() {
            continue;
        }
        let path = graph.fill_to_bezpath(FillId(i as u32));
        if path.winding(local_point) != 0 {
            return Some(VectorEditHit::Fill { fill_id: FillId(i as u32) });
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
        // TODO: VectorGraph - rewrite test
    }

    #[test]
    fn test_hit_test_with_transform() {
        // TODO: VectorGraph - rewrite test
    }

    #[test]
    fn test_marquee_selection() {
        // TODO: VectorGraph - rewrite test
    }
}
