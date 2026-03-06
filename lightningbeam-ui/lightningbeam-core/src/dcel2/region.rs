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
use kurbo::{BezPath, CubicBez, Point, Shape};

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

    /// Merge a modified `selected` DCEL back into `self` (the snapshot DCEL).
    ///
    /// Call this when deselecting after uncommitted modifications (e.g. a move).
    /// `self` is the snapshot; inside-vertex positions and edge curves are
    /// updated from `selected`, then the invisible region boundary edges are
    /// removed to restore a unified DCEL.
    pub fn merge_back_from_selected(
        &mut self,
        selected: &Dcel,
        inside_vertices: &[VertexId],
        _boundary_vertices: &[VertexId],
        region_edge_ids: &[EdgeId],
    ) {
        // Step 1: Copy inside vertex positions from selected_dcel.
        for &vid in inside_vertices {
            if vid.idx() < selected.vertices.len() && !selected.vertices[vid.idx()].deleted {
                if vid.idx() < self.vertices.len() && !self.vertices[vid.idx()].deleted {
                    self.vertices[vid.idx()].position = selected.vertices[vid.idx()].position;
                }
            }
        }

        // Step 2: Update curves for edges whose endpoints changed.
        let inside_set: std::collections::HashSet<VertexId> =
            inside_vertices.iter().copied().collect();

        for i in 0..self.edges.len() {
            let e = &self.edges[i];
            if e.deleted { continue; }
            let [fwd, bwd] = e.half_edges;
            let v1 = self.half_edges[fwd.idx()].origin;
            let v2 = self.half_edges[bwd.idx()].origin;
            let v1_inside = inside_set.contains(&v1);
            let v2_inside = inside_set.contains(&v2);

            if v1_inside && v2_inside {
                // Fully inside — copy transformed curve from selected.
                if i < selected.edges.len() && !selected.edges[i].deleted {
                    self.edges[i].curve = selected.edges[i].curve;
                }
            } else if v1_inside || v2_inside {
                // Mixed edge: one endpoint moved, one is boundary (fixed).
                // Approximate with a straight line to preserve connectivity.
                let p0 = self.vertices[v1.idx()].position;
                let p3 = self.vertices[v2.idx()].position;
                self.edges[i].curve = line_to_cubic_pts(p0, p3);
            }
        }

        // Step 3: Remove invisible region boundary edges to heal face splits.
        for &eid in region_edge_ids {
            if eid.idx() < self.edges.len() && !self.edges[eid.idx()].deleted {
                self.remove_edge(eid);
            }
        }

        // Step 4: Clean up stale inner half-edge references left by remove_edge.
        self.repair_stale_inner_half_edges();
    }

    /// Remove stale `inner_half_edges` entries that point to deleted half-edges.
    ///
    /// After `remove_edge` calls, some faces' hole-boundary references can point
    /// to half-edges that no longer exist. This cleans them up.
    pub fn repair_stale_inner_half_edges(&mut self) {
        for face in self.faces.iter_mut() {
            face.inner_half_edges.retain(|&he| {
                he.idx() < self.half_edges.len() && !self.half_edges[he.idx()].deleted
            });
        }
    }
}

/// Convert two points into a degenerate cubic (straight-line cubic bezier).
fn line_to_cubic_pts(p0: Point, p3: Point) -> CubicBez {
    CubicBez::new(
        p0,
        Point::new(p0.x + (p3.x - p0.x) / 3.0, p0.y + (p3.y - p0.y) / 3.0),
        Point::new(p0.x + 2.0 * (p3.x - p0.x) / 3.0, p0.y + 2.0 * (p3.y - p0.y) / 3.0),
        p3,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{CubicBez, Point};

    /// After `extract_region`, verify no face in `dcel` has half-edges from
    /// both Inside-classified and Outside-classified vertices.
    fn assert_no_face_straddles(dcel: &Dcel, region: &BezPath, boundary_verts: &[VertexId]) {
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        enum VClass { Inside, Outside, Boundary }

        let classes: Vec<VClass> = dcel.vertices.iter().enumerate().map(|(i, v)| {
            if v.deleted { return VClass::Outside; }
            let vid = VertexId(i as u32);
            if boundary_verts.contains(&vid) {
                VClass::Boundary
            } else if region.winding(v.position) != 0 {
                VClass::Inside
            } else {
                VClass::Outside
            }
        }).collect();

        for i in 1..dcel.faces.len() {
            let face = &dcel.faces[i];
            if face.deleted || face.outer_half_edge.is_none() { continue; }
            let boundary = dcel.face_boundary(FaceId(i as u32));
            let mut has_inside = false;
            let mut has_outside = false;
            for he_id in &boundary {
                let vid = dcel.half_edges[he_id.idx()].origin;
                match classes[vid.idx()] {
                    VClass::Inside => has_inside = true,
                    VClass::Outside => has_outside = true,
                    VClass::Boundary => {}
                }
            }
            assert!(
                !(has_inside && has_outside),
                "face {} straddles boundary (has both Inside and Outside vertices)",
                i
            );
        }
    }

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
    fn extract_region_no_face_straddles() {
        let mut dcel = Dcel::new();

        // Two horizontal lines spanning the region boundary.
        // After insert_stroke splits them, boundary vertices appear at x=50.
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

        let mut region = BezPath::new();
        region.move_to(Point::new(-10.0, -10.0));
        region.line_to(Point::new(50.0, -10.0));
        region.line_to(Point::new(50.0, 110.0));
        region.line_to(Point::new(-10.0, 110.0));
        region.close_path();

        // Simulate boundary splitting: mid-points at x=50
        let v_amid = dcel.alloc_vertex(Point::new(50.0, 30.0));
        let v_bmid = dcel.alloc_vertex(Point::new(50.0, 70.0));
        let boundary_verts = vec![v_amid, v_bmid];

        let extracted = dcel.extract_region(&region, &boundary_verts);

        assert_no_face_straddles(&dcel, &region, &boundary_verts);
        assert_no_face_straddles(&extracted, &region, &boundary_verts);
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

    /// Replicate: draw filled rect (0,0)-(100,100), region-select top-left corner
    /// with selection (-50,-50)-(50,50). Goal: validate passes on both DCELs.
    #[test]
    fn rect_corner_region_select() {
        let mut dcel = Dcel::new();

        // Draw rectangle: W=(0,0) X=(100,0) Y=(100,100) Z=(0,100)
        let w = Point::new(0.0, 0.0);
        let x = Point::new(100.0, 0.0);
        let y = Point::new(100.0, 100.0);
        let z = Point::new(0.0, 100.0);

        dcel.insert_stroke(&[
            line_cubic(w, x),
            line_cubic(x, y),
            line_cubic(y, z),
            line_cubic(z, w),
        ], None, None, 1.0);

        // Simulate paint-bucket: find the inner face cycle and create F1
        let face_query = dcel.find_face_at_point(Point::new(50.0, 50.0));
        eprintln!("paint bucket face query: {:?}, he: {:?}", face_query.face, face_query.cycle_he);
        if !face_query.cycle_he.is_none() && face_query.face.0 == 0 {
            let f1 = dcel.create_face_at_cycle(face_query.cycle_he);
            eprintln!("created F{} for interior", f1.0);
        }

        dcel.validate();
        eprintln!("DCEL valid after rect+paint");

        // Selection boundary: A=(-50,-50) B=(50,-50) C=(50,50) D=(-50,50)
        let a = Point::new(-50.0, -50.0);
        let b = Point::new(50.0, -50.0);
        let c = Point::new(50.0, 50.0);
        let d = Point::new(-50.0, 50.0);

        let mut region_path = BezPath::new();
        region_path.move_to(a);
        region_path.line_to(b);
        region_path.line_to(c);
        region_path.line_to(d);
        region_path.close_path();

        let stroke_result = dcel.insert_stroke(&[
            line_cubic(a, b),
            line_cubic(b, c),
            line_cubic(c, d),
            line_cubic(d, a),
        ], None, None, 1.0);

        eprintln!("after insert_stroke: {} new edges, {} new faces",
            stroke_result.new_edges.len(), stroke_result.new_faces.len());
        dcel.validate();
        eprintln!("DCEL valid after selection boundary insert");

        // Dump face structure after insert_stroke
        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted { continue; }
            let ohe = face.outer_half_edge;
            if ohe.is_none() {
                eprintln!("  F{i}: outer_he=NONE (unbounded)");
            } else {
                let cycle = dcel.walk_cycle(ohe);
                let origins: Vec<_> = cycle.iter().map(|&he| {
                    let vid = dcel.half_edges[he.idx()].origin;
                    dcel.vertices[vid.idx()].position
                }).collect();
                eprintln!("  F{i}: {} HEs, vertices: {:?}", cycle.len(), origins);
            }
        }

        let boundary_verts: Vec<VertexId> = stroke_result.new_vertices.clone();
        let mut extracted = dcel.extract_region(&region_path, &boundary_verts);

        eprintln!("after extract_region");
        // Check face_boundary on all non-F0 faces
        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            let b = dcel.face_boundary(FaceId(i as u32));
            eprintln!("  self F{i}: {} boundary HEs", b.len());
        }
        for (i, face) in extracted.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            let b = extracted.face_boundary(FaceId(i as u32));
            eprintln!("  extracted F{i}: {} boundary HEs", b.len());
        }
        dcel.validate();
        extracted.validate();
        eprintln!("both DCELs valid after extract_region");
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

    /// Replicate: two rectangles, region-select an area that doesn't intersect either.
    /// The selected area is to the left of both rectangles, selecting empty space.
    /// Goal: validate passes on both DCELs and walk_cycle doesn't infinite-loop.
    #[test]
    fn rect_empty_region_select() {
        let mut dcel = Dcel::new();

        // Rect 1: (220,353)-(290,379)
        let r1a = Point::new(220.0, 353.0);
        let r1b = Point::new(290.0, 353.0);
        let r1c = Point::new(290.0, 379.0);
        let r1d = Point::new(220.0, 379.0);
        dcel.insert_stroke(&[
            line_cubic(r1a, r1b), line_cubic(r1b, r1c),
            line_cubic(r1c, r1d), line_cubic(r1d, r1a),
        ], None, None, 1.0);

        // Rect 2: (304,383)-(551,483)
        let r2a = Point::new(304.0, 383.0);
        let r2b = Point::new(551.0, 383.0);
        let r2c = Point::new(551.0, 483.0);
        let r2d = Point::new(304.0, 483.0);
        dcel.insert_stroke(&[
            line_cubic(r2a, r2b), line_cubic(r2b, r2c),
            line_cubic(r2c, r2d), line_cubic(r2d, r2a),
        ], None, None, 1.0);

        dcel.validate();

        // Selection: (136,332)-(208,347) — entirely to the left of both rectangles
        let sel_min = Point::new(136.0, 332.0);
        let sel_max = Point::new(208.0, 347.0);
        let a = sel_min;
        let b = Point::new(sel_max.x, sel_min.y);
        let c = sel_max;
        let d = Point::new(sel_min.x, sel_max.y);

        let mut region_path = BezPath::new();
        region_path.move_to(a);
        region_path.line_to(b);
        region_path.line_to(c);
        region_path.line_to(d);
        region_path.close_path();

        let stroke_result = dcel.insert_stroke(&[
            line_cubic(a, b), line_cubic(b, c),
            line_cubic(c, d), line_cubic(d, a),
        ], None, None, 1.0);

        dcel.validate();

        let boundary_verts = stroke_result.new_vertices.clone();
        let extracted = dcel.extract_region(&region_path, &boundary_verts);

        // Walk all face boundaries in both DCELs — must not infinite-loop
        for (i, face) in dcel.faces.iter().enumerate() {
            if face.deleted { continue; }
            let _b = dcel.face_boundary(FaceId(i as u32));
            for &ihe in &face.inner_half_edges {
                if !ihe.is_none() && !dcel.half_edges[ihe.idx()].deleted {
                    let _c = dcel.walk_cycle(ihe);
                }
            }
        }
        for (i, face) in extracted.faces.iter().enumerate() {
            if face.deleted { continue; }
            let _b = extracted.face_boundary(FaceId(i as u32));
            for &ihe in &face.inner_half_edges {
                if !ihe.is_none() && !extracted.half_edges[ihe.idx()].deleted {
                    let _c = extracted.walk_cycle(ihe);
                }
            }
        }

        dcel.validate();
        extracted.validate();
    }

    /// Replicate: multiple consecutive region-selects on two rectangles,
    /// some of which select empty space. Verifies no crash accumulates.
    #[test]
    fn multiple_region_selects() {
        fn make_rect_dcel() -> Dcel {
            let mut dcel = Dcel::new();
            let r1a = Point::new(220.0, 353.0);
            let r1b = Point::new(290.0, 353.0);
            let r1c = Point::new(290.0, 379.0);
            let r1d = Point::new(220.0, 379.0);
            dcel.insert_stroke(&[
                line_cubic(r1a, r1b), line_cubic(r1b, r1c),
                line_cubic(r1c, r1d), line_cubic(r1d, r1a),
            ], None, None, 1.0);
            let r2a = Point::new(304.0, 383.0);
            let r2b = Point::new(551.0, 383.0);
            let r2c = Point::new(551.0, 483.0);
            let r2d = Point::new(304.0, 483.0);
            dcel.insert_stroke(&[
                line_cubic(r2a, r2b), line_cubic(r2b, r2c),
                line_cubic(r2c, r2d), line_cubic(r2d, r2a),
            ], None, None, 1.0);
            dcel
        }

        let selects: &[(f64, f64, f64, f64)] = &[
            (438.0, 326.0, 614.0, 445.0), // intersects rect 2
            (181.0, 312.0, 407.0, 448.0), // intersects both
            (460.0, 293.0, 516.0, 507.0), // intersects rect 2
            (167.0, 311.0, 380.0, 437.0), // intersects both
            (690.0, 360.0, 703.0, 448.0), // empty (right)
            (136.0, 332.0, 208.0, 347.0), // empty (left) — crashes in original bug
        ];

        for (min_x, min_y, max_x, max_y) in selects {
            let mut dcel = make_rect_dcel();
            let a = Point::new(*min_x, *min_y);
            let b = Point::new(*max_x, *min_y);
            let c = Point::new(*max_x, *max_y);
            let d = Point::new(*min_x, *max_y);

            let mut region_path = BezPath::new();
            region_path.move_to(a);
            region_path.line_to(b);
            region_path.line_to(c);
            region_path.line_to(d);
            region_path.close_path();

            let stroke_result = dcel.insert_stroke(&[
                line_cubic(a, b), line_cubic(b, c),
                line_cubic(c, d), line_cubic(d, a),
            ], None, None, 1.0);
            dcel.validate();

            let boundary_verts = stroke_result.new_vertices.clone();
            let extracted = dcel.extract_region(&region_path, &boundary_verts);

            // Walk all face boundaries and inner cycles — must not infinite-loop
            for (i, face) in extracted.faces.iter().enumerate() {
                if face.deleted { continue; }
                let _b = extracted.face_boundary(FaceId(i as u32));
                for &ihe in &face.inner_half_edges {
                    if !ihe.is_none() && !extracted.half_edges[ihe.idx()].deleted {
                        let _ = extracted.walk_cycle(ihe);
                    }
                }
            }
            dcel.validate();
            extracted.validate();
        }
    }
}
