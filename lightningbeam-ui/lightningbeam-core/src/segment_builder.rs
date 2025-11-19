//! Segment builder for constructing filled paths from boundary points
//!
//! This module takes boundary points from flood fill and builds a closed path
//! by extracting curve segments and connecting them with intersections or bridges.

use crate::curve_intersection::{deduplicate_intersections, find_intersections};
use crate::curve_segment::CurveSegment;
use crate::flood_fill::BoundaryPoint;
use std::collections::HashMap;
use vello::kurbo::{BezPath, Point, Shape};

/// Configuration for segment building
pub struct SegmentBuilderConfig {
    /// Maximum gap to bridge with a line segment
    pub gap_threshold: f64,
    /// Threshold for curve intersection detection
    pub intersection_threshold: f64,
}

impl Default for SegmentBuilderConfig {
    fn default() -> Self {
        Self {
            gap_threshold: 2.0,
            intersection_threshold: 0.5,
        }
    }
}

/// A curve segment extracted from boundary points
#[derive(Debug, Clone)]
struct ExtractedSegment {
    /// Original curve index
    curve_index: usize,
    /// Minimum parameter value from boundary points
    t_min: f64,
    /// Maximum parameter value from boundary points
    t_max: f64,
    /// The curve segment (trimmed to [t_min, t_max])
    segment: CurveSegment,
}

/// Build a closed path from boundary points
///
/// This function:
/// 1. Groups boundary points by curve
/// 2. Extracts curve segments for each group
/// 3. Connects adjacent segments (trimming at intersections or bridging gaps)
/// 4. Returns a closed BezPath
///
/// Returns None if the region cannot be closed (gaps too large, etc.)
///
/// The click_point parameter is used to verify that the found cycle actually
/// contains the clicked region.
pub fn build_path_from_boundary(
    boundary_points: &[BoundaryPoint],
    all_curves: &[CurveSegment],
    config: &SegmentBuilderConfig,
    click_point: Point,
) -> Option<BezPath> {
    if boundary_points.is_empty() {
        println!("build_path_from_boundary: No boundary points");
        return None;
    }

    println!("build_path_from_boundary: Processing {} boundary points", boundary_points.len());

    // Step 1: Group boundary points by curve and find parameter ranges
    let extracted_segments = extract_segments(boundary_points, all_curves, click_point)?;

    println!("build_path_from_boundary: Extracted {} segments", extracted_segments.len());

    if extracted_segments.is_empty() {
        println!("build_path_from_boundary: No segments extracted");
        return None;
    }

    // Step 2: Connect segments to form a closed path that contains the click point
    let connected_segments = connect_segments(&extracted_segments, config, click_point)?;

    println!("build_path_from_boundary: Connected {} segments", connected_segments.len());

    // Step 3: Build the final BezPath
    Some(build_bez_path(&connected_segments))
}

/// Split segments at intersection points
/// This handles cases where curves cross in an X pattern
fn split_segments_at_intersections(segments: Vec<ExtractedSegment>) -> Vec<ExtractedSegment> {
    use crate::curve_intersection::find_intersections;

    let mut result = Vec::new();
    let mut split_points: HashMap<usize, Vec<f64>> = HashMap::new();

    // Find all intersections between segments
    for i in 0..segments.len() {
        for j in (i + 1)..segments.len() {
            let intersections = find_intersections(&segments[i].segment, &segments[j].segment, 0.5);

            for intersection in intersections {
                // Record intersection parameters for both segments
                split_points.entry(i).or_default().push(intersection.t1);
                split_points.entry(j).or_default().push(intersection.t2);
            }
        }
    }

    println!("split_segments_at_intersections: Found {} segments with intersections", split_points.len());

    // Split each segment at its intersection points
    let original_count = segments.len();
    for (idx, seg) in segments.into_iter().enumerate() {
        if let Some(splits) = split_points.get(&idx) {
            if splits.is_empty() {
                result.push(seg);
                continue;
            }

            // Sort split points
            let mut sorted_splits = splits.clone();
            sorted_splits.sort_by(|a, b| a.partial_cmp(b).unwrap());

            // Add endpoints
            let mut all_t = vec![0.0];
            all_t.extend(sorted_splits.iter().copied());
            all_t.push(1.0);

            println!("  Splitting segment {} at {} points", idx, sorted_splits.len());

            // Create sub-segments
            for i in 0..(all_t.len() - 1) {
                let t_start = all_t[i];
                let t_end = all_t[i + 1];

                if (t_end - t_start).abs() < 0.001 {
                    continue; // Skip very small segments
                }

                // Create a subsegment with adjusted t parameters
                // The control_points stay the same, but we update t_start/t_end
                let subseg = CurveSegment::new(
                    seg.segment.shape_index,
                    seg.segment.segment_index,
                    seg.segment.curve_type,
                    t_start,
                    t_end,
                    seg.segment.control_points.clone(),
                );

                result.push(ExtractedSegment {
                    curve_index: seg.curve_index,
                    t_min: t_start,
                    t_max: t_end,
                    segment: subseg,
                });
            }
        } else {
            // No intersections, keep as-is
            result.push(seg);
        }
    }

    println!("split_segments_at_intersections: {} segments -> {} segments after splitting", original_count, result.len());
    result
}

/// Group boundary points by curve and extract segments
fn extract_segments(
    boundary_points: &[BoundaryPoint],
    all_curves: &[CurveSegment],
    click_point: Point,
) -> Option<Vec<ExtractedSegment>> {
    // Find the closest boundary point to the click
    // Boundary points come from flood fill, so they're already from the correct region
    println!("extract_segments: {} boundary points from flood fill", boundary_points.len());
    println!("extract_segments: Click point: ({:.1}, {:.1})", click_point.x, click_point.y);

    // Debug: print distribution of boundary points by curve
    let mut curve_counts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for bp in boundary_points.iter() {
        *curve_counts.entry(bp.curve_index).or_insert(0) += 1;
    }
    println!("extract_segments: Boundary points by curve:");
    for (curve_idx, count) in curve_counts.iter() {
        println!("  Curve {}: {} points", curve_idx, count);
    }

    // Debug: print first 5 boundary points
    println!("extract_segments: First 5 boundary points:");
    for (i, bp) in boundary_points.iter().take(5).enumerate() {
        println!("  {}: ({:.1}, {:.1}) curve {}", i, bp.point.x, bp.point.y, bp.curve_index);
    }

    let mut closest_distance = f64::MAX;
    let mut closest_boundary_point: Option<&BoundaryPoint> = None;

    for bp in boundary_points {
        let distance = click_point.distance(bp.point);
        if distance < closest_distance {
            closest_distance = distance;
            closest_boundary_point = Some(bp);
        }
    }

    let start_curve_idx = match closest_boundary_point {
        Some(bp) => {
            println!(
                "extract_segments: Nearest boundary point at ({:.1}, {:.1}), distance: {:.1}, curve: {}",
                bp.point.x, bp.point.y, closest_distance, bp.curve_index
            );
            bp.curve_index
        }
        None => {
            println!("extract_segments: No boundary points found");
            return None;
        }
    };

    // We don't need to track nearest_point and nearest_t for the segment finding
    // Just use the curve index to find segments after splitting

    // Group points by curve index
    let mut curve_points: HashMap<usize, Vec<&BoundaryPoint>> = HashMap::new();
    for bp in boundary_points {
        curve_points.entry(bp.curve_index).or_default().push(bp);
    }

    // Extract segment for each curve
    let mut segments = Vec::new();
    for (curve_idx, points) in curve_points {
        if points.is_empty() {
            continue;
        }

        // Find min and max t parameters
        let t_min = points
            .iter()
            .map(|p| p.t)
            .min_by(|a, b| a.partial_cmp(b).unwrap())?;
        let t_max = points
            .iter()
            .map(|p| p.t)
            .max_by(|a, b| a.partial_cmp(b).unwrap())?;

        if curve_idx >= all_curves.len() {
            continue;
        }

        let original_curve = &all_curves[curve_idx];

        // Use the full curve (t=0 to t=1) rather than just the portion touched by boundary points
        // This ensures we don't create artificial gaps in closed regions
        let segment = CurveSegment::new(
            original_curve.shape_index,
            original_curve.segment_index,
            original_curve.curve_type,
            0.0,  // Use full curve from start
            1.0,  // to end
            original_curve.control_points.clone(),
        );

        segments.push(ExtractedSegment {
            curve_index: curve_idx,
            t_min,
            t_max,
            segment,
        });
    }

    if segments.is_empty() {
        return None;
    }

    // Split segments at intersection points
    segments = split_segments_at_intersections(segments);

    // Find a segment from the ray-intersected curve to use as starting point
    let start_segment_idx = segments
        .iter()
        .position(|seg| seg.curve_index == start_curve_idx);

    let start_segment_idx = match start_segment_idx {
        Some(idx) => {
            println!("extract_segments: Starting from segment {} (curve {})", idx, start_curve_idx);
            idx
        }
        None => {
            println!("extract_segments: No segment found for start curve {}", start_curve_idx);
            return None;
        }
    };

    // Reorder segments using graph-based cycle detection
    // This finds the correct closed loop instead of greedy nearest-neighbor
    // Higher threshold needed for split curves at intersections (floating point precision)
    const CONNECTION_THRESHOLD: f64 = 5.0; // Endpoints within this distance can connect

    // Try to find a valid cycle that contains the click point
    // Start from the specific segment where the nearest boundary point was found
    // BFS will naturally only explore segments connected to this starting segment
    match find_segment_cycle(&segments, CONNECTION_THRESHOLD, click_point, start_segment_idx) {
        Some(ordered_segments) => {
            println!("extract_segments: Found valid cycle with {} segments", ordered_segments.len());
            Some(ordered_segments)
        }
        None => {
            println!("extract_segments: Could not find valid cycle through all segments");
            None
        }
    }
}

/// Adjacency information for segment connections
struct SegmentConnections {
    // Segments that can connect to the start point (when this segment is forward)
    connects_to_start: Vec<(usize, bool, f64)>, // (index, reversed, distance)
    // Segments that can connect to the end point (when this segment is forward)
    connects_to_end: Vec<(usize, bool, f64)>,
}

/// Find a cycle through segments that contains the click point
/// Returns segments in order with proper orientation
/// Starts ONLY from the given segment index
fn find_segment_cycle(
    segments: &[ExtractedSegment],
    threshold: f64,
    click_point: Point,
    start_segment_idx: usize,
) -> Option<Vec<ExtractedSegment>> {
    if segments.is_empty() {
        return None;
    }

    println!("find_segment_cycle: Searching for cycle through {} segments", segments.len());

    let mut connections: Vec<SegmentConnections> = (0..segments.len())
        .map(|_| SegmentConnections {
            connects_to_start: Vec::new(),
            connects_to_end: Vec::new(),
        })
        .collect();

    // Build connectivity graph
    for i in 0..segments.len() {
        for j in 0..segments.len() {
            if i == j {
                continue;
            }

            let seg_i = &segments[i];
            let seg_j = &segments[j];

            // Check all four possible connections:
            // 1. seg_i end -> seg_j start (both forward)
            let dist_end_to_start = seg_i.segment.end_point().distance(seg_j.segment.start_point());
            if dist_end_to_start < threshold {
                connections[i].connects_to_end.push((j, false, dist_end_to_start));
            }

            // 2. seg_i end -> seg_j end (j reversed)
            let dist_end_to_end = seg_i.segment.end_point().distance(seg_j.segment.end_point());
            if dist_end_to_end < threshold {
                connections[i].connects_to_end.push((j, true, dist_end_to_end));
            }

            // 3. seg_i start -> seg_j start (both forward, but we'd traverse i backwards)
            let dist_start_to_start = seg_i.segment.start_point().distance(seg_j.segment.start_point());
            if dist_start_to_start < threshold {
                connections[i].connects_to_start.push((j, false, dist_start_to_start));
            }

            // 4. seg_i start -> seg_j end (j reversed, i backwards)
            let dist_start_to_end = seg_i.segment.start_point().distance(seg_j.segment.end_point());
            if dist_start_to_end < threshold {
                connections[i].connects_to_start.push((j, true, dist_start_to_end));
            }
        }
    }

    // Debug: Print connectivity information
    for i in 0..segments.len() {
        println!(
            "  Segment {}: {} connections from end, {} from start",
            i,
            connections[i].connects_to_end.len(),
            connections[i].connects_to_start.len()
        );
    }

    // Use BFS to find the shortest cycle that contains the click point
    // BFS naturally explores shorter paths first
    // Start ONLY from the specified segment
    bfs_find_shortest_cycle(&segments, &connections, threshold, click_point, start_segment_idx)
}

/// Build a BezPath from ExtractedSegments (helper for testing containment)
fn build_bez_path_from_segments(segments: &[ExtractedSegment]) -> BezPath {
    let mut path = BezPath::new();

    if segments.is_empty() {
        return path;
    }

    // Start at the first point
    let start_point = segments[0].segment.start_point();
    path.move_to(start_point);

    // Add all segments
    for seg in segments {
        let element = seg.segment.to_path_element();
        path.push(element);
    }

    // Close the path
    path.close_path();

    path
}

/// BFS to find the shortest cycle that contains the click point
/// Returns the first (shortest) cycle found that contains the click point
/// Starts ONLY from the specified segment index
fn bfs_find_shortest_cycle(
    segments: &[ExtractedSegment],
    connections: &[SegmentConnections],
    threshold: f64,
    click_point: Point,
    start_segment_idx: usize,
) -> Option<Vec<ExtractedSegment>> {
    use std::collections::VecDeque;

    // State: (current_segment_idx, current_reversed, path so far, visited set)
    type State = (usize, bool, Vec<(usize, bool)>, Vec<bool>);

    if start_segment_idx >= segments.len() {
        println!("bfs_find_shortest_cycle: Invalid start segment index {}", start_segment_idx);
        return None;
    }

    println!("bfs_find_shortest_cycle: Starting ONLY from segment {} (curve {})",
             start_segment_idx, segments[start_segment_idx].curve_index);

    // Try starting from the one specified segment, in both orientations
    for start_reversed in [false, true] {
        let mut queue: VecDeque<State> = VecDeque::new();
        let mut visited = vec![false; segments.len()];
        visited[start_segment_idx] = true;

        queue.push_back((
            start_segment_idx,
            start_reversed,
            vec![(start_segment_idx, start_reversed)],
            visited.clone(),
        ));

        while let Some((current_idx, current_reversed, path, visited)) = queue.pop_front() {
                // Check if we can close the cycle (need at least 3 segments)
                if path.len() >= 3 {
                    let first = &path[0];
                    let current_end = if current_reversed {
                        segments[current_idx].segment.start_point()
                    } else {
                        segments[current_idx].segment.end_point()
                    };

                    let first_start = if first.1 {
                        segments[first.0].segment.end_point()
                    } else {
                        segments[first.0].segment.start_point()
                    };

                    let closing_gap = current_end.distance(first_start);
                    if closing_gap < threshold {
                        // Build final segment list with proper orientations
                        let mut result = Vec::new();
                        for (idx, reversed) in path.iter() {
                            let mut seg = segments[*idx].clone();
                            if *reversed {
                                seg.segment.control_points.reverse();
                            }
                            result.push(seg);
                        }

                        // Check if this cycle contains the click point
                        let test_path = build_bez_path_from_segments(&result);
                        let bbox = test_path.bounding_box();
                        let winding = test_path.winding(click_point);

                        println!(
                            "  Testing {}-segment cycle: bbox=({:.1},{:.1})-({:.1},{:.1}), click=({:.1},{:.1}), winding={}",
                            result.len(),
                            bbox.x0, bbox.y0, bbox.x1, bbox.y1,
                            click_point.x, click_point.y, winding
                        );

                        if winding != 0 {
                            println!(
                                "bfs_find_shortest_cycle: Found cycle with {} segments (closing gap: {:.2}, winding: {})",
                                path.len(),
                                closing_gap,
                                winding
                            );
                            return Some(result);
                        } else {
                            println!(
                                "bfs_find_shortest_cycle: Cycle doesn't contain click point (winding: 0), continuing search..."
                            );
                        }
                    }
                }

                // Explore neighbors
                let next_connections = if current_reversed {
                    &connections[current_idx].connects_to_start
                } else {
                    &connections[current_idx].connects_to_end
                };

                for (next_idx, next_reversed, _dist) in next_connections {
                    if !visited[*next_idx] {
                        let mut new_path = path.clone();
                        new_path.push((*next_idx, *next_reversed));

                        let mut new_visited = visited.clone();
                        new_visited[*next_idx] = true;

                        queue.push_back((*next_idx, *next_reversed, new_path, new_visited));
                    }
                }
            }
        }

    println!("bfs_find_shortest_cycle: No cycle found");
    None
}

/// Connected segment in the final path
#[derive(Debug, Clone)]
enum ConnectedSegment {
    /// A curve segment from the original geometry
    Curve {
        segment: CurveSegment,
        start: Point,
        end: Point,
    },
    /// A line segment bridging a gap
    Line { start: Point, end: Point },
}

/// Connect extracted segments into a closed path that contains the click point
fn connect_segments(
    extracted: &[ExtractedSegment],
    config: &SegmentBuilderConfig,
    click_point: Point,
) -> Option<Vec<ConnectedSegment>> {
    if extracted.is_empty() {
        println!("connect_segments: No segments to connect");
        return None;
    }

    println!("connect_segments: Connecting {} segments", extracted.len());

    let mut connected = Vec::new();

    for i in 0..extracted.len() {
        let current = &extracted[i];
        let next = &extracted[(i + 1) % extracted.len()];

        // Get the current segment's endpoint
        let current_end = current.segment.eval_at(1.0);

        // Get the next segment's start point
        let next_start = next.segment.eval_at(0.0);

        // Add the current curve segment
        connected.push(ConnectedSegment::Curve {
            segment: current.segment.clone(),
            start: current.segment.eval_at(0.0),
            end: current_end,
        });

        // Check if we need to connect to the next segment
        let gap = current_end.distance(next_start);

        println!("connect_segments: Gap between segment {} and {} is {:.2}", i, (i + 1) % extracted.len(), gap);

        if gap < 0.01 {
            // Close enough, no bridge needed
            continue;
        } else if gap < config.gap_threshold {
            // Bridge with a line segment
            println!("connect_segments: Bridging gap with line segment");
            connected.push(ConnectedSegment::Line {
                start: current_end,
                end: next_start,
            });
        } else {
            // Try to find intersection
            println!("connect_segments: Gap too large ({:.2}), trying to find intersection", gap);
            let intersections = find_intersections(
                &current.segment,
                &next.segment,
                config.intersection_threshold,
            );

            println!("connect_segments: Found {} intersections", intersections.len());

            if !intersections.is_empty() {
                // Use the first intersection to trim segments
                let deduplicated = deduplicate_intersections(&intersections, 0.1);
                println!("connect_segments: After deduplication: {} intersections", deduplicated.len());
                if !deduplicated.is_empty() {
                    // TODO: Properly trim the segments at the intersection
                    // For now, just bridge the gap
                    println!("connect_segments: Bridging gap at intersection");
                    connected.push(ConnectedSegment::Line {
                        start: current_end,
                        end: next_start,
                    });
                } else {
                    // Gap too large and no intersection - fail
                    println!("connect_segments: FAILED - Gap too large and no deduplicated intersections");
                    return None;
                }
            } else {
                // Try bridging if within threshold
                if gap < config.gap_threshold * 2.0 {
                    println!("connect_segments: Bridging gap (within 2x threshold)");
                    connected.push(ConnectedSegment::Line {
                        start: current_end,
                        end: next_start,
                    });
                } else {
                    println!("connect_segments: FAILED - Gap too large ({:.2}) and no intersections", gap);
                    return None;
                }
            }
        }
    }

    println!("connect_segments: Successfully connected all segments");
    Some(connected)
}

/// Build a BezPath from connected segments
fn build_bez_path(segments: &[ConnectedSegment]) -> BezPath {
    let mut path = BezPath::new();

    if segments.is_empty() {
        return path;
    }

    // Start at the first point
    let start_point = match &segments[0] {
        ConnectedSegment::Curve { start, .. } => *start,
        ConnectedSegment::Line { start, .. } => *start,
    };

    path.move_to(start_point);

    // Add all segments
    for segment in segments {
        match segment {
            ConnectedSegment::Curve { segment, .. } => {
                let element = segment.to_path_element();
                path.push(element);
            }
            ConnectedSegment::Line { end, .. } => {
                path.line_to(*end);
            }
        }
    }

    // Close the path
    path.close_path();

    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve_segment::CurveType;

    #[test]
    fn test_extract_segments_basic() {
        let curves = vec![
            CurveSegment::new(
                0,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
            ),
            CurveSegment::new(
                1,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(100.0, 0.0), Point::new(100.0, 100.0)],
            ),
        ];

        let boundary_points = vec![
            BoundaryPoint {
                point: Point::new(25.0, 0.0),
                curve_index: 0,
                t: 0.25,
                nearest_point: Point::new(25.0, 0.0),
                distance: 0.0,
            },
            BoundaryPoint {
                point: Point::new(75.0, 0.0),
                curve_index: 0,
                t: 0.75,
                nearest_point: Point::new(75.0, 0.0),
                distance: 0.0,
            },
            BoundaryPoint {
                point: Point::new(100.0, 50.0),
                curve_index: 1,
                t: 0.5,
                nearest_point: Point::new(100.0, 50.0),
                distance: 0.0,
            },
        ];

        let segments = extract_segments(&boundary_points, &curves).unwrap();

        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].curve_index, 0);
        assert!((segments[0].t_min - 0.25).abs() < 1e-6);
        assert!((segments[0].t_max - 0.75).abs() < 1e-6);
    }

    #[test]
    fn test_build_simple_path() {
        let curves = vec![
            CurveSegment::new(
                0,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0)],
            ),
            CurveSegment::new(
                1,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(100.0, 0.0), Point::new(100.0, 100.0)],
            ),
            CurveSegment::new(
                2,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(100.0, 100.0), Point::new(0.0, 100.0)],
            ),
            CurveSegment::new(
                3,
                0,
                CurveType::Line,
                0.0,
                1.0,
                vec![Point::new(0.0, 100.0), Point::new(0.0, 0.0)],
            ),
        ];

        let boundary_points = vec![
            BoundaryPoint {
                point: Point::new(50.0, 0.0),
                curve_index: 0,
                t: 0.5,
                nearest_point: Point::new(50.0, 0.0),
                distance: 0.0,
            },
            BoundaryPoint {
                point: Point::new(100.0, 50.0),
                curve_index: 1,
                t: 0.5,
                nearest_point: Point::new(100.0, 50.0),
                distance: 0.0,
            },
            BoundaryPoint {
                point: Point::new(50.0, 100.0),
                curve_index: 2,
                t: 0.5,
                nearest_point: Point::new(50.0, 100.0),
                distance: 0.0,
            },
            BoundaryPoint {
                point: Point::new(0.0, 50.0),
                curve_index: 3,
                t: 0.5,
                nearest_point: Point::new(0.0, 50.0),
                distance: 0.0,
            },
        ];

        let config = SegmentBuilderConfig::default();
        let click_point = Point::new(50.0, 50.0); // Center of the test square
        let path = build_path_from_boundary(&boundary_points, &curves, &config, click_point);

        assert!(path.is_some());
        let path = path.unwrap();

        // Should have a closed path
        assert!(!path.elements().is_empty());
    }
}
