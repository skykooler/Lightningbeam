use super::{BrushKind, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct DodgeBurnTool;
pub static DODGE_BURN: DodgeBurnTool = DodgeBurnTool;

impl RasterToolDef for DodgeBurnTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::DodgeBurn }
    fn header_label(&self) -> &'static str { "Dodge / Burn" }
    fn brush_kind(&self) -> BrushKind { BrushKind::DodgeBurn }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.dodge_burn_mode as f32, 0.0, 0.0, 0.0]
    }
    fn strength_label(&self) -> &'static str { "Exposure" }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            if ui.selectable_label(s.dodge_burn_mode == 0, "Dodge").clicked() {
                s.dodge_burn_mode = 0;
            }
            if ui.selectable_label(s.dodge_burn_mode == 1, "Burn").clicked() {
                s.dodge_burn_mode = 1;
            }
        });
    }
}
