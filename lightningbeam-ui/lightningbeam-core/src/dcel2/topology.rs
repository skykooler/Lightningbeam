//! Pure topology operations on the DCEL.
//!
//! Core invariants maintained by all operations:
//! - Half-edges leaving a vertex are in sorted CCW order by angle.
//!   The fan is traversed via: `twin(he).next` gives the next CCW outgoing.
//! - `he.next` walks CCW around the face to the left of `he`.
//! - `he.prev` is the inverse of `next`.
//! - `he.twin.origin` is the destination of `he`.
//! - Faces are only created when splitting an existing non-F0 face.

use super::{
    subdivide_cubic, Dcel, EdgeId, FaceId, HalfEdgeId, VertexId, DEFAULT_SNAP_EPSILON,
};
use kurbo::CubicBez;

impl Dcel {
    /// Angle of the curve's forward direction at its start (p0 → p1, fallback p0 → p3).
    pub fn curve_start_angle(curve: &CubicBez) -> f64 {
        let dx = curve.p1.x - curve.p0.x;
        let dy = curve.p1.y - curve.p0.y;
        if dx * dx + dy * dy > 1e-18 {
            dy.atan2(dx)
        } else {
            (curve.p3.y - curve.p0.y).atan2(curve.p3.x - curve.p0.x)
        }
    }

    /// Angle of the curve's backward direction at its end (p3 → p2, fallback p3 → p0).
    pub fn curve_end_angle(curve: &CubicBez) -> f64 {
        let dx = curve.p2.x - curve.p3.x;
        let dy = curve.p2.y - curve.p3.y;
        if dx * dx + dy * dy > 1e-18 {
            dy.atan2(dx)
        } else {
            (curve.p0.y - curve.p3.y).atan2(curve.p0.x - curve.p3.x)
        }
    }

    /// Outgoing angle of a half-edge at its origin vertex.
    pub fn outgoing_angle(&self, he: HalfEdgeId) -> f64 {
        let edge = &self.edges[self.half_edges[he.idx()].edge.idx()];
        if he == edge.half_edges[0] {
            Self::curve_start_angle(&edge.curve)
        } else {
            Self::curve_end_angle(&edge.curve)
        }
    }

    /// Find the existing outgoing half-edge from `vertex` that is the immediate
    /// CCW successor of `angle` in the vertex fan.
    ///
    /// Returns the half-edge whose angular position is the smallest CCW rotation
    /// from `angle`. This is where a new edge at `angle` should be spliced before.
    fn find_ccw_successor(&self, vertex: VertexId, angle: f64) -> HalfEdgeId {
        let start = self.vertices[vertex.idx()].outgoing;
        debug_assert!(!start.is_none(), "find_ccw_successor on isolated vertex");

        let mut best = start;
        let mut best_delta = f64::MAX;
        let mut cur = start;
        loop {
            let a = self.outgoing_angle(cur);
            let mut delta = a - angle;
            if delta <= 0.0 {
                delta += std::f64::consts::TAU;
            }
            if delta < best_delta {
                best_delta = delta;
                best = cur;
            }
            let twin = self.half_edges[cur.idx()].twin;
            cur = self.half_edges[twin.idx()].next;
            if cur == start {
                break;
            }
        }
        best
    }

    /// Insert an edge between two existing vertices.
    ///
    /// `face` is the face that both vertices lie on (for the (true, true) case,
    /// the face is determined by the angular sector and `face` is ignored).
    ///
    /// Returns `(edge_id, face_id)` where `face_id` is:
    /// - A new face if the edge split an existing non-F0 face
    /// - The face the edge was inserted into otherwise
    ///
    /// # Face creation rules
    /// - Faces are only created when both vertices are on the same boundary cycle
    ///   of a face that is NOT face 0 (the unbounded face). This is the "split" case.
    /// - Creating a closed cycle in face 0 does NOT auto-create a face.
    /// - The cycle containing the old face's `outer_half_edge` keeps the old face.
    ///   The other cycle gets a new face with inherited fill data.
    pub fn insert_edge(
        &mut self,
        v1: VertexId,
        v2: VertexId,
        face: FaceId,
        curve: CubicBez,
    ) -> (EdgeId, FaceId) {
        debug_assert!(v1 != v2, "cannot insert self-loop");

        let v1_isolated = self.vertices[v1.idx()].outgoing.is_none();
        let v2_isolated = self.vertices[v2.idx()].outgoing.is_none();

        // Allocate edge + half-edge pair
        let (he_fwd, he_bwd) = self.alloc_half_edge_pair();
        let edge_id = self.alloc_edge(curve);

        self.edges[edge_id.idx()].half_edges = [he_fwd, he_bwd];
        self.half_edges[he_fwd.idx()].edge = edge_id;
        self.half_edges[he_bwd.idx()].edge = edge_id;
        self.half_edges[he_fwd.idx()].origin = v1;
        self.half_edges[he_bwd.idx()].origin = v2;

        match (v1_isolated, v2_isolated) {
            (true, true) => {
                self.insert_edge_both_isolated(he_fwd, he_bwd, v1, v2, edge_id, face)
            }
            (false, false) => {
                self.insert_edge_both_connected(he_fwd, he_bwd, v1, v2, edge_id, &curve, face)
            }
            _ => {
                self.insert_edge_one_isolated(he_fwd, he_bwd, v1, v2, edge_id, &curve, v1_isolated, face)
            }
        }
    }

    /// Both vertices isolated: first edge, no face split possible.
    fn insert_edge_both_isolated(
        &mut self,
        he_fwd: HalfEdgeId,
        he_bwd: HalfEdgeId,
        v1: VertexId,
        v2: VertexId,
        edge_id: EdgeId,
        face: FaceId,
    ) -> (EdgeId, FaceId) {
        // Two half-edges form a trivial 2-cycle
        self.half_edges[he_fwd.idx()].next = he_bwd;
        self.half_edges[he_fwd.idx()].prev = he_bwd;
        self.half_edges[he_bwd.idx()].next = he_fwd;
        self.half_edges[he_bwd.idx()].prev = he_fwd;

        self.half_edges[he_fwd.idx()].face = face;
        self.half_edges[he_bwd.idx()].face = face;

        // Register with face
        if face.0 == 0 {
            self.faces[0].inner_half_edges.push(he_fwd);
        } else if self.faces[face.idx()].outer_half_edge.is_none() {
            self.faces[face.idx()].outer_half_edge = he_fwd;
        }

        self.vertices[v1.idx()].outgoing = he_fwd;
        self.vertices[v2.idx()].outgoing = he_bwd;

        (edge_id, face)
    }

    /// One vertex isolated, one connected: spur/antenna edge, no face split.
    fn insert_edge_one_isolated(
        &mut self,
        he_fwd: HalfEdgeId,
        he_bwd: HalfEdgeId,
        v1: VertexId,
        v2: VertexId,
        edge_id: EdgeId,
        curve: &CubicBez,
        v1_is_isolated: bool,
        face_hint: FaceId,
    ) -> (EdgeId, FaceId) {
        let (connected, isolated) = if v1_is_isolated { (v2, v1) } else { (v1, v2) };

        // Determine which half-edge goes OUT from connected vertex
        let (he_out, he_back) = if self.half_edges[he_fwd.idx()].origin == connected {
            (he_fwd, he_bwd)
        } else {
            (he_bwd, he_fwd)
        };

        // Find where to splice in the fan at the connected vertex
        let out_angle = if self.half_edges[he_fwd.idx()].origin == connected {
            Self::curve_start_angle(curve)
        } else {
            Self::curve_end_angle(curve)
        };
        let ccw_succ = self.find_ccw_successor(connected, out_angle);
        let he_into = self.half_edges[ccw_succ.idx()].prev;
        let angular_face = self.half_edges[he_into.idx()].face;

        // If angular ordering disagrees with face_hint, try to find the correct
        // sector using find_predecessor_on_face — same logic as insert_edge_both_connected.
        let (he_into, ccw_succ, actual_face) = if angular_face != face_hint {
            if let Some((alt_into, alt_ccw)) =
                self.find_predecessor_on_face(connected, out_angle, face_hint)
            {
                (alt_into, alt_ccw, face_hint)
            } else {
                (he_into, ccw_succ, angular_face)
            }
        } else {
            (he_into, ccw_succ, angular_face)
        };

        // Splice: ... → he_into → [he_out → he_back] → ccw_succ → ...
        self.half_edges[he_into.idx()].next = he_out;
        self.half_edges[he_out.idx()].prev = he_into;
        self.half_edges[he_out.idx()].next = he_back;
        self.half_edges[he_back.idx()].prev = he_out;
        self.half_edges[he_back.idx()].next = ccw_succ;
        self.half_edges[ccw_succ.idx()].prev = he_back;

        self.half_edges[he_out.idx()].face = actual_face;
        self.half_edges[he_back.idx()].face = actual_face;

        self.vertices[isolated.idx()].outgoing = he_back;

        (edge_id, actual_face)
    }

    /// Both vertices connected: may split a face.
    ///
    /// `face_hint` is the face the caller expects the edge to be in.
    /// If the angular ordering places the edge in F0 but `face_hint` is
    /// a non-F0 face, the opposite angular sector is tried.
    fn insert_edge_both_connected(
        &mut self,
        he_fwd: HalfEdgeId,
        he_bwd: HalfEdgeId,
        v1: VertexId,
        v2: VertexId,
        edge_id: EdgeId,
        curve: &CubicBez,
        face_hint: FaceId,
    ) -> (EdgeId, FaceId) {
        let fwd_angle = Self::curve_start_angle(curve);
        let bwd_angle = Self::curve_end_angle(curve);

        let ccw_v1 = self.find_ccw_successor(v1, fwd_angle);
        let ccw_v2 = self.find_ccw_successor(v2, bwd_angle);

        let into_v1 = self.half_edges[ccw_v1.idx()].prev;
        let into_v2 = self.half_edges[ccw_v2.idx()].prev;

        let face_v1 = self.half_edges[into_v1.idx()].face;
        let face_v2 = self.half_edges[into_v2.idx()].face;

        // If the angular ordering places both predecessors in F0 but the
        // caller expects a non-F0 face, try the opposite sector: use
        // `ccw_v1` and `ccw_v2`'s twins to find the other sector at each vertex.
        let (into_v1, into_v2, ccw_v1, ccw_v2, actual_face) =
            if face_v1 == face_v2 && face_v1.0 == 0 && face_hint.0 != 0 {
                // Try the opposite sector: at each vertex, the predecessor
                // of the OTHER outgoing edge in the face_hint cycle.
                let alt = self.find_predecessor_on_face(v1, fwd_angle, face_hint)
                    .zip(self.find_predecessor_on_face(v2, bwd_angle, face_hint));
                if let Some(((alt_into_v1, alt_ccw_v1), (alt_into_v2, alt_ccw_v2))) = alt {
                    (alt_into_v1, alt_into_v2, alt_ccw_v1, alt_ccw_v2, face_hint)
                } else {
                    debug_assert_eq!(face_v1, face_v2);
                    (into_v1, into_v2, ccw_v1, ccw_v2, face_v1)
                }
            } else if face_v1 != face_v2 {
                // Angular ordering disagrees between the two endpoints.
                // Trust face_hint (midpoint probe) as the authoritative face —
                // it correctly determines which face the edge's interior lies in,
                // regardless of which angular sector each vertex landed in.
                let target = face_hint;
                let fix_v1 = if face_v1 == target {
                    (into_v1, ccw_v1)
                } else {
                    self.find_predecessor_on_face(v1, fwd_angle, target)
                        .unwrap_or((into_v1, ccw_v1))
                };
                let fix_v2 = if face_v2 == target {
                    (into_v2, ccw_v2)
                } else {
                    self.find_predecessor_on_face(v2, bwd_angle, target)
                        .unwrap_or((into_v2, ccw_v2))
                };
                (fix_v1.0, fix_v2.0, fix_v1.1, fix_v2.1, target)
            } else {
                debug_assert_eq!(
                    face_v1, face_v2,
                    "insert_edge_both_connected: into_v1 (HE{}) on {:?} but into_v2 (HE{}) on {:?}",
                    into_v1.0, face_v1, into_v2.0, face_v2
                );
                (into_v1, into_v2, ccw_v1, ccw_v2, face_v1)
            };
        let actual_face = actual_face;

        // Splice:
        //   into_v1 → he_fwd → ccw_v2 → ...
        //   into_v2 → he_bwd → ccw_v1 → ...
        self.half_edges[he_fwd.idx()].prev = into_v1;
        self.half_edges[he_fwd.idx()].next = ccw_v2;
        self.half_edges[into_v1.idx()].next = he_fwd;
        self.half_edges[ccw_v2.idx()].prev = he_fwd;

        self.half_edges[he_bwd.idx()].prev = into_v2;
        self.half_edges[he_bwd.idx()].next = ccw_v1;
        self.half_edges[into_v2.idx()].next = he_bwd;
        self.half_edges[ccw_v1.idx()].prev = he_bwd;

        // Detect split vs bridge: walk from he_fwd. If we return to he_fwd
        // without seeing he_bwd, they are on separate cycles → split.
        let is_split = !self.cycle_contains(he_fwd, he_bwd);

        if !is_split {
            // Bridge: merged two cycles into one. All on actual_face.
            self.assign_cycle_face(he_fwd, actual_face);
            if actual_face.0 != 0 {
                self.faces[actual_face.idx()].outer_half_edge = he_fwd;
            }
            return (edge_id, actual_face);
        }

        // Split case: two separate cycles.
        // Only create a new face if the face being split is not F0.
        if actual_face.0 == 0 {
            // In the unbounded face, just assign both cycles to F0.
            self.half_edges[he_fwd.idx()].face = FaceId(0);
            self.assign_cycle_face(he_fwd, FaceId(0));
            self.assign_cycle_face(he_bwd, FaceId(0));
            return (edge_id, FaceId(0));
        }

        // Determine which cycle keeps the old face: the one containing
        // the old face's outer_half_edge.
        let old_ohe = self.faces[actual_face.idx()].outer_half_edge;
        let fwd_has_old = !old_ohe.is_none() && self.cycle_contains(he_fwd, old_ohe);

        let (he_old_cycle, he_new_cycle) = if fwd_has_old {
            (he_fwd, he_bwd)
        } else {
            (he_bwd, he_fwd)
        };

        // Old cycle keeps actual_face
        self.assign_cycle_face(he_old_cycle, actual_face);
        self.faces[actual_face.idx()].outer_half_edge = he_old_cycle;

        // New cycle gets a new face with inherited fill data
        let new_face = self.alloc_face();
        self.faces[new_face.idx()].fill_color = self.faces[actual_face.idx()].fill_color;
        self.faces[new_face.idx()].image_fill = self.faces[actual_face.idx()].image_fill;
        self.faces[new_face.idx()].fill_rule = self.faces[actual_face.idx()].fill_rule;
        self.faces[new_face.idx()].outer_half_edge = he_new_cycle;
        self.assign_cycle_face(he_new_cycle, new_face);

        (edge_id, new_face)
    }

    /// Find the predecessor and CCW-successor half-edges in the fan at `vertex`
    /// that belong to a specific face. Returns `(into_he, ccw_successor)` or
    /// `None` if no sector at `vertex` belongs to the given face.
    ///
    /// `into_he` is the HE arriving at `vertex` on the target face's cycle.
    /// `ccw_successor` is the next outgoing HE from `vertex` in the same face's cycle.
    fn find_predecessor_on_face(
        &self,
        vertex: VertexId,
        _angle: f64,
        face: FaceId,
    ) -> Option<(HalfEdgeId, HalfEdgeId)> {
        let start = self.vertices[vertex.idx()].outgoing;
        if start.is_none() {
            return None;
        }

        // Walk the fan at this vertex. For each outgoing HE `cur`, its twin
        // arrives at vertex. twin.next is the next outgoing HE in CCW order.
        // The sector between `cur` (outgoing) and the previous outgoing
        // (i.e., twin of the previous HE) has the face of `prev_twin`.
        // Equivalently: twin of `cur` is on the same face as the sector
        // between `cur` and the next outgoing.
        let mut cur = start;
        loop {
            let twin = self.half_edges[cur.idx()].twin;
            // The face of `twin` is the face of the sector between `cur`
            // (this outgoing) and the next outgoing (twin.next).
            if self.half_edges[twin.idx()].face == face {
                // Found it: `twin` arrives at vertex on the target face,
                // and `twin.next` (= next outgoing) leaves on the target face.
                let next_outgoing = self.half_edges[twin.idx()].next;
                return Some((twin, next_outgoing));
            }
            let next = self.half_edges[twin.idx()].next;
            if next == start {
                break;
            }
            cur = next;
        }
        None
    }

    /// Check if walking the cycle from `start` encounters `target`.
    fn cycle_contains(&self, start: HalfEdgeId, target: HalfEdgeId) -> bool {
        let mut cur = self.half_edges[start.idx()].next;
        let mut steps = 0;
        while cur != start {
            if cur == target {
                return true;
            }
            cur = self.half_edges[cur.idx()].next;
            steps += 1;
            debug_assert!(steps < 100_000, "infinite cycle in cycle_contains");
        }
        false
    }

    /// Set the face of every half-edge in the cycle starting at `start`.
    fn assign_cycle_face(&mut self, start: HalfEdgeId, face: FaceId) {
        self.half_edges[start.idx()].face = face;
        let mut cur = self.half_edges[start.idx()].next;
        let mut steps = 0;
        while cur != start {
            self.half_edges[cur.idx()].face = face;
            cur = self.half_edges[cur.idx()].next;
            steps += 1;
            debug_assert!(steps < 100_000, "infinite cycle in assign_cycle_face");
        }
    }

    /// Split an edge at parameter `t`, inserting a new vertex.
    ///
    /// The original edge is shortened to [0, t]. A new edge covers [t, 1].
    /// Stroke style is copied to the new edge.
    /// Returns `(new_vertex, new_edge)`.
    pub fn split_edge(&mut self, edge_id: EdgeId, t: f64) -> (VertexId, EdgeId) {
        let original_curve = self.edges[edge_id.idx()].curve;
        let (curve_a, _) = subdivide_cubic(original_curve, t);
        let split_point = curve_a.p3;
        let vertex = self
            .snap_vertex(split_point, DEFAULT_SNAP_EPSILON)
            .unwrap_or_else(|| self.alloc_vertex(split_point));
        self.split_edge_at_vertex(edge_id, t, vertex)
    }

    /// Split an edge at parameter `t`, using a specific pre-existing vertex.
    ///
    /// The original edge is shortened to [0, t]. A new edge covers [t, 1].
    /// Curve endpoints are snapped to the vertex position.
    /// Returns `(vertex, new_edge)`.
    pub fn split_edge_at_vertex(
        &mut self,
        edge_id: EdgeId,
        t: f64,
        vertex: VertexId,
    ) -> (VertexId, EdgeId) {
        debug_assert!((0.0..=1.0).contains(&t), "t out of range");

        let original_curve = self.edges[edge_id.idx()].curve;
        let (mut curve_a, mut curve_b) = subdivide_cubic(original_curve, t);

        let vpos = self.vertices[vertex.idx()].position;
        curve_a.p3 = vpos;
        curve_b.p0 = vpos;

        let [he_fwd, he_bwd] = self.edges[edge_id.idx()].half_edges;

        // Allocate new edge + half-edge pair for second segment
        let (new_he_fwd, new_he_bwd) = self.alloc_half_edge_pair();
        let new_edge_id = self.alloc_edge(curve_b);

        self.edges[new_edge_id.idx()].half_edges = [new_he_fwd, new_he_bwd];
        self.half_edges[new_he_fwd.idx()].edge = new_edge_id;
        self.half_edges[new_he_bwd.idx()].edge = new_edge_id;

        // Copy stroke style
        self.edges[new_edge_id.idx()].stroke_style =
            self.edges[edge_id.idx()].stroke_style.clone();
        self.edges[new_edge_id.idx()].stroke_color = self.edges[edge_id.idx()].stroke_color;

        // Shorten original edge
        self.edges[edge_id.idx()].curve = curve_a;

        // Set origins: new_he_fwd goes from vertex onward,
        // new_he_bwd goes from old destination toward vertex
        self.half_edges[new_he_fwd.idx()].origin = vertex;
        let old_dest = self.half_edges[he_bwd.idx()].origin;
        self.half_edges[new_he_bwd.idx()].origin = old_dest;

        // Splice new_he_fwd into forward cycle:
        // Before: ... → he_fwd → fwd_next → ...
        // After:  ... → he_fwd → new_he_fwd → fwd_next → ...
        let fwd_next = self.half_edges[he_fwd.idx()].next;
        self.half_edges[he_fwd.idx()].next = new_he_fwd;
        self.half_edges[new_he_fwd.idx()].prev = he_fwd;
        self.half_edges[new_he_fwd.idx()].next = fwd_next;
        self.half_edges[fwd_next.idx()].prev = new_he_fwd;
        self.half_edges[new_he_fwd.idx()].face = self.half_edges[he_fwd.idx()].face;

        // Splice new_he_bwd into backward cycle:
        // Before: ... → bwd_prev → he_bwd → ...
        // After:  ... → bwd_prev → new_he_bwd → he_bwd → ...
        let bwd_prev = self.half_edges[he_bwd.idx()].prev;
        self.half_edges[bwd_prev.idx()].next = new_he_bwd;
        self.half_edges[new_he_bwd.idx()].prev = bwd_prev;
        self.half_edges[new_he_bwd.idx()].next = he_bwd;
        self.half_edges[he_bwd.idx()].prev = new_he_bwd;
        self.half_edges[new_he_bwd.idx()].face = self.half_edges[he_bwd.idx()].face;

        // he_bwd now originates from vertex (it covers [vertex → v1])
        self.half_edges[he_bwd.idx()].origin = vertex;

        // Fix old destination's outgoing if it pointed at he_bwd
        if self.vertices[old_dest.idx()].outgoing == he_bwd {
            self.vertices[old_dest.idx()].outgoing = new_he_bwd;
        }

        // Set vertex's outgoing (may already have one if vertex is shared)
        if self.vertices[vertex.idx()].outgoing.is_none() {
            self.vertices[vertex.idx()].outgoing = new_he_fwd;
        }

        (vertex, new_edge_id)
    }

    /// Remove an edge, merging its two adjacent faces.
    /// Returns the surviving face (lower ID, always keeps face 0).
    pub fn remove_edge(&mut self, edge_id: EdgeId) -> FaceId {
        let [he_fwd, he_bwd] = self.edges[edge_id.idx()].half_edges;
        let face_a = self.half_edges[he_fwd.idx()].face;
        let face_b = self.half_edges[he_bwd.idx()].face;

        let (surviving, dying) = if face_a.0 <= face_b.0 {
            (face_a, face_b)
        } else {
            (face_b, face_a)
        };

        let fwd_prev = self.half_edges[he_fwd.idx()].prev;
        let fwd_next = self.half_edges[he_fwd.idx()].next;
        let bwd_prev = self.half_edges[he_bwd.idx()].prev;
        let bwd_next = self.half_edges[he_bwd.idx()].next;

        let v1 = self.half_edges[he_fwd.idx()].origin;
        let v2 = self.half_edges[he_bwd.idx()].origin;

        // Splice out half-edges. Four cases based on adjacency.
        if fwd_next == he_bwd && bwd_next == he_fwd {
            // Degenerate 2-cycle: both vertices become isolated
            self.vertices[v1.idx()].outgoing = HalfEdgeId::NONE;
            self.vertices[v2.idx()].outgoing = HalfEdgeId::NONE;
        } else if fwd_next == he_bwd {
            // Spur: he_fwd → he_bwd consecutive. Remove both.
            self.half_edges[fwd_prev.idx()].next = bwd_next;
            self.half_edges[bwd_next.idx()].prev = fwd_prev;
            self.vertices[v2.idx()].outgoing = HalfEdgeId::NONE;
            if self.vertices[v1.idx()].outgoing == he_fwd {
                self.vertices[v1.idx()].outgoing = bwd_next;
            }
        } else if bwd_next == he_fwd {
            // Spur: he_bwd → he_fwd consecutive. Remove both.
            self.half_edges[bwd_prev.idx()].next = fwd_next;
            self.half_edges[fwd_next.idx()].prev = bwd_prev;
            self.vertices[v1.idx()].outgoing = HalfEdgeId::NONE;
            if self.vertices[v2.idx()].outgoing == he_bwd {
                self.vertices[v2.idx()].outgoing = fwd_next;
            }
        } else {
            // Normal: splice out both half-edges
            self.half_edges[fwd_prev.idx()].next = bwd_next;
            self.half_edges[bwd_next.idx()].prev = fwd_prev;
            self.half_edges[bwd_prev.idx()].next = fwd_next;
            self.half_edges[fwd_next.idx()].prev = bwd_prev;

            if self.vertices[v1.idx()].outgoing == he_fwd {
                self.vertices[v1.idx()].outgoing = bwd_next;
            }
            if self.vertices[v2.idx()].outgoing == he_bwd {
                self.vertices[v2.idx()].outgoing = fwd_next;
            }
        }

        // Reassign dying face's half-edges to surviving face
        if surviving != dying && !dying.is_none() {
            let walk_start = self.find_surviving_he_for_face(dying, he_fwd, he_bwd, fwd_next, bwd_next);
            if !walk_start.is_none() {
                self.assign_cycle_face(walk_start, surviving);
            }
            // Merge holes
            let inner = std::mem::take(&mut self.faces[dying.idx()].inner_half_edges);
            self.faces[surviving.idx()].inner_half_edges.extend(inner);
        }

        // Fix surviving face's outer_half_edge
        if self.faces[surviving.idx()].outer_half_edge == he_fwd
            || self.faces[surviving.idx()].outer_half_edge == he_bwd
        {
            let replacement = [fwd_next, bwd_next]
                .into_iter()
                .find(|&he| he != he_fwd && he != he_bwd && !self.half_edges[he.idx()].deleted)
                .unwrap_or(HalfEdgeId::NONE);
            self.faces[surviving.idx()].outer_half_edge = replacement;
        }

        // Clean up inner_half_edges references
        self.faces[surviving.idx()]
            .inner_half_edges
            .retain(|&he| he != he_fwd && he != he_bwd);

        // Free
        self.free_half_edge(he_fwd);
        self.free_half_edge(he_bwd);
        self.free_edge(edge_id);
        if surviving != dying && !dying.is_none() && dying.0 != 0 {
            self.free_face(dying);
        }

        surviving
    }

    /// Find a valid starting half-edge for walking a dying face's cycle,
    /// avoiding the two half-edges being removed.
    fn find_surviving_he_for_face(
        &self,
        dying: FaceId,
        he_fwd: HalfEdgeId,
        he_bwd: HalfEdgeId,
        fwd_next: HalfEdgeId,
        bwd_next: HalfEdgeId,
    ) -> HalfEdgeId {
        let ohe = self.faces[dying.idx()].outer_half_edge;
        if !ohe.is_none() && ohe != he_fwd && ohe != he_bwd {
            return ohe;
        }
        for &candidate in &[fwd_next, bwd_next] {
            if !candidate.is_none() && candidate != he_fwd && candidate != he_bwd {
                return candidate;
            }
        }
        HalfEdgeId::NONE
    }

    /// Create a face from an existing cycle of half-edges in F0.
    ///
    /// Use this when a closed boundary exists but no face was created
    /// (e.g. paint bucket on a region that hasn't been filled yet).
    /// The cycle's half-edges are assigned to the new face.
    /// Returns the new FaceId.
    pub fn create_face_at_cycle(&mut self, cycle_he: HalfEdgeId) -> FaceId {
        let face = self.alloc_face();
        self.faces[face.idx()].outer_half_edge = cycle_he;
        self.assign_cycle_face(cycle_he, face);
        face
    }

    /// Re-sort all outgoing half-edges at a vertex by angle and fix the
    /// fan linkage (`twin.next` / `prev`). Call this after operations that
    /// add outgoing half-edges to an existing vertex without maintaining
    /// the CCW fan invariant (e.g. multiple `split_edge_at_vertex` calls
    /// reusing the same vertex).
    pub fn rebuild_vertex_fan(&mut self, vertex: VertexId) {
        let start = self.vertices[vertex.idx()].outgoing;
        if start.is_none() {
            return;
        }

        // Collect all outgoing half-edges by walking all connected sub-fans.
        // The fan may be broken into disconnected loops, so we gather them
        // by scanning all half-edges with origin == vertex.
        let mut fan: Vec<(f64, HalfEdgeId)> = Vec::new();
        for (i, he) in self.half_edges.iter().enumerate() {
            if he.deleted {
                continue;
            }
            if he.origin == vertex {
                let he_id = HalfEdgeId(i as u32);
                let angle = self.outgoing_angle(he_id);
                fan.push((angle, he_id));
            }
        }

        if fan.is_empty() {
            return;
        }

        // Sort by angle CCW
        fan.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        // Relink: twin(fan[i]).next = fan[(i+1) % n]
        let n = fan.len();
        for i in 0..n {
            let cur_he = fan[i].1;
            let next_he = fan[(i + 1) % n].1;
            let cur_twin = self.half_edges[cur_he.idx()].twin;
            self.half_edges[cur_twin.idx()].next = next_he;
            self.half_edges[next_he.idx()].prev = cur_twin;
        }

        self.vertices[vertex.idx()].outgoing = fan[0].1;
    }

    /// After `rebuild_vertex_fan` re-links `next`/`prev` pointers at a vertex,
    /// face assignments may be wrong in two ways:
    ///
    /// 1. **Multiple cycles per face**: A face's single boundary was split
    ///    into two separate cycles. Each extra cycle gets a new face.
    ///
    /// 2. **Pinched cycle**: A face's boundary visits a vertex more than
    ///    once ("figure-8" or "lollipop" shape). The cycle is split at the
    ///    repeated vertex into sub-cycles, each becoming its own face.
    ///
    /// Returns the list of newly created faces.
    pub fn repair_face_cycles_at_vertex(&mut self, vertex: VertexId) -> Vec<FaceId> {
        let outgoing = self.vertex_outgoing(vertex);
        if outgoing.is_empty() {
            return Vec::new();
        }

        use std::collections::HashMap;
        let mut new_faces = Vec::new();

        // --- Phase 1: Detect and split pinched cycles ---
        //
        // Walk ALL cycles touching this vertex. When a cycle visits a vertex
        // twice (pinch), extract the loop sub-path as a new cycle.
        // If the loop has positive area, create a new face (inheriting fill
        // from an adjacent non-F0 face if the parent cycle is F0).

        // Collect unique cycle start HEs touching this vertex
        let mut cycle_starts: Vec<HalfEdgeId> = Vec::new();
        let mut seen_cycle_reps: Vec<HalfEdgeId> = Vec::new();
        for &he in &outgoing {
            for start in [he, self.half_edges[he.idx()].twin] {
                let cycle = self.walk_cycle(start);
                let rep = cycle.iter().copied().min_by_key(|h| h.0).unwrap();
                if !seen_cycle_reps.contains(&rep) {
                    seen_cycle_reps.push(rep);
                    cycle_starts.push(start);
                }
            }
        }

        for cycle_start in cycle_starts {
            // Walk the cycle vertex-by-vertex, including one extra step
            // to re-check the start vertex for a closing pinch.
            let mut vertex_first_he: HashMap<VertexId, HalfEdgeId> = HashMap::new();
            let mut cur = cycle_start;
            let cycle_len = self.walk_cycle(cycle_start).len();
            let mut steps = 0;
            let mut finished = false;

            loop {
                let v = self.half_edges[cur.idx()].origin;

                if let Some(&first_he) = vertex_first_he.get(&v) {
                    // Pinch detected! Extract the loop (first_he..last_of_loop).
                    let prev_of_first = self.half_edges[first_he.idx()].prev;
                    let last_of_loop = self.half_edges[cur.idx()].prev;

                    // Relink: close the loop and bridge the main cycle
                    self.half_edges[last_of_loop.idx()].next = first_he;
                    self.half_edges[first_he.idx()].prev = last_of_loop;
                    self.half_edges[prev_of_first.idx()].next = cur;
                    self.half_edges[cur.idx()].prev = prev_of_first;

                    // Determine area of the extracted loop
                    let mut area = self.cycle_signed_area(first_he);
                    if area.abs() < 1e-6 {
                        area = self.cycle_curve_signed_area(first_he);
                    }

                    // Find a non-F0 donor face at the pinch vertex
                    let donor_face = if area > 0.0 {
                        let mut df = FaceId(0);
                        for he_rec in self.half_edges.iter() {
                            if he_rec.deleted { continue; }
                            if he_rec.origin == v {
                                if he_rec.face.0 != 0 {
                                    df = he_rec.face;
                                    break;
                                }
                                let tf = self.half_edges[he_rec.twin.idx()].face;
                                if tf.0 != 0 {
                                    df = tf;
                                    break;
                                }
                            }
                        }
                        df
                    } else {
                        FaceId(0)
                    };

                    if area > 0.0 && donor_face.0 != 0 {
                        let nf = self.alloc_face();
                        self.faces[nf.idx()].fill_color =
                            self.faces[donor_face.idx()].fill_color;
                        self.faces[nf.idx()].image_fill =
                            self.faces[donor_face.idx()].image_fill;
                        self.faces[nf.idx()].fill_rule =
                            self.faces[donor_face.idx()].fill_rule;
                        self.faces[nf.idx()].outer_half_edge = first_he;
                        self.assign_cycle_face(first_he, nf);
                        new_faces.push(nf);
                    } else {
                        // Undo the relink
                        self.half_edges[last_of_loop.idx()].next = cur;
                        self.half_edges[cur.idx()].prev = last_of_loop;
                        self.half_edges[prev_of_first.idx()].next = first_he;
                        self.half_edges[first_he.idx()].prev = prev_of_first;
                    }

                    vertex_first_he.insert(v, cur);
                } else {
                    vertex_first_he.insert(v, cur);
                }

                if finished {
                    break;
                }

                cur = self.half_edges[cur.idx()].next;
                steps += 1;
                if steps > cycle_len + 2 {
                    break;
                }
                // When we've come full circle, process cycle_start once
                // more (to detect a closing pinch) then stop.
                if cur == cycle_start {
                    finished = true;
                }
            }
        }

        // --- Phase 2: Handle multiple separate cycles per face ---
        // (This handles case 1: rebuild_vertex_fan split one cycle into two
        //  distinct cycles, without a pinch.)
        let outgoing = self.vertex_outgoing(vertex);
        let mut cycle_reps: Vec<(HalfEdgeId, FaceId)> = Vec::new();
        let mut seen_reps: Vec<HalfEdgeId> = Vec::new();

        for &he in &outgoing {
            for start in [he, self.half_edges[he.idx()].twin] {
                let cycle = self.walk_cycle(start);
                let rep = cycle.iter().copied().min_by_key(|h| h.0).unwrap();
                if !seen_reps.contains(&rep) {
                    let face = self.half_edges[start.idx()].face;
                    cycle_reps.push((rep, face));
                    seen_reps.push(rep);
                }
            }
        }

        let mut face_cycles: HashMap<FaceId, Vec<HalfEdgeId>> = HashMap::new();
        for &(rep, face) in &cycle_reps {
            face_cycles.entry(face).or_default().push(rep);
        }

        for (&face, cycles) in &face_cycles {
            if face.0 == 0 || cycles.len() <= 1 {
                continue;
            }

            let old_ohe = self.faces[face.idx()].outer_half_edge;

            for &cycle_rep in cycles {
                let has_old_ohe = !old_ohe.is_none()
                    && (cycle_rep == old_ohe || self.cycle_contains(cycle_rep, old_ohe));
                if has_old_ohe {
                    self.assign_cycle_face(cycle_rep, face);
                    continue;
                }

                let area = self.cycle_signed_area(cycle_rep);

                if area > 0.0 {
                    let nf = self.alloc_face();
                    self.faces[nf.idx()].fill_color = self.faces[face.idx()].fill_color;
                    self.faces[nf.idx()].image_fill = self.faces[face.idx()].image_fill;
                    self.faces[nf.idx()].fill_rule = self.faces[face.idx()].fill_rule;
                    self.faces[nf.idx()].outer_half_edge = cycle_rep;
                    self.assign_cycle_face(cycle_rep, nf);
                    new_faces.push(nf);
                } else {
                    self.assign_cycle_face(cycle_rep, face);
                    if !self.faces[face.idx()].inner_half_edges.contains(&cycle_rep) {
                        self.faces[face.idx()].inner_half_edges.push(cycle_rep);
                    }
                }
            }
        }

        new_faces
    }

    /// Merge vertex `v_remove` into `v_keep`. Both must be at the same position
    /// (or close enough). All half-edges originating from `v_remove` are re-homed
    /// to `v_keep`, and the combined fan is re-sorted by angle.
    pub fn merge_vertices(&mut self, v_keep: VertexId, v_remove: VertexId) {
        if v_keep == v_remove {
            return;
        }
        debug_assert!(!self.vertices[v_keep.idx()].outgoing.is_none());
        debug_assert!(!self.vertices[v_remove.idx()].outgoing.is_none());

        // Re-home all half-edges from v_remove to v_keep
        let start = self.vertices[v_remove.idx()].outgoing;
        let mut cur = start;
        loop {
            self.half_edges[cur.idx()].origin = v_keep;
            let twin = self.half_edges[cur.idx()].twin;
            cur = self.half_edges[twin.idx()].next;
            if cur == start {
                break;
            }
        }

        self.vertices[v_remove.idx()].outgoing = HalfEdgeId::NONE;
        self.vertices[v_remove.idx()].deleted = true;
        self.free_vertices.push(v_remove.0);

        // Rebuild the combined fan at v_keep
        self.rebuild_vertex_fan(v_keep);
        self.vertex_rtree = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn line_curve(p0: Point, p1: Point) -> CubicBez {
        let c1 = super::super::lerp_point(p0, p1, 1.0 / 3.0);
        let c2 = super::super::lerp_point(p0, p1, 2.0 / 3.0);
        CubicBez::new(p0, c1, c2, p1)
    }

    #[test]
    fn insert_single_edge() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let curve = line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0));

        let (edge_id, face) = dcel.insert_edge(v1, v2, FaceId(0), curve);
        assert!(!edge_id.is_none());
        assert_eq!(face, FaceId(0));

        // Both half-edges should form a 2-cycle
        let [he_fwd, he_bwd] = dcel.edges[edge_id.idx()].half_edges;
        assert_eq!(dcel.half_edges[he_fwd.idx()].next, he_bwd);
        assert_eq!(dcel.half_edges[he_bwd.idx()].next, he_fwd);
        assert_eq!(dcel.half_edges[he_fwd.idx()].origin, v1);
        assert_eq!(dcel.half_edges[he_bwd.idx()].origin, v2);
    }

    #[test]
    fn insert_spur() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let v3 = dcel.alloc_vertex(Point::new(10.0, 10.0));

        let c1 = line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0));
        let c2 = line_curve(Point::new(10.0, 0.0), Point::new(10.0, 10.0));

        dcel.insert_edge(v1, v2, FaceId(0), c1);
        let (e2, _) = dcel.insert_edge(v2, v3, FaceId(0), c2);

        // v3 should have outgoing pointing back toward v2
        let v3_out = dcel.vertices[v3.idx()].outgoing;
        assert!(!v3_out.is_none());
        assert_eq!(dcel.half_edges[v3_out.idx()].origin, v3);

        // Edge should exist
        assert!(!e2.is_none());
    }

    #[test]
    fn insert_triangle_no_face_in_f0() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let v3 = dcel.alloc_vertex(Point::new(5.0, 10.0));

        let c1 = line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0));
        let c2 = line_curve(Point::new(10.0, 0.0), Point::new(5.0, 10.0));
        let c3 = line_curve(Point::new(5.0, 10.0), Point::new(0.0, 0.0));

        dcel.insert_edge(v1, v2, FaceId(0), c1);
        dcel.insert_edge(v2, v3, FaceId(0), c2);
        let (_e3, face) = dcel.insert_edge(v3, v1, FaceId(0), c3);

        // In F0, closing a triangle should NOT create a new face
        assert_eq!(face, FaceId(0));
    }

    #[test]
    fn split_edge_creates_vertex() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let curve = line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0));

        let (edge_id, _) = dcel.insert_edge(v1, v2, FaceId(0), curve);
        let (new_v, new_e) = dcel.split_edge(edge_id, 0.5);

        // New vertex should be near (5, 0)
        let pos = dcel.vertices[new_v.idx()].position;
        assert!((pos.x - 5.0).abs() < 0.1);
        assert!((pos.y - 0.0).abs() < 0.1);

        // Should now have 2 edges
        assert!(!new_e.is_none());
        assert_ne!(edge_id, new_e);
    }

    #[test]
    fn remove_edge_basic() {
        let mut dcel = Dcel::new();
        let v1 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(10.0, 0.0));
        let curve = line_curve(Point::new(0.0, 0.0), Point::new(10.0, 0.0));

        let (edge_id, _) = dcel.insert_edge(v1, v2, FaceId(0), curve);
        let surviving = dcel.remove_edge(edge_id);

        assert_eq!(surviving, FaceId(0));
        assert!(dcel.vertices[v1.idx()].outgoing.is_none());
        assert!(dcel.vertices[v2.idx()].outgoing.is_none());
        assert!(dcel.edges[edge_id.idx()].deleted);
    }

    /// Test that `repair_face_cycles_at_vertex` correctly splits a face
    /// when `rebuild_vertex_fan` has broken one cycle into two.
    #[test]
    fn repair_face_cycles_splits_face() {
        use crate::shape::ShapeColor;

        let mut dcel = Dcel::new();

        // Build a rectangle manually: 6 vertices, 6 edges
        // The rectangle has split points on the left and right sides
        // to simulate the result of splitting edges at intersection points.
        //
        //  v3 ---- v2
        //  |        |
        // vL       vR
        //  |        |
        //  v0 ---- v1
        let v0 = dcel.alloc_vertex(Point::new(0.0, 0.0));
        let v1 = dcel.alloc_vertex(Point::new(100.0, 0.0));
        let v2 = dcel.alloc_vertex(Point::new(100.0, 100.0));
        let v3 = dcel.alloc_vertex(Point::new(0.0, 100.0));
        let vl = dcel.alloc_vertex(Point::new(0.0, 50.0));
        let vr = dcel.alloc_vertex(Point::new(100.0, 50.0));

        // Insert edges forming the rectangle boundary (with split points)
        // Bottom: v0 → v1
        dcel.insert_edge(v0, v1, FaceId(0), line_curve(Point::new(0.0, 0.0), Point::new(100.0, 0.0)));
        // Right-bottom: v1 → vR
        dcel.insert_edge(v1, vr, FaceId(0), line_curve(Point::new(100.0, 0.0), Point::new(100.0, 50.0)));
        // Right-top: vR → v2
        dcel.insert_edge(vr, v2, FaceId(0), line_curve(Point::new(100.0, 50.0), Point::new(100.0, 100.0)));
        // Top: v2 → v3
        dcel.insert_edge(v2, v3, FaceId(0), line_curve(Point::new(100.0, 100.0), Point::new(0.0, 100.0)));
        // Left-top: v3 → vL
        dcel.insert_edge(v3, vl, FaceId(0), line_curve(Point::new(0.0, 100.0), Point::new(0.0, 50.0)));
        // Left-bottom: vL → v0
        dcel.insert_edge(vl, v0, FaceId(0), line_curve(Point::new(0.0, 50.0), Point::new(0.0, 0.0)));

        // Create a face on the CCW interior cycle
        let he_opts = dcel.vertex_outgoing(v0);
        let interior_he = he_opts
            .iter()
            .copied()
            .find(|&he| dcel.cycle_signed_area(he) > 0.0)
            .expect("should have a CCW cycle");
        let face = dcel.create_face_at_cycle(interior_he);
        dcel.faces[face.idx()].fill_color = Some(ShapeColor::rgb(255, 0, 0));

        dcel.validate();

        // Now insert the cross edge vL → vR (splitting the face)
        let cross_curve = line_curve(Point::new(0.0, 50.0), Point::new(100.0, 50.0));
        let (_, _returned_face) = dcel.insert_edge(vl, vr, face, cross_curve);

        // insert_edge_both_connected should have split the face
        let filled_faces: Vec<_> = dcel
            .faces
            .iter()
            .enumerate()
            .filter(|(i, f)| *i != 0 && !f.deleted && f.fill_color.is_some())
            .collect();

        assert!(
            filled_faces.len() >= 2,
            "expected at least 2 filled faces after cross edge, got {}",
            filled_faces.len(),
        );

        dcel.validate();
    }
}
