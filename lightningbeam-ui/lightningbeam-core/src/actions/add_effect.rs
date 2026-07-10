//! Add effect action
//!
//! Handles adding a new effect instance (as a ClipInstance) to an effect layer.

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that adds an effect instance to an effect layer
///
/// Effect instances are represented as ClipInstance objects where clip_id
/// references an EffectDefinition.
pub struct AddEffectAction {
    /// ID of the layer to add the effect to
    layer_id: Uuid,
    /// The clip instance (effect) to add
    instance: Option<ClipInstance>,
    /// Index to insert at (None = append to end)
    insert_index: Option<usize>,
    /// ID of the created effect (set after execution)
    created_effect_id: Option<Uuid>,
}

impl AddEffectAction {
    /// Create a new add effect action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - ID of the effect layer to add the effect to
    /// * `instance` - The clip instance (referencing an effect definition) to add
    pub fn new(layer_id: Uuid, instance: ClipInstance) -> Self {
        Self {
            layer_id,
            instance: Some(instance),
            insert_index: None,
            created_effect_id: None,
        }
    }

    /// Create a new add effect action that inserts at a specific index
    ///
    /// # Arguments
    ///
    /// * `layer_id` - ID of the effect layer to add the effect to
    /// * `instance` - The clip instance (referencing an effect definition) to add
    /// * `index` - Index to insert at
    pub fn at_index(layer_id: Uuid, instance: ClipInstance, index: usize) -> Self {
        Self {
            layer_id,
            instance: Some(instance),
            insert_index: Some(index),
            created_effect_id: None,
        }
    }

    /// Get the ID of the created effect (after execution)
    pub fn created_effect_id(&self) -> Option<Uuid> {
        self.created_effect_id
    }

    /// Get the layer ID this effect was added to
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }
}

impl Action for AddEffectAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Take the instance (can only execute once without rollback)
        let instance = self.instance.take()
            .ok_or_else(|| "Effect already added (call rollback first)".to_string())?;

        // Store the instance ID
        let instance_id = instance.id;

        // Find the effect layer
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Ensure it's an effect layer
        let effect_layer = match layer {
            AnyLayer::Effect(ref mut el) => el,
            _ => return Err("Layer is not an effect layer".to_string()),
        };

        // Add or insert the effect
        match self.insert_index {
            Some(index) => {
                effect_layer.insert_clip_instance(index, instance);
            }
            None => {
                effect_layer.add_clip_instance(instance);
            }
        }

        self.created_effect_id = Some(instance_id);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let instance_id = self.created_effect_id
            .ok_or_else(|| "No effect to remove (not executed yet)".to_string())?;

        // Find the effect layer
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Ensure it's an effect layer
        let effect_layer = match layer {
            AnyLayer::Effect(ref mut el) => el,
            _ => return Err("Layer is not an effect layer".to_string()),
        };

        // Remove the instance and store it for potential re-execution
        let removed = effect_layer.remove_clip_instance(&instance_id)
            .ok_or_else(|| format!("Effect instance {} not found", instance_id))?;

        // Store the instance back for potential redo
        self.instance = Some(removed);
        self.created_effect_id = None;

        Ok(())
    }

    fn description(&self) -> String {
        "Add effect".to_string()
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
    fn test_add_effect() {
        let (mut document, layer_id, def) = create_test_setup();

        let instance = def.create_instance(daw_backend::Beats(0.0), daw_backend::Beats(10.0));
        let instance_id = instance.id;

        let mut action = AddEffectAction::new(layer_id, instance);
        action.execute(&mut document).unwrap();

        // Verify effect was added
        assert_eq!(action.created_effect_id(), Some(instance_id));

        let layer = document.get_layer(&layer_id).unwrap();
        if let AnyLayer::Effect(el) = layer {
            assert_eq!(el.clip_instances.len(), 1);
            assert_eq!(el.clip_instances[0].id, instance_id);
        } else {
            panic!("Expected effect layer");
        }
    }

    #[test]
    fn test_add_effect_rollback() {
        let (mut document, layer_id, def) = create_test_setup();

        let instance = def.create_instance(daw_backend::Beats(0.0), daw_backend::Beats(10.0));

        let mut action = AddEffectAction::new(layer_id, instance);
        action.execute(&mut document).unwrap();
        action.rollback(&mut document).unwrap();

        // Verify effect was removed
        let layer = document.get_layer(&layer_id).unwrap();
        if let AnyLayer::Effect(el) = layer {
            assert_eq!(el.clip_instances.len(), 0);
        } else {
            panic!("Expected effect layer");
        }
    }

    #[test]
    fn test_add_effect_at_index() {
        let (mut document, layer_id, def) = create_test_setup();

        // Add first effect
        let instance1 = def.create_instance(daw_backend::Beats(0.0), daw_backend::Beats(10.0));
        let id1 = instance1.id;
        let mut action1 = AddEffectAction::new(layer_id, instance1);
        action1.execute(&mut document).unwrap();

        // Add second effect
        let instance2 = def.create_instance(daw_backend::Beats(0.0), daw_backend::Beats(10.0));
        let id2 = instance2.id;
        let mut action2 = AddEffectAction::new(layer_id, instance2);
        action2.execute(&mut document).unwrap();

        // Insert third effect at index 1 (between first and second)
        let instance3 = def.create_instance(daw_backend::Beats(0.0), daw_backend::Beats(10.0));
        let id3 = instance3.id;
        let mut action3 = AddEffectAction::at_index(layer_id, instance3, 1);
        action3.execute(&mut document).unwrap();

        // Verify order: [id1, id3, id2]
        let layer = document.get_layer(&layer_id).unwrap();
        if let AnyLayer::Effect(el) = layer {
            assert_eq!(el.clip_instances.len(), 3);
            assert_eq!(el.clip_instances[0].id, id1);
            assert_eq!(el.clip_instances[1].id, id3);
            assert_eq!(el.clip_instances[2].id, id2);
        } else {
            panic!("Expected effect layer");
        }
    }
}
