//! Shared logic for the "Group" and "Convert to Movie Clip" actions: extract the
//! selected DCEL geometry from a vector layer's active keyframe into a new `VectorClip`
//! and drop a `ClipInstance` in its place (so it can then be motion-tweened).
//!
//! A *group* (`is_group = true`) is a static container; a *movie clip* (`is_group =
//! false`) has its own timeline. Both are tweenable via the clip instance's transform.
//!
//! Both the Select tool and the (cut-and-select) RegionSelect tool populate the same
//! `Selection` ID sets, so a single entry point — [`extract_geometry_to_clip`] — handles
//! Group and Convert-to-Movie-Clip from whatever is selected.

use std::collections::HashSet;

use crate::clip::{ClipInstance, VectorClip};
use crate::document::Document;
use crate::layer::{AnyLayer, ShapeKeyframe, VectorLayer};
use crate::object::Transform;
use crate::vector_graph::{EdgeId, FillId, VectorGraph};
use uuid::Uuid;

/// Build a clip holding `sub_graph` and place a `ClipInstance` (with `transform`) on the
/// layer. Shared by both extraction paths. Does **not** touch the source graph — the
/// caller is responsible for having removed the moved geometry first.
fn assemble_clip_from_graph(
    document: &mut Document,
    layer_id: Uuid,
    time: f64,
    sub_graph: VectorGraph,
    transform: Transform,
    clip_id: Uuid,
    instance_id: Uuid,
    is_group: bool,
    clip_name: &str,
) {
    let (doc_w, doc_h, doc_dur) = (document.width, document.height, document.duration.max(1.0));

    // A vector layer whose single keyframe holds the extracted graph (in the source's
    // coordinate space, so an identity/translation placement renders it in place).
    let mut inner = VectorLayer::new("Layer 1");
    let mut kf = ShapeKeyframe::new(0.0);
    kf.graph = sub_graph;
    inner.keyframes.push(kf);
    let mut clip = VectorClip::with_id(clip_id, clip_name, doc_w, doc_h, doc_dur);
    clip.is_group = is_group;
    clip.layers.add_root(AnyLayer::Vector(inner));
    document.add_vector_clip(clip);

    let mut instance = ClipInstance::with_id(instance_id, clip_id);
    instance.transform = transform;
    if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
        // Groups gate visibility by the active keyframe's clip_instance_ids; movie
        // clips render unconditionally.
        if is_group {
            if let Some(kf) = vl.keyframe_at_mut(time) {
                kf.clip_instance_ids.push(instance_id);
            }
        }
        vl.clip_instances.push(instance);
    }
}

/// Extract the selected geometry into a new clip + place a `ClipInstance`. Returns the
/// pre-extraction graph snapshot for undo. `clip_id`/`instance_id` are caller-provided
/// so undo/redo is stable. The selection sets come straight from the editor selection
/// (`select_fill` already includes each fill's boundary edges); `extract_subgraph`
/// derives which of those edges are shared with non-selected shapes.
pub fn extract_geometry_to_clip(
    document: &mut Document,
    layer_id: Uuid,
    time: f64,
    fills: &HashSet<FillId>,
    edges: &HashSet<EdgeId>,
    clip_id: Uuid,
    instance_id: Uuid,
    is_group: bool,
    clip_name: &str,
) -> Result<VectorGraph, String> {
    if fills.is_empty() && edges.is_empty() {
        return Err("No geometry selected".to_string());
    }

    // 1. Extract from the source graph (extract_subgraph removes the moved geometry).
    let (graph_before, sub_graph) = {
        let layer = document.get_layer_mut(&layer_id).ok_or("Layer not found")?;
        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };
        let graph = vl.graph_at_time_mut(time).ok_or("No keyframe at time")?;
        let before = graph.clone();
        // No explicit cut boundary — extract_subgraph derives shared-fill boundaries.
        let (sub, _, _) = graph.extract_subgraph(edges, fills, &HashSet::new());
        (before, sub)
    };

    // 2 & 3. Build the clip + place an identity-transform instance (geometry stays put).
    assemble_clip_from_graph(
        document, layer_id, time, sub_graph, Transform::default(),
        clip_id, instance_id, is_group, clip_name,
    );

    Ok(graph_before)
}

/// Reverse `extract_geometry_to_clip`: remove the clip + instance and restore the graph.
pub fn undo_extract_geometry(
    document: &mut Document,
    layer_id: Uuid,
    time: f64,
    clip_id: Uuid,
    instance_id: Uuid,
    graph_before: &VectorGraph,
) {
    document.vector_clips.remove(&clip_id);
    document.rebuild_layer_to_clip_map();
    if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
        vl.clip_instances.retain(|ci| ci.id != instance_id);
        if let Some(kf) = vl.keyframe_at_mut(time) {
            kf.clip_instance_ids.retain(|id| *id != instance_id);
        }
        if let Some(graph) = vl.graph_at_time_mut(time) {
            *graph = graph_before.clone();
        }
    }
}

