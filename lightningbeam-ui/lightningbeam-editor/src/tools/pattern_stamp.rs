use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct PatternStampTool;
pub static PATTERN_STAMP: PatternStampTool = PatternStampTool;

const PATTERN_NAMES: &[&str] = &[
    "Checkerboard", "Dots", "H-Lines", "V-Lines", "Diagonal \\", "Diagonal /", "Crosshatch",
];

impl RasterToolDef for PatternStampTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::PatternStamp }
    fn header_label(&self) -> &'static str { "Pattern Stamp" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: s.active_brush_settings.clone(),
            radius: s.brush_radius,
            opacity: s.brush_opacity,
            hardness: s.brush_hardness,
            spacing: s.brush_spacing,
        }
    }
    fn tool_params(&self, s: &RasterToolSettings) -> [f32; 4] {
        [s.pattern_type as f32, s.pattern_scale, 0.0, 0.0]
    }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        let selected_name = PATTERN_NAMES
            .get(s.pattern_type as usize)
            .copied()
            .unwrap_or("Checkerboard");
        ui.horizontal(|ui| {
            ui.label("Pattern:");
            egui::ComboBox::from_id_salt("pattern_type")
                .selected_text(selected_name)
                .show_ui(ui, |ui| {
                    for (i, name) in PATTERN_NAMES.iter().enumerate() {
                        ui.selectable_value(&mut s.pattern_type, i as u32, *name);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Scale:");
            ui.add(egui::Slider::new(&mut s.pattern_scale, 4.0_f32..=256.0)
                .logarithmic(true).suffix(" px"));
        });
        ui.add_space(4.0);
    }
}
