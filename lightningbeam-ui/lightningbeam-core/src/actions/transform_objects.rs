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
