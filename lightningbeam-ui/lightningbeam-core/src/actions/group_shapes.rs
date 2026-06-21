//! Group action — extract selected geometry into a group `VectorClip` + a `ClipInstance`.

use std::collections::HashSet;

use crate::action::Action;
use crate::actions::clip_from_geometry::{extract_geometry_to_clip, undo_extract_geometry};
use crate::document::Document;
use crate::vector_graph::{EdgeId, FillId, VectorGraph};
use uuid::Uuid;

/// Groups the selected DCEL geometry (fills/edges) of a vector layer's active keyframe
/// into a new group clip, placing a clip instance in its place (which can be tweened).
pub struct GroupAction {
    layer_id: Uuid,
    time: f64,
    fills: Vec<FillId>,
    edges: Vec<EdgeId>,
    clip_id: Uuid,
    instance_id: Uuid,
    graph_before: Option<VectorGraph>,
}

impl GroupAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        fills: Vec<FillId>,
        edges: Vec<EdgeId>,
        clip_id: Uuid,
        instance_id: Uuid,
    ) -> Self {
        Self { layer_id, time, fills, edges, clip_id, instance_id, graph_before: None }
    }
}

impl Action for GroupAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let fills: HashSet<FillId> = self.fills.iter().copied().collect();
        let edges: HashSet<EdgeId> = self.edges.iter().copied().collect();
        let before = extract_geometry_to_clip(
            document, self.layer_id, self.time, &fills, &edges,
            self.clip_id, self.instance_id, true, "Group",
        )?;
        self.graph_before = Some(before);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(before) = &self.graph_before {
            undo_extract_geometry(document, self.layer_id, self.time, self.clip_id, self.instance_id, before);
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Group".to_string()
    }
}
