use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::{brush_settings::BrushSettings, raster_layer::RasterBlendMode};

pub struct SpongeTool;
pub static SPONGE: SpongeTool = SpongeTool;

impl RasterToolDef for SpongeTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Sponge }
    fn header_label(&self) -> &'static str { "Sponge" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: BrushSettings::default(),
            radius: s.sponge_radius,
            opacity: s.sponge_flow,
            hardness: s.sponge_hardness,
            spacing: s.sponge_spacing,
        }
    }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.sponge_mode as f32, 0.0, 0.0, 0.0]
    }
    fn show_brush_preset_picker(&self) -> bool { false }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            if ui.selectable_label(s.sponge_mode == 0, "Saturate").clicked() {
                s.sponge_mode = 0;
            }
            if ui.selectable_label(s.sponge_mode == 1, "Desaturate").clicked() {
                s.sponge_mode = 1;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size:");
            ui.add(egui::Slider::new(&mut s.sponge_radius, 1.0_f32..=500.0).logarithmic(true).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Flow:");
            ui.add(egui::Slider::new(&mut s.sponge_flow, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Hardness:");
            ui.add(egui::Slider::new(&mut s.sponge_hardness, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Spacing:");
            ui.add(egui::Slider::new(&mut s.sponge_spacing, 0.5_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1}", v)));
        });
    }
}
