use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct EraseTool;
pub static ERASE: EraseTool = EraseTool;

impl RasterToolDef for EraseTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Erase }
    fn header_label(&self) -> &'static str { "Eraser" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: s.active_eraser_settings.clone(),
            radius: s.eraser_radius,
            opacity: s.eraser_opacity,
            hardness: s.eraser_hardness,
            spacing: s.eraser_spacing,
        }
    }
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn is_eraser(&self) -> bool { true }
    fn render_ui(&self, _ui: &mut egui::Ui, _s: &mut RasterToolSettings) {}
}
