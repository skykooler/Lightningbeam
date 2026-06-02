//! Tests for extract_subgraph and merge_subgraph (region selection support).

use super::super::*;
use kurbo::{Affine, CubicBez, Point};
use std::collections::{HashMap, HashSet};

/// Helper: create a straight-line cubic Bézier from a to b.
fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
        Point::new(a.x + 2.0 * (b.x - a.x) / 3.0, a.y + 2.0 * (b.y - a.y) / 3.0),
        b,
    )
}

/// Build a triangle graph: 3 vertices, 3 edges, 1 fill.
/// Returns (graph, [v0,v1,v2], [e0,e1,e2], fid).
fn triangle_graph() -> (VectorGraph, [VertexId; 3], [EdgeId; 3], FillId) {
    let mut g = VectorGraph::new();
    let p0 = Point::new(0.0, 0.0);
    let p1 = Point::new(100.0, 0.0);
    let p2 = Point::new(50.0, 100.0);

    let v0 = g.alloc_vertex(p0);
    let v1 = g.alloc_vertex(p1);
    let v2 = g.alloc_vertex(p2);

    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(p0, p1), v0, v1, Some(style.clone()), Some(color));
    let e1 = g.alloc_edge(line(p1, p2), v1, v2, Some(style.clone()), Some(color));
    let e2 = g.alloc_edge(line(p2, p0), v2, v0, Some(style), Some(color));

    let boundary = vec![
        (e0, Direction::Forward),
        (e1, Direction::Forward),
        (e2, Direction::Forward),
    ];
    let fid = g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    (g, [v0, v1, v2], [e0, e1, e2], fid)
}

// ── extract_subgraph ─────────────────────────────────────────────────

#[test]
fn extract_empty_region_returns_empty_graph() {
    let (mut g, _, _, _) = triangle_graph();
    let orig_edge_count = g.edges.iter().filter(|e| !e.deleted).count();

    let (new_g, vtx_map, edge_map) = g.extract_subgraph(
        &HashSet::new(),
        &HashSet::new(),
        &HashSet::new(),
    );

    // New graph should be empty
    assert_eq!(new_g.edges.iter().filter(|e| !e.deleted).count(), 0);
    assert_eq!(new_g.fills.iter().filter(|f| !f.deleted).count(), 0);
    assert!(vtx_map.is_empty());
    assert!(edge_map.is_empty());

    // Original should be unchanged
    assert_eq!(g.edges.iter().filter(|e| !e.deleted).count(), orig_edge_count);
}

#[test]
fn extract_single_edge_removes_from_original() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(100.0, 0.0));
    let v2 = g.alloc_vertex(Point::new(200.0, 0.0));

    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(Point::new(0.0, 0.0), Point::new(100.0, 0.0)), v0, v1, Some(style.clone()), Some(color));
    let e1 = g.alloc_edge(line(Point::new(100.0, 0.0), Point::new(200.0, 0.0)), v1, v2, Some(style), Some(color));

    let mut inside = HashSet::new();
    inside.insert(e0);

    let (new_g, vtx_map, edge_map) = g.extract_subgraph(
        &inside,
        &HashSet::new(),
        &HashSet::new(),
    );

    // e0 should be extracted
    assert!(g.edge(e0).deleted, "extracted edge should be freed from original");
    assert!(!g.edge(e1).deleted, "non-extracted edge should remain");

    // v0 is interior (only connected to e0), should be freed
    assert!(g.vertex(v0).deleted, "interior vertex should be freed");
    // v1 is shared (connected to e1 too), should NOT be freed
    assert!(!g.vertex(v1).deleted, "shared vertex should remain");

    // New graph should have the edge
    assert_eq!(new_g.edges.iter().filter(|e| !e.deleted).count(), 1);
    assert!(edge_map.contains_key(&e0));

    // New graph should have 2 vertices (v0 and v1 mapped)
    assert_eq!(new_g.vertices.iter().filter(|v| !v.deleted).count(), 2);
    assert!(vtx_map.contains_key(&v0));
    assert!(vtx_map.contains_key(&v1));
}

#[test]
fn extract_fill_duplicates_boundary_edges() {
    let (mut g, verts, edges, fid) = triangle_graph();

    // Pretend e0 is a boundary edge (from region selection insert_stroke)
    let mut boundary_edges = HashSet::new();
    boundary_edges.insert(edges[0]);

    // e1 and e2 are "inside"
    let mut inside_edges = HashSet::new();
    inside_edges.insert(edges[1]);
    inside_edges.insert(edges[2]);

    let mut inside_fills = HashSet::new();
    inside_fills.insert(fid);

    let (new_g, vtx_map, edge_map) = g.extract_subgraph(
        &inside_edges,
        &inside_fills,
        &boundary_edges,
    );

    // Boundary edge e0 should still exist in original (duplicated, not removed)
    assert!(!g.edge(edges[0]).deleted, "boundary edge should remain in original");

    // Inside edges should be removed from original
    assert!(g.edge(edges[1]).deleted, "inside edge should be freed from original");
    assert!(g.edge(edges[2]).deleted, "inside edge should be freed from original");

    // New graph should have 3 edges: e0 (boundary copy) + e1 + e2
    assert_eq!(new_g.edges.iter().filter(|e| !e.deleted).count(), 3);

    // New graph should have 1 fill
    assert_eq!(new_g.fills.iter().filter(|f| !f.deleted).count(), 1);

    // The fill's boundary in new graph should reference remapped edges
    let new_fill = &new_g.fills[0];
    assert_eq!(new_fill.boundary.len(), 3);
    for &(eid, _) in &new_fill.boundary {
        assert!(!eid.is_none(), "fill boundary should have valid edge IDs");
    }

    // Fill color should be preserved
    assert_eq!(new_fill.color, Some(ShapeColor::rgb(255, 0, 0)));
}

// ── merge_subgraph ───────────────────────────────────────────────────

#[test]
fn merge_round_trip_identity_restores_edges() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(100.0, 0.0));
    let v2 = g.alloc_vertex(Point::new(200.0, 0.0));

    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(Point::new(0.0, 0.0), Point::new(100.0, 0.0)), v0, v1, Some(style.clone()), Some(color));
    let _e1 = g.alloc_edge(line(Point::new(100.0, 0.0), Point::new(200.0, 0.0)), v1, v2, Some(style), Some(color));

    let mut inside = HashSet::new();
    inside.insert(e0);

    let (new_g, vtx_map, edge_map) = g.extract_subgraph(
        &inside,
        &HashSet::new(),
        &HashSet::new(),
    );

    // Build boundary vertex map (reverse of vtx_map, only for non-deleted vertices in g)
    let boundary_vtx_map: HashMap<VertexId, VertexId> = vtx_map.iter()
        .filter(|(&old, _)| !g.vertex(old).deleted)
        .map(|(&old, &new)| (new, old))
        .collect();

    // Merge back with identity transform
    g.merge_subgraph(&new_g, Affine::IDENTITY, &boundary_vtx_map, &HashMap::new());

    // Should have 2 non-deleted edges again
    let live_edges = g.edges.iter().filter(|e| !e.deleted).count();
    assert_eq!(live_edges, 2, "should have 2 edges after merge-back");

    // Should have 3 vertices (v0 was freed then re-added)
    let live_verts = g.vertices.iter().filter(|v| !v.deleted).count();
    assert_eq!(live_verts, 3, "should have 3 vertices after merge-back");
}

#[test]
fn merge_with_translation_moves_geometry() {
    let mut g = VectorGraph::new();
    let v0 = g.alloc_vertex(Point::new(0.0, 0.0));
    let v1 = g.alloc_vertex(Point::new(100.0, 0.0));

    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let e0 = g.alloc_edge(line(Point::new(0.0, 0.0), Point::new(100.0, 0.0)), v0, v1, Some(style), Some(color));

    let mut inside = HashSet::new();
    inside.insert(e0);

    let (new_g, _vtx_map, _edge_map) = g.extract_subgraph(
        &inside,
        &HashSet::new(),
        &HashSet::new(),
    );

    // Merge back with a translation of (50, 50)
    let transform = Affine::translate((50.0, 50.0));
    g.merge_subgraph(&new_g, transform, &HashMap::new(), &HashMap::new());

    // The merged edge's vertices should be at (50,50) and (150,50)
    let merged_edge = g.edges.iter().find(|e| !e.deleted).unwrap();
    let v0_pos = g.vertices[merged_edge.vertices[0].idx()].position;
    let v1_pos = g.vertices[merged_edge.vertices[1].idx()].position;

    assert!((v0_pos.x - 50.0).abs() < 0.01 && (v0_pos.y - 50.0).abs() < 0.01,
        "v0 should be at (50, 50), got ({}, {})", v0_pos.x, v0_pos.y);
    assert!((v1_pos.x - 150.0).abs() < 0.01 && (v1_pos.y - 50.0).abs() < 0.01,
        "v1 should be at (150, 50), got ({}, {})", v1_pos.x, v1_pos.y);
}

#[test]
fn extract_and_merge_fill_round_trip() {
    let (mut g, _verts, edges, fid) = triangle_graph();

    // Treat e0 as boundary, e1+e2 as inside, fill as inside
    let mut boundary_edges = HashSet::new();
    boundary_edges.insert(edges[0]);
    let mut inside_edges = HashSet::new();
    inside_edges.insert(edges[1]);
    inside_edges.insert(edges[2]);
    let mut inside_fills = HashSet::new();
    inside_fills.insert(fid);

    let (new_g, vtx_map, edge_map) = g.extract_subgraph(
        &inside_edges,
        &inside_fills,
        &boundary_edges,
    );

    // Build maps for merge-back
    let boundary_vtx_map: HashMap<VertexId, VertexId> = vtx_map.iter()
        .filter(|(&old, _)| !g.vertex(old).deleted)
        .map(|(&old, &new)| (new, old))
        .collect();
    let boundary_edge_map_for_merge: HashMap<EdgeId, EdgeId> = edge_map.iter()
        .filter(|(old_eid, _)| boundary_edges.contains(old_eid))
        .map(|(&old, &new)| (new, old))
        .collect();

    // Before merge: original has 1 edge (boundary), extracted has 3 edges + 1 fill
    assert_eq!(g.edges.iter().filter(|e| !e.deleted).count(), 1);
    assert_eq!(g.fills.iter().filter(|f| !f.deleted).count(), 0);
    assert_eq!(new_g.edges.iter().filter(|e| !e.deleted).count(), 3);
    assert_eq!(new_g.fills.iter().filter(|f| !f.deleted).count(), 1);

    // Merge back
    g.merge_subgraph(&new_g, Affine::IDENTITY, &boundary_vtx_map, &boundary_edge_map_for_merge);

    // After merge: should have 3 edges (boundary + 2 merged) and 1 fill
    assert_eq!(g.edges.iter().filter(|e| !e.deleted).count(), 3);
    assert_eq!(g.fills.iter().filter(|f| !f.deleted).count(), 1);
}
