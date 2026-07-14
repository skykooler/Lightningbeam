use super::{BrushKind, RasterToolDef};
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct PaintTool;
pub static PAINT: PaintTool = PaintTool;

impl RasterToolDef for PaintTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Normal }
    fn header_label(&self) -> &'static str { "Brush" }
    fn brush_kind(&self) -> BrushKind { BrushKind::Paint }
    fn tool_params(&self, _s: &super::RasterToolSettings) -> [f32; 4] { [0.0; 4] }
    fn uses_color(&self) -> bool { true }
}
