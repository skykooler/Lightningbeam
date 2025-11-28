//! Move shape instances action
//!
//! Handles moving one or more shape instances to new positions.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that moves shape instances to new positions
pub struct MoveShapeInstancesAction {
    /// Layer ID containing the shape instances
    layer_id: Uuid,

    /// Map of object IDs to their old and new positions
    shape_instance_positions: HashMap<Uuid, (Point, Point)>, // (old_pos, new_pos)
}

impl MoveShapeInstancesAction {
    /// Create a new move shape instances action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The layer containing the shape instances
    /// * `shape_instance_positions` - Map of object IDs to (old_position, new_position)
    pub fn new(layer_id: Uuid, shape_instance_positions: HashMap<Uuid, (Point, Point)>) -> Self {
        Self {
            layer_id,
            shape_instance_positions,
        }
    }
}

impl Action for MoveShapeInstancesAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            for (shape_instance_id, (_old, new)) in &self.shape_instance_positions {
                vector_layer.modify_object_internal(shape_instance_id, |obj| {
                    obj.transform.x = new.x;
                    obj.transform.y = new.y;
                });
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            for (shape_instance_id, (old, _new)) in &self.shape_instance_positions {
                vector_layer.modify_object_internal(shape_instance_id, |obj| {
                    obj.transform.x = old.x;
                    obj.transform.y = old.y;
                });
            }
        }
    }

    fn description(&self) -> String {
        let count = self.shape_instance_positions.len();
        if count == 1 {
            "Move shape instance".to_string()
        } else {
            format!("Move {} shape instances", count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::object::ShapeInstance;
    use crate::shape::Shape;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_move_shape_instances_action() {
        // Create a document with a test object
        let mut document = Document::new("Test");

        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path);
        let object = ShapeInstance::new(shape.id).with_position(50.0, 50.0);

        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        let shape_instance_id = vector_layer.add_object(object);
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create move action
        let mut positions = HashMap::new();
        positions.insert(
            shape_instance_id,
            (Point::new(50.0, 50.0), Point::new(150.0, 200.0))
        );

        let mut action = MoveShapeInstancesAction::new(layer_id, positions);

        // Execute
        action.execute(&mut document);

        // Verify position changed
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let obj = layer.get_object(&shape_instance_id).unwrap();
            assert_eq!(obj.transform.x, 150.0);
            assert_eq!(obj.transform.y, 200.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify position restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let obj = layer.get_object(&shape_instance_id).unwrap();
            assert_eq!(obj.transform.x, 50.0);
            assert_eq!(obj.transform.y, 50.0);
        }
    }
}
