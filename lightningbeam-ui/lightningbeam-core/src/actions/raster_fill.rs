//! Raster flood-fill action — records and undoes a paint bucket fill on a RasterLayer.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

pub struct RasterFillAction {
    layer_id: Uuid,
    time: f64,
    buffer_before: Vec<u8>,
    buffer_after: Vec<u8>,
    width: u32,
    height: u32,
    name: String,
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
        Self { layer_id, time, buffer_before, buffer_after, width, height, name: "Flood fill".to_string() }
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
        kf.raw_pixels = self.buffer_after.clone();
        kf.texture_dirty = true;
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
        kf.raw_pixels = self.buffer_before.clone();
        kf.texture_dirty = true;
        Ok(())
    }

    fn description(&self) -> String {
        self.name.clone()
    }
}
