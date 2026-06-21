//! On-demand loader for raster keyframe pixels backed by the project `.beam`
//! container (Phase 3 paging).
//!
//! Raster keyframes are no longer eagerly decoded at load; `raw_pixels` stays
//! empty until something needs the frame, then it is faulted in from the
//! container's `Raster` media row (keyed by the keyframe id). The store holds only
//! the container path and reads through a fresh **read-only** connection per call,
//! so it never conflicts with an in-place save and keeps no long-lived handle.
//! `None` path = an unsaved document (nothing to fault in).

use std::path::PathBuf;
use uuid::Uuid;

/// Faults in raster keyframe pixels from the project container on demand.
#[derive(Default, Clone)]
pub struct RasterStore {
    path: Option<PathBuf>,
}

impl RasterStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// Point the store at a (possibly new) container path, or `None` for an
    /// unsaved document. Call on load and on save-as.
    pub fn set_path(&mut self, path: Option<PathBuf>) {
        self.path = path;
    }

    pub fn has_path(&self) -> bool {
        self.path.is_some()
    }

    /// Decode the keyframe's full RGBA pixels from the container, or `None` if the
    /// container has no row for it (or decoding fails). The returned buffer is the
    /// working `raw_pixels` representation (`width*height*4` sRGB-premultiplied RGBA).
    pub fn load_pixels(&self, kf_id: Uuid) -> Option<Vec<u8>> {
        let path = self.path.as_ref()?;
        let png = match crate::beam_archive::read_packed_media_readonly(path, kf_id) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(e) => {
                eprintln!("[RasterStore] read {} failed: {}", kf_id, e);
                return None;
            }
        };
        match crate::brush_engine::decode_png(&png) {
            Ok(img) => Some(img.into_raw()),
            Err(e) => {
                eprintln!("[RasterStore] decode {} failed: {}", kf_id, e);
                None
            }
        }
    }
}
