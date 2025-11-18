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
    /// Selection tool - select and move objects
    Select,
    /// Draw/Pen tool - freehand drawing
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
        start_corner: Point,
        current_corner: Point,
    },

    /// Creating an ellipse shape
    CreatingEllipse {
        center: Point,
        current_point: Point,
    },

    /// Transforming selected objects (scale, rotate)
    Transforming {
        mode: TransformMode,
        original_transforms: HashMap<Uuid, crate::object::Transform>,
        pivot: Point,
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
            Tool::Select => "Select",
            Tool::Draw => "Draw",
            Tool::Transform => "Transform",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::PaintBucket => "Paint Bucket",
            Tool::Eyedropper => "Eyedropper",
        }
    }

    /// Get SVG icon file name for the tool
    pub fn icon_file(self) -> &'static str {
        match self {
            Tool::Select => "select.svg",
            Tool::Draw => "draw.svg",
            Tool::Transform => "transform.svg",
            Tool::Rectangle => "rectangle.svg",
            Tool::Ellipse => "ellipse.svg",
            Tool::PaintBucket => "paint_bucket.svg",
            Tool::Eyedropper => "eyedropper.svg",
        }
    }

    /// Get all available tools
    pub fn all() -> &'static [Tool] {
        &[
            Tool::Select,
            Tool::Draw,
            Tool::Transform,
            Tool::Rectangle,
            Tool::Ellipse,
            Tool::PaintBucket,
            Tool::Eyedropper,
        ]
    }

    /// Get keyboard shortcut hint
    pub fn shortcut_hint(self) -> &'static str {
        match self {
            Tool::Select => "V",
            Tool::Draw => "P",
            Tool::Transform => "Q",
            Tool::Rectangle => "R",
            Tool::Ellipse => "E",
            Tool::PaintBucket => "B",
            Tool::Eyedropper => "I",
        }
    }
}
