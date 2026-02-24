//! Selection state management
//!
//! Tracks selected DCEL elements (edges, faces, vertices) and clip instances for editing operations.

use crate::dcel::{Dcel, EdgeId, FaceId, VertexId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;
use vello::kurbo::BezPath;

/// Selection state for the editor
///
/// Maintains sets of selected DCEL elements and clip instances.
/// The vertex/edge/face sets implicitly represent a subgraph of the DCEL —
/// connectivity is determined by shared vertices between edges.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Selection {
    /// Currently selected vertices
    selected_vertices: HashSet<VertexId>,

    /// Currently selected edges
    selected_edges: HashSet<EdgeId>,

    /// Currently selected faces
    selected_faces: HashSet<FaceId>,

    /// Currently selected clip instances
    selected_clip_instances: Vec<Uuid>,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self {
            selected_vertices: HashSet::new(),
            selected_edges: HashSet::new(),
            selected_faces: HashSet::new(),
            selected_clip_instances: Vec::new(),
        }
    }

    // -----------------------------------------------------------------------
    // DCEL element selection
    // -----------------------------------------------------------------------

    /// Select an edge and its endpoint vertices, forming/extending a subgraph.
    pub fn select_edge(&mut self, edge_id: EdgeId, dcel: &Dcel) {
        if edge_id.is_none() || dcel.edge(edge_id).deleted {
            return;
        }
        self.selected_edges.insert(edge_id);

        // Add both endpoint vertices
        let [he_fwd, he_bwd] = dcel.edge(edge_id).half_edges;
        if !he_fwd.is_none() {
            let v = dcel.half_edge(he_fwd).origin;
            if !v.is_none() {
                self.selected_vertices.insert(v);
            }
        }
        if !he_bwd.is_none() {
            let v = dcel.half_edge(he_bwd).origin;
            if !v.is_none() {
                self.selected_vertices.insert(v);
            }
        }
    }

    /// Select a face and all its boundary edges + vertices.
    pub fn select_face(&mut self, face_id: FaceId, dcel: &Dcel) {
        if face_id.is_none() || face_id.0 == 0 || dcel.face(face_id).deleted {
            return;
        }
        self.selected_faces.insert(face_id);

        // Add all boundary edges and vertices
        let boundary = dcel.face_boundary(face_id);
        for he_id in boundary {
            let he = dcel.half_edge(he_id);
            let edge_id = he.edge;
            if !edge_id.is_none() {
                self.selected_edges.insert(edge_id);
                // Add endpoints
                let [he_fwd, he_bwd] = dcel.edge(edge_id).half_edges;
                if !he_fwd.is_none() {
                    let v = dcel.half_edge(he_fwd).origin;
                    if !v.is_none() {
                        self.selected_vertices.insert(v);
                    }
                }
                if !he_bwd.is_none() {
                    let v = dcel.half_edge(he_bwd).origin;
                    if !v.is_none() {
                        self.selected_vertices.insert(v);
                    }
                }
            }
        }
    }

    /// Deselect an edge and its vertices (if they have no other selected edges).
    pub fn deselect_edge(&mut self, edge_id: EdgeId, dcel: &Dcel) {
        self.selected_edges.remove(&edge_id);

        // Remove endpoint vertices only if they're not used by other selected edges
        let [he_fwd, he_bwd] = dcel.edge(edge_id).half_edges;
        for he_id in [he_fwd, he_bwd] {
            if he_id.is_none() {
                continue;
            }
            let v = dcel.half_edge(he_id).origin;
            if v.is_none() {
                continue;
            }
            // Check if any other selected edge uses this vertex
            let used = self.selected_edges.iter().any(|&eid| {
                let e = dcel.edge(eid);
                let [a, b] = e.half_edges;
                (!a.is_none() && dcel.half_edge(a).origin == v)
                    || (!b.is_none() && dcel.half_edge(b).origin == v)
            });
            if !used {
                self.selected_vertices.remove(&v);
            }
        }
    }

    /// Deselect a face (edges/vertices stay if still referenced by other selections).
    pub fn deselect_face(&mut self, face_id: FaceId) {
        self.selected_faces.remove(&face_id);
    }

    /// Toggle an edge's selection state.
    pub fn toggle_edge(&mut self, edge_id: EdgeId, dcel: &Dcel) {
        if self.selected_edges.contains(&edge_id) {
            self.deselect_edge(edge_id, dcel);
        } else {
            self.select_edge(edge_id, dcel);
        }
    }

    /// Toggle a face's selection state.
    pub fn toggle_face(&mut self, face_id: FaceId, dcel: &Dcel) {
        if self.selected_faces.contains(&face_id) {
            self.deselect_face(face_id);
        } else {
            self.select_face(face_id, dcel);
        }
    }

    /// Check if an edge is selected.
    pub fn contains_edge(&self, edge_id: &EdgeId) -> bool {
        self.selected_edges.contains(edge_id)
    }

    /// Check if a face is selected.
    pub fn contains_face(&self, face_id: &FaceId) -> bool {
        self.selected_faces.contains(face_id)
    }

    /// Check if a vertex is selected.
    pub fn contains_vertex(&self, vertex_id: &VertexId) -> bool {
        self.selected_vertices.contains(vertex_id)
    }

    /// Clear DCEL element selections (edges, faces, vertices).
    pub fn clear_dcel_selection(&mut self) {
        self.selected_vertices.clear();
        self.selected_edges.clear();
        self.selected_faces.clear();
    }

    /// Check if any DCEL elements are selected.
    pub fn has_dcel_selection(&self) -> bool {
        !self.selected_edges.is_empty() || !self.selected_faces.is_empty()
    }

    /// Get selected edges.
    pub fn selected_edges(&self) -> &HashSet<EdgeId> {
        &self.selected_edges
    }

    /// Get selected faces.
    pub fn selected_faces(&self) -> &HashSet<FaceId> {
        &self.selected_faces
    }

    /// Get selected vertices.
    pub fn selected_vertices(&self) -> &HashSet<VertexId> {
        &self.selected_vertices
    }

    // -----------------------------------------------------------------------
    // Clip instance selection (unchanged)
    // -----------------------------------------------------------------------

    /// Add a clip instance to the selection
    pub fn add_clip_instance(&mut self, id: Uuid) {
        if !self.selected_clip_instances.contains(&id) {
            self.selected_clip_instances.push(id);
        }
    }

    /// Remove a clip instance from the selection
    pub fn remove_clip_instance(&mut self, id: &Uuid) {
        self.selected_clip_instances.retain(|&x| x != *id);
    }

    /// Toggle a clip instance's selection state
    pub fn toggle_clip_instance(&mut self, id: Uuid) {
        if self.contains_clip_instance(&id) {
            self.remove_clip_instance(&id);
        } else {
            self.add_clip_instance(id);
        }
    }

    /// Check if a clip instance is selected
    pub fn contains_clip_instance(&self, id: &Uuid) -> bool {
        self.selected_clip_instances.contains(id)
    }

    /// Clear only clip instance selections
    pub fn clear_clip_instances(&mut self) {
        self.selected_clip_instances.clear();
    }

    /// Get the selected clip instances
    pub fn clip_instances(&self) -> &[Uuid] {
        &self.selected_clip_instances
    }

    /// Get the number of selected clip instances
    pub fn clip_instance_count(&self) -> usize {
        self.selected_clip_instances.len()
    }

    /// Set selection to a single clip instance (clears previous selection)
    pub fn select_only_clip_instance(&mut self, id: Uuid) {
        self.clear();
        self.add_clip_instance(id);
    }

    /// Set selection to multiple clip instances (clears previous clip selection)
    pub fn select_clip_instances(&mut self, ids: &[Uuid]) {
        self.clear_clip_instances();
        for &id in ids {
            self.add_clip_instance(id);
        }
    }

    // -----------------------------------------------------------------------
    // General
    // -----------------------------------------------------------------------

    /// Clear all selections
    pub fn clear(&mut self) {
        self.selected_vertices.clear();
        self.selected_edges.clear();
        self.selected_faces.clear();
        self.selected_clip_instances.clear();
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.selected_edges.is_empty()
            && self.selected_faces.is_empty()
            && self.selected_clip_instances.is_empty()
    }
}

/// Represents a temporary region-based selection.
///
/// When a region select is active, elements that cross the region boundary
/// are tracked. If the user performs an operation, the selection is
/// committed; if they deselect, the original state is restored.
#[derive(Clone, Debug)]
pub struct RegionSelection {
    /// The clipping region as a closed BezPath (polygon or rect)
    pub region_path: BezPath,
    /// Layer containing the affected elements
    pub layer_id: Uuid,
    /// Keyframe time
    pub time: f64,
    /// Per-shape split results (legacy, kept for compatibility)
    pub splits: Vec<()>,
    /// IDs that were fully inside the region
    pub fully_inside_ids: Vec<Uuid>,
    /// Whether the selection has been committed (via an operation on the selection)
    pub committed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_creation() {
        let selection = Selection::new();
        assert!(selection.is_empty());
        assert_eq!(selection.clip_instance_count(), 0);
    }

    #[test]
    fn test_add_remove_clip_instances() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_clip_instance(id1);
        assert_eq!(selection.clip_instance_count(), 1);
        assert!(selection.contains_clip_instance(&id1));

        selection.add_clip_instance(id2);
        assert_eq!(selection.clip_instance_count(), 2);

        selection.remove_clip_instance(&id1);
        assert_eq!(selection.clip_instance_count(), 1);
        assert!(!selection.contains_clip_instance(&id1));
        assert!(selection.contains_clip_instance(&id2));
    }

    #[test]
    fn test_toggle_clip_instance() {
        let mut selection = Selection::new();
        let id = Uuid::new_v4();

        selection.toggle_clip_instance(id);
        assert!(selection.contains_clip_instance(&id));

        selection.toggle_clip_instance(id);
        assert!(!selection.contains_clip_instance(&id));
    }

    #[test]
    fn test_select_only_clip_instance() {
        let mut selection = Selection::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        selection.add_clip_instance(id1);
        selection.add_clip_instance(id2);
        assert_eq!(selection.clip_instance_count(), 2);

        selection.select_only_clip_instance(id1);
        assert_eq!(selection.clip_instance_count(), 1);
        assert!(selection.contains_clip_instance(&id1));
        assert!(!selection.contains_clip_instance(&id2));
    }

    #[test]
    fn test_clear() {
        let mut selection = Selection::new();
        selection.add_clip_instance(Uuid::new_v4());

        assert!(!selection.is_empty());

        selection.clear();
        assert!(selection.is_empty());
    }

    #[test]
    fn test_dcel_selection_basics() {
        let selection = Selection::new();
        assert!(!selection.has_dcel_selection());
        assert!(selection.selected_edges().is_empty());
        assert!(selection.selected_faces().is_empty());
        assert!(selection.selected_vertices().is_empty());
    }

    #[test]
    fn test_clear_dcel_selection() {
        let mut selection = Selection::new();
        // Manually insert for unit test (no DCEL needed)
        selection.selected_edges.insert(EdgeId(0));
        selection.selected_vertices.insert(VertexId(0));
        assert!(selection.has_dcel_selection());

        selection.clear_dcel_selection();
        assert!(!selection.has_dcel_selection());
    }
}
