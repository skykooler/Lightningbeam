//! VectorGraph: a simple vertex+edge graph with explicit fill overlay.
//!
//! Replaces the DCEL for vector drawing storage. Key differences:
//! - No half-edges, no fan ordering invariant, no face objects
//! - Fills are stored as explicit boundary references, independent of topology
//! - Edges can be visible (strokes) or invisible (structural/gap-close)
//! - Curves are split at intersections; fills reference whole edges
//!
//! Lifecycle rules:
//! - Visible edge deleted by user → becomes invisible; removed only if no fill references it
//! - Fill deleted → its boundary edges checked; invisible unreferenced edges garbage collected
//! - Gap-close edges are invisible edges created by paint bucket with gap tolerance

pub mod tests;

use kurbo::{CubicBez, ParamCurve, Point};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::curve_intersections::find_curve_intersections;

// ---------------------------------------------------------------------------
// Index types
// ---------------------------------------------------------------------------

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub u32);

        impl $name {
            pub const NONE: Self = Self(u32::MAX);

            #[inline]
            pub fn is_none(self) -> bool {
                self.0 == u32::MAX
            }

            #[inline]
            pub fn idx(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.is_none() {
                    write!(f, "{}(NONE)", stringify!($name))
                } else {
                    write!(f, "{}({})", stringify!($name), self.0)
                }
            }
        }
    };
}

define_id!(VertexId);
define_id!(EdgeId);
define_id!(FillId);

// ---------------------------------------------------------------------------
// Direction for traversing an edge
// ---------------------------------------------------------------------------

/// Which direction to traverse an edge along its curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction {
    /// Traverse from vertex[0] to vertex[1] (curve parameter 0→1)
    Forward,
    /// Traverse from vertex[1] to vertex[0] (curve parameter 1→0)
    Backward,
}

// ---------------------------------------------------------------------------
// Core structs
// ---------------------------------------------------------------------------

/// A vertex in the graph — just a position.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    pub position: Point,
    pub deleted: bool,
}

/// An edge: a cubic Bézier curve between two vertices, with optional stroke style.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edge {
    pub curve: CubicBez,
    /// [start, end] — curve goes from vertices[0] to vertices[1].
    pub vertices: [VertexId; 2],
    /// Stroke style. None = invisible (structural edge, e.g., gap-close).
    pub stroke_style: Option<StrokeStyle>,
    /// Stroke color. None = invisible.
    pub stroke_color: Option<ShapeColor>,
    pub deleted: bool,
}

/// A fill: an explicit boundary referencing edges, with visual properties.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fill {
    /// Ordered cycle of directed edge references forming the boundary.
    /// `EdgeId::NONE` entries act as separators between outer contour and hole contours.
    pub boundary: Vec<(EdgeId, Direction)>,
    pub color: Option<ShapeColor>,
    pub fill_rule: FillRule,
    #[serde(default)]
    pub gradient_fill: Option<crate::gradient::ShapeGradient>,
    #[serde(default)]
    pub image_fill: Option<uuid::Uuid>,
    pub deleted: bool,
}

// ---------------------------------------------------------------------------
// Placeholder types (to be replaced with real imports)
// ---------------------------------------------------------------------------

// Re-export from shape module when wired up; for now define minimal versions
// so tests can compile.
pub use crate::shape::{FillRule, ShapeColor, StrokeStyle};

// ---------------------------------------------------------------------------
// VectorGraph container
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorGraph {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<Edge>,
    pub fills: Vec<Fill>,

    free_vertices: Vec<u32>,
    free_edges: Vec<u32>,
    free_fills: Vec<u32>,
}

impl Default for VectorGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorGraph {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            edges: Vec::new(),
            fills: Vec::new(),
            free_vertices: Vec::new(),
            free_edges: Vec::new(),
            free_fills: Vec::new(),
        }
    }

    // -------------------------------------------------------------------
    // Allocation
    // -------------------------------------------------------------------

    pub fn alloc_vertex(&mut self, position: Point) -> VertexId {
        if let Some(idx) = self.free_vertices.pop() {
            let id = VertexId(idx);
            self.vertices[id.idx()] = Vertex {
                position,
                deleted: false,
            };
            id
        } else {
            let id = VertexId(self.vertices.len() as u32);
            self.vertices.push(Vertex {
                position,
                deleted: false,
            });
            id
        }
    }

    pub fn alloc_edge(
        &mut self,
        curve: CubicBez,
        v0: VertexId,
        v1: VertexId,
        stroke_style: Option<StrokeStyle>,
        stroke_color: Option<ShapeColor>,
    ) -> EdgeId {
        let edge = Edge {
            curve,
            vertices: [v0, v1],
            stroke_style,
            stroke_color,
            deleted: false,
        };
        if let Some(idx) = self.free_edges.pop() {
            let id = EdgeId(idx);
            self.edges[id.idx()] = edge;
            id
        } else {
            let id = EdgeId(self.edges.len() as u32);
            self.edges.push(edge);
            id
        }
    }

    pub fn alloc_fill(
        &mut self,
        boundary: Vec<(EdgeId, Direction)>,
        color: impl Into<Option<ShapeColor>>,
        fill_rule: FillRule,
    ) -> FillId {
        let fill = Fill {
            boundary,
            color: color.into(),
            fill_rule,
            gradient_fill: None,
            image_fill: None,
            deleted: false,
        };
        if let Some(idx) = self.free_fills.pop() {
            let id = FillId(idx);
            self.fills[id.idx()] = fill;
            id
        } else {
            let id = FillId(self.fills.len() as u32);
            self.fills.push(fill);
            id
        }
    }

    // -------------------------------------------------------------------
    // Deallocation
    // -------------------------------------------------------------------

    pub fn free_vertex(&mut self, id: VertexId) {
        debug_assert!(!id.is_none());
        self.vertices[id.idx()].deleted = true;
        self.free_vertices.push(id.0);
    }

    pub fn free_edge(&mut self, id: EdgeId) {
        debug_assert!(!id.is_none());
        self.edges[id.idx()].deleted = true;
        self.free_edges.push(id.0);
    }

    pub fn free_fill(&mut self, id: FillId) {
        debug_assert!(!id.is_none());
        self.fills[id.idx()].deleted = true;
        self.free_fills.push(id.0);
    }

    // -------------------------------------------------------------------
    // Accessors
    // -------------------------------------------------------------------

    #[inline]
    pub fn vertex(&self, id: VertexId) -> &Vertex {
        &self.vertices[id.idx()]
    }

    #[inline]
    pub fn vertex_mut(&mut self, id: VertexId) -> &mut Vertex {
        &mut self.vertices[id.idx()]
    }

    #[inline]
    pub fn edge(&self, id: EdgeId) -> &Edge {
        &self.edges[id.idx()]
    }

    #[inline]
    pub fn edge_mut(&mut self, id: EdgeId) -> &mut Edge {
        &mut self.edges[id.idx()]
    }

    #[inline]
    pub fn fill(&self, id: FillId) -> &Fill {
        &self.fills[id.idx()]
    }

    #[inline]
    pub fn fill_mut(&mut self, id: FillId) -> &mut Fill {
        &mut self.fills[id.idx()]
    }

    // -------------------------------------------------------------------
    // Adjacency queries
    // -------------------------------------------------------------------

    /// Get all non-deleted edges incident to a vertex.
    pub fn edges_at_vertex(&self, vid: VertexId) -> Vec<EdgeId> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                !e.deleted && (e.vertices[0] == vid || e.vertices[1] == vid)
            })
            .map(|(i, _)| EdgeId(i as u32))
            .collect()
    }

    /// Check if two vertices share an edge.
    pub fn vertices_share_edge(&self, v0: VertexId, v1: VertexId) -> bool {
        self.edges.iter().any(|e| {
            !e.deleted
                && ((e.vertices[0] == v0 && e.vertices[1] == v1)
                    || (e.vertices[0] == v1 && e.vertices[1] == v0))
        })
    }

    // -------------------------------------------------------------------
    // Edge visibility
    // -------------------------------------------------------------------

    /// Make an edge invisible (remove its stroke but keep it if fills reference it).
    pub fn make_edge_invisible(&mut self, id: EdgeId) {
        let edge = &mut self.edges[id.idx()];
        edge.stroke_style = None;
        edge.stroke_color = None;
    }

    /// Check if an edge is visible (has a stroke).
    pub fn edge_is_visible(&self, id: EdgeId) -> bool {
        let edge = &self.edges[id.idx()];
        edge.stroke_style.is_some() || edge.stroke_color.is_some()
    }

    /// Check if any fill references this edge.
    pub fn edge_has_fill_reference(&self, id: EdgeId) -> bool {
        self.fills.iter().any(|f| {
            !f.deleted && f.boundary.iter().any(|(eid, _)| *eid == id)
        })
    }

    /// Garbage-collect invisible edges that no fill references.
    pub fn gc_invisible_edges(&mut self) {
        let to_free: Vec<EdgeId> = (0..self.edges.len())
            .filter(|&i| {
                let e = &self.edges[i];
                let eid = EdgeId(i as u32);
                !e.deleted
                    && e.stroke_style.is_none()
                    && e.stroke_color.is_none()
                    && !self.fills.iter().any(|f| {
                        !f.deleted && f.boundary.iter().any(|(fe, _)| *fe == eid)
                    })
            })
            .map(|i| EdgeId(i as u32))
            .collect();

        for eid in to_free {
            self.free_edge(eid);
        }
    }

    // -------------------------------------------------------------------
    // Fill / hit-test queries
    // -------------------------------------------------------------------

    /// Find a fill whose boundary encloses the given point.
    /// Returns the smallest (by area) enclosing fill.
    pub fn find_fill_at_point(&self, point: Point) -> Option<FillId> {
        let mut best: Option<(FillId, f64)> = None;
        for (i, fill) in self.fills.iter().enumerate() {
            if fill.deleted {
                continue;
            }
            let fid = FillId(i as u32);
            let path = self.fill_to_bezpath(fid);
            if kurbo::Shape::winding(&path, point) != 0 {
                let area = kurbo::Shape::area(&path).abs();
                if best.is_none() || area < best.unwrap().1 {
                    best = Some((fid, area));
                }
            }
        }
        best.map(|(fid, _)| fid)
    }

    /// Get the distinct edge IDs from a fill's boundary (skipping NONE separators).
    pub fn fill_boundary_edges(&self, fill_id: FillId) -> Vec<EdgeId> {
        let fill = &self.fills[fill_id.idx()];
        let mut edges = Vec::new();
        for &(eid, _) in &fill.boundary {
            if !eid.is_none() && !edges.contains(&eid) {
                edges.push(eid);
            }
        }
        edges
    }

    /// Get the distinct vertex IDs from a fill's boundary edges.
    pub fn fill_boundary_vertices(&self, fill_id: FillId) -> Vec<VertexId> {
        let mut verts = Vec::new();
        for eid in self.fill_boundary_edges(fill_id) {
            let e = &self.edges[eid.idx()];
            for &vid in &e.vertices {
                if !verts.contains(&vid) {
                    verts.push(vid);
                }
            }
        }
        verts
    }

    /// Alias for `delete_edge_by_user` — removes an edge, handling fill merging/invisibility.
    pub fn remove_edge(&mut self, id: EdgeId) {
        self.delete_edge_by_user(id);
    }

    // -------------------------------------------------------------------
    // Fill boundary → BezPath (for rendering)
    // -------------------------------------------------------------------

    /// Build a BezPath from a fill's boundary edges.
    /// Handles `EdgeId::NONE` separators to start new contours (holes).
    pub fn fill_to_bezpath(&self, fill_id: FillId) -> kurbo::BezPath {
        let fill = &self.fills[fill_id.idx()];
        self.boundary_to_bezpath(&fill.boundary)
    }

    // -------------------------------------------------------------------
    // Vertex editing
    // -------------------------------------------------------------------

    /// Update all edge curves incident to a vertex to reflect its current position.
    /// Call this after moving a vertex to keep curves in sync.
    pub fn update_edges_for_vertex(&mut self, vid: VertexId) {
        let pos = self.vertices[vid.idx()].position;
        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.vertices[0] == vid {
                edge.curve.p0 = pos;
            }
            if edge.vertices[1] == vid {
                edge.curve.p3 = pos;
            }
        }
    }

    // -------------------------------------------------------------------
    // Vertex merging
    // -------------------------------------------------------------------

    /// Replace all references to `old` with `keep` in edges and fills, then free `old`.
    pub fn merge_vertices(&mut self, keep: VertexId, old: VertexId) {
        let keep_pos = self.vertices[keep.idx()].position;
        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.vertices[0] == old {
                edge.vertices[0] = keep;
                edge.curve.p0 = keep_pos;
            }
            if edge.vertices[1] == old {
                edge.vertices[1] = keep;
                edge.curve.p3 = keep_pos;
            }
        }
        self.vertices[old.idx()].deleted = true;
        self.free_vertices.push(old.0);
    }

    /// If a vertex is within snap_epsilon of another vertex, merge them.
    /// Returns the surviving vertex ID.
    pub fn try_merge_vertex(&mut self, vid: VertexId, snap_epsilon: f64) -> VertexId {
        let pos = self.vertices[vid.idx()].position;
        let eps_sq = snap_epsilon * snap_epsilon;
        let mut best: Option<(VertexId, f64)> = None;
        for (i, v) in self.vertices.iter().enumerate() {
            let other = VertexId(i as u32);
            if v.deleted || other == vid {
                continue;
            }
            let dx = v.position.x - pos.x;
            let dy = v.position.y - pos.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < eps_sq {
                if best.is_none() || dist_sq < best.unwrap().1 {
                    best = Some((other, dist_sq));
                }
            }
        }
        if let Some((keep, _)) = best {
            self.merge_vertices(keep, vid);
            keep
        } else {
            vid
        }
    }

    // -------------------------------------------------------------------
    // Helper: snap to existing vertex
    // -------------------------------------------------------------------

    /// Find the nearest non-deleted vertex within epsilon of a point.
    pub fn snap_vertex(&self, point: Point, epsilon: f64) -> Option<VertexId> {
        let eps_sq = epsilon * epsilon;
        let mut best: Option<(VertexId, f64)> = None;
        for (i, v) in self.vertices.iter().enumerate() {
            if v.deleted {
                continue;
            }
            let dx = v.position.x - point.x;
            let dy = v.position.y - point.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < eps_sq {
                if best.is_none() || dist_sq < best.unwrap().1 {
                    best = Some((VertexId(i as u32), dist_sq));
                }
            }
        }
        best.map(|(id, _)| id)
    }

    // -------------------------------------------------------------------
    // Topology operations
    // -------------------------------------------------------------------

    /// Split an edge at parameter t, creating a new vertex and replacing
    /// the edge with two sub-edges. Updates any fills that reference the
    /// original edge.
    ///
    /// The original edge_id is modified in-place to become sub_a (the first half).
    /// A new edge is allocated for sub_b (the second half).
    pub fn split_edge(&mut self, edge_id: EdgeId, t: f64) -> (VertexId, EdgeId, EdgeId) {
        let edge = &self.edges[edge_id.idx()];
        let original_v0 = edge.vertices[0];
        let original_v1 = edge.vertices[1];
        let style = edge.stroke_style.clone();
        let color = edge.stroke_color;
        let curve = edge.curve;

        let (left, right) = subdivide_cubic(curve, t);
        let mid_v = self.alloc_vertex(left.p3);

        // Allocate both sub-edges as new edges
        let sub_a = self.alloc_edge(left, original_v0, mid_v, style.clone(), color);
        let sub_b = self.alloc_edge(right, mid_v, original_v1, style, color);

        // Update fills before freeing the old edge
        self.update_fills_after_split(edge_id, sub_a, sub_b);

        // Free the original edge
        self.edges[edge_id.idx()].deleted = true;
        self.free_edges.push(edge_id.0);

        (mid_v, sub_a, sub_b)
    }

    /// Update fill boundaries after an edge has been split into two sub-edges.
    /// Replaces (old_edge, dir) with [(sub_a, dir), (sub_b, dir)] in all fills.
    pub fn update_fills_after_split(
        &mut self,
        old_edge: EdgeId,
        sub_a: EdgeId,
        sub_b: EdgeId,
    ) {
        for fill in &mut self.fills {
            if fill.deleted {
                continue;
            }
            let mut new_boundary = Vec::with_capacity(fill.boundary.len() + 1);
            let mut changed = false;
            for &(eid, dir) in &fill.boundary {
                if eid == old_edge {
                    changed = true;
                    match dir {
                        Direction::Forward => {
                            new_boundary.push((sub_a, Direction::Forward));
                            new_boundary.push((sub_b, Direction::Forward));
                        }
                        Direction::Backward => {
                            new_boundary.push((sub_b, Direction::Backward));
                            new_boundary.push((sub_a, Direction::Backward));
                        }
                    }
                } else {
                    new_boundary.push((eid, dir));
                }
            }
            if changed {
                fill.boundary = new_boundary;
            }
        }
    }

    /// Insert a stroke (list of cubic segments) into the graph.
    /// Finds intersections with existing edges, splits both, creates vertices.
    /// Returns the new edge IDs.
    pub fn insert_stroke(
        &mut self,
        segments: &[CubicBez],
        stroke_style: Option<StrokeStyle>,
        stroke_color: Option<ShapeColor>,
        snap_epsilon: f64,
    ) -> Vec<EdgeId> {
        const ENDPOINT_T_MARGIN: f64 = 0.01;

        if segments.is_empty() {
            return Vec::new();
        }

        // Pre-pass: check for self-intersections within the stroke segments.
        // If segment i and segment j intersect (where j > i+1, or i==j for
        // self-intersecting curves), we need to split them.
        let mut expanded_segments: Vec<CubicBez> = Vec::new();
        for seg in segments {
            // Check single-curve self-intersection
            if let Some((t1, t2, _point)) = find_cubic_self_intersection(seg) {
                // Split at both t values
                let (left, rest) = subdivide_cubic(*seg, t1);
                let remapped_t2 = (t2 - t1) / (1.0 - t1);
                let (mid, right) = subdivide_cubic(rest, remapped_t2);
                expanded_segments.push(left);
                expanded_segments.push(mid);
                expanded_segments.push(right);
            } else {
                expanded_segments.push(*seg);
            }
        }

        // Check cross-segment intersections within the stroke itself
        let mut i = 0;
        while i < expanded_segments.len() {
            let mut j = i + 2; // skip adjacent (they share an endpoint)
            while j < expanded_segments.len() {
                let ints = find_curve_intersections(&expanded_segments[i], &expanded_segments[j]);
                if let Some(ix) = ints.first() {
                    let ti = ix.t1;
                    let tj = ix.t2.unwrap_or(0.5);

                    // Split segment j first (higher index, won't shift i)
                    if tj > ENDPOINT_T_MARGIN && tj < 1.0 - ENDPOINT_T_MARGIN {
                        let (jl, jr) = subdivide_cubic(expanded_segments[j], tj);
                        expanded_segments[j] = jl;
                        expanded_segments.insert(j + 1, jr);
                    }

                    // Split segment i
                    if ti > ENDPOINT_T_MARGIN && ti < 1.0 - ENDPOINT_T_MARGIN {
                        let (il, ir) = subdivide_cubic(expanded_segments[i], ti);
                        expanded_segments[i] = il;
                        expanded_segments.insert(i + 1, ir);
                        // Don't increment j — indices shifted, restart from i
                        break;
                    }
                }
                j += 1;
            }
            i += 1;
        }

        let mut all_new_edges = Vec::new();
        let mut prev_end_vertex: Option<VertexId> = None;

        for (seg_idx, seg) in expanded_segments.iter().enumerate() {
            // Snapshot existing edge count — only intersect with pre-existing edges
            let edge_count = self.edges.len();

            // Find intersections with existing edges and split them
            let mut seg_splits: Vec<(f64, VertexId)> = Vec::new();

            for ei in 0..edge_count {
                let eid = EdgeId(ei as u32);
                if self.edges[ei].deleted {
                    continue;
                }

                let existing_curve = self.edges[ei].curve;
                let ints = find_curve_intersections(seg, &existing_curve);

                // Collect valid intersections for this existing edge
                let mut edge_hits: Vec<(f64, f64, Point)> = Vec::new();
                for ix in &ints {
                    let seg_t = ix.t1;
                    let edge_t = ix.t2.unwrap_or(0.5);

                    let seg_near_endpoint = seg_t < ENDPOINT_T_MARGIN || seg_t > 1.0 - ENDPOINT_T_MARGIN;
                    let edge_near_endpoint = edge_t < ENDPOINT_T_MARGIN || edge_t > 1.0 - ENDPOINT_T_MARGIN;

                    if edge_near_endpoint {
                        // Near endpoint of existing edge — snap to that vertex
                        if !seg_near_endpoint {
                            let vid = if edge_t < 0.5 {
                                self.edges[eid.idx()].vertices[0]
                            } else {
                                self.edges[eid.idx()].vertices[1]
                            };
                            seg_splits.push((seg_t, vid));
                        }
                        continue;
                    }

                    // The existing edge needs splitting at edge_t regardless
                    // of whether the new segment is near its endpoint
                    edge_hits.push((seg_t, edge_t, ix.point));
                }

                // Sort by edge_t descending (high-to-low splitting)
                edge_hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

                let mut head_end = 1.0;
                for (seg_t, original_edge_t, point) in edge_hits {
                    let remapped_t = original_edge_t / head_end;
                    let remapped_t = remapped_t.clamp(ENDPOINT_T_MARGIN, 1.0 - ENDPOINT_T_MARGIN);

                    let (mid_v, _sub_a, _sub_b) = self.split_edge(eid, remapped_t);
                    // Snap vertex to intersection point
                    self.vertices[mid_v.idx()].position = point;
                    // Merge with nearby existing vertex if within snap distance
                    let mid_v = self.try_merge_vertex(mid_v, snap_epsilon);
                    head_end = original_edge_t;

                    // Only add as a segment split if not near an endpoint
                    let seg_near_endpoint = seg_t < ENDPOINT_T_MARGIN || seg_t > 1.0 - ENDPOINT_T_MARGIN;
                    if !seg_near_endpoint {
                        seg_splits.push((seg_t, mid_v));
                    }
                }
            }

            // Resolve start vertex
            let seg_start_pt = seg.p0;
            let start_v = if let Some(prev) = prev_end_vertex {
                prev
            } else if let Some(vid) = self.snap_vertex(seg_start_pt, snap_epsilon) {
                vid
            } else {
                self.alloc_vertex(seg_start_pt)
            };

            // Resolve end vertex
            let seg_end_pt = seg.p3;
            let is_last = seg_idx == expanded_segments.len() - 1;
            let end_v = if !is_last {
                // Check if next segment start snaps to an existing vertex
                let next_start = expanded_segments[seg_idx + 1].p0;
                if let Some(vid) = self.snap_vertex(next_start, snap_epsilon) {
                    vid
                } else if let Some(vid) = self.snap_vertex(seg_end_pt, snap_epsilon) {
                    vid
                } else {
                    self.alloc_vertex(seg_end_pt)
                }
            } else if let Some(vid) = self.snap_vertex(seg_end_pt, snap_epsilon) {
                vid
            } else {
                self.alloc_vertex(seg_end_pt)
            };

            // Build chain: start + splits (sorted by t) + end
            seg_splits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

            // Dedup: remove splits too close together or to endpoints
            let mut chain: Vec<(f64, VertexId)> = Vec::new();
            chain.push((0.0, start_v));
            for (t, vid) in &seg_splits {
                if let Some(last) = chain.last() {
                    if (*t - last.0).abs() < ENDPOINT_T_MARGIN {
                        continue;
                    }
                    if last.1 == *vid {
                        continue;
                    }
                }
                if (1.0 - *t).abs() < ENDPOINT_T_MARGIN {
                    continue;
                }
                chain.push((*t, *vid));
            }
            chain.push((1.0, end_v));

            // Dedup consecutive same-vertex entries
            chain.dedup_by(|b, a| a.1 == b.1);

            // Create edges for each consecutive pair in the chain
            for pair in chain.windows(2) {
                let (t0, v0) = pair[0];
                let (t1, v1) = pair[1];
                let sub_curve = subsegment_cubic(*seg, t0, t1);
                // Snap curve endpoints to vertex positions
                let mut snapped = sub_curve;
                snapped.p0 = self.vertices[v0.idx()].position;
                snapped.p3 = self.vertices[v1.idx()].position;
                let eid = self.alloc_edge(snapped, v0, v1, stroke_style.clone(), stroke_color);
                all_new_edges.push(eid);
            }

            prev_end_vertex = Some(end_v);
        }

        // Fill splitting pass: for each new edge, check if both endpoints
        // lie on any fill's boundary — if so, split that fill.
        let edges_to_check = all_new_edges.clone();
        for &eid in &edges_to_check {
            let v0 = self.edges[eid.idx()].vertices[0];
            let v1 = self.edges[eid.idx()].vertices[1];

            // Find fills where both v0 and v1 appear as boundary vertices
            let fill_ids: Vec<FillId> = self.fills
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.deleted)
                .filter(|(_, f)| {
                    let has_v0 = f.boundary.iter().any(|&(be, _)| {
                        let e = &self.edges[be.idx()];
                        e.vertices[0] == v0 || e.vertices[1] == v0
                    });
                    let has_v1 = f.boundary.iter().any(|&(be, _)| {
                        let e = &self.edges[be.idx()];
                        e.vertices[0] == v1 || e.vertices[1] == v1
                    });
                    has_v0 && has_v1
                })
                .map(|(i, _)| FillId(i as u32))
                .collect();

            for fid in fill_ids {
                self.split_fill_by_edge(fid, eid);
            }
        }

        all_new_edges
    }

    /// When a new edge splits a fill (both endpoints on the fill's boundary),
    /// split the fill into two fills.
    pub fn split_fill_by_edge(
        &mut self,
        fill_id: FillId,
        splitting_edge: EdgeId,
    ) -> Option<(FillId, FillId)> {
        let fill = &self.fills[fill_id.idx()];
        if fill.deleted {
            return None;
        }

        let split_v0 = self.edges[splitting_edge.idx()].vertices[0];
        let split_v1 = self.edges[splitting_edge.idx()].vertices[1];

        // Find the positions in the boundary where the splitting edge's
        // endpoint vertices appear as the "arrival" vertex of a directed edge.
        let boundary = fill.boundary.clone();

        // Helper: get the "end" vertex of a directed boundary edge
        let end_vertex = |eid: EdgeId, dir: Direction| -> VertexId {
            match dir {
                Direction::Forward => self.edges[eid.idx()].vertices[1],
                Direction::Backward => self.edges[eid.idx()].vertices[0],
            }
        };

        // Find positions where boundary edges arrive at split_v0 and split_v1
        let mut pos_v0: Option<usize> = None;
        let mut pos_v1: Option<usize> = None;

        for (i, &(eid, dir)) in boundary.iter().enumerate() {
            let ev = end_vertex(eid, dir);
            if ev == split_v0 && pos_v0.is_none() {
                pos_v0 = Some(i);
            }
            if ev == split_v1 && pos_v1.is_none() {
                pos_v1 = Some(i);
            }
        }

        let pos_v0 = pos_v0?;
        let pos_v1 = pos_v1?;

        // Ensure we have two distinct positions
        if pos_v0 == pos_v1 {
            return None;
        }

        // Walk boundary in two halves:
        // Half A: from pos_v0+1 to pos_v1 (inclusive), then splitting_edge Forward
        // Half B: from pos_v1+1 to pos_v0 (wrapping), then splitting_edge Backward
        let n = boundary.len();
        let color = fill.color;
        let fill_rule = fill.fill_rule;

        let mut half_a = Vec::new();
        let mut idx = (pos_v0 + 1) % n;
        loop {
            half_a.push(boundary[idx]);
            if idx == pos_v1 {
                break;
            }
            idx = (idx + 1) % n;
        }
        half_a.push((splitting_edge, Direction::Forward));

        let mut half_b = Vec::new();
        idx = (pos_v1 + 1) % n;
        loop {
            half_b.push(boundary[idx]);
            if idx == pos_v0 {
                break;
            }
            idx = (idx + 1) % n;
        }
        half_b.push((splitting_edge, Direction::Backward));

        // Delete the original fill
        self.fills[fill_id.idx()].deleted = true;
        self.free_fills.push(fill_id.0);

        // Create two new fills
        let fill_a = self.alloc_fill(half_a, color, fill_rule);
        let fill_b = self.alloc_fill(half_b, color, fill_rule);

        Some((fill_a, fill_b))
    }

    /// Merge two fills that share a boundary edge (e.g., after edge deletion).
    pub fn merge_fills(&mut self, fill_a: FillId, fill_b: FillId, shared_edge: EdgeId) -> FillId {
        let boundary_a = self.fills[fill_a.idx()].boundary.clone();
        let boundary_b = self.fills[fill_b.idx()].boundary.clone();
        let color = self.fills[fill_a.idx()].color;
        let fill_rule = self.fills[fill_a.idx()].fill_rule;

        // Find position of shared_edge in both boundaries
        let pos_a = boundary_a.iter().position(|&(eid, _)| eid == shared_edge);
        let pos_b = boundary_b.iter().position(|&(eid, _)| eid == shared_edge);

        if let (Some(pa), Some(pb)) = (pos_a, pos_b) {
            // Build merged boundary: boundary_a without shared_edge + boundary_b without shared_edge
            let na = boundary_a.len();
            let nb = boundary_b.len();

            let mut merged = Vec::new();

            // Walk boundary_a starting after the shared edge
            for i in 1..na {
                merged.push(boundary_a[(pa + i) % na]);
            }
            // Walk boundary_b starting after the shared edge
            for i in 1..nb {
                merged.push(boundary_b[(pb + i) % nb]);
            }

            // Delete old fills
            self.fills[fill_a.idx()].deleted = true;
            self.free_fills.push(fill_a.0);
            self.fills[fill_b.idx()].deleted = true;
            self.free_fills.push(fill_b.0);

            self.alloc_fill(merged, color, fill_rule)
        } else {
            // Fallback: can't find shared edge, just keep fill_a
            fill_a
        }
    }

    /// Delete an edge, handling fills:
    /// - If exactly 2 fills reference it, merge them and free the edge
    /// - If 1 fill references it, make it invisible
    /// - If unreferenced, actually delete it
    pub fn delete_edge_by_user(&mut self, id: EdgeId) {
        // Find fills referencing this edge
        let referencing_fills: Vec<FillId> = self.fills
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.deleted && f.boundary.iter().any(|(eid, _)| *eid == id))
            .map(|(i, _)| FillId(i as u32))
            .collect();

        match referencing_fills.len() {
            0 => self.free_edge(id),
            1 => self.make_edge_invisible(id),
            2 => {
                self.merge_fills(referencing_fills[0], referencing_fills[1], id);
                self.free_edge(id);
            }
            _ => self.make_edge_invisible(id),
        }
    }

    /// Trace the boundary of the region enclosing a point.
    /// Returns the boundary as a list of (EdgeId, Direction) pairs,
    /// or None if no enclosed region exists.
    pub fn trace_boundary_at_point(
        &mut self,
        point: Point,
        gap_tolerance: f64,
    ) -> Option<Vec<(EdgeId, Direction)>> {
        if self.edges.iter().all(|e| e.deleted) {
            return None;
        }

        // Pre-bridge: find close approaches between non-connected edges
        // and create invisible bridge edges before tracing.
        if gap_tolerance > 0.0 {
            self.create_gap_bridges(gap_tolerance);
        }

        // Collect candidate boundaries from all nearby edges, both directions.
        // Pick the smallest (by area) that contains the point.
        let mut candidates: Vec<Vec<(EdgeId, Direction)>> = Vec::new();

        // Try tracing from every non-deleted edge, both directions
        let edge_ids: Vec<EdgeId> = self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.deleted)
            .map(|(i, _)| EdgeId(i as u32))
            .collect();

        for eid in &edge_ids {
            for &dir in &[Direction::Forward, Direction::Backward] {
                if let Some(boundary) = self.trace_boundary_walk(*eid, dir, gap_tolerance) {
                    let path = self.boundary_to_bezpath(&boundary);
                    let winding = kurbo::Shape::winding(&path, point);
                    if winding != 0 {
                        candidates.push(boundary);
                    }
                }
            }
        }

        // Pick the smallest boundary by area
        let mut outer = candidates.into_iter().min_by(|a, b| {
            let area_a = self.boundary_area(a).abs();
            let area_b = self.boundary_area(b).abs();
            area_a.partial_cmp(&area_b).unwrap()
        })?;

        // Hole detection: find edges inside the outer boundary that aren't part of it.
        // Trace inner boundaries from them and append as hole contours.
        let outer_path = self.boundary_to_bezpath(&outer);
        let outer_area = kurbo::Shape::area(&outer_path);
        let outer_edge_set: std::collections::HashSet<EdgeId> =
            outer.iter().map(|(eid, _)| *eid).collect();

        // Find edges inside the outer boundary that aren't part of it
        let interior_edges: Vec<EdgeId> = self.edges.iter().enumerate()
            .filter(|(_, e)| !e.deleted)
            .map(|(i, _)| EdgeId(i as u32))
            .filter(|eid| !outer_edge_set.contains(eid))
            .filter(|eid| {
                let c = &self.edges[eid.idx()].curve;
                let mid = c.eval(0.5);
                kurbo::Shape::winding(&outer_path, mid) != 0
            })
            .collect();

        if !interior_edges.is_empty() {
            // Trace boundaries from interior edges, collect hole contours
            let mut used_edges: std::collections::HashSet<EdgeId> = outer_edge_set;
            let mut holes: Vec<Vec<(EdgeId, Direction)>> = Vec::new();

            for &eid in &interior_edges {
                if used_edges.contains(&eid) {
                    continue;
                }
                for &dir in &[Direction::Forward, Direction::Backward] {
                    if let Some(boundary) = self.trace_boundary_walk(eid, dir, 0.0) {
                        // Check all edges in this boundary are interior
                        let all_interior = boundary.iter().all(|(e, _)| interior_edges.contains(e));
                        if !all_interior {
                            continue;
                        }
                        let area = self.boundary_area(&boundary);
                        // Hole should have opposite sign from outer boundary
                        if (area > 0.0) != (outer_area > 0.0) {
                            for (e, _) in &boundary {
                                used_edges.insert(*e);
                            }
                            holes.push(boundary);
                            break; // Only need one direction per hole
                        }
                    }
                }
            }

            // Append holes with NONE separators
            for hole in holes {
                outer.push((EdgeId::NONE, Direction::Forward));
                outer.extend(hole);
            }
        }

        Some(outer)
    }

    /// Paint bucket: trace boundary at point and create a fill.
    pub fn paint_bucket(
        &mut self,
        point: Point,
        color: ShapeColor,
        fill_rule: FillRule,
        gap_tolerance: f64,
    ) -> Option<FillId> {
        let boundary = self.trace_boundary_at_point(point, gap_tolerance)?;
        Some(self.alloc_fill(boundary, color, fill_rule))
    }

    // -------------------------------------------------------------------
    // Boundary tracing internals
    // -------------------------------------------------------------------

    /// Find the nearest non-deleted edge to a point. Returns (EdgeId, t, distance).
    fn nearest_edge_to_point(&self, point: Point) -> Option<(EdgeId, f64, f64)> {
        let mut best: Option<(EdgeId, f64, f64)> = None;
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            let eid = EdgeId(i as u32);
            let (t, dist) = nearest_point_on_cubic(&e.curve, point);
            if best.is_none() || dist < best.unwrap().2 {
                best = Some((eid, t, dist));
            }
        }
        best
    }

    /// Build a BezPath from a boundary (without storing it as a fill).
    /// Handles `EdgeId::NONE` separators to start new contours (holes).
    fn boundary_to_bezpath(&self, boundary: &[(EdgeId, Direction)]) -> kurbo::BezPath {
        let mut path = kurbo::BezPath::new();
        if boundary.is_empty() {
            return path;
        }
        let mut contour_started = false;
        for &(eid, dir) in boundary {
            if eid.is_none() {
                // Separator: close current contour and start a new one
                if contour_started {
                    path.close_path();
                    contour_started = false;
                }
                continue;
            }
            let c = &self.edges[eid.idx()].curve;
            match dir {
                Direction::Forward => {
                    if !contour_started {
                        path.move_to(c.p0);
                        contour_started = true;
                    }
                    path.curve_to(c.p1, c.p2, c.p3);
                }
                Direction::Backward => {
                    if !contour_started {
                        path.move_to(c.p3);
                        contour_started = true;
                    }
                    path.curve_to(c.p2, c.p1, c.p0);
                }
            }
        }
        if contour_started {
            path.close_path();
        }
        path
    }

    /// Pre-create invisible bridge edges for close approaches between
    /// non-connected edges. Edges that share a vertex are skipped
    /// (connected geometry should never be bridged).
    fn create_gap_bridges(&mut self, gap_tolerance: f64) {
        use crate::curve_intersections::find_closest_approach;

        let edge_count = self.edges.len();
        let edge_data: Vec<(EdgeId, CubicBez, VertexId, VertexId)> = (0..edge_count)
            .filter(|&i| !self.edges[i].deleted)
            .map(|i| {
                let e = &self.edges[i];
                (EdgeId(i as u32), e.curve, e.vertices[0], e.vertices[1])
            })
            .collect();

        let mut bridges: Vec<(Point, Point, EdgeId, f64, EdgeId, f64)> = Vec::new();

        for i in 0..edge_data.len() {
            for j in (i + 1)..edge_data.len() {
                let (eid_i, curve_i, vi0, vi1) = &edge_data[i];
                let (eid_j, curve_j, vj0, vj1) = &edge_data[j];

                // Skip if edges share a vertex (connected geometry)
                if vi0 == vj0 || vi0 == vj1 || vi1 == vj0 || vi1 == vj1 {
                    continue;
                }

                if let Some(approach) = find_closest_approach(curve_i, curve_j, gap_tolerance) {
                    bridges.push((
                        approach.p1, approach.p2,
                        *eid_i, approach.t1,
                        *eid_j, approach.t2,
                    ));
                }
            }
        }
        const ENDPOINT_MARGIN: f64 = 0.05;

        for (p1, p2, eid_i, t_i, eid_j, t_j) in bridges {
            // Resolve vertex for edge i: snap to endpoint or split
            let v_i = if t_i < ENDPOINT_MARGIN {
                self.edges[eid_i.idx()].vertices[0]
            } else if t_i > 1.0 - ENDPOINT_MARGIN {
                self.edges[eid_i.idx()].vertices[1]
            } else {
                let (mid, _, _) = self.split_edge(eid_i, t_i);
                self.vertices[mid.idx()].position = p1;
                self.update_edges_for_vertex(mid);
                mid
            };

            // Resolve vertex for edge j: snap to endpoint or split
            let v_j = if t_j < ENDPOINT_MARGIN {
                self.edges[eid_j.idx()].vertices[0]
            } else if t_j > 1.0 - ENDPOINT_MARGIN {
                self.edges[eid_j.idx()].vertices[1]
            } else {
                let (mid, _, _) = self.split_edge(eid_j, t_j);
                self.vertices[mid.idx()].position = p2;
                self.update_edges_for_vertex(mid);
                mid
            };

            let pa = self.vertices[v_i.idx()].position;
            let pb = self.vertices[v_j.idx()].position;
            let bridge_curve = line_cubic(pa, pb);
            self.alloc_edge(bridge_curve, v_i, v_j, None, None);
        }
    }

    /// Compute the signed area of a boundary using the shoelace formula
    /// on a linearized version of the path.
    fn boundary_area(&self, boundary: &[(EdgeId, Direction)]) -> f64 {
        let path = self.boundary_to_bezpath(boundary);
        kurbo::Shape::area(&path)
    }

    /// Walk the boundary starting from a given edge+direction, using CCW angle
    /// selection at each vertex. Returns None if the walk doesn't form a cycle
    /// or exceeds a maximum length.
    fn trace_boundary_walk(
        &mut self,
        start_edge: EdgeId,
        start_dir: Direction,
        gap_tolerance: f64,
    ) -> Option<Vec<(EdgeId, Direction)>> {
        let max_boundary_len = self.edges.len() * 2 + 10;
        let mut boundary = Vec::new();
        let mut current_edge = start_edge;
        let mut current_dir = start_dir;

        loop {
            boundary.push((current_edge, current_dir));
            if boundary.len() > max_boundary_len {
                return None; // Not converging
            }

            // The vertex we arrive at
            let arrival_vertex = match current_dir {
                Direction::Forward => self.edges[current_edge.idx()].vertices[1],
                Direction::Backward => self.edges[current_edge.idx()].vertices[0],
            };

            // The incoming angle (reversed — the direction we came from)
            let incoming_angle = outgoing_angle_from_vertex(
                &self.edges[current_edge.idx()].curve,
                current_edge,
                arrival_vertex,
                &self.edges,
                match current_dir {
                    // We arrived going forward, so at vertex[1] the "outgoing" in reverse is backward
                    Direction::Forward => Direction::Backward,
                    Direction::Backward => Direction::Forward,
                },
            );

            // Find all edges at this vertex
            let incident: Vec<EdgeId> = self.edges
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    !e.deleted && (e.vertices[0] == arrival_vertex || e.vertices[1] == arrival_vertex)
                })
                .map(|(i, _)| EdgeId(i as u32))
                .collect();

            // For each incident edge (excluding the one we came from on the same side),
            // compute the outgoing angle and pick the most clockwise turn (smallest CCW angle).
            let mut best_next: Option<(EdgeId, Direction, f64)> = None;

            for &eid in &incident {
                let edge = &self.edges[eid.idx()];
                // Determine direction(s) we can leave this vertex on this edge
                let mut dirs = Vec::new();
                if edge.vertices[0] == arrival_vertex {
                    dirs.push(Direction::Forward);
                }
                if edge.vertices[1] == arrival_vertex {
                    dirs.push(Direction::Backward);
                }

                for dir in dirs {
                    // Don't go back the way we came.
                    // If we arrived Forward (at vertices[1]), going back is Backward.
                    // If we arrived Backward (at vertices[0]), going back is Forward.
                    // In both cases, "going back" = same edge, opposite direction.
                    if eid == current_edge {
                        let reverse_dir = match current_dir {
                            Direction::Forward => Direction::Backward,
                            Direction::Backward => Direction::Forward,
                        };
                        if dir == reverse_dir {
                            continue;
                        }
                    }

                    let out_angle = outgoing_angle_from_vertex(
                        &edge.curve,
                        eid,
                        arrival_vertex,
                        &self.edges,
                        dir,
                    );

                    // CCW angle from incoming direction
                    let mut delta = out_angle - incoming_angle;
                    if delta <= 0.0 {
                        delta += std::f64::consts::TAU;
                    }
                    // We want the smallest positive CCW turn (most clockwise)
                    if best_next.is_none() || delta < best_next.unwrap().2 {
                        best_next = Some((eid, dir, delta));
                    }
                }
            }

            if let Some((next_edge, next_dir, _delta)) = best_next {
                current_edge = next_edge;
                current_dir = next_dir;
            } else if gap_tolerance > 0.0 {
                // Dead end — try gap bridging
                if let Some((bridge_edge, bridge_dir)) =
                    self.try_gap_bridge(arrival_vertex, current_edge, gap_tolerance)
                {
                    current_edge = bridge_edge;
                    current_dir = bridge_dir;
                } else {
                    return None;
                }
            } else {
                return None; // Dead end with no gap tolerance
            }

            // Check if we've returned to the start
            if current_edge == start_edge && current_dir == start_dir {
                break;
            }
        }

        if boundary.len() >= 2 {
            Some(boundary)
        } else {
            None
        }
    }

    /// Try to bridge a gap at a dead-end vertex during boundary tracing.
    /// Creates an invisible edge to the nearest reachable vertex/edge
    /// that doesn't already share a vertex with the current edge.
    fn try_gap_bridge(
        &mut self,
        from_vertex: VertexId,
        current_edge: EdgeId,
        gap_tolerance: f64,
    ) -> Option<(EdgeId, Direction)> {
        let from_pos = self.vertices[from_vertex.idx()].position;
        let gap_tol_sq = gap_tolerance * gap_tolerance;

        // Find the current edge's vertices (to avoid bridging back to connected geometry)
        let current_v0 = self.edges[current_edge.idx()].vertices[0];
        let current_v1 = self.edges[current_edge.idx()].vertices[1];

        // Strategy 1: Find nearest vertex within tolerance that doesn't share
        // a vertex with the current edge
        let mut best_vertex: Option<(VertexId, f64)> = None;
        for (i, v) in self.vertices.iter().enumerate() {
            if v.deleted {
                continue;
            }
            let vid = VertexId(i as u32);
            if vid == from_vertex || vid == current_v0 || vid == current_v1 {
                continue;
            }
            // Check this vertex isn't connected to from_vertex
            if self.vertices_share_edge(from_vertex, vid) {
                continue;
            }
            let dx = v.position.x - from_pos.x;
            let dy = v.position.y - from_pos.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < gap_tol_sq {
                if best_vertex.is_none() || dist_sq < best_vertex.unwrap().1 {
                    best_vertex = Some((vid, dist_sq));
                }
            }
        }

        // Strategy 2: Find nearest point on any edge (mid-curve gap close)
        let mut best_edge_approach: Option<(EdgeId, f64, f64, Point)> = None; // (eid, t, dist_sq, point)
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            let eid = EdgeId(i as u32);
            // Skip edges connected to from_vertex
            if e.vertices[0] == from_vertex || e.vertices[1] == from_vertex {
                continue;
            }
            // Skip edges connected to current edge's other vertex
            if e.vertices[0] == current_v0 || e.vertices[1] == current_v0
                || e.vertices[0] == current_v1 || e.vertices[1] == current_v1
            {
                continue;
            }
            let (t, dist) = nearest_point_on_cubic(&e.curve, from_pos);
            let dist_sq = dist * dist;
            if dist_sq < gap_tol_sq {
                if best_edge_approach.is_none() || dist_sq < best_edge_approach.as_ref().unwrap().2 {
                    let pt = e.curve.eval(t);
                    best_edge_approach = Some((eid, t, dist_sq, pt));
                }
            }
        }

        // Pick the closer option
        let vertex_dist_sq = best_vertex.map(|(_, d)| d).unwrap_or(f64::MAX);
        let edge_dist_sq = best_edge_approach.as_ref().map(|x| x.2).unwrap_or(f64::MAX);

        if vertex_dist_sq < edge_dist_sq {
            if let Some((target_vid, _)) = best_vertex {
                // Create invisible bridge edge
                let target_pos = self.vertices[target_vid.idx()].position;
                let bridge_curve = line_cubic(from_pos, target_pos);
                let bridge = self.alloc_edge(bridge_curve, from_vertex, target_vid, None, None);
                return Some((bridge, Direction::Forward));
            }
        } else if let Some((target_eid, t, _, point)) = best_edge_approach {
            // Split the target edge at the closest point, then bridge to the new vertex
            let (mid_v, _sub_a, _sub_b) = self.split_edge(target_eid, t);
            self.vertices[mid_v.idx()].position = point;
            let bridge_curve = line_cubic(from_pos, point);
            let bridge = self.alloc_edge(bridge_curve, from_vertex, mid_v, None, None);
            return Some((bridge, Direction::Forward));
        }

        None
    }

    // ── Region selection: extract / merge subgraph ──────────────────────

    /// Extract a subgraph containing `inside_edges` and `inside_fills` (typically a
    /// geometry selection — `select_fill` already includes each fill's boundary edges).
    ///
    /// **Boundary edges** are *duplicated* (copied into the returned graph but kept in
    /// `self`, so remaining shapes keep closed boundaries). They are `explicit_boundary`
    /// (a cut the caller knows about, e.g. a lasso region — pass an empty set if none)
    /// UNION any inside edge still shared with a non-extracted fill (derived here, so a
    /// plain geometry selection needs no boundary analysis from the caller).
    ///
    /// Returns `(new_graph, vertex_map, edge_map)` where the maps go from
    /// old (self) IDs to new (returned graph) IDs.
    pub fn extract_subgraph(
        &mut self,
        inside_edges: &HashSet<EdgeId>,
        inside_fills: &HashSet<FillId>,
        explicit_boundary: &HashSet<EdgeId>,
    ) -> (VectorGraph, HashMap<VertexId, VertexId>, HashMap<EdgeId, EdgeId>) {
        let mut new_graph = VectorGraph::new();
        let mut vtx_map: HashMap<VertexId, VertexId> = HashMap::new();
        let mut edge_map: HashMap<EdgeId, EdgeId> = HashMap::new();

        // Boundary = `explicit_boundary` (e.g. a region/lasso cut the caller knows about)
        // UNION any inside edge still referenced by a fill we're NOT extracting (a shared
        // DCEL edge — must be duplicated, not moved, or that fill dangles). Deriving the
        // latter here means a plain geometry selection needs no boundary analysis.
        let mut boundary_edge_ids: HashSet<EdgeId> = explicit_boundary.clone();
        for (i, fill) in self.fills.iter().enumerate() {
            if fill.deleted || inside_fills.contains(&FillId(i as u32)) {
                continue;
            }
            for &(eid, _) in &fill.boundary {
                if !eid.is_none() && inside_edges.contains(&eid) {
                    boundary_edge_ids.insert(eid);
                }
            }
        }
        let boundary_edge_ids = &boundary_edge_ids;

        // Copy all inside edges + any boundary edges (the explicit ones may not be in
        // inside_edges); boundary edges are kept in self below.
        let edges_to_copy: HashSet<EdgeId> = inside_edges.union(boundary_edge_ids).copied().collect();

        // Collect all vertices referenced by edges we're copying
        let mut referenced_vids: HashSet<VertexId> = HashSet::new();
        for &eid in &edges_to_copy {
            if eid.is_none() || self.edges[eid.idx()].deleted {
                continue;
            }
            for &vid in &self.edges[eid.idx()].vertices {
                referenced_vids.insert(vid);
            }
        }

        // Determine which vertices are interior (exclusively owned by the
        // extracted subgraph) vs boundary (shared with remaining geometry).
        // A vertex is interior if ALL of its incident edges are either in
        // inside_edges or boundary_edge_ids.
        let mut interior_vertices: HashSet<VertexId> = HashSet::new();
        let mut boundary_vertices: HashSet<VertexId> = HashSet::new();
        for &vid in &referenced_vids {
            let incident = self.edges_at_vertex(vid);
            let all_inside = incident.iter().all(|&eid| edges_to_copy.contains(&eid));
            if all_inside {
                interior_vertices.insert(vid);
            } else {
                boundary_vertices.insert(vid);
            }
        }

        // Allocate vertices in new graph
        for &vid in &referenced_vids {
            let pos = self.vertices[vid.idx()].position;
            let new_vid = new_graph.alloc_vertex(pos);
            vtx_map.insert(vid, new_vid);
        }

        // Copy edges into new graph
        for &eid in &edges_to_copy {
            if eid.is_none() || self.edges[eid.idx()].deleted {
                continue;
            }
            let edge = &self.edges[eid.idx()];
            let new_v0 = vtx_map[&edge.vertices[0]];
            let new_v1 = vtx_map[&edge.vertices[1]];
            let new_eid = new_graph.alloc_edge(
                edge.curve,
                new_v0,
                new_v1,
                edge.stroke_style.clone(),
                edge.stroke_color.clone(),
            );
            edge_map.insert(eid, new_eid);
        }

        // Copy inside fills into new graph
        for &fid in inside_fills {
            if fid.is_none() || self.fills[fid.idx()].deleted {
                continue;
            }
            let fill = &self.fills[fid.idx()];
            let new_boundary: Vec<(EdgeId, Direction)> = fill
                .boundary
                .iter()
                .map(|&(eid, dir)| {
                    if eid.is_none() {
                        (EdgeId::NONE, dir)
                    } else if let Some(&new_eid) = edge_map.get(&eid) {
                        (new_eid, dir)
                    } else {
                        // Edge referenced by fill but not in edges_to_copy —
                        // shouldn't happen if classification is correct, but
                        // skip gracefully.
                        (EdgeId::NONE, dir)
                    }
                })
                .collect();
            let new_fid = new_graph.alloc_fill(
                new_boundary,
                fill.color,
                fill.fill_rule,
            );
            // Copy gradient and image fill
            new_graph.fills[new_fid.idx()].gradient_fill = fill.gradient_fill.clone();
            new_graph.fills[new_fid.idx()].image_fill = fill.image_fill;
        }

        // Remove inside_edges from self, EXCEPT boundary edges (those are duplicated —
        // a non-extracted fill still needs them).
        for &eid in inside_edges {
            if !eid.is_none() && !boundary_edge_ids.contains(&eid) && !self.edges[eid.idx()].deleted {
                self.free_edge(eid);
            }
        }

        // Remove inside fills from self
        for &fid in inside_fills {
            if !fid.is_none() && !self.fills[fid.idx()].deleted {
                self.free_fill(fid);
            }
        }

        // Free interior vertices (they have no remaining edges in self)
        for &vid in &interior_vertices {
            self.free_vertex(vid);
        }

        (new_graph, vtx_map, edge_map)
    }

    /// Merge another graph back into `self`, applying `transform` to all geometry.
    ///
    /// `boundary_vertex_map` maps vertex IDs in `other` to existing vertex IDs in
    /// `self` (shared boundary vertices that should reconnect rather than duplicate).
    ///
    /// `boundary_edge_map` maps edge IDs in `other` to existing edge IDs in `self`
    /// (duplicated boundary edges that should be skipped — `self` already has them).
    pub fn merge_subgraph(
        &mut self,
        other: &VectorGraph,
        transform: kurbo::Affine,
        boundary_vertex_map: &HashMap<VertexId, VertexId>,
        boundary_edge_map: &HashMap<EdgeId, EdgeId>,
    ) {
        let mut vtx_map: HashMap<VertexId, VertexId> = HashMap::new();
        let mut edge_map: HashMap<EdgeId, EdgeId> = HashMap::new();

        // Map or allocate vertices
        for (i, vertex) in other.vertices.iter().enumerate() {
            let other_vid = VertexId(i as u32);
            if vertex.deleted {
                continue;
            }
            if let Some(&self_vid) = boundary_vertex_map.get(&other_vid) {
                vtx_map.insert(other_vid, self_vid);
            } else {
                let pos = transform * vertex.position;
                let new_vid = self.alloc_vertex(pos);
                vtx_map.insert(other_vid, new_vid);
            }
        }

        // Map or allocate edges
        for (i, edge) in other.edges.iter().enumerate() {
            let other_eid = EdgeId(i as u32);
            if edge.deleted {
                continue;
            }
            if let Some(&self_eid) = boundary_edge_map.get(&other_eid) {
                edge_map.insert(other_eid, self_eid);
            } else {
                let new_v0 = vtx_map[&edge.vertices[0]];
                let new_v1 = vtx_map[&edge.vertices[1]];
                // Transform the curve control points
                let curve = CubicBez::new(
                    transform * edge.curve.p0,
                    transform * edge.curve.p1,
                    transform * edge.curve.p2,
                    transform * edge.curve.p3,
                );
                let new_eid = self.alloc_edge(
                    curve,
                    new_v0,
                    new_v1,
                    edge.stroke_style.clone(),
                    edge.stroke_color.clone(),
                );
                edge_map.insert(other_eid, new_eid);
            }
        }

        // Copy fills
        for (_i, fill) in other.fills.iter().enumerate() {
            if fill.deleted {
                continue;
            }
            let new_boundary: Vec<(EdgeId, Direction)> = fill
                .boundary
                .iter()
                .map(|&(eid, dir)| {
                    if eid.is_none() {
                        (EdgeId::NONE, dir)
                    } else if let Some(&new_eid) = edge_map.get(&eid) {
                        (new_eid, dir)
                    } else {
                        (EdgeId::NONE, dir)
                    }
                })
                .collect();
            let new_fid = self.alloc_fill(new_boundary, fill.color, fill.fill_rule);
            self.fills[new_fid.idx()].gradient_fill = fill.gradient_fill.clone();
            self.fills[new_fid.idx()].image_fill = fill.image_fill;
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions: curve utilities
// ---------------------------------------------------------------------------

/// De Casteljau subdivision of a cubic Bézier at parameter t.
fn subdivide_cubic(c: CubicBez, t: f64) -> (CubicBez, CubicBez) {
    let p01 = lerp_point(c.p0, c.p1, t);
    let p12 = lerp_point(c.p1, c.p2, t);
    let p23 = lerp_point(c.p2, c.p3, t);
    let p012 = lerp_point(p01, p12, t);
    let p123 = lerp_point(p12, p23, t);
    let p0123 = lerp_point(p012, p123, t);
    (
        CubicBez::new(c.p0, p01, p012, p0123),
        CubicBez::new(p0123, p123, p23, c.p3),
    )
}

/// Extract a sub-curve for parameter range [t0, t1].
fn subsegment_cubic(c: CubicBez, t0: f64, t1: f64) -> CubicBez {
    const EPS: f64 = 1e-9;
    if t0 < EPS && t1 > 1.0 - EPS {
        return c;
    }
    if t0 < EPS {
        return subdivide_cubic(c, t1).0;
    }
    if t1 > 1.0 - EPS {
        return subdivide_cubic(c, t0).1;
    }
    let (_, upper) = subdivide_cubic(c, t0);
    let remapped = (t1 - t0) / (1.0 - t0);
    subdivide_cubic(upper, remapped).0
}

#[inline]
fn lerp_point(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Create a straight-line cubic Bézier from a to b.
fn line_cubic(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        lerp_point(a, b, 1.0 / 3.0),
        lerp_point(a, b, 2.0 / 3.0),
        b,
    )
}

/// Algebraic self-intersection detection for a cubic Bézier.
fn find_cubic_self_intersection(curve: &CubicBez) -> Option<(f64, f64, Point)> {
    const ENDPOINT_T_MARGIN: f64 = 0.01;

    let ax = curve.p1.x - curve.p0.x;
    let ay = curve.p1.y - curve.p0.y;
    let bx = curve.p2.x - 2.0 * curve.p1.x + curve.p0.x;
    let by = curve.p2.y - 2.0 * curve.p1.y + curve.p0.y;
    let cx = curve.p3.x - 3.0 * curve.p2.x + 3.0 * curve.p1.x - curve.p0.x;
    let cy = curve.p3.y - 3.0 * curve.p2.y + 3.0 * curve.p1.y - curve.p0.y;

    let b_cross_c = bx * cy - by * cx;
    if b_cross_c.abs() < 1e-10 {
        return None;
    }

    let a_cross_c = ax * cy - ay * cx;
    let s = -a_cross_c / b_cross_c;

    // Back-substitute for p
    let p = if cx.abs() > cy.abs() {
        if cx.abs() < 1e-10 {
            return None;
        }
        s * s + (3.0 * bx * s + 3.0 * ax) / cx
    } else {
        if cy.abs() < 1e-10 {
            return None;
        }
        s * s + (3.0 * by * s + 3.0 * ay) / cy
    };

    let disc = s * s - 4.0 * p;
    if disc < 0.0 {
        return None;
    }

    let sqrt_disc = disc.sqrt();
    let t1 = (s - sqrt_disc) / 2.0;
    let t2 = (s + sqrt_disc) / 2.0;

    if t1 <= ENDPOINT_T_MARGIN || t2 >= 1.0 - ENDPOINT_T_MARGIN || t1 >= t2 {
        return None;
    }

    let pt1 = curve.eval(t1);
    let pt2 = curve.eval(t2);
    let point = Point::new((pt1.x + pt2.x) / 2.0, (pt1.y + pt2.y) / 2.0);
    Some((t1, t2, point))
}

/// Compute the tangent of a cubic at parameter t (unnormalized).
fn cubic_tangent(c: &CubicBez, t: f64) -> Point {
    let mt = 1.0 - t;
    let x = 3.0 * (mt * mt * (c.p1.x - c.p0.x)
        + 2.0 * mt * t * (c.p2.x - c.p1.x)
        + t * t * (c.p3.x - c.p2.x));
    let y = 3.0 * (mt * mt * (c.p1.y - c.p0.y)
        + 2.0 * mt * t * (c.p2.y - c.p1.y)
        + t * t * (c.p3.y - c.p2.y));
    Point::new(x, y)
}

/// Compute the outgoing angle of an edge leaving a vertex.
fn outgoing_angle_from_vertex(
    curve: &CubicBez,
    _edge_id: EdgeId,
    _vertex: VertexId,
    edges: &[Edge],
    dir: Direction,
) -> f64 {
    let _ = edges;
    let tangent = match dir {
        Direction::Forward => cubic_tangent(curve, 0.0),
        Direction::Backward => {
            let t = cubic_tangent(curve, 1.0);
            Point::new(-t.x, -t.y)
        }
    };
    tangent.y.atan2(tangent.x)
}

/// Find the nearest point on a cubic Bézier to a given point.
/// Returns (t, distance).
fn nearest_point_on_cubic(curve: &CubicBez, point: Point) -> (f64, f64) {
    // Sample at regular intervals, then refine with Newton's method
    let n = 32;
    let mut best_t = 0.0;
    let mut best_dist_sq = f64::MAX;

    for i in 0..=n {
        let t = i as f64 / n as f64;
        let p = curve.eval(t);
        let dx = p.x - point.x;
        let dy = p.y - point.y;
        let dist_sq = dx * dx + dy * dy;
        if dist_sq < best_dist_sq {
            best_dist_sq = dist_sq;
            best_t = t;
        }
    }

    // Newton refinement
    for _ in 0..8 {
        let p = curve.eval(best_t);
        let d = cubic_tangent(curve, best_t);
        let diff = Point::new(p.x - point.x, p.y - point.y);
        let dot = diff.x * d.x + diff.y * d.y;
        let d2 = d.x * d.x + d.y * d.y;
        if d2.abs() < 1e-12 {
            break;
        }
        let dt = -dot / d2;
        best_t = (best_t + dt).clamp(0.0, 1.0);
    }

    let p = curve.eval(best_t);
    let dx = p.x - point.x;
    let dy = p.y - point.y;
    (best_t, (dx * dx + dy * dy).sqrt())
}

/// Convert a BezPath into groups of cubic Bézier segments (one group per subpath).
pub fn bezpath_to_cubic_segments(path: &kurbo::BezPath) -> Vec<Vec<CubicBez>> {
    use kurbo::PathEl;

    let mut result: Vec<Vec<CubicBez>> = Vec::new();
    let mut current: Vec<CubicBez> = Vec::new();
    let mut subpath_start = Point::ZERO;
    let mut cursor = Point::ZERO;

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
                subpath_start = p;
                cursor = p;
            }
            PathEl::LineTo(p) => {
                let c1 = lerp_point(cursor, p, 1.0 / 3.0);
                let c2 = lerp_point(cursor, p, 2.0 / 3.0);
                current.push(CubicBez::new(cursor, c1, c2, p));
                cursor = p;
            }
            PathEl::QuadTo(p1, p2) => {
                let cp1 = Point::new(
                    cursor.x + (2.0 / 3.0) * (p1.x - cursor.x),
                    cursor.y + (2.0 / 3.0) * (p1.y - cursor.y),
                );
                let cp2 = Point::new(
                    p2.x + (2.0 / 3.0) * (p1.x - p2.x),
                    p2.y + (2.0 / 3.0) * (p1.y - p2.y),
                );
                current.push(CubicBez::new(cursor, cp1, cp2, p2));
                cursor = p2;
            }
            PathEl::CurveTo(p1, p2, p3) => {
                current.push(CubicBez::new(cursor, p1, p2, p3));
                cursor = p3;
            }
            PathEl::ClosePath => {
                let dist = ((cursor.x - subpath_start.x).powi(2)
                    + (cursor.y - subpath_start.y).powi(2))
                .sqrt();
                if dist > 1e-9 {
                    let c1 = lerp_point(cursor, subpath_start, 1.0 / 3.0);
                    let c2 = lerp_point(cursor, subpath_start, 2.0 / 3.0);
                    current.push(CubicBez::new(cursor, c1, c2, subpath_start));
                }
                cursor = subpath_start;
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            }
        }
    }
    if !current.is_empty() {
        result.push(current);
    }
    result
}
