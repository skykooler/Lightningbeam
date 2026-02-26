//! Region extraction from the DCEL.
//!
//! `extract_region` splits a DCEL along a closed boundary path: the inside
//! portion is returned as a new DCEL, the outside portion stays in `self`.
//! Boundary edges are kept in both.
//!
//! Vertex classification is deterministic: boundary vertices are known from
//! inserting the region stroke, all others are classified by winding number.
//! Faces are classified by which vertices they touch — no sampling needed.

use super::{Dcel, EdgeId, FaceId, VertexId};
use kurbo::{BezPath, Point, Shape};

/// Vertex classification relative to the region boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum VClass {
    Inside,
    Outside,
    Boundary,
}

impl Dcel {
    /// Extract the sub-DCEL inside a closed region path.
    ///
    /// The caller must have already inserted the region boundary via
    /// `insert_stroke`, passing the resulting vertex IDs as `boundary_vertices`.
    /// All other vertices are classified by winding number against `region`.
    ///
    /// Returns the extracted (inside) DCEL. `self` is modified to contain
    /// only the outside portion. Boundary edges appear in both.
    pub fn extract_region(
        &mut self,
        region: &BezPath,
        boundary_vertices: &[VertexId],
    ) -> Dcel {
        let classifications = self.classify_vertices(region, boundary_vertices);

        // Clone → extracted
        let mut extracted = self.clone();

        // In extracted: remove edges where either endpoint is Outside
        let to_remove: Vec<EdgeId> = extracted
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, edge)| {
                if edge.deleted { return None; }
                let [fwd, bwd] = edge.half_edges;
                let v1 = extracted.half_edges[fwd.idx()].origin;
                let v2 = extracted.half_edges[bwd.idx()].origin;
                if classifications[v1.idx()] == VClass::Outside
                    || classifications[v2.idx()] == VClass::Outside
                {
                    Some(EdgeId(i as u32))
                } else {
                    None
                }
            })
            .collect();

        for edge_id in to_remove {
            if !extracted.edges[edge_id.idx()].deleted {
                extracted.remove_edge(edge_id);
            }
        }

        // In self: remove edges where either endpoint is Inside
        let to_remove: Vec<EdgeId> = self
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, edge)| {
                if edge.deleted { return None; }
                let [fwd, bwd] = edge.half_edges;
                let v1 = self.half_edges[fwd.idx()].origin;
                let v2 = self.half_edges[bwd.idx()].origin;
                if classifications[v1.idx()] == VClass::Inside
                    || classifications[v2.idx()] == VClass::Inside
                {
                    Some(EdgeId(i as u32))
                } else {
                    None
                }
            })
            .collect();

        for edge_id in to_remove {
            if !self.edges[edge_id.idx()].deleted {
                self.remove_edge(edge_id);
            }
        }

        extracted
    }

    /// Classify every vertex as Inside, Outside, or Boundary.
    fn classify_vertices(
        &self,
        region: &BezPath,
        boundary_vertices: &[VertexId],
    ) -> Vec<VClass> {
        self.vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                if v.deleted {
                    return VClass::Outside;
                }
                let vid = VertexId(i as u32);
                if boundary_vertices.contains(&vid) {
                    VClass::Boundary
                } else if region.winding(v.position) != 0 {
                    VClass::Inside
                } else {
                    VClass::Outside
                }
            })
            .collect()
    }

    /// Copy fill properties from `snapshot` to faces in `self` that lost
    /// them when the region boundary split filled faces.
    ///
    /// For each unfilled face, walks its boundary to find an Inside vertex,
    /// then looks up the snapshot face at that vertex's position to inherit
    /// the fill. No sampling heuristic — vertex positions are exact.
    pub fn propagate_fills(
        &mut self,
        snapshot: &Dcel,
        region: &BezPath,
        boundary_vertices: &[VertexId],
    ) {
        let classifications = self.classify_vertices(region, boundary_vertices);

        for i in 1..self.faces.len() {
            let face = &self.faces[i];
            if face.deleted || face.outer_half_edge.is_none() {
                continue;
            }
            if face.fill_color.is_some() || face.image_fill.is_some() {
                continue;
            }

            let face_id = FaceId(i as u32);
            let boundary = self.face_boundary(face_id);

            // Find an inside vertex on this face's boundary
            let probe = boundary.iter().find_map(|&he_id| {
                let vid = self.half_edges[he_id.idx()].origin;
                if classifications[vid.idx()] == VClass::Inside {
                    Some(self.vertices[vid.idx()].position)
                } else {
                    None
                }
            });

            let probe_point = match probe {
                Some(p) => p,
                None => continue, // face has no inside vertices — skip
            };

            let snap_face_id = snapshot.find_face_containing_point(probe_point);
            if snap_face_id.0 == 0 {
                continue;
            }
            let snap_face = &snapshot.faces[snap_face_id.idx()];
            if snap_face.fill_color.is_some() || snap_face.image_fill.is_some() {
                self.faces[i].fill_color = snap_face.fill_color.clone();
                self.faces[i].image_fill = snap_face.image_fill;
                self.faces[i].fill_rule = snap_face.fill_rule;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{CubicBez, Point};

    fn line_cubic(a: Point, b: Point) -> CubicBez {
        CubicBez::new(
            a,
            Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
            Point::new(
                a.x + 2.0 * (b.x - a.x) / 3.0,
                a.y + 2.0 * (b.y - a.y) / 3.0,
            ),
            b,
        )
    }

    #[test]
    fn extract_region_basic() {
        let mut dcel = Dcel::new();

        // Two horizontal lines crossing the region boundary:
        //   line A at y=30: (0,30) → (100,30)
        //   line B at y=70: (0,70) → (100,70)
        let a0 = Point::new(0.0, 30.0);
        let a1 = Point::new(100.0, 30.0);
        let b0 = Point::new(0.0, 70.0);
        let b1 = Point::new(100.0, 70.0);

        let va0 = dcel.alloc_vertex(a0);
        let va1 = dcel.alloc_vertex(a1);
        let vb0 = dcel.alloc_vertex(b0);
        let vb1 = dcel.alloc_vertex(b1);

        dcel.insert_edge(va0, va1, FaceId(0), line_cubic(a0, a1));
        dcel.insert_edge(vb0, vb1, FaceId(0), line_cubic(b0, b1));

        assert_eq!(dcel.edges.iter().filter(|e| !e.deleted).count(), 2);

        // Region covers the left half: x ∈ [-10, 50]
        let mut region = BezPath::new();
        region.move_to(Point::new(-10.0, -10.0));
        region.line_to(Point::new(50.0, -10.0));
        region.line_to(Point::new(50.0, 110.0));
        region.line_to(Point::new(-10.0, 110.0));
        region.close_path();

        // va0, vb0 are inside (x=0), va1, vb1 are outside (x=100)
        // No boundary vertices in this simple test
        let extracted = dcel.extract_region(&region, &[]);

        // Both edges have one inside and one outside endpoint,
        // so both are removed from both halves
        let self_edges = dcel.edges.iter().filter(|e| !e.deleted).count();
        let ext_edges = extracted.edges.iter().filter(|e| !e.deleted).count();
        assert_eq!(self_edges, 0, "edges span boundary → removed from self");
        assert_eq!(ext_edges, 0, "edges span boundary → removed from extracted");
    }

    #[test]
    fn extract_region_with_boundary_vertices() {
        let mut dcel = Dcel::new();

        // Build a horizontal line that will be split by the region boundary.
        // We simulate what happens after insert_stroke splits it:
        //   left piece: (0,50) → (50,50)    [inside → boundary]
        //   right piece: (50,50) → (100,50)  [boundary → outside]
        let p_left = Point::new(0.0, 50.0);
        let p_mid = Point::new(50.0, 50.0);
        let p_right = Point::new(100.0, 50.0);

        let v_left = dcel.alloc_vertex(p_left);
        let v_mid = dcel.alloc_vertex(p_mid);
        let v_right = dcel.alloc_vertex(p_right);

        dcel.insert_edge(v_left, v_mid, FaceId(0), line_cubic(p_left, p_mid));
        dcel.insert_edge(v_mid, v_right, FaceId(0), line_cubic(p_mid, p_right));

        // Region: left half (x < 50)
        let mut region = BezPath::new();
        region.move_to(Point::new(-10.0, -10.0));
        region.line_to(Point::new(50.0, -10.0));
        region.line_to(Point::new(50.0, 110.0));
        region.line_to(Point::new(-10.0, 110.0));
        region.close_path();

        // v_mid is on the boundary
        let extracted = dcel.extract_region(&region, &[v_mid]);

        // Left edge: inside → boundary → kept in extracted
        // Right edge: boundary → outside → kept in self
        let ext_edges = extracted.edges.iter().filter(|e| !e.deleted).count();
        let self_edges = dcel.edges.iter().filter(|e| !e.deleted).count();
        assert_eq!(ext_edges, 1, "extracted should have left edge");
        assert_eq!(self_edges, 1, "self should have right edge");
    }
}
