/// Per-tool module for raster painting tools.
///
/// Each tool implements `RasterToolDef`. Adding a new tool requires:
///   1. A new file in this directory implementing `RasterToolDef`.
///   2. A `BrushKind` variant (if it paints dabs) and one entry in `raster_tool_def()` below.
///   3. Core changes: `RasterBlendMode` variant, `brush_engine.rs` constant, WGSL branch.
///
/// Every dab-painting tool owns a `BrushSlot`: its own size/strength/hardness/spacing, its own
/// brush from the shared `.myb` library, and its own FG/BG color choice. The blend mode is what
/// makes a tool a brush vs. an eraser vs. dodge/burn — the brush shape is orthogonal, so all of
/// them get the same library and the same controls.

use eframe::egui;
use lightningbeam_core::{
    brush_settings::{bundled_brushes, BrushSettings},
    raster_layer::RasterBlendMode,
    tool::Tool,
};

pub mod paint;
pub mod erase;
pub mod smudge;
pub mod clone_stamp;
pub mod healing_brush;
pub mod pattern_stamp;
pub mod dodge_burn;
pub mod sponge;
pub mod blur_sharpen;

// ---------------------------------------------------------------------------
// Brush slots — one per dab-painting tool
// ---------------------------------------------------------------------------

/// Identifies a tool's brush slot. Each slot remembers its own brush independently, so
/// switching from Dodge to Sponge doesn't clobber either one's size or preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BrushKind {
    Paint,
    Erase,
    Smudge,
    CloneStamp,
    HealingBrush,
    PatternStamp,
    DodgeBurn,
    Sponge,
    BlurSharpen,
}

impl BrushKind {
    fn index(self) -> usize {
        match self {
            BrushKind::Paint => 0,
            BrushKind::Erase => 1,
            BrushKind::Smudge => 2,
            BrushKind::CloneStamp => 3,
            BrushKind::HealingBrush => 4,
            BrushKind::PatternStamp => 5,
            BrushKind::DodgeBurn => 6,
            BrushKind::Sponge => 7,
            BrushKind::BlurSharpen => 8,
        }
    }
}

/// One tool's brush: the shared controls every dab-painting tool has.
#[derive(Debug, Clone)]
pub struct BrushSlot {
    pub radius: f32,
    /// What this means is per-tool — opacity, exposure, flow, strength. The label comes from
    /// [`RasterToolDef::strength_label`].
    pub strength: f32,
    pub hardness: f32,
    pub spacing: f32,
    /// The brush shape, from the bundled `.myb` library.
    pub settings: BrushSettings,
    /// Index into `bundled_brushes()` of the selected preset, if any.
    pub preset: Option<usize>,
    /// Added to the preset's `elliptical_dab_angle` (degrees), so stock brushes can be
    /// re-oriented without editing the file.
    pub angle_offset: f32,
    /// true = paint with the FG (stroke) color, false = BG (fill). Ignored when the tool
    /// doesn't use a color (see [`RasterToolDef::uses_color`]).
    pub use_fg: bool,
}

impl BrushSlot {
    fn new(radius: f32, strength: f32, hardness: f32, spacing: f32) -> Self {
        Self {
            radius,
            strength,
            hardness,
            spacing,
            settings: BrushSettings::default(),
            preset: None,
            angle_offset: 0.0,
            use_fg: true,
        }
    }

    /// Adopt a brush preset, pulling its opacity/hardness/spacing along with the shape.
    pub fn apply_preset(&mut self, index: usize, settings: &BrushSettings) {
        self.preset = Some(index);
        self.strength = settings.opaque.clamp(0.0, 1.0);
        self.hardness = settings.hardness.clamp(0.0, 1.0);
        self.spacing = settings.dabs_per_radius;
        self.settings = settings.clone();
    }
}

// ---------------------------------------------------------------------------
// Shared settings struct (replaces 20+ individual SharedPaneState / EditorApp fields)
// ---------------------------------------------------------------------------

/// All per-tool settings for raster painting.  Owned by `EditorApp`; borrowed
/// by `SharedPaneState` as a single `&'a mut RasterToolSettings`.
pub struct RasterToolSettings {
    /// Per-tool brush state, indexed by `BrushKind`.
    brushes: [BrushSlot; 9],
    // --- Clone / Healing ---
    /// World-space source point set by Alt+click.
    pub clone_source: Option<egui::Vec2>,
    // --- Pattern stamp ---
    pub pattern_type: u32,
    pub pattern_scale: f32,
    // --- Dodge / Burn ---
    /// 0 = dodge (lighten), 1 = burn (darken)
    pub dodge_burn_mode: u32,
    // --- Sponge ---
    /// 0 = saturate, 1 = desaturate
    pub sponge_mode: u32,
    // --- Blur / Sharpen ---
    /// Neighborhood kernel radius in canvas pixels (1–20)
    pub blur_sharpen_kernel: f32,
    /// 0 = blur, 1 = sharpen
    pub blur_sharpen_mode: u32,
    // --- Magic wand (raster) ---
    /// Color-distance threshold for magic wand selection (same scale as fill_threshold).
    pub wand_threshold: f32,
    /// Absolute = compare to seed pixel; Relative = compare to BFS parent.
    pub wand_mode: FillThresholdMode,
    /// true = BFS from click (contiguous region only); false = global color scan.
    pub wand_contiguous: bool,
    // --- Quick Select ---
    /// Brush radius in canvas pixels for the quick-select tool.
    pub quick_select_radius: f32,
    // --- Flood fill (Paint Bucket) ---
    /// Color-distance threshold (Euclidean RGBA, 0–510). Pixels within this
    /// distance of the comparison color are included in the fill.
    pub fill_threshold: f32,
    /// Soft-edge width as a percentage of the threshold (0 = hard, 100 = full fade).
    pub fill_softness: f32,
    /// Whether to compare each pixel to the seed pixel (Absolute) or to its BFS
    /// parent pixel (Relative, spreads across gradients).
    pub fill_threshold_mode: FillThresholdMode,
    /// true = fill with the FG (stroke) color, false = BG (fill). Mirrors `BrushSlot::use_fg`.
    pub fill_use_fg: bool,
    // --- Eyedropper ---
    /// true = sampled color replaces the FG (stroke) swatch, false = the BG (fill) swatch.
    pub eyedropper_use_fg: bool,
    // --- Marquee select shape ---
    /// Whether the rectangular select tool draws a rect or an ellipse.
    pub select_shape: SelectionShape,
    // --- Warp ---
    pub warp_grid_cols: u32,
    pub warp_grid_rows: u32,
    // --- Liquify ---
    pub liquify_mode:     LiquifyMode,
    pub liquify_radius:   f32,
    pub liquify_strength: f32,
    // --- Gradient ---
    pub gradient: lightningbeam_core::gradient::ShapeGradient,
    pub gradient_opacity: f32,
}

impl RasterToolSettings {
    pub fn brush(&self, kind: BrushKind) -> &BrushSlot {
        &self.brushes[kind.index()]
    }

    pub fn brush_mut(&mut self, kind: BrushKind) -> &mut BrushSlot {
        &mut self.brushes[kind.index()]
    }
}

/// Brush mode for the Liquify tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiquifyMode {
    #[default]
    Push,
    Pucker,
    Bloat,
    Smooth,
    Reconstruct,
}

impl LiquifyMode {
    pub fn as_u32(self) -> u32 {
        match self {
            LiquifyMode::Push        => 0,
            LiquifyMode::Pucker      => 1,
            LiquifyMode::Bloat       => 2,
            LiquifyMode::Smooth      => 3,
            LiquifyMode::Reconstruct => 4,
        }
    }
}

/// Shape mode for the rectangular-select tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionShape {
    #[default]
    Rect,
    Ellipse,
}

/// Threshold comparison mode for the raster flood fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillThresholdMode {
    /// Compare each candidate pixel to the original seed pixel (default).
    #[default]
    Absolute,
    /// Compare each candidate pixel to the pixel it was reached from.
    Relative,
}

impl Default for RasterToolSettings {
    fn default() -> Self {
        // The eraser defaults to the bundled "Brush" shape rather than a bare Gaussian.
        let mut erase = BrushSlot::new(10.0, 1.0, 0.5, 0.1);
        if let Some(idx) = bundled_brushes().iter().position(|p| p.name == "Brush") {
            erase.apply_preset(idx, &bundled_brushes()[idx].settings);
            // Keep the eraser's own size/opacity rather than the preset's.
            erase.radius = 10.0;
            erase.strength = 1.0;
        }

        Self {
            brushes: [
                /* Paint        */ BrushSlot::new(10.0, 1.0, 0.5, 0.1),
                /* Erase        */ erase,
                /* Smudge       */ BrushSlot::new(15.0, 1.0, 0.8, 8.0),
                /* CloneStamp   */ BrushSlot::new(10.0, 1.0, 0.5, 0.1),
                /* HealingBrush */ BrushSlot::new(10.0, 1.0, 0.5, 0.1),
                /* PatternStamp */ BrushSlot::new(10.0, 1.0, 0.5, 0.1),
                /* DodgeBurn    */ BrushSlot::new(30.0, 0.5, 0.5, 3.0),
                /* Sponge       */ BrushSlot::new(30.0, 0.5, 0.5, 3.0),
                /* BlurSharpen  */ BrushSlot::new(30.0, 0.5, 0.5, 3.0),
            ],
            clone_source: None,
            pattern_type: 0,
            pattern_scale: 32.0,
            dodge_burn_mode: 0,
            sponge_mode: 0,
            blur_sharpen_kernel: 5.0,
            blur_sharpen_mode: 0,
            wand_threshold: 15.0,
            wand_mode: FillThresholdMode::Absolute,
            wand_contiguous: true,
            fill_threshold: 15.0,
            fill_softness: 0.0,
            fill_threshold_mode: FillThresholdMode::Absolute,
            fill_use_fg: true,
            eyedropper_use_fg: true,
            quick_select_radius: 20.0,
            select_shape: SelectionShape::Rect,
            warp_grid_cols: 4,
            warp_grid_rows: 4,
            liquify_mode:     LiquifyMode::Push,
            liquify_radius:   50.0,
            liquify_strength: 0.5,
            gradient:         lightningbeam_core::gradient::ShapeGradient::default(),
            gradient_opacity: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Brush parameters extracted per-tool
// ---------------------------------------------------------------------------

pub struct BrushParams {
    pub base_settings: BrushSettings,
    pub radius: f32,
    pub opacity: f32,
    pub hardness: f32,
    pub spacing: f32,
}

/// Read a slot straight into `BrushParams`. This is what `RasterToolDef::brush_params` does by
/// default; it's a free function so a tool that overrides `brush_params` can still build on it.
pub fn default_brush_params(kind: BrushKind, s: &RasterToolSettings) -> BrushParams {
    let slot = s.brush(kind);
    let mut base_settings = slot.settings.clone();
    base_settings.elliptical_dab_angle += slot.angle_offset;
    BrushParams {
        base_settings,
        radius: slot.radius,
        opacity: slot.strength,
        hardness: slot.hardness,
        spacing: slot.spacing,
    }
}

// ---------------------------------------------------------------------------
// RasterToolDef trait
// ---------------------------------------------------------------------------

pub trait RasterToolDef: Send + Sync {
    fn blend_mode(&self) -> RasterBlendMode;
    fn header_label(&self) -> &'static str;
    /// Which brush slot this tool paints with.
    fn brush_kind(&self) -> BrushKind;
    /// Encode tool-specific state into the 4-float `StrokeRecord::tool_params`.
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4];

    /// Brush shape + size for this stroke. The default pulls everything from the tool's slot;
    /// override only if a tool needs to reinterpret one of the fields (see `smudge`).
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        default_brush_params(self.brush_kind(), s)
    }

    /// Cursor display radius (world pixels).
    fn cursor_radius(&self, s: &RasterToolSettings) -> f32 {
        self.brush_params(s).radius
    }

    /// Label for the slot's `strength` slider — the one field whose meaning is tool-specific.
    fn strength_label(&self) -> &'static str { "Opacity" }

    /// Render tool-specific controls in the infopanel. The shared brush controls (color, size,
    /// strength, hardness, spacing, angle, preset library) are rendered around this by the
    /// infopanel — only put genuinely tool-unique widgets here.
    fn render_ui(&self, _ui: &mut egui::Ui, _s: &mut RasterToolSettings) {}

    /// Whether this tool paints with the FG/BG color. False for tools that only transform
    /// pixels already on the canvas (erase, smudge, dodge/burn, sponge, blur, clone, heal).
    fn uses_color(&self) -> bool { false }

    /// Whether Alt+click sets a source point for this tool.
    fn uses_alt_click(&self) -> bool { false }
}

// ---------------------------------------------------------------------------
// Lookup: Tool → &'static dyn RasterToolDef
// ---------------------------------------------------------------------------

pub fn raster_tool_def(tool: &Tool) -> Option<&'static dyn RasterToolDef> {
    match tool {
        Tool::Draw | Tool::Pencil | Tool::Pen | Tool::Airbrush => Some(&paint::PAINT),
        Tool::Erase        => Some(&erase::ERASE),
        Tool::Smudge       => Some(&smudge::SMUDGE),
        Tool::CloneStamp   => Some(&clone_stamp::CLONE_STAMP),
        Tool::HealingBrush => Some(&healing_brush::HEALING_BRUSH),
        Tool::PatternStamp => Some(&pattern_stamp::PATTERN_STAMP),
        Tool::DodgeBurn    => Some(&dodge_burn::DODGE_BURN),
        Tool::Sponge       => Some(&sponge::SPONGE),
        Tool::BlurSharpen  => Some(&blur_sharpen::BLUR_SHARPEN),
        _                  => None,
    }
}
