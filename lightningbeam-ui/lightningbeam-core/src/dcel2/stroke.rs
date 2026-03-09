//! High-level stroke insertion into the DCEL.
//!
//! `insert_stroke` is the main entry point for the Draw tool.
//!
//! For each new stroke segment, we find intersections with existing edges and
//! immediately split both curves at the intersection point, sharing a single
//! vertex. This avoids the problem where batch-processing gives slightly
//! different intersection positions for the same crossing.

use super::{
    subsegment_cubic, Dcel, EdgeId, FaceId, VertexId,
};
use crate::curve_intersections::find_curve_intersections;
use crate::shape::{ShapeColor, StrokeStyle};
use kurbo::{CubicBez, ParamCurve, Point};

pub struct InsertStrokeResult {
    pub new_vertices: Vec<VertexId>,
    pub new_edges: Vec<EdgeId>,
    pub split_edges: Vec<(EdgeId, f64, VertexId, EdgeId)>,
    pub new_faces: Vec<FaceId>,
}

/// A split point along a stroke segment, in stroke-parameter order.
#[derive(Debug, Clone)]
struct SegmentSplit {
    /// Parameter on the stroke segment where the split occurs.
    t: f64,
    /// The vertex at the split point (already created by splitting the existing edge).
    vertex: VertexId,
}

/// Endpoint proximity threshold: intersections this close to an endpoint
/// are filtered (vertex snapping handles them instead).
const ENDPOINT_T_MARGIN: f64 = 0.01;

impl Dcel {
    /// For a single stroke segment, find all intersections with existing edges.
    /// For each intersection, immediately split the existing edge and create a
    /// shared vertex. Returns the split points sorted by t along the segment.
    ///
    /// For each existing edge, we find ALL intersections at once, then split
    /// that edge at all of them (high-t to low-t, remapping t values as the
    /// edge shortens). This correctly handles a stroke segment crossing the
    /// same edge multiple times.
    fn intersect_and_split_segment(
        &mut self,
        segment: &CubicBez,
        result: &mut InsertStrokeResult,
    ) -> Vec<SegmentSplit> {
        let mut splits: Vec<SegmentSplit> = Vec::new();

        // Snapshot edge count. Tail edges created by split_edge are portions
        // of edges we already found all intersections for, so they don't need
        // re-checking.
        let edge_count = self.edges.len();

        for edge_idx in 0..edge_count {
            if self.edges[edge_idx].deleted {
                continue;
            }
            let edge_id = EdgeId(edge_idx as u32);
            let edge_curve = self.edges[edge_idx].curve;

            let intersections = find_curve_intersections(segment, &edge_curve);

            // Filter and collect valid hits for this edge
            let mut edge_hits: Vec<(f64, f64, Point)> = intersections
                .iter()
                .filter_map(|ix| {
                    let seg_t = ix.t1;
                    let edge_t = ix.t2.unwrap_or(0.5);
                    if seg_t < ENDPOINT_T_MARGIN || seg_t > 1.0 - ENDPOINT_T_MARGIN {
                        return None;
                    }
                    if edge_t < ENDPOINT_T_MARGIN || edge_t > 1.0 - ENDPOINT_T_MARGIN {
                        return None;
                    }
                    Some((seg_t, edge_t, ix.point))
                })
                .collect();

            if edge_hits.is_empty() {
                continue;
            }

            // Sort by edge_t descending — split from the end first so that
            // earlier t values remain valid on the (shortening) original edge.
            edge_hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

            // Track how much of the original edge the current "head" covers.
            // After splitting at t, the head covers [0, t] of the original,
            // so the next split at t' < t needs remapping: t' / t.
            let mut head_end = 1.0_f64;

            for (seg_t, original_edge_t, point) in edge_hits {
                // Remap to the current head's parameter space
                let remapped_t = original_edge_t / head_end;
                let remapped_t = remapped_t.clamp(ENDPOINT_T_MARGIN, 1.0 - ENDPOINT_T_MARGIN);

                let (vertex, new_edge) = self.split_edge(edge_id, remapped_t);

                // Place vertex at the intersection point (shared between both curves)
                self.vertices[vertex.idx()].position = point;
                self.snap_edge_endpoints_to_vertex(edge_id, vertex);
                self.snap_edge_endpoints_to_vertex(new_edge, vertex);

                result.split_edges.push((edge_id, original_edge_t, vertex, new_edge));
                result.new_vertices.push(vertex);
                splits.push(SegmentSplit { t: seg_t, vertex });

                // The head edge now covers [0, original_edge_t] of the original
                head_end = original_edge_t;
            }
        }

        // Sort by t along the stroke segment
        splits.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());

        // Deduplicate near-identical splits
        splits.dedup_by(|a, b| (a.t - b.t).abs() < ENDPOINT_T_MARGIN);

        splits
    }

    /// Insert a multi-segment stroke into the DCEL.
    ///
    /// For each segment:
    /// 1. Find intersections with existing edges and split them immediately
    /// 2. Snap segment start/end to existing vertices or create new ones
    /// 3. Build a vertex chain: [seg_start, intersection_vertices..., seg_end]
    /// 4. Insert sub-edges between consecutive chain vertices
    pub fn insert_stroke(
        &mut self,
        segments: &[CubicBez],
        stroke_style: Option<StrokeStyle>,
        stroke_color: Option<ShapeColor>,
        epsilon: f64,
    ) -> InsertStrokeResult {
        if let Some(ref mut rec) = self.debug_recorder {
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

        // Pre-pass: split any self-intersecting segments into two.
        // A cubic can self-intersect at most once, producing two sub-segments
        // that share a vertex at the crossing. This must happen before the
        // main loop so the second half can intersect the first half's edge.
        let mut expanded: Vec<CubicBez> = Vec::with_capacity(segments.len());
        for seg in segments {
            if let Some((t1, t2, point)) = Self::find_cubic_self_intersection(seg) {
                // Split into 4 sub-segments: [0,t1], [t1,mid], [mid,t2], [t2,1]
                // where mid is the midpoint of the loop. This avoids creating
                // a loop edge (same start and end vertex) which would break
                // the DCEL topology.
                let t_mid = (t1 + t2) / 2.0;

                let mut s0 = subsegment_cubic(*seg, 0.0, t1);
                let mut s1 = subsegment_cubic(*seg, t1, t_mid);
                let mut s2 = subsegment_cubic(*seg, t_mid, t2);
                let mut s3 = subsegment_cubic(*seg, t2, 1.0);

                // Snap junctions to the crossing point
                s0.p3 = point;
                s1.p0 = point;
                s2.p3 = point;
                s3.p0 = point;

                expanded.push(s0);
                expanded.push(s1);
                expanded.push(s2);
                expanded.push(s3);
            } else {
                expanded.push(*seg);
            }
        }

        // Process each segment: find intersections, split existing edges,
        // then insert sub-edges for the stroke.
        //
        // We track prev_vertex so that adjacent segments share their
        // junction vertex (the end of segment N is the start of segment N+1).
        let mut prev_vertex: Option<VertexId> = None;

        for (seg_idx, seg) in expanded.iter().enumerate() {
            // Phase 1: Intersect this segment against all existing edges
            let splits = self.intersect_and_split_segment(seg, &mut result);

            // Phase 2: Resolve segment start vertex
            let seg_start = if let Some(pv) = prev_vertex {
                pv
            } else {
                self.snap_vertex(seg.p0, epsilon)
                    .unwrap_or_else(|| self.alloc_vertex(seg.p0))
            };

            // Phase 3: Resolve segment end vertex
            let seg_end = if seg_idx == expanded.len() - 1 {
                // Last segment: snap end point
                self.snap_vertex(seg.p3, epsilon)
                    .unwrap_or_else(|| self.alloc_vertex(seg.p3))
            } else {
                // Interior joint: snap to the shared endpoint with next segment
                self.snap_vertex(seg.p3, epsilon)
                    .unwrap_or_else(|| self.alloc_vertex(seg.p3))
            };

            // Phase 4: Build vertex chain
            let mut chain: Vec<(f64, VertexId)> = Vec::with_capacity(splits.len() + 2);
            chain.push((0.0, seg_start));
            for s in &splits {
                chain.push((s.t, s.vertex));
            }
            chain.push((1.0, seg_end));

            // Remove consecutive duplicates (e.g. if seg_start snapped to a split vertex)
            chain.dedup_by(|a, b| a.1 == b.1);

            // Phase 5: Insert sub-edges
            for pair in chain.windows(2) {
                let (t0, v0) = pair[0];
                let (t1, v1) = pair[1];
                if v0 == v1 {
                    continue;
                }

                let mut sub_curve = subsegment_cubic(*seg, t0, t1);
                // Snap curve endpoints to exact vertex positions
                sub_curve.p0 = self.vertices[v0.idx()].position;
                sub_curve.p3 = self.vertices[v1.idx()].position;

                // Determine face by probing the curve midpoint
                let mid = sub_curve.eval(0.5);
                let face = self.find_face_at_point(mid).face;

                let (edge_id, new_face) = self.insert_edge(v0, v1, face, sub_curve);

                self.edges[edge_id.idx()].stroke_style = stroke_style.clone();
                self.edges[edge_id.idx()].stroke_color = stroke_color;

                result.new_edges.push(edge_id);
                if new_face != face && new_face.0 != 0 {
                    result.new_faces.push(new_face);
                }
            }

            // Track vertices
            if !result.new_vertices.contains(&seg_start) {
                result.new_vertices.push(seg_start);
            }
            if !result.new_vertices.contains(&seg_end) {
                result.new_vertices.push(seg_end);
            }

            prev_vertex = Some(seg_end);
        }

        #[cfg(debug_assertions)]
        self.validate();

        result
    }

    /// Find the self-intersection of a cubic bezier, if any.
    ///
    /// A cubic can self-intersect at most once. Returns Some((t1, t2, point))
    /// with t1 < t2 if the curve crosses itself, None otherwise.
    ///
    /// Algebraic approach: B(t) = P0 + 3at + 3bt² + ct³ where
    ///   a = P1-P0,  b = P2-2P1+P0,  c = P3-3P2+3P1-P0
    ///
    /// B(t1) = B(t2), factor (t1-t2), let s=t1+t2, p=t1*t2:
    ///   3a + 3b·s + c·(s²-p) = 0   (two equations, x and y)
    ///
    /// Cross-product elimination gives s, back-substitution gives p,
    /// then t1,t2 = (s ± √(s²-4p)) / 2.
    fn find_cubic_self_intersection(curve: &CubicBez) -> Option<(f64, f64, Point)> {
        let ax = curve.p1.x - curve.p0.x;
        let ay = curve.p1.y - curve.p0.y;
        let bx = curve.p2.x - 2.0 * curve.p1.x + curve.p0.x;
        let by = curve.p2.y - 2.0 * curve.p1.y + curve.p0.y;
        let cx = curve.p3.x - 3.0 * curve.p2.x + 3.0 * curve.p1.x - curve.p0.x;
        let cy = curve.p3.y - 3.0 * curve.p2.y + 3.0 * curve.p1.y - curve.p0.y;

        // s = -(a × c) / (b × c)  where × is 2D cross product
        let b_cross_c = bx * cy - by * cx;
        if b_cross_c.abs() < 1e-10 {
            return None; // degenerate — no self-intersection
        }

        let a_cross_c = ax * cy - ay * cx;
        let s = -a_cross_c / b_cross_c;

        // Back-substitute to find p. Use whichever component of c is larger
        // to avoid division by near-zero.
        let p = if cx.abs() > cy.abs() {
            // From x: cx*(s²-p) + 3*bx*s + 3*ax = 0
            // p = s² + (3*bx*s + 3*ax) / cx
            s * s + (3.0 * bx * s + 3.0 * ax) / cx
        } else if cy.abs() > 1e-10 {
            s * s + (3.0 * by * s + 3.0 * ay) / cy
        } else {
            return None;
        };

        // t1, t2 = (s ± √(s²-4p)) / 2
        let disc = s * s - 4.0 * p;
        if disc < 0.0 {
            return None;
        }
        let sqrt_disc = disc.sqrt();
        let t1 = (s - sqrt_disc) / 2.0;
        let t2 = (s + sqrt_disc) / 2.0;

        // Both must be strictly inside (0, 1)
        if t1 <= ENDPOINT_T_MARGIN || t2 >= 1.0 - ENDPOINT_T_MARGIN || t1 >= t2 {
            return None;
        }

        let p1 = curve.eval(t1);
        let p2 = curve.eval(t2);
        let point = Point::new((p1.x + p2.x) * 0.5, (p1.y + p2.y) * 0.5);

        Some((t1, t2, point))
    }

    /// Recompute intersections for a single edge against all other edges.
    ///
    /// Used after editing a curve's control points — finds new crossings and
    /// splits both curves at each intersection. Returns the list of
    /// (new_vertex, new_edge) pairs created by splits.
    pub fn recompute_edge_intersections(
        &mut self,
        edge_id: EdgeId,
    ) -> Vec<(VertexId, EdgeId)> {
        if self.edges[edge_id.idx()].deleted {
            return Vec::new();
        }

        let curve = self.edges[edge_id.idx()].curve;
        let mut created = Vec::new();

        // 1. Check for self-intersection (loop in this single curve)
        if let Some((t1, t2, point)) = Self::find_cubic_self_intersection(&curve) {
            // Split into 4 sub-edges: [0,t1], [t1,mid], [mid,t2], [t2,1]
            // This avoids creating a self-loop edge (same start and end vertex).
            let t_mid = (t1 + t2) / 2.0;

            // Create one crossing vertex and one loop midpoint vertex
            let cv = self.alloc_vertex(point);
            let mid_point = curve.eval(t_mid);
            let v_mid = self.alloc_vertex(mid_point);

            // Split high-t to low-t, reusing cv for both crossing points
            let (_, tail_edge) = self.split_edge_at_vertex(edge_id, t2, cv);
            created.push((cv, tail_edge));

            let remapped_mid = t_mid / t2;
            let (_, mid_edge2) = self.split_edge_at_vertex(edge_id, remapped_mid, v_mid);
            created.push((v_mid, mid_edge2));

            let remapped_t1 = t1 / t_mid;
            let (_, mid_edge1) = self.split_edge_at_vertex(edge_id, remapped_t1, cv);
            created.push((cv, mid_edge1));

            // Splits inserted cv twice without maintaining the CCW fan — fix it
            self.rebuild_vertex_fan(cv);
            self.repair_face_cycles_at_vertex(cv);
        }

        // 2. Check against all other edges
        //
        // Collect (seg_t_on_edge_id, vertex, point, other_tail) for each
        // intersection so that we can also split edge_id after processing all
        // other edges.  `other_tail` is the sub-edge of the other edge that
        // starts at `vertex` (going forward) — used for sector-based face repair.
        let mut edge_id_splits: Vec<(f64, VertexId, Point, EdgeId)> = Vec::new();

        let edge_count = self.edges.len();
        for other_idx in 0..edge_count {
            if self.edges[other_idx].deleted {
                continue;
            }
            let other_id = EdgeId(other_idx as u32);
            if other_id == edge_id {
                continue;
            }

            // Also skip edges created by splitting edge_id above
            // (they are pieces of the same curve)
            if created.iter().any(|&(_, e)| e == other_id) {
                continue;
            }

            let other_curve = self.edges[other_idx].curve;
            let intersections = find_curve_intersections(&curve, &other_curve);

            let mut hits: Vec<(f64, f64, Point)> = intersections
                .iter()
                .filter_map(|ix| {
                    let seg_t = ix.t1;
                    let edge_t = ix.t2.unwrap_or(0.5);
                    if seg_t < ENDPOINT_T_MARGIN || seg_t > 1.0 - ENDPOINT_T_MARGIN {
                        return None;
                    }
                    if edge_t < ENDPOINT_T_MARGIN || edge_t > 1.0 - ENDPOINT_T_MARGIN {
                        return None;
                    }
                    Some((seg_t, edge_t, ix.point))
                })
                .collect();

            if hits.is_empty() {
                continue;
            }

            // Sort by edge_t descending — split other_id from end first
            hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

            let mut head_end = 1.0_f64;
            for (seg_t, original_edge_t, point) in hits {
                let remapped_t = (original_edge_t / head_end)
                    .clamp(ENDPOINT_T_MARGIN, 1.0 - ENDPOINT_T_MARGIN);

                let (vertex, new_edge) = self.split_edge(other_id, remapped_t);
                self.vertices[vertex.idx()].position = point;
                self.snap_edge_endpoints_to_vertex(other_id, vertex);
                self.snap_edge_endpoints_to_vertex(new_edge, vertex);

                created.push((vertex, new_edge));
                // Record this intersection for splitting edge_id below.
                // `new_edge` is other_tail: the sub-edge of other_id going
                // forward from vertex, used for sector-based face repair.
                edge_id_splits.push((seg_t, vertex, point, new_edge));
                head_end = original_edge_t;
            }
        }

        // 3. Split edge_id at every intersection point found above.
        //
        // We reuse the vertices already created for the other-edge splits so
        // the two sides of each crossing share exactly one vertex.
        //
        // After all splits, each crossing vertex has 4 outgoing half-edges
        // whose angular ordering was not maintained by the mechanical splices.
        // Call rebuild_vertex_fan + repair_face_cycles_at_vertex to fix this
        // (same pattern as the self-intersection case above).
        if !edge_id_splits.is_empty() {
            // Sort by seg_t descending — split edge_id from end first so
            // edge_id always remains the head piece [0 .. current_split_t].
            edge_id_splits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

            // Collect (vertex, editing_tail, other_tail) for sector-based repair.
            // editing_tail = sub-edge of edge_id going forward from vertex.
            // other_tail   = sub-edge of the other edge going forward from vertex.
            let mut touched_info: Vec<(VertexId, EdgeId, EdgeId)> = Vec::new();

            let mut head_end = 1.0_f64;
            for (original_seg_t, vertex, point, other_tail) in edge_id_splits {
                if self.edges[edge_id.idx()].deleted {
                    break;
                }
                let remapped_t = (original_seg_t / head_end)
                    .clamp(ENDPOINT_T_MARGIN, 1.0 - ENDPOINT_T_MARGIN);

                let (_, editing_tail) = self.split_edge_at_vertex(edge_id, remapped_t, vertex);
                // Snap both pieces to the exact intersection point.
                self.vertices[vertex.idx()].position = point;
                self.snap_edge_endpoints_to_vertex(edge_id, vertex);
                self.snap_edge_endpoints_to_vertex(editing_tail, vertex);

                created.push((vertex, editing_tail));
                // Only the first crossing for each vertex is repaired; extra
                // crossings at the same vertex are uncommon and fall back to
                // the basic fan rebuild inside repair_crossing_vertex.
                if !touched_info.iter().any(|&(v, _, _)| v == vertex) {
                    touched_info.push((vertex, editing_tail, other_tail));
                }
                head_end = original_seg_t;
            }

            // Use sector-based face assignment: for each crossing vertex,
            // rebuild the angular fan and assign faces using the tangent
            // cross-product rule (editing face wins the overlap region).
            for (v, editing_tail, other_tail) in touched_info {
                self.repair_crossing_vertex(v, editing_tail, other_tail);
            }
        }

        created
    }

    /// Ensure that any edge endpoint touching `vertex` has its curve snapped
    /// to the vertex's exact position.
    fn snap_edge_endpoints_to_vertex(&mut self, edge_id: EdgeId, vertex: VertexId) {
        let vpos = self.vertices[vertex.idx()].position;
        let edge = &self.edges[edge_id.idx()];
        let [fwd, bwd] = edge.half_edges;

        if self.half_edges[fwd.idx()].origin == vertex {
            self.edges[edge_id.idx()].curve.p0 = vpos;
        }
        if self.half_edges[bwd.idx()].origin == vertex {
            self.edges[edge_id.idx()].curve.p3 = vpos;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    #[test]
    fn u_and_c_four_intersections() {
        let mut dcel = Dcel::new();

        let u_curve = CubicBez::new(
            Point::new(0.0, 100.0),
            Point::new(0.0, -40.0),
            Point::new(100.0, -40.0),
            Point::new(100.0, 100.0),
        );

        let v1 = dcel.alloc_vertex(u_curve.p0);
        let v2 = dcel.alloc_vertex(u_curve.p3);
        dcel.insert_edge(v1, v2, FaceId(0), u_curve);

        let c_curve = CubicBez::new(
            Point::new(120.0, 80.0),
            Point::new(-40.0, 80.0),
            Point::new(-40.0, 20.0),
            Point::new(120.0, 20.0),
        );

        let mut result = InsertStrokeResult {
            new_vertices: Vec::new(),
            new_edges: Vec::new(),
            split_edges: Vec::new(),
            new_faces: Vec::new(),
        };

        let splits = dcel.intersect_and_split_segment(&c_curve, &mut result);

        println!("Found {} splits:", splits.len());
        for (i, s) in splits.iter().enumerate() {
            let pos = dcel.vertices[s.vertex.idx()].position;
            println!("  {i}: t={:.4} V{} ({:.2}, {:.2})", s.t, s.vertex.0, pos.x, pos.y);
        }

        assert_eq!(splits.len(), 4, "U and C should cross 4 times");
        assert_eq!(result.split_edges.len(), 4);

        let split_verts: Vec<VertexId> = splits.iter().map(|s| s.vertex).collect();

        // All split vertices distinct
        for i in 0..split_verts.len() {
            for j in (i + 1)..split_verts.len() {
                assert_ne!(split_verts[i], split_verts[j]);
            }
        }

        // t-values ascending along C
        for w in splits.windows(2) {
            assert!(w[0].t < w[1].t);
        }

        // --- Verify U is now 5 edges chained through the 4 split vertices ---
        // Walk from v1 along forward half-edges to v2.
        // The original edge (edge 0) was shortened; tails were appended.
        // Walk: v1 → split_v[highest_edge_t] → ... → split_v[lowest_edge_t] → v2
        // (splits were high-t-first, so the edge chain from v1 goes through
        // the lowest-edge_t vertex first)
        let mut u_chain: Vec<VertexId> = vec![v1];
        let mut cur_he = dcel.vertices[v1.idx()].outgoing;
        for _ in 0..10 {
            let dest = dcel.half_edge_dest(cur_he);
            u_chain.push(dest);
            if dest == v2 {
                break;
            }
            // Follow forward: next half-edge in the cycle from dest
            // For a chain in F0, the forward half-edge's next is the backward
            // of the same spur, so we need to use the twin's next instead
            // to walk along the chain.
            let twin = dcel.half_edges[cur_he.idx()].twin;
            // At dest, find the outgoing half-edge that continues the chain
            // (not the one going back the way we came)
            let outgoing = dcel.vertex_outgoing(dest);
            let back_he = twin; // the half-edge arriving at dest from our direction
            // The next edge in the chain is the outgoing that isn't the return
            cur_he = *outgoing.iter()
                .find(|&&he| he != dcel.half_edges[back_he.idx()].next
                    || outgoing.len() == 1)
                .unwrap_or(&outgoing[0]);
            // Actually for a simple chain (degree-2 vertices), there are exactly
            // 2 outgoing half-edges; pick the one that isn't the twin of how we arrived
            if outgoing.len() == 2 {
                let _arriving_twin = dcel.half_edges[cur_he.idx()].twin;
                // We want the outgoing that is NOT the reverse of our arrival
                cur_he = if outgoing[0] == dcel.half_edges[twin.idx()].next {
                    // twin.next is the next outgoing in the fan — that's continuing back
                    // For degree-2: the two outgoing are twin.next of each other
                    // We want the one that is NOT going back toward v1
                    outgoing[1]
                } else {
                    outgoing[0]
                };
            }
        }

        // Simpler approach: just verify that all 4 split vertices appear as
        // endpoints of non-deleted edges, and that v1 and v2 are still endpoints.
        let mut u_edge_vertices: Vec<VertexId> = Vec::new();
        for (_i, edge) in dcel.edges.iter().enumerate() {
            if edge.deleted { continue; }
            let [fwd, bwd] = edge.half_edges;
            let a = dcel.half_edges[fwd.idx()].origin;
            let b = dcel.half_edges[bwd.idx()].origin;
            u_edge_vertices.push(a);
            u_edge_vertices.push(b);
        }

        // v1 and v2 (U endpoints) should still be edge endpoints
        assert!(u_edge_vertices.contains(&v1), "v1 should be an edge endpoint");
        assert!(u_edge_vertices.contains(&v2), "v2 should be an edge endpoint");

        // All 4 split vertices should be edge endpoints (they split the U)
        for &sv in &split_verts {
            assert!(u_edge_vertices.contains(&sv),
                "split vertex V{} should be an edge endpoint", sv.0);
        }

        // Should have exactly 5 non-deleted edges (original U split into 5)
        let live_edges: Vec<EdgeId> = dcel.edges.iter().enumerate()
            .filter(|(_, e)| !e.deleted)
            .map(|(i, _)| EdgeId(i as u32))
            .collect();
        assert_eq!(live_edges.len(), 5, "U should be split into 5 edges");

        // Each split vertex should have degree 2 (connects two edge pieces)
        for &sv in &split_verts {
            let out = dcel.vertex_outgoing(sv);
            assert_eq!(out.len(), 2,
                "split vertex V{} should have degree 2, got {}", sv.0, out.len());
        }

        // --- Verify C sub-curves would share the same vertices ---
        // The C would be split into 5 sub-curves at t-values [0, t0, t1, t2, t3, 1].
        // Each sub-curve's endpoints should snap to the split vertices.
        let mut c_t_values: Vec<f64> = vec![0.0];
        c_t_values.extend(splits.iter().map(|s| s.t));
        c_t_values.push(1.0);

        for i in 0..5 {
            let t0 = c_t_values[i];
            let t1 = c_t_values[i + 1];
            let sub = subsegment_cubic(c_curve, t0, t1);

            // Start point of sub-curve should match a known vertex
            if i > 0 {
                let expected_v = split_verts[i - 1];
                let expected_pos = dcel.vertices[expected_v.idx()].position;
                let dist = ((sub.p0.x - expected_pos.x).powi(2)
                    + (sub.p0.y - expected_pos.y).powi(2)).sqrt();
                assert!(dist < 2.0,
                    "C sub-curve {i} start ({:.2},{:.2}) should be near V{} ({:.2},{:.2}), dist={:.3}",
                    sub.p0.x, sub.p0.y, expected_v.0, expected_pos.x, expected_pos.y, dist);
            }

            // End point should match
            if i < 4 {
                let expected_v = split_verts[i];
                let expected_pos = dcel.vertices[expected_v.idx()].position;
                let dist = ((sub.p3.x - expected_pos.x).powi(2)
                    + (sub.p3.y - expected_pos.y).powi(2)).sqrt();
                assert!(dist < 2.0,
                    "C sub-curve {i} end ({:.2},{:.2}) should be near V{} ({:.2},{:.2}), dist={:.3}",
                    sub.p3.x, sub.p3.y, expected_v.0, expected_pos.x, expected_pos.y, dist);
            }
        }

        dcel.validate();
    }

    #[test]
    fn insert_stroke_u_then_c() {
        let mut dcel = Dcel::new();

        // Insert U as a stroke
        let u_curve = CubicBez::new(
            Point::new(0.0, 100.0),
            Point::new(0.0, -40.0),
            Point::new(100.0, -40.0),
            Point::new(100.0, 100.0),
        );
        let u_result = dcel.insert_stroke(&[u_curve], None, None, 0.5);
        assert_eq!(u_result.new_edges.len(), 1);

        // Insert C as a stroke — should split both curves at 4 intersections
        let c_curve = CubicBez::new(
            Point::new(120.0, 80.0),
            Point::new(-40.0, 80.0),
            Point::new(-40.0, 20.0),
            Point::new(120.0, 20.0),
        );
        let c_result = dcel.insert_stroke(&[c_curve], None, None, 0.5);

        println!("C stroke: {} new edges, {} split edges, {} new vertices",
            c_result.new_edges.len(), c_result.split_edges.len(), c_result.new_vertices.len());

        // U was split at 4 points → 4 split_edges
        assert_eq!(c_result.split_edges.len(), 4);

        // C was inserted as 5 sub-edges (split at the 4 intersection points)
        assert_eq!(c_result.new_edges.len(), 5);

        // Total live edges: 5 (U pieces) + 5 (C pieces) = 10
        let live_edges = dcel.edges.iter().filter(|e| !e.deleted).count();
        assert_eq!(live_edges, 10);

        // The 4 intersection vertices should each have degree 4
        // (2 from U chain + 2 from C chain)
        let split_verts: Vec<VertexId> = c_result.split_edges.iter()
            .map(|&(_, _, v, _)| v)
            .collect();
        for &sv in &split_verts {
            let degree = dcel.vertex_outgoing(sv).len();
            assert_eq!(degree, 4,
                "intersection vertex V{} should have degree 4, got {}",
                sv.0, degree);
        }

        dcel.validate();
    }

    #[test]
    fn insert_stroke_simple_cross() {
        let mut dcel = Dcel::new();

        // Horizontal line
        let h = CubicBez::new(
            Point::new(0.0, 50.0),
            Point::new(33.0, 50.0),
            Point::new(66.0, 50.0),
            Point::new(100.0, 50.0),
        );
        dcel.insert_stroke(&[h], None, None, 0.5);

        // Vertical line crossing it
        let v = CubicBez::new(
            Point::new(50.0, 0.0),
            Point::new(50.0, 33.0),
            Point::new(50.0, 66.0),
            Point::new(50.0, 100.0),
        );
        let result = dcel.insert_stroke(&[v], None, None, 0.5);

        // One intersection
        assert_eq!(result.split_edges.len(), 1);
        // Vertical inserted as 2 sub-edges
        assert_eq!(result.new_edges.len(), 2);
        // Total: 2 (H pieces) + 2 (V pieces) = 4
        let live = dcel.edges.iter().filter(|e| !e.deleted).count();
        assert_eq!(live, 4);

        // Intersection vertex has degree 4
        let ix_v = result.split_edges[0].2;
        assert_eq!(dcel.vertex_outgoing(ix_v).len(), 4);

        dcel.validate();
    }

    /// Multi-segment stroke that loops back and crosses itself:
    ///
    ///   seg0: right →
    ///   seg1: down ↓
    ///   seg2: left ← (crosses seg0)
    ///
    /// Since segments are inserted sequentially, seg2 should find and split
    /// the already-inserted seg0 edge at the crossing.
    #[test]
    fn insert_stroke_self_crossing_multi_segment() {
        let mut dcel = Dcel::new();

        let seg0 = CubicBez::new(
            Point::new(0.0, 50.0),
            Point::new(33.0, 50.0),
            Point::new(66.0, 50.0),
            Point::new(100.0, 50.0),
        );
        let seg1 = CubicBez::new(
            Point::new(100.0, 50.0),
            Point::new(100.0, 66.0),
            Point::new(100.0, 83.0),
            Point::new(100.0, 100.0),
        );
        let seg2 = CubicBez::new(
            Point::new(100.0, 100.0),
            Point::new(66.0, 100.0),
            Point::new(33.0, 0.0),
            Point::new(0.0, 0.0),
        );

        let result = dcel.insert_stroke(&[seg0, seg1, seg2], None, None, 0.5);

        println!("Self-crossing: {} edges, {} splits, {} vertices",
            result.new_edges.len(), result.split_edges.len(), result.new_vertices.len());

        // seg2 should cross seg0 once
        assert_eq!(result.split_edges.len(), 1, "seg2 should cross seg0 once");

        // Crossing vertex should have degree 4
        let ix_v = result.split_edges[0].2;
        let degree = dcel.vertex_outgoing(ix_v).len();
        assert_eq!(degree, 4,
            "self-crossing vertex should have degree 4, got {}", degree);

        dcel.validate();
    }

    #[test]
    fn find_self_intersection_loop() {
        // Asymmetric control points that form a true loop (not a cusp).
        // The wider spread gives disc > 0, so t1 ≠ t2.
        let curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(200.0, 100.0),
            Point::new(-100.0, 100.0),
            Point::new(100.0, 0.0),
        );

        let result = Dcel::find_cubic_self_intersection(&curve);
        assert!(result.is_some(), "curve should self-intersect");

        let (t1, t2, point) = result.unwrap();
        println!("Self-ix: t1={t1:.4} t2={t2:.4} at ({:.2}, {:.2})", point.x, point.y);
        assert!(t1 > 0.0 && t1 < 1.0);
        assert!(t2 > t1 && t2 < 1.0);
        // Crossing point should be near the middle of the curve
        assert!((point.x - 50.0).abs() < 20.0);
    }

    #[test]
    fn find_self_intersection_none_for_simple_curve() {
        let curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(33.0, 0.0),
            Point::new(66.0, 0.0),
            Point::new(100.0, 0.0),
        );
        assert!(Dcel::find_cubic_self_intersection(&curve).is_none());
    }

    /// Simulate the editor flow: insert a straight edge, then change its
    /// curve to a self-intersecting loop, then call recompute_edge_intersections.
    /// The crossing vertex should have degree 4 (4 edges meeting there).
    #[test]
    fn recompute_self_intersecting_edge() {
        let mut dcel = Dcel::new();

        // Insert a straight edge
        let p0 = Point::new(0.0, 0.0);
        let p1 = Point::new(100.0, 0.0);
        let v0 = dcel.alloc_vertex(p0);
        let v1 = dcel.alloc_vertex(p1);
        let straight = CubicBez::new(p0, Point::new(33.0, 0.0), Point::new(66.0, 0.0), p1);
        let (edge_id, _) = dcel.insert_edge(v0, v1, FaceId(0), straight);

        assert_eq!(dcel.edges.iter().filter(|e| !e.deleted).count(), 1);

        // Mutate the curve to be self-intersecting (like the user dragging control points)
        dcel.edges[edge_id.idx()].curve = CubicBez::new(
            p0,
            Point::new(200.0, 100.0),
            Point::new(-100.0, 100.0),
            p1,
        );

        // Recompute — should detect self-intersection and split
        let created = dcel.recompute_edge_intersections(edge_id);
        println!("recompute created {} splits", created.len());

        // Should have 4 live edges: [0,t1], [t1,mid], [mid,t2], [t2,1]
        let live_edges = dcel.edges.iter().filter(|e| !e.deleted).count();
        println!("live edges: {live_edges}");
        assert_eq!(live_edges, 4, "self-intersecting curve should become 4 edges");

        // Find the crossing vertex: it's the one with degree 4
        let mut crossing_vertex = None;
        for (i, v) in dcel.vertices.iter().enumerate() {
            if v.deleted || v.outgoing.is_none() { continue; }
            let vid = super::super::VertexId(i as u32);
            let degree = dcel.vertex_outgoing(vid).len();
            println!("V{i}: degree={degree} pos=({:.1},{:.1})", v.position.x, v.position.y);
            if degree == 4 {
                crossing_vertex = Some(vid);
            }
        }

        let cv = crossing_vertex.expect("should have a degree-4 crossing vertex");

        // All 4 outgoing half-edges should belong to different edges
        let outgoing = dcel.vertex_outgoing(cv);
        assert_eq!(outgoing.len(), 4, "crossing vertex should have degree 4");

        let mut edge_ids: Vec<EdgeId> = outgoing
            .iter()
            .map(|&he| dcel.half_edges[he.idx()].edge)
            .collect();
        edge_ids.sort_by_key(|e| e.0);
        edge_ids.dedup();
        assert_eq!(edge_ids.len(), 4, "all 4 outgoing should be on different edges");

        // Verify all 4 edges have the crossing vertex as an endpoint
        for &eid in &edge_ids {
            let [fwd, bwd] = dcel.edges[eid.idx()].half_edges;
            let va = dcel.half_edges[fwd.idx()].origin;
            let vb = dcel.half_edges[bwd.idx()].origin;
            assert!(
                va == cv || vb == cv,
                "edge E{} endpoints V{},V{} should include crossing vertex V{}",
                eid.0, va.0, vb.0, cv.0
            );
        }

        dcel.validate();
    }

    /// Rectangle with a face, then a stroke drawn across it.
    /// The stroke should split two rectangle edges and create sub-edges
    /// inside and outside the face. All face assignments must be consistent.
    #[test]
    fn stroke_across_filled_rectangle() {
        let mut dcel = Dcel::new();

        // Insert rectangle as 4 line segments (like bezpath_to_cubic_segments would)
        let r = 100.0;
        let segs = [
            // bottom: (0,0) → (r,0)
            CubicBez::new(
                Point::new(0.0, 0.0), Point::new(r / 3.0, 0.0),
                Point::new(2.0 * r / 3.0, 0.0), Point::new(r, 0.0),
            ),
            // right: (r,0) → (r,r)
            CubicBez::new(
                Point::new(r, 0.0), Point::new(r, r / 3.0),
                Point::new(r, 2.0 * r / 3.0), Point::new(r, r),
            ),
            // top: (r,r) → (0,r)
            CubicBez::new(
                Point::new(r, r), Point::new(2.0 * r / 3.0, r),
                Point::new(r / 3.0, r), Point::new(0.0, r),
            ),
            // left: (0,r) → (0,0)
            CubicBez::new(
                Point::new(0.0, r), Point::new(0.0, 2.0 * r / 3.0),
                Point::new(0.0, r / 3.0), Point::new(0.0, 0.0),
            ),
        ];

        let rect_result = dcel.insert_stroke(&segs, None, None, 1.0);
        println!("Rectangle: {} edges, {} vertices",
            rect_result.new_edges.len(), rect_result.new_vertices.len());

        // Create a face on the interior cycle (like add_shape does)
        let first_edge = rect_result.new_edges[0];
        let [he_a, he_b] = dcel.edge(first_edge).half_edges;
        let interior_he = if dcel.cycle_signed_area(he_a) > 0.0 { he_a } else { he_b };
        let face = dcel.create_face_at_cycle(interior_he);
        println!("Created face {:?}", face);

        dcel.validate();

        // Now draw a horizontal stroke across the rectangle at y=50
        // from x=-50 to x=150 (extending beyond both sides)
        let stroke = CubicBez::new(
            Point::new(-50.0, 50.0), Point::new(16.0, 50.0),
            Point::new(83.0, 50.0), Point::new(150.0, 50.0),
        );

        let stroke_result = dcel.insert_stroke(&[stroke], None, None, 1.0);
        println!("Stroke: {} edges, {} splits, {} vertices",
            stroke_result.new_edges.len(), stroke_result.split_edges.len(),
            stroke_result.new_vertices.len());

        // Should have split 2 rectangle edges (left and right sides)
        assert_eq!(stroke_result.split_edges.len(), 2,
            "stroke should cross left and right sides of rectangle");

        dcel.validate();
    }

    #[test]
    fn insert_stroke_self_intersecting_segment() {
        let mut dcel = Dcel::new();

        // Single segment that loops on itself (same curve as find_self_intersection_loop)
        let loop_curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(200.0, 100.0),
            Point::new(-100.0, 100.0),
            Point::new(100.0, 0.0),
        );

        let result = dcel.insert_stroke(&[loop_curve], None, None, 0.5);

        // Expanded to 4 sub-segments: [0,t1], [t1,mid], [mid,t2], [t2,1]
        // The loop is split in half to avoid a same-vertex edge.
        assert_eq!(result.new_edges.len(), 4);

        dcel.validate();
    }

    /// After moving a vertex (simulating EditingVertex), the CCW fan ordering
    /// must be rebuilt before inserting new strokes. Without rebuild_vertex_fan,
    /// the stale angular ordering causes face/cycle mismatches.
    #[test]
    fn stroke_after_vertex_move() {
        let mut dcel = Dcel::new();

        // Build a rectangle and create a face
        let r = 100.0;
        let segs = [
            CubicBez::new(
                Point::new(0.0, 0.0), Point::new(r / 3.0, 0.0),
                Point::new(2.0 * r / 3.0, 0.0), Point::new(r, 0.0),
            ),
            CubicBez::new(
                Point::new(r, 0.0), Point::new(r, r / 3.0),
                Point::new(r, 2.0 * r / 3.0), Point::new(r, r),
            ),
            CubicBez::new(
                Point::new(r, r), Point::new(2.0 * r / 3.0, r),
                Point::new(r / 3.0, r), Point::new(0.0, r),
            ),
            CubicBez::new(
                Point::new(0.0, r), Point::new(0.0, 2.0 * r / 3.0),
                Point::new(0.0, r / 3.0), Point::new(0.0, 0.0),
            ),
        ];

        let rect_result = dcel.insert_stroke(&segs, None, None, 1.0);

        let first_edge = rect_result.new_edges[0];
        let [he_a, he_b] = dcel.edge(first_edge).half_edges;
        let interior_he = if dcel.cycle_signed_area(he_a) > 0.0 { he_a } else { he_b };
        let _face = dcel.create_face_at_cycle(interior_he);

        dcel.validate();

        // Simulate dragging the top-right vertex (r, r) → (r + 30, r + 20).
        // This is what finish_vector_editing does for EditingVertex:
        // 1. Update vertex position
        // 2. Update adjacent edge curves
        // 3. Rebuild fans at affected vertices
        let moved_vertex = {
            // Find the vertex at (r, r)
            let vid = dcel.snap_vertex(Point::new(r, r), 1.0).unwrap();
            let new_pos = Point::new(r + 30.0, r + 20.0);

            // Move the vertex
            dcel.vertex_mut(vid).position = new_pos;

            // Update the curves of connected edges to match the new position.
            // Collect edge info first to avoid borrow issues.
            let outgoing: Vec<_> = dcel.vertex_outgoing(vid)
                .iter()
                .map(|&he_id| {
                    let edge_id = dcel.half_edge(he_id).edge;
                    let [fwd, _bwd] = dcel.edge(edge_id).half_edges;
                    let is_fwd = fwd == he_id;
                    (edge_id, is_fwd)
                })
                .collect();

            for (edge_id, is_fwd) in outgoing {
                let curve = &mut dcel.edge_mut(edge_id).curve;
                if is_fwd {
                    // This vertex is the origin of the forward half-edge (p0)
                    let old_p0 = curve.p0;
                    let delta = new_pos - old_p0;
                    curve.p0 = new_pos;
                    curve.p1 = curve.p1 + delta;
                } else {
                    // This vertex is the origin of the backward half-edge (p3)
                    let old_p3 = curve.p3;
                    let delta = new_pos - old_p3;
                    curve.p3 = new_pos;
                    curve.p2 = curve.p2 + delta;
                }
            }

            vid
        };

        // Rebuild fans at the moved vertex and its neighbors — the fix under test
        dcel.rebuild_vertex_fan(moved_vertex);
        for &he_id in &dcel.vertex_outgoing(moved_vertex) {
            let edge_id = dcel.half_edge(he_id).edge;
            let [fwd, bwd] = dcel.edge(edge_id).half_edges;
            let neighbor = if dcel.half_edge(fwd).origin == moved_vertex {
                dcel.half_edge(bwd).origin
            } else {
                dcel.half_edge(fwd).origin
            };
            dcel.rebuild_vertex_fan(neighbor);
        }

        // Recompute intersections on connected edges
        let connected_edges: Vec<_> = dcel.vertex_outgoing(moved_vertex)
            .iter()
            .map(|&he_id| dcel.half_edge(he_id).edge)
            .collect();
        for eid in connected_edges {
            dcel.recompute_edge_intersections(eid);
        }

        dcel.validate();

        // Now insert a stroke across — this would crash with stale fan ordering
        let stroke = CubicBez::new(
            Point::new(-50.0, 50.0), Point::new(16.0, 50.0),
            Point::new(83.0, 50.0), Point::new(200.0, 50.0),
        );
        let _stroke_result = dcel.insert_stroke(&[stroke], None, None, 1.0);

        dcel.validate();
    }

    #[test]
    fn self_intersection_splits_face() {
        use crate::shape::ShapeColor;

        let mut dcel = Dcel::new();

        // Build a rectangle and create a filled face
        let r = 100.0;
        let segs = [
            CubicBez::new(
                Point::new(0.0, 0.0), Point::new(r / 3.0, 0.0),
                Point::new(2.0 * r / 3.0, 0.0), Point::new(r, 0.0),
            ),
            CubicBez::new(
                Point::new(r, 0.0), Point::new(r, r / 3.0),
                Point::new(r, 2.0 * r / 3.0), Point::new(r, r),
            ),
            CubicBez::new(
                Point::new(r, r), Point::new(2.0 * r / 3.0, r),
                Point::new(r / 3.0, r), Point::new(0.0, r),
            ),
            CubicBez::new(
                Point::new(0.0, r), Point::new(0.0, 2.0 * r / 3.0),
                Point::new(0.0, r / 3.0), Point::new(0.0, 0.0),
            ),
        ];

        let rect_result = dcel.insert_stroke(&segs, None, None, 1.0);

        let first_edge = rect_result.new_edges[0];
        let [he_a, he_b] = dcel.edge(first_edge).half_edges;
        let interior_he = if dcel.cycle_signed_area(he_a) > 0.0 { he_a } else { he_b };
        let face = dcel.create_face_at_cycle(interior_he);
        dcel.faces[face.idx()].fill_color = Some(ShapeColor::rgb(0, 0, 255));

        dcel.validate();

        // Replace the bottom edge curve with one that self-intersects.
        // The known-working loop: p0=(0,0), p1=(200,100), p2=(-100,100), p3=(100,0)
        // Bottom edge goes (0,0)→(100,0), same x-range, both y=0.
        let bottom_edge = rect_result.new_edges[0];
        dcel.edges[bottom_edge.idx()].curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(200.0, 100.0),
            Point::new(-100.0, 100.0),
            Point::new(r, 0.0),
        );
        // Verify the curve actually self-intersects
        assert!(
            Dcel::find_cubic_self_intersection(&dcel.edges[bottom_edge.idx()].curve).is_some(),
            "test curve should self-intersect",
        );

        // Recompute intersections — should detect self-intersection and split
        let _created = dcel.recompute_edge_intersections(bottom_edge);

        // Should now have more faces because the self-intersection created a loop
        let non_f0_faces: Vec<_> = dcel
            .faces
            .iter()
            .enumerate()
            .filter(|(i, f)| *i != 0 && !f.deleted)
            .collect();

        // The loop should have been detected and either:
        // - Created as a new face (if positive area in F0)
        // - Split the existing face (if same face had 2 cycles)
        assert!(
            non_f0_faces.len() >= 2,
            "expected at least 2 non-F0 faces after self-intersection split, got {}",
            non_f0_faces.len()
        );

        dcel.validate();
    }
}
