//! DCEL import/extract: merge a serialized DCEL subgraph into a live DCEL,
//! or extract a subgraph from selected edges for clipboard copy.
//!
//! Used by paste to insert clipboard geometry into the current layer,
//! and by copy to extract a sub-DCEL from the select-tool selection.

use super::{Dcel, EdgeId, FaceId};
use crate::shape::FillRule;
use kurbo::{BezPath, CubicBez, Point, Shape, Vec2};
use std::collections::HashSet;

impl Dcel {
    /// Import all non-deleted geometry from `source` into `self` using proper
    /// topological integration.
    ///
    /// **Phase 1** — build a closed `BezPath` (shifted by `offset`) for every
    /// filled face in `source`.
    ///
    /// **Phase 2** — insert each clipboard edge via `insert_stroke`, which
    /// handles intersections with existing edges, vertex snapping, and all
    /// topological invariants.
    ///
    /// **Phase 3** — apply fill colours.  Rather than computing an interior
    /// point for each live DCEL face (which fails for concave faces), we sample
    /// a dense grid of points *inside* each clipboard `BezPath` and call
    /// `find_face_at_point` on each.  Every live face hit by at least one
    /// sample point gets the clipboard fill colour.  This correctly handles
    /// the case where one clipboard face is split into several sub-faces at
    /// intersection points.
    ///
    /// `offset` is a translation applied to all positions (`Vec2::ZERO` for an
    /// exact in-place copy).
    pub fn import_from(&mut self, source: &Dcel, offset: Vec2) {
        // ── Phase 1: clipboard face → offset BezPath ─────────────────────────
        let mut fill_targets: Vec<(BezPath, super::ShapeColor, FillRule)> = Vec::new();
        for (face_idx, face) in source.faces.iter().enumerate() {
            if face.deleted || face_idx == 0 {
                continue;
            }
            let Some(color) = face.fill_color else { continue };
            if let Some(path) =
                clipboard_face_to_bezpath(source, FaceId(face_idx as u32), offset)
            {
                fill_targets.push((path, color, face.fill_rule));
            }
        }

        // ── Phase 2: insert each clipboard edge as a topologically-integrated stroke ──
        // Record the face count before insertion so Phase 3 can distinguish old
        // faces (keep their colour) from new faces (receive clipboard colour).
        let faces_count_before = self.faces.len();

        for edge in &source.edges {
            if edge.deleted {
                continue;
            }
            let c = edge.curve;
            let shifted = CubicBez::new(
                Point::new(c.p0.x + offset.x, c.p0.y + offset.y),
                Point::new(c.p1.x + offset.x, c.p1.y + offset.y),
                Point::new(c.p2.x + offset.x, c.p2.y + offset.y),
                Point::new(c.p3.x + offset.x, c.p3.y + offset.y),
            );
            self.insert_stroke(
                &[shifted],
                edge.stroke_style.clone(),
                edge.stroke_color,
                super::DEFAULT_SNAP_EPSILON,
            );
        }

        // ── Phase 3: grid-sample each clipboard BezPath, paint hit live faces ─
        //
        // For each clipboard face boundary (already offset), lay an N×N grid
        // over its bounding box.  Every grid point inside the path (non-zero
        // winding) is handed to `find_face_at_point`.
        //
        // Two cases for the returned face:
        //
        //  a) Non-F0 face created during Phase 2 (new sub-face of an existing
        //     face that got split by a clipboard edge).  These have face index
        //     >= `faces_count_before` and receive the clipboard colour.
        //
        //  b) Still in F0 (unbounded face) — this happens when the clipboard
        //     region extends *outside* all pre-existing faces.  The topology
        //     code does not auto-create faces inside F0.  In this case we use
        //     the `cycle_he` returned by `find_face_at_point` to explicitly
        //     claim that finite F0 cycle as a new face, then colour it.
        //
        // Old faces (index < `faces_count_before`) are left untouched so that
        // pre-existing geometry keeps its original colour.
        for (fill_path, color, fill_rule) in &fill_targets {
            let bbox = fill_path.bounding_box();
            if bbox.width() < 1e-9 || bbox.height() < 1e-9 {
                continue;
            }

            let mut painted: HashSet<u32> = HashSet::new();

            // N×N interior grid (avoid exact boundary lines).
            const N: usize = 8;
            for iy in 1..=N {
                for ix in 1..=N {
                    let x = bbox.min_x()
                        + (ix as f64 / (N + 1) as f64) * bbox.width();
                    let y = bbox.min_y()
                        + (iy as f64 / (N + 1) as f64) * bbox.height();
                    let pt = Point::new(x, y);

                    // Only consider points confirmed inside the clipboard face.
                    if fill_path.winding(pt) == 0 {
                        continue;
                    }

                    let fq = self.find_face_at_point(pt);
                    let mut fid = fq.face;

                    // Case (b): point is in the unbounded face but inside the
                    // clipboard BezPath.  The pasted edge(s) already form a
                    // closed cycle in F0 — claim it as a new face now.
                    if fid.0 == 0 && !fq.cycle_he.is_none() {
                        let new_face = self.alloc_face();
                        self.assign_cycle_face(fq.cycle_he, new_face);
                        self.faces[new_face.idx()].outer_half_edge = fq.cycle_he;
                        fid = new_face;
                    }

                    if fid.is_none() || fid.0 == 0 {
                        continue;
                    }

                    // Only paint faces that were created during this paste.
                    // Faces that existed before paste keep their original fill.
                    if fid.idx() < faces_count_before {
                        continue;
                    }

                    if painted.insert(fid.0) {
                        self.faces[fid.idx()].fill_color = Some(*color);
                        self.faces[fid.idx()].fill_rule = *fill_rule;
                    }
                }
            }
        }
    }
}

/// Build a closed `BezPath` for the outer boundary of a clipboard face,
/// with all positions shifted by `offset`.
///
/// Half-edges are walked CCW (DCEL convention: face is to the left of each
/// directed edge).  Returns `None` if the face has no outer boundary.
fn clipboard_face_to_bezpath(source: &Dcel, face_id: FaceId, offset: Vec2) -> Option<BezPath> {
    let start_he = source.face(face_id).outer_half_edge;
    if start_he.is_none() {
        return None;
    }

    let mut path = BezPath::new();
    let mut first = true;
    let mut he_id = start_he;
    let limit = source.half_edges.len() + 1;

    for _ in 0..limit {
        let he = source.half_edge(he_id);
        if he.deleted {
            break;
        }
        let edge = source.edge(he.edge);
        let c = edge.curve;

        // Forward half-edge → curve goes p0→p3; backward → reversed.
        let (p0, p1, p2, p3) = if edge.half_edges[0] == he_id {
            (c.p0, c.p1, c.p2, c.p3)
        } else {
            (c.p3, c.p2, c.p1, c.p0)
        };

        let shift = |p: Point| Point::new(p.x + offset.x, p.y + offset.y);

        if first {
            path.move_to(shift(p0));
            first = false;
        }
        path.curve_to(shift(p1), shift(p2), shift(p3));

        he_id = he.next;
        if he_id == start_he {
            break;
        }
    }

    if first {
        return None;
    }
    path.close_path();
    Some(path)
}

// ── Extract faces for select-tool copy ───────────────────────────────────────

/// Extract a sub-DCEL containing the faces adjacent to the given edges.
///
/// Includes all selected edges, both adjacent faces, and all boundary edges
/// of those faces.  Used by copy when the select tool has selected strokes.
pub fn extract_faces_for_edges(dcel: &Dcel, edge_ids: &HashSet<EdgeId>) -> Dcel {
    if edge_ids.is_empty() {
        return Dcel::new();
    }

    let mut face_set: HashSet<u32> = HashSet::new();
    for &eid in edge_ids {
        if eid.is_none() || dcel.edge(eid).deleted {
            continue;
        }
        let [he_fwd, he_bwd] = dcel.edge(eid).half_edges;
        for he_id in [he_fwd, he_bwd] {
            if !he_id.is_none() {
                let face = dcel.half_edge(he_id).face;
                if !face.is_none() && face.0 != 0 {
                    face_set.insert(face.0);
                }
            }
        }
    }

    if face_set.is_empty() {
        return Dcel::new();
    }

    let mut boundary_edge_ids: HashSet<u32> = HashSet::new();
    for (i, edge) in dcel.edges.iter().enumerate() {
        if edge.deleted {
            continue;
        }
        let [he_fwd, he_bwd] = edge.half_edges;
        let face_fwd =
            if !he_fwd.is_none() { dcel.half_edge(he_fwd).face } else { FaceId::NONE };
        let face_bwd =
            if !he_bwd.is_none() { dcel.half_edge(he_bwd).face } else { FaceId::NONE };
        if (!face_fwd.is_none() && face_set.contains(&face_fwd.0))
            || (!face_bwd.is_none() && face_set.contains(&face_bwd.0))
        {
            boundary_edge_ids.insert(i as u32);
        }
    }

    let mut extracted = dcel.clone();

    let to_remove: Vec<EdgeId> = extracted
        .edges
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            if !e.deleted && !boundary_edge_ids.contains(&(i as u32)) {
                Some(EdgeId(i as u32))
            } else {
                None
            }
        })
        .collect();

    for eid in to_remove {
        if !extracted.edges[eid.idx()].deleted {
            extracted.remove_edge(eid);
        }
    }

    for (i, face) in extracted.faces.iter_mut().enumerate() {
        if i == 0 || face.deleted {
            continue;
        }
        if !face_set.contains(&(i as u32)) {
            face.fill_color = None;
            face.image_fill = None;
        }
    }

    extracted
}
