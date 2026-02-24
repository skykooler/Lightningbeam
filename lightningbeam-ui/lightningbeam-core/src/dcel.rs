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

    /// Debug recorder: captures strokes and paint bucket clicks for test generation.
    /// Enable with `dcel.set_recording(true)`.
    #[serde(skip)]
    pub debug_recorder: Option<DebugRecorder>,
}

/// Records DCEL operations for test case generation.
#[derive(Clone, Debug, Default)]
pub struct DebugRecorder {
    pub strokes: Vec<Vec<CubicBez>>,
    pub paint_points: Vec<Point>,
}

impl DebugRecorder {
    /// Record a stroke (called from insert_stroke).
    pub fn record_stroke(&mut self, segments: &[CubicBez]) {
        self.strokes.push(segments.to_vec());
    }

    /// Record a paint bucket click (called from find_face_containing_point).
    pub fn record_paint(&mut self, point: Point) {
        self.paint_points.push(point);
    }

    /// Dump a Rust test function to stderr that reproduces the recorded operations.
    pub fn dump_test(&self, name: &str) {
        eprintln!("    #[test]");
        eprintln!("    fn {name}() {{");
        eprintln!("        let mut dcel = Dcel::new();");
        eprintln!();

        for (i, stroke) in self.strokes.iter().enumerate() {
            eprintln!("        // Stroke {i}");
            eprintln!("        dcel.insert_stroke(&[");
            for seg in stroke {
                eprintln!(
                    "            CubicBez::new(Point::new({:.1}, {:.1}), Point::new({:.1}, {:.1}), Point::new({:.1}, {:.1}), Point::new({:.1}, {:.1})),",
                    seg.p0.x, seg.p0.y, seg.p1.x, seg.p1.y,
                    seg.p2.x, seg.p2.y, seg.p3.x, seg.p3.y,
                );
            }
            eprintln!("        ], None, None, 5.0);");
            eprintln!();
        }

        if !self.paint_points.is_empty() {
            eprintln!("        // Each paint point should hit a bounded face, and no two should share a face");
            eprintln!("        let paint_points = vec![");
            for pt in &self.paint_points {
                eprintln!("            Point::new({:.1}, {:.1}),", pt.x, pt.y);
            }
            eprintln!("        ];");
            eprintln!("        let mut seen_faces = std::collections::HashSet::new();");
            eprintln!("        for (i, &pt) in paint_points.iter().enumerate() {{");
            eprintln!("            let face = dcel.find_face_containing_point(pt);");
            eprintln!("            eprintln!(\"paint point {{i}} at ({{:.1}}, {{:.1}}) → face {{:?}}\", pt.x, pt.y, face);");
            eprintln!("            assert!(");
            eprintln!("                face.0 != 0,");
            eprintln!("                \"paint point {{i}} at ({{:.1}}, {{:.1}}) hit unbounded face\",");
            eprintln!("                pt.x, pt.y,");
            eprintln!("            );");
            eprintln!("            assert!(");
            eprintln!("                seen_faces.insert(face),");
            eprintln!("                \"paint point {{i}} at ({{:.1}}, {{:.1}}) hit face {{:?}} which was already painted\",");
            eprintln!("                pt.x, pt.y, face,");
            eprintln!("            );");
            eprintln!("        }}");
        }

        eprintln!("    }}");
    }

    /// Dump the test to stderr and clear the recorder for the next test.
    pub fn dump_and_reset(&mut self, name: &str) {
        self.dump_test(name);
        self.strokes.clear();
        self.paint_points.clear();
    }
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
        let debug_recorder = if std::env::var("DAW_DCEL_RECORD").is_ok() {
            eprintln!("[DCEL_RECORD] Recording enabled for new DCEL");
            Some(DebugRecorder::default())
        } else {
            None
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
            debug_recorder,
        }
    }

    /// Enable or disable debug recording at runtime.
    pub fn set_recording(&mut self, enabled: bool) {
        if enabled {
            self.debug_recorder.get_or_insert_with(DebugRecorder::default);
        } else {
            self.debug_recorder = None;
        }
    }

    /// Returns true if debug recording is active.
    pub fn is_recording(&self) -> bool {
        self.debug_recorder.is_some()
    }

    /// Dump the recorded test and reset the recorder.
    /// Does nothing if recording is not active.
    pub fn dump_recorded_test(&mut self, name: &str) {
        if let Some(ref mut rec) = self.debug_recorder {
            rec.dump_and_reset(name);
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

    /// Build a BezPath from a half-edge cycle (raw, no spur stripping).
    /// Used for topology operations (winding tests, area comparisons).
    fn cycle_to_bezpath(&self, cycle: &[HalfEdgeId]) -> BezPath {
        self.halfedges_to_bezpath(cycle)
    }

    /// Build a BezPath with spur edges and vertex-revisit loops stripped.
    ///
    /// Spur edges (antennae) appear in the cycle as consecutive pairs that
    /// traverse the same edge in opposite directions. These contribute zero
    /// area but can cause fill rendering artifacts when the path is rasterized.
    ///
    /// Vertex-revisit loops occur when a face cycle visits the same vertex
    /// twice (e.g. A→B→C→D→E→C→F). The sub-path between the two visits
    /// (C→D→E→C) is a peninsula that inflates the cycle without enclosing
    /// additional area. We keep the last visit to each vertex and drop
    /// the loop: A→B→C→F.
    fn cycle_to_bezpath_stripped(&self, cycle: &[HalfEdgeId]) -> BezPath {
        let stripped = self.strip_cycle(cycle);
        if stripped.is_empty() {
            return BezPath::new();
        }
        self.halfedges_to_bezpath(&stripped)
    }

    /// Strip spur edges and vertex-revisit loops from a half-edge cycle.
    ///
    /// Returns the simplified list of half-edge IDs.
    fn strip_cycle(&self, cycle: &[HalfEdgeId]) -> Vec<HalfEdgeId> {
        // Pass 1: strip consecutive same-edge spur pairs (stack-based)
        let mut stripped: Vec<HalfEdgeId> = Vec::with_capacity(cycle.len());
        for &he_id in cycle {
            let edge = self.half_edge(he_id).edge;
            if let Some(&top) = stripped.last() {
                if self.half_edge(top).edge == edge {
                    stripped.pop();
                    continue;
                }
            }
            stripped.push(he_id);
        }
        // Handle wrap-around spur pairs.
        while stripped.len() >= 2 {
            let first_edge = self.half_edge(stripped[0]).edge;
            let last_edge = self.half_edge(*stripped.last().unwrap()).edge;
            if first_edge == last_edge {
                stripped.pop();
                stripped.remove(0);
            } else {
                break;
            }
        }

        // Pass 2: strip vertex-revisit loops.
        // Walk the stripped cycle. For each half-edge, record the *source*
        // vertex. If we've seen that vertex before, remove the sub-path
        // between the first and current visit (keeping the later path).
        //
        // We repeat until no more revisits are found, since removing one
        // loop can expose another.
        let mut changed = true;
        while changed {
            changed = false;
            let mut result: Vec<HalfEdgeId> = Vec::with_capacity(stripped.len());
            // Map from VertexId → index in `result` where that vertex was last seen as source
            let mut vertex_pos: std::collections::HashMap<VertexId, usize> = std::collections::HashMap::new();
            for &he_id in &stripped {
                let src = self.half_edge_source(he_id);
                if let Some(&prev_pos) = vertex_pos.get(&src) {
                    // Vertex revisit! Remove the loop between prev_pos and here.
                    // Keep result[0..prev_pos], drop result[prev_pos..], continue from here.
                    // Also remove stale vertex_pos entries for dropped half-edges.
                    let removed: Vec<HalfEdgeId> = result.drain(prev_pos..).collect();
                    for &removed_he in &removed {
                        let removed_src = self.half_edge_source(removed_he);
                        // Only remove from map if it points to a removed position
                        if let Some(&pos) = vertex_pos.get(&removed_src) {
                            if pos >= prev_pos {
                                vertex_pos.remove(&removed_src);
                            }
                        }
                    }
                    changed = true;
                }
                vertex_pos.insert(src, result.len());
                result.push(he_id);
            }
            // Check wrap-around: if the last half-edge's destination == first half-edge's source,
            // that's the expected cycle closure, not a revisit. But if the destination appears
            // as a source of some middle half-edge, we have a wrap-around revisit.
            if !result.is_empty() {
                let last_he = *result.last().unwrap();
                let last_dst = self.half_edge_dest(last_he);
                let first_src = self.half_edge_source(result[0]);
                if last_dst != first_src {
                    // The destination of the last edge should match the source of the first
                    // for a valid cycle. If not, something is off — don't strip further.
                } else if let Some(&wrap_pos) = vertex_pos.get(&first_src) {
                    if wrap_pos > 0 {
                        // The cycle start vertex appears mid-cycle. Drop the prefix.
                        result.drain(..wrap_pos);
                        changed = true;
                    }
                }
            }
            stripped = result;
        }

        stripped
    }

    /// Get the source (origin) vertex of a half-edge.
    #[inline]
    fn half_edge_source(&self, he_id: HalfEdgeId) -> VertexId {
        self.half_edge(he_id).origin
    }

    /// Convert a slice of half-edge IDs to a BezPath.
    fn halfedges_to_bezpath(&self, hes: &[HalfEdgeId]) -> BezPath {
        let mut path = BezPath::new();
        if hes.is_empty() {
            return path;
        }
        for (i, &he_id) in hes.iter().enumerate() {
            let he = self.half_edge(he_id);
            let edge_data = self.edge(he.edge);
            let is_forward = edge_data.half_edges[0] == he_id;
            let curve = if is_forward {
                edge_data.curve
            } else {
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

    /// Build a BezPath for a face with spur edges stripped (for fill rendering).
    ///
    /// Spur edges cause fill rendering artifacts because the back-and-forth
    /// path can enclose neighboring regions. Use this for all rendering;
    /// use `face_to_bezpath` (raw) for topology operations like winding tests.
    pub fn face_to_bezpath_stripped(&self, face_id: FaceId) -> BezPath {
        let boundary = self.face_boundary(face_id);
        self.cycle_to_bezpath_stripped(&boundary)
    }

    /// Build a BezPath for a face including holes (for correct filled rendering).
    /// Outer boundary is CCW, holes are CW (opposite winding for non-zero fill).
    /// Spur edges are stripped.
    pub fn face_to_bezpath_with_holes(&self, face_id: FaceId) -> BezPath {
        let boundary = self.face_boundary(face_id);
        let mut path = self.cycle_to_bezpath_stripped(&boundary);

        let face = self.face(face_id);
        for &inner_he in &face.inner_half_edges {
            let hole_cycle = self.walk_cycle(inner_he);
            let hole_path = self.cycle_to_bezpath_stripped(&hole_cycle);
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

        // 6. No unsplit crossings: every pair of non-deleted edges that
        //    geometrically cross must share a vertex at the crossing point.
        //    An interior crossing (away from endpoints) without a shared
        //    vertex means insert_stroke failed to split the edge.
        {
            use crate::curve_intersections::find_curve_intersections;

            // Collect live edges with their endpoint vertex IDs.
            let live_edges: Vec<(EdgeId, CubicBez, [VertexId; 2])> = self
                .edges
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.deleted)
                .map(|(i, e)| {
                    let eid = EdgeId(i as u32);
                    let v0 = self.half_edges[e.half_edges[0].idx()].origin;
                    let v1 = self.half_edges[e.half_edges[1].idx()].origin;
                    (eid, e.curve, [v0, v1])
                })
                .collect();

            for i in 0..live_edges.len() {
                for j in (i + 1)..live_edges.len() {
                    let (eid_a, curve_a, verts_a) = &live_edges[i];
                    let (eid_b, curve_b, verts_b) = &live_edges[j];

                    // Shared endpoint vertices — intersections near endpoints are expected.
                    let shared: Vec<VertexId> = verts_a
                        .iter()
                        .filter(|v| verts_b.contains(v))
                        .copied()
                        .collect();

                    let hits = find_curve_intersections(curve_a, curve_b);
                    for hit in &hits {
                        let t1 = hit.t1;
                        let t2 = hit.t2.unwrap_or(0.5);

                        // Check if intersection is close to a shared endpoint vertex.
                        // This handles edges that share a vertex and run nearly
                        // parallel near the junction — the intersection finder can
                        // report a hit a few pixels from the shared vertex.
                        let close_to_shared = shared.iter().any(|&sv| {
                            let sv_pos = self.vertex(sv).position;
                            (hit.point - sv_pos).hypot() < 2.0
                        });
                        if close_to_shared {
                            continue;
                        }

                        // Skip intersections that are at/near both endpoints
                        // (shared vertex at a T-junction or crossing already resolved).
                        let near_endpoint_a = t1 < 0.02 || t1 > 0.98;
                        let near_endpoint_b = t2 < 0.02 || t2 > 0.98;
                        if near_endpoint_a && near_endpoint_b {
                            continue;
                        }

                        // Interior crossing — check if ANY vertex exists near this point.
                        let has_vertex_at_crossing = self.vertices.iter().any(|v| {
                            !v.deleted && (v.position - hit.point).hypot() < 2.0
                        });

                        assert!(
                            has_vertex_at_crossing,
                            "Unsplit edge crossing: edge {:?} (t={:.3}) x edge {:?} (t={:.3}) \
                             at ({:.1}, {:.1}) — no vertex at crossing point.\n\
                             Edge A vertices: V{} ({:.1},{:.1}) → V{} ({:.1},{:.1})\n\
                             Edge B vertices: V{} ({:.1},{:.1}) → V{} ({:.1},{:.1})",
                            eid_a, t1, eid_b, t2, hit.point.x, hit.point.y,
                            verts_a[0].0, self.vertex(verts_a[0]).position.x, self.vertex(verts_a[0]).position.y,
                            verts_a[1].0, self.vertex(verts_a[1]).position.x, self.vertex(verts_a[1]).position.y,
                            verts_b[0].0, self.vertex(verts_b[0]).position.x, self.vertex(verts_b[0]).position.y,
                            verts_b[1].0, self.vertex(verts_b[1]).position.x, self.vertex(verts_b[1]).position.y,
                        );
                    }
                }
            }
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

                let actual_face = self.half_edges[he_into_v1.idx()].face;

                if cfg!(test) && std::env::var("DCEL_TRACE").is_ok() {
                    let face_v1 = self.half_edges[he_into_v1.idx()].face;
                    let face_v2 = self.half_edges[he_into_v2.idx()].face;
                    eprintln!("    (true,true) v1=V{} v2=V{} fwd_angle={:.3} bwd_angle={:.3}",
                        v1.0, v2.0, fwd_angle, bwd_angle);
                    // Dump fan at v1
                    {
                        let start = self.vertices[v1.idx()].outgoing;
                        let mut cur = start;
                        eprint!("      v1 fan:");
                        loop {
                            let a = self.outgoing_angle(cur);
                            let f = self.half_edge(cur).face;
                            eprint!(" HE{}(a={:.3},F{})", cur.0, a, f.0);
                            let twin = self.half_edge(cur).twin;
                            cur = self.half_edge(twin).next;
                            if cur == start { break; }
                        }
                        eprintln!();
                    }
                    // Dump fan at v2
                    {
                        let start = self.vertices[v2.idx()].outgoing;
                        let mut cur = start;
                        eprint!("      v2 fan:");
                        loop {
                            let a = self.outgoing_angle(cur);
                            let f = self.half_edge(cur).face;
                            eprint!(" HE{}(a={:.3},F{})", cur.0, a, f.0);
                            let twin = self.half_edge(cur).twin;
                            cur = self.half_edge(twin).next;
                            if cur == start { break; }
                        }
                        eprintln!();
                    }
                    eprintln!("      he_from_v1=HE{} he_into_v1=HE{} face_at_v1=F{}",
                        he_from_v1.0, he_into_v1.0, face_v1.0);
                    eprintln!("      he_from_v2=HE{} he_into_v2=HE{} face_at_v2=F{}",
                        he_from_v2.0, he_into_v2.0, face_v2.0);
                }

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

                // Detect split vs bridge: walk from he_fwd and check if
                // we encounter he_bwd (same cycle = bridge) or return to
                // he_fwd without seeing it (separate cycles = split).
                let is_split = {
                    let mut cur = self.half_edges[he_fwd.idx()].next;
                    let mut found = false;
                    while cur != he_fwd {
                        if cur == he_bwd {
                            found = true;
                            break;
                        }
                        cur = self.half_edges[cur.idx()].next;
                    }
                    !found
                };

                if cfg!(test) && std::env::var("DCEL_TRACE").is_ok() {
                    // Dump the cycle from he_fwd
                    eprint!("      fwd_cycle:");
                    let mut cur = he_fwd;
                    let mut count = 0;
                    loop {
                        eprint!(" HE{}", cur.0);
                        cur = self.half_edges[cur.idx()].next;
                        count += 1;
                        if cur == he_fwd || count > 50 { break; }
                    }
                    eprintln!(" (len={})", count);
                    eprint!("      bwd_cycle:");
                    cur = he_bwd;
                    count = 0;
                    loop {
                        eprint!(" HE{}", cur.0);
                        cur = self.half_edges[cur.idx()].next;
                        count += 1;
                        if cur == he_bwd || count > 50 { break; }
                    }
                    eprintln!(" (len={})", count);
                    eprintln!("      is_split={is_split} actual_face=F{}", actual_face.0);
                }

                if is_split {
                    // Normal case: splice split one cycle into two.
                    let new_face = self.alloc_face();

                    // Decide which cycle keeps actual_face and which gets new_face.
                    //
                    // For the unbounded face (FaceId(0)), we must keep FaceId(0) on
                    // the exterior cycle. The interior (bounded) cycle becomes the
                    // new face. We detect this by computing the signed area of each
                    // cycle via the bezpath: positive area = CCW interior, negative
                    // or larger absolute = CW exterior.
                    let (he_old, he_new) = if actual_face.0 == 0 {
                        // Compute signed area of both cycles to determine which is
                        // the exterior. The exterior has larger absolute area.
                        let fwd_cycle = self.walk_cycle(he_fwd);
                        let bwd_cycle = self.walk_cycle(he_bwd);
                        let fwd_path = self.cycle_to_bezpath(&fwd_cycle);
                        let bwd_path = self.cycle_to_bezpath(&bwd_cycle);
                        let fwd_area = kurbo::Shape::area(&fwd_path);
                        let bwd_area = kurbo::Shape::area(&bwd_path);
                        if fwd_area.abs() < bwd_area.abs() {
                            // he_fwd is the smaller (interior) → he_fwd gets new_face
                            (he_bwd, he_fwd)
                        } else {
                            // he_fwd is the larger (exterior) → he_bwd gets new_face
                            (he_fwd, he_bwd)
                        }
                    } else {
                        // For bounded faces, convention: he_fwd → old, he_bwd → new
                        (he_fwd, he_bwd)
                    };

                    self.half_edges[he_old.idx()].face = actual_face;
                    {
                        let mut cur = self.half_edges[he_old.idx()].next;
                        while cur != he_old {
                            self.half_edges[cur.idx()].face = actual_face;
                            cur = self.half_edges[cur.idx()].next;
                        }
                    }
                    self.half_edges[he_new.idx()].face = new_face;
                    {
                        let mut cur = self.half_edges[he_new.idx()].next;
                        while cur != he_new {
                            self.half_edges[cur.idx()].face = new_face;
                            cur = self.half_edges[cur.idx()].next;
                        }
                    }

                    self.faces[actual_face.idx()].outer_half_edge = he_old;
                    self.faces[new_face.idx()].outer_half_edge = he_new;

                    return (edge_id, new_face);
                } else {
                    // Bridge case: splice merged two cycles into one.
                    // No face split — assign the whole cycle to actual_face.
                    self.half_edges[he_fwd.idx()].face = actual_face;
                    {
                        let mut cur = self.half_edges[he_fwd.idx()].next;
                        while cur != he_fwd {
                            self.half_edges[cur.idx()].face = actual_face;
                            cur = self.half_edges[cur.idx()].next;
                        }
                    }
                    if actual_face.0 != 0 {
                        self.faces[actual_face.idx()].outer_half_edge = he_fwd;
                    }

                    return (edge_id, actual_face);
                }
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
    /// If an existing vertex is within snap tolerance of the split point,
    /// it is reused so that crossing strokes share the same vertex.
    /// Returns `(new_vertex_id, new_edge_id)`.
    pub fn split_edge(&mut self, edge_id: EdgeId, t: f64) -> (VertexId, EdgeId) {
        debug_assert!((0.0..=1.0).contains(&t), "t must be in [0, 1]");

        let original_curve = self.edges[edge_id.idx()].curve;
        // De Casteljau subdivision
        let (curve_a, curve_b) = subdivide_cubic(original_curve, t);

        let split_point = curve_a.p3; // == curve_b.p0
        let new_vertex = self
            .snap_vertex(split_point, DEFAULT_SNAP_EPSILON)
            .unwrap_or_else(|| self.alloc_vertex(split_point));

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

        // Record the stroke for debug test generation
        if let Some(ref mut rec) = self.debug_recorder {
            eprintln!("[DCEL_RECORD] insert_stroke: recording {} segments (total strokes: {})",
                segments.len(), rec.strokes.len() + 1);
            rec.record_stroke(segments);
        }

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
            // Sort descending by t so we split from end to start.
            // After each split, current_edge is the lower portion [0, t] in original
            // parameter space. Its parameter 1.0 maps to t in original space.
            splits.sort_by(|a, b| b.t.partial_cmp(&a.t).unwrap());

            let current_edge = splits[0].edge_id;
            // Upper bound of current_edge's range in original parameter space.
            // Initially [0, 1], then [0, t_high] after first split, etc.
            let mut current_t_end = 1.0_f64;

            for split in &splits {
                // Remap original t to current_edge's parameter space [0, 1]
                // which maps to original [0, current_t_end].
                let t_in_current = if current_t_end > 1e-12 {
                    split.t / current_t_end
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

                // After splitting at t_in_current, current_edge now covers
                // [0, split.t] in original space. Update the upper bound.
                current_t_end = split.t;
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
                let mid = midpoint_of_cubic(&sub_curve);
                let face = self.find_face_containing_point(mid);

                if cfg!(test) && std::env::var("DCEL_TRACE").is_ok() {
                    let p1 = self.vertices[prev_vertex.idx()].position;
                    let p2 = self.vertices[vertex.idx()].position;
                    eprintln!("  insert_edge: V{}({:.1},{:.1}) → V{}({:.1},{:.1}) face=F{} mid=({:.1},{:.1})",
                        prev_vertex.0, p1.x, p1.y, vertex.0, p2.x, p2.y, face.0, mid.x, mid.y);
                }

                let (edge_id, maybe_new_face) =
                    self.insert_edge(prev_vertex, *vertex, face, sub_curve);

                if cfg!(test) && std::env::var("DCEL_TRACE").is_ok() {
                    eprintln!("    → E{} new_face=F{}", edge_id.0, maybe_new_face.0);
                }

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

    /// Record a paint bucket click point for debug test generation.
    /// Call this before `find_face_containing_point` when the paint bucket is used.
    pub fn record_paint_point(&mut self, point: Point) {
        if let Some(ref mut rec) = self.debug_recorder {
            eprintln!("[DCEL_RECORD] paint_point: ({:.1}, {:.1}) (total points: {})",
                point.x, point.y, rec.paint_points.len() + 1);
            rec.record_paint(point);
        }
    }

    /// Find which face contains a given point.
    ///
    /// Returns the smallest-area face whose boundary encloses the point.
    /// This handles the case where a large "exterior boundary" face encloses
    /// smaller interior faces — we want the innermost one.
    /// Returns FaceId(0) (unbounded) if no bounded face contains the point.
    pub fn find_face_containing_point(&self, point: Point) -> FaceId {
        use kurbo::Shape;
        let mut best_face = FaceId(0);
        let mut best_area = f64::MAX;

        for (i, face) in self.faces.iter().enumerate() {
            if face.deleted || i == 0 {
                continue;
            }
            if face.outer_half_edge.is_none() {
                continue;
            }
            // Use stripped cycle to avoid bloated winding/area from spur
            // edges and vertex-revisit peninsulas.
            let path = self.face_to_bezpath_stripped(FaceId(i as u32));
            if path.winding(point) != 0 {
                let area = path.area().abs();
                if area < best_area {
                    best_area = area;
                    best_face = FaceId(i as u32);
                }
            }
        }
        best_face
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

    /// Render all filled faces of a DCEL to a tiny-skia pixmap.
    /// Returns the pixmap so callers can check pixel values.
    fn render_dcel_fills(dcel: &Dcel, width: u32, height: u32) -> tiny_skia::Pixmap {
        let mut pixmap = tiny_skia::Pixmap::new(width, height).unwrap();

        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            if face.fill_color.is_none() { continue; }
            if face.outer_half_edge.is_none() { continue; }

            let bez = dcel.face_to_bezpath_stripped(FaceId(i as u32));

            // Convert kurbo BezPath to tiny-skia PathBuilder
            let mut pb = tiny_skia::PathBuilder::new();
            for el in bez.elements() {
                match el {
                    kurbo::PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
                    kurbo::PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
                    kurbo::PathEl::CurveTo(p1, p2, p3) => {
                        pb.cubic_to(
                            p1.x as f32, p1.y as f32,
                            p2.x as f32, p2.y as f32,
                            p3.x as f32, p3.y as f32,
                        );
                    }
                    kurbo::PathEl::QuadTo(p1, p2) => {
                        pb.quad_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32);
                    }
                    kurbo::PathEl::ClosePath => pb.close(),
                }
            }

            if let Some(path) = pb.finish() {
                let paint = tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(
                        tiny_skia::Color::from_rgba8(0, 0, 255, 255),
                    ),
                    anti_alias: false,
                    ..Default::default()
                };
                pixmap.fill_path(
                    &path,
                    &paint,
                    tiny_skia::FillRule::Winding,
                    tiny_skia::Transform::identity(),
                    None,
                );
            }
        }

        pixmap
    }

    /// Check that a pixel at (x, y) is NOT filled (is transparent/background).
    fn assert_pixel_unfilled(pixmap: &tiny_skia::Pixmap, x: f64, y: f64, msg: &str) {
        let px = x.round() as u32;
        let py = y.round() as u32;
        if px >= pixmap.width() || py >= pixmap.height() {
            panic!("{msg}: point ({x:.1}, {y:.1}) is outside the pixmap");
        }
        let pixel = pixmap.pixel(px, py).unwrap();
        assert!(
            pixel.alpha() == 0,
            "{msg}: pixel at ({x:.1}, {y:.1}) is already filled (rgba={},{},{},{})",
            pixel.red(), pixel.green(), pixel.blue(), pixel.alpha(),
        );
    }

    /// Simulate paint bucket clicks: for each point, assert the pixel is unfilled,
    /// find the face, fill it, re-render, and continue.
    fn assert_paint_sequence(dcel: &mut Dcel, paint_points: &[Point], width: u32, height: u32) {
        for (i, &pt) in paint_points.iter().enumerate() {
            // Render current state and check this pixel is unfilled
            let pixmap = render_dcel_fills(dcel, width, height);
            assert_pixel_unfilled(
                &pixmap, pt.x, pt.y,
                &format!("paint point {i} at ({:.1}, {:.1})", pt.x, pt.y),
            );

            // Find and fill the face
            let face = dcel.find_face_containing_point(pt);
            assert!(
                face.0 != 0,
                "paint point {i} at ({:.1}, {:.1}) hit unbounded face",
                pt.x, pt.y,
            );
            dcel.face_mut(face).fill_color = Some(ShapeColor::new(0, 0, 255, 255));
        }
    }

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

    #[test]
    fn test_recorded_seven_lines() {
        // 7 line segments drawn across each other, creating triangles/quads/pentagon.
        // Recorded from live editor with DAW_DCEL_RECORD=1.
        let mut dcel = Dcel::new();

        let strokes: Vec<Vec<CubicBez>> = vec![
            vec![CubicBez::new(Point::new(172.3, 252.0), Point::new(342.2, 210.5), Point::new(512.0, 169.1), Point::new(681.8, 127.6))],
            vec![CubicBez::new(Point::new(222.6, 325.7), Point::new(365.7, 248.3), Point::new(508.7, 171.0), Point::new(651.7, 93.7))],
            vec![CubicBez::new(Point::new(210.4, 204.1), Point::new(359.4, 258.0), Point::new(508.4, 311.9), Point::new(657.5, 365.8))],
            vec![CubicBez::new(Point::new(287.5, 333.0), Point::new(323.8, 238.4), Point::new(360.2, 143.9), Point::new(396.6, 49.3))],
            vec![CubicBez::new(Point::new(425.9, 372.1), Point::new(418.7, 258.2), Point::new(411.6, 144.4), Point::new(404.5, 30.5))],
            vec![CubicBez::new(Point::new(363.1, 360.1), Point::new(421.4, 263.3), Point::new(479.8, 166.6), Point::new(538.2, 69.9))],
            vec![CubicBez::new(Point::new(292.8, 99.1), Point::new(398.5, 158.6), Point::new(504.3, 218.2), Point::new(610.0, 277.7))],
        ];

        for segs in &strokes {
            dcel.insert_stroke(segs, None, None, 5.0);
        }

        // Each paint point should hit a bounded face, and no two should share a face
        let paint_points = vec![
            Point::new(312.4, 224.1),
            Point::new(325.5, 259.2),
            Point::new(364.7, 223.4),
            Point::new(402.9, 247.7),
            Point::new(427.2, 226.3),
            Point::new(431.6, 198.7),
            Point::new(421.2, 181.6),
            Point::new(364.7, 177.0),
        ];
        let mut seen_faces = std::collections::HashSet::new();
        for (i, &pt) in paint_points.iter().enumerate() {
            let face = dcel.find_face_containing_point(pt);
            assert!(
                face.0 != 0,
                "paint point {i} at ({:.1}, {:.1}) hit unbounded face",
                pt.x, pt.y,
            );
            assert!(
                seen_faces.insert(face),
                "paint point {i} at ({:.1}, {:.1}) hit face {:?} which was already painted",
                pt.x, pt.y, face,
            );
        }
    }

    #[test]
    fn test_recorded_curves() {
        // 7 curved strokes (one multi-segment). Recorded from live editor.
        let mut dcel = Dcel::new();

        let strokes: Vec<Vec<CubicBez>> = vec![
            vec![CubicBez::new(Point::new(186.9, 301.1), Point::new(295.3, 221.6), Point::new(478.9, 181.7), Point::new(612.8, 148.2))],
            vec![CubicBez::new(Point::new(159.8, 189.5), Point::new(315.6, 210.9), Point::new(500.4, 371.0), Point::new(600.7, 371.0))],
            vec![CubicBez::new(Point::new(279.0, 330.6), Point::new(251.0, 262.7), Point::new(220.9, 175.9), Point::new(245.6, 102.1))],
            vec![CubicBez::new(Point::new(183.3, 119.3), Point::new(250.6, 132.8), Point::new(542.6, 225.7), Point::new(575.6, 225.7))],
            vec![CubicBez::new(Point::new(377.0, 353.6), Point::new(377.0, 280.8), Point::new(369.1, 166.5), Point::new(427.2, 108.5))],
            vec![
                CubicBez::new(Point::new(345.6, 333.3), Point::new(388.4, 299.7), Point::new(436.5, 274.6), Point::new(480.9, 243.5)),
                CubicBez::new(Point::new(480.9, 243.5), Point::new(525.0, 212.5), Point::new(565.2, 174.9), Point::new(610.1, 145.0)),
            ],
            vec![CubicBez::new(Point::new(493.5, 115.8), Point::new(475.6, 199.1), Point::new(461.0, 280.7), Point::new(461.0, 365.6))],
        ];

        for segs in &strokes {
            dcel.insert_stroke(segs, None, None, 5.0);
        }

        let paint_points = vec![
            Point::new(255.6, 232.3),
            Point::new(297.2, 200.0),
            Point::new(342.6, 248.4),
            Point::new(396.0, 192.5),
            Point::new(403.5, 233.3),
            Point::new(442.2, 288.3),
            Point::new(490.6, 218.3),
            Point::new(514.2, 194.9),
        ];
        // Dump per-stroke topology
        // Re-run from scratch with per-stroke tracking
        let mut dcel2 = Dcel::new();
        let strokes2 = strokes.clone();
        for (s, segs) in strokes2.iter().enumerate() {
            dcel2.insert_stroke(segs, None, None, 5.0);
            let face_info: Vec<_> = dcel2.faces.iter().enumerate()
                .filter(|(i, f)| !f.deleted && *i > 0 && !f.outer_half_edge.is_none())
                .map(|(i, _)| {
                    let cycle = dcel2.face_boundary(FaceId(i as u32));
                    (i, cycle.len())
                }).collect();
            eprintln!("After stroke {s}: faces={:?}", face_info);
        }

        // Dump all faces with cycle lengths
        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            if face.outer_half_edge.is_none() { continue; }
            let cycle = dcel.face_boundary(FaceId(i as u32));
            let path = dcel.face_to_bezpath(FaceId(i as u32));
            let area = kurbo::Shape::area(&path).abs();
            eprintln!("  Face {i}: cycle_len={}, area={:.1}", cycle.len(), area);
        }

        let mut seen_faces = std::collections::HashSet::new();
        for (i, &pt) in paint_points.iter().enumerate() {
            let face = dcel.find_face_containing_point(pt);
            let cycle_len = if face.0 != 0 {
                dcel.face_boundary(face).len()
            } else { 0 };
            eprintln!("paint point {i} at ({:.1}, {:.1}) → face {:?} (cycle_len={})", pt.x, pt.y, face, cycle_len);
            assert!(
                face.0 != 0,
                "paint point {i} at ({:.1}, {:.1}) hit unbounded face",
                pt.x, pt.y,
            );
            assert!(
                seen_faces.insert(face),
                "paint point {i} at ({:.1}, {:.1}) hit face {:?} which was already painted",
                pt.x, pt.y, face,
            );
        }
    }

    #[test]
    fn test_recorded_complex_curves() {
        let mut dcel = Dcel::new();

        // Stroke 0
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(285.4, 88.3), Point::new(211.5, 148.8), Point::new(140.3, 214.8), Point::new(98.2, 301.9)),
            CubicBez::new(Point::new(98.2, 301.9), Point::new(83.7, 331.9), Point::new(71.1, 364.5), Point::new(52.5, 392.4)),
        ], None, None, 5.0);

        // Stroke 1
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(96.5, 281.3), Point::new(244.8, 254.4), Point::new(304.4, 327.7), Point::new(427.7, 327.7)),
        ], None, None, 5.0);

        // Stroke 2
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(88.8, 86.7), Point::new(141.9, 105.4), Point::new(194.0, 126.2), Point::new(240.4, 158.6)),
            CubicBez::new(Point::new(240.4, 158.6), Point::new(273.3, 181.6), Point::new(297.7, 213.4), Point::new(327.6, 239.5)),
            CubicBez::new(Point::new(327.6, 239.5), Point::new(378.8, 284.1), Point::new(451.3, 317.7), Point::new(467.3, 389.8)),
            CubicBez::new(Point::new(467.3, 389.8), Point::new(470.1, 402.3), Point::new(480.1, 418.3), Point::new(461.2, 410.8)),
        ], None, None, 5.0);

        // Stroke 3
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(320.6, 375.9), Point::new(359.8, 251.8), Point::new(402.3, 201.6), Point::new(525.7, 160.4)),
        ], None, None, 5.0);

        // Stroke 4
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(72.2, 181.1), Point::new(97.2, 211.1), Point::new(129.2, 234.8), Point::new(154.8, 264.6)),
            CubicBez::new(Point::new(154.8, 264.6), Point::new(182.3, 296.5), Point::new(199.7, 334.9), Point::new(232.1, 363.0)),
            CubicBez::new(Point::new(232.1, 363.0), Point::new(251.8, 380.1), Point::new(276.7, 390.0), Point::new(295.4, 408.7)),
        ], None, None, 5.0);

        // Stroke 5
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(102.9, 316.2), Point::new(167.0, 209.3), Point::new(263.1, 110.6), Point::new(399.0, 110.6)),
        ], None, None, 5.0);

        // Stroke 6
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(159.4, 87.6), Point::new(216.5, 159.0), Point::new(260.1, 346.3), Point::new(229.7, 437.4)),
        ], None, None, 5.0);

        // Points 6, 7, 8 should each hit unique bounded faces
        let paint_points = vec![
            Point::new(217.4, 160.1),
            Point::new(184.2, 242.9),
            Point::new(202.0, 141.4),
        ];
        let mut seen_faces = std::collections::HashSet::new();
        for (i, &pt) in paint_points.iter().enumerate() {
            let face = dcel.find_face_containing_point(pt);
            assert!(
                face.0 != 0,
                "paint point {i} at ({:.1}, {:.1}) hit unbounded face",
                pt.x, pt.y,
            );
            assert!(
                seen_faces.insert(face),
                "paint point {i} at ({:.1}, {:.1}) hit face {:?} which was already painted",
                pt.x, pt.y, face,
            );
        }
    }

    #[test]
    fn test_d_shape_fill() {
        let mut dcel = Dcel::new();

        // Stroke 0: vertical line
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(354.2, 97.9), Point::new(354.2, 208.0), Point::new(357.7, 318.7), Point::new(357.7, 429.0)),
        ], None, None, 5.0);

        // Stroke 1: inner curve of D
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(332.9, 218.6), Point::new(359.1, 224.5), Point::new(386.8, 225.0), Point::new(412.0, 234.5)),
            CubicBez::new(Point::new(412.0, 234.5), Point::new(457.5, 251.5), Point::new(416.1, 313.5), Point::new(287.3, 313.5)),
        ], None, None, 5.0);

        // Stroke 2: outer curve of D
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(319.5, 154.5), Point::new(548.7, 154.5), Point::new(553.4, 359.5), Point::new(337.9, 392.6)),
            CubicBez::new(Point::new(337.9, 392.6), Point::new(310.3, 396.9), Point::new(279.8, 405.8), Point::new(251.8, 398.8)),
        ], None, None, 5.0);

        // The D-shape region should be fillable
        let face = dcel.find_face_containing_point(Point::new(439.8, 319.6));
        assert!(face.0 != 0, "D-shape region hit unbounded face");
    }

    #[test]
    fn test_recorded_seven_strokes() {
        let mut dcel = Dcel::new();

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(194.8, 81.4), Point::new(314.0, 126.0), Point::new(413.6, 198.4), Point::new(518.5, 268.3)),
            CubicBez::new(Point::new(518.5, 268.3), Point::new(558.0, 294.7), Point::new(598.6, 322.6), Point::new(638.9, 347.4)),
            CubicBez::new(Point::new(638.9, 347.4), Point::new(646.8, 352.3), Point::new(672.4, 358.1), Point::new(663.5, 360.6)),
            CubicBez::new(Point::new(663.5, 360.6), Point::new(654.9, 363.0), Point::new(644.3, 358.5), Point::new(636.2, 356.2)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(223.9, 308.2), Point::new(392.2, 242.0), Point::new(603.6, 211.2), Point::new(786.1, 211.2)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(157.2, 201.6), Point::new(287.7, 136.3), Point::new(442.7, 100.0), Point::new(589.3, 100.0)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(247.4, 56.4), Point::new(284.2, 122.7), Point::new(271.2, 201.4), Point::new(289.0, 272.2)),
            CubicBez::new(Point::new(289.0, 272.2), Point::new(298.4, 310.2), Point::new(314.3, 344.7), Point::new(327.6, 380.0)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(249.3, 383.6), Point::new(287.6, 353.0), Point::new(604.8, 19.5), Point::new(612.9, 17.5)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(436.9, 73.9), Point::new(520.7, 157.8), Point::new(574.8, 262.5), Point::new(574.8, 383.2)),
        ], None, None, 5.0);

        dcel.insert_stroke(&[
            CubicBez::new(Point::new(361.1, 356.7), Point::new(311.8, 291.0), Point::new(299.5, 204.6), Point::new(174.0, 183.6)),
        ], None, None, 5.0);

        let paint_points = vec![
            Point::new(303.9, 296.1),
            Point::new(290.4, 260.4),
            Point::new(245.1, 186.4),
            Point::new(284.2, 133.3),
            Point::new(334.1, 201.4),
            Point::new(329.3, 283.7),
            Point::new(425.6, 229.4),
            Point::new(405.2, 145.5),
            Point::new(492.9, 115.7),
            Point::new(480.0, 208.1),
            Point::new(521.2, 249.9),
        ];
        assert_paint_sequence(&mut dcel, &paint_points, 800, 450);
    }

    #[test]
    fn test_recorded_eight_strokes() {
        let mut dcel = Dcel::new();

        // Stroke 0
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(205.0, 366.2), Point::new(244.7, 255.0), Point::new(301.5, 184.3), Point::new(398.7, 119.5)),
            CubicBez::new(Point::new(398.7, 119.5), Point::new(419.4, 105.7), Point::new(438.3, 87.0), Point::new(464.6, 87.0)),
        ], None, None, 5.0);

        // Stroke 1
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(131.7, 126.8), Point::new(278.6, 184.4), Point::new(420.9, 260.3), Point::new(570.1, 310.0)),
        ], None, None, 5.0);

        // Stroke 2
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(252.7, 369.6), Point::new(245.6, 297.8), Point::new(246.6, 225.3), Point::new(240.6, 153.5)),
            CubicBez::new(Point::new(240.6, 153.5), Point::new(238.9, 132.9), Point::new(228.3, 112.7), Point::new(228.3, 92.0)),
        ], None, None, 5.0);

        // Stroke 3
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(362.6, 105.6), Point::new(317.6, 210.5), Point::new(160.1, 315.5), Point::new(149.0, 332.1)),
        ], None, None, 5.0);

        // Stroke 4
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(134.6, 218.2), Point::new(228.4, 208.3), Point::new(368.1, 233.7), Point::new(458.8, 263.9)),
        ], None, None, 5.0);

        // Stroke 5
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(329.0, 300.6), Point::new(339.5, 221.5), Point::new(342.3, 147.5), Point::new(316.7, 70.4)),
        ], None, None, 5.0);

        // Stroke 6
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(186.0, 99.2), Point::new(263.5, 118.6), Point::new(342.2, 129.8), Point::new(417.9, 156.3)),
            CubicBez::new(Point::new(417.9, 156.3), Point::new(456.4, 169.8), Point::new(494.6, 191.3), Point::new(533.9, 201.1)),
        ], None, None, 5.0);

        // Stroke 7
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(287.5, 73.5), Point::new(266.9, 135.2), Point::new(224.9, 188.7), Point::new(202.3, 251.0)),
            CubicBez::new(Point::new(202.3, 251.0), Point::new(187.7, 291.0), Point::new(194.5, 335.7), Point::new(181.2, 375.8)),
        ], None, None, 5.0);

        // Dump face topology after all strokes
        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            if face.outer_half_edge.is_none() { continue; }
            let cycle = dcel.face_boundary(FaceId(i as u32));
            let path = dcel.face_to_bezpath(FaceId(i as u32));
            let area = kurbo::Shape::area(&path).abs();
            eprintln!("  Face {i}: cycle_len={}, area={:.1}", cycle.len(), area);
            if cycle.len() > 20 {
                // Dump the full cycle for bloated faces
                let start = face.outer_half_edge;
                let mut cur = start;
                let mut step = 0;
                loop {
                    let he = &dcel.half_edges[cur.idx()];
                    let origin = he.origin;
                    let pos = dcel.vertices[origin.idx()].position;
                    let edge = he.edge;
                    let twin = dcel.edges[edge.idx()].half_edges;
                    let is_fwd = twin[0] == cur;
                    eprintln!("    step {step}: he={:?} origin={:?} ({:.1},{:.1}) edge={:?} dir={}",
                        cur, origin, pos.x, pos.y, edge, if is_fwd {"fwd"} else {"bwd"});
                    cur = he.next;
                    step += 1;
                    if cur == start || step > 60 { break; }
                }
            }
        }

        // Check what face each point lands on
        let paint_points = vec![
            Point::new(219.8, 233.7),
            Point::new(227.2, 205.8),
            Point::new(253.2, 203.3),
            Point::new(281.2, 149.0),
        ];
        for (i, &pt) in paint_points.iter().enumerate() {
            let face = dcel.find_face_containing_point(pt);
            if face.0 != 0 {
                let cycle = dcel.face_boundary(face);
                let stripped = dcel.strip_cycle(&cycle);
                let path_raw = dcel.face_to_bezpath(face);
                let path_stripped = dcel.face_to_bezpath_stripped(face);
                let area_raw = kurbo::Shape::area(&path_raw).abs();
                let area_stripped = kurbo::Shape::area(&path_stripped).abs();
                eprintln!("paint point {i} at ({:.1}, {:.1}) → face {:?} raw_len={} stripped_len={} raw_area={:.1} stripped_area={:.1}",
                    pt.x, pt.y, face, cycle.len(), stripped.len(), area_raw, area_stripped);
                if i == 2 {
                    eprintln!("  Raw cycle vertices for face {:?}:", face);
                    for (j, &he_id) in cycle.iter().enumerate() {
                        let src = dcel.half_edge_source(he_id);
                        let pos = dcel.vertex(src).position;
                        eprintln!("    step {j}: HE{} src=V{} ({:.1},{:.1})", he_id.0, src.0, pos.x, pos.y);
                    }
                    eprintln!("  Stripped cycle vertices for face {:?}:", face);
                    for (j, &he_id) in stripped.iter().enumerate() {
                        let src = dcel.half_edge_source(he_id);
                        let pos = dcel.vertex(src).position;
                        eprintln!("    step {j}: HE{} src=V{} ({:.1},{:.1})", he_id.0, src.0, pos.x, pos.y);
                    }
                }
            } else {
                eprintln!("paint point {i} at ({:.1}, {:.1}) → UNBOUNDED", pt.x, pt.y);
            }
        }

        assert_paint_sequence(&mut dcel, &paint_points, 600, 400);
    }

    #[test]
    fn test_dump_svg() {
        let mut dcel = Dcel::new();

        // Same 8 strokes as test_recorded_eight_strokes
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(205.0, 366.2), Point::new(244.7, 255.0), Point::new(301.5, 184.3), Point::new(398.7, 119.5)),
            CubicBez::new(Point::new(398.7, 119.5), Point::new(419.4, 105.7), Point::new(438.3, 87.0), Point::new(464.6, 87.0)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(131.7, 126.8), Point::new(278.6, 184.4), Point::new(420.9, 260.3), Point::new(570.1, 310.0)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(252.7, 369.6), Point::new(245.6, 297.8), Point::new(246.6, 225.3), Point::new(240.6, 153.5)),
            CubicBez::new(Point::new(240.6, 153.5), Point::new(238.9, 132.9), Point::new(228.3, 112.7), Point::new(228.3, 92.0)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(362.6, 105.6), Point::new(317.6, 210.5), Point::new(160.1, 315.5), Point::new(149.0, 332.1)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(134.6, 218.2), Point::new(228.4, 208.3), Point::new(368.1, 233.7), Point::new(458.8, 263.9)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(329.0, 300.6), Point::new(339.5, 221.5), Point::new(342.3, 147.5), Point::new(316.7, 70.4)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(186.0, 99.2), Point::new(263.5, 118.6), Point::new(342.2, 129.8), Point::new(417.9, 156.3)),
            CubicBez::new(Point::new(417.9, 156.3), Point::new(456.4, 169.8), Point::new(494.6, 191.3), Point::new(533.9, 201.1)),
        ], None, None, 5.0);
        dcel.insert_stroke(&[
            CubicBez::new(Point::new(287.5, 73.5), Point::new(266.9, 135.2), Point::new(224.9, 188.7), Point::new(202.3, 251.0)),
            CubicBez::new(Point::new(202.3, 251.0), Point::new(187.7, 291.0), Point::new(194.5, 335.7), Point::new(181.2, 375.8)),
        ], None, None, 5.0);

        // Generate distinct colors via HSL hue rotation
        fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
            let m = l - c / 2.0;
            let (r1, g1, b1) = if h < 60.0 { (c, x, 0.0) }
                else if h < 120.0 { (x, c, 0.0) }
                else if h < 180.0 { (0.0, c, x) }
                else if h < 240.0 { (0.0, x, c) }
                else if h < 300.0 { (x, 0.0, c) }
                else { (c, 0.0, x) };
            (((r1 + m) * 255.0) as u8, ((g1 + m) * 255.0) as u8, ((b1 + m) * 255.0) as u8)
        }

        let mut svg = String::new();
        svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"100 40 500 380\" width=\"1200\" height=\"912\">\n");
        svg.push_str("<rect x=\"100\" y=\"40\" width=\"500\" height=\"380\" fill=\"white\"/>\n");
        svg.push_str("<defs><marker id=\"ah\" markerWidth=\"1.6\" markerHeight=\"1.2\" refX=\"1.6\" refY=\"0.6\" orient=\"auto\"><path d=\"M0,0 L1.6,0.6 L0,1.2\" fill=\"context-stroke\"/></marker></defs>\n");

        // Draw each half-edge as a colored arrow
        let n_he = dcel.half_edges.len();
        for (i, he) in dcel.half_edges.iter().enumerate() {
            if he.deleted { continue; }
            let he_id = HalfEdgeId(i as u32);
            let edge = &dcel.edges[he.edge.idx()];
            let is_fwd = edge.half_edges[0] == he_id;

            let curve = if is_fwd {
                edge.curve
            } else {
                CubicBez::new(edge.curve.p3, edge.curve.p2, edge.curve.p1, edge.curve.p0)
            };

            // Color based on half-edge index
            let hue = (i as f64 / n_he as f64) * 360.0;
            let (r, g, b) = hsl_to_rgb(hue, 0.9, 0.4);

            // Offset slightly so fwd/bwd don't overlap perfectly
            let offset = if is_fwd { -1.5 } else { 1.5 };
            // Simple normal offset: perpendicular to start→end direction
            let dx = curve.p3.x - curve.p0.x;
            let dy = curve.p3.y - curve.p0.y;
            let len = (dx * dx + dy * dy).sqrt().max(0.01);
            let nx = -dy / len * offset;
            let ny = dx / len * offset;

            svg.push_str(&format!(
                "<path d=\"M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" \
                 fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-width=\"1.5\" \
                 marker-end=\"url(#ah)\" opacity=\"0.8\">\
                 <title>HE{i} E{} F{} {}</title></path>\n",
                curve.p0.x + nx, curve.p0.y + ny,
                curve.p1.x + nx, curve.p1.y + ny,
                curve.p2.x + nx, curve.p2.y + ny,
                curve.p3.x + nx, curve.p3.y + ny,
                he.edge.0, he.face.0, if is_fwd { "fwd" } else { "bwd" },
            ));

            // Label near destination (t=0.85) so fwd/bwd labels don't overlap
            let label_pt = curve.eval(0.85);
            svg.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"1.4\" fill=\"rgb({r},{g},{b})\" \
                 text-anchor=\"middle\" dominant-baseline=\"middle\" opacity=\"0.9\">HE{i}</text>\n",
                label_pt.x + nx * 3.0, label_pt.y + ny * 3.0,
            ));
        }

        // Draw vertices as labeled circles
        for (i, v) in dcel.vertices.iter().enumerate() {
            if v.deleted { continue; }
            svg.push_str(&format!(
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"0.6\" fill=\"black\"/>\n\
                 <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"1.8\" font-weight=\"bold\" \
                 fill=\"black\" text-anchor=\"start\" dominant-baseline=\"hanging\">V{i}</text>\n",
                v.position.x, v.position.y,
                v.position.x + 1.0, v.position.y + 0.2,
            ));
        }

        // Mark paint points
        let paint_points = [
            (219.8, 233.7, "P0"),
            (227.2, 205.8, "P1"),
            (253.2, 203.3, "P2"),
            (281.2, 149.0, "P3"),
        ];
        for (x, y, label) in &paint_points {
            let is_p2 = *label == "P2";
            let color = if is_p2 { "magenta" } else { "red" };
            let r = if is_p2 { "1.4" } else { "1.0" };
            let sw = if is_p2 { "0.6" } else { "0.4" };
            let extra = if is_p2 { " (BLOATED)" } else { "" };
            svg.push_str(&format!(
                "<circle cx=\"{x}\" cy=\"{y}\" r=\"{r}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{sw}\"/>\n\
                 <text x=\"{}\" y=\"{}\" font-size=\"2\" font-weight=\"bold\" fill=\"{color}\">{label}{extra}</text>\n",
                x + 1.5, y - 0.4,
            ));
        }

        // Highlight Face 15 stripped cycle
        let face15 = FaceId(15);
        if !dcel.faces[15].deleted && !dcel.faces[15].outer_half_edge.is_none() {
            let cycle = dcel.face_boundary(face15);
            let stripped = dcel.strip_cycle(&cycle);
            let mut d = String::new();
            for (j, &he_id) in stripped.iter().enumerate() {
                let edge = &dcel.edges[dcel.half_edge(he_id).edge.idx()];
                let is_fwd = edge.half_edges[0] == he_id;
                let curve = if is_fwd {
                    edge.curve
                } else {
                    CubicBez::new(edge.curve.p3, edge.curve.p2, edge.curve.p1, edge.curve.p0)
                };
                if j == 0 {
                    d.push_str(&format!("M{:.1},{:.1} ", curve.p0.x, curve.p0.y));
                }
                d.push_str(&format!("C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} ",
                    curve.p1.x, curve.p1.y, curve.p2.x, curve.p2.y, curve.p3.x, curve.p3.y));
            }
            d.push_str("Z");
            svg.push_str(&format!(
                "<path d=\"{d}\" fill=\"rgba(255,0,0,0.12)\" stroke=\"red\" stroke-width=\"2\" stroke-dasharray=\"6,3\">\
                 <title>Face 15 stripped cycle ({} edges)</title></path>\n",
                stripped.len(),
            ));
        }

        svg.push_str("</svg>\n");

        std::fs::write("/tmp/dcel_debug.svg", &svg).expect("write SVG");
        eprintln!("Wrote /tmp/dcel_debug.svg");

        // --- Zoomed SVG around P2, V7/V37, V3/V11 ---
        let mut svg2 = String::new();
        // V38=(241.8,169.1) V7=(246.8,274.7) — center ~(244, 222), span ~130
        svg2.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"215 155 65 135\" width=\"975\" height=\"2025\">\n");
        svg2.push_str("<rect x=\"215\" y=\"155\" width=\"65\" height=\"135\" fill=\"white\"/>\n");
        svg2.push_str("<defs><marker id=\"ah\" markerWidth=\"1.6\" markerHeight=\"1.2\" refX=\"1.6\" refY=\"0.6\" orient=\"auto\"><path d=\"M0,0 L1.6,0.6 L0,1.2\" fill=\"context-stroke\"/></marker></defs>\n");

        // Draw all half-edges (clipped by viewBox naturally)
        for (i, he) in dcel.half_edges.iter().enumerate() {
            if he.deleted { continue; }
            let he_id = HalfEdgeId(i as u32);
            let edge = &dcel.edges[he.edge.idx()];
            let is_fwd = edge.half_edges[0] == he_id;
            let curve = if is_fwd {
                edge.curve
            } else {
                CubicBez::new(edge.curve.p3, edge.curve.p2, edge.curve.p1, edge.curve.p0)
            };
            let hue = (i as f64 / n_he as f64) * 360.0;
            let (r, g, b) = hsl_to_rgb(hue, 0.9, 0.4);
            let offset = if is_fwd { -0.8 } else { 0.8 };
            let dx = curve.p3.x - curve.p0.x;
            let dy = curve.p3.y - curve.p0.y;
            let len = (dx * dx + dy * dy).sqrt().max(0.01);
            let nx = -dy / len * offset;
            let ny = dx / len * offset;

            svg2.push_str(&format!(
                "<path d=\"M{:.1},{:.1} C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1}\" \
                 fill=\"none\" stroke=\"rgb({r},{g},{b})\" stroke-width=\"0.4\" \
                 marker-end=\"url(#ah)\" opacity=\"0.8\">\
                 <title>HE{i} E{} F{} {}</title></path>\n",
                curve.p0.x + nx, curve.p0.y + ny,
                curve.p1.x + nx, curve.p1.y + ny,
                curve.p2.x + nx, curve.p2.y + ny,
                curve.p3.x + nx, curve.p3.y + ny,
                he.edge.0, he.face.0, if is_fwd { "fwd" } else { "bwd" },
            ));

            let label_pt = curve.eval(0.85);
            svg2.push_str(&format!(
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"1.2\" fill=\"rgb({r},{g},{b})\" \
                 text-anchor=\"middle\" dominant-baseline=\"middle\" opacity=\"0.9\">HE{i}</text>\n",
                label_pt.x + nx * 2.0, label_pt.y + ny * 2.0,
            ));
        }

        // Vertices
        for (i, v) in dcel.vertices.iter().enumerate() {
            if v.deleted { continue; }
            // Highlight V38,V7 specially
            let special = matches!(i, 38 | 7);
            let (fill, rad, fs) = if special {
                ("blue", "0.8", "1.6")
            } else {
                ("black", "0.4", "1.2")
            };
            svg2.push_str(&format!(
                "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"{rad}\" fill=\"{fill}\"/>\n\
                 <text x=\"{:.1}\" y=\"{:.1}\" font-size=\"{fs}\" font-weight=\"bold\" \
                 fill=\"{fill}\" text-anchor=\"start\" dominant-baseline=\"hanging\">V{i}</text>\n",
                v.position.x, v.position.y,
                v.position.x + 1.0, v.position.y + 0.2,
            ));
        }

        // Paint points
        for (x, y, label) in &paint_points {
            let is_p2 = *label == "P2";
            let color = if is_p2 { "magenta" } else { "red" };
            let r = if is_p2 { "1.4" } else { "1.0" };
            let sw = if is_p2 { "0.3" } else { "0.2" };
            svg2.push_str(&format!(
                "<circle cx=\"{x}\" cy=\"{y}\" r=\"{r}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"{sw}\"/>\n\
                 <text x=\"{}\" y=\"{}\" font-size=\"1.6\" font-weight=\"bold\" fill=\"{color}\">{label}</text>\n",
                x + 1.5, y - 0.4,
            ));
        }

        // Face 15 stripped outline
        if !dcel.faces[15].deleted && !dcel.faces[15].outer_half_edge.is_none() {
            let cycle = dcel.face_boundary(face15);
            let stripped = dcel.strip_cycle(&cycle);
            let mut d = String::new();
            for (j, &he_id) in stripped.iter().enumerate() {
                let edge = &dcel.edges[dcel.half_edge(he_id).edge.idx()];
                let is_fwd = edge.half_edges[0] == he_id;
                let curve = if is_fwd {
                    edge.curve
                } else {
                    CubicBez::new(edge.curve.p3, edge.curve.p2, edge.curve.p1, edge.curve.p0)
                };
                if j == 0 {
                    d.push_str(&format!("M{:.1},{:.1} ", curve.p0.x, curve.p0.y));
                }
                d.push_str(&format!("C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} ",
                    curve.p1.x, curve.p1.y, curve.p2.x, curve.p2.y, curve.p3.x, curve.p3.y));
            }
            d.push_str("Z");
            svg2.push_str(&format!(
                "<path d=\"{d}\" fill=\"rgba(255,0,0,0.08)\" stroke=\"red\" stroke-width=\"0.4\" stroke-dasharray=\"1,0.5\">\
                 <title>Face 15 stripped</title></path>\n",
            ));
        }

        // Also draw ALL non-exterior face boundaries so we can see the inner cycle
        for (fi, face) in dcel.faces.iter().enumerate() {
            if face.deleted || fi == 0 || face.outer_half_edge.is_none() { continue; }
            if fi == 15 { continue; } // already drawn
            let fid = FaceId(fi as u32);
            let cycle = dcel.face_boundary(fid);
            let stripped = dcel.strip_cycle(&cycle);
            if stripped.is_empty() { continue; }
            // Check if this face's stripped path contains P2
            let fp = dcel.cycle_to_bezpath_stripped(&cycle);
            let w = kurbo::Shape::winding(&fp, Point::new(253.2, 203.3));
            if w == 0 { continue; } // Only draw faces that contain P2
            let a = kurbo::Shape::area(&fp).abs();
            let mut d = String::new();
            for (j, &he_id) in stripped.iter().enumerate() {
                let edge = &dcel.edges[dcel.half_edge(he_id).edge.idx()];
                let is_fwd = edge.half_edges[0] == he_id;
                let curve = if is_fwd {
                    edge.curve
                } else {
                    CubicBez::new(edge.curve.p3, edge.curve.p2, edge.curve.p1, edge.curve.p0)
                };
                if j == 0 {
                    d.push_str(&format!("M{:.1},{:.1} ", curve.p0.x, curve.p0.y));
                }
                d.push_str(&format!("C{:.1},{:.1} {:.1},{:.1} {:.1},{:.1} ",
                    curve.p1.x, curve.p1.y, curve.p2.x, curve.p2.y, curve.p3.x, curve.p3.y));
            }
            d.push_str("Z");
            svg2.push_str(&format!(
                "<path d=\"{d}\" fill=\"rgba(0,128,255,0.1)\" stroke=\"blue\" stroke-width=\"0.3\" stroke-dasharray=\"1,0.5\">\
                 <title>Face {fi} (area={a:.0}, {}-edge stripped)</title></path>\n",
                stripped.len(),
            ));
        }

        svg2.push_str("</svg>\n");
        std::fs::write("/tmp/dcel_zoom.svg", &svg2).expect("write zoomed SVG");
        eprintln!("Wrote /tmp/dcel_zoom.svg");
    }

    /// Minimal test to isolate bloated face bug from eight_strokes test.
    #[test]
    fn test_bloated_face_minimal() {
        fn strip_spurs_len(dcel: &Dcel, cycle: &[HalfEdgeId]) -> usize {
            let mut stripped: Vec<HalfEdgeId> = Vec::new();
            for &he_id in cycle {
                let edge = dcel.half_edge(he_id).edge;
                if let Some(&top) = stripped.last() {
                    if dcel.half_edge(top).edge == edge { stripped.pop(); continue; }
                }
                stripped.push(he_id);
            }
            while stripped.len() >= 2 {
                let fe = dcel.half_edge(stripped[0]).edge;
                let le = dcel.half_edge(*stripped.last().unwrap()).edge;
                if fe == le { stripped.pop(); stripped.remove(0); } else { break; }
            }
            if stripped.is_empty() { cycle.len() } else { stripped.len() }
        }
        fn max_stripped_cycle(dcel: &Dcel) -> (usize, usize) {
            let mut worst = (0usize, 0usize);
            for (i, face) in dcel.faces.iter().enumerate() {
                if face.deleted || i == 0 || face.outer_half_edge.is_none() { continue; }
                let cycle = dcel.face_boundary(FaceId(i as u32));
                let n = strip_spurs_len(dcel, &cycle);
                if n > worst.1 { worst = (i, n); }
            }
            worst
        }

        // Strokes 0-3 from seven_strokes test (stroke 0 simplified to first seg only)
        // Eight strokes test data — reduce to find minimal reproduction
        let strokes: Vec<Vec<CubicBez>> = vec![
            vec![ // 0
                CubicBez::new(Point::new(205.0, 366.2), Point::new(244.7, 255.0), Point::new(301.5, 184.3), Point::new(398.7, 119.5)),
                CubicBez::new(Point::new(398.7, 119.5), Point::new(419.4, 105.7), Point::new(438.3, 87.0), Point::new(464.6, 87.0)),
            ],
            vec![CubicBez::new(Point::new(131.7, 126.8), Point::new(278.6, 184.4), Point::new(420.9, 260.3), Point::new(570.1, 310.0))], // 1
            vec![ // 2
                CubicBez::new(Point::new(252.7, 369.6), Point::new(245.6, 297.8), Point::new(246.6, 225.3), Point::new(240.6, 153.5)),
                CubicBez::new(Point::new(240.6, 153.5), Point::new(238.9, 132.9), Point::new(228.3, 112.7), Point::new(228.3, 92.0)),
            ],
            vec![CubicBez::new(Point::new(362.6, 105.6), Point::new(317.6, 210.5), Point::new(160.1, 315.5), Point::new(149.0, 332.1))], // 3
            vec![CubicBez::new(Point::new(134.6, 218.2), Point::new(228.4, 208.3), Point::new(368.1, 233.7), Point::new(458.8, 263.9))], // 4
            vec![CubicBez::new(Point::new(329.0, 300.6), Point::new(339.5, 221.5), Point::new(342.3, 147.5), Point::new(316.7, 70.4))], // 5
            vec![ // 6
                CubicBez::new(Point::new(186.0, 99.2), Point::new(263.5, 118.6), Point::new(342.2, 129.8), Point::new(417.9, 156.3)),
                CubicBez::new(Point::new(417.9, 156.3), Point::new(456.4, 169.8), Point::new(494.6, 191.3), Point::new(533.9, 201.1)),
            ],
            vec![ // 7
                CubicBez::new(Point::new(287.5, 73.5), Point::new(266.9, 135.2), Point::new(224.9, 188.7), Point::new(202.3, 251.0)),
                CubicBez::new(Point::new(202.3, 251.0), Point::new(187.7, 291.0), Point::new(194.5, 335.7), Point::new(181.2, 375.8)),
            ],
        ];

        // Per-stroke tracking (disabled to avoid DCEL_TRACE noise)
        // let mut dcel = Dcel::new();
        // for (i, s) in strokes.iter().enumerate() {
        //     dcel.insert_stroke(s, None, None, 5.0);
        //     let (f, c) = max_stripped_cycle(&dcel);
        //     eprintln!("After stroke {i}: worst stripped Face {f} cycle={c}");
        // }

        // Focus: strokes 0-3 create a face. Stroke 4 should split it but grows it.
        fn dump_all_faces(d: &Dcel, label: &str) {
            eprintln!("\n  {label}:");
            for (i, face) in d.faces.iter().enumerate() {
                if face.deleted || i == 0 || face.outer_half_edge.is_none() { continue; }
                let cycle = d.face_boundary(FaceId(i as u32));
                let stripped_n = strip_spurs_len(d, &cycle);
                eprintln!("    Face {i}: raw={} stripped={stripped_n}", cycle.len());
                // Show stripped half-edges
                let mut stripped: Vec<HalfEdgeId> = Vec::new();
                for &he_id in &cycle {
                    let edge = d.half_edge(he_id).edge;
                    if let Some(&top) = stripped.last() {
                        if d.half_edge(top).edge == edge { stripped.pop(); continue; }
                    }
                    stripped.push(he_id);
                }
                while stripped.len() >= 2 {
                    let fe = d.half_edge(stripped[0]).edge;
                    let le = d.half_edge(*stripped.last().unwrap()).edge;
                    if fe == le { stripped.pop(); stripped.remove(0); } else { break; }
                }
                for (s, &he_id) in stripped.iter().enumerate() {
                    let he = d.half_edge(he_id);
                    let pos = d.vertices[he.origin.idx()].position;
                    let edge_data = d.edge(he.edge);
                    let dir = if edge_data.half_edges[0] == he_id { "fwd" } else { "bwd" };
                    let dest_he = d.half_edge(he.twin);
                    let dest_pos = d.vertices[dest_he.origin.idx()].position;
                    eprintln!("      [{s}] HE{} V{}({:.1},{:.1})->V{}({:.1},{:.1}) E{} {dir}",
                        he_id.0, he.origin.0, pos.x, pos.y,
                        dest_he.origin.0, dest_pos.x, dest_pos.y,
                        he.edge.0, );
                }
            }
        }

        let mut d = Dcel::new();
        for i in 0..3 {
            d.insert_stroke(&strokes[i], None, None, 5.0);
        }
        dump_all_faces(&d, "After strokes 0-2");

        let result3 = d.insert_stroke(&strokes[3], None, None, 5.0);
        eprintln!("\nStroke 3 result: splits={} new_faces={:?} new_verts={:?}",
            result3.split_edges.len(), result3.new_faces, result3.new_vertices);
        dump_all_faces(&d, "After stroke 3");

        // Dump vertex fans at key vertices before stroke 4
        fn dump_vertex_fan(d: &Dcel, v: VertexId, label: &str) {
            let pos = d.vertices[v.idx()].position;
            eprintln!("  Vertex fan at V{}({:.1},{:.1}) {label}:", v.0, pos.x, pos.y);
            let start = d.vertices[v.idx()].outgoing;
            if start.is_none() { eprintln!("    (no edges)"); return; }
            let mut cur = start;
            loop {
                let he = d.half_edge(cur);
                let dest = d.half_edge(he.twin).origin;
                let dest_pos = d.vertices[dest.idx()].position;
                let angle = d.outgoing_angle(cur);
                let face = he.face;
                let edge = he.edge;
                let edge_data = d.edge(edge);
                let dir = if edge_data.half_edges[0] == cur { "fwd" } else { "bwd" };
                eprintln!("    HE{} → V{}({:.1},{:.1}) E{} {} angle={:.3} face=F{}",
                    cur.0, dest.0, dest_pos.x, dest_pos.y, edge.0, dir, angle, face.0);
                let twin = he.twin;
                cur = d.half_edge(twin).next;
                if cur == start { break; }
            }
        }

        // Before stroke 4, dump the fan at key vertices
        eprintln!("\n--- Before stroke 4 ---");
        // Find all non-isolated vertices
        for vi in 0..d.vertices.len() {
            if d.vertices[vi].outgoing.is_none() { continue; }
            dump_vertex_fan(&d, VertexId(vi as u32), "before stroke 4");
        }

        let result4 = d.insert_stroke(&strokes[4], None, None, 5.0);
        eprintln!("\nStroke 4 result: splits={} new_faces={:?} new_verts={:?}",
            result4.split_edges.len(), result4.new_faces, result4.new_vertices);

        // After stroke 4, dump fans at vertices on the problematic face
        eprintln!("\n--- After stroke 4 ---");
        for vi in 0..d.vertices.len() {
            if d.vertices[vi].outgoing.is_none() { continue; }
            dump_vertex_fan(&d, VertexId(vi as u32), "after stroke 4");
        }

        dump_all_faces(&d, "After stroke 4");
    }
}
