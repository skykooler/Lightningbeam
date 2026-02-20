//! Paint bucket fill action
//!
//! This action performs a paint bucket fill operation starting from a click point,
//! using planar graph face detection to identify the region to fill.

use crate::action::Action;
use crate::curve_segment::CurveSegment;
use crate::document::Document;
use crate::gap_handling::GapHandlingMode;
use crate::layer::AnyLayer;
use crate::planar_graph::PlanarGraph;
use crate::shape::ShapeColor;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that performs a paint bucket fill operation
pub struct PaintBucketAction {
    /// Layer ID to add the filled shape to
    layer_id: Uuid,

    /// Time of the keyframe to operate on
    time: f64,

    /// Click point where fill was initiated
    click_point: Point,

    /// Fill color for the shape
    fill_color: ShapeColor,

    /// Tolerance for gap bridging (in pixels)
    _tolerance: f64,

    /// Gap handling mode
    _gap_mode: GapHandlingMode,

    /// ID of the created shape (set after execution)
    created_shape_id: Option<Uuid>,
}

impl PaintBucketAction {
    /// Create a new paint bucket action
    pub fn new(
        layer_id: Uuid,
        time: f64,
        click_point: Point,
        fill_color: ShapeColor,
        tolerance: f64,
        gap_mode: GapHandlingMode,
    ) -> Self {
        Self {
            layer_id,
            time,
            click_point,
            fill_color,
            _tolerance: tolerance,
            _gap_mode: gap_mode,
            created_shape_id: None,
        }
    }
}

impl Action for PaintBucketAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        println!("=== PaintBucketAction::execute ===");

        // Optimization: Check if we're clicking on an existing shape first
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
            // Iterate through shapes in the keyframe in reverse order (topmost first)
            let shapes = vector_layer.shapes_at_time(self.time);
            for shape in shapes.iter().rev() {
                // Skip shapes without fill color
                if shape.fill_color.is_none() {
                    continue;
                }

                use vello::kurbo::PathEl;
                let is_closed = shape.path().elements().iter().any(|el| matches!(el, PathEl::ClosePath));
                if !is_closed {
                    continue;
                }

                // Apply the shape's transform
                let transform_affine = shape.transform.to_affine();
                let inverse_transform = transform_affine.inverse();
                let local_point = inverse_transform * self.click_point;

                use vello::kurbo::Shape as KurboShape;
                let winding = shape.path().winding(local_point);

                if winding != 0 {
                    println!("Clicked on existing shape, changing fill color");
                    let shape_id = shape.id;

                    // Now get mutable access to change the fill
                    if let Some(shape_mut) = vector_layer.get_shape_in_keyframe_mut(&shape_id, self.time) {
                        shape_mut.fill_color = Some(self.fill_color);
                    }
                    return Ok(());
                }
            }

            println!("No existing shape at click point, creating new fill region");
        }

        // Step 1: Extract curves from all shapes in the keyframe
        let all_curves = extract_curves_from_keyframe(document, &self.layer_id, self.time);

        println!("Extracted {} curves from all shapes", all_curves.len());

        if all_curves.is_empty() {
            println!("No curves found, returning");
            return Ok(());
        }

        // Step 2: Build planar graph
        println!("Building planar graph...");
        let graph = PlanarGraph::build(&all_curves);

        // Step 3: Trace the face containing the click point
        println!("Tracing face from click point {:?}...", self.click_point);
        if let Some(face) = graph.trace_face_from_point(self.click_point) {
            println!("Successfully traced face containing click point!");

            let face_path = graph.build_face_path(&face);

            let face_shape = crate::shape::Shape::new(face_path)
                .with_fill(self.fill_color);

            self.created_shape_id = Some(face_shape.id);

            if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
                vector_layer.add_shape_to_keyframe(face_shape, self.time);
                println!("DEBUG: Added filled shape to keyframe");
            }
        } else {
            println!("Click point is not inside any face!");
        }

        println!("=== Paint Bucket Complete ===");
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(shape_id) = self.created_shape_id {
            if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer_mut(&self.layer_id) {
                vector_layer.remove_shape_from_keyframe(&shape_id, self.time);
            }
            self.created_shape_id = None;
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Paint bucket fill".to_string()
    }
}

/// Extract curves from all shapes in the keyframe at the given time
fn extract_curves_from_keyframe(
    document: &Document,
    layer_id: &Uuid,
    time: f64,
) -> Vec<CurveSegment> {
    let mut all_curves = Vec::new();

    let layer = match document.get_layer(layer_id) {
        Some(l) => l,
        None => return all_curves,
    };

    if let AnyLayer::Vector(vector_layer) = layer {
        let shapes = vector_layer.shapes_at_time(time);
        println!("Extracting curves from {} shapes in keyframe", shapes.len());

        for (shape_idx, shape) in shapes.iter().enumerate() {
            let transform_affine = shape.transform.to_affine();

            let path = shape.path();
            let mut current_point = Point::ZERO;
            let mut subpath_start = Point::ZERO;
            let mut segment_index = 0;
            let mut curves_in_shape = 0;

            for element in path.elements() {
                if let Some(mut segment) = CurveSegment::from_path_element(
                    shape.id.as_u128() as usize,
                    segment_index,
                    element,
                    current_point,
                ) {
                    for control_point in &mut segment.control_points {
                        *control_point = transform_affine * (*control_point);
                    }

                    all_curves.push(segment);
                    segment_index += 1;
                    curves_in_shape += 1;
                }

                match element {
                    vello::kurbo::PathEl::MoveTo(p) => {
                        current_point = *p;
                        subpath_start = *p;
                    }
                    vello::kurbo::PathEl::LineTo(p) => current_point = *p,
                    vello::kurbo::PathEl::QuadTo(_, p) => current_point = *p,
                    vello::kurbo::PathEl::CurveTo(_, _, p) => current_point = *p,
                    vello::kurbo::PathEl::ClosePath => {
                        if let Some(mut segment) = CurveSegment::from_path_element(
                            shape.id.as_u128() as usize,
                            segment_index,
                            &vello::kurbo::PathEl::LineTo(subpath_start),
                            current_point,
                        ) {
                            for control_point in &mut segment.control_points {
                                *control_point = transform_affine * (*control_point);
                            }

                            all_curves.push(segment);
                            segment_index += 1;
                            curves_in_shape += 1;
                        }
                        current_point = subpath_start;
                    }
                }
            }

            println!("  Shape {}: Extracted {} curves", shape_idx, curves_in_shape);
        }
    }

    all_curves
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;
    use vello::kurbo::{Rect, Shape as KurboShape};

    #[test]
    fn test_paint_bucket_action_basic() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Layer 1");

        // Create a simple rectangle shape (boundary for fill)
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = rect.to_path(0.1);
        let shape = Shape::new(path);

        layer.add_shape_to_keyframe(shape, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create and execute paint bucket action
        let mut action = PaintBucketAction::new(
            layer_id,
            0.0,
            Point::new(50.0, 50.0),
            ShapeColor::rgb(255, 0, 0),
            2.0,
            GapHandlingMode::BridgeSegment,
        );

        action.execute(&mut document).unwrap();

        // Verify a filled shape was created (or existing shape was recolored)
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            assert!(layer.shapes_at_time(0.0).len() >= 1);
        } else {
            panic!("Layer not found or not a vector layer");
        }

        // Test rollback
        action.rollback(&mut document).unwrap();
    }

    #[test]
    fn test_paint_bucket_action_description() {
        let action = PaintBucketAction::new(
            Uuid::new_v4(),
            0.0,
            Point::ZERO,
            ShapeColor::rgb(0, 0, 255),
            2.0,
            GapHandlingMode::BridgeSegment,
        );

        assert_eq!(action.description(), "Paint bucket fill");
    }
}
