//! Selection state management
//!
//! Tracks selected objects and shapes for editing operations.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Selection state for the editor
///
/// Maintains sets of selected objects and shapes.
/// This is separate from the document to make it easy to
/// pass around for UI rendering without needing mutable access.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Selection {
    /// Currently selected objects (instances)
    selected_objects: Vec<Uuid>,

    /// Currently selected shapes (definitions)
    selected_shapes: Vec<Uuid>,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self {
            selected_objects: Vec::new(),
            selected_shapes: Vec::new(),
        }
    }

    /// Add an object to the selection
    pub fn add_object(&mut self, id: Uuid) {
        if !self.selected_objects.contains(&id) {
            self.selected_objects.push(id);
        }
    }

    /// Add a shape to the selection
    pub fn add_shape(&mut self, id: Uuid) {
        if !self.selected_shapes.contains(&id) {
            self.selected_shapes.push(id);
        }
    }

    /// Remove an object from the selection
    pub fn remove_object(&mut self, id: &Uuid) {
        self.selected_objects.retain(|&x| x != *id);
    }

    /// Remove a shape from the selection
    pub fn remove_shape(&mut self, id: &Uuid) {
        self.selected_shapes.retain(|&x| x != *id);
    }

    /// Toggle an object's selection state
    pub fn toggle_object(&mut self, id: Uuid) {
        if self.contains_object(&id) {
            self.remove_object(&id);
        } else {
            self.add_object(id);
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

    /// Clear all selections
    pub fn clear(&mut self) {
        self.selected_objects.clear();
        self.selected_shapes.clear();
    }

    /// Clear only object selections
    pub fn clear_objects(&mut self) {
        self.selected_objects.clear();
    }

    /// Clear only shape selections
    pub fn clear_shapes(&mut self) {
        self.selected_shapes.clear();
    }

    /// Check if an object is selected
    pub fn contains_object(&self, id: &Uuid) -> bool {
        self.selected_objects.contains(id)
    }

    /// Check if a shape is selected
    pub fn contains_shape(&self, id: &Uuid) -> bool {
        self.selected_shapes.contains(id)
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.selected_objects.is_empty() && self.selected_shapes.is_empty()
    }

    /// Get the selected objects
    pub fn objects(&self) -> &[Uuid] {
        &self.selected_objects
    }

    /// Get the selected shapes
    pub fn shapes(&self) -> &[Uuid] {
        &self.selected_shapes
    }

    /// Get the number of selected objects
    pub fn object_count(&self) -> usize {
        self.selected_objects.len()
    }

    /// Get the number of selected shapes
    pub fn shape_count(&self) -> usize {
        self.selected_shapes.len()
    }

    /// Set selection to a single object (clears previous selection)
    pub fn select_only_object(&mut self, id: Uuid) {
        self.clear();
        self.add_object(id);
    }

    /// Set selection to a single shape (clears previous selection)
    pub fn select_only_shape(&mut self, id: Uuid) {
        self.clear();
        self.add_shape(id);
    }

    /// Set selection to multiple objects (clears previous selection)
    pub fn select_objects(&mut self, ids: &[Uuid]) {
        self.clear_objects();
        for &id in ids {
            self.add_object(id);
        }
    }

    /// Set selection to multiple shapes (clears previous selection)
    pub fn select_shapes(&mut self, ids: &[Uuid]) {
        self.clear_shapes();
        for &id in ids {
            self.add_shape(id);
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
        assert_eq!(selection.object_count(), 0);
        assert_eq!(selection.shape_count(), 0);
    }

    #[test]
    fn test_add_remove_objects() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_object(id1);
        assert_eq!(selection.object_count(), 1);
        assert!(selection.contains_object(&id1));

        selection.add_object(id2);
        assert_eq!(selection.object_count(), 2);

        selection.remove_object(&id1);
        assert_eq!(selection.object_count(), 1);
        assert!(!selection.contains_object(&id1));
        assert!(selection.contains_object(&id2));
    }

    #[test]
    fn test_toggle() {
        let mut selection = Selection::new();
        let id = Uuid::new_v4();

        selection.toggle_object(id);
        assert!(selection.contains_object(&id));

        selection.toggle_object(id);
        assert!(!selection.contains_object(&id));
    }

    #[test]
    fn test_select_only() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_object(id1);
        selection.add_object(id2);
        assert_eq!(selection.object_count(), 2);

        selection.select_only_object(id1);
        assert_eq!(selection.object_count(), 1);
        assert!(selection.contains_object(&id1));
        assert!(!selection.contains_object(&id2));
    }

    #[test]
    fn test_clear() {
        let mut selection = Selection::new();
        selection.add_object(Uuid::new_v4());
        selection.add_shape(Uuid::new_v4());

        assert!(!selection.is_empty());

        selection.clear();
        assert!(selection.is_empty());
    }
}
