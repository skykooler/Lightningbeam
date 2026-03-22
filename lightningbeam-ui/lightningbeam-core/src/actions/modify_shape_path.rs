//! Modify graph action — snapshot-based undo for VectorGraph editing

use crate::action::Action;
use crate::vector_graph::VectorGraph;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that captures a before/after VectorGraph snapshot for undo/redo.
///
/// Used by vertex editing, curve editing, and control point editing.
/// The caller provides both snapshots (taken before and after the edit).
pub struct ModifyGraphAction {
    layer_id: Uuid,
    time: f64,
    graph_before: Option<VectorGraph>,
    graph_after: Option<VectorGraph>,
    description_text: String,
}

impl ModifyGraphAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        graph_before: VectorGraph,
        graph_after: VectorGraph,
        description: impl Into<String>,
    ) -> Self {
        Self {
            layer_id,
            time,
            graph_before: Some(graph_before),
            graph_after: Some(graph_after),
            description_text: description.into(),
        }
    }
}

impl Action for ModifyGraphAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let graph_after = self.graph_after.as_ref()
            .ok_or("ModifyGraphAction: no graph_after snapshot")?
            .clone();

        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.graph = graph_after;
                Ok(())
            } else {
                Err(format!("No keyframe at time {}", self.time))
            }
        } else {
            Err("Not a vector layer".to_string())
        }
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let graph_before = self.graph_before.as_ref()
            .ok_or("ModifyGraphAction: no graph_before snapshot")?
            .clone();

        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.graph = graph_before;
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
