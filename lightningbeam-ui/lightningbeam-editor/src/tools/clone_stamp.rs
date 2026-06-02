use super::{BrushParams, RasterToolDef, RasterToolSettings};
use eframe::egui;
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct CloneStampTool;
pub static CLONE_STAMP: CloneStampTool = CloneStampTool;

impl RasterToolDef for CloneStampTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::CloneStamp }
    fn header_label(&self) -> &'static str { "Clone Stamp" }
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        BrushParams {
            base_settings: s.active_brush_settings.clone(),
            radius: s.brush_radius,
            opacity: s.brush_opacity,
            hardness: s.brush_hardness,
            spacing: s.brush_spacing,
        }
    }
    /// For Clone Stamp, tool_params are filled by stage.rs at stroke-start time
    /// (offset = clone_source - stroke_start), not from settings directly.
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn uses_alt_click(&self) -> bool { true }
    fn render_ui(&self, ui: &mut egui::Ui, s: &mut RasterToolSettings) {
        if s.clone_source.is_none() {
            ui.label("Alt+click to set source point.");
        }
    }
}
