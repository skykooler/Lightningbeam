use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::{brush_settings::BrushSettings, raster_layer::RasterBlendMode};

pub struct DodgeBurnTool;
pub static DODGE_BURN: DodgeBurnTool = DodgeBurnTool;

impl RasterToolDef for DodgeBurnTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::DodgeBurn }
    fn header_label(&self) -> &'static str { "Dodge / Burn" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: BrushSettings::default(),
            radius: s.dodge_burn_radius,
            opacity: s.dodge_burn_exposure,
            hardness: s.dodge_burn_hardness,
            spacing: s.dodge_burn_spacing,
        }
    }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.dodge_burn_mode as f32, 0.0, 0.0, 0.0]
    }
    fn show_brush_preset_picker(&self) -> bool { false }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        ui.horizontal(|ui| {
            if ui.selectable_label(s.dodge_burn_mode == 0, "Dodge").clicked() {
                s.dodge_burn_mode = 0;
            }
            if ui.selectable_label(s.dodge_burn_mode == 1, "Burn").clicked() {
                s.dodge_burn_mode = 1;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Size:");
            ui.add(egui::Slider::new(&mut s.dodge_burn_radius, 1.0_f32..=500.0).logarithmic(true).suffix(" px"));
        });
        ui.horizontal(|ui| {
            ui.label("Exposure:");
            ui.add(egui::Slider::new(&mut s.dodge_burn_exposure, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Hardness:");
            ui.add(egui::Slider::new(&mut s.dodge_burn_hardness, 0.0_f32..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
        });
        ui.horizontal(|ui| {
            ui.label("Spacing:");
            ui.add(egui::Slider::new(&mut s.dodge_burn_spacing, 0.5_f32..=20.0)
                .logarithmic(true)
                .custom_formatter(|v, _| format!("{:.1}", v)));
        });
    }
}
