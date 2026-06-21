//! Reproduces the unified region-select behaviour at the graph level: cutting a shape
//! along a lasso outline (`insert_stroke`) and classifying which resulting fills/edges
//! fall inside the lasso — the same logic the editor's `execute_region_select` runs.
//!
//! The shape is built exactly as the editor builds a rectangle: `insert_stroke` of a
//! closed rect loop, then `paint_bucket` to create the fill (NOT a hand-rolled
//! `alloc_fill`), because the bug is in how that traced fill's boundary survives being
//! split by the lasso cut.

use super::super::*;
use kurbo::{BezPath, CubicBez, ParamCurve, Point, Shape};

/// Straight-line cubic from a to b.
fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
        Point::new(a.x + 2.0 * (b.x - a.x) / 3.0, a.y + 2.0 * (b.y - a.y) / 3.0),
        b,
    )
}

fn closed_rect_path(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> BezPath {
    let mut p = BezPath::new();
    p.move_to((min_x, min_y));
    p.line_to((max_x, min_y));
    p.line_to((max_x, max_y));
    p.line_to((min_x, max_y));
    p.close_path();
    p
}

/// Draw a filled rectangle into an existing graph the way the editor's rectangle tool does.
fn draw_filled_rect(g: &mut VectorGraph, min_x: f64, min_y: f64, max_x: f64, max_y: f64) {
    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    for segs in bezpath_to_cubic_segments(&closed_rect_path(min_x, min_y, max_x, max_y)) {
        g.insert_stroke(&segs, Some(style.clone()), Some(color), 0.5);
    }
    let centroid = Point::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
    g.paint_bucket(centroid, ShapeColor::rgb(255, 0, 0), FillRule::NonZero, 0.0)
        .expect("paint_bucket should create a fill");
}

/// Build a filled rectangle the way the editor's rectangle tool does.
fn editor_filled_rect(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> VectorGraph {
    let mut g = VectorGraph::new();
    draw_filled_rect(&mut g, min_x, min_y, max_x, max_y);
    g
}

/// Closed rectangular lasso: cubic segments (for insert_stroke) + BezPath (for winding).
fn rect_lasso(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> (Vec<CubicBez>, BezPath) {
    let p = [
        Point::new(min_x, min_y),
        Point::new(max_x, min_y),
        Point::new(max_x, max_y),
        Point::new(min_x, max_y),
    ];
    let segs = vec![line(p[0], p[1]), line(p[1], p[2]), line(p[2], p[3]), line(p[3], p[0])];
    let mut path = BezPath::new();
    path.move_to(p[0]);
    for pt in &p[1..] {
        path.line_to(*pt);
    }
    path.close_path();
    (segs, path)
}

// Classify-against-lasso uses the same guaranteed-interior probe point the editor uses.
fn fill_centroid(g: &VectorGraph, fid: FillId) -> Point {
    g.fill_interior_point(fid)
}

/// Directed start/end points of a boundary entry.
fn dir_start(g: &VectorGraph, eid: EdgeId, dir: Direction) -> Point {
    let c = g.edge(eid).curve;
    match dir {
        Direction::Forward => c.p0,
        Direction::Backward => c.p3,
    }
}
fn dir_end(g: &VectorGraph, eid: EdgeId, dir: Direction) -> Point {
    let c = g.edge(eid).curve;
    match dir {
        Direction::Forward => c.p3,
        Direction::Backward => c.p0,
    }
}

/// Describe a fill's boundary as an ordered list of (start → end) segments for diagnostics.
fn describe_boundary(g: &VectorGraph, fid: FillId) -> Vec<(EdgeId, Point, Point)> {
    g.fill(fid)
        .boundary
        .iter()
        .filter(|(e, _)| !e.is_none())
        .map(|&(e, d)| (e, dir_start(g, e, d), dir_end(g, e, d)))
        .collect()
}

/// Assert no fill boundary uses the same edge twice (the signature of a degenerate
/// "spike" where the trace runs out and back along a dangling edge).
fn assert_no_spikes(g: &VectorGraph) {
    for (fi, f) in g.fills.iter().enumerate() {
        if f.deleted { continue; }
        let mut seen = std::collections::HashSet::new();
        for &(e, _) in &f.boundary {
            if e.is_none() { continue; }
            assert!(seen.insert(e.0), "fill {fi} uses edge {} twice (spike)", e.0);
        }
    }
}

/// Assert every non-deleted fill's boundary is a connected closed loop.
fn assert_all_fills_connected(g: &VectorGraph) {
    for (i, f) in g.fills.iter().enumerate() {
        if f.deleted {
            continue;
        }
        let fid = FillId(i as u32);
        let b = describe_boundary(g, fid);
        if b.is_empty() {
            continue;
        }
        let n = b.len();
        for k in 0..n {
            let (_, _, end) = b[k];
            let (_, next_start, _) = b[(k + 1) % n];
            // Tolerance well below any real gap (pixels) but above accumulated float drift.
            assert!(
                (end.x - next_start.x).abs() < 1e-2 && (end.y - next_start.y).abs() < 1e-2,
                "fill {i} boundary disconnected between edge {k} (ends {end:?}) and \
                 edge {} (starts {next_start:?}); boundary = {b:#?}",
                (k + 1) % n
            );
        }
    }
}

#[test]
fn single_vertical_cut_produces_two_connected_fills() {
    // Rectangle (0,0)-(200,100), one vertical cut at x=100.
    let mut g = editor_filled_rect(0.0, 0.0, 200.0, 100.0);
    let cut = vec![line(Point::new(100.0, -10.0), Point::new(100.0, 110.0))];
    g.insert_stroke(&cut, None, None, 1.0);

    let live = g.fills.iter().filter(|f| !f.deleted).count();
    assert_eq!(live, 2, "one cut should split the rect into 2 fills, got {live}");
    assert_all_fills_connected(&g);
}

#[test]
fn lasso_over_second_of_two_rects_splits_that_rect() {
    // Two separate rectangles; lasso a vertical strip over the SECOND one.
    let mut g = editor_filled_rect(0.0, 0.0, 200.0, 100.0);
    draw_filled_rect(&mut g, 0.0, 150.0, 200.0, 250.0);
    assert_eq!(g.fills.iter().filter(|f| !f.deleted).count(), 2, "two rects → two fills");

    let (segs, lasso_path) = rect_lasso(50.0, 120.0, 150.0, 300.0); // crosses rect B only
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();

    // Rect B should now be split into 3 fills (left/middle/right strips); rect A is untouched.
    // So 1 (rect A) + 3 (rect B pieces) = 4 fills total.
    let live = g.fills.iter().filter(|f| !f.deleted).count();
    let inside_fills: Vec<FillId> = g
        .fills
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.deleted)
        .map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso_path.winding(fill_centroid(&g, fid)) != 0)
        .collect();

    let dump = || {
        g.fills
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.deleted)
            .map(|(i, _)| (i, fill_centroid(&g, FillId(i as u32))))
            .collect::<Vec<_>>()
    };

    assert_eq!(live, 4, "rect B should split into 3 (total 4 fills); got {live}: {:#?}", dump());
    assert_eq!(
        inside_fills.len(),
        1,
        "exactly the center strip of rect B should be inside; got {:#?}",
        dump()
    );
    assert_all_fills_connected(&g);
}

#[test]
fn lasso_strip_through_side_by_side_second_rect() {
    // Two rectangles side by side; lasso a vertical strip through the SECOND (right) one.
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    draw_filled_rect(&mut g, 150.0, 0.0, 250.0, 100.0);

    let (segs, lasso_path) = rect_lasso(180.0, -50.0, 220.0, 150.0);
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();
    assert_all_fills_connected(&g);

    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso_path.winding(fill_centroid(&g, fid)) != 0).collect();
    let dump: Vec<_> = g.fills.iter().enumerate().filter(|(_, f)| !f.deleted)
        .map(|(i, _)| (i, fill_centroid(&g, FillId(i as u32)), describe_boundary(&g, FillId(i as u32)).len())).collect();
    assert_eq!(inside.len(), 1, "one strip of the right rect should be inside; got {dump:#?}");
}

/// No live fill may reference a freed edge or vertex (whose slot a later alloc reuses).
fn assert_no_freed_but_referenced(g: &VectorGraph) {
    let freed_v: std::collections::HashSet<u32> = g.free_vertices.iter().copied().collect();
    let freed_e: std::collections::HashSet<u32> = g.free_edges.iter().copied().collect();
    for (fi, f) in g.fills.iter().enumerate() {
        if f.deleted { continue; }
        for &(e, _) in &f.boundary {
            if e.is_none() { continue; }
            assert!(!freed_e.contains(&e.0), "live fill {fi} references freed edge {}", e.0);
            for &v in &g.edge(e).vertices {
                assert!(!freed_v.contains(&v.0), "live fill {fi} references freed vertex {}", v.0);
            }
        }
    }
}

#[test]
fn extract_after_cut_keeps_remaining_fill_intact() {
    // Cut a rectangle's corner with a lasso, then extract (Group) the clipped corner. The
    // extraction must not free vertices/edges still referenced by the L-shaped remainder —
    // previously it freed the shared cut vertices, and a later `alloc_vertex` reused those
    // slots and corrupted the remainder (the "second lasso deletes all faces" bug).
    let mut g = editor_filled_rect(0.0, 0.0, 200.0, 100.0);
    let (segs, lasso) = rect_lasso(100.0, -50.0, 300.0, 50.0); // clips the top-right corner
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();

    // Pick the fill inside the lasso (the clipped corner) and extract it.
    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso.winding(g.fill_interior_point(fid)) != 0).collect();
    assert_eq!(inside.len(), 1, "one corner fill should be inside the lasso");
    let inside_fills: HashSet<FillId> = inside.iter().copied().collect();
    let inside_edges: HashSet<EdgeId> = g.fill(inside[0]).boundary.iter()
        .filter_map(|&(e, _)| (!e.is_none()).then_some(e)).collect();

    let _ = g.extract_subgraph(&inside_edges, &inside_fills, &HashSet::new());

    assert_no_freed_but_referenced(&g);
    assert_all_fills_connected(&g);

    // Simulate the next lasso allocating vertices, then re-validate: a corrupted (freed but
    // referenced) vertex would now be at a bogus position.
    let (segs2, _) = rect_lasso(20.0, 20.0, 60.0, 80.0);
    g.insert_stroke(&segs2, None, None, 1.0);
    g.gc_invisible_edges();
    assert_all_fills_connected(&g);
}

/// Captured region-select cases (`LIGHTNINGBEAM_DUMP_REGION=1`), embedded so the regression
/// survives `/tmp` being cleared. Each is `{ "graph": <VectorGraph>, "segments": [[[x,y]*4]*] }`.
/// They span: two separate rects, side-by-side, overlapping, a notched post-group fill, and
/// dense self-intersecting freehand lassos (dump 3 is the boundary-spike repro).
const REGION_DUMPS: &[&str] = &[
    include_str!("region_dumps/dump0.json"),
    include_str!("region_dumps/dump1.json"),
    include_str!("region_dumps/dump2.json"),
    include_str!("region_dumps/dump3.json"),
    include_str!("region_dumps/dump4.json"),
];

#[test]
fn dumped_region_selects_are_valid() {
    // Replays each captured region-select cut and asserts it yields only valid, non-corrupt
    // geometry (no freed-but-referenced refs, no spikes, every fill a connected loop).
    for json in REGION_DUMPS {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let mut g: VectorGraph = serde_json::from_value(v["graph"].clone()).unwrap();
        let segs: Vec<CubicBez> = v["segments"].as_array().unwrap().iter().map(|s| {
            let p = |i: usize| { let a = s[i].as_array().unwrap(); Point::new(a[0].as_f64().unwrap(), a[1].as_f64().unwrap()) };
            CubicBez::new(p(0), p(1), p(2), p(3))
        }).collect();
        g.insert_stroke(&segs, None, None, 1.0);
        g.gc_invisible_edges();
        assert_no_freed_but_referenced(&g);
        assert_no_spikes(&g);
        assert_all_fills_connected(&g);
    }
}

#[test]
fn near_coincident_needle_does_not_spike() {
    // Smoke test: a stroke poking into a fill and returning along a near-coincident path
    // must still yield valid, non-corrupt geometry. (The dense-freehand accordion that the
    // boundary-spike collapse specifically fixes is only reliably reproduced by the captured
    // region dumps; this is a lightweight portable guard for the same family of inputs.)
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    g.insert_stroke(
        &[
            line(Point::new(50.0, -10.0), Point::new(50.0, 50.0)),
            line(Point::new(50.0, 50.0), Point::new(50.0003, -10.0)),
        ],
        None, None, 1.0,
    );
    g.gc_invisible_edges();
    assert_no_spikes(&g);
    assert_all_fills_connected(&g);
}

#[test]
fn second_stroke_crossing_first_splits_into_quadrants() {
    // A later stroke that crosses an earlier stroke's edge inside a fill triggers an
    // edge-domain `split_edge`, whose sub-edges aren't tracked in the second stroke's new
    // edges. The retrace must still see them (induced-subgraph expansion) or the cut breaks
    // and unravels. Two crossing cuts through a rectangle must yield four connected quadrants.
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    g.insert_stroke(&[line(Point::new(-10.0, 50.0), Point::new(110.0, 50.0))], None, None, 1.0); // horizontal
    assert_eq!(g.fills.iter().filter(|f| !f.deleted).count(), 2, "horizontal cut → 2");
    g.insert_stroke(&[line(Point::new(50.0, -10.0), Point::new(50.0, 110.0))], None, None, 1.0); // vertical, crosses it
    g.gc_invisible_edges();

    let live: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32)).collect();
    assert_eq!(live.len(), 4, "two crossing cuts → four quadrants; got {}", live.len());
    assert_all_fills_connected(&g);
    assert_no_spikes(&g);
    for &fid in &live {
        assert!((polygon_area(&g, fid) - 2500.0).abs() < 1.0, "each quadrant is 50x50, got {}", polygon_area(&g, fid));
    }
}

#[test]
fn stroke_dead_ending_inside_fill_does_not_spike() {
    // A stroke (e.g. a freehand lasso that wiggles or nearly self-touches) can leave a
    // dangling stub: an edge whose inner endpoint has degree 1. Re-tracing must not run out
    // and back along it (a self-touching "spike" boundary); the fill stays a clean loop.
    let mut g = editor_filled_rect(0.0, 0.0, 200.0, 100.0);
    // Open stroke entering the top edge and dead-ending inside at (100,50).
    let stub = vec![line(Point::new(50.0, -20.0), Point::new(100.0, 50.0))];
    g.insert_stroke(&stub, None, None, 1.0);
    g.gc_invisible_edges();

    let live: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32)).collect();
    assert_eq!(live.len(), 1, "a dead-end stub must not split the fill; got {}", live.len());
    assert_all_fills_connected(&g);
    assert_no_spikes(&g);
    // The fill is still the full rectangle.
    assert!((polygon_area(&g, live[0]) - 20000.0).abs() < 1.0,
        "fill should remain the 200x100 rectangle, area {}", polygon_area(&g, live[0]));
}

#[test]
fn groupfail_two_notched_fills_second_lasso() {
    // Faithful reconstruction of the captured post-group state (dump_1): two adjacent
    // fills sharing the x=337 column, each with an invisible notch (the hole left by the
    // first lasso+group). A second lasso (398,249)-(495,327) clips fill 3's corner.
    use std::collections::HashMap;
    let mut g = VectorGraph::new();
    let vp = [
        (130.0, 232.0), (337.0, 232.0), (337.0, 362.0), (130.0, 362.0), (337.0, 297.0),
        (535.0, 297.0), (535.0, 417.0), (337.0, 417.0), (0.0, 0.0), (337.0, 315.0),
        (220.0, 315.0), (456.0, 315.0), (456.0, 345.0), (337.0, 345.0), (220.0, 345.0),
    ];
    let vs: Vec<_> = vp.iter().map(|&(x, y)| g.alloc_vertex(Point::new(x, y))).collect();
    let style = StrokeStyle { width: 3.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    // (name, v_start, v_end, visible)
    let edge_defs: &[(&str, usize, usize, bool)] = &[
        ("e0", 0, 1, true), ("e1", 4, 5, true), ("e2", 2, 3, true), ("e3", 3, 0, true),
        ("e4", 1, 4, true), ("e5", 7, 2, true), ("e6", 5, 6, true), ("e7", 6, 7, true),
        ("e8", 10, 9, false), ("e9", 12, 13, false), ("e11", 4, 9, true), ("e12", 9, 11, false),
        ("e13", 11, 12, false), ("e15", 13, 2, true), ("e16", 13, 14, false), ("e17", 14, 10, false),
    ];
    let mut em: HashMap<&str, EdgeId> = HashMap::new();
    for &(name, a, b, vis) in edge_defs {
        let (ss, sc) = if vis { (Some(style.clone()), Some(color)) } else { (None, None) };
        let e = g.alloc_edge(line(vp[a].into(), vp[b].into()), vs[a], vs[b], ss, sc);
        em.insert(name, e);
    }
    let mk = |spec: &[(&str, Direction)]| -> Vec<(EdgeId, Direction)> {
        spec.iter().map(|&(n, d)| (em[n], d)).collect()
    };
    use Direction::{Backward as B, Forward as F};
    g.alloc_fill(mk(&[("e11", B), ("e4", B), ("e0", B), ("e3", B), ("e2", B), ("e15", B), ("e16", F), ("e17", F), ("e8", F)]),
        ShapeColor::rgb(100, 100, 255), FillRule::NonZero);
    g.alloc_fill(mk(&[("e15", F), ("e5", B), ("e7", B), ("e6", B), ("e1", B), ("e11", F), ("e12", F), ("e13", F), ("e9", F)]),
        ShapeColor::rgb(100, 100, 255), FillRule::NonZero);

    assert_eq!(g.fills.iter().filter(|f| !f.deleted).count(), 2, "starts with 2 fills");
    let (segs, lasso) = rect_lasso(398.0, 249.0, 495.0, 327.0);
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();

    let live = g.fills.iter().filter(|f| !f.deleted).count();
    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso.winding(g.fill_interior_point(fid)) != 0).collect();
    eprintln!("groupfail: {live} live fills, {} inside lasso", inside.len());
    for (i, f) in g.fills.iter().enumerate() {
        if f.deleted { continue; }
        let fid = FillId(i as u32);
        eprintln!("  fill {i}: bbox {:?} area {:.0}", fill_bbox(&g, fid), polygon_area(&g, fid));
    }
    assert_all_fills_connected(&g);
    // The two original shapes must survive (no faces wrongly deleted); the lasso adds a cut.
    assert!(live >= 2, "must keep both shapes plus the cut; got {live}");
    assert_eq!(inside.len(), 1, "exactly the clipped corner of fill 3 should be selected");
}

#[test]
fn lasso_across_notched_fill_never_corrupts_boundaries() {
    // A rectangle with a rectangular notch bitten out of its top edge — the shape a fill
    // is left as after grouping a lasso selection (the notch is the extracted region's
    // hole). A second lasso crossing the notch must never produce disconnected/corrupt
    // fill boundaries (it previously incorporated the lasso's out-of-shape edges, drawing
    // stray diagonals). It may under-select on such complex geometry, but must stay valid.
    let mut g = VectorGraph::new();
    let pts = [
        Point::new(0.0, 0.0), Point::new(80.0, 0.0), Point::new(80.0, 40.0),
        Point::new(120.0, 40.0), Point::new(120.0, 0.0), Point::new(200.0, 0.0),
        Point::new(200.0, 100.0), Point::new(0.0, 100.0),
    ];
    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let color = ShapeColor::rgb(0, 0, 0);
    let v: Vec<_> = pts.iter().map(|&p| g.alloc_vertex(p)).collect();
    let mut boundary = Vec::new();
    for i in 0..pts.len() {
        let e = g.alloc_edge(line(pts[i], pts[(i + 1) % pts.len()]), v[i], v[(i + 1) % pts.len()],
            Some(style.clone()), Some(color));
        boundary.push((e, Direction::Forward));
    }
    g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);

    // Lasso a rectangle that straddles the notch and the shape body. Inside the fill it
    // covers (60..140, 0..60) minus the notch (80..120, 0..40) — an arch shape — which the
    // cut must isolate as one connected fill, with the remainder outside.
    let (segs, lasso) = rect_lasso(60.0, -20.0, 140.0, 60.0);
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();

    // Every resulting fill is a valid connected loop (never corrupt).
    assert_all_fills_connected(&g);

    // Exactly one fill lies inside the lasso, and it is the arch (area = 80*60 - 40*40).
    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso.winding(g.fill_interior_point(fid)) != 0).collect();
    assert_eq!(inside.len(), 1, "the arch (lasso ∩ notched fill) should be one selected fill");
    let area = polygon_area(&g, inside[0]);
    assert!((area - 3200.0).abs() < 1.0, "arch area should be 3200, got {area}");
}

/// Absolute polygon area of a fill from its boundary corner points.
fn polygon_area(g: &VectorGraph, fid: FillId) -> f64 {
    let pts: Vec<Point> = g.fill(fid).boundary.iter()
        .filter(|(e, _)| !e.is_none())
        .map(|&(e, d)| dir_start(g, e, d))
        .collect();
    if pts.len() < 3 { return 0.0; }
    let mut a2 = 0.0;
    for i in 0..pts.len() {
        let p0 = pts[i]; let p1 = pts[(i + 1) % pts.len()];
        a2 += p0.x * p1.y - p1.x * p0.y;
    }
    (a2 * 0.5).abs()
}

#[test]
fn two_edge_adjacent_rects_make_two_clean_fills() {
    // Two rectangles sharing the x=100 edge. The second's left edge lands exactly on the
    // first's right edge; without coincident-edge cleanup this produced duplicate edges and
    // zero-area "sliver" fills (4 fills instead of 2) before any lasso.
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    draw_filled_rect(&mut g, 100.0, 0.0, 200.0, 100.0);
    let live: Vec<_> = g.fills.iter().enumerate().filter(|(_, f)| !f.deleted)
        .map(|(i, _)| (i, fill_bbox(&g, FillId(i as u32)))).collect();
    assert_eq!(live.len(), 2, "two adjacent rects should make exactly two fills; got {live:?}");
    assert_all_fills_connected(&g);
}

#[test]
fn lasso_strip_through_adjacent_second_rect() {
    // Two rectangles sharing an edge (snapped adjacent), lasso a strip through the second.
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    draw_filled_rect(&mut g, 100.0, 0.0, 200.0, 100.0); // shares the x=100 edge with the first

    let (segs, lasso_path) = rect_lasso(130.0, -50.0, 170.0, 150.0); // strip through 2nd
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();
    assert_all_fills_connected(&g);

    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso_path.winding(fill_centroid(&g, fid)) != 0).collect();
    let dump: Vec<_> = g.fills.iter().enumerate().filter(|(_, f)| !f.deleted)
        .map(|(i, _)| (i, fill_centroid(&g, FillId(i as u32)), describe_boundary(&g, FillId(i as u32)).len())).collect();
    assert_eq!(inside.len(), 1, "one strip of the 2nd rect should be inside; got {dump:#?}");
}

#[test]
fn lasso_strip_through_overlapping_second_rect() {
    // Second rectangle overlaps the first; lasso a strip through the second's free part.
    let mut g = editor_filled_rect(0.0, 0.0, 100.0, 100.0);
    draw_filled_rect(&mut g, 60.0, 60.0, 200.0, 160.0);

    let (segs, _lasso) = rect_lasso(120.0, 40.0, 160.0, 180.0);
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();
    assert_all_fills_connected(&g);
}

/// Bounding box of a fill's boundary edge endpoints.
fn fill_bbox(g: &VectorGraph, fid: FillId) -> (f64, f64, f64, f64) {
    let (mut minx, mut miny, mut maxx, mut maxy) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for &(eid, _) in &g.fill(fid).boundary {
        if eid.is_none() {
            continue;
        }
        for &vid in &g.edge(eid).vertices {
            let p = g.vertex(vid).position;
            minx = minx.min(p.x);
            miny = miny.min(p.y);
            maxx = maxx.max(p.x);
            maxy = maxy.max(p.y);
        }
    }
    (minx, miny, maxx, maxy)
}

#[test]
fn lasso_corner_clip_of_second_rect_splits_off_corner() {
    // Captured repro (/tmp dump): two separate rects; the lasso clips the top-left CORNER
    // of the second rect, so the cut path turns a corner at a vertex INTERIOR to the rect.
    let mut g = editor_filled_rect(128.37, 255.97, 290.91, 336.55); // rect 1
    draw_filled_rect(&mut g, 417.39, 311.62, 620.25, 442.55); // rect 2
    let (segs, lasso) = rect_lasso(360.12, 277.92, 537.32, 397.14); // clips rect-2 corner
    g.insert_stroke(&segs, None, None, 1.0);
    g.gc_invisible_edges();
    assert_all_fills_connected(&g);

    let inside: Vec<FillId> = g.fills.iter().enumerate()
        .filter(|(_, f)| !f.deleted).map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso.winding(fill_centroid(&g, fid)) != 0).collect();
    let dump: Vec<_> = g.fills.iter().enumerate().filter(|(_, f)| !f.deleted)
        .map(|(i, _)| (i, fill_centroid(&g, FillId(i as u32)), fill_bbox(&g, FillId(i as u32)), describe_boundary(&g, FillId(i as u32)).len())).collect();
    assert_eq!(inside.len(), 1, "exactly the clipped corner should be inside; fills = {dump:#?}");

    // The selected fill must be the clipped CORNER, not the whole rect 2.
    let (minx, miny, maxx, maxy) = fill_bbox(&g, inside[0]);
    let eps = 1.0;
    assert!(
        (minx - 417.39).abs() < eps && (miny - 311.62).abs() < eps
            && (maxx - 537.32).abs() < eps && (maxy - 397.14).abs() < eps,
        "expected clipped-corner bbox (417.4,311.6)-(537.3,397.1), got ({minx:.1},{miny:.1})-({maxx:.1},{maxy:.1}) \
         — whole rect2 is (417,311)-(620,442); the fill wasn't split"
    );
}

#[test]
fn lasso_over_middle_of_rect_selects_clean_center_strip() {
    // Rectangle (0,0)-(200,100); lasso a vertical strip (50,-50)-(150,150).
    let mut g = editor_filled_rect(0.0, 0.0, 200.0, 100.0);
    let (segs, lasso_path) = rect_lasso(50.0, -50.0, 150.0, 150.0);

    g.insert_stroke(&segs, None, None, 1.0);
    // The editor runs this right after the cut: the lasso's top/bottom edges and the
    // out-of-shape extensions of its sides are invisible and unreferenced — they must be
    // collected so they can't be selected/edited later.
    g.gc_invisible_edges();
    for (i, e) in g.edges.iter().enumerate() {
        if e.deleted || e.stroke_style.is_some() || e.stroke_color.is_some() {
            continue;
        }
        let eid = EdgeId(i as u32);
        let in_a_fill = g
            .fills
            .iter()
            .any(|f| !f.deleted && f.boundary.iter().any(|&(fe, _)| fe == eid));
        assert!(
            in_a_fill,
            "stray invisible edge {i} ({:?}) survived gc — not part of any fill",
            e.curve
        );
    }

    // Classify fills by centroid winding (the editor's logic).
    let inside_fills: Vec<FillId> = g
        .fills
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.deleted)
        .map(|(i, _)| FillId(i as u32))
        .filter(|&fid| lasso_path.winding(fill_centroid(&g, fid)) != 0)
        .collect();

    let dump = || {
        g.fills
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.deleted)
            .map(|(i, _)| {
                let fid = FillId(i as u32);
                (i, fill_centroid(&g, fid), describe_boundary(&g, fid))
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(
        inside_fills.len(),
        1,
        "expected exactly one inside fill (the center strip); live fills = {:#?}",
        dump()
    );

    let fid = inside_fills[0];
    let boundary = describe_boundary(&g, fid);

    // The center strip is a rectangle: it must have 4 boundary edges that form a
    // *connected closed loop* (each edge's directed end == the next edge's directed start).
    // Before the fix, the split produced a 2-edge boundary (left cut + top segment) whose
    // bbox is still (50,0)-(150,100) but which `fill_to_bezpath` renders as left+top edges
    // plus a diagonal close — the artifact the user reported.
    assert_eq!(
        boundary.len(),
        4,
        "center strip should have 4 boundary edges, got {}: {:#?}",
        boundary.len(),
        boundary
    );

    let eps = 1e-6;
    let n = boundary.len();
    for i in 0..n {
        let (_, _, end) = boundary[i];
        let (_, next_start, _) = boundary[(i + 1) % n];
        assert!(
            (end.x - next_start.x).abs() < eps && (end.y - next_start.y).abs() < eps,
            "boundary is disconnected between edge {i} (ends {end:?}) and edge {} \
             (starts {next_start:?}); full boundary = {boundary:#?}",
            (i + 1) % n
        );
    }

    // And the loop should visit the four expected corners.
    let corners = [
        Point::new(50.0, 0.0),
        Point::new(150.0, 0.0),
        Point::new(150.0, 100.0),
        Point::new(50.0, 100.0),
    ];
    for c in corners {
        let hit = boundary
            .iter()
            .any(|&(_, s, _)| (s.x - c.x).abs() < 0.5 && (s.y - c.y).abs() < 0.5);
        assert!(hit, "center strip boundary should pass through corner {c:?}: {boundary:#?}");
    }
}
