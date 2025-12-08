//! Remove effect action
//!
//! Handles removing an effect instance (ClipInstance) from an effect layer.

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that removes an effect instance from an effect layer
pub struct RemoveEffectAction {
    /// ID of the layer containing the effect
    layer_id: Uuid,
    /// ID of the effect instance to remove
    instance_id: Uuid,
    /// The removed instance (stored for undo)
    removed_instance: Option<ClipInstance>,
    /// Index where the instance was (for proper undo position)
    removed_index: Option<usize>,
}

impl RemoveEffectAction {
    /// Create a new remove effect action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - ID of the effect layer containing the effect
    /// * `instance_id` - ID of the clip instance to remove
    pub fn new(layer_id: Uuid, instance_id: Uuid) -> Self {
        Self {
            layer_id,
            instance_id,
            removed_instance: None,
            removed_index: None,
        }
    }

    /// Get the layer ID
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }

    /// Get the instance ID that was/will be removed
    pub fn instance_id(&self) -> Uuid {
        self.instance_id
    }
}

impl Action for RemoveEffectAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Find the effect layer
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Ensure it's an effect layer
        let effect_layer = match layer {
            AnyLayer::Effect(ref mut el) => el,
            _ => return Err("Layer is not an effect layer".to_string()),
        };

        // Find the index before removing
        let index = effect_layer.clip_instance_index(&self.instance_id)
            .ok_or_else(|| format!("Effect instance {} not found", self.instance_id))?;

        // Remove the instance
        let removed = effect_layer.remove_clip_instance(&self.instance_id)
            .ok_or_else(|| format!("Effect instance {} not found", self.instance_id))?;

        // Store for undo
        self.removed_instance = Some(removed);
        self.removed_index = Some(index);

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = self.removed_instance.take()
            .ok_or_else(|| "No instance to restore (not executed yet)".to_string())?;
        let index = self.removed_index
            .ok_or_else(|| "No index stored (not executed yet)".to_string())?;

        // Find the effect layer
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Ensure it's an effect layer
        let effect_layer = match layer {
            AnyLayer::Effect(ref mut el) => el,
            _ => return Err("Layer is not an effect layer".to_string()),
        };

        // Insert the instance back at its original position
        effect_layer.insert_clip_instance(index, instance);

        self.removed_index = None;

        Ok(())
    }

    fn description(&self) -> String {
        "Remove effect".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectCategory, EffectDefinition, EffectParameterDef};
    use crate::effect_layer::EffectLayer;
    use crate::layer::AnyLayer;

    fn create_test_setup() -> (Document, Uuid, EffectDefinition) {
        let mut document = Document::new("Test");

        // Create effect layer
        let effect_layer = EffectLayer::new("Effects");
        let layer_id = effect_layer.layer.id;
        document.root_mut().add_child(AnyLayer::Effect(effect_layer));

        // Create effect definition
        let def = EffectDefinition::new(
            "Test Effect",
            EffectCategory::Color,
            "// shader code",
            vec![EffectParameterDef::float_range("intensity", "Intensity", 1.0, 0.0, 2.0)],
        );

        (document, layer_id, def)
    }

    #[test]
    fn test_remove_effect() {
        let (mut document, layer_id, def) = create_test_setup();

        // Add an effect first
        let instance = def.create_instance(0.0, 10.0);
        let instance_id = instance.id;

        if let Some(AnyLayer::Effect(el)) = document.get_layer_mut(&layer_id) {
            el.add_clip_instance(instance);
        }

        // Verify effect exists
        if let Some(AnyLayer::Effect(el)) = document.get_layer(&layer_id) {
            assert_eq!(el.clip_instances.len(), 1);
        }

        // Remove the effect
        let mut action = RemoveEffectAction::new(layer_id, instance_id);
        action.execute(&mut document).unwrap();

        // Verify effect was removed
        if let Some(AnyLayer::Effect(el)) = document.get_layer(&layer_id) {
            assert_eq!(el.clip_instances.len(), 0);
        }
    }

    #[test]
    fn test_remove_effect_rollback() {
        let (mut document, layer_id, def) = create_test_setup();

        // Add an effect first
        let instance = def.create_instance(0.0, 10.0);
        let instance_id = instance.id;

        if let Some(AnyLayer::Effect(el)) = document.get_layer_mut(&layer_id) {
            el.add_clip_instance(instance);
        }

        // Remove and rollback
        let mut action = RemoveEffectAction::new(layer_id, instance_id);
        action.execute(&mut document).unwrap();
        action.rollback(&mut document).unwrap();

        // Verify effect was restored
        if let Some(AnyLayer::Effect(el)) = document.get_layer(&layer_id) {
            assert_eq!(el.clip_instances.len(), 1);
            assert_eq!(el.clip_instances[0].id, instance_id);
        }
    }

    #[test]
    fn test_remove_preserves_order() {
        let (mut document, layer_id, def) = create_test_setup();

        // Add three effects
        let instance1 = def.create_instance(0.0, 10.0);
        let id1 = instance1.id;
        let instance2 = def.create_instance(0.0, 10.0);
        let id2 = instance2.id;
        let instance3 = def.create_instance(0.0, 10.0);
        let id3 = instance3.id;

        if let Some(AnyLayer::Effect(el)) = document.get_layer_mut(&layer_id) {
            el.add_clip_instance(instance1);
            el.add_clip_instance(instance2);
            el.add_clip_instance(instance3);
        }

        // Remove middle effect
        let mut action = RemoveEffectAction::new(layer_id, id2);
        action.execute(&mut document).unwrap();

        // Verify order: [id1, id3]
        if let Some(AnyLayer::Effect(el)) = document.get_layer(&layer_id) {
            assert_eq!(el.clip_instances.len(), 2);
            assert_eq!(el.clip_instances[0].id, id1);
            assert_eq!(el.clip_instances[1].id, id3);
        }

        // Rollback - effect should be restored at index 1
        action.rollback(&mut document).unwrap();

        // Verify order: [id1, id2, id3]
        if let Some(AnyLayer::Effect(el)) = document.get_layer(&layer_id) {
            assert_eq!(el.clip_instances.len(), 3);
            assert_eq!(el.clip_instances[0].id, id1);
            assert_eq!(el.clip_instances[1].id, id2);
            assert_eq!(el.clip_instances[2].id, id3);
        }
    }
}
