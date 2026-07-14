use super::{BrushKind, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct SpongeTool;
pub static SPONGE: SpongeTool = SpongeTool;

impl RasterToolDef for SpongeTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Sponge }
    fn header_label(&self) -> &'static str { "Sponge" }
    fn brush_kind(&self) -> BrushKind { BrushKind::Sponge }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.sponge_mode as f32, 0.0, 0.0, 0.0]
    }
    fn strength_label(&self) -> &'static str { "Flow" }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            if ui.selectable_label(s.sponge_mode == 0, "Saturate").clicked() {
                s.sponge_mode = 0;
            }
            if ui.selectable_label(s.sponge_mode == 1, "Desaturate").clicked() {
                s.sponge_mode = 1;
            }
        });
    }
}
