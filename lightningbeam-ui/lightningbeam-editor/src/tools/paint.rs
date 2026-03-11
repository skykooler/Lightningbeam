use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct PaintTool;
pub static PAINT: PaintTool = PaintTool;

impl RasterToolDef for PaintTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Normal }
    fn header_label(&self) -> &'static str { "Brush" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        let mut base_settings = s.active_brush_settings.clone();
        base_settings.elliptical_dab_angle += s.brush_angle_offset;
        BrushParams {
            base_settings,
            radius: s.brush_radius,
            opacity: s.brush_opacity,
            hardness: s.brush_hardness,
            spacing: s.brush_spacing,
        }
    }
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn render_ui(&self, _ui: &mut egui::Ui, _s: &mut RasterToolSettings) {}
}
