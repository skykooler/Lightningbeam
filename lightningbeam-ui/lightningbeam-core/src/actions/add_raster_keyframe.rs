//! Add a blank raster keyframe at the playhead (the explicit "New Keyframe" command
//! for raster layers — mirrors `SetKeyframeAction` for vector, but inserts an empty
//! cel rather than copying the current graph).

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

pub struct AddRasterKeyframeAction {
    layer_id: Uuid,
    time: f64,
    width: u32,
    height: u32,
    /// Id of the keyframe created by the last `execute` (so `rollback` can remove
    /// exactly that one). `None` if a keyframe already existed at `time` (no-op).
    created_id: Option<Uuid>,
}

impl AddRasterKeyframeAction {
    pub fn new(layer_id: Uuid, time: f64, width: u32, height: u32) -> Self {
        Self { layer_id, time, width, height, created_id: None }
    }
}

impl Action for AddRasterKeyframeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let rl = match layer {
            AnyLayer::Raster(rl) => rl,
            _ => return Err("Not a raster layer".to_string()),
        };
        // Inserts a blank cel only if one doesn't already exist at this time.
        self.created_id = rl.insert_blank_keyframe_at(self.time, self.width, self.height);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(id) = self.created_id.take() {
            if let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&self.layer_id) {
                rl.remove_keyframe(id);
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        "New keyframe".to_string()
    }
}
