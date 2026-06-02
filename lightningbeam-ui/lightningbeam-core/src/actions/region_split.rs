//! Region split action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use crate::shape::Shape;
use uuid::Uuid;
use vello::kurbo::BezPath;

/// Action that commits a region split
/// TODO: Rewrite for DCEL edge splitting
pub struct RegionSplitAction {
    layer_id: Uuid,
    time: f64,
}

impl RegionSplitAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        _split_data: Vec<(Shape, BezPath, Uuid, BezPath, Uuid)>,
    ) -> Self {
        Self {
            layer_id,
            time,
        }
    }
}

impl Action for RegionSplitAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        "Region split".to_string()
    }
}
