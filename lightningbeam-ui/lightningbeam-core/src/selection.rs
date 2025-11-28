//! Selection state management
//!
//! Tracks selected shape instances, clip instances, and shapes for editing operations.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Selection state for the editor
///
/// Maintains sets of selected shape instances, clip instances, and shapes.
/// This is separate from the document to make it easy to
/// pass around for UI rendering without needing mutable access.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Selection {
    /// Currently selected shape instances
    selected_shape_instances: Vec<Uuid>,

    /// Currently selected shapes (definitions)
    selected_shapes: Vec<Uuid>,

    /// Currently selected clip instances
    selected_clip_instances: Vec<Uuid>,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self {
            selected_shape_instances: Vec::new(),
            selected_shapes: Vec::new(),
            selected_clip_instances: Vec::new(),
        }
    }

    /// Add a shape instance to the selection
    pub fn add_shape_instance(&mut self, id: Uuid) {
        if !self.selected_shape_instances.contains(&id) {
            self.selected_shape_instances.push(id);
        }
    }

    /// Add a shape definition to the selection
    pub fn add_shape(&mut self, id: Uuid) {
        if !self.selected_shapes.contains(&id) {
            self.selected_shapes.push(id);
        }
    }

    /// Remove a shape instance from the selection
    pub fn remove_shape_instance(&mut self, id: &Uuid) {
        self.selected_shape_instances.retain(|&x| x != *id);
    }

    /// Remove a shape definition from the selection
    pub fn remove_shape(&mut self, id: &Uuid) {
        self.selected_shapes.retain(|&x| x != *id);
    }

    /// Toggle a shape instance's selection state
    pub fn toggle_shape_instance(&mut self, id: Uuid) {
        if self.contains_shape_instance(&id) {
            self.remove_shape_instance(&id);
        } else {
            self.add_shape_instance(id);
        }
    }

    /// Toggle a shape's selection state
    pub fn toggle_shape(&mut self, id: Uuid) {
        if self.contains_shape(&id) {
            self.remove_shape(&id);
        } else {
            self.add_shape(id);
        }
    }

    /// Add a clip instance to the selection
    pub fn add_clip_instance(&mut self, id: Uuid) {
        if !self.selected_clip_instances.contains(&id) {
            self.selected_clip_instances.push(id);
        }
    }

    /// Remove a clip instance from the selection
    pub fn remove_clip_instance(&mut self, id: &Uuid) {
        self.selected_clip_instances.retain(|&x| x != *id);
    }

    /// Toggle a clip instance's selection state
    pub fn toggle_clip_instance(&mut self, id: Uuid) {
        if self.contains_clip_instance(&id) {
            self.remove_clip_instance(&id);
        } else {
            self.add_clip_instance(id);
        }
    }

    /// Clear all selections
    pub fn clear(&mut self) {
        self.selected_shape_instances.clear();
        self.selected_shapes.clear();
        self.selected_clip_instances.clear();
    }

    /// Clear only object selections
    pub fn clear_shape_instances(&mut self) {
        self.selected_shape_instances.clear();
    }

    /// Clear only shape selections
    pub fn clear_shapes(&mut self) {
        self.selected_shapes.clear();
    }

    /// Clear only clip instance selections
    pub fn clear_clip_instances(&mut self) {
        self.selected_clip_instances.clear();
    }

    /// Check if an object is selected
    pub fn contains_shape_instance(&self, id: &Uuid) -> bool {
        self.selected_shape_instances.contains(id)
    }

    /// Check if a shape is selected
    pub fn contains_shape(&self, id: &Uuid) -> bool {
        self.selected_shapes.contains(id)
    }

    /// Check if a clip instance is selected
    pub fn contains_clip_instance(&self, id: &Uuid) -> bool {
        self.selected_clip_instances.contains(id)
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.selected_shape_instances.is_empty()
            && self.selected_shapes.is_empty()
            && self.selected_clip_instances.is_empty()
    }

    /// Get the selected objects
    pub fn shape_instances(&self) -> &[Uuid] {
        &self.selected_shape_instances
    }

    /// Get the selected shapes
    pub fn shapes(&self) -> &[Uuid] {
        &self.selected_shapes
    }

    /// Get the number of selected objects
    pub fn shape_instance_count(&self) -> usize {
        self.selected_shape_instances.len()
    }

    /// Get the number of selected shapes
    pub fn shape_count(&self) -> usize {
        self.selected_shapes.len()
    }

    /// Get the selected clip instances
    pub fn clip_instances(&self) -> &[Uuid] {
        &self.selected_clip_instances
    }

    /// Get the number of selected clip instances
    pub fn clip_instance_count(&self) -> usize {
        self.selected_clip_instances.len()
    }

    /// Set selection to a single object (clears previous selection)
    pub fn select_only_shape_instance(&mut self, id: Uuid) {
        self.clear();
        self.add_shape_instance(id);
    }

    /// Set selection to a single shape (clears previous selection)
    pub fn select_only_shape(&mut self, id: Uuid) {
        self.clear();
        self.add_shape(id);
    }

    /// Set selection to a single clip instance (clears previous selection)
    pub fn select_only_clip_instance(&mut self, id: Uuid) {
        self.clear();
        self.add_clip_instance(id);
    }

    /// Set selection to multiple objects (clears previous selection)
    pub fn select_shape_instances(&mut self, ids: &[Uuid]) {
        self.clear_shape_instances();
        for &id in ids {
            self.add_shape_instance(id);
        }
    }

    /// Set selection to multiple shapes (clears previous selection)
    pub fn select_shapes(&mut self, ids: &[Uuid]) {
        self.clear_shapes();
        for &id in ids {
            self.add_shape(id);
        }
    }

    /// Set selection to multiple clip instances (clears previous selection)
    pub fn select_clip_instances(&mut self, ids: &[Uuid]) {
        self.clear_clip_instances();
        for &id in ids {
            self.add_clip_instance(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_creation() {
        let selection = Selection::new();
        assert!(selection.is_empty());
        assert_eq!(selection.shape_instance_count(), 0);
        assert_eq!(selection.shape_count(), 0);
    }

    #[test]
    fn test_add_remove_objects() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_shape_instance(id1);
        assert_eq!(selection.shape_instance_count(), 1);
        assert!(selection.contains_shape_instance(&id1));

        selection.add_shape_instance(id2);
        assert_eq!(selection.shape_instance_count(), 2);

        selection.remove_shape_instance(&id1);
        assert_eq!(selection.shape_instance_count(), 1);
        assert!(!selection.contains_shape_instance(&id1));
        assert!(selection.contains_shape_instance(&id2));
    }

    #[test]
    fn test_toggle() {
        let mut selection = Selection::new();
        let id = Uuid::new_v4();

        selection.toggle_shape_instance(id);
        assert!(selection.contains_shape_instance(&id));

        selection.toggle_shape_instance(id);
        assert!(!selection.contains_shape_instance(&id));
    }

    #[test]
    fn test_select_only() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_shape_instance(id1);
        selection.add_shape_instance(id2);
        assert_eq!(selection.shape_instance_count(), 2);

        selection.select_only_shape_instance(id1);
        assert_eq!(selection.shape_instance_count(), 1);
        assert!(selection.contains_shape_instance(&id1));
        assert!(!selection.contains_shape_instance(&id2));
    }

    #[test]
    fn test_clear() {
        let mut selection = Selection::new();
        selection.add_shape_instance(Uuid::new_v4());
        selection.add_shape(Uuid::new_v4());

        assert!(!selection.is_empty());

        selection.clear();
        assert!(selection.is_empty());
    }
}
