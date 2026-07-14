use super::{BrushKind, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct BlurSharpenTool;
pub static BLUR_SHARPEN: BlurSharpenTool = BlurSharpenTool;

impl RasterToolDef for BlurSharpenTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::BlurSharpen }
    fn header_label(&self) -> &'static str { "Blur / Sharpen" }
    fn brush_kind(&self) -> BrushKind { BrushKind::BlurSharpen }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.blur_sharpen_mode as f32, s.blur_sharpen_kernel, 0.0, 0.0]
    }
    fn strength_label(&self) -> &'static str { "Strength" }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            if ui.selectable_label(s.blur_sharpen_mode == 0, "Blur").clicked() {
                s.blur_sharpen_mode = 0;
            }
            if ui.selectable_label(s.blur_sharpen_mode == 1, "Sharpen").clicked() {
                s.blur_sharpen_mode = 1;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Kernel:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_kernel, 1.0_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1} px", v)));
        });
    }
}
