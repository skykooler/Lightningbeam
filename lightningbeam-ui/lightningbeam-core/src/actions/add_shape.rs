//! Add shape action
//!
//! Handles adding a new shape to a vector layer's keyframe.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::Shape;
use uuid::Uuid;

/// Action that adds a shape to a vector layer's keyframe
pub struct AddShapeAction {
    /// Layer ID to add the shape to
    layer_id: Uuid,

    /// The shape to add (contains geometry, styling, transform, opacity)
    shape: Shape,

    /// Time of the keyframe to add to
    time: f64,

    /// ID of the created shape (set after execution)
    created_shape_id: Option<Uuid>,
}

impl AddShapeAction {
    pub fn new(layer_id: Uuid, shape: Shape, time: f64) -> Self {
        Self {
            layer_id,
            shape,
            time,
            created_shape_id: None,
        }
    }
}

impl Action for AddShapeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            let shape_id = self.shape.id;
            vector_layer.add_shape_to_keyframe(self.shape.clone(), self.time);
            self.created_shape_id = Some(shape_id);
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(shape_id) = self.created_shape_id {
            let layer = match document.get_layer_mut(&self.layer_id) {
                Some(l) => l,
                None => return Ok(()),
            };

            if let AnyLayer::Vector(vector_layer) = layer {
                vector_layer.remove_shape_from_keyframe(&shape_id, self.time);
            }

            self.created_shape_id = None;
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Add shape".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::ShapeColor;
    use vello::kurbo::{Rect, Shape as KurboShape};

    #[test]
    fn test_add_shape_action_rectangle() {
        let mut document = Document::new("Test");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let path = rect.to_path(0.1);
        let shape = Shape::new(path)
            .with_fill(ShapeColor::rgb(255, 0, 0))
            .with_position(50.0, 50.0);

        let mut action = AddShapeAction::new(layer_id, shape, 0.0);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let shapes = layer.shapes_at_time(0.0);
            assert_eq!(shapes.len(), 1);
            assert_eq!(shapes[0].transform.x, 50.0);
            assert_eq!(shapes[0].transform.y, 50.0);
        } else {
            panic!("Layer not found or not a vector layer");
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            assert_eq!(layer.shapes_at_time(0.0).len(), 0);
        }
    }
}
