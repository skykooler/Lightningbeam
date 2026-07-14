use super::{BrushKind, RasterToolDef};
use lightningbeam_core::raster_layer::RasterBlendMode;

pub struct EraseTool;
pub static ERASE: EraseTool = EraseTool;

impl RasterToolDef for EraseTool {
    fn blend_mode(&self) -> RasterBlendMode { RasterBlendMode::Erase }
    fn header_label(&self) -> &'static str { "Eraser" }
    fn brush_kind(&self) -> BrushKind { BrushKind::Erase }
    fn tool_params(&self, _s: &super::RasterToolSettings) -> [f32; 4] { [0.0; 4] }
}
