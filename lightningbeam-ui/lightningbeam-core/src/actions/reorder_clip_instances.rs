//! Reorder clip instances within a layer's stacking order (Send to Back / Bring to Front).
//!
//! A layer's `clip_instances` Vec order *is* the stacking order — the last element renders on top
//! (hit-testing walks it in reverse). Geometry renders underneath and is unaffected. This action
//! moves the selected instances to the front (end) or back (start) of that Vec.

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

pub struct ReorderClipInstancesAction {
    layer_id: Uuid,
    instance_ids: Vec<Uuid>,
    /// `true` = bring to front (top), `false` = send to back (bottom).
    to_front: bool,
    /// Full instance order captured on `execute`, for `rollback`.
    old_order: Option<Vec<Uuid>>,
}

impl ReorderClipInstancesAction {
    pub fn new(layer_id: Uuid, instance_ids: Vec<Uuid>, to_front: bool) -> Self {
        Self { layer_id, instance_ids, to_front, old_order: None }
    }
}

/// The clip-instance stack for a layer, if it has one (Group/Raster/Text don't).
fn clip_instances_mut<'a>(document: &'a mut Document, layer_id: &Uuid) -> Option<&'a mut Vec<ClipInstance>> {
    match document.get_layer_mut(layer_id)? {
        AnyLayer::Vector(l) => Some(&mut l.clip_instances),
        AnyLayer::Audio(l) => Some(&mut l.clip_instances),
        AnyLayer::Video(l) => Some(&mut l.clip_instances),
        AnyLayer::Effect(l) => Some(&mut l.clip_instances),
        _ => None,
    }
}

impl Action for ReorderClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let instances = clip_instances_mut(document, &self.layer_id)
            .ok_or_else(|| "Layer has no clip-instance stack".to_string())?;
        self.old_order = Some(instances.iter().map(|c| c.id).collect());

        let mut selected = Vec::new();
        let mut rest = Vec::new();
        for ci in instances.drain(..) {
            if self.instance_ids.contains(&ci.id) {
                selected.push(ci);
            } else {
                rest.push(ci);
            }
        }
        if self.to_front {
            rest.extend(selected); // selected last → rendered on top
            *instances = rest;
        } else {
            selected.extend(rest); // selected first → rendered at the bottom (of the stack)
            *instances = selected;
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(order) = self.old_order.clone() else {
            return Ok(());
        };
        let instances = clip_instances_mut(document, &self.layer_id)
            .ok_or_else(|| "Layer has no clip-instance stack".to_string())?;
        let rank = |id: &Uuid| order.iter().position(|o| o == id).unwrap_or(usize::MAX);
        instances.sort_by(|a, b| rank(&a.id).cmp(&rank(&b.id)));
        Ok(())
    }

    fn description(&self) -> String {
        if self.to_front { "Bring to Front".to_string() } else { "Send to Back".to_string() }
    }
}
