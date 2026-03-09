//! Raster stroke action — records and undoes a brush stroke on a RasterLayer.
//!
//! The brush engine paints directly into `RasterKeyframe::raw_pixels` during the
//! drag (via `document_mut()`).  This action captures the pixel buffer state
//! *before* and *after* the stroke so it can be undone / redone without
//! re-running the brush engine.
//!
//! `execute` → swap in `buffer_after`
//! `rollback` → swap in `buffer_before`

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that records a single brush stroke for undo/redo.
///
/// The stroke must already be painted into the document's `raw_pixels` before
/// this action is executed for the first time.
pub struct RasterStrokeAction {
    layer_id: Uuid,
    time: f64,
    /// Raw RGBA pixels *before* the stroke (for rollback / undo)
    buffer_before: Vec<u8>,
    /// Raw RGBA pixels *after* the stroke (for execute / redo)
    buffer_after: Vec<u8>,
    width: u32,
    height: u32,
}

impl RasterStrokeAction {
    /// Create the action.
    ///
    /// * `buffer_before` – raw RGBA pixels captured just before the stroke began.
    /// * `buffer_after`  – raw RGBA pixels captured just after the stroke finished.
    pub fn new(
        layer_id: Uuid,
        time: f64,
        buffer_before: Vec<u8>,
        buffer_after: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Self {
        Self { layer_id, time, buffer_before, buffer_after, width, height }
    }
}

impl Action for RasterStrokeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let kf = get_keyframe_mut(document, &self.layer_id, self.time, self.width, self.height)?;
        kf.raw_pixels = self.buffer_after.clone();
        kf.texture_dirty = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let kf = get_keyframe_mut(document, &self.layer_id, self.time, self.width, self.height)?;
        kf.raw_pixels = self.buffer_before.clone();
        kf.texture_dirty = true;
        Ok(())
    }

    fn description(&self) -> String {
        "Paint stroke".to_string()
    }
}

fn get_keyframe_mut<'a>(
    document: &'a mut Document,
    layer_id: &Uuid,
    time: f64,
    width: u32,
    height: u32,
) -> Result<&'a mut crate::raster_layer::RasterKeyframe, String> {
    let layer = document
        .get_layer_mut(layer_id)
        .ok_or_else(|| format!("Layer {} not found", layer_id))?;
    let raster = match layer {
        AnyLayer::Raster(rl) => rl,
        _ => return Err("Not a raster layer".to_string()),
    };
    Ok(raster.ensure_keyframe_at(time, width, height))
}
