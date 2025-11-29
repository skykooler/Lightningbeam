//! Set layer properties action
//!
//! Handles changing layer properties (volume, mute, solo, lock, opacity, visible)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::LayerTrait;
use uuid::Uuid;

/// Property that can be set on a layer
#[derive(Clone, Debug)]
pub enum LayerProperty {
    Volume(f64),
    Muted(bool),
    Soloed(bool),
    Locked(bool),
    Opacity(f64),
    Visible(bool),
}

/// Stored old value for rollback
#[derive(Clone, Debug)]
enum OldValue {
    Volume(f64),
    Muted(bool),
    Soloed(bool),
    Locked(bool),
    Opacity(f64),
    Visible(bool),
}

/// Action that sets a property on one or more layers
pub struct SetLayerPropertiesAction {
    /// IDs of layers to modify
    layer_ids: Vec<Uuid>,

    /// Property to set
    property: LayerProperty,

    /// Old values for rollback (stored after first execution)
    old_values: Vec<Option<OldValue>>,
}

impl SetLayerPropertiesAction {
    /// Create a new action to set a property on a single layer
    ///
    /// # Arguments
    ///
    /// * `layer_id` - ID of the layer to modify
    /// * `property` - Property to set
    pub fn new(layer_id: Uuid, property: LayerProperty) -> Self {
        Self {
            layer_ids: vec![layer_id],
            property,
            old_values: vec![None],
        }
    }

    /// Create a new action to set a property on multiple layers
    ///
    /// # Arguments
    ///
    /// * `layer_ids` - IDs of layers to modify
    /// * `property` - Property to set on all layers
    pub fn new_batch(layer_ids: Vec<Uuid>, property: LayerProperty) -> Self {
        let old_values = vec![None; layer_ids.len()];
        Self {
            layer_ids,
            property,
            old_values,
        }
    }
}

impl Action for SetLayerPropertiesAction {
    fn execute(&mut self, document: &mut Document) {
        for (i, &layer_id) in self.layer_ids.iter().enumerate() {
            // Find the layer in the document
            if let Some(layer) = document.root_mut().get_child_mut(&layer_id) {
                // Store old value if not already stored
                if self.old_values[i].is_none() {
                    self.old_values[i] = Some(match &self.property {
                        LayerProperty::Volume(_) => OldValue::Volume(layer.volume()),
                        LayerProperty::Muted(_) => OldValue::Muted(layer.muted()),
                        LayerProperty::Soloed(_) => OldValue::Soloed(layer.soloed()),
                        LayerProperty::Locked(_) => OldValue::Locked(layer.locked()),
                        LayerProperty::Opacity(_) => OldValue::Opacity(layer.opacity()),
                        LayerProperty::Visible(_) => OldValue::Visible(layer.visible()),
                    });
                }

                // Set new value
                match &self.property {
                    LayerProperty::Volume(v) => layer.set_volume(*v),
                    LayerProperty::Muted(m) => layer.set_muted(*m),
                    LayerProperty::Soloed(s) => layer.set_soloed(*s),
                    LayerProperty::Locked(l) => layer.set_locked(*l),
                    LayerProperty::Opacity(o) => layer.set_opacity(*o),
                    LayerProperty::Visible(v) => layer.set_visible(*v),
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        for (i, &layer_id) in self.layer_ids.iter().enumerate() {
            // Find the layer in the document
            if let Some(layer) = document.root_mut().get_child_mut(&layer_id) {
                // Restore old value if we have one
                if let Some(old_value) = &self.old_values[i] {
                    match old_value {
                        OldValue::Volume(v) => layer.set_volume(*v),
                        OldValue::Muted(m) => layer.set_muted(*m),
                        OldValue::Soloed(s) => layer.set_soloed(*s),
                        OldValue::Locked(l) => layer.set_locked(*l),
                        OldValue::Opacity(o) => layer.set_opacity(*o),
                        OldValue::Visible(v) => layer.set_visible(*v),
                    }
                }
            }
        }
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            LayerProperty::Volume(_) => "volume",
            LayerProperty::Muted(_) => "mute",
            LayerProperty::Soloed(_) => "solo",
            LayerProperty::Locked(_) => "lock",
            LayerProperty::Opacity(_) => "opacity",
            LayerProperty::Visible(_) => "visibility",
        };

        if self.layer_ids.len() == 1 {
            format!("Set layer {}", property_name)
        } else {
            format!("Set layer {} on {} layers", property_name, self.layer_ids.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{AnyLayer, LayerTrait, VectorLayer};

    #[test]
    fn test_set_volume() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial volume should be 1.0
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 1.0);

        // Create and execute action
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Volume(0.5));
        action.execute(&mut document);

        // Verify volume changed
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 0.5);

        // Rollback
        action.rollback(&mut document);

        // Verify volume restored
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 1.0);
    }

    #[test]
    fn test_toggle_mute() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial state should be unmuted
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.muted(), false);

        // Mute
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Muted(true));
        action.execute(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.muted(), true);

        // Unmute via rollback
        action.rollback(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.muted(), false);
    }

    #[test]
    fn test_batch_solo() {
        let mut document = Document::new("Test");
        let layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");
        let id1 = document.root_mut().add_child(AnyLayer::Vector(layer1));
        let id2 = document.root_mut().add_child(AnyLayer::Vector(layer2));

        // Solo both layers
        let mut action = SetLayerPropertiesAction::new_batch(
            vec![id1, id2],
            LayerProperty::Soloed(true),
        );
        action.execute(&mut document);

        // Verify both soloed
        assert_eq!(document.root.get_child(&id1).unwrap().soloed(), true);
        assert_eq!(document.root.get_child(&id2).unwrap().soloed(), true);

        // Rollback
        action.rollback(&mut document);

        // Verify both unsoloed
        assert_eq!(document.root.get_child(&id1).unwrap().soloed(), false);
        assert_eq!(document.root.get_child(&id2).unwrap().soloed(), false);
    }

    #[test]
    fn test_set_locked() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial state should be unlocked
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.locked(), false);

        // Lock
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Locked(true));
        action.execute(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.locked(), true);

        // Unlock via rollback
        action.rollback(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.locked(), false);
    }

    #[test]
    fn test_set_opacity() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial opacity should be 1.0
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.opacity(), 1.0);

        // Set opacity to 0.5
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Opacity(0.5));
        action.execute(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.opacity(), 0.5);

        // Rollback
        action.rollback(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.opacity(), 1.0);
    }

    #[test]
    fn test_set_visible() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial state should be visible
        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.visible(), true);

        // Hide
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Visible(false));
        action.execute(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.visible(), false);

        // Show via rollback
        action.rollback(&mut document);

        let layer_ref = document.root.get_child(&layer_id).unwrap();
        assert_eq!(layer_ref.visible(), true);
    }

    #[test]
    fn test_batch_lock() {
        let mut document = Document::new("Test");
        let layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");
        let id1 = document.root_mut().add_child(AnyLayer::Vector(layer1));
        let id2 = document.root_mut().add_child(AnyLayer::Vector(layer2));

        // Lock both layers
        let mut action = SetLayerPropertiesAction::new_batch(
            vec![id1, id2],
            LayerProperty::Locked(true),
        );
        action.execute(&mut document);

        // Verify both locked
        assert_eq!(document.root.get_child(&id1).unwrap().locked(), true);
        assert_eq!(document.root.get_child(&id2).unwrap().locked(), true);

        // Rollback
        action.rollback(&mut document);

        // Verify both unlocked
        assert_eq!(document.root.get_child(&id1).unwrap().locked(), false);
        assert_eq!(document.root.get_child(&id2).unwrap().locked(), false);
    }

    #[test]
    fn test_batch_opacity() {
        let mut document = Document::new("Test");
        let layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");
        let id1 = document.root_mut().add_child(AnyLayer::Vector(layer1));
        let id2 = document.root_mut().add_child(AnyLayer::Vector(layer2));

        // Set opacity on both layers
        let mut action = SetLayerPropertiesAction::new_batch(
            vec![id1, id2],
            LayerProperty::Opacity(0.25),
        );
        action.execute(&mut document);

        // Verify both have reduced opacity
        assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 0.25);
        assert_eq!(document.root.get_child(&id2).unwrap().opacity(), 0.25);

        // Rollback
        action.rollback(&mut document);

        // Verify both restored to 1.0
        assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 1.0);
        assert_eq!(document.root.get_child(&id2).unwrap().opacity(), 1.0);
    }

    #[test]
    fn test_description() {
        let layer_id = uuid::Uuid::new_v4();

        let action1 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Volume(0.5));
        assert_eq!(action1.description(), "Set layer volume");

        let action2 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Muted(true));
        assert_eq!(action2.description(), "Set layer mute");

        let action3 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Soloed(true));
        assert_eq!(action3.description(), "Set layer solo");

        let action4 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Locked(true));
        assert_eq!(action4.description(), "Set layer lock");

        let action5 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Opacity(0.5));
        assert_eq!(action5.description(), "Set layer opacity");

        let action6 = SetLayerPropertiesAction::new(layer_id, LayerProperty::Visible(false));
        assert_eq!(action6.description(), "Set layer visibility");

        // Test batch description
        let action_batch = SetLayerPropertiesAction::new_batch(
            vec![uuid::Uuid::new_v4(), uuid::Uuid::new_v4()],
            LayerProperty::Locked(true),
        );
        assert_eq!(action_batch.description(), "Set layer lock on 2 layers");
    }

    #[test]
    fn test_nonexistent_layer() {
        let mut document = Document::new("Test");
        let fake_id = uuid::Uuid::new_v4();

        let mut action = SetLayerPropertiesAction::new(fake_id, LayerProperty::Locked(true));

        // Should not panic
        action.execute(&mut document);
        action.rollback(&mut document);
    }
}
