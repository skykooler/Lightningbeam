//! Move clip instances action
//!
//! Handles moving one or more clip instances along the timeline.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that moves clip instances to new timeline positions
pub struct MoveClipInstancesAction {
    /// Map of layer IDs to vectors of (clip_instance_id, old_timeline_start, new_timeline_start)
    layer_moves: HashMap<Uuid, Vec<(Uuid, f64, f64)>>,
}

impl MoveClipInstancesAction {
    /// Create a new move clip instances action
    ///
    /// # Arguments
    ///
    /// * `layer_moves` - Map of layer IDs to vectors of (clip_instance_id, old_timeline_start, new_timeline_start)
    pub fn new(layer_moves: HashMap<Uuid, Vec<(Uuid, f64, f64)>>) -> Self {
        Self { layer_moves }
    }
}

impl Action for MoveClipInstancesAction {
    fn execute(&mut self, document: &mut Document) {
        for (layer_id, moves) in &self.layer_moves {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Update timeline_start for each clip instance
            for (clip_id, _old, new) in moves {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    clip_instance.timeline_start = *new;
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        for (layer_id, moves) in &self.layer_moves {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Restore original timeline_start for each clip instance
            for (clip_id, old, _new) in moves {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    clip_instance.timeline_start = *old;
                }
            }
        }
    }

    fn description(&self) -> String {
        let total_count: usize = self.layer_moves.values().map(|v| v.len()).sum();
        if total_count == 1 {
            "Move clip instance".to_string()
        } else {
            format!("Move {} clip instances", total_count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::ClipInstance;
    use crate::layer::VectorLayer;

    #[test]
    fn test_move_clip_instances_action() {
        // Create a document with a test clip instance
        let mut document = Document::new("Test");

        // Create a clip ID (no Clip definition needed for ClipInstance)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.timeline_start = 1.0; // Start at 1 second
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create move action: move from 1.0 to 5.0 seconds
        let mut layer_moves = HashMap::new();
        layer_moves.insert(layer_id, vec![(instance_id, 1.0, 5.0)]);

        let mut action = MoveClipInstancesAction::new(layer_moves);

        // Execute
        action.execute(&mut document);

        // Verify position changed
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.timeline_start, 5.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify position restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.timeline_start, 1.0);
        }
    }
}
