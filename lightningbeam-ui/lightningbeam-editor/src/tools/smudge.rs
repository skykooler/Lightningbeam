use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::{brush_settings::BrushSettings, raster_layer::RasterBlendMode};

pub struct SmudgeTool;
pub static SMUDGE: SmudgeTool = SmudgeTool;

impl RasterToolDef for SmudgeTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Smudge }
    fn header_label(&self) -> &'static str { "Smudge" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: BrushSettings::default(),
            radius: s.smudge_radius,
            opacity: 1.0, // strength is a separate smudge_dist multiplier
            hardness: s.smudge_hardness,
            spacing: s.smudge_spacing,
        }
    }
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn show_brush_preset_picker(&self) -> bool { false }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            ui.label("Size:");
            ui.add(egui::Slider::new(&mut s.smudge_radius, 1.0_f32..=200.0).logarithmic(true).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Strength:");
            ui.add(egui::Slider::new(&mut s.smudge_strength, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Hardness:");
            ui.add(egui::Slider::new(&mut s.smudge_hardness, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Spacing:");
            ui.add(egui::Slider::new(&mut s.smudge_spacing, 0.5_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1}", v)));
        });
    }
}
