/// Tool system for the toolbar
///
/// Defines the available drawing/editing tools

use serde::{Deserialize, Serialize};

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
