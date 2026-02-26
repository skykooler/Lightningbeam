//! Region selection path clipping
//!
//! Clips BezPaths against a closed polygon region (rectangle or lasso),
//! producing separate inside and outside paths.
//!
//! Uses a Weiler-Atherton-style approach: walk the subject path, alternating
//! between following the subject (when inside) and following the clip boundary
//! (when transitioning between crossings).

use vello::kurbo::{
    BezPath, CubicBez, Line, ParamCurve, PathEl, Point, Rect, Shape as KurboShape,
};

/// Result of clipping a shape path against a region
#[derive(Debug, Clone)]
pub struct ClipResult {
    /// Path segments inside the region
    pub inside: BezPath,
    /// Path segments outside the region
    pub outside: BezPath,
}

/// Convert a Rect to a closed BezPath (4 line segments)
pub fn rect_to_path(rect: Rect) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(Point::new(rect.x0, rect.y0));
    path.line_to(Point::new(rect.x1, rect.y0));
    path.line_to(Point::new(rect.x1, rect.y1));
    path.line_to(Point::new(rect.x0, rect.y1));
    path.close_path();
    path
}

/// Convert a list of lasso points to a closed BezPath (polygon)
pub fn lasso_to_path(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    if points.is_empty() {
        return path;
    }
    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
    path
}

/// Test if a point is inside a closed region using winding number
fn point_in_region(point: Point, region: &BezPath) -> bool {
    region.winding(point) != 0
}

/// Extract line segments from a region path (which is always a polygon)
fn region_line_segments(region: &BezPath) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;

    for el in region.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                current = p;
                subpath_start = p;
            }
            PathEl::LineTo(p) => {
                lines.push(Line::new(current, p));
                current = p;
            }
            PathEl::ClosePath => {
                if dist(current, subpath_start) > 1e-10 {
                    lines.push(Line::new(current, subpath_start));
                }
                current = subpath_start;
            }
            PathEl::QuadTo(_, p) => {
                lines.push(Line::new(current, p));
                current = p;
            }
            PathEl::CurveTo(_, _, p) => {
                lines.push(Line::new(current, p));
                current = p;
            }
        }
    }
    lines
}

fn dist(a: Point, b: Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

// ── Line-line intersection (exact, no cubic conversion) ──────────────────

/// Find the intersection of two line segments.
/// Returns (t1, t2) parameters on line1 and line2 respectively, or None.
fn line_line_intersection(l1: &Line, l2: &Line) -> Option<(f64, f64)> {
    let d1x = l1.p1.x - l1.p0.x;
    let d1y = l1.p1.y - l1.p0.y;
    let d2x = l2.p1.x - l2.p0.x;
    let d2y = l2.p1.y - l2.p0.y;

    let denom = d1x * d2y - d1y * d2x;
    if denom.abs() < 1e-12 {
        return None; // Parallel
    }

    let dx = l2.p0.x - l1.p0.x;
    let dy = l2.p0.y - l1.p0.y;

    let t1 = (dx * d2y - dy * d2x) / denom;
    let t2 = (dx * d1y - dy * d1x) / denom;

    // Both parameters must be in [0, 1] for segments to intersect
    // Use a small epsilon to avoid edge-case issues at endpoints
    let eps = 1e-9;
    if t1 >= -eps && t1 <= 1.0 + eps && t2 >= -eps && t2 <= 1.0 + eps {
        Some((t1.clamp(0.0, 1.0), t2.clamp(0.0, 1.0)))
    } else {
        None
    }
}

/// Find intersection of a cubic bezier with a line segment.
/// Returns list of t-parameters on the cubic where it crosses the line.
fn cubic_line_intersections(cubic: &CubicBez, line: &Line) -> Vec<f64> {
    // Express the line as ax + by + c = 0
    let lx = line.p1.x - line.p0.x;
    let ly = line.p1.y - line.p0.y;
    let line_len_sq = lx * lx + ly * ly;
    if line_len_sq < 1e-20 {
        return Vec::new();
    }

    // Normal to the line
    let a = -ly;
    let b = lx;
    let c = -(a * line.p0.x + b * line.p0.y);

    // Evaluate signed distance of each control point to the line
    let d0 = a * cubic.p0.x + b * cubic.p0.y + c;
    let d1 = a * cubic.p1.x + b * cubic.p1.y + c;
    let d2 = a * cubic.p2.x + b * cubic.p2.y + c;
    let d3 = a * cubic.p3.x + b * cubic.p3.y + c;

    // Cubic polynomial coefficients: d(t) = at^3 + bt^2 + ct + d
    // where d(t) is the signed distance at parameter t
    let ca = -d0 + 3.0 * d1 - 3.0 * d2 + d3;
    let cb = 3.0 * d0 - 6.0 * d1 + 3.0 * d2;
    let cc = -3.0 * d0 + 3.0 * d1;
    let cd = d0;

    let roots = solve_cubic(ca, cb, cc, cd);

    // Filter: t must be in [0,1] and the point must lie on the line segment
    let eps = 1e-6;
    let mut result = Vec::new();
    for t in roots {
        if t < -eps || t > 1.0 + eps {
            continue;
        }
        let t = t.clamp(0.0, 1.0);
        let p = cubic.eval(t);

        // Check if point is on the line segment by projecting
        let dx = p.x - line.p0.x;
        let dy = p.y - line.p0.y;
        let s = (dx * lx + dy * ly) / line_len_sq;
        if s >= -eps && s <= 1.0 + eps {
            // Avoid duplicate t values
            if !result.iter().any(|&existing: &f64| (existing - t).abs() < 1e-6) {
                result.push(t);
            }
        }
    }

    result.sort_by(|a, b| a.partial_cmp(b).unwrap());
    result
}

/// Solve cubic equation at^3 + bt^2 + ct + d = 0
/// Returns real roots.
fn solve_cubic(a: f64, b: f64, c: f64, d: f64) -> Vec<f64> {
    if a.abs() < 1e-12 {
        // Degenerate to quadratic
        return solve_quadratic(b, c, d);
    }

    // Normalize: t^3 + pt^2 + qt + r = 0
    let p = b / a;
    let q = c / a;
    let r = d / a;

    // Depressed cubic substitution: t = u - p/3
    // u^3 + Au + B = 0
    let a2 = q - p * p / 3.0;
    let b2 = r - p * q / 3.0 + 2.0 * p * p * p / 27.0;

    let discriminant = b2 * b2 / 4.0 + a2 * a2 * a2 / 27.0;

    let mut roots = Vec::new();

    if discriminant.abs() < 1e-14 {
        // Triple or double root
        if a2.abs() < 1e-12 {
            roots.push(-p / 3.0);
        } else {
            let u = (b2 / 2.0).cbrt();
            roots.push(2.0 * u - p / 3.0);  // wait, this is wrong for the double root case
            // Actually: u^3 + Au + B = 0 with disc=0
            // roots: -2*(B/2)^(1/3) and (B/2)^(1/3) (double)
            roots.clear();
            let cb = if b2 > 0.0 { -(b2 / 2.0).cbrt() } else { (-b2 / 2.0).cbrt() };
            roots.push(2.0 * cb - p / 3.0);
            roots.push(-cb - p / 3.0);
        }
    } else if discriminant > 0.0 {
        // One real root
        let sq = discriminant.sqrt();
        let u = cbrt(-b2 / 2.0 + sq);
        let v = cbrt(-b2 / 2.0 - sq);
        roots.push(u + v - p / 3.0);
    } else {
        // Three real roots (casus irreducibilis)
        let r_mag = (-a2 * a2 * a2 / 27.0).sqrt();
        let theta = (-b2 / (2.0 * r_mag)).acos();
        let m = 2.0 * (r_mag).cbrt();

        roots.push(m * (theta / 3.0).cos() - p / 3.0);
        roots.push(m * ((theta + 2.0 * std::f64::consts::PI) / 3.0).cos() - p / 3.0);
        roots.push(m * ((theta + 4.0 * std::f64::consts::PI) / 3.0).cos() - p / 3.0);
    }

    roots
}

fn cbrt(x: f64) -> f64 {
    if x >= 0.0 { x.cbrt() } else { -(-x).cbrt() }
}

fn solve_quadratic(a: f64, b: f64, c: f64) -> Vec<f64> {
    if a.abs() < 1e-12 {
        // Linear
        if b.abs() < 1e-12 {
            return Vec::new();
        }
        return vec![-c / b];
    }

    let disc = b * b - 4.0 * a * c;
    if disc < -1e-12 {
        return Vec::new();
    }
    if disc.abs() < 1e-12 {
        return vec![-b / (2.0 * a)];
    }
    let sq = disc.sqrt();
    vec![(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)]
}

// ── Segment representation ───────────────────────────────────────────────

/// A segment from the subject path, possibly split at intersection points.
/// Tracks the cubic curve and which region boundary edge it crosses at each end.
#[derive(Debug, Clone)]
struct SubSegment {
    cubic: CubicBez,
    inside: bool,
}

/// A crossing point where the subject path crosses the region boundary.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Crossing {
    /// Point of intersection
    point: Point,
    /// Index into the region boundary edges
    edge_index: usize,
    /// Parameter on the region boundary edge
    edge_t: f64,
    /// True if this crossing goes from outside to inside
    entering: bool,
    /// Global parameter encoding for ordering crossings on the boundary:
    /// edge_index + edge_t (allows sorting crossings around the boundary)
    boundary_param: f64,
}

// ── Core clipping ────────────────────────────────────────────────────────

/// Convert a line segment to a CubicBez
pub fn line_to_cubic(line: &Line) -> CubicBez {
    let p0 = line.p0;
    let p1 = line.p1;
    let cp1 = Point::new(
        p0.x + (p1.x - p0.x) / 3.0,
        p0.y + (p1.y - p0.y) / 3.0,
    );
    let cp2 = Point::new(
        p0.x + 2.0 * (p1.x - p0.x) / 3.0,
        p0.y + 2.0 * (p1.y - p0.y) / 3.0,
    );
    CubicBez::new(p0, cp1, cp2, p1)
}

/// Extract cubic bezier curves from a BezPath (converting lines/quads to cubics)
fn extract_cubics(path: &BezPath) -> Vec<CubicBez> {
    let mut cubics = Vec::new();
    let mut current = Point::ZERO;
    let mut subpath_start = Point::ZERO;

    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                current = p;
                subpath_start = p;
            }
            PathEl::LineTo(p) => {
                if dist(current, p) > 1e-10 {
                    cubics.push(line_to_cubic(&Line::new(current, p)));
                }
                current = p;
            }
            PathEl::QuadTo(cp, p) => {
                let cp1 = Point::new(
                    current.x + 2.0 / 3.0 * (cp.x - current.x),
                    current.y + 2.0 / 3.0 * (cp.y - current.y),
                );
                let cp2 = Point::new(
                    p.x + 2.0 / 3.0 * (cp.x - p.x),
                    p.y + 2.0 / 3.0 * (cp.y - p.y),
                );
                cubics.push(CubicBez::new(current, cp1, cp2, p));
                current = p;
            }
            PathEl::CurveTo(cp1, cp2, p) => {
                cubics.push(CubicBez::new(current, cp1, cp2, p));
                current = p;
            }
            PathEl::ClosePath => {
                if dist(current, subpath_start) > 1e-10 {
                    cubics.push(line_to_cubic(&Line::new(current, subpath_start)));
                }
                current = subpath_start;
            }
        }
    }
    cubics
}

/// Find all intersection t-values of a cubic with the region boundary lines.
/// Returns (t_on_cubic, edge_index, t_on_edge) sorted by t_on_cubic.
fn find_all_intersections(
    cubic: &CubicBez,
    region_lines: &[Line],
) -> Vec<(f64, usize, f64)> {
    let mut hits = Vec::new();

    // Check if this cubic is actually a line (degenerate cubic from line_to_cubic)
    let is_line = is_degenerate_line(cubic);

    for (edge_idx, line) in region_lines.iter().enumerate() {
        let t_values = if is_line {
            // Use exact line-line intersection
            let subject_line = Line::new(cubic.p0, cubic.p3);
            if let Some((t1, t2)) = line_line_intersection(&subject_line, line) {
                // Skip intersections at exact endpoints of the region edge to avoid
                // double-counting at region vertices
                if t2 > 1e-9 && t2 < 1.0 - 1e-9 {
                    vec![(t1, t2)]
                } else if t1 > 1e-9 && t1 < 1.0 - 1e-9 {
                    // The intersection is at an endpoint of the region edge.
                    // Only count it for one edge (the one where t2 > 0) to avoid doubles.
                    vec![(t1, t2)]
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        } else {
            // Cubic-line intersection
            cubic_line_intersections(cubic, line)
                .into_iter()
                .map(|t| {
                    let p = cubic.eval(t);
                    let dx = p.x - line.p0.x;
                    let dy = p.y - line.p0.y;
                    let lx = line.p1.x - line.p0.x;
                    let ly = line.p1.y - line.p0.y;
                    let s = (dx * lx + dy * ly) / (lx * lx + ly * ly);
                    (t, s.clamp(0.0, 1.0))
                })
                .collect()
        };

        for (t_cubic, t_edge) in t_values {
            // Avoid duplicates
            if !hits.iter().any(|&(existing_t, _, _): &(f64, usize, f64)| {
                (existing_t - t_cubic).abs() < 1e-6
            }) {
                hits.push((t_cubic, edge_idx, t_edge));
            }
        }
    }

    hits.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    hits
}

/// Check if a cubic is actually a degenerate line (from line_to_cubic)
fn is_degenerate_line(cubic: &CubicBez) -> bool {
    // A cubic from line_to_cubic has control points at 1/3 and 2/3 along the line
    let expected_p1 = Point::new(
        cubic.p0.x + (cubic.p3.x - cubic.p0.x) / 3.0,
        cubic.p0.y + (cubic.p3.y - cubic.p0.y) / 3.0,
    );
    let expected_p2 = Point::new(
        cubic.p0.x + 2.0 * (cubic.p3.x - cubic.p0.x) / 3.0,
        cubic.p0.y + 2.0 * (cubic.p3.y - cubic.p0.y) / 3.0,
    );
    dist(cubic.p1, expected_p1) < 1e-6 && dist(cubic.p2, expected_p2) < 1e-6
}

/// Split cubics at intersections with boundary lines and classify each piece.
/// Returns (sub_segments, crossings).
fn split_and_classify(
    cubics: &[CubicBez],
    boundary_lines: &[Line],
    containment_region: &BezPath,
) -> (Vec<SubSegment>, Vec<Crossing>) {
    let mut sub_segments: Vec<SubSegment> = Vec::new();
    let mut crossings: Vec<Crossing> = Vec::new();

    for cubic in cubics {
        let hits = find_all_intersections(cubic, boundary_lines);

        if hits.is_empty() {
            let mid = cubic.eval(0.5);
            let inside = point_in_region(mid, containment_region);
            sub_segments.push(SubSegment { cubic: *cubic, inside });
        } else {
            let mut prev_t = 0.0;
            for &(t, edge_idx, edge_t) in &hits {
                if t - prev_t > 1e-8 {
                    let sub = cubic.subsegment(prev_t..t);
                    let mid = sub.eval(0.5);
                    let inside = point_in_region(mid, containment_region);
                    sub_segments.push(SubSegment { cubic: sub, inside });
                }

                let point = cubic.eval(t);
                let before = cubic.eval((t - 0.005).max(0.0));
                let after = cubic.eval((t + 0.005).min(1.0));
                let entering = !point_in_region(before, containment_region)
                    && point_in_region(after, containment_region);

                crossings.push(Crossing {
                    point,
                    edge_index: edge_idx,
                    edge_t,
                    entering,
                    boundary_param: edge_idx as f64 + edge_t,
                });

                prev_t = t;
            }
            if 1.0 - prev_t > 1e-8 {
                let sub = cubic.subsegment(prev_t..1.0);
                let mid = sub.eval(0.5);
                let inside = point_in_region(mid, containment_region);
                sub_segments.push(SubSegment { cubic: sub, inside });
            }
        }
    }

    (sub_segments, crossings)
}

/// One-sided clip: build the "inside" path of `subject_cubics` clipped against `boundary`.
fn clip_one_side(
    subject_cubics: &[CubicBez],
    boundary: &BezPath,
    want_inside: bool,
) -> BezPath {
    let boundary_lines = region_line_segments(boundary);
    if boundary_lines.is_empty() {
        return BezPath::new();
    }
    let (sub_segments, crossings) = split_and_classify(subject_cubics, &boundary_lines, boundary);
    build_clipped_path(&sub_segments, &crossings, &boundary_lines, want_inside, None)
}

/// Clip a BezPath against a closed polygon region.
///
/// Uses a Weiler-Atherton-inspired approach:
/// 1. Split all subject path segments at region boundary crossings
/// 2. Classify each sub-segment as inside or outside
/// 3. For the "inside" path: chain inside sub-segments together, connecting
///    consecutive runs by walking the region boundary from exit to entry point
/// 4. Same for "outside" but walking the other way
///
/// When the region extends beyond the subject (e.g., a lasso that overshoots),
/// the boundary walk for the inside path may include region boundary segments
/// outside the subject. A second-pass clip against the subject trims these,
/// producing the correct intersection.
pub fn clip_path_to_region(path: &BezPath, region: &BezPath) -> ClipResult {
    let region_lines = region_line_segments(region);
    if region_lines.is_empty() {
        return ClipResult {
            inside: BezPath::new(),
            outside: path.clone(),
        };
    }

    let cubics = extract_cubics(path);
    if cubics.is_empty() {
        return ClipResult {
            inside: BezPath::new(),
            outside: BezPath::new(),
        };
    }

    // Step 1: Split and classify subject against region
    let (sub_segments, crossings) = split_and_classify(&cubics, &region_lines, region);

    // Step 2: Build raw inside and outside paths
    let inside_raw = build_clipped_path(&sub_segments, &crossings, &region_lines, true, None);
    let outside_raw = build_clipped_path(&sub_segments, &crossings, &region_lines, false, Some(path));

    // Step 3: Check if any region vertex lies outside the subject.
    // If so, boundary walks for the inside path may have followed region edges
    // outside the subject. Reclip the inside against the subject.
    // The outside doesn't need reclipping — it uses subject-aware grouping instead.
    let region_extends_beyond = region_lines.iter().any(|line| {
        !point_in_region(line.p0, path)
    });
    let inside = reclip_against_subject(&inside_raw, path, region_extends_beyond);
    let outside = outside_raw;

    ClipResult { inside, outside }
}

/// Clip `raw_path` against `subject` to ensure it stays within the subject.
/// This trims boundary walks that followed region edges outside the subject.
/// `region_extends_beyond` indicates whether any region vertex lies outside
/// the subject, meaning boundary walks could have escaped.
fn reclip_against_subject(raw_path: &BezPath, subject: &BezPath, region_extends_beyond: bool) -> BezPath {
    if raw_path.elements().is_empty() || !region_extends_beyond {
        return raw_path.clone();
    }
    let cubics = extract_cubics(raw_path);
    if cubics.is_empty() {
        return raw_path.clone();
    }
    let reclipped = clip_one_side(&cubics, subject, true);
    if reclipped.elements().is_empty() {
        raw_path.clone()
    } else {
        reclipped
    }
}

/// Build a clipped path for one side (inside=true or outside=false).
///
/// Strategy:
/// - Walk through sub_segments, collecting those matching `want_inside`
/// - When we encounter a gap (transition from wanted to unwanted), we've hit
///   a boundary crossing. Walk the region boundary to connect to the next
///   run of wanted sub-segments.
/// - When multiple disconnected pieces exist (e.g., a lasso splits the
///   remainder into two), emit them as separate sub-paths.
///
/// `subject`: if provided, used to validate boundary walks. Walks whose midpoint
/// falls outside the subject indicate disconnected groups that need separate sub-paths.
fn build_clipped_path(
    sub_segments: &[SubSegment],
    _crossings: &[Crossing],
    region_lines: &[Line],
    want_inside: bool,
    subject: Option<&BezPath>,
) -> BezPath {
    let mut path = BezPath::new();

    if sub_segments.is_empty() {
        return path;
    }

    // Collect runs of consecutive sub-segments that are `want_inside`
    let mut runs: Vec<(usize, usize)> = Vec::new(); // (start_idx, end_idx exclusive)
    let mut i = 0;
    while i < sub_segments.len() {
        if sub_segments[i].inside == want_inside {
            let start = i;
            while i < sub_segments.len() && sub_segments[i].inside == want_inside {
                i += 1;
            }
            runs.push((start, i));
        } else {
            i += 1;
        }
    }

    if runs.is_empty() {
        return path;
    }

    // If there's only one run and it covers the entire path, just output it closed
    if runs.len() == 1 && runs[0].0 == 0 && runs[0].1 == sub_segments.len() {
        let (start, end) = runs[0];
        path.move_to(sub_segments[start].cubic.p0);
        for seg in &sub_segments[start..end] {
            emit_cubic(&mut path, &seg.cubic);
        }
        path.close_path();
        return path;
    }

    // Group runs into separate sub-paths. Two consecutive runs belong to the
    // same sub-path if they can be connected by a boundary walk that doesn't
    // need to traverse the "other side". We detect this by checking if the
    // boundary walk midpoint is on the correct side of the region.
    //
    // Each group will form its own closed sub-path.
    let groups = group_runs_into_subpaths(&runs, sub_segments, region_lines, want_inside, subject);

    for group in &groups {
        let first_run = group[0];
        path.move_to(sub_segments[first_run.0].cubic.p0);

        for (gi, &(start, end)) in group.iter().enumerate() {
            // Emit the subject-path segments for this run
            for seg in &sub_segments[start..end] {
                emit_cubic(&mut path, &seg.cubic);
            }

            // Connect to the next run in this group via boundary walk
            let next_gi = (gi + 1) % group.len();
            let next_run = group[next_gi];

            let exit_point = sub_segments[end - 1].cubic.p3;
            let entry_point = sub_segments[next_run.0].cubic.p0;

            if dist(exit_point, entry_point) > 0.5 {
                let boundary_pts = walk_boundary(
                    exit_point,
                    entry_point,
                    region_lines,
                    want_inside,
                );
                for &bp in &boundary_pts {
                    path.line_to(bp);
                }
                path.line_to(entry_point);
            }
        }

        path.close_path();
    }

    path
}

/// Group runs into separate sub-paths based on whether boundary walks
/// between them stay within the subject.
///
/// When `subject` is provided, boundary walks whose midpoint falls outside
/// the subject indicate disconnected groups. When not provided, all runs
/// are grouped into a single sub-path.
fn group_runs_into_subpaths(
    runs: &[(usize, usize)],
    sub_segments: &[SubSegment],
    region_lines: &[Line],
    want_inside: bool,
    subject: Option<&BezPath>,
) -> Vec<Vec<(usize, usize)>> {
    if runs.len() <= 1 {
        return vec![runs.to_vec()];
    }

    let subject = match subject {
        Some(s) => s,
        None => return vec![runs.to_vec()],
    };

    // For each pair of consecutive runs, check if the boundary walk
    // between them stays inside the subject.
    let mut break_after: Vec<bool> = vec![false; runs.len()];

    for run_idx in 0..runs.len() {
        let next_idx = (run_idx + 1) % runs.len();
        let (_, end) = runs[run_idx];
        let (next_start, _) = runs[next_idx];

        let exit_point = sub_segments[end - 1].cubic.p3;
        let entry_point = sub_segments[next_start].cubic.p0;

        if dist(exit_point, entry_point) <= 0.5 {
            continue;
        }

        // Get the boundary walk
        let boundary_pts = walk_boundary(
            exit_point,
            entry_point,
            region_lines,
            want_inside,
        );

        // Check if any walk point or walk midpoint lies outside the subject.
        // If so, this walk escapes the subject and the runs should be in
        // separate sub-paths.
        let mut all_points = vec![exit_point];
        all_points.extend_from_slice(&boundary_pts);
        all_points.push(entry_point);

        for window in all_points.windows(2) {
            let mid = Point::new(
                (window[0].x + window[1].x) / 2.0,
                (window[0].y + window[1].y) / 2.0,
            );
            if !point_in_region(mid, subject) {
                break_after[run_idx] = true;
                break;
            }
        }
    }

    // Build groups based on break points.
    // Walk runs in order, breaking at break points.
    // Handle the circular nature: if last→first is NOT a break, merge them.
    let mut groups: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut current_group: Vec<(usize, usize)> = vec![runs[0]];

    for i in 0..runs.len() - 1 {
        if break_after[i] {
            groups.push(current_group);
            current_group = Vec::new();
        }
        current_group.push(runs[i + 1]);
    }

    // Handle wrap-around
    if !groups.is_empty() && !break_after[runs.len() - 1] {
        // Last run connects back to first group — merge
        let first_group = groups.remove(0);
        current_group.extend(first_group);
    }
    groups.push(current_group);

    groups
}

/// Emit a cubic to a BezPath, using line_to for degenerate (linear) cubics
fn emit_cubic(path: &mut BezPath, cubic: &CubicBez) {
    if is_degenerate_line(cubic) {
        path.line_to(cubic.p3);
    } else {
        path.curve_to(cubic.p1, cubic.p2, cubic.p3);
    }
}

/// Walk along the region boundary from `from` to `to`.
///
/// For the "inside" clip, we walk the shorter path along the boundary
/// (staying close to the region interior). For the "outside" clip, we walk
/// the longer path (going around the outside of the region).
fn walk_boundary(
    from: Point,
    to: Point,
    region_lines: &[Line],
    want_inside: bool,
) -> Vec<Point> {
    let n = region_lines.len();
    if n == 0 {
        return vec![to];
    }

    // Find boundary position for `from` and `to`
    let from_pos = project_onto_boundary(from, region_lines);
    let to_pos = project_onto_boundary(to, region_lines);

    // Walk clockwise (increasing edge index)
    let cw = walk_boundary_direction(from_pos, to_pos, region_lines, true);
    // Walk counter-clockwise (decreasing edge index)
    let ccw = walk_boundary_direction(from_pos, to_pos, region_lines, false);

    let cw_len = chain_length(from, &cw, to);
    let ccw_len = chain_length(from, &ccw, to);

    // Always take the shorter walk — for inside clips this connects
    // inside runs, for outside clips this connects outside runs,
    // and in both cases we want the shortest boundary path.
    let _ = want_inside;
    if cw_len <= ccw_len { cw } else { ccw }
}

/// A position on the boundary: (edge_index, t along that edge)
#[derive(Clone, Copy, Debug)]
struct BoundaryPos {
    edge: usize,
    t: f64,
}

fn project_onto_boundary(point: Point, lines: &[Line]) -> BoundaryPos {
    let mut best_edge = 0;
    let mut best_t = 0.0;
    let mut best_dist = f64::MAX;

    for (i, line) in lines.iter().enumerate() {
        let lx = line.p1.x - line.p0.x;
        let ly = line.p1.y - line.p0.y;
        let len_sq = lx * lx + ly * ly;
        if len_sq < 1e-20 {
            continue;
        }
        let t = ((point.x - line.p0.x) * lx + (point.y - line.p0.y) * ly) / len_sq;
        let t = t.clamp(0.0, 1.0);
        let proj = Point::new(line.p0.x + t * lx, line.p0.y + t * ly);
        let d = dist(point, proj);
        if d < best_dist {
            best_dist = d;
            best_edge = i;
            best_t = t;
        }
    }

    BoundaryPos { edge: best_edge, t: best_t }
}

/// Walk the boundary from `from_pos` to `to_pos` in a given direction.
/// `clockwise` = true means walk forward (increasing edge index).
/// Returns intermediate points (not including `from`, not including `to`).
fn walk_boundary_direction(
    from_pos: BoundaryPos,
    to_pos: BoundaryPos,
    lines: &[Line],
    clockwise: bool,
) -> Vec<Point> {
    let n = lines.len();
    let mut result = Vec::new();

    if from_pos.edge == to_pos.edge {
        // Same edge — check if we can go directly
        if clockwise && to_pos.t > from_pos.t + 1e-9 {
            return result; // Direct, no intermediate vertices needed
        }
        if !clockwise && to_pos.t < from_pos.t - 1e-9 {
            return result; // Direct
        }
        // Otherwise we need to go all the way around
    }

    if clockwise {
        // Walk forward: from from_pos.edge to to_pos.edge
        let mut edge = from_pos.edge;
        // First: emit the end vertex of the current edge (if we're not already at it)
        if from_pos.t < 1.0 - 1e-9 {
            result.push(lines[edge].p1);
        }
        edge = (edge + 1) % n;

        let mut safety = 0;
        while edge != to_pos.edge && safety < n + 1 {
            result.push(lines[edge].p1);
            edge = (edge + 1) % n;
            safety += 1;
        }
        // We're now on the target edge; the caller will add `to` point
    } else {
        // Walk backward: from from_pos.edge to to_pos.edge
        let mut edge = from_pos.edge;
        // First: emit the start vertex of the current edge (if we're not already at it)
        if from_pos.t > 1e-9 {
            result.push(lines[edge].p0);
        }
        edge = if edge == 0 { n - 1 } else { edge - 1 };

        let mut safety = 0;
        while edge != to_pos.edge && safety < n + 1 {
            result.push(lines[edge].p0);
            edge = if edge == 0 { n - 1 } else { edge - 1 };
            safety += 1;
        }
    }

    result
}

fn chain_length(start: Point, intermediates: &[Point], end: Point) -> f64 {
    let mut len = 0.0;
    let mut prev = start;
    for &p in intermediates {
        len += dist(prev, p);
        prev = p;
    }
    len += dist(prev, end);
    len
}

/// Check if a shape path has any segments that cross the region boundary
pub fn path_intersects_region(path: &BezPath, region: &BezPath) -> bool {
    let region_lines = region_line_segments(region);
    let cubics = extract_cubics(path);

    for cubic in &cubics {
        let hits = find_all_intersections(&cubic, &region_lines);
        if !hits.is_empty() {
            return true;
        }
    }
    false
}

/// Check if all points of a path are inside the region
pub fn path_fully_inside_region(path: &BezPath, region: &BezPath) -> bool {
    for el in path.elements() {
        let p = match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) => p,
            PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => p,
            PathEl::ClosePath => continue,
        };
        if !point_in_region(p, region) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rect_to_path() {
        let rect = Rect::new(10.0, 20.0, 100.0, 200.0);
        let path = rect_to_path(rect);
        // MoveTo + 3 LineTo + ClosePath = 5 elements
        assert!(path.elements().len() >= 5);
    }

    #[test]
    fn test_lasso_to_path() {
        let points = vec![
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Point::new(100.0, 100.0),
            Point::new(0.0, 100.0),
        ];
        let path = lasso_to_path(&points);
        assert!(path.elements().len() >= 5);
    }

    #[test]
    fn test_point_in_region() {
        let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(point_in_region(Point::new(50.0, 50.0), &region));
        assert!(!point_in_region(Point::new(150.0, 50.0), &region));
    }

    #[test]
    fn test_line_line_intersection() {
        let l1 = Line::new(Point::new(0.0, 5.0), Point::new(10.0, 5.0));
        let l2 = Line::new(Point::new(5.0, 0.0), Point::new(5.0, 10.0));
        let result = line_line_intersection(&l1, &l2);
        assert!(result.is_some());
        let (t1, t2) = result.unwrap();
        assert!((t1 - 0.5).abs() < 1e-6);
        assert!((t2 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_clip_rect_corner() {
        // Rectangle from (0,0) to (100,100)
        let mut subject = BezPath::new();
        subject.move_to(Point::new(0.0, 0.0));
        subject.line_to(Point::new(100.0, 0.0));
        subject.line_to(Point::new(100.0, 100.0));
        subject.line_to(Point::new(0.0, 100.0));
        subject.close_path();

        // Clip to upper-right corner: (50,0) to (100,50)
        let region = rect_to_path(Rect::new(50.0, 0.0, 150.0, 50.0));
        let result = clip_path_to_region(&subject, &region);

        // Inside should have elements (the upper-right portion)
        assert!(!result.inside.elements().is_empty(),
            "inside path should not be empty");
        // Outside should have elements (the rest of the rectangle)
        assert!(!result.outside.elements().is_empty(),
            "outside path should not be empty");

        // The inside portion should be a roughly rectangular region
        // Its bounding box should be approximately (50,0)-(100,50)
        let inside_bb = result.inside.bounding_box();
        assert!((inside_bb.x0 - 50.0).abs() < 2.0,
            "inside x0 should be ~50, got {}", inside_bb.x0);
        assert!((inside_bb.y0 - 0.0).abs() < 2.0,
            "inside y0 should be ~0, got {}", inside_bb.y0);
        assert!((inside_bb.x1 - 100.0).abs() < 2.0,
            "inside x1 should be ~100, got {}", inside_bb.x1);
        assert!((inside_bb.y1 - 50.0).abs() < 2.0,
            "inside y1 should be ~50, got {}", inside_bb.y1);
    }

    #[test]
    fn test_clip_fully_inside() {
        let mut path = BezPath::new();
        path.move_to(Point::new(20.0, 20.0));
        path.line_to(Point::new(80.0, 20.0));
        path.line_to(Point::new(80.0, 80.0));
        path.line_to(Point::new(20.0, 80.0));
        path.close_path();

        let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        let result = clip_path_to_region(&path, &region);

        assert!(!result.inside.elements().is_empty());
        assert!(result.outside.elements().is_empty());
    }

    #[test]
    fn test_clip_fully_outside() {
        let mut path = BezPath::new();
        path.move_to(Point::new(200.0, 200.0));
        path.line_to(Point::new(300.0, 200.0));
        path.line_to(Point::new(300.0, 300.0));
        path.close_path();

        let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        let result = clip_path_to_region(&path, &region);

        assert!(result.inside.elements().is_empty());
        assert!(!result.outside.elements().is_empty());
    }

    #[test]
    fn test_path_intersects_region() {
        let mut path = BezPath::new();
        path.move_to(Point::new(-50.0, 50.0));
        path.line_to(Point::new(150.0, 50.0));

        let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(path_intersects_region(&path, &region));
    }

    #[test]
    fn test_path_fully_inside() {
        let mut path = BezPath::new();
        path.move_to(Point::new(20.0, 20.0));
        path.line_to(Point::new(80.0, 20.0));
        path.line_to(Point::new(80.0, 80.0));
        path.close_path();

        let region = rect_to_path(Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(path_fully_inside_region(&path, &region));
        assert!(!path_intersects_region(&path, &region));
    }

    #[test]
    fn test_cubic_line_intersection() {
        // Horizontal line as cubic
        let cubic = CubicBez::new(
            Point::new(0.0, 50.0),
            Point::new(33.33, 50.0),
            Point::new(66.67, 50.0),
            Point::new(100.0, 50.0),
        );
        // Vertical line segment
        let line = Line::new(Point::new(50.0, 0.0), Point::new(50.0, 100.0));
        let hits = cubic_line_intersections(&cubic, &line);
        assert_eq!(hits.len(), 1, "Expected 1 intersection, got {}", hits.len());
        assert!((hits[0] - 0.5).abs() < 0.01, "t should be ~0.5, got {}", hits[0]);
    }
}
