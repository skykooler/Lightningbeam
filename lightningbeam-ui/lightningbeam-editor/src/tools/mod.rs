/// Per-tool module for raster painting tools.
///
/// Each tool implements `RasterToolDef`. Adding a new tool requires:
///   1. A new file in this directory implementing `RasterToolDef`.
///   2. One entry in `raster_tool_def()` below.
///   3. Core changes: `RasterBlendMode` variant, `brush_engine.rs` constant, WGSL branch.

use eframe::egui;
use lightningbeam_core::{
    brush_settings::BrushSettings,
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

// ---------------------------------------------------------------------------
// Shared settings struct (replaces 20+ individual SharedPaneState / EditorApp fields)
// ---------------------------------------------------------------------------

/// All per-tool settings for raster painting.  Owned by `EditorApp`; borrowed
/// by `SharedPaneState` as a single `&'a mut RasterToolSettings`.
pub struct RasterToolSettings {
    // --- Paint brush ---
    pub brush_radius: f32,
    pub brush_opacity: f32,
    pub brush_hardness: f32,
    pub brush_spacing: f32,
    /// true = paint with FG (stroke) color, false = BG (fill) color
    pub brush_use_fg: bool,
    pub active_brush_settings: BrushSettings,
    // --- Eraser ---
    pub eraser_radius: f32,
    pub eraser_opacity: f32,
    pub eraser_hardness: f32,
    pub eraser_spacing: f32,
    pub active_eraser_settings: BrushSettings,
    // --- Smudge ---
    pub smudge_radius: f32,
    pub smudge_hardness: f32,
    pub smudge_spacing: f32,
    pub smudge_strength: f32,
    // --- Clone / Healing ---
    /// World-space source point set by Alt+click.
    pub clone_source: Option<egui::Vec2>,
    // --- Pattern stamp ---
    pub pattern_type: u32,
    pub pattern_scale: f32,
    // --- Dodge / Burn ---
    pub dodge_burn_radius: f32,
    pub dodge_burn_hardness: f32,
    pub dodge_burn_spacing: f32,
    pub dodge_burn_exposure: f32,
    /// 0 = dodge (lighten), 1 = burn (darken)
    pub dodge_burn_mode: u32,
    // --- Sponge ---
    pub sponge_radius: f32,
    pub sponge_hardness: f32,
    pub sponge_spacing: f32,
    pub sponge_flow: f32,
    /// 0 = saturate, 1 = desaturate
    pub sponge_mode: u32,
}

impl Default for RasterToolSettings {
    fn default() -> Self {
        Self {
            brush_radius: 10.0,
            brush_opacity: 1.0,
            brush_hardness: 0.5,
            brush_spacing: 0.1,
            brush_use_fg: true,
            active_brush_settings: BrushSettings::default(),
            eraser_radius: 10.0,
            eraser_opacity: 1.0,
            eraser_hardness: 0.5,
            eraser_spacing: 0.1,
            active_eraser_settings: lightningbeam_core::brush_settings::bundled_brushes()
                .iter()
                .find(|p| p.name == "Brush")
                .map(|p| p.settings.clone())
                .unwrap_or_default(),
            smudge_radius: 15.0,
            smudge_hardness: 0.8,
            smudge_spacing: 8.0,
            smudge_strength: 1.0,
            clone_source: None,
            pattern_type: 0,
            pattern_scale: 32.0,
            dodge_burn_radius: 30.0,
            dodge_burn_hardness: 0.5,
            dodge_burn_spacing: 3.0,
            dodge_burn_exposure: 0.5,
            dodge_burn_mode: 0,
            sponge_radius: 30.0,
            sponge_hardness: 0.5,
            sponge_spacing: 3.0,
            sponge_flow: 0.5,
            sponge_mode: 0,
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

// ---------------------------------------------------------------------------
// RasterToolDef trait
// ---------------------------------------------------------------------------

pub trait RasterToolDef: Send + Sync {
    fn blend_mode(&self) -> RasterBlendMode;
    fn header_label(&self) -> &'static str;
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams;
    /// Encode tool-specific state into the 4-float `StrokeRecord::tool_params`.
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4];
    /// Cursor display radius (world pixels).
    fn cursor_radius(&self, s: &RasterToolSettings) -> f32 {
        self.brush_params(s).radius
    }
    /// Render tool-specific controls in the infopanel (called before preset picker if any).
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings);
    /// Whether to show the brush preset picker after `render_ui`.
    fn show_brush_preset_picker(&self) -> bool { true }
    /// Whether this tool is the eraser (drives preset picker + color UI visibility).
    fn is_eraser(&self) -> bool { false }
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
        _                  => None,
    }
}
