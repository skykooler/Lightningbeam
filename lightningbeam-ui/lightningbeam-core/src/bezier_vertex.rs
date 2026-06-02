//! Bezier vertex and editable curves structures for vector editing
//!
//! Provides data structures for editing bezier paths by extracting
//! vertices (where curves meet) and individual curve segments.

use vello::kurbo::{CubicBez, Point};

/// A vertex in a shape path where curve segments meet
///
/// Vertices are automatically generated from curve endpoints, with nearby
/// endpoints merged together (within VERTEX_MERGE_EPSILON). This allows
/// dragging a single vertex to update all connected curves simultaneously.
#[derive(Debug, Clone)]
pub struct BezierVertex {
    /// The point location in local shape space
    pub point: Point,

    /// Indices of curves that start at this vertex
    /// (i.e., curves where p0 == this point)
    pub start_curves: Vec<usize>,

    /// Indices of curves that end at this vertex
    /// (i.e., curves where p3 == this point)
    pub end_curves: Vec<usize>,
}

impl BezierVertex {
    /// Create a new vertex at the given point
    pub fn new(point: Point) -> Self {
        Self {
            point,
            start_curves: Vec::new(),
            end_curves: Vec::new(),
        }
    }

    /// Check if this vertex connects to any curves
    pub fn is_connected(&self) -> bool {
        !self.start_curves.is_empty() || !self.end_curves.is_empty()
    }

    /// Get total number of curves connected to this vertex
    pub fn connection_count(&self) -> usize {
        self.start_curves.len() + self.end_curves.len()
    }
}

/// Extracted editable bezier curve segments from a BezPath
///
/// This structure represents a BezPath converted into an editable form,
/// with explicit curve segments and auto-generated vertices. This allows
/// for vertex-based and curve-based editing operations.
#[derive(Debug, Clone)]
pub struct EditableBezierCurves {
    /// All cubic bezier curves extracted from the path
    ///
    /// All path elements (lines, quadratics, etc.) are converted to cubic beziers
    /// for uniform editing. Each CubicBez has four control points: p0, p1, p2, p3.
    pub curves: Vec<CubicBez>,

    /// Auto-generated vertices from curve endpoints
    ///
    /// Vertices are created by merging nearby endpoints (within epsilon tolerance).
    /// Each vertex tracks which curves connect to it via start_curves and end_curves.
    pub vertices: Vec<BezierVertex>,

    /// Whether the path is closed
    ///
    /// A path is considered closed if the first curve's p0 is within epsilon
    /// of the last curve's p3.
    pub is_closed: bool,
}

impl EditableBezierCurves {
    /// Create a new empty editable curves structure
    pub fn new() -> Self {
        Self {
            curves: Vec::new(),
            vertices: Vec::new(),
            is_closed: false,
        }
    }

    /// Get the number of curves
    pub fn curve_count(&self) -> usize {
        self.curves.len()
    }

    /// Get the number of vertices
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Check if the structure is empty (no curves)
    pub fn is_empty(&self) -> bool {
        self.curves.is_empty()
    }
}

impl Default for EditableBezierCurves {
    fn default() -> Self {
        Self::new()
    }
}
