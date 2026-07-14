use super::{BrushKind, BrushParams, RasterToolDef, RasterToolSettings};
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct SmudgeTool;
pub static SMUDGE: SmudgeTool = SmudgeTool;

impl RasterToolDef for SmudgeTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Smudge }
    fn header_label(&self) -> &'static str { "Smudge" }
    fn brush_kind(&self) -> BrushKind { BrushKind::Smudge }
    fn tool_params(&self, _s: &RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn strength_label(&self) -> &'static str { "Strength" }

    /// Smudge's slot `strength` drives the smudge distance (applied by the stage as
    /// `smudge_radius_log`), not dab opacity — dabs always composite fully.
    fn brush_params(&self, s: &RasterToolSettings) -> BrushParams {
        let mut p = super::default_brush_params(self.brush_kind(), s);
        p.opacity = 1.0;
        p
    }
}
