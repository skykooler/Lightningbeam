//! Transform clip instances action
//!
//! Handles spatial transformation (move, scale, rotate) of clip instances on the stage.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that transforms clip instances spatially on the stage
pub struct TransformClipInstancesAction {
    layer_id: Uuid,
    /// Map of clip instance ID to (old transform, new transform)
    clip_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformClipInstancesAction {
    pub fn new(
        layer_id: Uuid,
        clip_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            clip_instance_transforms,
        }
    }
}

impl Action for TransformClipInstancesAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        // Get mutable reference to clip_instances for this layer type
        let clip_instances = match layer {
            AnyLayer::Vector(vl) => &mut vl.clip_instances,
            AnyLayer::Audio(al) => &mut al.clip_instances,
            AnyLayer::Video(vl) => &mut vl.clip_instances,
        };

        // Apply new transforms
        for (clip_id, (_old, new)) in &self.clip_instance_transforms {
            if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                clip_instance.transform = new.clone();
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return,
        };

        // Get mutable reference to clip_instances for this layer type
        let clip_instances = match layer {
            AnyLayer::Vector(vl) => &mut vl.clip_instances,
            AnyLayer::Audio(al) => &mut al.clip_instances,
            AnyLayer::Video(vl) => &mut vl.clip_instances,
        };

        // Restore old transforms
        for (clip_id, (old, _new)) in &self.clip_instance_transforms {
            if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                clip_instance.transform = old.clone();
            }
        }
    }

    fn description(&self) -> String {
        format!(
            "Transform {} clip instance(s)",
            self.clip_instance_transforms.len()
        )
    }
}
