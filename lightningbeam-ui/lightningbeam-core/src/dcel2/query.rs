//! Queries, iteration, and BezPath construction for the DCEL.

use super::{Dcel, EdgeId, FaceId, HalfEdgeId, VertexEntry, VertexId};
use kurbo::{BezPath, ParamCurve, ParamCurveNearest, PathEl, Point};
use rstar::{PointDistance, RTree};
use std::collections::HashSet;

/// Result of a face-at-point query.
pub struct FaceQuery {
    /// The face currently assigned to the cycle (may be F0 if no face was created).
    pub face: FaceId,
    /// A half-edge on the enclosing cycle. Walk via `next` to traverse.
    pub cycle_he: HalfEdgeId,
}

impl Dcel {
    // -------------------------------------------------------------------
    // Iteration
    // -------------------------------------------------------------------

    /// Walk the half-edge cycle starting at `start`, returning all half-edges.
    pub fn walk_cycle(&self, start: HalfEdgeId) -> Vec<HalfEdgeId> {
        let mut result = vec![start];
        let mut cur = self.half_edges[start.idx()].next;
        let mut steps = 0;
        while cur != start {
            result.push(cur);
            cur = self.half_edges[cur.idx()].next;
            steps += 1;
            debug_assert!(steps < 100_000, "infinite cycle in walk_cycle");
        }
        result
    }

    /// Compute the signed area of the cycle starting at `start`.
    /// Positive = CCW (interior), negative = CW (exterior).
    pub fn cycle_signed_area(&self, start: HalfEdgeId) -> f64 {
        let mut area = 0.0;
        let mut cur = start;
        loop {
            let p0 = self.vertices[self.half_edges[cur.idx()].origin.idx()].position;
            cur = self.half_edges[cur.idx()].next;
            let p1 = self.vertices[self.half_edges[cur.idx()].origin.idx()].position;
            area += p0.x * p1.y - p1.x * p0.y;
            if cur == start {
                break;
            }
        }
        area * 0.5
    }

    /// Get all half-edges on a face's outer boundary.
    pub fn face_boundary(&self, face_id: FaceId) -> Vec<HalfEdgeId> {
        let ohe = self.faces[face_id.idx()].outer_half_edge;
        if ohe.is_none() {
            return Vec::new();
        }
        self.walk_cycle(ohe)
    }

    /// Get all outgoing half-edges from a vertex in CCW order.
    pub fn vertex_outgoing(&self, vertex_id: VertexId) -> Vec<HalfEdgeId> {
        let start = self.vertices[vertex_id.idx()].outgoing;
        if start.is_none() {
            return Vec::new();
        }
        let mut result = vec![start];
        let twin = self.half_edges[start.idx()].twin;
        let mut cur = self.half_edges[twin.idx()].next;
        let mut steps = 0;
        while cur != start {
            result.push(cur);
            let twin = self.half_edges[cur.idx()].twin;
            cur = self.half_edges[twin.idx()].next;
            steps += 1;
            debug_assert!(steps < 100_000, "infinite fan in vertex_outgoing");
        }
        result
    }

    // -------------------------------------------------------------------
    // Face detection
    // -------------------------------------------------------------------

    /// Find the enclosing face/cycle for a point.
    ///
    /// Algorithm:
    /// 1. Find the nearest edge to `point`
    /// 2. Pick the half-edge with `point` on its left side (cross product of tangent)
    /// 3. Walk that half-edge's cycle — this is the innermost boundary enclosing `point`
    /// 4. Return the cycle's face (which may be F0 if no face has been created yet)
    ///    along with a half-edge on the cycle so the caller can create a face if needed
    ///
    /// Returns F0 with NONE cycle_he if there are no edges.
    pub fn find_face_at_point(&self, point: Point) -> FaceQuery {
        let mut best: Option<(EdgeId, f64, f64)> = None;

        for (i, edge) in self.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let nearest = edge.curve.nearest(point, 0.5);
            if best.is_none() || nearest.distance_sq < best.unwrap().1 {
                best = Some((EdgeId(i as u32), nearest.distance_sq, nearest.t));
            }
        }

        let Some((edge_id, _, t)) = best else {
            return FaceQuery {
                face: FaceId(0),
                cycle_he: HalfEdgeId::NONE,
            };
        };

        let edge = &self.edges[edge_id.idx()];

        // Tangent via finite difference (clamped to valid range)
        let t_lo = (t - 0.001).max(0.0);
        let t_hi = (t + 0.001).min(1.0);
        let p_lo = edge.curve.eval(t_lo);
        let p_hi = edge.curve.eval(t_hi);
        let tan_x = p_hi.x - p_lo.x;
        let tan_y = p_hi.y - p_lo.y;

        let curve_pt = edge.curve.eval(t);
        let to_pt_x = point.x - curve_pt.x;
        let to_pt_y = point.y - curve_pt.y;
        let cross = tan_x * to_pt_y - tan_y * to_pt_x;

        // cross > 0: point is to the left of the forward half-edge
        let he = if cross >= 0.0 {
            edge.half_edges[0]
        } else {
            edge.half_edges[1]
        };

        // Walk the cycle to find the actual face
        let face = self.half_edges[he.idx()].face;

        FaceQuery {
            face,
            cycle_he: he,
        }
    }

    /// Convenience: just return the FaceId (backward-compatible).
    pub fn find_face_containing_point(&self, point: Point) -> FaceId {
        self.find_face_at_point(point).face
    }

    // -------------------------------------------------------------------
    // Spatial index (vertex snapping)
    // -------------------------------------------------------------------

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

    pub fn ensure_spatial_index(&mut self) {
        if self.vertex_rtree.is_none() {
            self.rebuild_spatial_index();
        }
    }

    pub fn snap_vertex(&mut self, point: Point, epsilon: f64) -> Option<VertexId> {
        self.ensure_spatial_index();
        let tree = self.vertex_rtree.as_ref().unwrap();
        let query = [point.x, point.y];
        let nearest = tree.nearest_neighbor(&query)?;
        let dist_sq = nearest.distance_2(&query);
        if dist_sq <= epsilon * epsilon {
            Some(nearest.id)
        } else {
            None
        }
    }

    // -------------------------------------------------------------------
    // BezPath construction for rendering
    // -------------------------------------------------------------------

    /// Raw bezpath from a face's outer boundary cycle.
    pub fn face_to_bezpath(&self, face_id: FaceId) -> BezPath {
        let cycle = self.face_boundary(face_id);
        self.cycle_to_bezpath(&cycle)
    }

    /// Build a BezPath from a cycle of half-edges.
    pub fn cycle_to_bezpath(&self, cycle: &[HalfEdgeId]) -> BezPath {
        let mut path = BezPath::new();
        if cycle.is_empty() {
            return path;
        }

        let first_he = &self.half_edges[cycle[0].idx()];
        let first_pos = self.vertices[first_he.origin.idx()].position;
        path.move_to(first_pos);

        for &he_id in cycle {
            let he = &self.half_edges[he_id.idx()];
            let edge = &self.edges[he.edge.idx()];
            if he_id == edge.half_edges[0] {
                path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
            } else {
                path.curve_to(edge.curve.p2, edge.curve.p1, edge.curve.p0);
            }
        }
        path.close_path();
        path
    }

    /// Bezpath with spur edges stripped (for fill rendering).
    pub fn face_to_bezpath_stripped(&self, face_id: FaceId) -> BezPath {
        let cycle = self.face_boundary(face_id);
        let stripped = self.strip_spurs(&cycle);
        self.cycle_to_bezpath(&stripped)
    }

    /// Bezpath with outer boundary + reversed holes (for fill rendering).
    pub fn face_to_bezpath_with_holes(&self, face_id: FaceId) -> BezPath {
        let face = &self.faces[face_id.idx()];
        let mut path = self.face_to_bezpath_stripped(face_id);

        let inner_hes: Vec<HalfEdgeId> = face.inner_half_edges.clone();
        for inner_he in inner_hes {
            if inner_he.is_none() || self.half_edges[inner_he.idx()].deleted {
                continue;
            }
            let inner_cycle = self.walk_cycle(inner_he);
            let stripped = self.strip_spurs(&inner_cycle);
            if stripped.is_empty() {
                continue;
            }
            // Append hole reversed so winding rule cuts it out
            let reversed = self.cycle_to_bezpath_reversed(&stripped);
            for el in reversed.elements() {
                match *el {
                    PathEl::MoveTo(p) => path.move_to(p),
                    PathEl::LineTo(p) => path.line_to(p),
                    PathEl::QuadTo(p1, p2) => path.quad_to(p1, p2),
                    PathEl::CurveTo(p1, p2, p3) => path.curve_to(p1, p2, p3),
                    PathEl::ClosePath => path.close_path(),
                }
            }
        }
        path
    }

    /// Build a BezPath traversing a cycle in reverse direction.
    fn cycle_to_bezpath_reversed(&self, cycle: &[HalfEdgeId]) -> BezPath {
        let mut path = BezPath::new();
        if cycle.is_empty() {
            return path;
        }

        // Start from the destination of the last half-edge
        let last_dest = self.half_edge_dest(*cycle.last().unwrap());
        let start_pos = self.vertices[last_dest.idx()].position;
        path.move_to(start_pos);

        for &he_id in cycle.iter().rev() {
            let he = &self.half_edges[he_id.idx()];
            let edge = &self.edges[he.edge.idx()];
            if he_id == edge.half_edges[0] {
                // Was forward, now traversing backward
                path.curve_to(edge.curve.p2, edge.curve.p1, edge.curve.p0);
            } else {
                // Was backward, now traversing forward
                path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
            }
        }
        path.close_path();
        path
    }

    /// Strip spur (antenna) edges from a cycle.
    ///
    /// A spur traverses an edge forward then immediately backward (or vice versa).
    /// Stack-based: push half-edges; if top shares the same edge as the new one,
    /// pop (cancel the pair).
    fn strip_spurs(&self, cycle: &[HalfEdgeId]) -> Vec<HalfEdgeId> {
        if cycle.is_empty() {
            return Vec::new();
        }

        let mut stack: Vec<HalfEdgeId> = Vec::with_capacity(cycle.len());
        for &he in cycle {
            if let Some(&top) = stack.last() {
                if self.half_edges[top.idx()].edge == self.half_edges[he.idx()].edge {
                    stack.pop();
                    continue;
                }
            }
            stack.push(he);
        }

        // Handle wrap-around spurs at the seam
        while stack.len() >= 2 {
            let first_edge = self.half_edges[stack[0].idx()].edge;
            let last_edge = self.half_edges[stack.last().unwrap().idx()].edge;
            if first_edge == last_edge {
                stack.remove(0);
                stack.pop();
            } else {
                break;
            }
        }

        stack
    }

    // -------------------------------------------------------------------
    // Validation
    // -------------------------------------------------------------------

    /// Validate DCEL invariants. Panics with a descriptive message on failure.
    pub fn validate(&self) {
        // 1. Twin symmetry
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted { continue; }
            let id = HalfEdgeId(i as u32);
            let twin = he.twin;
            assert!(!twin.is_none(), "HE{i} has NONE twin");
            assert!(!self.half_edges[twin.idx()].deleted, "HE{i} twin is deleted");
            assert_eq!(self.half_edges[twin.idx()].twin, id, "HE{i} twin symmetry broken");
        }

        // 2. Next/prev consistency
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted { continue; }
            let id = HalfEdgeId(i as u32);
            assert!(!he.next.is_none(), "HE{i} has NONE next");
            assert!(!he.prev.is_none(), "HE{i} has NONE prev");
            assert_eq!(self.half_edges[he.next.idx()].prev, id, "HE{i} next.prev != self");
            assert_eq!(self.half_edges[he.prev.idx()].next, id, "HE{i} prev.next != self");
        }

        // 3. Face boundary consistency: all half-edges in a cycle share the same face
        let mut visited = HashSet::new();
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted { continue; }
            let id = HalfEdgeId(i as u32);
            if visited.contains(&id) { continue; }
            let cycle = self.walk_cycle(id);
            let face = he.face;
            for &cid in &cycle {
                assert_eq!(
                    self.half_edges[cid.idx()].face, face,
                    "HE{} face {:?} != cycle leader HE{i} face {:?}",
                    cid.0, self.half_edges[cid.idx()].face, face
                );
                visited.insert(cid);
            }
        }

        // 4. Vertex outgoing consistency
        for (i, v) in self.vertices.iter().enumerate() {
            if v.deleted || v.outgoing.is_none() { continue; }
            let he = &self.half_edges[v.outgoing.idx()];
            assert!(!he.deleted, "V{i} outgoing points to deleted HE");
            assert_eq!(he.origin, VertexId(i as u32), "V{i} outgoing.origin mismatch");
        }

        // 5. Edge ↔ half-edge consistency
        for (i, edge) in self.edges.iter().enumerate() {
            if edge.deleted { continue; }
            let [fwd, bwd] = edge.half_edges;
            assert!(!fwd.is_none() && !bwd.is_none(), "E{i} has NONE half-edges");
            assert_eq!(self.half_edges[fwd.idx()].edge, EdgeId(i as u32), "E{i} fwd.edge mismatch");
            assert_eq!(self.half_edges[bwd.idx()].edge, EdgeId(i as u32), "E{i} bwd.edge mismatch");
            assert_eq!(self.half_edges[fwd.idx()].twin, bwd, "E{i} fwd.twin != bwd");
        }

        // 6. Curve endpoint ↔ vertex position
        for (i, edge) in self.edges.iter().enumerate() {
            if edge.deleted { continue; }
            let [fwd, bwd] = edge.half_edges;
            let v_start = self.half_edges[fwd.idx()].origin;
            let v_end = self.half_edges[bwd.idx()].origin;
            let p_start = self.vertices[v_start.idx()].position;
            let p_end = self.vertices[v_end.idx()].position;
            let d0 = (p_start.x - edge.curve.p0.x).powi(2) + (p_start.y - edge.curve.p0.y).powi(2);
            let d3 = (p_end.x - edge.curve.p3.x).powi(2) + (p_end.y - edge.curve.p3.y).powi(2);
            assert!(d0 < 1.0, "E{i} p0 far from V{}", v_start.0);
            assert!(d3 < 1.0, "E{i} p3 far from V{}", v_end.0);
        }
    }
}
