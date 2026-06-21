//! Selection state management
//!
//! Tracks selected DCEL elements (edges, faces, vertices) and clip instances for editing operations.

use crate::vector_graph::{VectorGraph, EdgeId, FillId, VertexId};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

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

    /// Currently selected fills
    selected_fills: HashSet<FillId>,

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
    pub vector_subgraph: Option<VectorGraph>,
}

impl Selection {
    /// Create a new empty selection
    pub fn new() -> Self {
        Self {
            selected_vertices: HashSet::new(),
            selected_edges: HashSet::new(),
            selected_fills: HashSet::new(),
            selected_clip_instances: Vec::new(),
            raster_selection: None,
            raster_floating: None,
            vector_subgraph: None,
        }
    }

    // -----------------------------------------------------------------------
    // Geometry element selection (VectorGraph)
    // -----------------------------------------------------------------------

    /// Select an edge and its endpoint vertices, forming/extending a subgraph.
    pub fn select_edge(&mut self, edge_id: EdgeId, graph: &VectorGraph) {
        if edge_id.is_none() || graph.edge(edge_id).deleted {
            return;
        }
        self.selected_edges.insert(edge_id);

        // Add both endpoint vertices
        let [v0, v1] = graph.edge(edge_id).vertices;
        if !v0.is_none() {
            self.selected_vertices.insert(v0);
        }
        if !v1.is_none() {
            self.selected_vertices.insert(v1);
        }
    }

    /// Select a fill by ID only, without adding boundary edges or vertices.
    ///
    /// Use this when the geometry lives in a separate graph (e.g. region selection's
    /// `selected_graph`) so we don't add stale edge/vertex IDs to the selection.
    pub fn select_fill_id_only(&mut self, fill_id: FillId) {
        if !fill_id.is_none() {
            self.selected_fills.insert(fill_id);
        }
    }

    /// Select a fill and all its boundary edges + vertices.
    pub fn select_fill(&mut self, fill_id: FillId, graph: &VectorGraph) {
        if fill_id.is_none() || graph.fill(fill_id).deleted {
            return;
        }
        self.selected_fills.insert(fill_id);

        // Add all boundary edges and vertices
        for eid in graph.fill_boundary_edges(fill_id) {
            self.selected_edges.insert(eid);
            let [v0, v1] = graph.edge(eid).vertices;
            if !v0.is_none() {
                self.selected_vertices.insert(v0);
            }
            if !v1.is_none() {
                self.selected_vertices.insert(v1);
            }
        }
    }

    /// Deselect an edge and its vertices (if they have no other selected edges).
    pub fn deselect_edge(&mut self, edge_id: EdgeId, graph: &VectorGraph) {
        self.selected_edges.remove(&edge_id);

        // Remove endpoint vertices only if they're not used by other selected edges
        let [v0, v1] = graph.edge(edge_id).vertices;
        for v in [v0, v1] {
            if v.is_none() {
                continue;
            }
            // Check if any other selected edge uses this vertex
            let used = self.selected_edges.iter().any(|&eid| {
                let e = graph.edge(eid);
                e.vertices[0] == v || e.vertices[1] == v
            });
            if !used {
                self.selected_vertices.remove(&v);
            }
        }
    }

    /// Deselect a fill (edges/vertices stay if still referenced by other selections).
    pub fn deselect_fill(&mut self, fill_id: FillId) {
        self.selected_fills.remove(&fill_id);
    }

    /// Toggle an edge's selection state.
    pub fn toggle_edge(&mut self, edge_id: EdgeId, graph: &VectorGraph) {
        if self.selected_edges.contains(&edge_id) {
            self.deselect_edge(edge_id, graph);
        } else {
            self.select_edge(edge_id, graph);
        }
    }

    /// Toggle a fill's selection state.
    pub fn toggle_fill(&mut self, fill_id: FillId, graph: &VectorGraph) {
        if self.selected_fills.contains(&fill_id) {
            self.deselect_fill(fill_id);
        } else {
            self.select_fill(fill_id, graph);
        }
    }

    /// Check if an edge is selected.
    pub fn contains_edge(&self, edge_id: &EdgeId) -> bool {
        self.selected_edges.contains(edge_id)
    }

    /// Check if a fill is selected.
    pub fn contains_fill(&self, fill_id: &FillId) -> bool {
        self.selected_fills.contains(fill_id)
    }

    /// Check if a vertex is selected.
    pub fn contains_vertex(&self, vertex_id: &VertexId) -> bool {
        self.selected_vertices.contains(vertex_id)
    }

    /// Clear geometry element selections (edges, fills, vertices).
    pub fn clear_geometry_selection(&mut self) {
        self.selected_vertices.clear();
        self.selected_edges.clear();
        self.selected_fills.clear();
        self.vector_subgraph = None;
    }

    /// Check if any geometry elements are selected.
    pub fn has_geometry_selection(&self) -> bool {
        !self.selected_edges.is_empty() || !self.selected_fills.is_empty()
    }

    /// Get selected edges.
    pub fn selected_edges(&self) -> &HashSet<EdgeId> {
        &self.selected_edges
    }

    /// Get selected fills.
    pub fn selected_fills(&self) -> &HashSet<FillId> {
        &self.selected_fills
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
        self.selected_fills.clear();
        self.selected_clip_instances.clear();
        self.raster_selection = None;
        self.raster_floating = None;
        self.vector_subgraph = None;
    }

    /// Check if selection is empty
    pub fn is_empty(&self) -> bool {
        self.selected_edges.is_empty()
            && self.selected_fills.is_empty()
            && self.selected_clip_instances.is_empty()
    }
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
    fn test_geometry_selection_basics() {
        let selection = Selection::new();
        assert!(!selection.has_geometry_selection());
        assert!(selection.selected_edges().is_empty());
        assert!(selection.selected_fills().is_empty());
        assert!(selection.selected_vertices().is_empty());
    }

    #[test]
    fn test_clear_geometry_selection() {
        let mut selection = Selection::new();
        // Manually insert for unit test (no graph needed)
        selection.selected_edges.insert(EdgeId(0));
        selection.selected_vertices.insert(VertexId(0));
        assert!(selection.has_geometry_selection());

        selection.clear_geometry_selection();
        assert!(!selection.has_geometry_selection());
    }
}
