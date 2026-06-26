//! Resize every keyframe of a raster layer to a new canvas size (Scale or Canvas mode), with undo.
//!
//! Used by the info panel's "Layer to document size" button. Pixels must be resident (the editor
//! faults them in first) so the resample/copy is exact and a later page-in won't mismatch the size.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::raster_layer::RasterResizeMode;
use crate::raster_store::RasterStore;
use uuid::Uuid;

/// Per-keyframe state captured for undo.
struct OldKeyframe {
    id: Uuid,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

pub struct ResizeRasterLayerAction {
    layer_id: Uuid,
    new_w: u32,
    new_h: u32,
    mode: RasterResizeMode,
    /// Read-only page-in for keyframes whose pixels aren't resident. (No incremental write exists, so
    /// resized keyframes stay resident + dirty and persist on the next full save.)
    store: RasterStore,
    /// Captured on first execute for rollback.
    old: Option<Vec<OldKeyframe>>,
}

impl ResizeRasterLayerAction {
    pub fn new(layer_id: Uuid, new_w: u32, new_h: u32, mode: RasterResizeMode, store: RasterStore) -> Self {
        Self { layer_id, new_w, new_h, mode, store, old: None }
    }
}

impl Action for ResizeRasterLayerAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&self.layer_id) else {
            return Err("ResizeRasterLayerAction: layer is not a raster layer".into());
        };
        let capture = self.old.is_none();
        let mut old = Vec::new();
        for kf in rl.keyframes.iter_mut() {
            // Page the keyframe in one at a time so we never hold the whole layer in memory at once.
            if kf.raw_pixels.is_empty() {
                if let Some(px) = self.store.load_pixels(kf.id) {
                    kf.raw_pixels = px;
                    kf.needs_fault_in = false;
                }
            }
            if capture {
                old.push(OldKeyframe { id: kf.id, width: kf.width, height: kf.height, pixels: kf.raw_pixels.clone() });
            }
            kf.resize_to(self.new_w, self.new_h, self.mode);
        }
        if capture {
            self.old = Some(old);
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&self.layer_id) else {
            return Err("ResizeRasterLayerAction: layer is not a raster layer".into());
        };
        if let Some(old) = &self.old {
            for o in old {
                if let Some(kf) = rl.keyframes.iter_mut().find(|kf| kf.id == o.id) {
                    kf.width = o.width;
                    kf.height = o.height;
                    kf.raw_pixels = o.pixels.clone();
                    kf.proxy = None;
                    kf.texture_dirty = true;
                    kf.dirty = true;
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        match self.mode {
            RasterResizeMode::Scale => "Scale raster layer".into(),
            RasterResizeMode::Canvas => "Resize raster canvas".into(),
        }
    }
}
