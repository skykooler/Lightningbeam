//! Geometry snapping for vector editing
//!
//! Provides snap-to-geometry queries that find the nearest vertex, edge midpoint,
//! or curve point within a given radius. Priority order: Vertex > Midpoint > Curve.

use crate::vector_graph::{VectorGraph, EdgeId, VertexId};
use vello::kurbo::{ParamCurve, ParamCurveNearest, Point};

/// Default snap radius in screen pixels (converted to document space via zoom).
pub const SNAP_SCREEN_RADIUS: f64 = 12.0;

/// What the cursor snapped to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SnapTarget {
    /// Snapped to an existing vertex position.
    Vertex { vertex_id: VertexId },
    /// Snapped to the midpoint (t=0.5) of an edge.
    Midpoint { edge_id: EdgeId },
    /// Snapped to the nearest point on a curve.
    Curve { edge_id: EdgeId, parameter_t: f64 },
}

/// Result of a snap query.
#[derive(Debug, Clone, Copy)]
pub struct SnapResult {
    /// The position to snap to (in document/local space).
    pub position: Point,
    /// What type of element was snapped to.
    pub target: SnapTarget,
    /// Distance from the query point to the snap position.
    pub distance: f64,
}

/// Configuration for snap behavior.
#[derive(Debug, Clone, Copy)]
pub struct SnapConfig {
    /// Snap search radius in document units.
    pub radius: f64,
    /// Whether vertex snapping is enabled.
    pub snap_to_vertices: bool,
    /// Whether midpoint snapping is enabled.
    pub snap_to_midpoints: bool,
    /// Whether curve snapping is enabled.
    pub snap_to_curves: bool,
}

impl SnapConfig {
    /// Create a snap config from a screen-pixel radius, converted to document space.
    pub fn from_screen_radius(screen_pixels: f64, zoom: f64) -> Self {
        Self {
            radius: screen_pixels / zoom,
            snap_to_vertices: true,
            snap_to_midpoints: true,
            snap_to_curves: true,
        }
    }
}

/// Elements to exclude from snap queries (self-exclusion during drag).
#[derive(Debug, Clone, Default)]
pub struct SnapExclusion {
    /// Vertices to skip (e.g. the vertex being dragged).
    pub vertices: Vec<VertexId>,
    /// Edges to skip (e.g. edges connected to the dragged vertex).
    pub edges: Vec<EdgeId>,
}

/// Find the best snap target for a point within a DCEL.
///
/// Priority: Vertex > Edge Midpoint > Nearest point on Curve.
/// Returns `None` if nothing is within the configured radius.
pub fn find_snap_target(
    graph: &VectorGraph,
    point: Point,
    config: &SnapConfig,
    exclusion: &SnapExclusion,
) -> Option<SnapResult> {
    let radius_sq = config.radius * config.radius;

    // Phase 1: Vertex snap (highest priority)
    if config.snap_to_vertices {
        let mut best: Option<(VertexId, Point, f64)> = None;
        for (i, vertex) in graph.vertices.iter().enumerate() {
            if vertex.deleted {
                continue;
            }
            let vid = VertexId(i as u32);
            if exclusion.vertices.contains(&vid) {
                continue;
            }
            let dx = vertex.position.x - point.x;
            let dy = vertex.position.y - point.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= radius_sq {
                if best.is_none() || dist_sq < best.unwrap().2 {
                    best = Some((vid, vertex.position, dist_sq));
                }
            }
        }
        if let Some((vid, pos, dist_sq)) = best {
            return Some(SnapResult {
                position: pos,
                target: SnapTarget::Vertex { vertex_id: vid },
                distance: dist_sq.sqrt(),
            });
        }
    }

    // Phase 2: Edge midpoint snap
    if config.snap_to_midpoints {
        let mut best: Option<(EdgeId, Point, f64)> = None;
        for (i, edge) in graph.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let eid = EdgeId(i as u32);
            if exclusion.edges.contains(&eid) {
                continue;
            }
            let midpoint = edge.curve.eval(0.5);
            let dx = midpoint.x - point.x;
            let dy = midpoint.y - point.y;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= radius_sq {
                if best.is_none() || dist_sq < best.unwrap().2 {
                    best = Some((eid, midpoint, dist_sq));
                }
            }
        }
        if let Some((eid, pos, dist_sq)) = best {
            return Some(SnapResult {
                position: pos,
                target: SnapTarget::Midpoint { edge_id: eid },
                distance: dist_sq.sqrt(),
            });
        }
    }

    // Phase 3: Nearest point on curve
    if config.snap_to_curves {
        let mut best: Option<(EdgeId, f64, Point, f64)> = None;
        for (i, edge) in graph.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            }
            let eid = EdgeId(i as u32);
            if exclusion.edges.contains(&eid) {
                continue;
            }
            let nearest = edge.curve.nearest(point, 0.5);
            let dist = nearest.distance_sq.sqrt();
            if dist <= config.radius {
                if best.is_none() || dist < best.unwrap().3 {
                    let snap_point = edge.curve.eval(nearest.t);
                    best = Some((eid, nearest.t, snap_point, dist));
                }
            }
        }
        if let Some((eid, t, pos, dist)) = best {
            return Some(SnapResult {
                position: pos,
                target: SnapTarget::Curve {
                    edge_id: eid,
                    parameter_t: t,
                },
                distance: dist,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use vello::kurbo::CubicBez;

    fn make_graph_with_edge() -> VectorGraph {
        let mut graph = VectorGraph::new();
        let curve = CubicBez::new(
            Point::new(0.0, 0.0),
            Point::new(33.0, 0.0),
            Point::new(67.0, 0.0),
            Point::new(100.0, 0.0),
        );
        graph.insert_stroke(&[curve], None, None, 0.5);
        graph
    }

    #[test]
    fn snap_to_vertex() {
        let graph = make_graph_with_edge();
        let config = SnapConfig {
            radius: 5.0,
            snap_to_vertices: true,
            snap_to_midpoints: true,
            snap_to_curves: true,
        };
        let exclusion = SnapExclusion::default();
        let result = find_snap_target(&graph, Point::new(2.0, 0.0), &config, &exclusion);
        assert!(result.is_some());
        assert!(matches!(result.unwrap().target, SnapTarget::Vertex { .. }));
    }

    #[test]
    fn snap_to_midpoint() {
        let graph = make_graph_with_edge();
        let config = SnapConfig {
            radius: 5.0,
            snap_to_vertices: true,
            snap_to_midpoints: true,
            snap_to_curves: true,
        };
        let exclusion = SnapExclusion::default();
        // Point near midpoint (50, 0) but far from vertices (0,0) and (100,0)
        let result = find_snap_target(&graph, Point::new(51.0, 0.0), &config, &exclusion);
        assert!(result.is_some());
        assert!(matches!(result.unwrap().target, SnapTarget::Midpoint { .. }));
    }

    #[test]
    fn snap_to_curve() {
        let graph = make_graph_with_edge();
        let config = SnapConfig {
            radius: 5.0,
            snap_to_vertices: true,
            snap_to_midpoints: true,
            snap_to_curves: true,
        };
        let exclusion = SnapExclusion::default();
        // Point near t=0.25 on curve (25, 0) — not near a vertex or midpoint
        let result = find_snap_target(&graph, Point::new(25.0, 3.0), &config, &exclusion);
        assert!(result.is_some());
        assert!(matches!(result.unwrap().target, SnapTarget::Curve { .. }));
    }

    #[test]
    fn no_snap_outside_radius() {
        let graph = make_graph_with_edge();
        let config = SnapConfig {
            radius: 5.0,
            snap_to_vertices: true,
            snap_to_midpoints: true,
            snap_to_curves: true,
        };
        let exclusion = SnapExclusion::default();
        let result = find_snap_target(&graph, Point::new(50.0, 20.0), &config, &exclusion);
        assert!(result.is_none());
    }

    #[test]
    fn exclusion_skips_vertex() {
        let graph = make_graph_with_edge();
        let config = SnapConfig {
            radius: 5.0,
            snap_to_vertices: true,
            snap_to_midpoints: false,
            snap_to_curves: false,
        };
        // Exclude vertex 0
        let exclusion = SnapExclusion {
            vertices: vec![VertexId(0)],
            edges: vec![],
        };
        let result = find_snap_target(&graph, Point::new(2.0, 0.0), &config, &exclusion);
        assert!(result.is_none());
    }
}
