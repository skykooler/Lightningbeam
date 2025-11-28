//! Add layer action
//!
//! Handles adding a new layer to the document.

use crate::action::Action;
use crate::document::Document;
use crate::layer::{AnyLayer, VectorLayer};
use uuid::Uuid;

/// Action that adds a new layer to the document
pub struct AddLayerAction {
    /// The layer to add
    layer: AnyLayer,

    /// ID of the created layer (set after execution)
    created_layer_id: Option<Uuid>,
}

impl AddLayerAction {
    /// Create a new add layer action with a vector layer
    ///
    /// # Arguments
    ///
    /// * `name` - The name for the new layer
    pub fn new_vector(name: impl Into<String>) -> Self {
        let layer = VectorLayer::new(name);
        Self {
            layer: AnyLayer::Vector(layer),
            created_layer_id: None,
        }
    }

    /// Create a new add layer action with any layer type
    ///
    /// # Arguments
    ///
    /// * `layer` - The layer to add
    pub fn new(layer: AnyLayer) -> Self {
        Self {
            layer,
            created_layer_id: None,
        }
    }

    /// Get the ID of the created layer (after execution)
    pub fn created_layer_id(&self) -> Option<Uuid> {
        self.created_layer_id
    }
}

impl Action for AddLayerAction {
    fn execute(&mut self, document: &mut Document) {
        // Add layer to the document's root
        let layer_id = document.root_mut().add_child(self.layer.clone());

        // Store the ID for rollback
        self.created_layer_id = Some(layer_id);
    }

    fn rollback(&mut self, document: &mut Document) {
        // Remove the created layer if it exists
        if let Some(layer_id) = self.created_layer_id {
            document.root_mut().remove_child(&layer_id);

            // Clear the stored ID
            self.created_layer_id = None;
        }
    }

    fn description(&self) -> String {
        match &self.layer {
            AnyLayer::Vector(_) => "Add vector layer",
            AnyLayer::Audio(_) => "Add audio layer",
            AnyLayer::Video(_) => "Add video layer",
        }
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_vector_layer() {
        let mut document = Document::new("Test");
        assert_eq!(document.root.children.len(), 0);

        // Create and execute action
        let mut action = AddLayerAction::new_vector("New Layer");
        action.execute(&mut document);

        // Verify layer was added
        assert_eq!(document.root.children.len(), 1);
        let layer = &document.root.children[0];
        assert_eq!(layer.layer().name, "New Layer");
        assert!(matches!(layer, AnyLayer::Vector(_)));

        // Rollback
        action.rollback(&mut document);

        // Verify layer was removed
        assert_eq!(document.root.children.len(), 0);
    }

    #[test]
    fn test_add_layer_description() {
        let action = AddLayerAction::new_vector("Test");
        assert_eq!(action.description(), "Add vector layer");
    }

    #[test]
    fn test_add_multiple_layers() {
        let mut document = Document::new("Test");

        let mut action1 = AddLayerAction::new_vector("Layer 1");
        let mut action2 = AddLayerAction::new_vector("Layer 2");

        action1.execute(&mut document);
        action2.execute(&mut document);

        assert_eq!(document.root.children.len(), 2);
        assert_eq!(document.root.children[0].layer().name, "Layer 1");
        assert_eq!(document.root.children[1].layer().name, "Layer 2");
    }
}
