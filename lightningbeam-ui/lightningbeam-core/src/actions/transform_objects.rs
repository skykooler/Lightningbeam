//! Transform shapes action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action to transform multiple shapes in a keyframe
/// TODO: Replace with DCEL-based transforms (affine on vertices/edges)
pub struct TransformShapeInstancesAction {
    layer_id: Uuid,
    time: f64,
    shape_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformShapeInstancesAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        shape_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            time,
            shape_transforms,
        }
    }
}

impl Action for TransformShapeInstancesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time, &self.shape_transforms);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        format!("Transform {} shape(s)", self.shape_transforms.len())
    }
}
