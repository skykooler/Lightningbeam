//! Raster flood-fill action — records and undoes a paint bucket fill on a RasterLayer.

use crate::action::Action;
use crate::actions::raster_diff::RasterDiff;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

pub struct RasterFillAction {
    layer_id: Uuid,
    time: f64,
    width: u32,
    height: u32,
    name: String,
    diff: RasterDiff,
    /// Full post-fill buffer, kept only for the first `execute` (commit); see
    /// `RasterStrokeAction::full_after`.
    full_after: Option<Vec<u8>>,
}

impl RasterFillAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        buffer_before: Vec<u8>,
        buffer_after: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Self {
        let diff = RasterDiff::compute(&buffer_before, &buffer_after, width, height);
        Self { layer_id, time, width, height, name: "Flood fill".to_string(),
               diff, full_after: Some(buffer_after) }
    }

    pub fn with_description(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }
}

impl Action for RasterFillAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let raster = match layer {
            AnyLayer::Raster(rl) => rl,
            _ => return Err("Not a raster layer".to_string()),
        };
        let kf = raster.ensure_keyframe_at(self.time, self.width, self.height);
        if let Some(full) = self.full_after.take() {
            kf.raw_pixels = full;
        } else {
            self.diff.apply_after(&mut kf.raw_pixels);
        }
        kf.texture_dirty = true;
        kf.dirty = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document.get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let raster = match layer {
            AnyLayer::Raster(rl) => rl,
            _ => return Err("Not a raster layer".to_string()),
        };
        let kf = raster.ensure_keyframe_at(self.time, self.width, self.height);
        self.diff.apply_before(&mut kf.raw_pixels);
        kf.texture_dirty = true;
        kf.dirty = true;
        Ok(())
    }

    fn description(&self) -> String {
        self.name.clone()
    }

    fn raster_resident_hint(&self) -> Option<(Uuid, f64)> {
        Some((self.layer_id, self.time))
    }
}
