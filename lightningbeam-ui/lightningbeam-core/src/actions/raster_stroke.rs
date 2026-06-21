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
use crate::actions::raster_diff::RasterDiff;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that records a single brush stroke for undo/redo.
///
/// The stroke must already be painted into the document's `raw_pixels` before
/// this action is executed for the first time. Only the changed bounding box is
/// retained (see [`RasterDiff`]) rather than two full frame buffers.
pub struct RasterStrokeAction {
    layer_id: Uuid,
    time: f64,
    width: u32,
    height: u32,
    diff: RasterDiff,
    /// The full post-stroke buffer, kept ONLY for the first `execute` (the commit),
    /// which establishes `raw_pixels` exactly like the old code did — robust no matter
    /// what state the working buffer is in (empty new keyframe, GPU-canvas readback,
    /// etc.). Taken (dropped) on first execute, so the action sitting in the undo stack
    /// retains only the small `diff`; redo then replays via the diff.
    full_after: Option<Vec<u8>>,
}

impl RasterStrokeAction {
    /// Create the action.
    ///
    /// * `buffer_before` – raw RGBA pixels captured just before the stroke began.
    /// * `buffer_after`  – raw RGBA pixels captured just after the stroke finished.
    ///
    /// The full buffers are diffed down to the changed bbox here and then dropped.
    pub fn new(
        layer_id: Uuid,
        time: f64,
        buffer_before: Vec<u8>,
        buffer_after: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Self {
        let diff = RasterDiff::compute(&buffer_before, &buffer_after, width, height);
        Self { layer_id, time, width, height, diff, full_after: Some(buffer_after) }
    }
}

impl Action for RasterStrokeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let kf = get_keyframe_mut(document, &self.layer_id, self.time, self.width, self.height)?;
        if let Some(full) = self.full_after.take() {
            // First execute (commit): assign the full buffer outright.
            kf.raw_pixels = full;
        } else {
            // Redo: replay via the diff onto the (resident) base.
            self.diff.apply_after(&mut kf.raw_pixels);
        }
        kf.texture_dirty = true;
        kf.dirty = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let kf = get_keyframe_mut(document, &self.layer_id, self.time, self.width, self.height)?;
        self.diff.apply_before(&mut kf.raw_pixels);
        kf.texture_dirty = true;
        kf.dirty = true;
        Ok(())
    }

    fn description(&self) -> String {
        "Paint stroke".to_string()
    }

    fn raster_resident_hint(&self) -> Option<(Uuid, f64)> {
        Some((self.layer_id, self.time))
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
