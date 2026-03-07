/// Tool system for the toolbar
///
/// Defines the available drawing/editing tools

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::Point;

/// Drawing and editing tools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Tool {
    // ── Vector / shared tools ──────────────────────────────────────────────
    /// Selection tool - select and move objects
    Select,
    /// Draw/Brush tool - freehand drawing (vector) / paintbrush (raster)
    Draw,
    /// Transform tool - scale, rotate, skew
    Transform,
    /// Rectangle shape tool
    Rectangle,
    /// Ellipse/Circle shape tool
    Ellipse,
    /// Paint bucket - fill areas with color
    PaintBucket,
    /// Eyedropper - pick colors from the canvas
    Eyedropper,
    /// Line tool - draw straight lines
    Line,
    /// Polygon tool - draw polygons
    Polygon,
    /// Bezier edit tool - edit bezier curve control points
    BezierEdit,
    /// Text tool - add and edit text
    Text,
    /// Region select tool - select sub-regions of shapes by clipping
    RegionSelect,
    /// Split tool - split audio/video clips at a point
    Split,
    // ── Raster brush tools ────────────────────────────────────────────────
    /// Pencil tool - hard-edged raster brush
    Pencil,
    /// Pen tool - pressure-sensitive raster pen
    Pen,
    /// Airbrush tool - soft spray raster brush
    Airbrush,
    /// Erase tool - erase raster pixels
    Erase,
    /// Smudge tool - smudge/blend raster pixels
    Smudge,
    /// Clone Stamp - copy pixels from a source point
    CloneStamp,
    /// Healing Brush - content-aware pixel repair
    HealingBrush,
    /// Pattern Stamp - paint with a repeating pattern
    PatternStamp,
    /// Dodge/Burn - lighten or darken pixels
    DodgeBurn,
    /// Sponge - saturate or desaturate pixels
    Sponge,
    /// Blur/Sharpen - blur or sharpen pixel regions
    BlurSharpen,
    // ── Raster fill / shape ───────────────────────────────────────────────
    /// Gradient tool - fill with a gradient
    Gradient,
    /// Custom Shape tool - draw from a shape library
    CustomShape,
    // ── Raster selection tools ────────────────────────────────────────────
    /// Elliptical marquee selection
    SelectEllipse,
    /// Lasso select tool - freehand / polygonal / magnetic selection
    SelectLasso,
    /// Magic Wand - select by colour similarity
    MagicWand,
    /// Quick Select - brush-based smart selection
    QuickSelect,
    // ── Raster transform tools ────────────────────────────────────────────
    /// Warp / perspective transform
    Warp,
    /// Liquify - freeform pixel warping
    Liquify,
}

/// Region select mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RegionSelectMode {
    /// Rectangular region selection
    Rectangle,
    /// Freehand lasso region selection
    Lasso,
}

impl Default for RegionSelectMode {
    fn default() -> Self {
        Self::Rectangle
    }
}

/// Lasso selection sub-mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LassoMode {
    /// Freehand lasso (existing, implemented)
    Freehand,
    /// Click-to-place polygonal lasso
    Polygonal,
    /// Magnetically snaps to edges
    Magnetic,
}

impl Default for LassoMode {
    fn default() -> Self {
        Self::Freehand
    }
}

/// Tool state tracking for interactive operations
#[derive(Debug, Clone)]
pub enum ToolState {
    /// Tool is idle (no operation in progress)
    Idle,

    /// Drawing a freehand path
    DrawingPath {
        points: Vec<Point>,
        simplify_mode: SimplifyMode,
    },

    /// Drawing a raster paint stroke
    DrawingRasterStroke {
        points: Vec<crate::raster_layer::StrokePoint>,
    },

    /// Drawing a freehand lasso selection on a raster layer
    DrawingRasterLasso {
        points: Vec<(i32, i32)>,
    },

    /// Drawing a rectangular marquee selection on a raster layer
    DrawingRasterMarquee {
        start: (i32, i32),
        current: (i32, i32),
    },

    /// Moving an existing raster selection (and its floating pixels, if any).
    MovingRasterSelection {
        /// Canvas position of the pointer at the last processed event, used to
        /// compute per-frame deltas.
        last: (i32, i32),
    },

    /// Dragging selected objects
    DraggingSelection {
        start_pos: Point,
        start_mouse: Point,
        original_positions: HashMap<Uuid, Point>,
    },

    /// Creating a marquee selection rectangle
    MarqueeSelecting {
        start: Point,
        current: Point,
    },

    /// Creating a rectangle shape
    CreatingRectangle {
        start_point: Point,      // Starting point (corner or center depending on modifiers)
        current_point: Point,    // Current mouse position
        centered: bool,          // If true, start_point is center; if false, it's a corner
        constrain_square: bool,  // If true, constrain to square (equal width/height)
    },

    /// Creating an ellipse shape
    CreatingEllipse {
        start_point: Point,      // Starting point (center or corner depending on modifiers)
        current_point: Point,    // Current mouse position
        corner_mode: bool,       // If true, start is corner; if false, start is center
        constrain_circle: bool,  // If true, constrain to circle (equal radii)
    },

    /// Transforming selected objects (scale, rotate)
    Transforming {
        mode: TransformMode,
        original_transforms: HashMap<Uuid, crate::object::Transform>,
        pivot: Point,
        start_mouse: Point,    // Mouse position when transform started
        current_mouse: Point,  // Current mouse position during drag
        original_bbox: vello::kurbo::Rect,  // Bounding box at start of transform (fixed)
    },

    /// Creating a line
    CreatingLine {
        start_point: Point,    // Starting point of the line
        current_point: Point,  // Current mouse position (end point)
    },

    /// Creating a polygon
    CreatingPolygon {
        center: Point,         // Center point of the polygon
        current_point: Point,  // Current mouse position (determines radius)
        num_sides: u32,        // Number of sides (from properties, default 5)
    },

    /// Editing a vertex (dragging it and connected edges)
    EditingVertex {
        vertex_id: crate::dcel::VertexId,
        connected_edges: Vec<crate::dcel::EdgeId>,  // edges to update when vertex moves
    },

    /// Editing a curve (reshaping with moldCurve algorithm)
    EditingCurve {
        edge_id: crate::dcel::EdgeId,
        original_curve: vello::kurbo::CubicBez,
        start_mouse: Point,
        parameter_t: f64,
    },

    /// Pending curve interaction: click selects edge, drag starts curve editing
    PendingCurveInteraction {
        edge_id: crate::dcel::EdgeId,
        parameter_t: f64,
        start_mouse: Point,
    },

    /// Drawing a region selection rectangle
    RegionSelectingRect {
        start: Point,
        current: Point,
    },

    /// Drawing a freehand lasso region selection
    RegionSelectingLasso {
        points: Vec<Point>,
    },

    /// Editing a control point (BezierEdit tool only)
    EditingControlPoint {
        edge_id: crate::dcel::EdgeId,
        point_index: u8,           // 1 or 2 (p1 or p2 of the cubic bezier)
        original_curve: vello::kurbo::CubicBez,
        start_pos: Point,
    },
}

/// Path simplification mode for the draw tool
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimplifyMode {
    /// Ramer-Douglas-Peucker corner detection
    Corners,
    /// Schneider curve fitting for smooth curves
    Smooth,
    /// No simplification (use raw points)
    Verbatim,
}

/// Transform mode for the transform tool
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransformMode {
    /// Scale from a corner
    ScaleCorner { origin: Point },
    /// Scale along an edge
    ScaleEdge { axis: Axis, origin: Point },
    /// Rotate around a pivot
    Rotate { center: Point },
    /// Skew along an edge
    Skew { axis: Axis, origin: Point },
}

/// Axis for edge scaling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

impl Default for ToolState {
    fn default() -> Self {
        Self::Idle
    }
}

impl Tool {
    /// Get display name for the tool
    pub fn display_name(self) -> &'static str {
        match self {
            Tool::Select        => "Select",
            Tool::Draw          => "Brush",
            Tool::Transform     => "Transform",
            Tool::Rectangle     => "Rectangle",
            Tool::Ellipse       => "Ellipse",
            Tool::PaintBucket   => "Paint Bucket",
            Tool::Eyedropper    => "Eyedropper",
            Tool::Line          => "Line",
            Tool::Polygon       => "Polygon",
            Tool::BezierEdit    => "Bezier Edit",
            Tool::Text          => "Text",
            Tool::RegionSelect  => "Region Select",
            Tool::Split         => "Split",
            Tool::Pencil        => "Pencil",
            Tool::Pen           => "Pen",
            Tool::Airbrush      => "Airbrush",
            Tool::Erase         => "Eraser",
            Tool::Smudge        => "Smudge",
            Tool::CloneStamp    => "Clone Stamp",
            Tool::HealingBrush  => "Healing Brush",
            Tool::PatternStamp  => "Pattern Stamp",
            Tool::DodgeBurn     => "Dodge / Burn",
            Tool::Sponge        => "Sponge",
            Tool::BlurSharpen   => "Blur / Sharpen",
            Tool::Gradient      => "Gradient",
            Tool::CustomShape   => "Custom Shape",
            Tool::SelectEllipse => "Elliptical Select",
            Tool::SelectLasso   => "Lasso Select",
            Tool::MagicWand     => "Magic Wand",
            Tool::QuickSelect   => "Quick Select",
            Tool::Warp          => "Warp",
            Tool::Liquify       => "Liquify",
        }
    }

    /// Get SVG icon file name for the tool
    pub fn icon_file(self) -> &'static str {
        match self {
            Tool::Select        => "select.svg",
            Tool::Draw          => "draw.svg",
            Tool::Transform     => "transform.svg",
            Tool::Rectangle     => "rectangle.svg",
            Tool::Ellipse       => "ellipse.svg",
            Tool::PaintBucket   => "paint_bucket.svg",
            Tool::Eyedropper    => "eyedropper.svg",
            Tool::Line          => "line.svg",
            Tool::Polygon       => "polygon.svg",
            Tool::BezierEdit    => "bezier_edit.svg",
            Tool::Text          => "text.svg",
            Tool::RegionSelect  => "region_select.svg",
            Tool::Split         => "split.svg",
            Tool::Erase         => "erase.svg",
            Tool::Smudge        => "smudge.svg",
            Tool::SelectLasso   => "lasso.svg",
            // Not yet implemented — use the placeholder icon
            Tool::Pencil
            | Tool::Pen
            | Tool::Airbrush
            | Tool::CloneStamp
            | Tool::HealingBrush
            | Tool::PatternStamp
            | Tool::DodgeBurn
            | Tool::Sponge
            | Tool::BlurSharpen
            | Tool::Gradient
            | Tool::CustomShape
            | Tool::SelectEllipse
            | Tool::MagicWand
            | Tool::QuickSelect
            | Tool::Warp
            | Tool::Liquify     => "todo.svg",
        }
    }

    /// Get all vector-layer tools (the full drawing toolset)
    pub fn all() -> &'static [Tool] {
        &[
            Tool::Select,
            Tool::Draw,
            Tool::Transform,
            Tool::Rectangle,
            Tool::Ellipse,
            Tool::PaintBucket,
            Tool::Eyedropper,
            Tool::Line,
            Tool::Polygon,
            Tool::BezierEdit,
            Tool::Text,
            Tool::RegionSelect,
        ]
    }

    /// Get the tools available for a given layer type
    pub fn for_layer_type(layer_type: Option<crate::layer::LayerType>) -> &'static [Tool] {
        use crate::layer::LayerType;
        match layer_type {
            None | Some(LayerType::Vector) => Tool::all(),
            Some(LayerType::Audio) | Some(LayerType::Video) => &[Tool::Select, Tool::Split],
            Some(LayerType::Raster) => &[
                // Brush tools
                Tool::Draw, Tool::Pencil, Tool::Pen, Tool::Airbrush,
                Tool::Erase, Tool::Smudge,
                Tool::CloneStamp, Tool::HealingBrush, Tool::PatternStamp,
                Tool::DodgeBurn, Tool::Sponge, Tool::BlurSharpen,
                // Fill / shape
                Tool::PaintBucket, Tool::Gradient,
                Tool::Rectangle, Tool::Ellipse, Tool::Polygon, Tool::Line, Tool::CustomShape,
                // Selection
                Tool::Select, Tool::SelectLasso,
                Tool::MagicWand, Tool::QuickSelect,
                // Transform
                Tool::Transform, Tool::Warp, Tool::Liquify,
                // Utility
                Tool::Eyedropper,
            ],
            _ => &[Tool::Select],
        }
    }

}
