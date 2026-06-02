use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct HealingBrushTool;
pub static HEALING_BRUSH: HealingBrushTool = HealingBrushTool;

impl RasterToolDef for HealingBrushTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Healing }
    fn header_label(&self) -> &'static str { "Healing Brush" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: s.active_brush_settings.clone(),
            radius: s.brush_radius,
            opacity: s.brush_opacity,
            hardness: s.brush_hardness,
            spacing: s.brush_spacing,
        }
    }
    /// tool_params are filled by stage.rs at stroke-start time (clone offset).
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn uses_alt_click(&self) -> bool { true }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        if s.clone_source.is_none() {
            ui.label("Alt+click to set source point.");
        }
    }
}
