//! Group action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use uuid::Uuid;

/// Action that groups selected shapes and/or clip instances into a VectorClip
/// TODO: Rewrite for DCEL (group DCEL faces/edges into a sub-clip)
#[allow(dead_code)]
pub struct GroupAction {
    layer_id: Uuid,
    time: f64,
    shape_ids: Vec<Uuid>,
    clip_instance_ids: Vec<Uuid>,
    instance_id: Uuid,
    created_clip_id: Option<Uuid>,
    removed_clip_instances: Vec<ClipInstance>,
}

impl GroupAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        shape_ids: Vec<Uuid>,
        clip_instance_ids: Vec<Uuid>,
        instance_id: Uuid,
    ) -> Self {
        Self {
            layer_id,
            time,
            shape_ids,
            clip_instance_ids,
            instance_id,
            created_clip_id: None,
            removed_clip_instances: Vec::new(),
        }
    }
}

impl Action for GroupAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time, &self.shape_ids, &self.clip_instance_ids, self.instance_id);
        // TODO: Implement DCEL-aware grouping
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_ids.len() + self.clip_instance_ids.len();
        format!("Group {} objects", count)
    }
}
