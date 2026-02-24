//! Doubly-Connected Edge List (DCEL) for planar subdivision vector drawing.
//!
//! Each vector layer keyframe stores a DCEL representing a Flash-style planar
//! subdivision. Strokes live on edges, fills live on faces, and the topology is
//! maintained such that wherever two strokes intersect there is a vertex.

use crate::shape::{FillRule, ShapeColor, StrokeStyle};
use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveArclen, Point};
use rstar::{PointDistance, RTree, RTreeObject, AABB};
use serde::{Deserialize, Serialize};
use std::fmt;

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
define_id!(HalfEdgeId);
define_id!(EdgeId);
define_id!(FaceId);

// ---------------------------------------------------------------------------
// Core structs
// ---------------------------------------------------------------------------

/// A vertex in the DCEL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vertex {
    /// Position in document coordinate space.
    pub position: Point,
    /// One outgoing half-edge from this vertex (any one; used to start iteration).
    pub outgoing: HalfEdgeId,
    /// Tombstone flag for free-list reuse.
    #[serde(default)]
    pub deleted: bool,
}

/// A half-edge in the DCEL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HalfEdge {
    /// Origin vertex of this half-edge.
    pub origin: VertexId,
    /// Twin (opposite direction) half-edge.
    pub twin: HalfEdgeId,
    /// Next half-edge around the face (CCW).
    pub next: HalfEdgeId,
    /// Previous half-edge around the face (CCW).
    pub prev: HalfEdgeId,
    /// Face to the left of this half-edge.
    pub face: FaceId,
    /// Parent edge (shared between this half-edge and its twin).
    pub edge: EdgeId,
    /// Tombstone flag for free-list reuse.
    #[serde(default)]
    pub deleted: bool,
}

/// Geometric and style data for an edge (shared by the two half-edges).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeData {
    /// The two half-edges for this edge: [forward, backward].
    /// Forward half-edge goes from curve.p0 to curve.p3.
    pub half_edges: [HalfEdgeId; 2],
    /// Cubic bezier curve. p0 matches origin of half_edges[0],
    /// p3 matches origin of half_edges[1].
    pub curve: CubicBez,
    /// Stroke style (None = no visible stroke).
    pub stroke_style: Option<StrokeStyle>,
    /// Stroke color (None = no visible stroke).
    pub stroke_color: Option<ShapeColor>,
    /// Tombstone flag for free-list reuse.
    #[serde(default)]
    pub deleted: bool,
}

/// A face (region) in the DCEL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Face {
    /// One half-edge on the outer boundary (walk via `next` to traverse).
    /// NONE for the unbounded face (face 0), which has no outer boundary.
    pub outer_half_edge: HalfEdgeId,
    /// Half-edges on inner boundary cycles (holes).
    pub inner_half_edges: Vec<HalfEdgeId>,
    /// Fill color (None = transparent).
    pub fill_color: Option<ShapeColor>,
    /// Image fill (references ImageAsset by UUID).
    pub image_fill: Option<uuid::Uuid>,
    /// Fill rule.
    pub fill_rule: FillRule,
    /// Tombstone flag for free-list reuse.
    #[serde(default)]
    pub deleted: bool,
}

// ---------------------------------------------------------------------------
// Spatial index
// ---------------------------------------------------------------------------

/// R-tree entry for vertex snap queries.
#[derive(Clone, Debug)]
pub struct VertexEntry {
    pub id: VertexId,
    pub position: [f64; 2],
}

impl RTreeObject for VertexEntry {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.position)
    }
}

impl PointDistance for VertexEntry {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        let dx = self.position[0] - point[0];
        let dy = self.position[1] - point[1];
        dx * dx + dy * dy
    }
}

// ---------------------------------------------------------------------------
// DCEL container
// ---------------------------------------------------------------------------

/// Default snap epsilon in document coordinate units.
pub const DEFAULT_SNAP_EPSILON: f64 = 0.5;

/// Doubly-Connected Edge List for a single keyframe's vector artwork.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dcel {
    pub vertices: Vec<Vertex>,
    pub half_edges: Vec<HalfEdge>,
    pub edges: Vec<EdgeData>,
    pub faces: Vec<Face>,

    free_vertices: Vec<u32>,
    free_half_edges: Vec<u32>,
    free_edges: Vec<u32>,
    free_faces: Vec<u32>,

    /// Transient spatial index — rebuilt on load, not serialized.
    #[serde(skip)]
    vertex_rtree: Option<RTree<VertexEntry>>,
}

impl Default for Dcel {
    fn default() -> Self {
        Self::new()
    }
}

impl Dcel {
    /// Create a new empty DCEL with just the unbounded outer face (face 0).
    pub fn new() -> Self {
        let unbounded = Face {
            outer_half_edge: HalfEdgeId::NONE,
            inner_half_edges: Vec::new(),
            fill_color: None,
            image_fill: None,
            fill_rule: FillRule::NonZero,
            deleted: false,
        };
        Dcel {
            vertices: Vec::new(),
            half_edges: Vec::new(),
            edges: Vec::new(),
            faces: vec![unbounded],
            free_vertices: Vec::new(),
            free_half_edges: Vec::new(),
            free_edges: Vec::new(),
            free_faces: Vec::new(),
            vertex_rtree: None,
        }
    }

    // -----------------------------------------------------------------------
    // Allocation
    // -----------------------------------------------------------------------

    /// Allocate a new vertex at the given position.
    pub fn alloc_vertex(&mut self, position: Point) -> VertexId {
        let id = if let Some(idx) = self.free_vertices.pop() {
            let id = VertexId(idx);
            self.vertices[id.idx()] = Vertex {
                position,
                outgoing: HalfEdgeId::NONE,
                deleted: false,
            };
            id
        } else {
            let id = VertexId(self.vertices.len() as u32);
            self.vertices.push(Vertex {
                position,
                outgoing: HalfEdgeId::NONE,
                deleted: false,
            });
            id
        };
        // Invalidate spatial index
        self.vertex_rtree = None;
        id
    }

    /// Allocate a half-edge pair (always allocated in pairs). Returns (he_a, he_b).
    pub fn alloc_half_edge_pair(&mut self) -> (HalfEdgeId, HalfEdgeId) {
        let tombstone = HalfEdge {
            origin: VertexId::NONE,
            twin: HalfEdgeId::NONE,
            next: HalfEdgeId::NONE,
            prev: HalfEdgeId::NONE,
            face: FaceId::NONE,
            edge: EdgeId::NONE,
            deleted: false,
        };

        let alloc_one = |dcel: &mut Dcel| -> HalfEdgeId {
            if let Some(idx) = dcel.free_half_edges.pop() {
                let id = HalfEdgeId(idx);
                dcel.half_edges[id.idx()] = tombstone.clone();
                id
            } else {
                let id = HalfEdgeId(dcel.half_edges.len() as u32);
                dcel.half_edges.push(tombstone.clone());
                id
            }
        };

        let a = alloc_one(self);
        let b = alloc_one(self);
        // Wire twins
        self.half_edges[a.idx()].twin = b;
        self.half_edges[b.idx()].twin = a;
        (a, b)
    }

    /// Allocate an edge. Returns the EdgeId.
    pub fn alloc_edge(&mut self, curve: CubicBez) -> EdgeId {
        let data = EdgeData {
            half_edges: [HalfEdgeId::NONE, HalfEdgeId::NONE],
            curve,
            stroke_style: None,
            stroke_color: None,
            deleted: false,
        };
        if let Some(idx) = self.free_edges.pop() {
            let id = EdgeId(idx);
            self.edges[id.idx()] = data;
            id
        } else {
            let id = EdgeId(self.edges.len() as u32);
            self.edges.push(data);
            id
        }
    }

    /// Allocate a face. Returns the FaceId.
    pub fn alloc_face(&mut self) -> FaceId {
        let face = Face {
            outer_half_edge: HalfEdgeId::NONE,
            inner_half_edges: Vec::new(),
            fill_color: None,
            image_fill: None,
            fill_rule: FillRule::NonZero,
            deleted: false,
        };
        if let Some(idx) = self.free_faces.pop() {
            let id = FaceId(idx);
            self.faces[id.idx()] = face;
            id
        } else {
            let id = FaceId(self.faces.len() as u32);
            self.faces.push(face);
            id
        }
    }

    // -----------------------------------------------------------------------
    // Deallocation
    // -----------------------------------------------------------------------

    pub fn free_vertex(&mut self, id: VertexId) {
        debug_assert!(!id.is_none());
        self.vertices[id.idx()].deleted = true;
        self.free_vertices.push(id.0);
        self.vertex_rtree = None;
    }

    pub fn free_half_edge(&mut self, id: HalfEdgeId) {
        debug_assert!(!id.is_none());
        self.half_edges[id.idx()].deleted = true;
        self.free_half_edges.push(id.0);
    }

    pub fn free_edge(&mut self, id: EdgeId) {
        debug_assert!(!id.is_none());
        self.edges[id.idx()].deleted = true;
        self.free_edges.push(id.0);
    }

    pub fn free_face(&mut self, id: FaceId) {
        debug_assert!(!id.is_none());
        debug_assert!(id.0 != 0, "cannot free the unbounded face");
        self.faces[id.idx()].deleted = true;
        self.free_faces.push(id.0);
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    #[inline]
    pub fn vertex(&self, id: VertexId) -> &Vertex {
        &self.vertices[id.idx()]
    }

    #[inline]
    pub fn vertex_mut(&mut self, id: VertexId) -> &mut Vertex {
        &mut self.vertices[id.idx()]
    }

    #[inline]
    pub fn half_edge(&self, id: HalfEdgeId) -> &HalfEdge {
        &self.half_edges[id.idx()]
    }

    #[inline]
    pub fn half_edge_mut(&mut self, id: HalfEdgeId) -> &mut HalfEdge {
        &mut self.half_edges[id.idx()]
    }

    #[inline]
    pub fn edge(&self, id: EdgeId) -> &EdgeData {
        &self.edges[id.idx()]
    }

    #[inline]
    pub fn edge_mut(&mut self, id: EdgeId) -> &mut EdgeData {
        &mut self.edges[id.idx()]
    }

    #[inline]
    pub fn face(&self, id: FaceId) -> &Face {
        &self.faces[id.idx()]
    }

    #[inline]
    pub fn face_mut(&mut self, id: FaceId) -> &mut Face {
        &mut self.faces[id.idx()]
    }

    /// Get the destination vertex of a half-edge (i.e., the origin of its twin).
    #[inline]
    pub fn half_edge_dest(&self, he: HalfEdgeId) -> VertexId {
        let twin = self.half_edge(he).twin;
        self.half_edge(twin).origin
    }

    // -----------------------------------------------------------------------
    // Spatial index
    // -----------------------------------------------------------------------

    /// Rebuild the R-tree from current (non-deleted) vertices.
    pub fn rebuild_spatial_index(&mut self) {
        let entries: Vec<VertexEntry> = self
            .vertices
            .iter()
            .enumerate()
            .filter(|(_, v)| !v.deleted)
            .map(|(i, v)| VertexEntry {
                id: VertexId(i as u32),
                position: [v.position.x, v.position.y],
            })
            .collect();
        self.vertex_rtree = Some(RTree::bulk_load(entries));
    }

    /// Ensure the spatial index is built.
    pub fn ensure_spatial_index(&mut self) {
        if self.vertex_rtree.is_none() {
            self.rebuild_spatial_index();
        }
    }

    /// Find a vertex within `epsilon` distance of `point`, or None.
    pub fn snap_vertex(&mut self, point: Point, epsilon: f64) -> Option<VertexId> {
        self.ensure_spatial_index();
        let rtree = self.vertex_rtree.as_ref().unwrap();
        let query = [point.x, point.y];
        let nearest = rtree.nearest_neighbor(&query)?;
        let dist_sq = nearest.distance_2(&query);
        if dist_sq <= epsilon * epsilon {
            Some(nearest.id)
        } else {
            None
        }
    }

    // -----------------------------------------------------------------------
    // Iteration helpers
    // -----------------------------------------------------------------------

    /// Iterate half-edges around a face boundary, starting from `start_he`.
    /// Returns half-edge IDs in order following `next` pointers.
    pub fn face_boundary(&self, face_id: FaceId) -> Vec<HalfEdgeId> {
        let face = self.face(face_id);
        if face.outer_half_edge.is_none() {
            return Vec::new();
        }
        self.walk_cycle(face.outer_half_edge)
    }

    /// Walk a half-edge cycle starting from `start`, following `next` pointers.
    pub fn walk_cycle(&self, start: HalfEdgeId) -> Vec<HalfEdgeId> {
        let mut result = Vec::new();
        let mut current = start;
        loop {
            result.push(current);
            current = self.half_edge(current).next;
            if current == start {
                break;
            }
            // Safety: prevent infinite loops in corrupted data
            if result.len() > self.half_edges.len() {
                debug_assert!(false, "infinite loop in walk_cycle");
                break;
            }
        }
        result
    }

    /// Iterate all outgoing half-edges from a vertex, sorted CCW by angle.
    /// Returns half-edge IDs where each has `origin == vertex_id`.
    pub fn vertex_outgoing(&self, vertex_id: VertexId) -> Vec<HalfEdgeId> {
        let v = self.vertex(vertex_id);
        if v.outgoing.is_none() {
            return Vec::new();
        }
        // Walk around the vertex: from outgoing, follow twin.next to get
        // the next outgoing half-edge in CCW order.
        let mut result = Vec::new();
        let mut current = v.outgoing;
        loop {
            result.push(current);
            // Go to twin, then next — this gives the next outgoing half-edge CCW
            let twin = self.half_edge(current).twin;
            current = self.half_edge(twin).next;
            if current == v.outgoing {
                break;
            }
            if result.len() > self.half_edges.len() {
                debug_assert!(false, "infinite loop in vertex_outgoing");
                break;
            }
        }
        result
    }

    /// Build a BezPath from a face's outer boundary cycle.
    pub fn face_to_bezpath(&self, face_id: FaceId) -> BezPath {
        let boundary = self.face_boundary(face_id);
        self.cycle_to_bezpath(&boundary)
    }

    /// Build a BezPath from a half-edge cycle.
    fn cycle_to_bezpath(&self, cycle: &[HalfEdgeId]) -> BezPath {
        let mut path = BezPath::new();
        if cycle.is_empty() {
            return path;
        }

        for (i, &he_id) in cycle.iter().enumerate() {
            let he = self.half_edge(he_id);
            let edge_data = self.edge(he.edge);
            // Determine if this half-edge is the forward or backward direction
            let is_forward = edge_data.half_edges[0] == he_id;
            let curve = if is_forward {
                edge_data.curve
            } else {
                // Reverse the cubic bezier
                CubicBez::new(
                    edge_data.curve.p3,
                    edge_data.curve.p2,
                    edge_data.curve.p1,
                    edge_data.curve.p0,
                )
            };

            if i == 0 {
                path.move_to(curve.p0);
            }
            path.curve_to(curve.p1, curve.p2, curve.p3);
        }
        path.close_path();
        path
    }

    /// Build a BezPath for a face including holes (for correct filled rendering).
    /// Outer boundary is CCW, holes are CW (opposite winding for non-zero fill).
    pub fn face_to_bezpath_with_holes(&self, face_id: FaceId) -> BezPath {
        let mut path = self.face_to_bezpath(face_id);

        let face = self.face(face_id);
        for &inner_he in &face.inner_half_edges {
            let hole_cycle = self.walk_cycle(inner_he);
            let hole_path = self.cycle_to_bezpath(&hole_cycle);
            // Append hole path — its winding should be opposite to outer
            for el in hole_path.elements() {
                path.push(*el);
            }
        }
        path
    }

    // -----------------------------------------------------------------------
    // Validation (debug)
    // -----------------------------------------------------------------------

    /// Check all DCEL invariants. Panics on violation. Only run in debug/test.
    pub fn validate(&self) {
        // 1. Twin symmetry: twin(twin(he)) == he
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted {
                continue;
            }
            let he_id = HalfEdgeId(i as u32);
            let twin = he.twin;
            assert!(
                !twin.is_none(),
                "half-edge {:?} has NONE twin",
                he_id
            );
            assert!(
                !self.half_edges[twin.idx()].deleted,
                "half-edge {:?} twin {:?} is deleted",
                he_id,
                twin
            );
            assert_eq!(
                self.half_edges[twin.idx()].twin,
                he_id,
                "twin symmetry violated for {:?}",
                he_id
            );
        }

        // 2. Next/prev consistency: next(prev(he)) == he, prev(next(he)) == he
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted {
                continue;
            }
            let he_id = HalfEdgeId(i as u32);
            assert!(
                !he.next.is_none(),
                "half-edge {:?} has NONE next",
                he_id
            );
            assert!(
                !he.prev.is_none(),
                "half-edge {:?} has NONE prev",
                he_id
            );
            assert_eq!(
                self.half_edges[he.next.idx()].prev,
                he_id,
                "next.prev != self for {:?}",
                he_id
            );
            assert_eq!(
                self.half_edges[he.prev.idx()].next,
                he_id,
                "prev.next != self for {:?}",
                he_id
            );
        }

        // 3. Face boundary cycles: every non-deleted half-edge's next-chain
        //    forms a cycle, and all half-edges in the cycle share the same face.
        let mut visited = vec![false; self.half_edges.len()];
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted || visited[i] {
                continue;
            }
            let start = HalfEdgeId(i as u32);
            let face = he.face;
            let mut current = start;
            let mut count = 0;
            loop {
                assert!(
                    !self.half_edges[current.idx()].deleted,
                    "cycle contains deleted half-edge {:?}",
                    current
                );
                assert_eq!(
                    self.half_edges[current.idx()].face,
                    face,
                    "half-edge {:?} has face {:?} but cycle started with face {:?}",
                    current,
                    self.half_edges[current.idx()].face,
                    face
                );
                visited[current.idx()] = true;
                current = self.half_edges[current.idx()].next;
                count += 1;
                if current == start {
                    break;
                }
                assert!(
                    count <= self.half_edges.len(),
                    "infinite cycle from {:?}",
                    start
                );
            }
        }

        // 4. Vertex outgoing: every non-deleted vertex's outgoing half-edge
        //    originates from that vertex.
        for (i, v) in self.vertices.iter().enumerate() {
            if v.deleted {
                continue;
            }
            let v_id = VertexId(i as u32);
            if !v.outgoing.is_none() {
                let he = &self.half_edges[v.outgoing.idx()];
                assert!(
                    !he.deleted,
                    "vertex {:?} outgoing {:?} is deleted",
                    v_id,
                    v.outgoing
                );
                assert_eq!(
                    he.origin, v_id,
                    "vertex {:?} outgoing {:?} has origin {:?}",
                    v_id, v.outgoing, he.origin
                );
            }
        }

        // 5. Edge half-edge consistency
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            let e_id = EdgeId(i as u32);
            for &he_id in &e.half_edges {
                assert!(
                    !he_id.is_none(),
                    "edge {:?} has NONE half-edge",
                    e_id
                );
                assert_eq!(
                    self.half_edges[he_id.idx()].edge,
                    e_id,
                    "edge {:?} half-edge {:?} doesn't point back",
                    e_id,
                    he_id
                );
            }
            // The two half-edges should be twins
            assert_eq!(
                self.half_edges[e.half_edges[0].idx()].twin,
                e.half_edges[1],
                "edge {:?} half-edges are not twins",
                e_id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Topology operations
// ---------------------------------------------------------------------------

/// Result of inserting a stroke into the DCEL.
#[derive(Clone, Debug)]
pub struct InsertStrokeResult {
    /// All new vertex IDs created.
    pub new_vertices: Vec<VertexId>,
    /// All new edge IDs created.
    pub new_edges: Vec<EdgeId>,
    /// Existing edges that were split: (original_edge, parameter, new_vertex, new_edge).
    pub split_edges: Vec<(EdgeId, f64, VertexId, EdgeId)>,
    /// New face IDs created by edge insertion.
    pub new_faces: Vec<FaceId>,
}

impl Dcel {
    // -----------------------------------------------------------------------
    // insert_edge: add an edge between two vertices on the same face boundary
    // -----------------------------------------------------------------------

    /// Insert an edge between `v1` and `v2` within `face`, splitting it into two faces.
    ///
    /// Both vertices must be on the boundary of `face`. The new edge's curve is `curve`.
    /// Returns `(new_edge_id, new_face_id)` where the new face is on one side of the edge.
    ///
    /// If `v1 == v2` or the vertices are not both on the face boundary, this will panic
    /// in debug mode.
    pub fn insert_edge(
        &mut self,
        v1: VertexId,
        v2: VertexId,
        face: FaceId,
        curve: CubicBez,
    ) -> (EdgeId, FaceId) {
        debug_assert!(v1 != v2, "cannot insert edge from vertex to itself");

        let v1_has_edges = !self.vertices[v1.idx()].outgoing.is_none();
        let v2_has_edges = !self.vertices[v2.idx()].outgoing.is_none();

        // Allocate the new edge and half-edge pair
        let (he_fwd, he_bwd) = self.alloc_half_edge_pair();
        let edge_id = self.alloc_edge(curve);

        // Wire edge ↔ half-edges
        self.edges[edge_id.idx()].half_edges = [he_fwd, he_bwd];
        self.half_edges[he_fwd.idx()].edge = edge_id;
        self.half_edges[he_bwd.idx()].edge = edge_id;

        // Set origins
        self.half_edges[he_fwd.idx()].origin = v1;
        self.half_edges[he_bwd.idx()].origin = v2;

        match (v1_has_edges, v2_has_edges) {
            (false, false) => {
                // Both vertices are isolated (no existing edges). This is the first
                // edge in this face. Wire next/prev to form two trivial cycles.
                self.half_edges[he_fwd.idx()].next = he_bwd;
                self.half_edges[he_fwd.idx()].prev = he_bwd;
                self.half_edges[he_bwd.idx()].next = he_fwd;
                self.half_edges[he_bwd.idx()].prev = he_fwd;

                // Both half-edges are on the same face initially (no real split).
                self.half_edges[he_fwd.idx()].face = face;
                self.half_edges[he_bwd.idx()].face = face;

                // Set face outer half-edge if unset
                if self.faces[face.idx()].outer_half_edge.is_none() || face.0 == 0 {
                    if face.0 == 0 {
                        self.faces[0].inner_half_edges.push(he_fwd);
                    } else {
                        self.faces[face.idx()].outer_half_edge = he_fwd;
                    }
                }

                // Set vertex outgoing
                if self.vertices[v1.idx()].outgoing.is_none() {
                    self.vertices[v1.idx()].outgoing = he_fwd;
                }
                if self.vertices[v2.idx()].outgoing.is_none() {
                    self.vertices[v2.idx()].outgoing = he_bwd;
                }

                return (edge_id, face);
            }
            (true, true) => {
                // Both vertices have existing edges. Use angular position to find
                // the correct sector in each vertex's fan for the splice.
                //
                // The standard DCEL rule: at a vertex with outgoing half-edges
                // sorted CCW by angle, the new edge goes between the half-edge
                // just before it (CW) and just after it (CCW). he_from_v is the
                // CCW successor — the existing outgoing half-edge that will follow
                // the new edge in the fan after insertion.
                let fwd_angle = Self::curve_angle_at_start(&curve);
                let bwd_angle = Self::curve_angle_at_end(&curve);

                let he_from_v1 = self.find_ccw_successor(v1, fwd_angle);
                let he_from_v2 = self.find_ccw_successor(v2, bwd_angle);

                let he_into_v1 = self.half_edges[he_from_v1.idx()].prev;
                let he_into_v2 = self.half_edges[he_from_v2.idx()].prev;

                // The actual face being split is determined by the sector, not the
                // parameter — the parameter may be stale after prior inserts.
                let actual_face = self.half_edges[he_into_v1.idx()].face;

                // Splice: he_into_v1 → he_fwd → he_from_v2 → ...
                //         he_into_v2 → he_bwd → he_from_v1 → ...
                self.half_edges[he_fwd.idx()].next = he_from_v2;
                self.half_edges[he_fwd.idx()].prev = he_into_v1;
                self.half_edges[he_into_v1.idx()].next = he_fwd;
                self.half_edges[he_from_v2.idx()].prev = he_fwd;

                self.half_edges[he_bwd.idx()].next = he_from_v1;
                self.half_edges[he_bwd.idx()].prev = he_into_v2;
                self.half_edges[he_into_v2.idx()].next = he_bwd;
                self.half_edges[he_from_v1.idx()].prev = he_bwd;

                // Allocate new face for one side of the split
                let new_face = self.alloc_face();

                // Walk each cycle and assign faces
                self.half_edges[he_fwd.idx()].face = actual_face;
                {
                    let mut cur = self.half_edges[he_fwd.idx()].next;
                    while cur != he_fwd {
                        self.half_edges[cur.idx()].face = actual_face;
                        cur = self.half_edges[cur.idx()].next;
                    }
                }
                self.half_edges[he_bwd.idx()].face = new_face;
                {
                    let mut cur = self.half_edges[he_bwd.idx()].next;
                    while cur != he_bwd {
                        self.half_edges[cur.idx()].face = new_face;
                        cur = self.half_edges[cur.idx()].next;
                    }
                }

                // Update face boundary pointers
                self.faces[actual_face.idx()].outer_half_edge = he_fwd;
                self.faces[new_face.idx()].outer_half_edge = he_bwd;

                return (edge_id, new_face);
            }
            _ => {
                // One vertex has edges, the other is isolated.
                // This creates a "spur" (antenna) edge — no face split.
                let (connected_v, isolated_v) = if v1_has_edges {
                    (v1, v2)
                } else {
                    (v2, v1)
                };

                // he_out: new half-edge FROM connected_v TO isolated_v
                // he_back: new half-edge FROM isolated_v TO connected_v
                let (he_out, he_back) = if self.half_edges[he_fwd.idx()].origin == connected_v {
                    (he_fwd, he_bwd)
                } else {
                    (he_bwd, he_fwd)
                };

                // Find correct sector at connected vertex using angle
                let spur_angle = if self.half_edges[he_fwd.idx()].origin == connected_v {
                    Self::curve_angle_at_start(&curve)
                } else {
                    Self::curve_angle_at_end(&curve)
                };
                let existing_he = self.find_ccw_successor(connected_v, spur_angle);

                let he_into_connected = self.half_edges[existing_he.idx()].prev;
                let actual_face = self.half_edges[he_into_connected.idx()].face;

                // Splice spur into the cycle at connected_v:
                // Before: ... → he_into_connected → existing_he → ...
                // After:  ... → he_into_connected → he_out → he_back → existing_he → ...
                self.half_edges[he_into_connected.idx()].next = he_out;
                self.half_edges[he_out.idx()].prev = he_into_connected;
                self.half_edges[he_out.idx()].next = he_back;
                self.half_edges[he_back.idx()].prev = he_out;
                self.half_edges[he_back.idx()].next = existing_he;
                self.half_edges[existing_he.idx()].prev = he_back;

                // Both half-edges are on the same face (no split)
                self.half_edges[he_out.idx()].face = actual_face;
                self.half_edges[he_back.idx()].face = actual_face;

                // Isolated vertex's outgoing must originate FROM isolated_v
                self.vertices[isolated_v.idx()].outgoing = he_back;

                return (edge_id, actual_face);
            }
        }
    }

    /// Find the outgoing half-edge from `vertex` that is the immediate CCW
    /// successor of `new_angle` in the vertex fan.
    ///
    /// In the DCEL fan around a vertex, outgoing half-edges are ordered by
    /// angle with the rule `twin(out[i]).next = out[(i+1) % n]`. Inserting a
    /// new edge at `new_angle` requires splicing before this CCW successor.
    fn find_ccw_successor(&self, vertex: VertexId, new_angle: f64) -> HalfEdgeId {
        let v = self.vertex(vertex);
        debug_assert!(!v.outgoing.is_none(), "find_ccw_successor on isolated vertex");

        let start = v.outgoing;
        let mut best_he = start;
        let mut best_delta = f64::MAX;

        let mut current = start;
        loop {
            let angle = self.outgoing_angle(current);
            // How far CCW from new_angle to this half-edge's angle
            let mut delta = angle - new_angle;
            if delta <= 0.0 {
                delta += std::f64::consts::TAU;
            }
            if delta < best_delta {
                best_delta = delta;
                best_he = current;
            }

            let twin = self.half_edge(current).twin;
            current = self.half_edge(twin).next;
            if current == start {
                break;
            }
        }

        best_he
    }

    /// Outgoing angle of a curve at its start point (p0 → p1, fallback p3).
    fn curve_angle_at_start(curve: &CubicBez) -> f64 {
        let from = curve.p0;
        let dx = curve.p1.x - from.x;
        let dy = curve.p1.y - from.y;
        if dx * dx + dy * dy > 1e-18 {
            dy.atan2(dx)
        } else {
            (curve.p3.y - from.y).atan2(curve.p3.x - from.x)
        }
    }

    /// Outgoing angle of the backward half-edge at the curve's end point
    /// (p3 → p2, fallback p0).
    fn curve_angle_at_end(curve: &CubicBez) -> f64 {
        let from = curve.p3;
        let dx = curve.p2.x - from.x;
        let dy = curve.p2.y - from.y;
        if dx * dx + dy * dy > 1e-18 {
            dy.atan2(dx)
        } else {
            (curve.p0.y - from.y).atan2(curve.p0.x - from.x)
        }
    }

    // -----------------------------------------------------------------------
    // split_edge: split an edge at parameter t via de Casteljau
    // -----------------------------------------------------------------------

    /// Split an edge at parameter `t` (0..1), inserting a new vertex at the split point.
    /// The original edge is shortened to [0, t], a new edge covers [t, 1].
    /// Returns `(new_vertex_id, new_edge_id)`.
    pub fn split_edge(&mut self, edge_id: EdgeId, t: f64) -> (VertexId, EdgeId) {
        debug_assert!((0.0..=1.0).contains(&t), "t must be in [0, 1]");

        let original_curve = self.edges[edge_id.idx()].curve;
        // De Casteljau subdivision
        let (curve_a, curve_b) = subdivide_cubic(original_curve, t);

        let split_point = curve_a.p3; // == curve_b.p0
        let new_vertex = self.alloc_vertex(split_point);

        // Get the original half-edges
        let [he_fwd, he_bwd] = self.edges[edge_id.idx()].half_edges;

        // Allocate new edge and half-edge pair for the second segment
        let (new_he_fwd, new_he_bwd) = self.alloc_half_edge_pair();
        let new_edge_id = self.alloc_edge(curve_b);

        // Wire new edge ↔ half-edges
        self.edges[new_edge_id.idx()].half_edges = [new_he_fwd, new_he_bwd];
        self.half_edges[new_he_fwd.idx()].edge = new_edge_id;
        self.half_edges[new_he_bwd.idx()].edge = new_edge_id;

        // Copy stroke style from original edge
        self.edges[new_edge_id.idx()].stroke_style =
            self.edges[edge_id.idx()].stroke_style.clone();
        self.edges[new_edge_id.idx()].stroke_color = self.edges[edge_id.idx()].stroke_color;

        // Update original edge's curve to the first segment
        self.edges[edge_id.idx()].curve = curve_a;

        // Set origins for new half-edges
        // new_he_fwd goes from new_vertex toward the old destination
        // new_he_bwd goes from old destination toward new_vertex
        self.half_edges[new_he_fwd.idx()].origin = new_vertex;
        // new_he_bwd's origin = old destination of he_fwd = origin of he_bwd's twin...
        // Actually, he_bwd.origin = destination of original forward edge
        self.half_edges[new_he_bwd.idx()].origin = self.half_edges[he_bwd.idx()].origin;

        // Now splice into the boundary cycles.
        // Forward direction: ... → he_fwd → he_fwd.next → ...
        // becomes: ... → he_fwd → new_he_fwd → old_he_fwd.next → ...
        let fwd_next = self.half_edges[he_fwd.idx()].next;
        self.half_edges[he_fwd.idx()].next = new_he_fwd;
        self.half_edges[new_he_fwd.idx()].prev = he_fwd;
        self.half_edges[new_he_fwd.idx()].next = fwd_next;
        self.half_edges[fwd_next.idx()].prev = new_he_fwd;
        self.half_edges[new_he_fwd.idx()].face = self.half_edges[he_fwd.idx()].face;

        // Backward direction: ... → he_bwd → he_bwd.next → ...
        // becomes: ... → new_he_bwd → he_bwd → he_bwd.next → ...
        // (new_he_bwd is inserted before he_bwd)
        let bwd_prev = self.half_edges[he_bwd.idx()].prev;
        self.half_edges[he_bwd.idx()].prev = new_he_bwd;
        self.half_edges[new_he_bwd.idx()].next = he_bwd;
        self.half_edges[new_he_bwd.idx()].prev = bwd_prev;
        self.half_edges[bwd_prev.idx()].next = new_he_bwd;
        self.half_edges[new_he_bwd.idx()].face = self.half_edges[he_bwd.idx()].face;

        // Update he_bwd's origin to the new vertex (it now covers [new_vertex → v1])
        // new_he_bwd covers [old_dest → new_vertex]
        let old_dest = self.half_edges[he_bwd.idx()].origin;
        self.half_edges[he_bwd.idx()].origin = new_vertex;

        // Update old destination vertex's outgoing: it was pointing at he_bwd,
        // but he_bwd.origin is now new_vertex. new_he_bwd has origin = old_dest.
        if self.vertices[old_dest.idx()].outgoing == he_bwd {
            self.vertices[old_dest.idx()].outgoing = new_he_bwd;
        }

        // Set new vertex's outgoing half-edge
        self.vertices[new_vertex.idx()].outgoing = new_he_fwd;

        (new_vertex, new_edge_id)
    }

    // -----------------------------------------------------------------------
    // remove_edge: remove an edge, merging the two adjacent faces
    // -----------------------------------------------------------------------

    /// Remove an edge, merging its two adjacent faces into one.
    /// Returns the surviving face ID.
    pub fn remove_edge(&mut self, edge_id: EdgeId) -> FaceId {
        let [he_fwd, he_bwd] = self.edges[edge_id.idx()].half_edges;
        let face_a = self.half_edges[he_fwd.idx()].face;
        let face_b = self.half_edges[he_bwd.idx()].face;

        // The surviving face (prefer lower ID, always keep face 0)
        let (surviving, dying) = if face_a.0 <= face_b.0 {
            (face_a, face_b)
        } else {
            (face_b, face_a)
        };

        let fwd_prev = self.half_edges[he_fwd.idx()].prev;
        let fwd_next = self.half_edges[he_fwd.idx()].next;
        let bwd_prev = self.half_edges[he_bwd.idx()].prev;
        let bwd_next = self.half_edges[he_bwd.idx()].next;

        // Check if removing this edge leaves isolated vertices
        let v1 = self.half_edges[he_fwd.idx()].origin;
        let v2 = self.half_edges[he_bwd.idx()].origin;

        // Splice out the half-edges from boundary cycles
        if fwd_next == he_bwd && bwd_next == he_fwd {
            // The edge forms a complete boundary by itself (degenerate 2-cycle)
            // Both vertices become isolated
            self.vertices[v1.idx()].outgoing = HalfEdgeId::NONE;
            self.vertices[v2.idx()].outgoing = HalfEdgeId::NONE;
        } else if fwd_next == he_bwd {
            // he_fwd → he_bwd is a spur (consecutive in cycle):
            // ... → fwd_prev → he_fwd → he_bwd → bwd_next → ...
            // Splice both out: fwd_prev → bwd_next
            self.half_edges[fwd_prev.idx()].next = bwd_next;
            self.half_edges[bwd_next.idx()].prev = fwd_prev;
            // v2 (origin of he_bwd) becomes isolated
            self.vertices[v2.idx()].outgoing = HalfEdgeId::NONE;
            // Update v1's outgoing if needed
            if self.vertices[v1.idx()].outgoing == he_fwd {
                self.vertices[v1.idx()].outgoing = bwd_next;
            }
        } else if bwd_next == he_fwd {
            // he_bwd → he_fwd is a spur (consecutive in cycle):
            // ... → bwd_prev → he_bwd → he_fwd → fwd_next → ...
            // Splice both out: bwd_prev → fwd_next
            self.half_edges[bwd_prev.idx()].next = fwd_next;
            self.half_edges[fwd_next.idx()].prev = bwd_prev;
            self.vertices[v1.idx()].outgoing = HalfEdgeId::NONE;
            if self.vertices[v2.idx()].outgoing == he_bwd {
                self.vertices[v2.idx()].outgoing = fwd_next;
            }
        } else {
            // Normal case: splice out both half-edges
            self.half_edges[fwd_prev.idx()].next = bwd_next;
            self.half_edges[bwd_next.idx()].prev = fwd_prev;
            self.half_edges[bwd_prev.idx()].next = fwd_next;
            self.half_edges[fwd_next.idx()].prev = bwd_prev;

            // Update vertex outgoing pointers if they pointed to removed half-edges
            if self.vertices[v1.idx()].outgoing == he_fwd {
                self.vertices[v1.idx()].outgoing = bwd_next;
            }
            if self.vertices[v2.idx()].outgoing == he_bwd {
                self.vertices[v2.idx()].outgoing = fwd_next;
            }
        }

        // Reassign all half-edges from dying face to surviving face
        if surviving != dying && !dying.is_none() {
            // Find a valid starting half-edge for the walk.
            // The dying face's outer_half_edge may point to one of the removed half-edges,
            // so we use a surviving neighbor (fwd_next or bwd_next) that was spliced in.
            let dying_ohe = self.faces[dying.idx()].outer_half_edge;
            let walk_start = if dying_ohe.is_none() {
                HalfEdgeId::NONE
            } else if dying_ohe != he_fwd && dying_ohe != he_bwd {
                dying_ohe
            } else {
                // The outer_half_edge was removed; use a surviving neighbor instead.
                // After splicing, fwd_next and bwd_next are the half-edges that replaced
                // the removed ones in the cycle. Pick one that belongs to dying face.
                if !fwd_next.is_none() && fwd_next != he_fwd && fwd_next != he_bwd {
                    fwd_next
                } else if !bwd_next.is_none() && bwd_next != he_fwd && bwd_next != he_bwd {
                    bwd_next
                } else {
                    HalfEdgeId::NONE
                }
            };

            if !walk_start.is_none() {
                let mut cur = walk_start;
                loop {
                    self.half_edges[cur.idx()].face = surviving;
                    cur = self.half_edges[cur.idx()].next;
                    if cur == walk_start {
                        break;
                    }
                }
            }

            // Merge inner half-edges (holes) from dying into surviving
            let inner = std::mem::take(&mut self.faces[dying.idx()].inner_half_edges);
            self.faces[surviving.idx()].inner_half_edges.extend(inner);
        }

        // Update surviving face's outer half-edge if it pointed to a removed half-edge
        if self.faces[surviving.idx()].outer_half_edge == he_fwd
            || self.faces[surviving.idx()].outer_half_edge == he_bwd
        {
            // Find a remaining half-edge on this face
            if fwd_next != he_bwd && !self.half_edges[fwd_next.idx()].deleted {
                self.faces[surviving.idx()].outer_half_edge = fwd_next;
            } else if bwd_next != he_fwd && !self.half_edges[bwd_next.idx()].deleted {
                self.faces[surviving.idx()].outer_half_edge = bwd_next;
            } else {
                self.faces[surviving.idx()].outer_half_edge = HalfEdgeId::NONE;
            }
        }

        // Remove inner_half_edges references to removed half-edges
        self.faces[surviving.idx()]
            .inner_half_edges
            .retain(|&he| he != he_fwd && he != he_bwd);

        // Free the removed elements
        self.free_half_edge(he_fwd);
        self.free_half_edge(he_bwd);
        self.free_edge(edge_id);
        if surviving != dying && !dying.is_none() && dying.0 != 0 {
            self.free_face(dying);
        }

        surviving
    }

    // -----------------------------------------------------------------------
    // insert_stroke: compound operation for adding a multi-segment stroke
    // -----------------------------------------------------------------------

    /// Insert a stroke (sequence of cubic bezier segments) into the DCEL.
    ///
    /// This is the main entry point for the Draw tool. It:
    /// 1. Snaps stroke endpoints to nearby existing vertices (within epsilon)
    /// 2. Finds intersections between stroke segments and existing edges
    /// 3. Splits existing edges at intersection points
    /// 4. Inserts new vertices and edges for the stroke segments
    /// 5. Updates face topology as edges split faces
    ///
    /// The segments should be connected end-to-end (segment[i].p3 == segment[i+1].p0).
    pub fn insert_stroke(
        &mut self,
        segments: &[CubicBez],
        stroke_style: Option<StrokeStyle>,
        stroke_color: Option<ShapeColor>,
        epsilon: f64,
    ) -> InsertStrokeResult {
        use crate::curve_intersections::find_curve_intersections;

        let mut result = InsertStrokeResult {
            new_vertices: Vec::new(),
            new_edges: Vec::new(),
            split_edges: Vec::new(),
            new_faces: Vec::new(),
        };

        if segments.is_empty() {
            return result;
        }

        // Collect all intersection points between new segments and existing edges.
        // For each new segment, we need to know where to split it, and for each
        // existing edge, we need to know where to split it.

        // Structure: for each new segment index, a sorted list of (t, point, existing_edge_id, t_on_existing)
        #[allow(dead_code)]
        struct StrokeIntersection {
            t_on_segment: f64,
            point: Point,
            existing_edge: EdgeId,
            t_on_existing: f64,
        }

        let mut segment_intersections: Vec<Vec<StrokeIntersection>> =
            (0..segments.len()).map(|_| Vec::new()).collect();

        // Find intersections with existing edges
        let existing_edge_count = self.edges.len();
        for (seg_idx, seg) in segments.iter().enumerate() {
            for edge_idx in 0..existing_edge_count {
                if self.edges[edge_idx].deleted {
                    continue;
                }
                let edge_id = EdgeId(edge_idx as u32);
                let existing_curve = &self.edges[edge_idx].curve;

                let intersections = find_curve_intersections(seg, existing_curve);
                for inter in intersections {
                    if let Some(t2) = inter.t2 {
                        // Skip intersections at the very endpoints (these are handled by snapping)
                        if (inter.t1 < 0.001 || inter.t1 > 0.999)
                            && (t2 < 0.001 || t2 > 0.999)
                        {
                            continue;
                        }
                        segment_intersections[seg_idx].push(StrokeIntersection {
                            t_on_segment: inter.t1,
                            point: inter.point,
                            existing_edge: edge_id,
                            t_on_existing: t2,
                        });
                    }
                }
            }
            // Sort by t on segment
            segment_intersections[seg_idx]
                .sort_by(|a, b| a.t_on_segment.partial_cmp(&b.t_on_segment).unwrap());
        }

        // Within-stroke self-intersections.
        //
        // There are two kinds:
        //  (a) A single cubic segment crosses itself (loop-shaped curve).
        //  (b) Two different segments of the stroke cross each other.
        //
        // For (a) we split each segment at its midpoint and intersect the two
        // halves using the robust recursive finder, then remap t-values back to
        // the original segment's parameter space.
        //
        // For (b) we check all (i, j) pairs where j > i. Adjacent pairs share
        // an endpoint — we filter out that shared-endpoint hit (t1≈1, t2≈0).
        struct IntraStrokeIntersection {
            seg_a: usize,
            t_on_a: f64,
            seg_b: usize,
            t_on_b: f64,
            point: Point,
        }
        let mut intra_intersections: Vec<IntraStrokeIntersection> = Vec::new();

        // (a) Single-segment self-intersections
        for (i, seg) in segments.iter().enumerate() {
            let left = seg.subsegment(0.0..0.5);
            let right = seg.subsegment(0.5..1.0);
            let hits = find_curve_intersections(&left, &right);
            for inter in hits {
                if let Some(t2) = inter.t2 {
                    // Remap from half-curve parameter space to full segment:
                    // left half [0,1] → segment [0, 0.5], right half [0,1] → segment [0.5, 1]
                    let t_on_seg_a = inter.t1 * 0.5;
                    let t_on_seg_b = 0.5 + t2 * 0.5;
                    // Skip the shared midpoint (t1≈1 on left, t2≈0 on right → seg t≈0.5 both)
                    if (t_on_seg_b - t_on_seg_a).abs() < 0.01 {
                        continue;
                    }
                    // Skip near-endpoint hits
                    if t_on_seg_a < 0.001 || t_on_seg_b > 0.999 {
                        continue;
                    }
                    intra_intersections.push(IntraStrokeIntersection {
                        seg_a: i,
                        t_on_a: t_on_seg_a,
                        seg_b: i,
                        t_on_b: t_on_seg_b,
                        point: inter.point,
                    });
                }
            }
        }

        // (b) Inter-segment crossings
        for i in 0..segments.len() {
            for j in (i + 1)..segments.len() {
                let hits = find_curve_intersections(&segments[i], &segments[j]);
                for inter in hits {
                    if let Some(t2) = inter.t2 {
                        // Skip near-endpoint hits: these are shared vertices between
                        // consecutive segments (t1≈1, t2≈0) or stroke start/end,
                        // not real crossings. Use a wider threshold for adjacent
                        // segments since the recursive finder can converge to t-values
                        // that are close-but-not-quite at the shared corner.
                        let tol = if j == i + 1 { 0.02 } else { 0.001 };
                        if (inter.t1 < tol || inter.t1 > 1.0 - tol)
                            && (t2 < tol || t2 > 1.0 - tol)
                        {
                            continue;
                        }
                        intra_intersections.push(IntraStrokeIntersection {
                            seg_a: i,
                            t_on_a: inter.t1,
                            seg_b: j,
                            t_on_b: t2,
                            point: inter.point,
                        });
                    }
                }
            }
        }

        // Dedup nearby intra-stroke intersections (recursive finder can return
        // near-duplicate hits for one crossing)
        intra_intersections.sort_by(|a, b| {
            a.seg_a
                .cmp(&b.seg_a)
                .then(a.seg_b.cmp(&b.seg_b))
                .then(a.t_on_a.partial_cmp(&b.t_on_a).unwrap())
        });
        intra_intersections.dedup_by(|a, b| {
            a.seg_a == b.seg_a
                && a.seg_b == b.seg_b
                && (a.point - b.point).hypot() < 1.0
        });

        // Create vertices for each intra-stroke crossing and record split points.
        //
        // For single-segment self-intersections (seg_a == seg_b), the loop
        // sub-curve would go from vertex V back to V, which insert_edge
        // doesn't support. We break the loop by adding a midpoint vertex
        // halfway between the two crossing t-values, splitting the loop
        // sub-curve into two halves.
        let mut intra_split_points: Vec<Vec<(f64, VertexId)>> =
            (0..segments.len()).map(|_| Vec::new()).collect();

        for intra in &intra_intersections {
            let v = self.alloc_vertex(intra.point);
            result.new_vertices.push(v);
            intra_split_points[intra.seg_a].push((intra.t_on_a, v));
            if intra.seg_a == intra.seg_b {
                // Same segment: add a midpoint vertex to break the V→V loop
                let mid_t = (intra.t_on_a + intra.t_on_b) / 2.0;
                let mid_point = segments[intra.seg_a].eval(mid_t);
                let mid_v = self.alloc_vertex(mid_point);
                result.new_vertices.push(mid_v);
                intra_split_points[intra.seg_a].push((mid_t, mid_v));
            }
            intra_split_points[intra.seg_b].push((intra.t_on_b, v));
        }

        // Split existing edges at intersection points.
        // We need to track how edge splits affect subsequent intersection parameters.
        // Process from highest t to lowest per edge to avoid parameter shift.
        struct EdgeSplit {
            edge_id: EdgeId,
            t: f64,
            seg_idx: usize,
            inter_idx: usize,
        }

        // Group intersections by existing edge
        let mut splits_by_edge: std::collections::HashMap<u32, Vec<EdgeSplit>> =
            std::collections::HashMap::new();
        for (seg_idx, inters) in segment_intersections.iter().enumerate() {
            for (inter_idx, inter) in inters.iter().enumerate() {
                splits_by_edge
                    .entry(inter.existing_edge.0)
                    .or_default()
                    .push(EdgeSplit {
                        edge_id: inter.existing_edge,
                        t: inter.t_on_existing,
                        seg_idx,
                        inter_idx,
                    });
            }
        }

        // For each existing edge, sort splits by t descending and apply them.
        // Map from (seg_idx, inter_idx) to the vertex created at the split.
        let mut split_vertex_map: std::collections::HashMap<(usize, usize), VertexId> =
            std::collections::HashMap::new();

        for (_edge_raw, mut splits) in splits_by_edge {
            // Sort descending by t so we split from end to start (no parameter shift)
            splits.sort_by(|a, b| b.t.partial_cmp(&a.t).unwrap());

            let current_edge = splits[0].edge_id;
            let remaining_t_start = 0.0_f64;

            for split in &splits {
                // Remap t from original [0,1] to current sub-edge's parameter space
                let t_in_current = if remaining_t_start < split.t {
                    (split.t - remaining_t_start) / (1.0 - remaining_t_start)
                } else {
                    0.0
                };

                if t_in_current < 0.001 || t_in_current > 0.999 {
                    // Too close to endpoint — snap to existing vertex instead
                    let vertex = if t_in_current <= 0.5 {
                        let he = self.edges[current_edge.idx()].half_edges[0];
                        self.half_edges[he.idx()].origin
                    } else {
                        let he = self.edges[current_edge.idx()].half_edges[1];
                        self.half_edges[he.idx()].origin
                    };
                    split_vertex_map.insert((split.seg_idx, split.inter_idx), vertex);
                    continue;
                }

                let (new_vertex, new_edge) = self.split_edge(current_edge, t_in_current);
                result.split_edges.push((current_edge, split.t, new_vertex, new_edge));
                split_vertex_map.insert((split.seg_idx, split.inter_idx), new_vertex);

                // After splitting at t_in_current, the "upper" portion is new_edge.
                // For subsequent splits (which have smaller t), they are on current_edge.
                // remaining_t_start stays the same since we split descending.
                // Actually, since we sorted descending, the next split has a smaller t
                // and is on the first portion (current_edge, which is now [remaining_t_start, split.t]).
                // remaining_t_start stays same — current_edge is the lower portion
                let _ = new_edge;
            }
        }

        // Now insert the stroke segments as edges.
        // For each segment, split it at intersection points and create sub-edges.
        // Collect the vertex chain for the entire stroke.
        let mut stroke_vertices: Vec<VertexId> = Vec::new();

        // First vertex: snap or create
        let first_point = segments[0].p0;
        let first_v = self
            .snap_vertex(first_point, epsilon)
            .unwrap_or_else(|| {
                let v = self.alloc_vertex(first_point);
                result.new_vertices.push(v);
                v
            });
        stroke_vertices.push(first_v);

        for (seg_idx, seg) in segments.iter().enumerate() {
            let inters = &segment_intersections[seg_idx];

            // Collect split points along this segment in order
            let mut split_points: Vec<(f64, VertexId)> = Vec::new();
            for (inter_idx, inter) in inters.iter().enumerate() {
                if let Some(&vertex) = split_vertex_map.get(&(seg_idx, inter_idx)) {
                    split_points.push((inter.t_on_segment, vertex));
                }
            }

            // Merge intra-stroke split points (self-crossing vertices)
            if let Some(intra) = intra_split_points.get(seg_idx) {
                for &(t, v) in intra {
                    split_points.push((t, v));
                }
            }
            // Sort by t so all split points (existing-edge + intra-stroke) are in order
            split_points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

            // End vertex: snap or create
            let end_point = seg.p3;
            let end_v = if seg_idx + 1 < segments.len() {
                // Interior join — snap to next segment's start (which should be the same point)
                self.snap_vertex(end_point, epsilon).unwrap_or_else(|| {
                    let v = self.alloc_vertex(end_point);
                    result.new_vertices.push(v);
                    v
                })
            } else {
                // Last segment endpoint
                self.snap_vertex(end_point, epsilon).unwrap_or_else(|| {
                    let v = self.alloc_vertex(end_point);
                    result.new_vertices.push(v);
                    v
                })
            };
            split_points.push((1.0, end_v));

            // Create sub-edges from last vertex through split points
            let mut prev_t = 0.0;
            let mut prev_vertex = *stroke_vertices.last().unwrap();

            for (t, vertex) in &split_points {
                // Skip zero-length sub-edges: an intra-stroke split point near
                // a segment endpoint can snap to the same vertex, producing a
                // degenerate v→v edge.
                if prev_vertex == *vertex {
                    prev_t = *t;
                    continue;
                }

                let sub_curve = subsegment_cubic(*seg, prev_t, *t);

                // Find the face containing this edge's midpoint for insertion
                let face = self.find_face_containing_point(midpoint_of_cubic(&sub_curve));

                let (edge_id, maybe_new_face) =
                    self.insert_edge(prev_vertex, *vertex, face, sub_curve);

                // Apply stroke style
                self.edges[edge_id.idx()].stroke_style = stroke_style.clone();
                self.edges[edge_id.idx()].stroke_color = stroke_color;

                result.new_edges.push(edge_id);
                if maybe_new_face != face && maybe_new_face.0 != 0 {
                    result.new_faces.push(maybe_new_face);
                }

                prev_t = *t;
                prev_vertex = *vertex;
            }

            stroke_vertices.push(end_v);
        }

        #[cfg(debug_assertions)]
        self.validate();

        result
    }

    // -----------------------------------------------------------------------
    // recompute_edge_intersections: find and split new intersections after edit
    // -----------------------------------------------------------------------

    /// Recompute intersections between `edge_id` and all other non-deleted edges.
    ///
    /// After a curve edit, the moved edge may now cross other edges. This method
    /// finds those intersections and splits both the edited edge and the crossed
    /// edges at each intersection point (mirroring the logic in `insert_stroke`).
    ///
    /// Returns a list of `(new_vertex, new_edge)` pairs created by splits.
    pub fn recompute_edge_intersections(
        &mut self,
        edge_id: EdgeId,
    ) -> Vec<(VertexId, EdgeId)> {
        use crate::curve_intersections::find_curve_intersections;

        let mut created = Vec::new();

        if self.edges[edge_id.idx()].deleted {
            return created;
        }

        // Collect intersections between the edited edge and every other edge.
        struct Hit {
            t_on_edited: f64,
            t_on_other: f64,
            other_edge: EdgeId,
        }

        let edited_curve = self.edges[edge_id.idx()].curve;

        // --- Self-intersection: split curve at midpoint, intersect the halves ---
        {
            let left = edited_curve.subsegment(0.0..0.5);
            let right = edited_curve.subsegment(0.5..1.0);
            let self_hits = find_curve_intersections(&left, &right);

            // Collect valid self-intersection t-pairs (remapped to full curve)
            let mut self_crossings: Vec<(f64, f64)> = Vec::new();
            for inter in self_hits {
                if let Some(t2) = inter.t2 {
                    let t_a = inter.t1 * 0.5;          // left half → [0, 0.5]
                    let t_b = 0.5 + t2 * 0.5;          // right half → [0.5, 1]
                    // Skip shared midpoint and near-endpoint hits
                    if (t_b - t_a).abs() < 0.01 || t_a < 0.001 || t_b > 0.999 {
                        continue;
                    }
                    self_crossings.push((t_a, t_b));
                }
            }
            // Dedup
            self_crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            self_crossings.dedup_by(|a, b| (a.0 - b.0).abs() < 0.02);

            if !self_crossings.is_empty() {
                // For each self-crossing, split the edge at t_a, midpoint, and t_b.
                // We process from high-t to low-t to avoid parameter shift.
                // Collect all split t-values with a flag for shared-vertex pairs.
                let mut self_split_ts: Vec<f64> = Vec::new();
                for &(t_a, t_b) in &self_crossings {
                    self_split_ts.push(t_a);
                    self_split_ts.push((t_a + t_b) / 2.0);
                    self_split_ts.push(t_b);
                }
                self_split_ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
                self_split_ts.dedup_by(|a, b| (*a - *b).abs() < 0.001);

                // Split from high-t to low-t
                let current_edge = edge_id;
                let mut remaining_t_end = 1.0_f64;
                let mut split_vertices: Vec<(f64, VertexId)> = Vec::new();

                for &t in self_split_ts.iter().rev() {
                    let t_in_current = t / remaining_t_end;
                    if t_in_current < 0.001 || t_in_current > 0.999 {
                        continue;
                    }
                    let (new_vertex, new_edge) = self.split_edge(current_edge, t_in_current);
                    created.push((new_vertex, new_edge));
                    split_vertices.push((t, new_vertex));
                    remaining_t_end = t;
                }

                // Now merge the crossing vertex pairs. For each (t_a, t_b),
                // the vertices at t_a and t_b should be the same point.
                for &(t_a, t_b) in &self_crossings {
                    let v_a = split_vertices.iter().find(|(t, _)| (*t - t_a).abs() < 0.01);
                    let v_b = split_vertices.iter().find(|(t, _)| (*t - t_b).abs() < 0.01);
                    if let (Some(&(_, va)), Some(&(_, vb))) = (v_a, v_b) {
                        if !self.vertices[va.idx()].deleted && !self.vertices[vb.idx()].deleted {
                            self.merge_vertices_at_crossing(va, vb);
                        }
                    }
                }

                // Reassign faces after the self-intersection merges
                self.reassign_faces_after_merges();

                #[cfg(debug_assertions)]
                self.validate();

                return created;
            }
        }

        let mut hits = Vec::new();

        for (idx, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            let other_id = EdgeId(idx as u32);
            if other_id == edge_id {
                continue;
            }

            // Approximate arc lengths for scaling the near-endpoint
            // threshold to a consistent spatial tolerance (pixels).
            let edited_len = edited_curve.arclen(0.5).max(1.0);
            let other_len = e.curve.arclen(0.5).max(1.0);
            let spatial_tol = 1.0_f64; // pixels
            let t1_tol = spatial_tol / edited_len;
            let t2_tol = spatial_tol / other_len;

            let intersections = find_curve_intersections(&edited_curve, &e.curve);
            for inter in intersections {
                if let Some(t2) = inter.t2 {
                    // Skip intersections where either t is too close to an
                    // endpoint to produce a usable split. The threshold is
                    // scaled by arc length so it corresponds to a consistent
                    // spatial tolerance. This filters:
                    // - Shared-vertex hits (both t near endpoints)
                    // - Spurious near-vertex bbox-overlap false positives
                    // - Hits that would create one-sided splits
                    if inter.t1 < t1_tol || inter.t1 > 1.0 - t1_tol
                        || t2 < t2_tol || t2 > 1.0 - t2_tol
                    {
                        continue;
                    }

                    hits.push(Hit {
                        t_on_edited: inter.t1,
                        t_on_other: t2,
                        other_edge: other_id,
                    });
                }
            }
        }

        if hits.is_empty() {
            return created;
        }

        // Group by other_edge, split each from high-t to low-t to avoid param shift.
        let mut by_other: std::collections::HashMap<u32, Vec<(f64, f64)>> =
            std::collections::HashMap::new();
        for h in &hits {
            by_other
                .entry(h.other_edge.0)
                .or_default()
                .push((h.t_on_other, h.t_on_edited));
        }

        // Deduplicate within each group: the recursive intersection finder
        // often returns many near-identical hits for one crossing. Keep one
        // representative per cluster (using t_on_other distance < 0.1).
        for splits in by_other.values_mut() {
            splits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            splits.dedup_by(|a, b| (a.0 - b.0).abs() < 0.1);
        }

        // Track (t_on_edited, vertex_from_other_edge_split) pairs so we can
        // later split the edited edge and merge each pair of co-located vertices.
        let mut edited_edge_splits: Vec<(f64, VertexId)> = Vec::new();

        for (other_raw, mut splits) in by_other {
            let other_edge = EdgeId(other_raw);
            // Sort descending by t_on_other
            splits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

            let current_edge = other_edge;
            // Upper bound of current_edge in original parameter space.
            // split_edge(edge, t) keeps [0, t] on current_edge, so after
            // splitting at t_high the edge spans [0, t_high] (reparam to [0,1]).
            let mut remaining_t_end = 1.0_f64;

            for (t_on_other, t_on_edited) in splits {
                let t_in_current = t_on_other / remaining_t_end;

                if t_in_current < 0.001 || t_in_current > 0.999 {
                    continue;
                }

                let (new_vertex, new_edge) = self.split_edge(current_edge, t_in_current);
                created.push((new_vertex, new_edge));
                edited_edge_splits.push((t_on_edited, new_vertex));

                // After splitting at t_in_current, current_edge is [0, t_on_other]
                // in original space. Update remaining_t_end for the next iteration.
                remaining_t_end = t_on_other;
                let _ = new_edge;
            }
        }

        // Now split the edited edge itself at all intersection t-values.
        // Sort descending by t to avoid parameter shift.
        edited_edge_splits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        // Deduplicate near-equal t values (keep the first = highest t)
        edited_edge_splits.dedup_by(|a, b| (a.0 - b.0).abs() < 0.001);

        let current_edge = edge_id;
        let mut remaining_t_end = 1.0_f64;

        // Collect crossing pairs: (vertex_on_edited_edge, vertex_on_other_edge)
        let mut crossing_pairs: Vec<(VertexId, VertexId)> = Vec::new();

        for (t, other_vertex) in &edited_edge_splits {
            let t_in_current = *t / remaining_t_end;

            if t_in_current < 0.001 || t_in_current > 0.999 {
                continue;
            }

            let (new_vertex, new_edge) = self.split_edge(current_edge, t_in_current);
            created.push((new_vertex, new_edge));
            crossing_pairs.push((new_vertex, *other_vertex));
            remaining_t_end = *t;
            let _ = new_edge;
        }

        // Post-process: merge co-located vertex pairs at each crossing point.
        // Do all vertex merges first (topology only), then reassign faces once.
        let has_merges = !crossing_pairs.is_empty();
        for (v_edited, v_other) in &crossing_pairs {
            if self.vertices[v_edited.idx()].deleted || self.vertices[v_other.idx()].deleted {
                continue;
            }
            self.merge_vertices_at_crossing(*v_edited, *v_other);
        }

        // Now that all merges are done, walk all cycles and assign faces.
        if has_merges {
            self.reassign_faces_after_merges();
        }

        #[cfg(debug_assertions)]
        self.validate();

        created
    }

    /// Compute the outgoing angle (in radians, via atan2) of a half-edge at its
    /// origin vertex. Used to sort half-edges CCW around a vertex.
    fn outgoing_angle(&self, he: HalfEdgeId) -> f64 {
        let he_data = self.half_edge(he);
        let edge_data = self.edge(he_data.edge);
        let is_forward = edge_data.half_edges[0] == he;

        let (from, to, fallback) = if is_forward {
            // Forward half-edge: direction from curve.p0 → curve.p1 (fallback curve.p3)
            (edge_data.curve.p0, edge_data.curve.p1, edge_data.curve.p3)
        } else {
            // Backward half-edge: direction from curve.p3 → curve.p2 (fallback curve.p0)
            (edge_data.curve.p3, edge_data.curve.p2, edge_data.curve.p0)
        };

        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx * dx + dy * dy > 1e-18 {
            dy.atan2(dx)
        } else {
            // Degenerate: control point coincides with endpoint, use far endpoint
            let dx = fallback.x - from.x;
            let dy = fallback.y - from.y;
            dy.atan2(dx)
        }
    }

    /// Merge two co-located vertices at a crossing point and relink half-edges.
    ///
    /// After `split_edge()` creates two separate vertices at the same crossing,
    /// this merges them into one, sorts the (now valence-4) outgoing half-edges
    /// by angle, and relinks `next`/`prev` using the standard DCEL vertex rule.
    ///
    /// Face assignment is NOT done here — call `reassign_faces_after_merges()`
    /// once after all merges are complete.
    fn merge_vertices_at_crossing(
        &mut self,
        v_keep: VertexId,
        v_remove: VertexId,
    ) {
        // Re-home half-edges from v_remove → v_keep
        for i in 0..self.half_edges.len() {
            if self.half_edges[i].deleted {
                continue;
            }
            if self.half_edges[i].origin == v_remove {
                self.half_edges[i].origin = v_keep;
            }
        }

        // Collect & sort outgoing half-edges by angle (CCW).
        // We can't use vertex_outgoing() because the next/prev links
        // aren't correct for the merged vertex yet.
        let mut outgoing: Vec<HalfEdgeId> = Vec::new();
        for i in 0..self.half_edges.len() {
            if self.half_edges[i].deleted {
                continue;
            }
            if self.half_edges[i].origin == v_keep {
                outgoing.push(HalfEdgeId(i as u32));
            }
        }
        outgoing.sort_by(|&a, &b| {
            let angle_a = self.outgoing_angle(a);
            let angle_b = self.outgoing_angle(b);
            angle_a.partial_cmp(&angle_b).unwrap()
        });

        let n = outgoing.len();
        if n < 2 {
            self.vertices[v_keep.idx()].outgoing = if n == 1 {
                outgoing[0]
            } else {
                HalfEdgeId::NONE
            };
            self.free_vertex(v_remove);
            return;
        }

        // Relink next/prev at vertex using the standard DCEL rule:
        //     twin(outgoing[i]).next = outgoing[(i+1) % N]
        for i in 0..n {
            let twin_i = self.half_edges[outgoing[i].idx()].twin;
            let next_out = outgoing[(i + 1) % n];
            self.half_edges[twin_i.idx()].next = next_out;
            self.half_edges[next_out.idx()].prev = twin_i;
        }

        // Cleanup vertex
        self.vertices[v_keep.idx()].outgoing = outgoing[0];
        self.free_vertex(v_remove);
    }

    /// After merging vertices at crossing points, walk all face cycles and
    /// reassign faces. This must be called once after all merges are done,
    /// because individual merges can break cycles created by earlier merges.
    fn reassign_faces_after_merges(&mut self) {
        let mut visited = vec![false; self.half_edges.len()];
        let mut cycles: Vec<(HalfEdgeId, Vec<HalfEdgeId>)> = Vec::new();

        // Discover all face cycles by walking from every unvisited half-edge.
        for i in 0..self.half_edges.len() {
            if self.half_edges[i].deleted || visited[i] {
                continue;
            }
            let start_he = HalfEdgeId(i as u32);
            let mut cycle_hes: Vec<HalfEdgeId> = Vec::new();
            let mut cur = start_he;
            loop {
                if visited[cur.idx()] {
                    break;
                }
                visited[cur.idx()] = true;
                cycle_hes.push(cur);
                cur = self.half_edges[cur.idx()].next;
                if cur == start_he {
                    break;
                }
                if cycle_hes.len() > self.half_edges.len() {
                    debug_assert!(false, "infinite loop in face reassignment cycle walk");
                    break;
                }
            }
            if !cycle_hes.is_empty() {
                cycles.push((start_he, cycle_hes));
            }
        }

        // Collect old face assignments from half-edges (before reassignment).
        // Each cycle votes on which old face it belongs to.
        struct CycleInfo {
            start_he: HalfEdgeId,
            half_edges: Vec<HalfEdgeId>,
            face_votes: std::collections::HashMap<u32, usize>,
        }
        let cycle_infos: Vec<CycleInfo> = cycles
            .into_iter()
            .map(|(start_he, hes)| {
                let mut face_votes: std::collections::HashMap<u32, usize> =
                    std::collections::HashMap::new();
                for &he in &hes {
                    let f = self.half_edges[he.idx()].face;
                    if !f.is_none() {
                        *face_votes.entry(f.0).or_insert(0) += 1;
                    }
                }
                CycleInfo {
                    start_he,
                    half_edges: hes,
                    face_votes,
                }
            })
            .collect();

        // Collect all old faces referenced.
        let mut all_old_faces: std::collections::HashSet<u32> =
            std::collections::HashSet::new();
        for c in &cycle_infos {
            for &f in c.face_votes.keys() {
                all_old_faces.insert(f);
            }
        }

        // For each old face, assign it to the cycle with the most votes.
        let mut cycle_face_assignment: Vec<Option<FaceId>> =
            vec![None; cycle_infos.len()];

        for &old_face_raw in &all_old_faces {
            let mut best_idx: Option<usize> = None;
            let mut best_count: usize = 0;
            for (i, c) in cycle_infos.iter().enumerate() {
                if cycle_face_assignment[i].is_some() {
                    continue;
                }
                let count = c.face_votes.get(&old_face_raw).copied().unwrap_or(0);
                if count > best_count {
                    best_count = count;
                    best_idx = Some(i);
                }
            }
            if let Some(idx) = best_idx {
                cycle_face_assignment[idx] = Some(FaceId(old_face_raw));
            }
        }

        // Any cycle without an assigned face gets a new one, inheriting
        // fill properties from the old face it voted for most.
        for i in 0..cycle_infos.len() {
            if cycle_face_assignment[i].is_none() {
                // Determine which face to inherit fill from. Check both
                // the cycle's own old face votes AND the adjacent faces
                // (via twin half-edges), because at crossings the inside/
                // outside flips and the cycle's own votes may point to F0.
                let mut fill_candidates: std::collections::HashMap<u32, usize> =
                    std::collections::HashMap::new();
                // Own votes
                for (&face_raw, &count) in &cycle_infos[i].face_votes {
                    *fill_candidates.entry(face_raw).or_insert(0) += count;
                }
                // Adjacent faces (twins)
                for &he in &cycle_infos[i].half_edges {
                    let twin = self.half_edges[he.idx()].twin;
                    let twin_face = self.half_edges[twin.idx()].face;
                    if !twin_face.is_none() {
                        *fill_candidates.entry(twin_face.0).or_insert(0) += 1;
                    }
                }
                // Pick the best non-F0 candidate (F0 is unbounded, no fill).
                let parent_face = fill_candidates
                    .iter()
                    .filter(|(&face_raw, _)| face_raw != 0)
                    .max_by_key(|&(_, &count)| count)
                    .map(|(&face_raw, _)| FaceId(face_raw));

                let f = self.alloc_face();
                // Copy fill properties from the parent face.
                if let Some(parent) = parent_face {
                    self.faces[f.idx()].fill_color =
                        self.faces[parent.idx()].fill_color.clone();
                    self.faces[f.idx()].image_fill =
                        self.faces[parent.idx()].image_fill;
                    self.faces[f.idx()].fill_rule =
                        self.faces[parent.idx()].fill_rule;
                }
                cycle_face_assignment[i] = Some(f);
            }
        }

        // Apply assignments.
        for (i, cycle) in cycle_infos.iter().enumerate() {
            let face = cycle_face_assignment[i].unwrap();
            for &he in &cycle.half_edges {
                self.half_edges[he.idx()].face = face;
            }
            if face.0 == 0 {
                self.faces[0]
                    .inner_half_edges
                    .retain(|h| !cycle.half_edges.contains(h));
                self.faces[0].inner_half_edges.push(cycle.start_he);
            } else {
                self.faces[face.idx()].outer_half_edge = cycle.start_he;
            }
        }
    }

    /// Find which face contains a given point (brute force for now).
    /// Returns FaceId(0) (unbounded) if no bounded face contains the point.
    pub fn find_face_containing_point(&self, point: Point) -> FaceId {
        use kurbo::Shape;
        for (i, face) in self.faces.iter().enumerate() {
            if face.deleted || i == 0 {
                continue;
            }
            if face.outer_half_edge.is_none() {
                continue;
            }
            let path = self.face_to_bezpath(FaceId(i as u32));
            if path.winding(point) != 0 {
                return FaceId(i as u32);
            }
        }
        FaceId(0)
    }
}

/// Extract a subsegment of a cubic bezier for parameter range [t0, t1].
fn subsegment_cubic(c: CubicBez, t0: f64, t1: f64) -> CubicBez {
    if (t0 - 0.0).abs() < 1e-10 && (t1 - 1.0).abs() < 1e-10 {
        return c;
    }
    // Split at t1 first, take the first part, then split that at t0/t1
    if (t0 - 0.0).abs() < 1e-10 {
        subdivide_cubic(c, t1).0
    } else if (t1 - 1.0).abs() < 1e-10 {
        subdivide_cubic(c, t0).1
    } else {
        let (_, upper) = subdivide_cubic(c, t0);
        let remapped_t1 = (t1 - t0) / (1.0 - t0);
        subdivide_cubic(upper, remapped_t1).0
    }
}

/// Get the midpoint of a cubic bezier.
fn midpoint_of_cubic(c: &CubicBez) -> Point {
    c.eval(0.5)
}

// ---------------------------------------------------------------------------
// Bezier subdivision
// ---------------------------------------------------------------------------

/// Split a cubic bezier at parameter t using de Casteljau's algorithm.
/// Returns (first_half, second_half).
pub fn subdivide_cubic(c: CubicBez, t: f64) -> (CubicBez, CubicBez) {
    // Level 1
    let p01 = lerp_point(c.p0, c.p1, t);
    let p12 = lerp_point(c.p1, c.p2, t);
    let p23 = lerp_point(c.p2, c.p3, t);
    // Level 2
    let p012 = lerp_point(p01, p12, t);
    let p123 = lerp_point(p12, p23, t);
    // Level 3
    let p0123 = lerp_point(p012, p123, t);

    (
        CubicBez::new(c.p0, p01, p012, p0123),
        CubicBez::new(p0123, p123, p23, c.p3),
    )
}

#[inline]
fn lerp_point(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

// ---------------------------------------------------------------------------
// BezPath → cubic segments conversion
// ---------------------------------------------------------------------------

/// Convert a `BezPath` into a list of sub-paths, each a `Vec<CubicBez>`.
///
/// - `MoveTo` starts a new sub-path.
/// - `LineTo` is promoted to a degenerate cubic.
/// - `QuadTo` is degree-elevated to cubic.
/// - `CurveTo` is passed through directly.
/// - `ClosePath` emits a closing line segment if the current point differs
///   from the sub-path start.
pub fn bezpath_to_cubic_segments(path: &BezPath) -> Vec<Vec<CubicBez>> {
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
                // Degree-elevate: CP1 = P0 + 2/3*(Q1-P0), CP2 = P2 + 2/3*(Q1-P2)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_dcel_has_unbounded_face() {
        let dcel = Dcel::new();
        assert_eq!(dcel.faces.len(), 1);
        assert!(!dcel.faces[0].deleted);
        assert!(dcel.faces[0].outer_half_edge.is_none());
        assert!(dcel.faces[0].fill_color.is_none());
    }

    #[test]
    fn test_alloc_vertex() {
        let mut dcel = Dcel::new();
        let v = dcel.alloc_vertex(Point::new(1.0, 2.0));
        assert_eq!(v.0, 0);
        assert_eq!(dcel.vertex(v).position, Point::new(1.0, 2.0));
        assert!(dcel.vertex(v).outgoing.is_none());
    }

    #[test]
    fn test_free_and_reuse_vertex() {
        let mut dcel = Dcel::new();
        let v0 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v1 = dcel.alloc_vertex(Point::new(1.0, 1.0));
        dcel.free_vertex(v0);
        let v2 = dcel.alloc_vertex(Point::new(2.0, 2.0));
        // Should reuse slot 0
        assert_eq!(v2.0, 0);
        assert_eq!(dcel.vertex(v2).position, Point::new(2.0, 2.0));
        assert!(!dcel.vertex(v2).deleted);
        let _ = v1; // suppress unused warning
    }

    #[test]
    fn test_snap_vertex() {
        let mut dcel = Dcel::new();
        let v = dcel.alloc_vertex(Point::new(10.0, 10.0));
        // Exact match
        assert_eq!(dcel.snap_vertex(Point::new(10.0, 10.0), 0.5), Some(v));
        // Within epsilon
        assert_eq!(dcel.snap_vertex(Point::new(10.3, 10.0), 0.5), Some(v));
        // Outside epsilon
        assert_eq!(dcel.snap_vertex(Point::new(11.0, 10.0), 0.5), None);
    }

    fn line_curve(p0: Point, p1: Point) -> CubicBez {
        // A straight-line cubic bezier
        let d = p1 - p0;
        CubicBez::new(
            p0,
            Point::new(p0.x + d.x / 3.0, p0.y + d.y / 3.0),
            Point::new(p0.x + 2.0 * d.x / 3.0, p0.y + 2.0 * d.y / 3.0),
            p1,
        )
    }

    #[test]
    fn test_insert_first_edge_into_unbounded_face() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));

        let (edge_id, _) = dcel.insert_edge(
            v1,
            v2,
            FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0)),
        );

        assert!(!dcel.edge(edge_id).deleted);
        assert_eq!(dcel.edges.len(), 1);
        // Both half-edges should exist
        let [he_fwd, he_bwd] = dcel.edge(edge_id).half_edges;
        assert!(!he_fwd.is_none());
        assert!(!he_bwd.is_none());
        assert_eq!(dcel.half_edge(he_fwd).origin, v1);
        assert_eq!(dcel.half_edge(he_bwd).origin, v2);
        // Twins
        assert_eq!(dcel.half_edge(he_fwd).twin, he_bwd);
        assert_eq!(dcel.half_edge(he_bwd).twin, he_fwd);
        // Next/prev form a 2-cycle
        assert_eq!(dcel.half_edge(he_fwd).next, he_bwd);
        assert_eq!(dcel.half_edge(he_bwd).next, he_fwd);

        dcel.validate();
    }

    #[test]
    fn test_insert_triangle_splits_face() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let v3 = dcel.alloc_vertex(Point::new(5.0, 10.0));

        // Insert three edges to form a triangle
        let (e1, _) = dcel.insert_edge(
            v1,
            v2,
            FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0)),
        );

        // v2 → v3: v2 has an outgoing edge, v3 is isolated → spur case
        let (e2, _) = dcel.insert_edge(
            v2,
            v3,
            FaceId(0),
            line_curve(Point::new(10.0, 0.0), Point::new(5.0, 10.0)),
        );

        // v3 → v1: both have outgoing edges on face 0 → face split
        let (e3, new_face) = dcel.insert_edge(
            v3,
            v1,
            FaceId(0),
            line_curve(Point::new(5.0, 10.0), Point::new(0.0, 0.0)),
        );

        // Should have created a new face (the triangle interior)
        assert!(new_face.0 > 0, "should create a new face for the triangle interior");

        // Validate all invariants
        dcel.validate();

        // Count non-deleted faces (should be 2: unbounded + triangle)
        let live_faces = dcel.faces.iter().filter(|f| !f.deleted).count();
        assert_eq!(live_faces, 2, "expected 2 faces (unbounded + triangle)");

        let _ = (e1, e2, e3);
    }

    #[test]
    fn test_split_edge() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));

        let (edge_id, _) = dcel.insert_edge(
            v1,
            v2,
            FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0)),
        );

        let (new_vertex, new_edge) = dcel.split_edge(edge_id, 0.5);

        // New vertex should be at midpoint
        let pos = dcel.vertex(new_vertex).position;
        assert!((pos.x - 5.0).abs() < 0.01);
        assert!((pos.y - 0.0).abs() < 0.01);

        // Should have 2 edges now
        let live_edges = dcel.edges.iter().filter(|e| !e.deleted).count();
        assert_eq!(live_edges, 2);

        // Original edge curve.p3 should be at midpoint
        assert!((dcel.edge(edge_id).curve.p3.x - 5.0).abs() < 0.01);
        // New edge curve.p0 should be at midpoint
        assert!((dcel.edge(new_edge).curve.p0.x - 5.0).abs() < 0.01);
        // New edge curve.p3 should be at original endpoint
        assert!((dcel.edge(new_edge).curve.p3.x - 10.0).abs() < 0.01);

        dcel.validate();
    }

    #[test]
    fn test_remove_edge() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));

        let (edge_id, _) = dcel.insert_edge(
            v1,
            v2,
            FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0)),
        );

        let surviving = dcel.remove_edge(edge_id);
        assert_eq!(surviving, FaceId(0));

        // Edge should be deleted
        assert!(dcel.edge(edge_id).deleted);

        // Vertices should be isolated
        assert!(dcel.vertex(v1).outgoing.is_none());
        assert!(dcel.vertex(v2).outgoing.is_none());
    }

    #[test]
    fn test_subdivide_cubic_midpoint() {
        let c = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(1.0, 2.0),
            Point::new(3.0, 2.0),
            Point::new(4.0, 0.0),
        );
        let (a, b) = subdivide_cubic(c, 0.5);
        // Endpoints should match
        assert_eq!(a.p0, c.p0);
        assert_eq!(b.p3, c.p3);
        // Junction should match
        assert!((a.p3.x - b.p0.x).abs() < 1e-10);
        assert!((a.p3.y - b.p0.y).abs() < 1e-10);
    }

    #[test]
    fn test_face_to_bezpath() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let v3 = dcel.alloc_vertex(Point::new(5.0, 10.0));

        // Build triangle
        dcel.insert_edge(v1, v2, FaceId(0), line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0)));
        dcel.insert_edge(v2, v3, FaceId(0), line_curve(Point::new(10.0, 0.0), Point::new(5.0, 10.0)));
        let (_, new_face) = dcel.insert_edge(v3, v1, FaceId(0), line_curve(Point::new(5.0, 10.0), Point::new(0.0, 0.0)));

        dcel.validate();

        // The new face should produce a non-empty BezPath
        let path = dcel.face_to_bezpath(new_face);
        assert!(!path.elements().is_empty());
    }

    /// Rectangle ABCD, drag midpoint of AB across BC creating crossing X.
    /// Two polygons should result: AXCD and a bigon XB (the "XMB" region).
    #[test]
    fn test_crossing_creates_two_faces() {
        let mut dcel = Dcel::new();

        // Rectangle at pixel scale: A=(0,100), B=(100,100), C=(100,0), D=(0,0)
        let a = dcel.alloc_vertex(Point::new(0.0, 100.0));
        let b = dcel.alloc_vertex(Point::new(100.0, 100.0));
        let c = dcel.alloc_vertex(Point::new(100.0, 0.0));
        let d = dcel.alloc_vertex(Point::new(0.0, 0.0));

        // Build rectangle edges AB, BC, CD, DA
        let (e_ab, _) = dcel.insert_edge(
            a, b, FaceId(0),
            line_curve(Point::new(0.0, 100.0), Point::new(100.0, 100.0)),
        );
        let (e_bc, _) = dcel.insert_edge(
            b, c, FaceId(0),
            line_curve(Point::new(100.0, 100.0), Point::new(100.0, 0.0)),
        );
        let (e_cd, _) = dcel.insert_edge(
            c, d, FaceId(0),
            line_curve(Point::new(100.0, 0.0), Point::new(0.0, 0.0)),
        );
        let (e_da, _) = dcel.insert_edge(
            d, a, FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(0.0, 100.0)),
        );

        dcel.validate();

        let faces_before = dcel.faces.iter().filter(|f| !f.deleted).count();

        // Simulate dragging midpoint M of AB to (200, 50).
        // Control points at (180, 50) and (220, 50) — same as user's
        // coordinates scaled by 100.
        let new_ab_curve = CubicBez::new(
            Point::new(0.0, 100.0),
            Point::new(180.0, 50.0),
            Point::new(220.0, 50.0),
            Point::new(100.0, 100.0),
        );
        dcel.edges[e_ab.idx()].curve = new_ab_curve;

        // Recompute intersections — this should split AB and BC at the crossing,
        // merge the co-located vertices, and create the new face.
        let created = dcel.recompute_edge_intersections(e_ab);

        // Should have created vertices and edges from the splits
        assert!(
            !created.is_empty(),
            "recompute_edge_intersections should have found the crossing"
        );

        dcel.validate();

        let faces_after = dcel.faces.iter().filter(|f| !f.deleted).count();
        assert!(
            faces_after > faces_before,
            "a new face should have been created for the XMB region \
             (before: {}, after: {})",
            faces_before,
            faces_after
        );

        let _ = (e_bc, e_cd, e_da);
    }

    #[test]
    fn test_two_crossings_creates_three_faces() {
        let mut dcel = Dcel::new();

        // Rectangle at pixel scale: A=(0,100), B=(100,100), C=(100,0), D=(0,0)
        let a = dcel.alloc_vertex(Point::new(0.0, 100.0));
        let b = dcel.alloc_vertex(Point::new(100.0, 100.0));
        let c = dcel.alloc_vertex(Point::new(100.0, 0.0));
        let d = dcel.alloc_vertex(Point::new(0.0, 0.0));

        let (e_ab, _) = dcel.insert_edge(
            a, b, FaceId(0),
            line_curve(Point::new(0.0, 100.0), Point::new(100.0, 100.0)),
        );
        let (e_bc, _) = dcel.insert_edge(
            b, c, FaceId(0),
            line_curve(Point::new(100.0, 100.0), Point::new(100.0, 0.0)),
        );
        let (e_cd, _) = dcel.insert_edge(
            c, d, FaceId(0),
            line_curve(Point::new(100.0, 0.0), Point::new(0.0, 0.0)),
        );
        let (e_da, _) = dcel.insert_edge(
            d, a, FaceId(0),
            line_curve(Point::new(0.0, 0.0), Point::new(0.0, 100.0)),
        );

        dcel.validate();
        let faces_before = dcel.faces.iter().filter(|f| !f.deleted).count();

        // Drag M through CD: curve from A to B that dips below y=0,
        // crossing CD (y=0 line) twice.
        let new_ab_curve = CubicBez::new(
            Point::new(0.0, 100.0),
            Point::new(30.0, -80.0),
            Point::new(70.0, -80.0),
            Point::new(100.0, 100.0),
        );
        dcel.edges[e_ab.idx()].curve = new_ab_curve;

        let created = dcel.recompute_edge_intersections(e_ab);

        eprintln!("created: {:?}", created);
        eprintln!("vertices: {}", dcel.vertices.iter().filter(|v| !v.deleted).count());
        eprintln!("edges: {}", dcel.edges.iter().filter(|e| !e.deleted).count());
        eprintln!("faces (non-deleted):");
        for (i, f) in dcel.faces.iter().enumerate() {
            if !f.deleted {
                let cycle_len = if !f.outer_half_edge.is_none() {
                    dcel.walk_cycle(f.outer_half_edge).len()
                } else {
                    0
                };
                eprintln!("  F{}: outer={:?} cycle_len={}", i, f.outer_half_edge, cycle_len);
            }
        }

        // Should have 4 splits (2 on CD, 2 on AB)
        assert!(
            created.len() >= 4,
            "expected at least 4 splits, got {}",
            created.len()
        );

        dcel.validate();

        let faces_after = dcel.faces.iter().filter(|f| !f.deleted).count();
        // Before: 2 faces (interior + exterior). After: 4 (AX1D, X1X2M, X2BC + exterior)
        assert!(
            faces_after >= faces_before + 2,
            "should have at least 2 new faces (before: {}, after: {})",
            faces_before,
            faces_after
        );

        let _ = (e_bc, e_cd, e_da);
    }

    #[test]
    fn test_single_segment_self_intersection() {
        // A single cubic bezier that loops back on itself.
        // Control points (300,150) and (-100,150) are far apart and on opposite
        // sides of the chord, forcing the curve to reverse in X and cross itself
        // near t≈0.175 and t≈0.825.
        let mut dcel = Dcel::new();

        let seg = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(300.0, 150.0),
            Point::new(-100.0, 150.0),
            Point::new(200.0, 0.0),
        );

        let result = dcel.insert_stroke(&[seg], None, None, 5.0);

        eprintln!("new_vertices: {:?}", result.new_vertices);
        eprintln!("new_edges: {:?}", result.new_edges);
        eprintln!("new_faces: {:?}", result.new_faces);
        let live_faces = dcel.faces.iter().filter(|f| !f.deleted).count();
        eprintln!("total live faces: {}", live_faces);

        // The self-intersection splits the single segment into 3 sub-edges,
        // creating 1 enclosed loop → at least 2 faces (loop + unbounded).
        assert!(
            live_faces >= 2,
            "expected at least 2 faces (1 loop + unbounded), got {}",
            live_faces,
        );
        assert!(
            result.new_edges.len() >= 3,
            "expected at least 3 sub-edges from self-intersecting segment, got {}",
            result.new_edges.len(),
        );
    }

    #[test]
    fn test_adjacent_segments_crossing() {
        // Two adjacent segments that cross each other.
        // seg0 is an S-curve going right; seg1 comes back left, crossing seg0
        // in the middle.
        let mut dcel = Dcel::new();

        // seg0 curves up-right, seg1 curves down-left, they cross.
        let seg0 = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(200.0, 0.0),
            Point::new(200.0, 100.0),
            Point::new(100.0, 50.0),
        );
        let seg1 = CubicBez::new(
            Point::new(100.0, 50.0),
            Point::new(0.0, 0.0),   // pulls back left
            Point::new(0.0, 100.0),
            Point::new(200.0, 100.0),
        );

        let result = dcel.insert_stroke(&[seg0, seg1], None, None, 5.0);

        eprintln!("new_vertices: {:?}", result.new_vertices);
        eprintln!("new_edges: {:?}", result.new_edges);
        eprintln!("new_faces: {:?}", result.new_faces);
        let live_faces = dcel.faces.iter().filter(|f| !f.deleted).count();
        eprintln!("total live faces: {}", live_faces);

        // If the segments cross, we expect at least one new face beyond unbounded.
        // If they don't cross, at least verify the stroke inserted without panic.
        assert!(
            result.new_edges.len() >= 2,
            "expected at least 2 edges, got {}",
            result.new_edges.len(),
        );
    }

    #[test]
    fn test_cross_then_circle() {
        // Draw a cross (two strokes), then a circle crossing all 4 arms.
        // This exercises insert_edge's angular half-edge selection at vertices
        // where multiple edges share the same face.
        let mut dcel = Dcel::new();

        // Horizontal stroke: (-100, 0) → (100, 0)
        let h_seg = line_curve(Point::new(-100.0, 0.0), Point::new(100.0, 0.0));
        dcel.insert_stroke(&[h_seg], None, None, 5.0);

        // Vertical stroke: (0, -100) → (0, 100) — crosses horizontal at origin
        let v_seg = line_curve(Point::new(0.0, -100.0), Point::new(0.0, 100.0));
        dcel.insert_stroke(&[v_seg], None, None, 5.0);

        let faces_before = dcel.faces.iter().filter(|f| !f.deleted).count();
        eprintln!("faces after cross: {}", faces_before);

        // Circle as 4 cubic segments, radius 50, centered at origin.
        // Each arc covers 90 degrees.
        // Using the standard cubic approximation: k = 4*(sqrt(2)-1)/3 ≈ 0.5523
        let r = 50.0;
        let k = r * 0.5522847498;
        let circle_segs = [
            // Top-right arc: (r,0) → (0,r)
            CubicBez::new(
                Point::new(r, 0.0), Point::new(r, k),
                Point::new(k, r), Point::new(0.0, r),
            ),
            // Top-left arc: (0,r) → (-r,0)
            CubicBez::new(
                Point::new(0.0, r), Point::new(-k, r),
                Point::new(-r, k), Point::new(-r, 0.0),
            ),
            // Bottom-left arc: (-r,0) → (0,-r)
            CubicBez::new(
                Point::new(-r, 0.0), Point::new(-r, -k),
                Point::new(-k, -r), Point::new(0.0, -r),
            ),
            // Bottom-right arc: (0,-r) → (r,0)
            CubicBez::new(
                Point::new(0.0, -r), Point::new(k, -r),
                Point::new(r, -k), Point::new(r, 0.0),
            ),
        ];
        let result = dcel.insert_stroke(&circle_segs, None, None, 5.0);

        let live_faces = dcel.faces.iter().filter(|f| !f.deleted).count();
        eprintln!("faces after circle: {} (new_faces: {:?})", live_faces, result.new_faces);
        eprintln!("new_edges: {}", result.new_edges.len());

        // The circle crosses all 4 arms, creating 4 intersection vertices.
        // This should produce several faces (the 4 quadrant sectors inside the
        // circle, plus the outside). validate() checks face consistency.
        // The key assertion: it doesn't panic.
        assert!(
            live_faces >= 5,
            "expected at least 5 faces (4 inner sectors + unbounded), got {}",
            live_faces,
        );
    }

    #[test]
    fn test_drag_edge_into_self_intersection() {
        // Insert a straight edge, then edit its curve to loop back on itself.
        // recompute_edge_intersections should detect the self-crossing and split.
        let mut dcel = Dcel::new();

        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(200.0, 0.0));
        let straight = line_curve(Point::new(0.0, 0.0), Point::new(200.0, 0.0));
        let (edge_id, _) = dcel.insert_edge(v1, v2, FaceId(0), straight);
        dcel.validate();

        let edges_before = dcel.edges.iter().filter(|e| !e.deleted).count();

        // Now "drag" the edge into a self-intersecting loop (same curve as the
        // single-segment self-intersection test).
        dcel.edges[edge_id.idx()].curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(300.0, 150.0),
            Point::new(-100.0, 150.0),
            Point::new(200.0, 0.0),
        );

        let result = dcel.recompute_edge_intersections(edge_id);

        let edges_after = dcel.edges.iter().filter(|e| !e.deleted).count();
        let faces_after = dcel.faces.iter().filter(|f| !f.deleted).count();
        eprintln!("created: {:?}", result);
        eprintln!("edges: {} → {}", edges_before, edges_after);
        eprintln!("faces: {}", faces_after);

        // The edge should have been split at the self-crossing.
        assert!(
            edges_after > edges_before,
            "expected edge to be split by self-intersection ({} → {})",
            edges_before,
            edges_after,
        );
        assert!(
            faces_after >= 2,
            "expected at least 2 faces (loop + unbounded), got {}",
            faces_after,
        );
    }
}
