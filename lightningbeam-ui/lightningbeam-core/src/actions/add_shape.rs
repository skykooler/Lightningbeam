//! Add shape action
//!
//! Handles adding a new shape and object to a vector layer.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::ShapeInstance;
use crate::shape::Shape;
use uuid::Uuid;

/// Action that adds a shape and object to a vector layer
///
/// This action creates both a Shape (the path/geometry) and an ShapeInstance
/// (the instance with transform). Both are added to the layer.
pub struct AddShapeAction {
    /// Layer ID to add the shape to
    layer_id: Uuid,

    /// The shape to add (contains path and styling)
    shape: Shape,

    /// The object to add (references the shape with transform)
    object: ShapeInstance,

    /// ID of the created shape (set after execution)
    created_shape_id: Option<Uuid>,

    /// ID of the created object (set after execution)
    created_object_id: Option<Uuid>,
}

impl AddShapeAction {
    /// Create a new add shape action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The layer to add the shape to
    /// * `shape` - The shape to add
    /// * `object` - The object instance referencing the shape
    pub fn new(layer_id: Uuid, shape: Shape, object: ShapeInstance) -> Self {
        Self {
            layer_id,
            shape,
            object,
            created_shape_id: None,
            created_object_id: None,
        }
    }
}

impl Action for AddShapeAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            // Add shape and object to the layer
            let shape_id = vector_layer.add_shape_internal(self.shape.clone());
            let object_id = vector_layer.add_object_internal(self.object.clone());

            // Store the IDs for rollback
            self.created_shape_id = Some(shape_id);
            self.created_object_id = Some(object_id);
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        // Remove the created shape and object if they exist
        if let (Some(shape_id), Some(object_id)) = (self.created_shape_id, self.created_object_id) {
            let layer = match document.get_layer_mut(&self.layer_id) {
                Some(l) => l,
                None => return,
            };

            if let AnyLayer::Vector(vector_layer) = layer {
                // Remove in reverse order: object first, then shape
                vector_layer.remove_object_internal(&object_id);
                vector_layer.remove_shape_internal(&shape_id);
            }

            // Clear the stored IDs
            self.created_shape_id = None;
            self.created_object_id = None;
        }
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
    use vello::kurbo::{Circle, Rect, Shape as KurboShape};

    #[test]
    fn test_add_shape_action_rectangle() {
        // Create a document with a vector layer
        let mut document = Document::new("Test");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create a rectangle shape
        let rect = Rect::new(0.0, 0.0, 100.0, 50.0);
        let path = rect.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));
        let object = ShapeInstance::new(shape.id).with_position(50.0, 50.0);

        // Create and execute action
        let mut action = AddShapeAction::new(layer_id, shape, object);
        action.execute(&mut document);

        // Verify shape and object were added
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            assert_eq!(layer.shapes.len(), 1);
            assert_eq!(layer.shape_instances.len(), 1);

            let added_object = &layer.shape_instances[0];
            assert_eq!(added_object.transform.x, 50.0);
            assert_eq!(added_object.transform.y, 50.0);
        } else {
            panic!("Layer not found or not a vector layer");
        }

        // Rollback
        action.rollback(&mut document);

        // Verify shape and object were removed
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            assert_eq!(layer.shapes.len(), 0);
            assert_eq!(layer.shape_instances.len(), 0);
        }
    }

    #[test]
    fn test_add_shape_action_circle() {
        let mut document = Document::new("Test");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create a circle shape
        let circle = Circle::new((50.0, 50.0), 25.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path)
            .with_fill(ShapeColor::rgb(0, 255, 0));
        let object = ShapeInstance::new(shape.id);

        let mut action = AddShapeAction::new(layer_id, shape, object);

        // Test description
        assert_eq!(action.description(), "Add shape");

        // Execute
        action.execute(&mut document);

        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            assert_eq!(layer.shapes.len(), 1);
            assert_eq!(layer.shape_instances.len(), 1);
        }
    }

    #[test]
    fn test_add_shape_action_multiple_execute() {
        let mut document = Document::new("Test");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        let rect = Rect::new(0.0, 0.0, 50.0, 50.0);
        let path = rect.to_path(0.1);
        let shape = Shape::new(path);
        let object = ShapeInstance::new(shape.id);

        let mut action = AddShapeAction::new(layer_id, shape, object);

        // Execute twice (should add duplicate)
        action.execute(&mut document);
        action.execute(&mut document);

        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            // Should have 2 shapes and 2 objects
            assert_eq!(layer.shapes.len(), 2);
            assert_eq!(layer.shape_instances.len(), 2);
        }
    }
}
