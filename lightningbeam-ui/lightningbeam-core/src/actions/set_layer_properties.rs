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
    use crate::layer::{AnyLayer, VectorLayer};

    #[test]
    fn test_set_volume() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial volume should be 1.0
        let layer_ref = document.root().find_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 1.0);

        // Create and execute action
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Volume(0.5));
        action.execute(&mut document);

        // Verify volume changed
        let layer_ref = document.root().find_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 0.5);

        // Rollback
        action.rollback(&mut document);

        // Verify volume restored
        let layer_ref = document.root().find_child(&layer_id).unwrap();
        assert_eq!(layer_ref.volume(), 1.0);
    }

    #[test]
    fn test_toggle_mute() {
        let mut document = Document::new("Test");
        let layer = VectorLayer::new("Test Layer");
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Initial state should be unmuted
        let layer_ref = document.root().find_child(&layer_id).unwrap();
        assert_eq!(layer_ref.muted(), false);

        // Mute
        let mut action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Muted(true));
        action.execute(&mut document);

        let layer_ref = document.root().find_child(&layer_id).unwrap();
        assert_eq!(layer_ref.muted(), true);

        // Unmute via rollback
        action.rollback(&mut document);

        let layer_ref = document.root().find_child(&layer_id).unwrap();
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
        assert_eq!(document.root().find_child(&id1).unwrap().soloed(), true);
        assert_eq!(document.root().find_child(&id2).unwrap().soloed(), true);

        // Rollback
        action.rollback(&mut document);

        // Verify both unsoloed
        assert_eq!(document.root().find_child(&id1).unwrap().soloed(), false);
        assert_eq!(document.root().find_child(&id2).unwrap().soloed(), false);
    }
}
