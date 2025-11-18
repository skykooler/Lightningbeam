//! Move objects action
//!
//! Handles moving one or more objects to new positions.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that moves objects to new positions
pub struct MoveObjectsAction {
    /// Layer ID containing the objects
    layer_id: Uuid,

    /// Map of object IDs to their old and new positions
    object_positions: HashMap<Uuid, (Point, Point)>, // (old_pos, new_pos)
}

impl MoveObjectsAction {
    /// Create a new move objects action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The layer containing the objects
    /// * `object_positions` - Map of object IDs to (old_position, new_position)
    pub fn new(layer_id: Uuid, object_positions: HashMap<Uuid, (Point, Point)>) -> Self {
        Self {
            layer_id,
            object_positions,
        }
    }
}

impl Action for MoveObjectsAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            for (object_id, (_old, new)) in &self.object_positions {
                vector_layer.modify_object_internal(object_id, |obj| {
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
            for (object_id, (old, _new)) in &self.object_positions {
                vector_layer.modify_object_internal(object_id, |obj| {
                    obj.transform.x = old.x;
                    obj.transform.y = old.y;
                });
            }
        }
    }

    fn description(&self) -> String {
        let count = self.object_positions.len();
        if count == 1 {
            "Move object".to_string()
        } else {
            format!("Move {} objects", count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::object::Object;
    use crate::shape::Shape;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_move_objects_action() {
        // Create a document with a test object
        let mut document = Document::new("Test");

        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path);
        let object = Object::new(shape.id).with_position(50.0, 50.0);

        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        let object_id = vector_layer.add_object(object);
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create move action
        let mut positions = HashMap::new();
        positions.insert(
            object_id,
            (Point::new(50.0, 50.0), Point::new(150.0, 200.0))
        );

        let mut action = MoveObjectsAction::new(layer_id, positions);

        // Execute
        action.execute(&mut document);

        // Verify position changed
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let obj = layer.get_object(&object_id).unwrap();
            assert_eq!(obj.transform.x, 150.0);
            assert_eq!(obj.transform.y, 200.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify position restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let obj = layer.get_object(&object_id).unwrap();
            assert_eq!(obj.transform.x, 50.0);
            assert_eq!(obj.transform.y, 50.0);
        }
    }
}
