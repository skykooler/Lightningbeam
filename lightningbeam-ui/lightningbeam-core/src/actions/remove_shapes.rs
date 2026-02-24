//! Remove shapes action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use uuid::Uuid;

/// Action that removes shapes from a vector layer's keyframe
/// TODO: Replace with DCEL edge/face removal actions
pub struct RemoveShapesAction {
    layer_id: Uuid,
    shape_ids: Vec<Uuid>,
    time: f64,
}

impl RemoveShapesAction {
    pub fn new(layer_id: Uuid, shape_ids: Vec<Uuid>, time: f64) -> Self {
        Self {
            layer_id,
            shape_ids,
            time,
        }
    }
}

impl Action for RemoveShapesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, &self.shape_ids, self.time);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_ids.len();
        if count == 1 {
            "Delete shape".to_string()
        } else {
            format!("Delete {} shapes", count)
        }
    }
}
