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
    /// Sponge: saturate or desaturate existing pixels
    Sponge,
    /// Blur / Sharpen: soften or crisp up existing pixels
    BlurSharpen,
}

impl Default for RasterBlendMode {
    fn default() -> Self {
        Self::Normal
    }
}

impl RasterBlendMode {
    /// Returns false for blend modes that operate on existing pixels and don't
    /// use the brush color at all (clone, heal, dodge/burn, sponge).
    /// Used by brush_engine.rs to decide whether color_a should be 1.0 or stroke.color[3].
    pub fn uses_brush_color(self) -> bool {
        !matches!(self, Self::CloneStamp | Self::Healing | Self::DodgeBurn | Self::Sponge | Self::BlurSharpen)
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
    /// Generic tool parameters — encoding depends on blend_mode:
    /// - CloneStamp / Healing: [offset_x, offset_y, 0, 0]
    /// - PatternStamp:         [pattern_type, pattern_scale, 0, 0]
    /// - DodgeBurn / Sponge:   [mode, 0, 0, 0]
    /// - all others:           [0, 0, 0, 0]
    #[serde(default)]
    pub tool_params: [f32; 4],
    pub points: Vec<StrokePoint>,
}

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

/// A low-res decoded RGBA proxy of a keyframe's pixels, shown while the full-res
/// buffer pages in from the container so cold scrubs don't flash blank.
#[derive(Clone, Debug)]
pub struct RasterProxy {
    pub width: u32,
    pub height: u32,
    /// RGBA, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
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
    /// Set to `true` whenever `raw_pixels` changes so the GPU texture cache can re-upload.
    /// Always `true` after load; cleared by the renderer after uploading.
    #[serde(skip, default = "default_true")]
    pub texture_dirty: bool,
    /// Phase 3 paging: the keyframe's pixels live in the container and must be
    /// faulted in (`raw_pixels` empty *and* this true ⇒ page in from the store).
    /// A *new* keyframe is `false` (intentionally blank/resident, nothing to load);
    /// set true on load and again when evicted. Never serialized.
    #[serde(skip)]
    pub needs_fault_in: bool,
    /// Phase 3a eviction: set `true` whenever user editing mutates `raw_pixels`
    /// (brush, fill, paint-bucket, floating-selection commit/lift, undo/redo of
    /// those). A dirty keyframe's current pixels are NOT yet persisted in the
    /// container, so it must NEVER be evicted (doing so would silently lose the
    /// unsaved edit). Cleared on a successful save. Never serialized.
    #[serde(skip)]
    pub dirty: bool,
    /// Phase 3a-3: low-res proxy decoded from the container on load, rendered while
    /// the full pixels page in (removes the cold-scrub blank flash). `None` if the
    /// keyframe has no persisted proxy yet (new/unsaved, or pre-proxy project).
    #[serde(skip)]
    pub proxy: Option<RasterProxy>,
}

fn default_true() -> bool { true }

impl RasterKeyframe {
    /// Returns true when the pixel buffer has been initialised (non-blank).
    pub fn has_pixels(&self) -> bool {
        !self.raw_pixels.is_empty()
    }

    pub fn new(time: f64, width: u32, height: u32) -> Self {
        let id = Uuid::new_v4();
        Self {
            id,
            time,
            width,
            height,
            stroke_log: Vec::new(),
            tween_after: TweenType::Hold,
            raw_pixels: Vec::new(),
            texture_dirty: true,
            needs_fault_in: false,
            dirty: false,
            proxy: None,
        }
    }

    /// Change the canvas to `(new_w, new_h)`. `Scale` resamples the content (Lanczos3) to fill the
    /// new size; `Canvas` keeps the content at native resolution anchored top-left, padding with
    /// transparent (expand) or trimming (crop). No-op if the size is unchanged. If pixels aren't
    /// resident the buffer is left empty and only the declared size changes — the caller must fault
    /// pixels in first (a later load would otherwise mismatch the new size).
    pub fn resize_to(&mut self, new_w: u32, new_h: u32, mode: RasterResizeMode) {
        if new_w == 0 || new_h == 0 || (self.width == new_w && self.height == new_h) {
            return;
        }
        // Resample/recanvas the buffer only when pixels are resident. A blank keyframe (no content
        // and no store row) just takes the new declared size — there's nothing to corrupt. Paged-out
        // keyframes are loaded by the caller (ResizeRasterLayerAction) before this runs.
        if !self.raw_pixels.is_empty() {
            let old = std::mem::take(&mut self.raw_pixels);
            self.raw_pixels = match mode {
                RasterResizeMode::Scale => match image::RgbaImage::from_raw(self.width, self.height, old) {
                    Some(img) => image::imageops::resize(&img, new_w, new_h, image::imageops::FilterType::Lanczos3).into_raw(),
                    None => vec![0u8; (new_w as usize) * (new_h as usize) * 4],
                },
                RasterResizeMode::Canvas => {
                    // Copy the old pixels into a transparent new buffer, anchored top-left (matching
                    // the raster's (0,0) document anchor); right/bottom is padded or trimmed.
                    let mut buf = vec![0u8; (new_w as usize) * (new_h as usize) * 4];
                    let copy_w = self.width.min(new_w) as usize;
                    let copy_h = self.height.min(new_h) as usize;
                    let (ow, nw) = (self.width as usize, new_w as usize);
                    for y in 0..copy_h {
                        let src = y * ow * 4;
                        let dst = y * nw * 4;
                        buf[dst..dst + copy_w * 4].copy_from_slice(&old[src..src + copy_w * 4]);
                    }
                    buf
                }
            };
            self.proxy = None; // invalidate any downsampled proxy
        }
        self.width = new_w;
        self.height = new_h;
        self.texture_dirty = true;
        self.dirty = true;
    }
}

/// How a raster canvas resize treats existing pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RasterResizeMode {
    /// Resample the content to fill the new size (changes pixel resolution).
    Scale,
    /// Keep content at native resolution, anchored top-left; pad/trim the canvas.
    Canvas,
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

    /// Insert a blank keyframe at `time` if none exists there (within tolerance).
    /// Returns the new keyframe's id if one was created, `None` if a keyframe already
    /// existed. Used by the explicit "New Keyframe" command (blank cel).
    pub fn insert_blank_keyframe_at(&mut self, time: f64, width: u32, height: u32) -> Option<Uuid> {
        if self.keyframe_index_at_exact(time, 0.001).is_some() {
            return None;
        }
        let (w, h) = if width == 0 || height == 0 {
            self.keyframe_at(time)
                .map(|kf| (kf.width, kf.height))
                .unwrap_or((1920, 1080))
        } else {
            (width, height)
        };
        let insert_idx = self.keyframes.partition_point(|kf| kf.time < time);
        let kf = RasterKeyframe::new(time, w, h);
        let id = kf.id;
        self.keyframes.insert(insert_idx, kf);
        Some(id)
    }

    /// Remove the keyframe with the given id, returning it if found.
    pub fn remove_keyframe(&mut self, id: Uuid) -> Option<RasterKeyframe> {
        self.keyframes
            .iter()
            .position(|kf| kf.id == id)
            .map(|pos| self.keyframes.remove(pos))
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
