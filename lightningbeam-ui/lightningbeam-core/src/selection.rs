//! Selection state management
//!
//! Tracks selected DCEL elements (edges, faces, vertices) and clip instances for editing operations.

use crate::dcel::{Dcel, EdgeId, FaceId, VertexId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;
use vello::kurbo::{Affine, BezPath};

/// Shape of a raster pixel selection, in canvas pixel coordinates.
#[derive(Clone, Debug)]
pub enum RasterSelection {
    /// Axis-aligned rectangle: (x0, y0, x1, y1), x1 >= x0, y1 >= y0.
    Rect(i32, i32, i32, i32),
    /// Closed freehand lasso polygon.
    Lasso(Vec<(i32, i32)>),
    /// Per-pixel boolean mask (e.g. from magic wand flood fill).
    /// `data` is row-major, length = width × height.
    Mask {
        data: Vec<bool>,
        width: u32,
        height: u32,
        /// Top-left canvas pixel of the mask's bounding canvas region.
        origin_x: i32,
        origin_y: i32,
    },
}

impl RasterSelection {
    /// Bounding box as (x0, y0, x1, y1).
    pub fn bounding_rect(&self) -> (i32, i32, i32, i32) {
        match self {
            Self::Rect(x0, y0, x1, y1) => (*x0, *y0, *x1, *y1),
            Self::Lasso(pts) => {
                let x0 = pts.iter().map(|p| p.0).min().unwrap_or(0);
                let y0 = pts.iter().map(|p| p.1).min().unwrap_or(0);
                let x1 = pts.iter().map(|p| p.0).max().unwrap_or(0);
                let y1 = pts.iter().map(|p| p.1).max().unwrap_or(0);
                (x0, y0, x1, y1)
            }
            Self::Mask { data, width, height, origin_x, origin_y } => {
                let w = *width as i32;
                let mut bx0 = i32::MAX; let mut by0 = i32::MAX;
                let mut bx1 = i32::MIN; let mut by1 = i32::MIN;
                for row in 0..*height as i32 {
                    for col in 0..w {
                        if data[(row * w + col) as usize] {
                            bx0 = bx0.min(origin_x + col);
                            by0 = by0.min(origin_y + row);
                            bx1 = bx1.max(origin_x + col + 1);
                            by1 = by1.max(origin_y + row + 1);
                        }
                    }
                }
                if bx0 == i32::MAX { (*origin_x, *origin_y, *origin_x, *origin_y) }
                else { (bx0, by0, bx1, by1) }
            }
        }
    }

    /// Returns true if the given canvas pixel is inside the selection.
    pub fn contains_pixel(&self, px: i32, py: i32) -> bool {
        match self {
            Self::Rect(x0, y0, x1, y1) => px >= *x0 && px < *x1 && py >= *y0 && py < *y1,
            Self::Lasso(pts) => point_in_polygon(px, py, pts),
            Self::Mask { data, width, height, origin_x, origin_y } => {
                let lx = px - origin_x;
                let ly = py - origin_y;
                if lx < 0 || ly < 0 || lx >= *width as i32 || ly >= *height as i32 {
                    return false;
                }
                data[(ly * *width as i32 + lx) as usize]
            }
        }
    }
}

/// Even-odd point-in-polygon test for integer coordinates.
fn point_in_polygon(px: i32, py: i32, polygon: &[(i32, i32)]) -> bool {
    let n = polygon.len();
    if n < 3 { return false; }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (polygon[i].0 as f64, polygon[i].1 as f64);
        let (xj, yj) = (polygon[j].0 as f64, polygon[j].1 as f64);
        let x = px as f64;
        let y = py as f64;
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A pasted or cut selection that floats above the canvas until committed.
///
/// While a floating selection is alive `raw_pixels` on the target keyframe is
/// left in a "pre-composite" state (hole punched for cut, unchanged for copy).
/// The floating pixels are rendered as an overlay.  Committing composites them
/// into `raw_pixels` and records a `RasterStrokeAction` for undo.
#[derive(Clone, Debug)]
pub struct RasterFloatingSelection {
    /// sRGB-encoded premultiplied RGBA, width × height × 4 bytes.
    /// Wrapped in Arc so the renderer can clone a reference each frame (O(1))
    /// instead of copying megabytes of pixel data.
    pub pixels: std::sync::Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Top-left position in canvas pixel coordinates.
    pub x: i32,
    pub y: i32,
    /// Which raster layer and keyframe this float belongs to.
    pub layer_id: Uuid,
    pub time: f64,
    /// Snapshot of `raw_pixels` before the cut/paste was initiated, used for
    /// undo (via `RasterStrokeAction`) when the float is committed, and for
    /// Cancel (Escape) to restore the canvas without creating an undo entry.
    pub canvas_before: std::sync::Arc<Vec<u8>>,
    /// Key for this float's GPU canvas in `GpuBrushEngine::canvases`.
    /// Allows painting strokes directly onto the float buffer (B) without
    /// touching the layer canvas (A).
    pub canvas_id: Uuid,
}

/// Tracks the most recently selected thing(s) across the entire document.
///
/// Lightweight overlay on top of per-domain selection state. Tells consumers
/// "the user's attention is on this kind of thing" — for properties panels,
/// delete/copy/paste dispatch, group commands, etc.
#[derive(Clone, Debug, Default)]
pub enum FocusSelection {
    #[default]
    None,
    /// One or more layers selected (by UUID)
    Layers(Vec<Uuid>),
    /// One or more clip instances selected (by UUID)
    ClipInstances(Vec<Uuid>),
    /// DCEL geometry selected on a specific layer at a specific time
    Geometry { layer_id: Uuid, time: f64 },
    /// MIDI notes selected in piano roll
    Notes { layer_id: Uuid, midi_clip_id: u32, indices: Vec<usize> },
    /// Node graph nodes selected (backend node indices)
    Nodes(Vec<u32>),
    /// Assets selected in asset library (by UUID)
    Assets(Vec<Uuid>),
}

impl FocusSelection {
    pub fn is_none(&self) -> bool {
        matches!(self, FocusSelection::None)
    }

    pub fn layer_ids(&self) -> Option<&[Uuid]> {
        match self {
            FocusSelection::Layers(ids) => Some(ids),
            _ => Option::None,
        }
    }

    pub fn clip_instance_ids(&self) -> Option<&[Uuid]> {
        match self {
            FocusSelection::ClipInstances(ids) => Some(ids),
            _ => Option::None,
        }
    }
}

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

    /// Active raster pixel selection (marquee or lasso outline).
    /// Transient UI state — not persisted.
    #[serde(skip)]
    pub raster_selection: Option<RasterSelection>,

    /// Floating raster selection waiting to be committed or cancelled.
    /// Transient UI state — not persisted.
    #[serde(skip)]
    pub raster_floating: Option<RasterFloatingSelection>,

    /// Standalone DCEL subgraph ready for clipboard operations.
    ///
    /// Set when a region selection is committed (contains the extracted geometry).
    /// Cleared when the selection is cleared. Used by clipboard_copy_selection
    /// to avoid re-extracting the geometry from the live DCEL.
    #[serde(skip)]
    pub vector_subgraph: Option<Dcel>,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self {
            selected_vertices: HashSet::new(),
            selected_edges: HashSet::new(),
            selected_faces: HashSet::new(),
            selected_clip_instances: Vec::new(),
            raster_selection: None,
            raster_floating: None,
            vector_subgraph: None,
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

    /// Select a face by ID only, without adding boundary edges or vertices.
    ///
    /// Use this when the geometry lives in a separate DCEL (e.g. region selection's
    /// `selected_dcel`) so we don't add stale edge/vertex IDs to the selection.
    pub fn select_face_id_only(&mut self, face_id: FaceId) {
        if !face_id.is_none() && face_id.0 != 0 {
            self.selected_faces.insert(face_id);
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
        self.vector_subgraph = None;
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
        self.raster_selection = None;
        self.raster_floating = None;
        self.vector_subgraph = None;
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
/// When a region select is active, the region boundary is inserted into the
/// DCEL as invisible edges, splitting existing geometry. Faces inside the
/// region are added to the normal `Selection`. If the user performs an
/// operation, the selection is committed; if they deselect, the DCEL is
/// restored from the snapshot.
#[derive(Clone, Debug)]
pub struct RegionSelection {
    /// The clipping region as a closed BezPath (polygon or rect)
    pub region_path: BezPath,
    /// Layer containing the affected elements
    pub layer_id: Uuid,
    /// Keyframe time
    pub time: f64,
    /// Snapshot of the DCEL before region boundary insertion, for revert
    pub dcel_snapshot: Dcel,
    /// The extracted DCEL containing geometry inside the region
    pub selected_dcel: Dcel,
    /// Transform applied to the selected DCEL (e.g. from dragging)
    pub transform: Affine,
    /// Whether the selection has been committed (via an operation on the selection)
    pub committed: bool,
    /// Non-boundary vertices that are strictly inside the region (for merge-back).
    pub inside_vertices: Vec<VertexId>,
    /// Region boundary intersection vertices (for merge-back and fill propagation).
    pub boundary_vertices: Vec<VertexId>,
    /// IDs of the invisible edges inserted for the region boundary stroke.
    /// Removing these during merge-back heals the face splits they created.
    pub region_edge_ids: Vec<EdgeId>,
    /// Action epoch recorded when this selection was created.
    /// Compared against `ActionExecutor::epoch()` on deselect to decide
    /// whether merge-back is needed or a clean snapshot restore suffices.
    pub action_epoch_at_selection: u64,
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
