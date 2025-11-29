//! Transform shape instances action
//!
//! Applies scale, rotation, and other transformations to shape instances with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action to transform multiple shape instances
pub struct TransformShapeInstancesAction {
    layer_id: Uuid,
    /// Map of shape instance ID to (old transform, new transform)
    shape_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformShapeInstancesAction {
    /// Create a new transform action
    pub fn new(
        layer_id: Uuid,
        shape_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            shape_instance_transforms,
        }
    }
}

impl Action for TransformShapeInstancesAction {
    fn execute(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_instance_id, (_old, new)) in &self.shape_instance_transforms {
                    vector_layer.modify_object_internal(shape_instance_id, |obj| {
                        obj.transform = new.clone();
                    });
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_instance_id, (old, _new)) in &self.shape_instance_transforms {
                    vector_layer.modify_object_internal(shape_instance_id, |obj| {
                        obj.transform = old.clone();
                    });
                }
            }
        }
    }

    fn description(&self) -> String {
        format!("Transform {} shape instance(s)", self.shape_instance_transforms.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::object::ShapeInstance;

    #[test]
    fn test_transform_single_shape_instance() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        // Create a shape instance with initial position
        let shape_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ShapeInstance::new(shape_id);
        instance.id = instance_id;
        instance.transform = Transform::with_position(10.0, 20.0);
        layer.add_object(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create transform action
        let old_transform = Transform::with_position(10.0, 20.0);
        let new_transform = Transform::with_position(100.0, 200.0);
        let mut transforms = HashMap::new();
        transforms.insert(instance_id, (old_transform, new_transform));

        let mut action = TransformShapeInstancesAction::new(layer_id, transforms);

        // Execute
        action.execute(&mut document);

        // Verify transform changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.x, 100.0);
            assert_eq!(obj.transform.y, 200.0);
        } else {
            panic!("Layer not found");
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.x, 10.0);
            assert_eq!(obj.transform.y, 20.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_shape_instance_rotation_scale() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ShapeInstance::new(shape_id);
        instance.id = instance_id;
        instance.transform.rotation = 0.0;
        instance.transform.scale_x = 1.0;
        instance.transform.scale_y = 1.0;
        layer.add_object(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create transform with rotation and scale
        let mut old_transform = Transform::new();
        let mut new_transform = Transform::new();
        new_transform.rotation = 90.0;
        new_transform.scale_x = 2.0;
        new_transform.scale_y = 0.5;

        let mut transforms = HashMap::new();
        transforms.insert(instance_id, (old_transform, new_transform));

        let mut action = TransformShapeInstancesAction::new(layer_id, transforms);
        action.execute(&mut document);

        // Verify
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.rotation, 90.0);
            assert_eq!(obj.transform.scale_x, 2.0);
            assert_eq!(obj.transform.scale_y, 0.5);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_multiple_shape_instances() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();
        let instance1_id = Uuid::new_v4();
        let instance2_id = Uuid::new_v4();

        let mut instance1 = ShapeInstance::new(shape_id);
        instance1.id = instance1_id;
        instance1.transform = Transform::with_position(0.0, 0.0);

        let mut instance2 = ShapeInstance::new(shape_id);
        instance2.id = instance2_id;
        instance2.transform = Transform::with_position(50.0, 50.0);

        layer.add_object(instance1);
        layer.add_object(instance2);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Transform both
        let mut transforms = HashMap::new();
        transforms.insert(
            instance1_id,
            (Transform::with_position(0.0, 0.0), Transform::with_position(10.0, 10.0)),
        );
        transforms.insert(
            instance2_id,
            (Transform::with_position(50.0, 50.0), Transform::with_position(60.0, 60.0)),
        );

        let mut action = TransformShapeInstancesAction::new(layer_id, transforms);
        action.execute(&mut document);

        // Verify both transformed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj1 = vl.get_object(&instance1_id).unwrap();
            assert_eq!(obj1.transform.x, 10.0);
            assert_eq!(obj1.transform.y, 10.0);

            let obj2 = vl.get_object(&instance2_id).unwrap();
            assert_eq!(obj2.transform.x, 60.0);
            assert_eq!(obj2.transform.y, 60.0);
        } else {
            panic!("Layer not found");
        }

        // Rollback
        action.rollback(&mut document);

        // Verify both restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj1 = vl.get_object(&instance1_id).unwrap();
            assert_eq!(obj1.transform.x, 0.0);
            assert_eq!(obj1.transform.y, 0.0);

            let obj2 = vl.get_object(&instance2_id).unwrap();
            assert_eq!(obj2.transform.x, 50.0);
            assert_eq!(obj2.transform.y, 50.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_nonexistent_layer() {
        let mut document = Document::new("Test");
        let fake_layer_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();

        let mut transforms = HashMap::new();
        transforms.insert(
            instance_id,
            (Transform::new(), Transform::with_position(10.0, 10.0)),
        );

        let mut action = TransformShapeInstancesAction::new(fake_layer_id, transforms);

        // Should not panic
        action.execute(&mut document);
        action.rollback(&mut document);
    }

    #[test]
    fn test_transform_nonexistent_instance() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let fake_instance_id = Uuid::new_v4();
        let mut transforms = HashMap::new();
        transforms.insert(
            fake_instance_id,
            (Transform::new(), Transform::with_position(10.0, 10.0)),
        );

        let mut action = TransformShapeInstancesAction::new(layer_id, transforms);

        // Should not panic - just silently skip nonexistent instance
        action.execute(&mut document);
        action.rollback(&mut document);
    }

    #[test]
    fn test_transform_on_non_vector_layer() {
        use crate::layer::AudioLayer;

        let mut document = Document::new("Test");
        let layer = AudioLayer::new("Audio Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Audio(layer));

        let instance_id = Uuid::new_v4();
        let mut transforms = HashMap::new();
        transforms.insert(
            instance_id,
            (Transform::new(), Transform::with_position(10.0, 10.0)),
        );

        let mut action = TransformShapeInstancesAction::new(layer_id, transforms);

        // Should not panic - action only operates on vector layers
        action.execute(&mut document);
        action.rollback(&mut document);
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();

        let mut transforms1 = HashMap::new();
        transforms1.insert(Uuid::new_v4(), (Transform::new(), Transform::new()));

        let action1 = TransformShapeInstancesAction::new(layer_id, transforms1);
        assert_eq!(action1.description(), "Transform 1 shape instance(s)");

        let mut transforms3 = HashMap::new();
        transforms3.insert(Uuid::new_v4(), (Transform::new(), Transform::new()));
        transforms3.insert(Uuid::new_v4(), (Transform::new(), Transform::new()));
        transforms3.insert(Uuid::new_v4(), (Transform::new(), Transform::new()));

        let action3 = TransformShapeInstancesAction::new(layer_id, transforms3);
        assert_eq!(action3.description(), "Transform 3 shape instance(s)");
    }
}
