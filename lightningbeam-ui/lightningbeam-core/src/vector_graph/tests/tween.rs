//! Tests for same-topology shape-tween interpolation (`VectorGraph::interpolated`).

use super::super::*;
use kurbo::{CubicBez, Point};

fn line(a: Point, b: Point) -> CubicBez {
    CubicBez::new(
        a,
        Point::new(a.x + (b.x - a.x) / 3.0, a.y + (b.y - a.y) / 3.0),
        Point::new(a.x + 2.0 * (b.x - a.x) / 3.0, a.y + 2.0 * (b.y - a.y) / 3.0),
        b,
    )
}

/// Triangle (3 verts, 3 edges, 1 fill) offset by (ox, oy).
fn triangle(ox: f64, oy: f64) -> VectorGraph {
    let mut g = VectorGraph::new();
    let p = [
        Point::new(ox, oy),
        Point::new(ox + 100.0, oy),
        Point::new(ox + 50.0, oy + 100.0),
    ];
    let v: Vec<_> = p.iter().map(|&pt| g.alloc_vertex(pt)).collect();
    let style = StrokeStyle { width: 1.0, ..Default::default() };
    let mut boundary = Vec::new();
    for i in 0..3 {
        let e = g.alloc_edge(
            line(p[i], p[(i + 1) % 3]),
            v[i],
            v[(i + 1) % 3],
            Some(style.clone()),
            Some(ShapeColor::rgb(0, 0, 0)),
        );
        boundary.push((e, Direction::Forward));
    }
    g.alloc_fill(boundary, ShapeColor::rgb(255, 0, 0), FillRule::NonZero);
    g
}

#[test]
fn interpolate_same_topology_lerps_positions() {
    let a = triangle(0.0, 0.0);
    let b = triangle(100.0, 50.0);

    let mid = a.interpolated(&b, 0.5).expect("same topology should interpolate");
    // Vertex 0: (0,0) and (100,50) → (50,25). Curve endpoints follow.
    assert!((mid.vertices[0].position.x - 50.0).abs() < 1e-6);
    assert!((mid.vertices[0].position.y - 25.0).abs() < 1e-6);
    assert!((mid.edges[0].curve.p0.x - 50.0).abs() < 1e-6);

    // Endpoints: t=0 is `a`, t=1 is `b`.
    assert!((a.interpolated(&b, 0.0).unwrap().vertices[0].position.x - 0.0).abs() < 1e-6);
    assert!((a.interpolated(&b, 1.0).unwrap().vertices[0].position.x - 100.0).abs() < 1e-6);
}

#[test]
fn interpolate_lerps_fill_color() {
    let mut a = triangle(0.0, 0.0);
    let mut b = triangle(0.0, 0.0);
    a.fills[0].color = Some(ShapeColor::rgb(0, 0, 0));
    b.fills[0].color = Some(ShapeColor::rgb(100, 200, 40));
    let mid = a.interpolated(&b, 0.5).unwrap();
    let c = mid.fills[0].color.unwrap();
    assert_eq!((c.r, c.g, c.b), (50, 100, 20));
}

#[test]
fn interpolate_topology_mismatch_returns_none() {
    let a = triangle(0.0, 0.0);
    let mut more_verts = triangle(0.0, 0.0);
    more_verts.alloc_vertex(Point::new(999.0, 999.0));
    assert!(a.interpolated(&more_verts, 0.5).is_none(), "different vertex count");

    // Same counts but a moved edge endpoint (different vertices) is still a mismatch.
    let mut rewired = triangle(0.0, 0.0);
    rewired.edges[0].vertices = [VertexId(2), VertexId(1)];
    assert!(a.interpolated(&rewired, 0.5).is_none(), "different edge endpoints");
}
