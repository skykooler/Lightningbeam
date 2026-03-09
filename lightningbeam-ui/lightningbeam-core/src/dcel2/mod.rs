//! Doubly-Connected Edge List (DCEL) for planar subdivision vector drawing.
//!
//! Each vector layer keyframe stores a DCEL representing a Flash-style planar
//! subdivision. Strokes live on edges, fills live on faces, and the topology is
//! maintained such that wherever two strokes intersect there is a vertex.
//!
//! Half-edges leaving a vertex are maintained in sorted CCW order. This enables
//! efficient face detection by ray-casting to the nearest edge and walking CCW.

pub mod topology;
pub mod query;
pub mod stroke;
pub mod region;
pub mod import;

pub use import::extract_faces_for_edges;

use crate::shape::{FillRule, ShapeColor, StrokeStyle};
use kurbo::{CubicBez, Point};
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
    pub position: Point,
    /// One outgoing half-edge (any one; iteration via twin.next gives the CCW fan).
    /// NONE if the vertex is isolated (no edges).
    pub outgoing: HalfEdgeId,
    #[serde(default)]
    pub deleted: bool,
}

/// A half-edge in the DCEL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HalfEdge {
    pub origin: VertexId,
    pub twin: HalfEdgeId,
    /// Next half-edge around the face (CCW).
    pub next: HalfEdgeId,
    /// Previous half-edge around the face (CCW).
    pub prev: HalfEdgeId,
    /// Face to the left of this half-edge.
    pub face: FaceId,
    /// Parent edge (shared between this half-edge and its twin).
    pub edge: EdgeId,
    #[serde(default)]
    pub deleted: bool,
}

/// Geometric and style data for an edge (shared by the two half-edges).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EdgeData {
    /// The two half-edges: [forward, backward].
    /// Forward goes from curve.p0 to curve.p3.
    pub half_edges: [HalfEdgeId; 2],
    pub curve: CubicBez,
    pub stroke_style: Option<StrokeStyle>,
    pub stroke_color: Option<ShapeColor>,
    #[serde(default)]
    pub deleted: bool,
}

/// A face (region) in the DCEL.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Face {
    /// One half-edge on the outer boundary. NONE for the unbounded face (face 0).
    pub outer_half_edge: HalfEdgeId,
    /// Half-edges on inner boundary cycles (holes).
    pub inner_half_edges: Vec<HalfEdgeId>,
    pub fill_color: Option<ShapeColor>,
    pub image_fill: Option<uuid::Uuid>,
    pub fill_rule: FillRule,
    #[serde(default)]
    pub deleted: bool,
}

// ---------------------------------------------------------------------------
// Spatial index for vertex snapping
// ---------------------------------------------------------------------------

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
// Debug recorder
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct DebugRecorder {
    pub strokes: Vec<Vec<CubicBez>>,
    pub paint_points: Vec<Point>,
}

impl DebugRecorder {
    pub fn record_stroke(&mut self, segments: &[CubicBez]) {
        self.strokes.push(segments.to_vec());
    }

    pub fn record_paint(&mut self, point: Point) {
        self.paint_points.push(point);
    }

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
        for (i, pt) in self.paint_points.iter().enumerate() {
            eprintln!("        // Paint {i}");
            eprintln!(
                "        let _f{i} = dcel.find_face_at_point(Point::new({:.1}, {:.1}));",
                pt.x, pt.y
            );
        }
        eprintln!("    }}");
    }

    pub fn dump_and_reset(&mut self, name: &str) {
        self.dump_test(name);
        self.strokes.clear();
        self.paint_points.clear();
    }
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default snap epsilon in document coordinate units.
pub const DEFAULT_SNAP_EPSILON: f64 = 0.5;

// ---------------------------------------------------------------------------
// DCEL container
// ---------------------------------------------------------------------------

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

    #[serde(skip)]
    vertex_rtree: Option<RTree<VertexEntry>>,

    #[serde(skip)]
    pub debug_recorder: Option<DebugRecorder>,
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

    // -----------------------------------------------------------------------
    // Debug recording
    // -----------------------------------------------------------------------

    pub fn set_recording(&mut self, enabled: bool) {
        if enabled {
            self.debug_recorder.get_or_insert_with(DebugRecorder::default);
        } else {
            self.debug_recorder = None;
        }
    }

    pub fn is_recording(&self) -> bool {
        self.debug_recorder.is_some()
    }

    pub fn dump_recorded_test(&mut self, name: &str) {
        if let Some(ref mut rec) = self.debug_recorder {
            rec.dump_and_reset(name);
        }
    }

    pub fn record_paint_point(&mut self, point: Point) {
        if let Some(ref mut rec) = self.debug_recorder {
            rec.record_paint(point);
        }
    }

    // -----------------------------------------------------------------------
    // Allocation
    // -----------------------------------------------------------------------

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
        self.vertex_rtree = None;
        id
    }

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
        self.half_edges[a.idx()].twin = b;
        self.half_edges[b.idx()].twin = a;
        (a, b)
    }

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

    /// Destination vertex of a half-edge (origin of its twin).
    #[inline]
    pub fn half_edge_dest(&self, he: HalfEdgeId) -> VertexId {
        let twin = self.half_edge(he).twin;
        self.half_edge(twin).origin
    }
}

// ---------------------------------------------------------------------------
// Bezier utilities
// ---------------------------------------------------------------------------

/// Split a cubic bezier at parameter t using de Casteljau's algorithm.
pub fn subdivide_cubic(c: CubicBez, t: f64) -> (CubicBez, CubicBez) {
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

/// Extract subsegment of a cubic bezier for parameter range [t0, t1].
pub fn subsegment_cubic(c: CubicBez, t0: f64, t1: f64) -> CubicBez {
    if (t0).abs() < 1e-10 && (t1 - 1.0).abs() < 1e-10 {
        return c;
    }
    if (t0).abs() < 1e-10 {
        subdivide_cubic(c, t1).0
    } else if (t1 - 1.0).abs() < 1e-10 {
        subdivide_cubic(c, t0).1
    } else {
        let (_, upper) = subdivide_cubic(c, t0);
        let remapped_t1 = (t1 - t0) / (1.0 - t0);
        subdivide_cubic(upper, remapped_t1).0
    }
}

#[inline]
pub fn lerp_point(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

/// Convert a `BezPath` into a list of sub-paths, each a `Vec<CubicBez>`.
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
