//! Text layer for Lightningbeam
//!
//! A text layer holds a single editable text field inside a resizable box,
//! with editable size, color, and font. Text is rendered as vector glyphs
//! (via parley + Vello) so it composites through the same path as vector art.
//!
//! The text/style fields are grouped in [`TextContent`] so they can move to a
//! per-keyframe model later without touching call sites; v1 stores a single
//! static instance and reads it through [`TextLayer::content_at`].

use crate::layer::{Layer, LayerTrait, LayerType};
use kurbo::Point;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use uuid::Uuid;

/// Horizontal alignment of text within the box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

impl Default for TextAlign {
    fn default() -> Self {
        TextAlign::Left
    }
}

/// The text content + styling for a text layer.
///
/// Grouped as its own struct so a future keyframed model can store
/// `Vec<TextKeyframe>` of these without changing the layer's public shape.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    /// The text string.
    pub text: String,
    /// Font size in pixels (document space).
    pub font_size: f64,
    /// Fill color, linear RGBA in 0..=1.
    pub color: [f32; 4],
    /// Logical font family name. Empty string = the bundled default font.
    pub font_family: String,
    /// Horizontal alignment within the box.
    pub align: TextAlign,
}

impl Default for TextContent {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: 48.0,
            color: [1.0, 1.0, 1.0, 1.0],
            font_family: String::new(),
            align: TextAlign::Left,
        }
    }
}

impl TextContent {
    /// A stable hash of everything that affects shaped layout (everything except
    /// `color`, which only affects the brush, not glyph positions). Used to key
    /// the renderer's external parley-layout cache, including the wrap width.
    pub fn layout_hash(&self, box_width: f64) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut h);
        self.font_size.to_bits().hash(&mut h);
        self.font_family.hash(&mut h);
        self.align.hash(&mut h);
        box_width.to_bits().hash(&mut h);
        h.finish()
    }
}

impl Hash for TextAlign {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

/// A text layer: a single resizable text box.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextLayer {
    /// Base layer properties (id, name, opacity, visibility, animation data, …).
    pub layer: Layer,
    /// Top-left of the text box, in layer/clip-local space.
    pub box_origin: Point,
    /// Box width in document units (drives text wrapping).
    pub box_width: f64,
    /// Box height in document units.
    pub box_height: f64,
    /// The text content + styling (single static instance for v1).
    pub content: TextContent,
}

impl TextLayer {
    /// Default text-box dimensions for a freshly created layer.
    pub const DEFAULT_WIDTH: f64 = 300.0;
    pub const DEFAULT_HEIGHT: f64 = 100.0;

    /// Create a new, empty text layer with the given name, positioned at `origin`.
    pub fn new(name: impl Into<String>, origin: Point) -> Self {
        Self {
            layer: Layer::new(LayerType::Text, name),
            box_origin: origin,
            box_width: Self::DEFAULT_WIDTH,
            box_height: Self::DEFAULT_HEIGHT,
            content: TextContent::default(),
        }
    }

    /// Read the active text content at `time`. v1 is static, so `time` is unused;
    /// a future keyframed model will select the active keyframe here.
    pub fn content_at(&self, _time: f64) -> &TextContent {
        &self.content
    }

    /// Mutable access to the active text content at `time`.
    pub fn content_at_mut(&mut self, _time: f64) -> &mut TextContent {
        &mut self.content
    }
}

// Delegate all LayerTrait methods to self.layer (mirrors RasterLayer).
impl LayerTrait for TextLayer {
    fn id(&self) -> Uuid { self.layer.id }
    fn name(&self) -> &str { &self.layer.name }
    fn set_name(&mut self, name: String) { self.layer.name = name; }
    fn has_custom_name(&self) -> bool { self.layer.has_custom_name }
    fn set_has_custom_name(&mut self, custom: bool) { self.layer.has_custom_name = custom; }
    fn visible(&self) -> bool { self.layer.visible }
    fn set_visible(&mut self, visible: bool) { self.layer.visible = visible; }
    fn opacity(&self) -> f64 { self.layer.opacity }
    fn set_opacity(&mut self, opacity: f64) { self.layer.opacity = opacity; }
    fn volume(&self) -> f64 { self.layer.volume }
    fn set_volume(&mut self, volume: f64) { self.layer.volume = volume; }
    fn muted(&self) -> bool { self.layer.muted }
    fn set_muted(&mut self, muted: bool) { self.layer.muted = muted; }
    fn soloed(&self) -> bool { self.layer.soloed }
    fn set_soloed(&mut self, soloed: bool) { self.layer.soloed = soloed; }
    fn locked(&self) -> bool { self.layer.locked }
    fn set_locked(&mut self, locked: bool) { self.layer.locked = locked; }
}
