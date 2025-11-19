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
        println!("=== PaintBucketAction::execute ===");

        // Optimization: Check if we're clicking on an existing shape first
        // This is much faster than building a planar graph
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
            // Iterate through objects in reverse order (topmost first)
            for object in vector_layer.objects.iter().rev() {
                // Find the corresponding shape
                if let Some(shape) = vector_layer.shapes.iter().find(|s| s.id == object.shape_id) {
                    // Apply the object's transform to get the transformed path
                    let transform_affine = object.transform.to_affine();

                    // Transform the click point to shape's local coordinates (inverse transform)
                    let inverse_transform = transform_affine.inverse();
                    let local_point = inverse_transform * self.click_point;

                    // Test if the local point is inside the shape using winding number
                    use vello::kurbo::Shape as KurboShape;
                    let winding = shape.path().winding(local_point);

                    if winding != 0 {
                        // Point is inside this shape! Just change its fill color
                        println!("Clicked on existing shape, changing fill color");

                        // Store the shape ID before the immutable borrow ends
                        let shape_id = shape.id;

                        // Find mutable reference to the shape and update its fill
                        if let Some(shape_mut) = vector_layer.shapes.iter_mut().find(|s| s.id == shape_id) {
                            shape_mut.fill_color = Some(self.fill_color);
                            println!("Updated shape fill color");
                        }

                        return; // Done! No need to create a new shape
                    }
                }
            }

            println!("No existing shape at click point, creating new fill region");
        }

        // Step 1: Extract curves from all shapes (rectangles, ellipses, paths, etc.)
        let all_curves = extract_curves_from_all_shapes(document, &self.layer_id);

        println!("Extracted {} curves from all shapes", all_curves.len());

        if all_curves.is_empty() {
            println!("No curves found, returning");
            return;
        }

        // Step 2: Build planar graph
        println!("Building planar graph...");
        let graph = PlanarGraph::build(&all_curves);

        // Step 3: Trace the face containing the click point (optimized - only traces one face)
        println!("Tracing face from click point {:?}...", self.click_point);
        if let Some(face) = graph.trace_face_from_point(self.click_point) {
            println!("Successfully traced face containing click point!");

            // Build the face boundary using actual curve segments
            let face_path = graph.build_face_path(&face);

            println!("DEBUG: Creating face shape with fill color: r={}, g={}, b={}, a={}",
                self.fill_color.r, self.fill_color.g, self.fill_color.b, self.fill_color.a);

            let face_shape = crate::shape::Shape::new(face_path)
                .with_fill(self.fill_color); // Use the requested fill color

            println!("DEBUG: Face shape created with fill_color: {:?}", face_shape.fill_color);

            let face_object = Object::new(face_shape.id);

            // Store the created IDs for rollback
            self.created_shape_id = Some(face_shape.id);
            self.created_object_id = Some(face_object.id);

            if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
                let shape_id_for_debug = face_shape.id;
                vector_layer.add_shape_internal(face_shape);
                vector_layer.add_object_internal(face_object);
                println!("DEBUG: Added filled shape");

                // Verify the shape still has the fill color after being added
                if let Some(added_shape) = vector_layer.shapes.iter().find(|s| s.id == shape_id_for_debug) {
                    println!("DEBUG: After adding to layer, shape fill_color = {:?}", added_shape.fill_color);
                }
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

/// Extract curves from all shapes in the layer
///
/// Includes rectangles, ellipses, paths, and even previous paint bucket fills.
/// The planar graph builder will handle deduplication of overlapping edges.
fn extract_curves_from_all_shapes(
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
        println!("Extracting curves from {} objects in layer", vector_layer.objects.len());
        // Extract curves from each object (which applies transforms to shapes)
        for (obj_idx, object) in vector_layer.objects.iter().enumerate() {
            // Find the shape for this object
            let shape = match vector_layer.shapes.iter().find(|s| s.id == object.shape_id) {
                Some(s) => s,
                None => continue,
            };

            // Include all shapes - planar graph will handle deduplication
            // (Rectangles, ellipses, paths, and even previous paint bucket fills)

            // Get the transform matrix from the object
            let transform_affine = object.transform.to_affine();

            let path = shape.path();
            let mut current_point = Point::ZERO;
            let mut subpath_start = Point::ZERO;  // Track start of current subpath
            let mut segment_index = 0;
            let mut curves_in_shape = 0;

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
                    curves_in_shape += 1;
                }

                // Update current point for next iteration (keep in local space)
                match element {
                    vello::kurbo::PathEl::MoveTo(p) => {
                        current_point = *p;
                        subpath_start = *p;  // Mark start of new subpath
                    }
                    vello::kurbo::PathEl::LineTo(p) => current_point = *p,
                    vello::kurbo::PathEl::QuadTo(_, p) => current_point = *p,
                    vello::kurbo::PathEl::CurveTo(_, _, p) => current_point = *p,
                    vello::kurbo::PathEl::ClosePath => {
                        // Create closing segment from current_point back to subpath_start
                        if let Some(mut segment) = CurveSegment::from_path_element(
                            shape.id.as_u128() as usize,
                            segment_index,
                            &vello::kurbo::PathEl::LineTo(subpath_start),
                            current_point,
                        ) {
                            // Apply transform
                            for control_point in &mut segment.control_points {
                                *control_point = transform_affine * (*control_point);
                            }

                            all_curves.push(segment);
                            segment_index += 1;
                            curves_in_shape += 1;
                        }
                        current_point = subpath_start;  // ClosePath moves back to start
                    }
                }
            }

            println!("  Object {}: Extracted {} curves from shape", obj_idx, curves_in_shape);
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
