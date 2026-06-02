use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::{brush_settings::BrushSettings, raster_layer::RasterBlendMode};

pub struct BlurSharpenTool;
pub static BLUR_SHARPEN: BlurSharpenTool = BlurSharpenTool;

impl RasterToolDef for BlurSharpenTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::BlurSharpen }
    fn header_label(&self) -> &'static str { "Blur / Sharpen" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: BrushSettings::default(),
            radius: s.blur_sharpen_radius,
            opacity: s.blur_sharpen_strength,
            hardness: s.blur_sharpen_hardness,
            spacing: s.blur_sharpen_spacing,
        }
    }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.blur_sharpen_mode as f32, s.blur_sharpen_kernel, 0.0, 0.0]
    }
    fn show_brush_preset_picker(&self) -> bool { false }
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
            ui.label("Size:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_radius, 1.0_f32..=500.0).logarithmic(true).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Strength:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_strength, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Hardness:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_hardness, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Kernel:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_kernel, 1.0_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1} px", v)));
        });
        ui.horizontal(|ui| {
            ui.label("Spacing:");
            ui.add(egui::Slider::new(&mut s.blur_sharpen_spacing, 0.5_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1}", v)));
        });
    }
}
