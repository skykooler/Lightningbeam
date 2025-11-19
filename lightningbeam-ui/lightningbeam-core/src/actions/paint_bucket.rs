//! Paint bucket fill action
//!
//! This action performs a paint bucket fill operation starting from a click point,
//! using planar graph face detection to identify the region to fill.

use crate::action::Action;
use crate::curve_segment::CurveSegment;
use crate::document::Document;
use crate::gap_handling::GapHandlingMode;
use crate::layer::AnyLayer;
use crate::object::Object;
use crate::planar_graph::PlanarGraph;
use crate::shape::ShapeColor;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that performs a paint bucket fill operation
pub struct PaintBucketAction {
    /// Layer ID to add the filled shape to
    layer_id: Uuid,

    /// Click point where fill was initiated
    click_point: Point,

    /// Fill color for the shape
    fill_color: ShapeColor,

    /// Tolerance for gap bridging (in pixels)
    tolerance: f64,

    /// Gap handling mode
    gap_mode: GapHandlingMode,

    /// ID of the created shape (set after execution)
    created_shape_id: Option<Uuid>,

    /// ID of the created object (set after execution)
    created_object_id: Option<Uuid>,
}

impl PaintBucketAction {
    /// Create a new paint bucket action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The layer to add the filled shape to
    /// * `click_point` - Point where the user clicked to initiate fill
    /// * `fill_color` - Color to fill the region with
    /// * `tolerance` - Gap tolerance in pixels (default: 2.0)
    /// * `gap_mode` - Gap handling mode (SnapAndSplit or BridgeSegment)
    pub fn new(
        layer_id: Uuid,
        click_point: Point,
        fill_color: ShapeColor,
        tolerance: f64,
        gap_mode: GapHandlingMode,
    ) -> Self {
        Self {
            layer_id,
            click_point,
            fill_color,
            tolerance,
            gap_mode,
            created_shape_id: None,
            created_object_id: None,
        }
    }
}

impl Action for PaintBucketAction {
    fn execute(&mut self, document: &mut Document) {
        println!("=== PaintBucketAction::execute (Planar Graph Approach) ===");

        // Step 1: Extract curves from stroked shapes only (not filled regions)
        let all_curves = extract_curves_from_stroked_shapes(document, &self.layer_id);

        println!("Extracted {} curves from stroked shapes", all_curves.len());

        if all_curves.is_empty() {
            println!("No curves found, returning");
            return;
        }

        // Step 2: Build planar graph
        println!("Building planar graph...");
        let graph = PlanarGraph::build(&all_curves);

        // Store graph for debug visualization
        if let Ok(mut debug_graph) = crate::planar_graph::DEBUG_GRAPH.lock() {
            *debug_graph = Some(graph.clone());
        }

        // Step 3: Render debug visualization of planar graph
        println!("Rendering planar graph debug visualization...");
        let (nodes_shape, edges_shape) = graph.render_debug();
        let nodes_object = Object::new(nodes_shape.id);
        let edges_object = Object::new(edges_shape.id);

        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
            vector_layer.add_shape_internal(edges_shape);
            vector_layer.add_object_internal(edges_object);
            vector_layer.add_shape_internal(nodes_shape);
            vector_layer.add_object_internal(nodes_object);
            println!("DEBUG: Added graph visualization (yellow=edges, red=nodes)");
        }

        // Step 4: Find all faces
        println!("Finding faces in planar graph...");
        let faces = graph.find_faces();

        // Step 5: Find which face contains the click point
        println!("Finding face containing click point {:?}...", self.click_point);
        if let Some(face_idx) = graph.find_face_containing_point(self.click_point, &faces) {
            println!("Found face {} containing click point!", face_idx);

            // Build the face boundary using actual curve segments
            let face = &faces[face_idx];
            let face_path = graph.build_face_path(face);

            let face_shape = crate::shape::Shape::new(face_path)
                .with_fill(self.fill_color); // Use the requested fill color

            let face_object = Object::new(face_shape.id);

            // Store the created IDs for rollback
            self.created_shape_id = Some(face_shape.id);
            self.created_object_id = Some(face_object.id);

            if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
                vector_layer.add_shape_internal(face_shape);
                vector_layer.add_object_internal(face_object);
                println!("DEBUG: Added filled shape for face {}", face_idx);
            }
        } else {
            println!("Click point is not inside any face!");
        }

        println!("=== Paint Bucket Complete: Face filled with curves ===");
    }

    fn rollback(&mut self, document: &mut Document) {
        // Remove the created shape and object if they exist
        if let (Some(shape_id), Some(object_id)) = (self.created_shape_id, self.created_object_id) {
            let layer = match document.get_layer_mut(&self.layer_id) {
                Some(l) => l,
                None => return,
            };

            if let AnyLayer::Vector(vector_layer) = layer {
                vector_layer.remove_object_internal(&object_id);
                vector_layer.remove_shape_internal(&shape_id);
            }

            self.created_shape_id = None;
            self.created_object_id = None;
        }
    }

    fn description(&self) -> String {
        "Paint bucket fill".to_string()
    }
}

/// Extract curves from stroked shapes only (not filled regions)
///
/// This filters out paint bucket filled shapes which have only fills, not strokes.
/// Stroked shapes define boundaries for the planar graph.
fn extract_curves_from_stroked_shapes(
    document: &Document,
    layer_id: &Uuid,
) -> Vec<CurveSegment> {
    let mut all_curves = Vec::new();

    // Get the specified layer
    let layer = match document.get_layer(layer_id) {
        Some(l) => l,
        None => return all_curves,
    };

    // Extract curves only from this vector layer
    if let AnyLayer::Vector(vector_layer) = layer {
        // Extract curves from each object (which applies transforms to shapes)
        for object in &vector_layer.objects {
            // Find the shape for this object
            let shape = match vector_layer.shapes.iter().find(|s| s.id == object.shape_id) {
                Some(s) => s,
                None => continue,
            };

            // Skip shapes without strokes (these are filled regions, not boundaries)
            if shape.stroke_color.is_none() {
                continue;
            }

            // Get the transform matrix from the object
            let transform_affine = object.transform.to_affine();

            let path = shape.path();
            let mut current_point = Point::ZERO;
            let mut segment_index = 0;

            for element in path.elements() {
                // Extract curve segment from path element
                if let Some(mut segment) = CurveSegment::from_path_element(
                    shape.id.as_u128() as usize,
                    segment_index,
                    element,
                    current_point,
                ) {
                    // Apply the object's transform to all control points
                    for control_point in &mut segment.control_points {
                        *control_point = transform_affine * (*control_point);
                    }

                    all_curves.push(segment);
                    segment_index += 1;
                }

                // Update current point for next iteration (keep in local space)
                match element {
                    vello::kurbo::PathEl::MoveTo(p) => current_point = *p,
                    vello::kurbo::PathEl::LineTo(p) => current_point = *p,
                    vello::kurbo::PathEl::QuadTo(_, p) => current_point = *p,
                    vello::kurbo::PathEl::CurveTo(_, _, p) => current_point = *p,
                    vello::kurbo::PathEl::ClosePath => {}
                }
            }
        }
    }

    all_curves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use vello::kurbo::{Rect, Shape as KurboShape};

    #[test]
    fn test_paint_bucket_action_basic() {
        // Create a document with a vector layer
        let mut document = Document::new("Test");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create a simple rectangle shape (boundary for fill)
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = rect.to_path(0.1);
        let shape = Shape::new(path);
        let object = Object::new(shape.id);

        // Add the boundary shape
        if let Some(AnyLayer::Vector(layer)) = document.get_layer_mut(&layer_id) {
            layer.add_shape_internal(shape);
            layer.add_object_internal(object);
        }

        // Create and execute paint bucket action
        let mut action = PaintBucketAction::new(
            layer_id,
            Point::new(50.0, 50.0), // Click in center
            ShapeColor::rgb(255, 0, 0), // Red fill
            2.0,
            GapHandlingMode::BridgeSegment,
        );

        action.execute(&mut document);

        // Verify a filled shape was created
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            // Should have original shape + filled shape
            assert!(layer.shapes.len() >= 1);
            assert!(layer.objects.len() >= 1);
        } else {
            panic!("Layer not found or not a vector layer");
        }

        // Test rollback
        action.rollback(&mut document);

        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            // Should only have original shape
            assert_eq!(layer.shapes.len(), 1);
            assert_eq!(layer.objects.len(), 1);
        }
    }

    #[test]
    fn test_paint_bucket_action_description() {
        let action = PaintBucketAction::new(
            Uuid::new_v4(),
            Point::ZERO,
            ShapeColor::rgb(0, 0, 255),
            2.0,
            GapHandlingMode::BridgeSegment,
        );

        assert_eq!(action.description(), "Paint bucket fill");
    }
}
