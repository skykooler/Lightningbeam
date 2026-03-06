//! Raster (pixel-buffer) layer for Lightningbeam
//!
//! Each keyframe holds a PNG-compressed pixel buffer stored in the .beam ZIP
//! under `media/raster/<uuid>.png`. A brush engine renders dabs along strokes
//! and the resulting RGBA image is composited into the Vello scene.

use crate::brush_settings::BrushSettings;
use crate::layer::{Layer, LayerTrait, LayerType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How a raster stroke blends onto the layer buffer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RasterBlendMode {
    /// Normal alpha-compositing (paint over)
    Normal,
    /// Erase pixels (reduce alpha)
    Erase,
    /// Smudge / blend surrounding pixels
    Smudge,
    /// Clone stamp: copy pixels from a source region
    CloneStamp,
    /// Healing brush: color-corrected clone stamp (preserves source texture, shifts color to match destination)
    Healing,
    /// Pattern stamp: paint with a repeating procedural tile pattern
    PatternStamp,
    /// Dodge / Burn: lighten (dodge) or darken (burn) existing pixels
    DodgeBurn,
}

impl Default for RasterBlendMode {
    fn default() -> Self {
        Self::Normal
    }
}

/// A single point along a stroke
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f32,
    pub y: f32,
    /// Pen/tablet pressure 0.0–1.0 (mouse uses 0.5)
    pub pressure: f32,
    /// Pen tilt X in radians
    pub tilt_x: f32,
    /// Pen tilt Y in radians
    pub tilt_y: f32,
    /// Seconds elapsed since start of this stroke
    pub timestamp: f64,
}

/// Record of a single brush stroke applied to a keyframe
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StrokeRecord {
    pub brush_settings: BrushSettings,
    /// RGBA linear color [r, g, b, a]
    pub color: [f32; 4],
    pub blend_mode: RasterBlendMode,
    /// Clone stamp source offset: (source_x - drag_start_x, source_y - drag_start_y).
    /// For each dab at canvas position D, the source pixel is sampled from D + offset.
    /// None for all non-clone-stamp blend modes.
    #[serde(default)]
    pub clone_src_offset: Option<(f32, f32)>,
    /// Pattern stamp: procedural pattern type (0=Checkerboard, 1=Dots, 2=H-Lines, 3=V-Lines, 4=Diagonal, 5=Crosshatch)
    #[serde(default)]
    pub pattern_type: u32,
    /// Pattern stamp: tile size in pixels
    #[serde(default = "default_pattern_scale")]
    pub pattern_scale: f32,
    /// Dodge/Burn mode: 0 = dodge (lighten), 1 = burn (darken)
    #[serde(default)]
    pub dodge_burn_mode: u32,
    pub points: Vec<StrokePoint>,
}

fn default_pattern_scale() -> f32 { 32.0 }

/// Specifies how the raster content transitions to the next keyframe
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TweenType {
    /// Hold the pixel buffer until the next keyframe
    Hold,
}

impl Default for TweenType {
    fn default() -> Self {
        Self::Hold
    }
}

/// A single keyframe of a raster layer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RasterKeyframe {
    /// Unique ID for this keyframe (used as pixel-cache key)
    pub id: Uuid,
    /// Time position in seconds
    pub time: f64,
    pub width: u32,
    pub height: u32,
    /// ZIP-relative path: `"media/raster/<uuid>.png"`
    pub media_path: String,
    /// Stroke history (for potential replay / future non-destructive editing)
    pub stroke_log: Vec<StrokeRecord>,
    pub tween_after: TweenType,
    /// Raw RGBA pixel buffer (width × height × 4 bytes).
    ///
    /// This is the working in-memory representation used by the brush engine and renderer.
    /// NOT serialized to the document JSON — populated from the ZIP's PNG on load,
    /// and encoded back to PNG on save.  An empty Vec means the canvas is blank (transparent).
    #[serde(skip)]
    pub raw_pixels: Vec<u8>,
}

impl RasterKeyframe {
    /// Returns true when the pixel buffer has been initialised (non-blank).
    pub fn has_pixels(&self) -> bool {
        !self.raw_pixels.is_empty()
    }

    pub fn new(time: f64, width: u32, height: u32) -> Self {
        let id = Uuid::new_v4();
        let media_path = format!("media/raster/{}.png", id);
        Self {
            id,
            time,
            width,
            height,
            media_path,
            stroke_log: Vec::new(),
            tween_after: TweenType::Hold,
            raw_pixels: Vec::new(),
        }
    }
}

/// A pixel-buffer painting layer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RasterLayer {
    /// Base layer properties (id, name, opacity, visibility, …)
    pub layer: Layer,
    /// Keyframes sorted by time
    pub keyframes: Vec<RasterKeyframe>,
}

impl RasterLayer {
    /// Create a new raster layer with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::new(LayerType::Raster, name),
            keyframes: Vec::new(),
        }
    }

    // === Keyframe accessors ===

    /// Get the active keyframe at-or-before `time`
    pub fn keyframe_at(&self, time: f64) -> Option<&RasterKeyframe> {
        let idx = self.keyframes.partition_point(|kf| kf.time <= time);
        if idx > 0 { Some(&self.keyframes[idx - 1]) } else { None }
    }

    /// Get a mutable reference to the active keyframe at-or-before `time`
    pub fn keyframe_at_mut(&mut self, time: f64) -> Option<&mut RasterKeyframe> {
        let idx = self.keyframes.partition_point(|kf| kf.time <= time);
        if idx > 0 { Some(&mut self.keyframes[idx - 1]) } else { None }
    }

    /// Find the index of a keyframe at exactly `time` (within tolerance)
    fn keyframe_index_at_exact(&self, time: f64, tolerance: f64) -> Option<usize> {
        self.keyframes.iter().position(|kf| (kf.time - time).abs() < tolerance)
    }

    /// Ensure a keyframe exists at `time`; create one (with given dimensions) if not.
    ///
    /// If `width`/`height` are 0 the new keyframe inherits dimensions from the
    /// previous active keyframe, falling back to 1920×1080.
    pub fn ensure_keyframe_at(&mut self, time: f64, width: u32, height: u32) -> &mut RasterKeyframe {
        let tolerance = 0.001;
        if let Some(idx) = self.keyframe_index_at_exact(time, tolerance) {
            return &mut self.keyframes[idx];
        }

        let (w, h) = if width == 0 || height == 0 {
            self.keyframe_at(time)
                .map(|kf| (kf.width, kf.height))
                .unwrap_or((1920, 1080))
        } else {
            (width, height)
        };

        let insert_idx = self.keyframes.partition_point(|kf| kf.time < time);
        self.keyframes.insert(insert_idx, RasterKeyframe::new(time, w, h));
        &mut self.keyframes[insert_idx]
    }

    /// Return the ZIP-relative PNG path for the active keyframe at `time`, or `None`.
    pub fn buffer_path_at_time(&self, time: f64) -> Option<&str> {
        self.keyframe_at(time).map(|kf| kf.media_path.as_str())
    }
}

// Delegate all LayerTrait methods to self.layer
impl LayerTrait for RasterLayer {
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
