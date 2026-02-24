//! Move shapes action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that moves shapes to new positions within a keyframe
/// TODO: Replace with DCEL vertex translation
pub struct MoveShapeInstancesAction {
    layer_id: Uuid,
    time: f64,
    shape_positions: HashMap<Uuid, (Point, Point)>,
}

impl MoveShapeInstancesAction {
    pub fn new(layer_id: Uuid, time: f64, shape_positions: HashMap<Uuid, (Point, Point)>) -> Self {
        Self {
            layer_id,
            time,
            shape_positions,
        }
    }
}

impl Action for MoveShapeInstancesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time, &self.shape_positions);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_positions.len();
        if count == 1 {
            "Move shape".to_string()
        } else {
            format!("Move {} shapes", count)
        }
    }
}
