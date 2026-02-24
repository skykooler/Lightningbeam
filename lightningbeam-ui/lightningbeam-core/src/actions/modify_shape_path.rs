//! Modify DCEL action — snapshot-based undo for DCEL editing

use crate::action::Action;
use crate::dcel::Dcel;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that captures a before/after DCEL snapshot for undo/redo.
///
/// Used by vertex editing, curve editing, and control point editing.
/// The caller provides both snapshots (taken before and after the edit).
pub struct ModifyDcelAction {
    layer_id: Uuid,
    time: f64,
    dcel_before: Option<Dcel>,
    dcel_after: Option<Dcel>,
    description_text: String,
}

impl ModifyDcelAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        dcel_before: Dcel,
        dcel_after: Dcel,
        description: impl Into<String>,
    ) -> Self {
        Self {
            layer_id,
            time,
            dcel_before: Some(dcel_before),
            dcel_after: Some(dcel_after),
            description_text: description.into(),
        }
    }
}

impl Action for ModifyDcelAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel_after = self.dcel_after.as_ref()
            .ok_or("ModifyDcelAction: no dcel_after snapshot")?
            .clone();

        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.dcel = dcel_after;
                Ok(())
            } else {
                Err(format!("No keyframe at time {}", self.time))
            }
        } else {
            Err("Not a vector layer".to_string())
        }
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel_before = self.dcel_before.as_ref()
            .ok_or("ModifyDcelAction: no dcel_before snapshot")?
            .clone();

        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.dcel = dcel_before;
                Ok(())
            } else {
                Err(format!("No keyframe at time {}", self.time))
            }
        } else {
            Err("Not a vector layer".to_string())
        }
    }

    fn description(&self) -> String {
        self.description_text.clone()
    }
}
