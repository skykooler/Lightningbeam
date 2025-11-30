//! Add clip instance action
//!
//! Handles adding a clip instance to a layer.

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that adds a clip instance to a layer
pub struct AddClipInstanceAction {
    /// The target layer ID
    layer_id: Uuid,

    /// The clip instance to add
    clip_instance: ClipInstance,

    /// Whether the action has been executed (for rollback)
    executed: bool,
}

impl AddClipInstanceAction {
    /// Create a new add clip instance action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The ID of the layer to add the clip instance to
    /// * `clip_instance` - The clip instance to add
    pub fn new(layer_id: Uuid, clip_instance: ClipInstance) -> Self {
        Self {
            layer_id,
            clip_instance,
            executed: false,
        }
    }

    /// Get the ID of the clip instance that will be/was added
    pub fn clip_instance_id(&self) -> Uuid {
        self.clip_instance.id
    }

    /// Get the layer ID this action targets
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }
}

impl Action for AddClipInstanceAction {
    fn execute(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            match layer {
                AnyLayer::Vector(vector_layer) => {
                    vector_layer.clip_instances.push(self.clip_instance.clone());
                }
                AnyLayer::Audio(audio_layer) => {
                    audio_layer.clip_instances.push(self.clip_instance.clone());
                }
                AnyLayer::Video(video_layer) => {
                    video_layer.clip_instances.push(self.clip_instance.clone());
                }
            }
            self.executed = true;
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if !self.executed {
            return;
        }

        let instance_id = self.clip_instance.id;

        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            match layer {
                AnyLayer::Vector(vector_layer) => {
                    vector_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
                AnyLayer::Audio(audio_layer) => {
                    audio_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
                AnyLayer::Video(video_layer) => {
                    video_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
            }
            self.executed = false;
        }
    }

    fn description(&self) -> String {
        "Add clip instance".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;

    #[test]
    fn test_add_clip_instance_to_vector_layer() {
        let mut document = Document::new("Test");

        // Add a layer
        let layer = VectorLayer::new("Test Layer");
        let layer_id = layer.layer.id;
        document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create a clip instance (using a fake clip_id since we're just testing the action)
        let clip_id = Uuid::new_v4();
        let clip_instance = ClipInstance::new(clip_id);
        let instance_id = clip_instance.id;

        // Execute action
        let mut action = AddClipInstanceAction::new(layer_id, clip_instance);
        action.execute(&mut document);

        // Verify clip instance was added
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer(&layer_id) {
            assert_eq!(vector_layer.clip_instances.len(), 1);
            assert_eq!(vector_layer.clip_instances[0].id, instance_id);
        } else {
            panic!("Layer not found");
        }

        // Rollback
        action.rollback(&mut document);

        // Verify clip instance was removed
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer(&layer_id) {
            assert_eq!(vector_layer.clip_instances.len(), 0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_add_clip_instance_description() {
        let action = AddClipInstanceAction::new(Uuid::new_v4(), ClipInstance::new(Uuid::new_v4()));
        assert_eq!(action.description(), "Add clip instance");
    }
}
