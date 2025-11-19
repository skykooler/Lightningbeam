//! Planar graph construction for paint bucket fill
//!
//! This module builds a planar graph from a collection of curves by:
//! 1. Finding all intersections (curve-curve and self-intersections)
//! 2. Splitting curves at intersection points to create graph edges
//! 3. Creating nodes at all intersection points and curve endpoints
//! 4. Connecting edges to form a complete planar graph
//!
//! The resulting graph can be used for face detection to identify regions for filling.

use crate::curve_intersections::{find_curve_intersections, find_self_intersections};
use crate::curve_segment::CurveSegment;
use crate::shape::{Shape, ShapeColor, StrokeStyle};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use vello::kurbo::{BezPath, Circle, CubicBez, Point, Shape as KurboShape};

/// Global debug storage for the last planar graph (for visualization)
pub static DEBUG_GRAPH: Mutex<Option<PlanarGraph>> = Mutex::new(None);

/// A node in the planar graph (intersection point or endpoint)
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Position of the node
    pub position: Point,
    /// Indices of edges connected to this node
    pub edge_indices: Vec<usize>,
}

impl GraphNode {
    pub fn new(position: Point) -> Self {
        Self {
            position,
            edge_indices: Vec::new(),
        }
    }
}

/// An edge in the planar graph (curve segment between two nodes)
#[derive(Debug, Clone)]
pub struct GraphEdge {
    /// Index of start node
    pub start_node: usize,
    /// Index of end node
    pub end_node: usize,
    /// Original curve ID
    pub curve_id: usize,
    /// Parameter at start of this edge on the original curve [0, 1]
    pub t_start: f64,
    /// Parameter at end of this edge on the original curve [0, 1]
    pub t_end: f64,
}

impl GraphEdge {
    pub fn new(
        start_node: usize,
        end_node: usize,
        curve_id: usize,
        t_start: f64,
        t_end: f64,
    ) -> Self {
        Self {
            start_node,
            end_node,
            curve_id,
            t_start,
            t_end,
        }
    }
}

/// Planar graph structure
#[derive(Clone)]
pub struct PlanarGraph {
    /// All nodes in the graph
    pub nodes: Vec<GraphNode>,
    /// All edges in the graph
    pub edges: Vec<GraphEdge>,
    /// Original curves (referenced by edges)
    pub curves: Vec<CubicBez>,
}

impl PlanarGraph {
    /// Build a planar graph from a collection of curve segments
    ///
    /// # Arguments
    ///
    /// * `curve_segments` - The input curve segments
    ///
    /// # Returns
    ///
    /// A complete planar graph with nodes at all intersections and edges connecting them
    pub fn build(curve_segments: &[CurveSegment]) -> Self {
        println!("PlanarGraph::build started with {} curves", curve_segments.len());

        // Convert curve segments to cubic beziers
        let curves: Vec<CubicBez> = curve_segments
            .iter()
            .map(|seg| seg.to_cubic_bez())
            .collect();

        // Find all intersection points
        let intersections = Self::find_all_intersections(&curves);
        println!("Found {} intersection points", intersections.len());

        // Create nodes and edges
        let (nodes, edges) = Self::build_nodes_and_edges(&curves, intersections);
        println!("Created {} nodes and {} edges", nodes.len(), edges.len());

        let mut graph = Self {
            nodes,
            edges,
            curves,
        };

        // Prune dangling nodes
        graph.prune_dangling_nodes();

        graph
    }

    /// Find all intersections between curves
    ///
    /// Returns a map from curve_id to sorted list of (t_value, point) intersections
    fn find_all_intersections(curves: &[CubicBez]) -> HashMap<usize, Vec<(f64, Point)>> {
        let mut intersections: HashMap<usize, Vec<(f64, Point)>> = HashMap::new();

        // Initialize with endpoints for all curves
        for (i, curve) in curves.iter().enumerate() {
            let mut curve_intersections = vec![
                (0.0, curve.p0),
                (1.0, curve.p3),
            ];
            intersections.insert(i, curve_intersections);
        }

        // Find curve-curve intersections
        println!("Checking {} curve pairs for intersections...", (curves.len() * (curves.len() - 1)) / 2);
        let mut total_intersections = 0;
        for i in 0..curves.len() {
            for j in (i + 1)..curves.len() {
                let curve_i_intersections = find_curve_intersections(&curves[i], &curves[j]);

                if !curve_i_intersections.is_empty() {
                    println!("  Curves {} and {} intersect at {} points:", i, j, curve_i_intersections.len());
                    for (idx, intersection) in curve_i_intersections.iter().enumerate() {
                        println!("    {} - t1={:.3}, t2={:.3}, point=({:.1}, {:.1})",
                            idx, intersection.t1, intersection.t2.unwrap_or(0.0),
                            intersection.point.x, intersection.point.y);
                    }
                    total_intersections += curve_i_intersections.len();
                }

                for intersection in curve_i_intersections {
                    // Add to curve i
                    intersections
                        .get_mut(&i)
                        .unwrap()
                        .push((intersection.t1, intersection.point));

                    // Add to curve j
                    if let Some(t2) = intersection.t2 {
                        intersections
                            .get_mut(&j)
                            .unwrap()
                            .push((t2, intersection.point));
                    }
                }
            }

            // Find self-intersections
            let self_intersections = find_self_intersections(&curves[i]);
            for intersection in self_intersections {
                intersections
                    .get_mut(&i)
                    .unwrap()
                    .push((intersection.t1, intersection.point));
                if let Some(t2) = intersection.t2 {
                    intersections
                        .get_mut(&i)
                        .unwrap()
                        .push((t2, intersection.point));
                }
            }
        }

        println!("Total curve-curve intersections found: {}", total_intersections);

        // Sort and deduplicate intersections for each curve
        for curve_intersections in intersections.values_mut() {
            curve_intersections.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

            // Remove duplicates (points very close together)
            let mut i = 0;
            while i + 1 < curve_intersections.len() {
                let dist = (curve_intersections[i].1 - curve_intersections[i + 1].1).hypot();
                if dist < 0.5 {
                    curve_intersections.remove(i + 1);
                } else {
                    i += 1;
                }
            }
        }

        intersections
    }

    /// Build nodes and edges from curves and their intersections
    fn build_nodes_and_edges(
        curves: &[CubicBez],
        intersections: HashMap<usize, Vec<(f64, Point)>>,
    ) -> (Vec<GraphNode>, Vec<GraphEdge>) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Helper to get or create node at a position
        // Uses distance-based deduplication with 0.5 pixel tolerance
        const NODE_TOLERANCE: f64 = 0.5;
        let get_or_create_node = |position: Point, nodes: &mut Vec<GraphNode>| -> usize {
            // Check if there's already a node within tolerance
            for (idx, node) in nodes.iter().enumerate() {
                let dist = (position - node.position).hypot();
                if dist < NODE_TOLERANCE {
                    return idx;
                }
            }

            // No nearby node found, create new one
            let node_idx = nodes.len();
            nodes.push(GraphNode::new(position));
            node_idx
        };

        // Create edges for each curve
        for (curve_id, curve_intersections) in intersections.iter() {
            // Create edges between consecutive intersection points
            for i in 0..(curve_intersections.len() - 1) {
                let (t_start, p_start) = curve_intersections[i];
                let (t_end, p_end) = curve_intersections[i + 1];

                // Get or create nodes
                let start_node = get_or_create_node(p_start, &mut nodes);
                let end_node = get_or_create_node(p_end, &mut nodes);

                // Create edge
                let edge_idx = edges.len();
                edges.push(GraphEdge::new(
                    start_node,
                    end_node,
                    *curve_id,
                    t_start,
                    t_end,
                ));

                // Add edge to nodes
                nodes[start_node].edge_indices.push(edge_idx);
                nodes[end_node].edge_indices.push(edge_idx);
            }
        }

        (nodes, edges)
    }

    /// Prune dangling nodes (nodes with only one edge) from the graph
    ///
    /// This is useful for cleaning up the graph structure by removing dead ends
    /// that cannot be part of any face. Nodes are pruned iteratively until only
    /// nodes that are part of face loops remain (or the graph becomes empty).
    fn prune_dangling_nodes(&mut self) {
        println!("Starting graph pruning...");

        let mut iteration = 0;
        loop {
            // Find nodes with only 1 edge
            let mut nodes_to_remove = Vec::new();
            for (idx, node) in self.nodes.iter().enumerate() {
                if node.edge_indices.len() == 1 {
                    nodes_to_remove.push(idx);
                }
            }

            if nodes_to_remove.is_empty() {
                println!("Pruning complete after {} iterations", iteration);
                break;
            }

            iteration += 1;
            println!("Pruning iteration {}: removing {} nodes", iteration, nodes_to_remove.len());

            // Find edges connected to these nodes
            let mut edges_to_remove = HashSet::new();
            for &node_idx in &nodes_to_remove {
                for &edge_idx in &self.nodes[node_idx].edge_indices {
                    edges_to_remove.insert(edge_idx);
                }
            }

            // Remove the edges and nodes
            // We need to rebuild the structure since indices change

            // Create new nodes list (excluding removed ones)
            let mut new_nodes = Vec::new();
            let mut old_to_new_node: HashMap<usize, usize> = HashMap::new();

            for (old_idx, node) in self.nodes.iter().enumerate() {
                if !nodes_to_remove.contains(&old_idx) {
                    let new_idx = new_nodes.len();
                    old_to_new_node.insert(old_idx, new_idx);
                    new_nodes.push(node.clone());
                }
            }

            // Create new edges list (excluding removed ones and updating node indices)
            let mut new_edges = Vec::new();
            for (old_idx, edge) in self.edges.iter().enumerate() {
                if !edges_to_remove.contains(&old_idx) {
                    // Update node indices
                    if let (Some(&new_start), Some(&new_end)) =
                        (old_to_new_node.get(&edge.start_node), old_to_new_node.get(&edge.end_node)) {
                        let mut new_edge = edge.clone();
                        new_edge.start_node = new_start;
                        new_edge.end_node = new_end;
                        new_edges.push(new_edge);
                    }
                }
            }

            // Rebuild edge_indices in nodes
            for node in &mut new_nodes {
                node.edge_indices.clear();
            }

            for (edge_idx, edge) in new_edges.iter().enumerate() {
                new_nodes[edge.start_node].edge_indices.push(edge_idx);
                new_nodes[edge.end_node].edge_indices.push(edge_idx);
            }

            // Update graph
            self.nodes = new_nodes;
            self.edges = new_edges;

            println!("After pruning: {} nodes, {} edges", self.nodes.len(), self.edges.len());
        }
    }

    /// Render debug visualization of the planar graph
    ///
    /// Returns two shapes: one for nodes (red circles) and one for edges (yellow lines)
    pub fn render_debug(&self) -> (Shape, Shape) {
        // Render nodes as red circles
        let mut nodes_path = BezPath::new();
        for node in &self.nodes {
            let circle = Circle::new(node.position, 3.0);
            nodes_path.extend(circle.to_path(0.1));
        }
        let nodes_shape = Shape::new(nodes_path).with_stroke(
            ShapeColor::rgb(255, 0, 0),
            StrokeStyle {
                width: 1.0,
                ..Default::default()
            },
        );

        // Render edges as yellow straight lines
        let mut edges_path = BezPath::new();
        for edge in &self.edges {
            let start_pos = self.nodes[edge.start_node].position;
            let end_pos = self.nodes[edge.end_node].position;
            edges_path.move_to(start_pos);
            edges_path.line_to(end_pos);
        }
        let edges_shape = Shape::new(edges_path).with_stroke(
            ShapeColor::rgb(255, 255, 0),
            StrokeStyle {
                width: 0.5,
                ..Default::default()
            },
        );

        (nodes_shape, edges_shape)
    }

    /// Find all faces in the planar graph
    pub fn find_faces(&self) -> Vec<Face> {
        let mut faces = Vec::new();
        let mut used_half_edges = HashSet::new();

        println!("Finding faces: trying {} edges in both directions", self.edges.len());

        // Try starting from each edge in both directions
        for edge_idx in 0..self.edges.len() {
            // Try forward direction
            if !used_half_edges.contains(&(edge_idx, true)) {
                if let Some(face) = self.trace_face(edge_idx, true, &mut used_half_edges) {
                    let start_edge = &self.edges[edge_idx];
                    print!("Successfully traced face {} starting from {} -> {} (edge {} fwd) with {} edges: ",
                        faces.len(), start_edge.start_node, start_edge.end_node, edge_idx, face.edges.len());
                    for (idx, (e, fwd)) in face.edges.iter().enumerate() {
                        let e_obj: &GraphEdge = &self.edges[*e];
                        let (n1, n2) = if *fwd {
                            (e_obj.start_node, e_obj.end_node)
                        } else {
                            (e_obj.end_node, e_obj.start_node)
                        };
                        print!("{} -> {}{}", n1, n2, if idx < face.edges.len() - 1 { " -> " } else { "" });
                    }
                    println!();
                    faces.push(face);
                }
            }

            // Try backward direction
            if !used_half_edges.contains(&(edge_idx, false)) {
                if let Some(face) = self.trace_face(edge_idx, false, &mut used_half_edges) {
                    let start_edge = &self.edges[edge_idx];
                    print!("Successfully traced face {} starting from {} -> {} (edge {} bwd) with {} edges: ",
                        faces.len(), start_edge.end_node, start_edge.start_node, edge_idx, face.edges.len());
                    for (idx, (e, fwd)) in face.edges.iter().enumerate() {
                        let e_obj: &GraphEdge = &self.edges[*e];
                        let (n1, n2) = if *fwd {
                            (e_obj.start_node, e_obj.end_node)
                        } else {
                            (e_obj.end_node, e_obj.start_node)
                        };
                        print!("{} -> {}{}", n1, n2, if idx < face.edges.len() - 1 { " -> " } else { "" });
                    }
                    println!();
                    faces.push(face);
                }
            }
        }

        println!("Found {} faces", faces.len());
        faces
    }

    /// Trace a face starting from an edge in a given direction
    /// Returns None if the face is already traced or invalid
    fn trace_face(
        &self,
        start_edge: usize,
        forward: bool,
        used_half_edges: &mut HashSet<(usize, bool)>,
    ) -> Option<Face> {
        // Use a local set for this trace attempt
        // Only add to global set if we successfully complete a face
        let mut temp_used = HashSet::new();
        let mut edge_sequence = Vec::new();
        let mut visited_nodes = HashSet::new();
        let mut current_edge = start_edge;
        let mut current_forward = forward;

        // Get start node info for logging
        let start_edge_obj = &self.edges[start_edge];
        let (start_node, start_end_node) = if forward {
            (start_edge_obj.start_node, start_edge_obj.end_node)
        } else {
            (start_edge_obj.end_node, start_edge_obj.start_node)
        };

        println!("trace_face: Starting from node {} -> {} (edge {} {})",
            start_node, start_end_node, start_edge, if forward { "fwd" } else { "bwd" });

        // Mark the starting node as visited
        visited_nodes.insert(start_node);

        loop {
            // Check if this half-edge is already used (globally or in this trace)
            if used_half_edges.contains(&(current_edge, current_forward))
                || temp_used.contains(&(current_edge, current_forward)) {
                // Already traced this half-edge
                let current_edge_obj = &self.edges[current_edge];
                let (curr_start, curr_end) = if current_forward {
                    (current_edge_obj.start_node, current_edge_obj.end_node)
                } else {
                    (current_edge_obj.end_node, current_edge_obj.start_node)
                };

                println!("trace_face: Found already-used edge: {} -> {} (edge {} {}) after {} steps",
                    curr_start, curr_end, current_edge, if current_forward { "fwd" } else { "bwd" },
                    edge_sequence.len());

                // Print the full edge sequence to understand the sub-cycle
                print!("  Full sequence: ");
                for (idx, (e, fwd)) in edge_sequence.iter().enumerate() {
                    let e_obj: &GraphEdge = &self.edges[*e];
                    let (n1, n2) = if *fwd {
                        (e_obj.start_node, e_obj.end_node)
                    } else {
                        (e_obj.end_node, e_obj.start_node)
                    };
                    print!("{} -> {}{}", n1, n2, if idx < edge_sequence.len() - 1 { " -> " } else { "" });
                }
                println!(" -> {} -> {} (already used)", curr_start, curr_end);

                return None;
            }

            edge_sequence.push((current_edge, current_forward));
            temp_used.insert((current_edge, current_forward));

            // Get the end node of this half-edge
            let edge = &self.edges[current_edge];
            let start_node_this_edge = if current_forward {
                edge.start_node
            } else {
                edge.end_node
            };
            let end_node = if current_forward {
                edge.end_node
            } else {
                edge.start_node
            };

            // Check if we've returned to the starting node - if so, we've completed the face!
            if end_node == start_node && edge_sequence.len() >= 2 {
                println!("trace_face: Completed cycle back to starting node {} after {} edges", start_node, edge_sequence.len());
                // Success! Add all edges from this trace to the global used set
                for &half_edge in &temp_used {
                    used_half_edges.insert(half_edge);
                }
                return Some(Face { edges: edge_sequence });
            }

            // Check if we've visited this end node before (it's not the start, so it's a self-intersection)
            if visited_nodes.contains(&end_node) {
                println!("trace_face: Detected node revisit at node {} - rejecting self-intersecting path", end_node);
                return None;
            }

            // Mark this node as visited
            visited_nodes.insert(end_node);

            // Find the next edge in counterclockwise order around end_node
            let next = self.find_next_ccw_edge(current_edge, current_forward, end_node);

            if let Some((next_edge, next_forward)) = next {
                current_edge = next_edge;
                current_forward = next_forward;
                // Continue to next iteration
            } else {
                // Dead end - not a valid face
                println!("trace_face: Dead end at node {}", end_node);
                return None;
            }

            // Safety check to prevent infinite loops
            if edge_sequence.len() > self.edges.len() * 2 {
                println!("Warning: Potential infinite loop detected in face tracing");
                return None;
            }
        }
    }

    /// Find the next edge in counterclockwise order around a node
    fn find_next_ccw_edge(
        &self,
        incoming_edge: usize,
        incoming_forward: bool,
        node_idx: usize,
    ) -> Option<(usize, bool)> {
        let node = &self.nodes[node_idx];

        // Get the reverse of the incoming direction (pointing back to where we came FROM)
        // This way, angle 0 = going back, and we measure CCW turns from the incoming edge
        let edge = &self.edges[incoming_edge];
        let incoming_dir = if incoming_forward {
            let start_pos = self.nodes[edge.start_node].position;
            let end_pos = self.nodes[edge.end_node].position;
            // Reverse: point from end back to start
            (start_pos.x - end_pos.x, start_pos.y - end_pos.y)
        } else {
            let start_pos = self.nodes[edge.start_node].position;
            let end_pos = self.nodes[edge.end_node].position;
            // Reverse: point from start back to end
            (end_pos.x - start_pos.x, end_pos.y - start_pos.y)
        };

        // Find all outgoing edges from this node
        let mut candidates = Vec::new();
        for &edge_idx in &node.edge_indices {
            let edge = &self.edges[edge_idx];

            // Check if this edge goes out from node_idx
            if edge.start_node == node_idx {
                // Forward direction
                let end_pos = self.nodes[edge.end_node].position;
                let node_pos = node.position;
                let out_dir = (end_pos.x - node_pos.x, end_pos.y - node_pos.y);
                candidates.push((edge_idx, true, out_dir));
            }

            if edge.end_node == node_idx {
                // Backward direction
                let start_pos = self.nodes[edge.start_node].position;
                let node_pos = node.position;
                let out_dir = (start_pos.x - node_pos.x, start_pos.y - node_pos.y);
                candidates.push((edge_idx, false, out_dir));
            }
        }

        // Debug: show incoming edge info
        let incoming_edge_obj = &self.edges[incoming_edge];
        let (inc_start, inc_end) = if incoming_forward {
            (incoming_edge_obj.start_node, incoming_edge_obj.end_node)
        } else {
            (incoming_edge_obj.end_node, incoming_edge_obj.start_node)
        };

        println!("  find_next_ccw_edge at node {} (incoming: {} -> {}, edge {} {})",
            node_idx, inc_start, inc_end, incoming_edge, if incoming_forward { "fwd" } else { "bwd" });
        println!("    Available edges ({} candidates):", candidates.len());

        // Find the edge with the largest CCW angle (rightmost turn for face tracing)
        // Since incoming_dir points back to where we came from, the largest angle
        // gives us the rightmost turn, which traces faces correctly.
        let mut best_edge = None;
        let mut best_angle = 0.0;

        for &(edge_idx, forward, out_dir) in &candidates {
            // Skip the edge we came from (in opposite direction)
            if edge_idx == incoming_edge && forward == !incoming_forward {
                println!("      Edge {} {} -> SKIP (reverse of incoming)", edge_idx, if forward { "fwd" } else { "bwd" });
                continue;
            }

            // Compute angle from incoming to outgoing (counterclockwise)
            let angle = angle_between_ccw(incoming_dir, out_dir);

            // Get the destination node for this candidate
            let cand_edge = &self.edges[edge_idx];
            let dest_node = if forward { cand_edge.end_node } else { cand_edge.start_node };

            println!("      Edge {} {} -> node {} (angle: {:.3} rad = {:.1}°){}",
                edge_idx, if forward { "fwd" } else { "bwd" }, dest_node,
                angle, angle.to_degrees(),
                if angle > best_angle { " <- BEST" } else { "" });

            if angle > best_angle {
                best_angle = angle;
                best_edge = Some((edge_idx, forward));
            }
        }

        if best_edge.is_none() {
            println!("    FAILED: No valid next edge found!");
        }

        best_edge
    }

    /// Find which face contains a given point
    pub fn find_face_containing_point(&self, point: Point, faces: &[Face]) -> Option<usize> {
        for (i, face) in faces.iter().enumerate() {
            // Build polygon for debugging
            let mut polygon_points = Vec::new();
            for &(edge_idx, forward) in &face.edges {
                let edge = &self.edges[edge_idx];
                let node_idx = if forward { edge.start_node } else { edge.end_node };
                polygon_points.push(self.nodes[node_idx].position);
            }

            // Calculate bounding box
            let mut min_x = f64::MAX;
            let mut max_x = f64::MIN;
            let mut min_y = f64::MAX;
            let mut max_y = f64::MIN;
            for p in &polygon_points {
                min_x = min_x.min(p.x);
                max_x = max_x.max(p.x);
                min_y = min_y.min(p.y);
                max_y = max_y.max(p.y);
            }

            println!("Face {}: {} edges, {} points, bbox: ({:.1},{:.1}) to ({:.1},{:.1})",
                i, face.edges.len(), polygon_points.len(), min_x, min_y, max_x, max_y);

            if self.point_in_face(point, face) {
                return Some(i);
            }
        }
        None
    }

    /// Test if a point is inside a face using ray casting
    fn point_in_face(&self, point: Point, face: &Face) -> bool {
        // Build polygon from face edges
        let mut polygon_points = Vec::new();

        for &(edge_idx, forward) in &face.edges {
            let edge = &self.edges[edge_idx];
            let node_idx = if forward { edge.start_node } else { edge.end_node };
            polygon_points.push(self.nodes[node_idx].position);
        }

        // Ray casting algorithm
        point_in_polygon(point, &polygon_points)
    }

    /// Build a BezPath from a face using the actual curve segments
    pub fn build_face_path(&self, face: &Face) -> BezPath {
        use vello::kurbo::ParamCurve;

        let mut path = BezPath::new();
        let mut first = true;

        for &(edge_idx, forward) in &face.edges {
            let edge = &self.edges[edge_idx];
            let orig_curve = &self.curves[edge.curve_id];

            // Get the curve segment for this edge
            let segment = if forward {
                orig_curve.subsegment(edge.t_start..edge.t_end)
            } else {
                // Reverse the segment
                orig_curve.subsegment(edge.t_end..edge.t_start)
            };

            if first {
                path.move_to(segment.p0);
                first = false;
            }

            // Add the curve segment
            path.curve_to(segment.p1, segment.p2, segment.p3);
        }

        path.close_path();
        path
    }
}

/// A face in the planar graph (bounded region)
#[derive(Debug, Clone)]
pub struct Face {
    /// Sequence of (edge_index, is_forward) pairs that form the boundary
    pub edges: Vec<(usize, bool)>,
}

/// Compute the counterclockwise angle from v1 to v2
fn angle_between_ccw(v1: (f64, f64), v2: (f64, f64)) -> f64 {
    let angle1 = v1.1.atan2(v1.0);
    let angle2 = v2.1.atan2(v2.0);
    let mut diff = angle2 - angle1;

    // Normalize to [0, 2π)
    while diff < 0.0 {
        diff += 2.0 * std::f64::consts::PI;
    }
    while diff >= 2.0 * std::f64::consts::PI {
        diff -= 2.0 * std::f64::consts::PI;
    }

    diff
}

/// Test if a point is inside a polygon using ray casting
fn point_in_polygon(point: Point, polygon: &[Point]) -> bool {
    let mut inside = false;
    let n = polygon.len();

    for i in 0..n {
        let j = (i + 1) % n;
        let pi = polygon[i];
        let pj = polygon[j];

        if ((pi.y > point.y) != (pj.y > point.y)) &&
           (point.x < (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x) {
            inside = !inside;
        }
    }

    inside
}
