//! Shape system for Lightningbeam
//!
//! Provides bezier-based vector shapes with morphing support.
//! All shapes are composed of cubic bezier curves using kurbo::BezPath.

use crate::path_interpolation::interpolate_paths;
use kurbo::{BezPath, Cap as KurboCap, Join as KurboJoin, Stroke as KurboStroke};
use vello::peniko::{Brush, Color, Fill};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A version of a shape (for morphing between different states)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeVersion {
    /// The bezier path defining this shape version
    pub path: BezPath,
    /// Index in the shape's versions array
    pub index: usize,
}

impl ShapeVersion {
    /// Create a new shape version
    pub fn new(path: BezPath, index: usize) -> Self {
        Self { path, index }
    }
}

/// Fill rule for shapes
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillRule {
    /// Non-zero winding rule
    NonZero,
    /// Even-odd rule
    EvenOdd,
}

impl Default for FillRule {
    fn default() -> Self {
        FillRule::NonZero
    }
}

impl From<FillRule> for Fill {
    fn from(rule: FillRule) -> Self {
        match rule {
            FillRule::NonZero => Fill::NonZero,
            FillRule::EvenOdd => Fill::EvenOdd,
        }
    }
}

/// Stroke cap style
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

impl Default for Cap {
    fn default() -> Self {
        Cap::Butt
    }
}

impl From<Cap> for KurboCap {
    fn from(cap: Cap) -> Self {
        match cap {
            Cap::Butt => KurboCap::Butt,
            Cap::Round => KurboCap::Round,
            Cap::Square => KurboCap::Square,
        }
    }
}

/// Stroke join style
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

impl Default for Join {
    fn default() -> Self {
        Join::Miter
    }
}

impl From<Join> for KurboJoin {
    fn from(join: Join) -> Self {
        match join {
            Join::Miter => KurboJoin::Miter,
            Join::Round => KurboJoin::Round,
            Join::Bevel => KurboJoin::Bevel,
        }
    }
}

/// Stroke style for shapes
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrokeStyle {
    /// Stroke width in pixels
    pub width: f64,
    /// Cap style
    #[serde(default)]
    pub cap: Cap,
    /// Join style
    #[serde(default)]
    pub join: Join,
    /// Miter limit (for miter joins)
    #[serde(default = "default_miter_limit")]
    pub miter_limit: f64,
}

fn default_miter_limit() -> f64 {
    4.0
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
        }
    }
}

impl StrokeStyle {
    /// Convert to kurbo Stroke
    pub fn to_stroke(&self) -> KurboStroke {
        KurboStroke {
            width: self.width,
            join: self.join.into(),
            miter_limit: self.miter_limit,
            start_cap: self.cap.into(),
            end_cap: self.cap.into(),
            dash_pattern: Default::default(),
            dash_offset: 0.0,
        }
    }
}

/// Serializable color representation
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl ShapeColor {
    /// Create a new color
    pub fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Create from RGB (opaque)
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// Create from RGBA
    pub fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// Convert to peniko Color
    pub fn to_peniko(&self) -> Color {
        Color::from_rgba8(self.r, self.g, self.b, self.a)
    }

    /// Convert to peniko Brush
    pub fn to_brush(&self) -> Brush {
        Brush::Solid(self.to_peniko())
    }

    /// Create from egui Color32
    pub fn from_egui(color: egui::Color32) -> Self {
        Self {
            r: color.r(),
            g: color.g(),
            b: color.b(),
            a: color.a(),
        }
    }
}

impl Default for ShapeColor {
    fn default() -> Self {
        Self::rgb(0, 0, 0)
    }
}

impl From<Color> for ShapeColor {
    fn from(color: Color) -> Self {
        // peniko 0.4 uses components array [r, g, b, a] as floats 0.0-1.0
        let components = color.components;
        Self {
            r: (components[0] * 255.0) as u8,
            g: (components[1] * 255.0) as u8,
            b: (components[2] * 255.0) as u8,
            a: (components[3] * 255.0) as u8,
        }
    }
}

/// A shape with geometry and styling
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    /// Unique identifier for this shape
    pub id: Uuid,

    /// Multiple versions of the shape for morphing
    /// The shape animates between these by varying the shapeIndex property
    pub versions: Vec<ShapeVersion>,

    /// Fill color (used when image_fill is None)
    pub fill_color: Option<ShapeColor>,

    /// Image fill - references an ImageAsset by UUID
    /// When set, the image is rendered as the fill instead of fill_color
    #[serde(default)]
    pub image_fill: Option<Uuid>,

    /// Fill rule
    #[serde(default)]
    pub fill_rule: FillRule,

    /// Stroke color
    pub stroke_color: Option<ShapeColor>,

    /// Stroke style
    pub stroke_style: Option<StrokeStyle>,
}

impl Shape {
    /// Create a new shape with a single path (no fill or stroke by default)
    pub fn new(path: BezPath) -> Self {
        Self {
            id: Uuid::new_v4(),
            versions: vec![ShapeVersion::new(path, 0)],
            fill_color: None,
            image_fill: None,
            fill_rule: FillRule::NonZero,
            stroke_color: None,
            stroke_style: None,
        }
    }

    /// Create a new shape with a specific ID (no fill or stroke by default)
    pub fn with_id(id: Uuid, path: BezPath) -> Self {
        Self {
            id,
            versions: vec![ShapeVersion::new(path, 0)],
            fill_color: None,
            image_fill: None,
            fill_rule: FillRule::NonZero,
            stroke_color: None,
            stroke_style: None,
        }
    }

    /// Set image fill (references an ImageAsset by UUID)
    pub fn with_image_fill(mut self, image_asset_id: Uuid) -> Self {
        self.image_fill = Some(image_asset_id);
        self.fill_color = None; // Image fill takes precedence
        self
    }

    /// Add a new version for morphing
    pub fn add_version(&mut self, path: BezPath) -> usize {
        let index = self.versions.len();
        self.versions.push(ShapeVersion::new(path, index));
        index
    }

    /// Get the interpolated path for a fractional shape index
    /// Used for shape morphing animation using d3-interpolate-path algorithm
    pub fn get_morphed_path(&self, shape_index: f64) -> BezPath {
        if self.versions.is_empty() {
            return BezPath::new();
        }

        // Clamp to valid range
        let shape_index = shape_index.max(0.0);

        // Get the two versions to interpolate between
        let index0 = shape_index.floor() as usize;
        let index1 = (index0 + 1).min(self.versions.len() - 1);

        if index0 >= self.versions.len() {
            // Beyond last version, return last version
            return self.versions.last().unwrap().path.clone();
        }

        if index0 == index1 {
            // Exactly on a version
            return self.versions[index0].path.clone();
        }

        // Interpolate between the two versions using d3-interpolate-path
        let t = shape_index - index0 as f64;
        interpolate_paths(&self.versions[index0].path, &self.versions[index1].path, t)
    }

    /// Set fill color
    pub fn with_fill(mut self, color: ShapeColor) -> Self {
        self.fill_color = Some(color);
        self
    }

    /// Set stroke
    pub fn with_stroke(mut self, color: ShapeColor, style: StrokeStyle) -> Self {
        self.stroke_color = Some(color);
        self.stroke_style = Some(style);
        self
    }

    /// Set fill rule
    pub fn with_fill_rule(mut self, rule: FillRule) -> Self {
        self.fill_rule = rule;
        self
    }

    /// Get the base path (first version) for this shape
    ///
    /// This is useful for hit testing and bounding box calculations
    /// when shape morphing is not being considered.
    pub fn path(&self) -> &BezPath {
        &self.versions[0].path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_shape_creation() {
        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path);

        assert_eq!(shape.versions.len(), 1);
        assert!(shape.fill_color.is_some());
    }

    #[test]
    fn test_shape_morphing() {
        let circle1 = Circle::new((100.0, 100.0), 50.0);
        let circle2 = Circle::new((100.0, 100.0), 100.0);

        let mut shape = Shape::new(circle1.to_path(0.1));
        shape.add_version(circle2.to_path(0.1));

        // Test that morphing doesn't panic
        let _morphed = shape.get_morphed_path(0.5);
        assert_eq!(shape.versions.len(), 2);
    }
}
