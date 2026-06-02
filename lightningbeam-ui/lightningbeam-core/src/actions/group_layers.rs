//! Group layers action
//!
//! Creates a new GroupLayer containing the selected sibling layers.

use crate::action::Action;
use crate::document::Document;
use crate::layer::{AnyLayer, GroupLayer};
use uuid::Uuid;

/// Action that groups sibling layers into a new GroupLayer.
///
/// All layers must share the same parent (root or a specific GroupLayer).
pub struct GroupLayersAction {
    /// IDs of layers to group
    layer_ids: Vec<Uuid>,
    /// Parent group ID (None = document root)
    parent_group_id: Option<Uuid>,
    /// Pre-generated UUID for the new GroupLayer
    group_id: Uuid,
    /// Rollback: index where the group was inserted
    insert_index: Option<usize>,
    /// Rollback: (original_index, layer) pairs, sorted by index ascending
    removed_layers: Vec<(usize, AnyLayer)>,
}

impl GroupLayersAction {
    pub fn new(layer_ids: Vec<Uuid>, parent_group_id: Option<Uuid>, group_id: Uuid) -> Self {
        Self {
            layer_ids,
            parent_group_id,
            group_id,
            insert_index: None,
            removed_layers: Vec::new(),
        }
    }
}

/// Get a mutable reference to the children vec of the given parent.
fn get_parent_children<'a>(
    document: &'a mut Document,
    parent_group_id: Option<Uuid>,
) -> Result<&'a mut Vec<AnyLayer>, String> {
    match parent_group_id {
        None => Ok(&mut document.root.children),
        Some(id) => {
            let layer = document.root.get_child_mut(&id)
                .ok_or_else(|| format!("Parent group {} not found", id))?;
            match layer {
                AnyLayer::Group(g) => Ok(&mut g.children),
                _ => Err(format!("Layer {} is not a group", id)),
            }
        }
    }
}

impl Action for GroupLayersAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let children = get_parent_children(document, self.parent_group_id)?;

        // Find indices of all selected layers within the parent's children
        let mut indices: Vec<usize> = Vec::new();
        for layer_id in &self.layer_ids {
            if let Some(idx) = children.iter().position(|l| l.id() == *layer_id) {
                indices.push(idx);
            } else {
                return Err(format!("Layer {} not found in parent", layer_id));
            }
        }
        indices.sort();

        // The timeline displays layers in reverse order (highest index = visually on top).
        // Insert the group at the highest selected index so it appears where the
        // topmost visual layer was. After removing N layers before that position,
        // the actual insert index shifts down by the count of removed layers below it.
        let highest_index = *indices.last().unwrap();
        let removals_before_highest = indices.iter().filter(|&&i| i < highest_index).count();
        let insert_index = highest_index - removals_before_highest;
        self.insert_index = Some(insert_index);

        // Remove layers back-to-front to preserve indices
        self.removed_layers.clear();
        for &idx in indices.iter().rev() {
            let layer = children.remove(idx);
            self.removed_layers.push((idx, layer));
        }
        // Reverse so removed_layers is sorted by index ascending
        self.removed_layers.reverse();

        // Build the new GroupLayer with children in their original order
        let mut group = GroupLayer::new("Group");
        group.layer.id = self.group_id;
        group.expanded = false;
        for (_, layer) in &self.removed_layers {
            group.add_child(layer.clone());
        }

        // Insert the group at the computed position
        let children = get_parent_children(document, self.parent_group_id)?;
        children.insert(insert_index, AnyLayer::Group(group));

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(insert_index) = self.insert_index else {
            return Err("Cannot rollback: action was not executed".to_string());
        };

        // Remove the GroupLayer
        let children = get_parent_children(document, self.parent_group_id)?;
        children.remove(insert_index);

        // Re-insert original layers at their original indices (ascending order)
        for (idx, layer) in &self.removed_layers {
            let children = get_parent_children(document, self.parent_group_id)?;
            children.insert(*idx, layer.clone());
        }

        self.insert_index = None;

        Ok(())
    }

    fn description(&self) -> String {
        format!("Group {} layers", self.layer_ids.len())
    }
}
