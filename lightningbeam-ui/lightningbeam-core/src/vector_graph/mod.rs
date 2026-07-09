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

    /// Free any vertex no longer referenced by a non-deleted edge (e.g. after deleting a shape's
    /// edges), so stale vertices don't linger as snap targets.
    pub fn gc_isolated_vertices(&mut self) {
        let mut referenced = vec![false; self.vertices.len()];
        for e in &self.edges {
            if e.deleted {
                continue;
            }
            for v in e.vertices.iter() {
                if !v.is_none() {
                    referenced[v.idx()] = true;
                }
            }
        }
        let to_free: Vec<VertexId> = (0..self.vertices.len())
            .filter(|&i| !self.vertices[i].deleted && !referenced[i])
            .map(|i| VertexId(i as u32))
            .collect();
        for vid in to_free {
            self.free_vertex(vid);
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

    /// Interpolate toward `other` by `t` ∈ [0,1] for a same-topology shape tween.
    ///
    /// Returns `None` if the two graphs don't share identical topology — same vertex,
    /// edge and fill structure (counts, deleted flags, edge endpoints, fill boundaries).
    /// In that case the caller should hold the source keyframe instead of morphing.
    /// Vertex positions, edge curves, stroke widths and stroke/fill colours are lerped.
    pub fn interpolated(&self, other: &VectorGraph, t: f64) -> Option<VectorGraph> {
        if self.vertices.len() != other.vertices.len()
            || self.edges.len() != other.edges.len()
            || self.fills.len() != other.fills.len()
        {
            return None;
        }
        for (a, b) in self.vertices.iter().zip(&other.vertices) {
            if a.deleted != b.deleted {
                return None;
            }
        }
        for (a, b) in self.edges.iter().zip(&other.edges) {
            if a.deleted != b.deleted || a.vertices != b.vertices {
                return None;
            }
        }
        for (a, b) in self.fills.iter().zip(&other.fills) {
            if a.deleted != b.deleted || a.boundary != b.boundary {
                return None;
            }
        }

        let lf = |x: f64, y: f64| x + (y - x) * t;
        let lp = |p: Point, q: Point| Point::new(lf(p.x, q.x), lf(p.y, q.y));
        let lc = |a: Option<ShapeColor>, b: Option<ShapeColor>| match (a, b) {
            (Some(a), Some(b)) => {
                let c = |x: u8, y: u8| (lf(x as f64, y as f64)).round().clamp(0.0, 255.0) as u8;
                Some(ShapeColor::new(c(a.r, b.r), c(a.g, b.g), c(a.b, b.b), c(a.a, b.a)))
            }
            (a, _) => a,
        };

        let mut g = self.clone();
        for (i, v) in g.vertices.iter_mut().enumerate() {
            v.position = lp(self.vertices[i].position, other.vertices[i].position);
        }
        for (i, e) in g.edges.iter_mut().enumerate() {
            let (a, b) = (self.edges[i].curve, other.edges[i].curve);
            e.curve = CubicBez::new(lp(a.p0, b.p0), lp(a.p1, b.p1), lp(a.p2, b.p2), lp(a.p3, b.p3));
            if let (Some(s), Some(sa), Some(sb)) = (
                e.stroke_style.as_mut(),
                self.edges[i].stroke_style.as_ref(),
                other.edges[i].stroke_style.as_ref(),
            ) {
                s.width = lf(sa.width, sb.width);
            }
            e.stroke_color = lc(self.edges[i].stroke_color, other.edges[i].stroke_color);
        }
        for (i, f) in g.fills.iter_mut().enumerate() {
            f.color = lc(self.fills[i].color, other.fills[i].color);
        }
        Some(g)
    }

    /// A point guaranteed to lie inside the fill — for point-in-region classification
    /// (e.g. deciding whether a fill is inside a lasso). Prefers the polygon area-centroid,
    /// but for a non-convex fill (e.g. an L-shape, where the area-centroid can fall in the
    /// concavity *outside* the shape) it steps just inward from a boundary edge instead.
    /// The naive average of boundary-edge midpoints is NOT reliable here — it can land
    /// outside a non-convex fill and misclassify it.
    pub fn fill_interior_point(&self, fill_id: FillId) -> Point {
        let boundary = self.fills[fill_id.idx()].boundary.clone();
        self.boundary_interior_point(&boundary)
    }

    /// A point guaranteed to lie inside the region enclosed by a `(edge, direction)`
    /// boundary loop. See [`fill_interior_point`].
    pub fn boundary_interior_point(&self, boundary: &[(EdgeId, Direction)]) -> Point {
        use kurbo::{ParamCurve, Shape, Vec2};
        let path = self.boundary_to_bezpath(boundary);

        // Ordered polygon corners: the directed start point of each boundary edge.
        let mut pts: Vec<Point> = Vec::new();
        for &(eid, dir) in boundary {
            if eid.is_none() {
                continue;
            }
            let c = self.edges[eid.idx()].curve;
            pts.push(match dir {
                Direction::Forward => c.p0,
                Direction::Backward => c.p3,
            });
        }
        if pts.len() < 3 {
            if pts.is_empty() {
                return Point::ZERO;
            }
            let (sx, sy) = pts.iter().fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
            return Point::new(sx / pts.len() as f64, sy / pts.len() as f64);
        }

        // Shoelace area-centroid.
        let (mut a2, mut cx, mut cy) = (0.0, 0.0, 0.0);
        for i in 0..pts.len() {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % pts.len()];
            let cross = p0.x * p1.y - p1.x * p0.y;
            a2 += cross;
            cx += (p0.x + p1.x) * cross;
            cy += (p0.y + p1.y) * cross;
        }
        if a2.abs() > 1e-9 {
            let c = Point::new(cx / (3.0 * a2), cy / (3.0 * a2));
            if path.winding(c) != 0 {
                return c;
            }
        }

        // Fallback: step a small distance inward from a boundary edge midpoint.
        let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &pts {
            minx = minx.min(p.x);
            miny = miny.min(p.y);
            maxx = maxx.max(p.x);
            maxy = maxy.max(p.y);
        }
        let eps = ((maxx - minx).min(maxy - miny) * 1e-3).max(1e-4);
        for &(eid, _) in boundary {
            if eid.is_none() {
                continue;
            }
            let c = self.edges[eid.idx()].curve;
            let mid = c.eval(0.5);
            let tangent = c.eval(0.5001) - c.eval(0.4999);
            let len = tangent.hypot();
            if len < 1e-12 {
                continue;
            }
            let n = Vec2::new(-tangent.y / len, tangent.x / len);
            for s in [1.0_f64, -1.0] {
                let cand = mid + n * (s * eps);
                if path.winding(cand) != 0 {
                    return cand;
                }
            }
        }
        pts[0]
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
        // Sub-edges produced when a stroke segment splits an existing edge (incl. an earlier
        // segment of this same stroke). Tracked separately so they reach the fill re-tracer
        // without polluting the returned edge list.
        let mut split_products: Vec<EdgeId> = Vec::new();
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

                    let (mid_v, sub_a, sub_b) = self.split_edge(eid, remapped_t);
                    // Track the split products so the fill re-tracer sees them: when a later
                    // stroke segment crosses an earlier one, these sub-edges are part of the
                    // stroke's arrangement but would otherwise be invisible to the re-trace.
                    // (Kept out of `all_new_edges` so the returned edge list stays the stroke's
                    // own edges only.)
                    split_products.push(sub_a);
                    split_products.push(sub_b);
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

        // Weld dangling stroke endpoints onto a near-coincident existing vertex. A
        // self-intersecting freehand stroke can create an intersection vertex a fraction of
        // a pixel away from a segment endpoint that should be the same point; if they don't
        // merge, the stroke's loop is broken by a degree-1 stub and the cut is lost.
        self.weld_dangling_endpoints(&all_new_edges, snap_epsilon.max(1.0));

        // Coincident-edge cleanup: a new edge that lands exactly on an existing edge
        // between the same two vertices (e.g. drawing a shape whose edge snaps onto an
        // existing one) must not be duplicated — duplicates produce zero-area "sliver"
        // fills. Merge such duplicates before splitting/filling.
        self.dedupe_coincident_new_edges(&all_new_edges);

        // Re-derive the fills touched by the new edges. Rather than incrementally splitting
        // along single cut edges (which can't handle a lasso whose path is interrupted by a
        // hole/notch in a non-convex fill), we re-trace the planar faces of the affected
        // sub-arrangement and rebuild the fills from them. This is robust for arbitrary
        // holed/concave fills (e.g. cutting across geometry left behind by a prior group).
        // Include split products so a self-crossing stroke's full arrangement is seen.
        let mut retrace_edges = all_new_edges.clone();
        retrace_edges.extend(split_products);
        self.retrace_fills_after_cut(&retrace_edges);

        // Drop any zero-area fills (e.g. slivers left between coincident edges).
        self.remove_degenerate_fills();

        all_new_edges
    }

    /// Merge edges from the latest stroke that are geometrically coincident with an
    /// existing edge between the same two vertices (drawing a shape whose edge lands exactly
    /// on an existing edge). Keeps the existing edge, redirects fill references, and frees
    /// the duplicate — preventing zero-area "sliver" fills between the two copies.
    /// Weld degree-1 endpoints of freshly inserted edges onto a near-coincident existing
    /// vertex. A self-intersecting freehand stroke can create an intersection vertex a
    /// fraction of a pixel from a segment endpoint that should be the same point; if they
    /// don't merge, the stroke's loop is broken by a degree-1 stub and the cut is lost.
    fn weld_dangling_endpoints(&mut self, new_edges: &[EdgeId], eps: f64) {
        // Repeatedly weld the closest dangling new endpoint to its nearest neighbour.
        loop {
            let mut to_merge: Option<(VertexId, VertexId)> = None; // (keep, merge)
            'scan: for &e in new_edges {
                if self.edges[e.idx()].deleted {
                    continue;
                }
                for &v in &self.edges[e.idx()].vertices {
                    if self.vertices[v.idx()].deleted || self.edges_at_vertex(v).len() != 1 {
                        continue;
                    }
                    let pv = self.vertices[v.idx()].position;
                    let mut best: Option<(f64, VertexId)> = None;
                    for ui in 0..self.vertices.len() {
                        if ui == v.idx() || self.vertices[ui].deleted {
                            continue;
                        }
                        let pu = self.vertices[ui].position;
                        let d = (pu.x - pv.x).hypot(pu.y - pv.y);
                        if d < eps && best.map_or(true, |(bd, _)| d < bd) {
                            best = Some((d, VertexId(ui as u32)));
                        }
                    }
                    if let Some((_, keep)) = best {
                        to_merge = Some((keep, v));
                        break 'scan;
                    }
                }
            }
            match to_merge {
                Some((keep, merge)) => self.merge_vertices(keep, merge),
                None => break,
            }
        }
        // Drop any edges that collapsed to zero length (both endpoints welded together).
        let degenerate: Vec<EdgeId> = self
            .edges
            .iter()
            .enumerate()
            .filter(|(_, e)| !e.deleted && e.vertices[0] == e.vertices[1])
            .map(|(i, _)| EdgeId(i as u32))
            .collect();
        for e in degenerate {
            for fill in &mut self.fills {
                if !fill.deleted {
                    fill.boundary.retain(|&(fe, _)| fe != e);
                }
            }
            self.free_edge(e);
        }
    }

    fn dedupe_coincident_new_edges(&mut self, new_edges: &[EdgeId]) {
        for &ne in new_edges {
            if self.edges[ne.idx()].deleted {
                continue;
            }
            let va = self.edges[ne.idx()].vertices[0];
            let vb = self.edges[ne.idx()].vertices[1];
            let candidates: Vec<EdgeId> = self
                .edges_at_vertex(va)
                .into_iter()
                .filter(|&e| e != ne && !self.edges[e.idx()].deleted)
                .filter(|&e| {
                    let v = self.edges[e.idx()].vertices;
                    (v[0] == va && v[1] == vb) || (v[0] == vb && v[1] == va)
                })
                .collect();
            for c in candidates {
                if self.edges[c.idx()].deleted {
                    continue;
                }
                if self.curves_coincident(ne, c) {
                    self.redirect_edge_in_fills(ne, c);
                    self.free_edge(ne);
                    break;
                }
            }
        }
    }

    /// Whether two edges (already known to share both endpoints) trace the same path.
    /// Coincident duplicates in practice are straight collinear segments (a shape edge
    /// snapping onto an existing edge), so we treat "both are straight lines between the
    /// same endpoints" as coincident. Comparing curve `eval(t)` directly is unreliable —
    /// split sub-edges are non-uniformly parameterised, so equal `t` ≠ equal point.
    fn curves_coincident(&self, a: EdgeId, b: EdgeId) -> bool {
        let is_straight = |c: kurbo::CubicBez| {
            let chord = c.p3 - c.p0;
            let len = chord.hypot();
            if len < 1e-9 {
                return true; // zero-length chord — degenerate, treat as coincident
            }
            // Perpendicular distance of each control point from the p0→p3 chord.
            let dist = |p: Point| ((p - c.p0).cross(chord)).abs() / len;
            dist(c.p1) < 1e-2 && dist(c.p2) < 1e-2
        };
        is_straight(self.edges[a.idx()].curve) && is_straight(self.edges[b.idx()].curve)
    }

    /// Replace every `from` boundary reference with `to`, preserving traversal direction.
    fn redirect_edge_in_fills(&mut self, from: EdgeId, to: EdgeId) {
        let f = self.edges[from.idx()].vertices;
        let t = self.edges[to.idx()].vertices;
        let same_dir = t[0] == f[0] && t[1] == f[1];
        for fill in &mut self.fills {
            if fill.deleted {
                continue;
            }
            for entry in &mut fill.boundary {
                if entry.0 == from {
                    entry.1 = match (entry.1, same_dir) {
                        (Direction::Forward, true) | (Direction::Backward, false) => {
                            Direction::Forward
                        }
                        (Direction::Backward, true) | (Direction::Forward, false) => {
                            Direction::Backward
                        }
                    };
                    entry.0 = to;
                }
            }
        }
    }

    /// Drop fills that enclose ~zero area (degenerate slivers from coincident edges).
    fn remove_degenerate_fills(&mut self) {
        for i in 0..self.fills.len() {
            if self.fills[i].deleted {
                continue;
            }
            let mut pts: Vec<Point> = Vec::new();
            for &(eid, dir) in &self.fills[i].boundary {
                if eid.is_none() {
                    continue;
                }
                let c = self.edges[eid.idx()].curve;
                pts.push(match dir {
                    Direction::Forward => c.p0,
                    Direction::Backward => c.p3,
                });
            }
            let area = if pts.len() < 3 {
                0.0
            } else {
                let mut a2 = 0.0;
                for k in 0..pts.len() {
                    let p0 = pts[k];
                    let p1 = pts[(k + 1) % pts.len()];
                    a2 += p0.x * p1.y - p1.x * p0.y;
                }
                (a2 * 0.5).abs()
            };
            if area < 1e-6 {
                self.fills[i].deleted = true;
                self.free_fills.push(i as u32);
            }
        }
    }

    /// Directed end vertex of a `(edge, direction)` boundary entry.
    #[inline]
    fn entry_end_vertex(&self, eid: EdgeId, dir: Direction) -> VertexId {
        match dir {
            Direction::Forward => self.edges[eid.idx()].vertices[1],
            Direction::Backward => self.edges[eid.idx()].vertices[0],
        }
    }

    /// Re-derive the fills touched by a freshly inserted stroke by re-tracing the planar
    /// faces of the affected sub-arrangement. This replaces incremental "split a fill by a
    /// cut edge" logic, which can't handle a cut whose path is interrupted by a hole/notch
    /// in a non-convex fill. Each affected fill is deleted and rebuilt from the traced
    /// faces that lie inside it (inheriting its colour/rule); faces outside it — or in a
    /// hole — are dropped.
    fn retrace_fills_after_cut(&mut self, new_edges: &[EdgeId]) {
        use kurbo::Shape;
        let new_set: HashSet<EdgeId> = new_edges
            .iter()
            .filter(|&&e| !e.is_none() && !self.edges[e.idx()].deleted)
            .copied()
            .collect();
        if new_set.is_empty() {
            return;
        }
        let new_verts: HashSet<VertexId> =
            new_set.iter().flat_map(|&e| self.edges[e.idx()].vertices).collect();

        // Affected fills: any non-deleted fill that shares a vertex with a new edge.
        let affected: Vec<FillId> = (0..self.fills.len())
            .filter(|&i| !self.fills[i].deleted)
            .filter(|&i| {
                self.fills[i].boundary.iter().any(|&(e, _)| {
                    !e.is_none()
                        && self.edges[e.idx()].vertices.iter().any(|v| new_verts.contains(v))
                })
            })
            .map(|i| FillId(i as u32))
            .collect();
        if affected.is_empty() {
            return;
        }

        // Snapshot each affected fill's path + attributes before we delete them.
        let originals: Vec<(kurbo::BezPath, Option<ShapeColor>, FillRule)> = affected
            .iter()
            .map(|&f| {
                (
                    self.fill_to_bezpath(f),
                    self.fills[f.idx()].color,
                    self.fills[f.idx()].fill_rule,
                )
            })
            .collect();

        // Edge set for the local arrangement: every affected fill's boundary edges plus the
        // ENTIRE inserted stroke. We include the whole stroke (not just the segments whose
        // midpoint is inside a fill) because a wiggly freehand lasso has segments that dip
        // just outside the fill; excluding them would break the inside-arc chain into
        // dangling fragments that then get pruned away, losing the cut entirely. The stroke
        // forms closed loops, so it contributes no dangling edges; faces that end up outside
        // every affected fill are discarded by the classification below.
        let mut edge_set: HashSet<EdgeId> = HashSet::new();
        for &f in &affected {
            for &(e, _) in &self.fills[f.idx()].boundary {
                if !e.is_none() && !self.edges[e.idx()].deleted {
                    edge_set.insert(e);
                }
            }
        }
        edge_set.extend(new_set.iter().copied());

        // Expand to the induced subgraph on the covered vertices. A self-intersecting
        // freehand stroke splits its own edges via `split_edge`, whose sub-edges aren't in
        // `new_edges`; without them the local arrangement has gaps and the stroke's loop
        // looks like dangling fragments. Adding every edge whose endpoints are both already
        // covered closes those gaps (the sub-edges connect already-covered stroke vertices).
        loop {
            let verts: HashSet<VertexId> = edge_set
                .iter()
                .flat_map(|&e| self.edges[e.idx()].vertices)
                .collect();
            let added: Vec<EdgeId> = (0..self.edges.len())
                .map(|i| EdgeId(i as u32))
                .filter(|&e| !self.edges[e.idx()].deleted && !edge_set.contains(&e))
                .filter(|&e| {
                    let [a, b] = self.edges[e.idx()].vertices;
                    verts.contains(&a) && verts.contains(&b)
                })
                .collect();
            if added.is_empty() {
                break;
            }
            edge_set.extend(added);
        }

        // Prune dangling edges (a vertex with degree < 2 in the local arrangement). They
        // form spikes, never real face boundaries — a freehand lasso that wiggles or nearly
        // self-touches leaves such stubs, and tracing them produces a face that runs out and
        // back along the same edge (a degenerate self-touching boundary). Iterate, since
        // removing one stub can expose another.
        loop {
            let mut degree: HashMap<VertexId, usize> = HashMap::new();
            for &e in &edge_set {
                for &v in &self.edges[e.idx()].vertices {
                    *degree.entry(v).or_default() += 1;
                }
            }
            let dangling: Vec<EdgeId> = edge_set
                .iter()
                .copied()
                .filter(|&e| {
                    let [a, b] = self.edges[e.idx()].vertices;
                    a == b || degree[&a] < 2 || degree[&b] < 2
                })
                .collect();
            if dangling.is_empty() {
                break;
            }
            for e in dangling {
                edge_set.remove(&e);
            }
        }

        let faces = self.trace_faces(&edge_set);

        // Replace the affected fills with the re-traced bounded faces that fall inside them.
        for &f in &affected {
            self.fills[f.idx()].deleted = true;
            self.free_fills.push(f.0);
        }
        for mut face in faces {
            // Collapse degenerate "spikes" — a sequence that runs out to a point and back
            // (e.g. across near-coincident duplicate tiny edges from a dense freehand path).
            self.collapse_boundary_spikes(&mut face);
            if face.len() < 3 {
                continue;
            }
            // Only bounded (counter-clockwise, positive-area) faces are real regions; the
            // outer face is clockwise/negative.
            if self.face_signed_area(&face) <= 1e-6 {
                continue;
            }
            let sample = self.boundary_interior_point(&face);
            if let Some((_, color, rule)) =
                originals.iter().find(|(p, _, _)| p.winding(sample) != 0)
            {
                self.alloc_fill(face, *color, *rule);
            }
        }
    }

    /// Remove out-and-back "spikes" from a face boundary: consecutive entries where the
    /// second exactly reverses the first (the boundary returns to where it started, e.g.
    /// bouncing across near-coincident duplicate edges). These are zero-area and would make
    /// `boundary_to_bezpath` render a stray hair; collapsing them yields a simple loop.
    fn collapse_boundary_spikes(&self, face: &mut Vec<(EdgeId, Direction)>) {
        // The four control points of an entry's curve in its traversal order.
        let traversed = |entry: &(EdgeId, Direction)| -> [Point; 4] {
            let c = self.edges[entry.0.idx()].curve;
            match entry.1 {
                Direction::Forward => [c.p0, c.p1, c.p2, c.p3],
                Direction::Backward => [c.p3, c.p2, c.p1, c.p0],
            }
        };
        const EPS: f64 = 0.5;
        // Entries i and j cancel only when j is the *exact reverse* of i — every control point of
        // j matches the mirror of i. Testing endpoints alone would also collapse a genuine
        // lens/sliver (two distinct edges that merely share near-coincident endpoints), silently
        // deleting real boundary geometry and dropping the fill.
        let reverses = |a: &(EdgeId, Direction), b: &(EdgeId, Direction)| -> bool {
            let (ca, cb) = (traversed(a), traversed(b));
            (0..4).all(|k| {
                let (p, q) = (ca[k], cb[3 - k]);
                (p.x - q.x).hypot(p.y - q.y) < EPS
            })
        };
        loop {
            let n = face.len();
            if n < 2 {
                break;
            }
            let mut collapsed = false;
            for i in 0..n {
                let j = (i + 1) % n;
                if reverses(&face[i], &face[j]) {
                    let (hi, lo) = if i > j { (i, j) } else { (j, i) };
                    face.remove(hi);
                    face.remove(lo);
                    collapsed = true;
                    break;
                }
            }
            if !collapsed {
                break;
            }
        }
    }

    /// Trace all faces of the planar arrangement formed by `edge_set`, using the standard
    /// angular next-edge rule (turn to the clockwise-adjacent dart of the twin at each
    /// vertex). Returns each face as an ordered `(edge, direction)` loop. Bounded faces
    /// come out counter-clockwise (positive signed area); the outer face clockwise.
    fn trace_faces(&self, edge_set: &HashSet<EdgeId>) -> Vec<Vec<(EdgeId, Direction)>> {
        // Outgoing darts per vertex, sorted by outgoing angle (CCW).
        let mut out: HashMap<VertexId, Vec<(f64, (EdgeId, Direction))>> = HashMap::new();
        for &e in edge_set {
            if self.edges[e.idx()].deleted {
                continue;
            }
            let [a, b] = self.edges[e.idx()].vertices;
            out.entry(a)
                .or_default()
                .push((self.dart_angle(e, Direction::Forward), (e, Direction::Forward)));
            out.entry(b)
                .or_default()
                .push((self.dart_angle(e, Direction::Backward), (e, Direction::Backward)));
        }
        for darts in out.values_mut() {
            darts.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
        }

        let mut visited: HashSet<(EdgeId, Direction)> = HashSet::new();
        let mut faces: Vec<Vec<(EdgeId, Direction)>> = Vec::new();
        let cap = edge_set.len() * 2 + 4;
        for &e in edge_set {
            if self.edges[e.idx()].deleted {
                continue;
            }
            for dir in [Direction::Forward, Direction::Backward] {
                let start = (e, dir);
                if visited.contains(&start) {
                    continue;
                }
                let mut face: Vec<(EdgeId, Direction)> = Vec::new();
                let mut d = start;
                loop {
                    visited.insert(d);
                    face.push(d);
                    // Next dart: at the end vertex, the dart clockwise-adjacent to the twin.
                    let end_v = self.entry_end_vertex(d.0, d.1);
                    let twin = (
                        d.0,
                        match d.1 {
                            Direction::Forward => Direction::Backward,
                            Direction::Backward => Direction::Forward,
                        },
                    );
                    let darts = match out.get(&end_v) {
                        Some(d) => d,
                        None => break,
                    };
                    let Some(idx) = darts.iter().position(|&(_, dd)| dd == twin) else {
                        break;
                    };
                    let next = darts[(idx + darts.len() - 1) % darts.len()].1;
                    if next == start {
                        break;
                    }
                    if visited.contains(&next) || face.len() > cap {
                        break;
                    }
                    d = next;
                }
                if face.len() >= 3 {
                    faces.push(face);
                }
            }
        }
        faces
    }

    /// Outgoing direction angle of a dart at its start vertex.
    fn dart_angle(&self, e: EdgeId, dir: Direction) -> f64 {
        let c = self.edges[e.idx()].curve;
        let (base, cands) = match dir {
            Direction::Forward => (c.p0, [c.p1, c.p2, c.p3]),
            Direction::Backward => (c.p3, [c.p2, c.p1, c.p0]),
        };
        for cand in cands {
            let d = cand - base;
            if d.hypot() > 1e-9 {
                return d.y.atan2(d.x);
            }
        }
        0.0
    }

    /// Signed area of a face given as an ordered `(edge, direction)` loop (CCW positive).
    fn face_signed_area(&self, face: &[(EdgeId, Direction)]) -> f64 {
        let pts: Vec<Point> = face
            .iter()
            .map(|&(e, dir)| {
                let c = self.edges[e.idx()].curve;
                match dir {
                    Direction::Forward => c.p0,
                    Direction::Backward => c.p3,
                }
            })
            .collect();
        if pts.len() < 3 {
            return 0.0;
        }
        let mut a2 = 0.0;
        for i in 0..pts.len() {
            let p0 = pts[i];
            let p1 = pts[(i + 1) % pts.len()];
            a2 += p0.x * p1.y - p1.x * p0.y;
        }
        a2 * 0.5
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

        // Augment the inside set with every boundary edge of an extracted fill. A
        // selection might not enumerate them all (e.g. lasso/region selection populates
        // edges differently than `select_fill`); without this an extracted fill would be
        // copied with `EdgeId::NONE` standing in for a missing edge, and that NONE later
        // panics any code that indexes `fill.boundary` (e.g. `insert_stroke`).
        let inside_edges: HashSet<EdgeId> = {
            let mut s = inside_edges.clone();
            for &fid in inside_fills {
                if fid.is_none() {
                    continue;
                }
                if let Some(fill) = self.fills.get(fid.idx()) {
                    if fill.deleted {
                        continue;
                    }
                    for &(eid, _) in &fill.boundary {
                        if !eid.is_none() {
                            s.insert(eid);
                        }
                    }
                }
            }
            s
        };
        let inside_edges = &inside_edges;

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

        // Determine which vertices are safe to free from `self`. A vertex can only be
        // freed if EVERY one of its incident edges is actually being removed from `self`,
        // i.e. is an inside edge that is NOT a boundary edge. Boundary edges are kept
        // (duplicated) in `self`, so a vertex touching one is still referenced and must
        // remain — otherwise it becomes a freed-but-referenced vertex whose slot a later
        // `alloc_vertex` reuses, corrupting the remaining fill.
        let mut interior_vertices: HashSet<VertexId> = HashSet::new();
        let mut boundary_vertices: HashSet<VertexId> = HashSet::new();
        for &vid in &referenced_vids {
            let incident = self.edges_at_vertex(vid);
            let all_removed = !incident.is_empty()
                && incident
                    .iter()
                    .all(|&eid| inside_edges.contains(&eid) && !boundary_edge_ids.contains(&eid));
            if all_removed {
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
