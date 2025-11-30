//! Set shape instance properties action
//!
//! Handles changing individual properties on shape instances (position, rotation, scale, etc.)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Individual property change for a shape instance
#[derive(Clone, Debug)]
pub enum InstancePropertyChange {
    X(f64),
    Y(f64),
    Rotation(f64),
    ScaleX(f64),
    ScaleY(f64),
    SkewX(f64),
    SkewY(f64),
    Opacity(f64),
}

impl InstancePropertyChange {
    /// Extract the f64 value from any variant
    fn value(&self) -> f64 {
        match self {
            InstancePropertyChange::X(v) => *v,
            InstancePropertyChange::Y(v) => *v,
            InstancePropertyChange::Rotation(v) => *v,
            InstancePropertyChange::ScaleX(v) => *v,
            InstancePropertyChange::ScaleY(v) => *v,
            InstancePropertyChange::SkewX(v) => *v,
            InstancePropertyChange::SkewY(v) => *v,
            InstancePropertyChange::Opacity(v) => *v,
        }
    }
}

/// Action that sets a property on one or more shape instances
pub struct SetInstancePropertiesAction {
    /// Layer containing the instances
    layer_id: Uuid,

    /// Instance IDs to modify and their old values
    instance_changes: Vec<(Uuid, Option<f64>)>,

    /// Property to change
    property: InstancePropertyChange,
}

impl SetInstancePropertiesAction {
    /// Create a new action to set a property on a single instance
    pub fn new(layer_id: Uuid, instance_id: Uuid, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            instance_changes: vec![(instance_id, None)],
            property,
        }
    }

    /// Create a new action to set a property on multiple instances
    pub fn new_batch(layer_id: Uuid, instance_ids: Vec<Uuid>, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            instance_changes: instance_ids.into_iter().map(|id| (id, None)).collect(),
            property,
        }
    }

    fn get_instance_value(&self, document: &Document, instance_id: &Uuid) -> Option<f64> {
        if let Some(layer) = document.get_layer(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                if let Some(instance) = vector_layer.get_object(instance_id) {
                    return Some(match &self.property {
                        InstancePropertyChange::X(_) => instance.transform.x,
                        InstancePropertyChange::Y(_) => instance.transform.y,
                        InstancePropertyChange::Rotation(_) => instance.transform.rotation,
                        InstancePropertyChange::ScaleX(_) => instance.transform.scale_x,
                        InstancePropertyChange::ScaleY(_) => instance.transform.scale_y,
                        InstancePropertyChange::SkewX(_) => instance.transform.skew_x,
                        InstancePropertyChange::SkewY(_) => instance.transform.skew_y,
                        InstancePropertyChange::Opacity(_) => instance.opacity,
                    });
                }
            }
        }
        None
    }

    fn apply_to_instance(&self, document: &mut Document, instance_id: &Uuid, value: f64) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                vector_layer.modify_object_internal(instance_id, |instance| {
                    match &self.property {
                        InstancePropertyChange::X(_) => instance.transform.x = value,
                        InstancePropertyChange::Y(_) => instance.transform.y = value,
                        InstancePropertyChange::Rotation(_) => instance.transform.rotation = value,
                        InstancePropertyChange::ScaleX(_) => instance.transform.scale_x = value,
                        InstancePropertyChange::ScaleY(_) => instance.transform.scale_y = value,
                        InstancePropertyChange::SkewX(_) => instance.transform.skew_x = value,
                        InstancePropertyChange::SkewY(_) => instance.transform.skew_y = value,
                        InstancePropertyChange::Opacity(_) => instance.opacity = value,
                    }
                });
            }
        }
    }
}

impl Action for SetInstancePropertiesAction {
    fn execute(&mut self, document: &mut Document) {
        let new_value = self.property.value();
        let layer_id = self.layer_id;

        // First pass: collect old values for instances that don't have them yet
        for (instance_id, old_value) in &mut self.instance_changes {
            if old_value.is_none() {
                // Get old value inline to avoid borrow issues
                if let Some(layer) = document.get_layer(&layer_id) {
                    if let AnyLayer::Vector(vector_layer) = layer {
                        if let Some(instance) = vector_layer.get_object(instance_id) {
                            *old_value = Some(match &self.property {
                                InstancePropertyChange::X(_) => instance.transform.x,
                                InstancePropertyChange::Y(_) => instance.transform.y,
                                InstancePropertyChange::Rotation(_) => instance.transform.rotation,
                                InstancePropertyChange::ScaleX(_) => instance.transform.scale_x,
                                InstancePropertyChange::ScaleY(_) => instance.transform.scale_y,
                                InstancePropertyChange::SkewX(_) => instance.transform.skew_x,
                                InstancePropertyChange::SkewY(_) => instance.transform.skew_y,
                                InstancePropertyChange::Opacity(_) => instance.opacity,
                            });
                        }
                    }
                }
            }
        }

        // Second pass: apply new values
        for (instance_id, _) in &self.instance_changes {
            self.apply_to_instance(document, instance_id, new_value);
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        for (instance_id, old_value) in &self.instance_changes {
            if let Some(value) = old_value {
                self.apply_to_instance(document, instance_id, *value);
            }
        }
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            InstancePropertyChange::X(_) => "X position",
            InstancePropertyChange::Y(_) => "Y position",
            InstancePropertyChange::Rotation(_) => "rotation",
            InstancePropertyChange::ScaleX(_) => "scale X",
            InstancePropertyChange::ScaleY(_) => "scale Y",
            InstancePropertyChange::SkewX(_) => "skew X",
            InstancePropertyChange::SkewY(_) => "skew Y",
            InstancePropertyChange::Opacity(_) => "opacity",
        };

        if self.instance_changes.len() == 1 {
            format!("Set {}", property_name)
        } else {
            format!("Set {} on {} objects", property_name, self.instance_changes.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::object::{ShapeInstance, Transform};

    #[test]
    fn test_set_x_position() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();
        let mut instance = ShapeInstance::new(shape_id);
        let instance_id = instance.id;
        instance.transform = Transform::with_position(10.0, 20.0);
        layer.add_object(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create and execute action
        let mut action = SetInstancePropertiesAction::new(
            layer_id,
            instance_id,
            InstancePropertyChange::X(50.0),
        );
        action.execute(&mut document);

        // Verify position changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.x, 50.0);
            assert_eq!(obj.transform.y, 20.0); // Y unchanged
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.x, 10.0);
        }
    }

    #[test]
    fn test_set_rotation() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();
        let mut instance = ShapeInstance::new(shape_id);
        let instance_id = instance.id;
        instance.transform.rotation = 0.0;
        layer.add_object(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create and execute action
        let mut action = SetInstancePropertiesAction::new(
            layer_id,
            instance_id,
            InstancePropertyChange::Rotation(45.0),
        );
        action.execute(&mut document);

        // Verify rotation changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.rotation, 45.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.transform.rotation, 0.0);
        }
    }

    #[test]
    fn test_set_opacity() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();
        let mut instance = ShapeInstance::new(shape_id);
        let instance_id = instance.id;
        instance.opacity = 1.0;
        layer.add_object(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create and execute action
        let mut action = SetInstancePropertiesAction::new(
            layer_id,
            instance_id,
            InstancePropertyChange::Opacity(0.5),
        );
        action.execute(&mut document);

        // Verify opacity changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.opacity, 0.5);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let obj = vl.get_object(&instance_id).unwrap();
            assert_eq!(obj.opacity, 1.0);
        }
    }

    #[test]
    fn test_batch_set_scale() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape_id = Uuid::new_v4();

        let mut instance1 = ShapeInstance::new(shape_id);
        let instance1_id = instance1.id;
        instance1.transform.scale_x = 1.0;

        let mut instance2 = ShapeInstance::new(shape_id);
        let instance2_id = instance2.id;
        instance2.transform.scale_x = 1.0;

        layer.add_object(instance1);
        layer.add_object(instance2);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create and execute batch action
        let mut action = SetInstancePropertiesAction::new_batch(
            layer_id,
            vec![instance1_id, instance2_id],
            InstancePropertyChange::ScaleX(2.0),
        );
        action.execute(&mut document);

        // Verify both changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            assert_eq!(vl.get_object(&instance1_id).unwrap().transform.scale_x, 2.0);
            assert_eq!(vl.get_object(&instance2_id).unwrap().transform.scale_x, 2.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify both restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            assert_eq!(vl.get_object(&instance1_id).unwrap().transform.scale_x, 1.0);
            assert_eq!(vl.get_object(&instance2_id).unwrap().transform.scale_x, 1.0);
        }
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();

        let action1 = SetInstancePropertiesAction::new(
            layer_id,
            instance_id,
            InstancePropertyChange::X(0.0),
        );
        assert_eq!(action1.description(), "Set X position");

        let action2 = SetInstancePropertiesAction::new(
            layer_id,
            instance_id,
            InstancePropertyChange::Rotation(0.0),
        );
        assert_eq!(action2.description(), "Set rotation");

        let action3 = SetInstancePropertiesAction::new_batch(
            layer_id,
            vec![Uuid::new_v4(), Uuid::new_v4()],
            InstancePropertyChange::Opacity(1.0),
        );
        assert_eq!(action3.description(), "Set opacity on 2 objects");
    }
}
