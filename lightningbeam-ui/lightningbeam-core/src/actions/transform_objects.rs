//! Transform objects action
//!
//! Applies scale, rotation, and other transformations to objects with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action to transform multiple objects
pub struct TransformObjectsAction {
    layer_id: Uuid,
    /// Map of object ID to (old transform, new transform)
    object_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformObjectsAction {
    /// Create a new transform action
    pub fn new(
        layer_id: Uuid,
        object_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            object_transforms,
        }
    }
}

impl Action for TransformObjectsAction {
    fn execute(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (object_id, (_old, new)) in &self.object_transforms {
                    vector_layer.modify_object_internal(object_id, |obj| {
                        obj.transform = new.clone();
                    });
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (object_id, (old, _new)) in &self.object_transforms {
                    vector_layer.modify_object_internal(object_id, |obj| {
                        obj.transform = old.clone();
                    });
                }
            }
        }
    }

    fn description(&self) -> String {
        format!("Transform {} object(s)", self.object_transforms.len())
    }
}
