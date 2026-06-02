use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that moves one or more layers to a new position, possibly changing their parent group.
/// All layers are inserted contiguously into the same target parent.
/// Handles batch moves atomically: removes all, then inserts all, so indices stay consistent.
pub struct MoveLayerAction {
    /// (layer_id, old_parent_id) for each layer to move, in visual order (top to bottom)
    layers: Vec<(Uuid, Option<Uuid>)>,
    new_parent_id: Option<Uuid>,
    /// Insertion index in the new parent's children vec AFTER all dragged layers have been removed
    new_base_index: usize,
    /// Stored during execute for rollback: (layer, old_parent_id, old_index_in_parent)
    removed: Vec<(AnyLayer, Option<Uuid>, usize)>,
}

impl MoveLayerAction {
    pub fn new(
        layers: Vec<(Uuid, Option<Uuid>)>,
        new_parent_id: Option<Uuid>,
        new_base_index: usize,
    ) -> Self {
        Self {
            layers,
            new_parent_id,
            new_base_index,
            removed: Vec::new(),
        }
    }
}

fn get_parent_children(
    document: &mut Document,
    parent_id: Option<Uuid>,
) -> Result<&mut Vec<AnyLayer>, String> {
    match parent_id {
        None => Ok(&mut document.root.children),
        Some(id) => {
            let layer = document
                .root
                .get_child_mut(&id)
                .ok_or_else(|| format!("Parent group {} not found", id))?;
            match layer {
                AnyLayer::Group(g) => Ok(&mut g.children),
                _ => Err(format!("Layer {} is not a group", id)),
            }
        }
    }
}

impl Action for MoveLayerAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.removed.clear();

        // Phase 1: Remove all layers from their old parents.
        // Group removals by parent, then remove back-to-front within each parent.
        // Collect (layer_id, old_parent_id) with their current index.
        let mut removals: Vec<(Uuid, Option<Uuid>, usize)> = Vec::new();
        for (layer_id, old_parent_id) in &self.layers {
            let children = get_parent_children(document, *old_parent_id)?;
            let idx = children.iter().position(|l| l.id() == *layer_id)
                .ok_or_else(|| format!("Layer {} not found in parent", layer_id))?;
            removals.push((*layer_id, *old_parent_id, idx));
        }

        // Sort by (parent, index) descending so we remove back-to-front
        removals.sort_by(|a, b| {
            a.1.cmp(&b.1).then(b.2.cmp(&a.2))
        });

        let mut removed_layers: Vec<(Uuid, AnyLayer, Option<Uuid>, usize)> = Vec::new();
        for (layer_id, old_parent_id, idx) in &removals {
            let children = get_parent_children(document, *old_parent_id)?;
            let layer = children.remove(*idx);
            removed_layers.push((*layer_id, layer, *old_parent_id, *idx));
        }

        // Phase 2: Insert all at new parent, in visual order (self.layers order).
        // self.new_base_index is the index in the post-removal children vec.
        let new_children = get_parent_children(document, self.new_parent_id)?;
        let base = self.new_base_index.min(new_children.len());

        // Insert in forward visual order, all at `base`. Each insert pushes the previous
        // one to a higher children index. Since the timeline displays children in reverse,
        // a higher children index = visually higher. So the first visual layer (layers[0])
        // ends up at the highest children index = visually topmost. Correct.
        for (layer_id, _) in self.layers.iter() {
            // Find this layer in removed_layers
            let pos = removed_layers.iter().position(|(id, _, _, _)| id == layer_id)
                .ok_or_else(|| format!("Layer {} missing from removed set", layer_id))?;
            let (_, layer, old_parent_id, old_idx) = removed_layers.remove(pos);
            self.removed.push((layer.clone(), old_parent_id, old_idx));

            let new_children = get_parent_children(document, self.new_parent_id)?;
            let insert_at = base.min(new_children.len());
            new_children.insert(insert_at, layer);
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if self.removed.is_empty() {
            return Err("Cannot rollback: action was not executed".to_string());
        }

        // Phase 1: Remove all layers from new parent (back-to-front by insertion order).
        for (layer_id, _) in self.layers.iter().rev() {
            let new_children = get_parent_children(document, self.new_parent_id)?;
            let pos = new_children.iter().position(|l| l.id() == *layer_id)
                .ok_or_else(|| format!("Layer {} not found in new parent for rollback", layer_id))?;
            new_children.remove(pos);
        }

        // Phase 2: Re-insert at old positions, sorted by (parent, index) ascending.
        let mut restore: Vec<(AnyLayer, Option<Uuid>, usize)> = self.removed.drain(..).collect();
        restore.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));

        for (layer, old_parent_id, old_idx) in restore {
            let children = get_parent_children(document, old_parent_id)?;
            let idx = old_idx.min(children.len());
            children.insert(idx, layer);
        }

        Ok(())
    }

    fn description(&self) -> String {
        if self.layers.len() == 1 {
            "Move layer".to_string()
        } else {
            format!("Move {} layers", self.layers.len())
        }
    }
}
