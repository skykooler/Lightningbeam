//! Clipboard management for cut/copy/paste operations.
//!
//! # Content types
//! [`ClipboardContent`] covers every selectable item in the app:
//! - [`ClipInstances`](ClipboardContent::ClipInstances) — timeline clips
//! - [`VectorGeometry`](ClipboardContent::VectorGeometry) — DCEL shapes (stub; Phase 2)
//! - [`MidiNotes`](ClipboardContent::MidiNotes) — piano-roll notes
//! - [`RasterPixels`](ClipboardContent::RasterPixels) — raster selection
//! - [`Layers`](ClipboardContent::Layers) — complete layer subtrees
//! - [`AudioNodes`](ClipboardContent::AudioNodes) — audio-graph node subgraph
//!
//! # Storage strategy
//! Content is kept in three places simultaneously:
//! 1. **Internal** (`self.internal`) — in-process, zero-copy, always preferred.
//! 2. **Platform custom type** (`application/x-lightningbeam`) via
//!    [`crate::clipboard_platform`] — enables cross-process paste between LB windows.
//! 3. **arboard text fallback** — `LIGHTNINGBEAM_CLIPBOARD:<json>` in the system
//!    text clipboard for maximum compatibility (e.g. terminals, remote desktops).
//!
//! For `RasterPixels` an additional `image/png` entry is set on macOS and Windows
//! so the image can be pasted into external apps.
//!
//! # Temporary note
//! The custom-MIME platform layer ([`crate::clipboard_platform`]) is a shim until
//! arboard supports custom MIME types natively
//! (<https://github.com/1Password/arboard/issues/14>).  When that lands, remove
//! `clipboard_platform`, the `objc2*` and `windows-sys` Cargo deps, and call
//! arboard directly.

use crate::clip::{AudioClip, ClipInstance, ImageAsset, VectorClip, VideoClip};
use crate::layer::{AudioLayerType, AnyLayer};
use crate::clipboard_platform;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// MIME type used for cross-process Lightningbeam clipboard data.
pub const LIGHTNINGBEAM_MIME: &str = "application/x-lightningbeam";

// ─────────────────────────────── Layer type tag ─────────────────────────────

/// Layer type tag for clipboard — tells paste where clip instances can go.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClipboardLayerType {
    Vector,
    Video,
    AudioSampled,
    AudioMidi,
    Effect,
}

impl ClipboardLayerType {
    /// Determine the clipboard layer type from a document layer.
    pub fn from_layer(layer: &AnyLayer) -> Self {
        match layer {
            AnyLayer::Vector(_) => ClipboardLayerType::Vector,
            AnyLayer::Video(_) => ClipboardLayerType::Video,
            AnyLayer::Audio(al) => match al.audio_layer_type {
                AudioLayerType::Sampled => ClipboardLayerType::AudioSampled,
                AudioLayerType::Midi => ClipboardLayerType::AudioMidi,
            },
            AnyLayer::Effect(_) => ClipboardLayerType::Effect,
            AnyLayer::Group(_) => ClipboardLayerType::Vector,
            AnyLayer::Raster(_) => ClipboardLayerType::Vector,
        }
    }

    /// Check if a layer is compatible with this clipboard layer type.
    pub fn is_compatible(&self, layer: &AnyLayer) -> bool {
        match (self, layer) {
            (ClipboardLayerType::Vector, AnyLayer::Vector(_)) => true,
            (ClipboardLayerType::Video, AnyLayer::Video(_)) => true,
            (ClipboardLayerType::AudioSampled, AnyLayer::Audio(al)) => {
                al.audio_layer_type == AudioLayerType::Sampled
            }
            (ClipboardLayerType::AudioMidi, AnyLayer::Audio(al)) => {
                al.audio_layer_type == AudioLayerType::Midi
            }
            (ClipboardLayerType::Effect, AnyLayer::Effect(_)) => true,
            _ => false,
        }
    }
}

// ──────────────────────────── Shared clip bundle ─────────────────────────────

/// Clip definitions referenced by clipboard content.
///
/// Shared between [`ClipboardContent::ClipInstances`] and [`ClipboardContent::Layers`].
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ReferencedClips {
    pub audio_clips: Vec<(Uuid, AudioClip)>,
    pub video_clips: Vec<(Uuid, VideoClip)>,
    pub vector_clips: Vec<(Uuid, VectorClip)>,
    pub image_assets: Vec<(Uuid, ImageAsset)>,
}

// ───────────────────────────── Clipboard content ─────────────────────────────

/// Content stored in the clipboard.
///
/// The `serde(tag = "type")` discriminant is stable — unknown variants
/// deserialize as `None`, so new variants can be added without breaking
/// existing serialized data.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClipboardContent {
    /// Timeline clip instances with all referenced clip definitions.
    ClipInstances {
        /// Source layer type (for paste compatibility).
        layer_type: ClipboardLayerType,
        /// The clip instances (IDs regenerated on paste).
        instances: Vec<ClipInstance>,
        /// Referenced audio clip definitions.
        audio_clips: Vec<(Uuid, AudioClip)>,
        /// Referenced video clip definitions.
        video_clips: Vec<(Uuid, VideoClip)>,
        /// Referenced vector clip definitions.
        vector_clips: Vec<(Uuid, VectorClip)>,
        /// Referenced image assets.
        image_assets: Vec<(Uuid, ImageAsset)>,
    },

    /// Selected DCEL geometry from a vector layer.
    ///
    /// Currently a stub — `data` is opaque bytes whose format is TBD in Phase 2
    /// once DCEL serialization is implemented.  Copy/paste of vector shapes does
    /// nothing until then.  Secondary formats (`image/svg+xml`, `image/png`) are
    /// also deferred to Phase 2.
    VectorGeometry {
        /// Opaque DCEL subgraph bytes (format TBD, Phase 2).
        data: Vec<u8>,
    },

    /// MIDI notes from the piano roll.
    MidiNotes {
        /// `(start_time, note, velocity, duration)` — times relative to selection start.
        notes: Vec<(f64, u8, u8, f64)>,
    },

    /// Raw pixel region from a raster layer selection.
    ///
    /// Pixels are sRGB-encoded premultiplied RGBA, `width × height × 4` bytes —
    /// the same in-memory format as `RasterKeyframe::raw_pixels`.
    ///
    /// On macOS and Windows an `image/png` secondary format is also set so the
    /// content can be pasted into external apps.
    RasterPixels {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    },

    /// One or more complete layers copied from the timeline.
    ///
    /// [`AnyLayer`] derives `Serialize`/`Deserialize`; only
    /// `RasterKeyframe::raw_pixels` is excluded from serde (`#[serde(skip)]`) and
    /// is therefore carried separately in `raster_pixels`.
    ///
    /// On paste: insert as sibling layers at the current selection point with all
    /// UUIDs regenerated.
    Layers {
        /// Complete serialized layer trees (raw_pixels absent).
        layers: Vec<AnyLayer>,
        /// Raster pixel data keyed by `(layer_id, time.to_bits())`.
        /// Restored into `RasterKeyframe::raw_pixels` after deserialization by
        /// matching layer_id + time_bits against the deserialized keyframes.
        raster_pixels: Vec<(Uuid, u64, Vec<u8>)>,
        /// All clip definitions referenced by any of the copied layers.
        referenced_clips: ReferencedClips,
    },

    /// Selected nodes and edges from an audio effect/synthesis graph.
    ///
    /// Uses the same serialization types as preset save/load
    /// (`daw_backend::audio::node_graph::preset`).
    ///
    /// On paste: add nodes to the target layer's graph with new IDs, then sync
    /// to the DAW backend (same pattern as `ClipInstances`).
    AudioNodes {
        /// Selected nodes.
        nodes: Vec<daw_backend::SerializedNode>,
        /// Connections between the selected nodes only.
        connections: Vec<daw_backend::SerializedConnection>,
        /// Source layer UUID — hint for paste target validation.
        source_layer_id: Uuid,
    },
}

// ──────────────────────── ID regeneration ───────────────────────────────────

impl ClipboardContent {
    /// Clone this content with all UUIDs regenerated.
    ///
    /// Returns the new content and a mapping from old → new IDs.
    pub fn with_regenerated_ids(&self) -> (Self, HashMap<Uuid, Uuid>) {
        let mut id_map: HashMap<Uuid, Uuid> = HashMap::new();

        match self {
            // ── ClipInstances ───────────────────────────────────────────────
            ClipboardContent::ClipInstances {
                layer_type,
                instances,
                audio_clips,
                video_clips,
                vector_clips,
                image_assets,
            } => {
                let new_audio_clips = regen_audio_clips(audio_clips, &mut id_map);
                let new_video_clips = regen_video_clips(video_clips, &mut id_map);
                let new_vector_clips = regen_vector_clips(vector_clips, &mut id_map);
                let new_image_assets = regen_image_assets(image_assets, &mut id_map);
                let new_instances = regen_clip_instances(instances, &mut id_map);
                (
                    ClipboardContent::ClipInstances {
                        layer_type: layer_type.clone(),
                        instances: new_instances,
                        audio_clips: new_audio_clips,
                        video_clips: new_video_clips,
                        vector_clips: new_vector_clips,
                        image_assets: new_image_assets,
                    },
                    id_map,
                )
            }

            // ── VectorGeometry ──────────────────────────────────────────────
            // TODO (Phase 2): remap DCEL vertex/edge UUIDs once DCEL serialization
            // is defined.
            ClipboardContent::VectorGeometry { data } => {
                (ClipboardContent::VectorGeometry { data: data.clone() }, id_map)
            }

            // ── MidiNotes ───────────────────────────────────────────────────
            ClipboardContent::MidiNotes { notes } => {
                (ClipboardContent::MidiNotes { notes: notes.clone() }, id_map)
            }

            // ── RasterPixels ────────────────────────────────────────────────
            ClipboardContent::RasterPixels { pixels, width, height } => (
                ClipboardContent::RasterPixels {
                    pixels: pixels.clone(),
                    width: *width,
                    height: *height,
                },
                id_map,
            ),

            // ── Layers ──────────────────────────────────────────────────────
            ClipboardContent::Layers { layers, raster_pixels, referenced_clips } => {
                let new_clips = regen_referenced_clips(referenced_clips, &mut id_map);
                let new_layers: Vec<AnyLayer> = layers
                    .iter()
                    .map(|l| regen_any_layer(l, &mut id_map))
                    .collect();
                // Remap raster_pixels layer_id keys.
                let new_raster: Vec<(Uuid, u64, Vec<u8>)> = raster_pixels
                    .iter()
                    .map(|(old_lid, time_bits, px)| {
                        let new_lid = id_map.get(old_lid).copied().unwrap_or(*old_lid);
                        (new_lid, *time_bits, px.clone())
                    })
                    .collect();
                (
                    ClipboardContent::Layers {
                        layers: new_layers,
                        raster_pixels: new_raster,
                        referenced_clips: new_clips,
                    },
                    id_map,
                )
            }

            // ── AudioNodes ──────────────────────────────────────────────────
            ClipboardContent::AudioNodes { nodes, connections, source_layer_id } => {
                // Remap u32 node IDs.
                let mut node_id_map: HashMap<u32, u32> = HashMap::new();
                let new_nodes: Vec<daw_backend::SerializedNode> = nodes
                    .iter()
                    .map(|n| {
                        let new_id = node_id_map.len() as u32 + 1;
                        node_id_map.insert(n.id, new_id);
                        let mut nn = n.clone();
                        nn.id = new_id;
                        nn
                    })
                    .collect();
                let new_connections: Vec<daw_backend::SerializedConnection> = connections
                    .iter()
                    .map(|c| {
                        let mut nc = c.clone();
                        nc.from_node = node_id_map.get(&c.from_node).copied().unwrap_or(c.from_node);
                        nc.to_node = node_id_map.get(&c.to_node).copied().unwrap_or(c.to_node);
                        nc
                    })
                    .collect();
                (
                    ClipboardContent::AudioNodes {
                        nodes: new_nodes,
                        connections: new_connections,
                        source_layer_id: *source_layer_id,
                    },
                    id_map,
                )
            }
        }
    }
}

// ──────────────────────── ID regeneration helpers ───────────────────────────

fn regen_audio_clips(
    clips: &[(Uuid, AudioClip)],
    id_map: &mut HashMap<Uuid, Uuid>,
) -> Vec<(Uuid, AudioClip)> {
    clips
        .iter()
        .map(|(old_id, clip)| {
            let new_id = Uuid::new_v4();
            id_map.insert(*old_id, new_id);
            let mut c = clip.clone();
            c.id = new_id;
            (new_id, c)
        })
        .collect()
}

fn regen_video_clips(
    clips: &[(Uuid, crate::clip::VideoClip)],
    id_map: &mut HashMap<Uuid, Uuid>,
) -> Vec<(Uuid, crate::clip::VideoClip)> {
    clips
        .iter()
        .map(|(old_id, clip)| {
            let new_id = Uuid::new_v4();
            id_map.insert(*old_id, new_id);
            let mut c = clip.clone();
            c.id = new_id;
            (new_id, c)
        })
        .collect()
}

fn regen_vector_clips(
    clips: &[(Uuid, VectorClip)],
    id_map: &mut HashMap<Uuid, Uuid>,
) -> Vec<(Uuid, VectorClip)> {
    clips
        .iter()
        .map(|(old_id, clip)| {
            let new_id = Uuid::new_v4();
            id_map.insert(*old_id, new_id);
            let mut c = clip.clone();
            c.id = new_id;
            (new_id, c)
        })
        .collect()
}

fn regen_image_assets(
    assets: &[(Uuid, ImageAsset)],
    id_map: &mut HashMap<Uuid, Uuid>,
) -> Vec<(Uuid, ImageAsset)> {
    assets
        .iter()
        .map(|(old_id, asset)| {
            let new_id = Uuid::new_v4();
            id_map.insert(*old_id, new_id);
            let mut a = asset.clone();
            a.id = new_id;
            (new_id, a)
        })
        .collect()
}

fn regen_clip_instances(
    instances: &[ClipInstance],
    id_map: &mut HashMap<Uuid, Uuid>,
) -> Vec<ClipInstance> {
    instances
        .iter()
        .map(|inst| {
            let new_id = Uuid::new_v4();
            id_map.insert(inst.id, new_id);
            let mut i = inst.clone();
            i.id = new_id;
            if let Some(new_clip_id) = id_map.get(&inst.clip_id) {
                i.clip_id = *new_clip_id;
            }
            i
        })
        .collect()
}

fn regen_referenced_clips(
    rc: &ReferencedClips,
    id_map: &mut HashMap<Uuid, Uuid>,
) -> ReferencedClips {
    ReferencedClips {
        audio_clips: regen_audio_clips(&rc.audio_clips, id_map),
        video_clips: regen_video_clips(&rc.video_clips, id_map),
        vector_clips: regen_vector_clips(&rc.vector_clips, id_map),
        image_assets: regen_image_assets(&rc.image_assets, id_map),
    }
}

/// Regenerate the layer's own ID (and all descendant IDs for group layers).
fn regen_any_layer(layer: &AnyLayer, id_map: &mut HashMap<Uuid, Uuid>) -> AnyLayer {
    match layer {
        AnyLayer::Vector(vl) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(vl.layer.id, new_layer_id);
            let mut nl = vl.clone();
            nl.layer.id = new_layer_id;
            nl.clip_instances = regen_clip_instances(&vl.clip_instances, id_map);
            AnyLayer::Vector(nl)
        }
        AnyLayer::Audio(al) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(al.layer.id, new_layer_id);
            let mut nl = al.clone();
            nl.layer.id = new_layer_id;
            nl.clip_instances = regen_clip_instances(&al.clip_instances, id_map);
            AnyLayer::Audio(nl)
        }
        AnyLayer::Video(vl) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(vl.layer.id, new_layer_id);
            let mut nl = vl.clone();
            nl.layer.id = new_layer_id;
            nl.clip_instances = regen_clip_instances(&vl.clip_instances, id_map);
            AnyLayer::Video(nl)
        }
        AnyLayer::Effect(el) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(el.layer.id, new_layer_id);
            let mut nl = el.clone();
            nl.layer.id = new_layer_id;
            nl.clip_instances = regen_clip_instances(&el.clip_instances, id_map);
            AnyLayer::Effect(nl)
        }
        AnyLayer::Raster(rl) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(rl.layer.id, new_layer_id);
            let mut nl = rl.clone();
            nl.layer.id = new_layer_id;
            AnyLayer::Raster(nl)
        }
        AnyLayer::Group(gl) => {
            let new_layer_id = Uuid::new_v4();
            id_map.insert(gl.layer.id, new_layer_id);
            let mut nl = gl.clone();
            nl.layer.id = new_layer_id;
            nl.children = gl.children.iter().map(|c| regen_any_layer(c, id_map)).collect();
            AnyLayer::Group(nl)
        }
    }
}

// ──────────────────────── Pixel format conversion helpers ────────────────────

/// Convert straight-alpha RGBA bytes to premultiplied RGBA.
fn straight_to_premul(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3];
            if a == 0 {
                [0u8, 0, 0, 0]
            } else {
                let scale = a as f32 / 255.0;
                [
                    (p[0] as f32 * scale).round() as u8,
                    (p[1] as f32 * scale).round() as u8,
                    (p[2] as f32 * scale).round() as u8,
                    a,
                ]
            }
        })
        .collect()
}

/// Convert premultiplied RGBA bytes to straight-alpha RGBA.
fn premul_to_straight(bytes: &[u8]) -> Vec<u8> {
    bytes
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3];
            if a == 0 {
                [0u8, 0, 0, 0]
            } else {
                let inv = 255.0 / a as f32;
                [
                    (p[0] as f32 * inv).round().min(255.0) as u8,
                    (p[1] as f32 * inv).round().min(255.0) as u8,
                    (p[2] as f32 * inv).round().min(255.0) as u8,
                    a,
                ]
            }
        })
        .collect()
}

// ──────────────────────────── PNG encoding helper ────────────────────────────

/// Encode sRGB premultiplied RGBA pixels as PNG bytes.
///
/// Returns `None` on encoding failure (logged to stderr).
pub(crate) fn encode_raster_as_png(pixels: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    use image::RgbaImage;
    let img = RgbaImage::from_raw(width, height, premul_to_straight(pixels))?;
    match crate::brush_engine::encode_png(&img) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            eprintln!("clipboard: PNG encode failed: {e}");
            None
        }
    }
}

// ───────────────────────────── ClipboardManager ─────────────────────────────

/// Manages clipboard operations with internal + system clipboard.
pub struct ClipboardManager {
    /// Internal clipboard (preserves rich data without serialization loss).
    internal: Option<ClipboardContent>,
    /// System clipboard handle (lazy-initialized).
    system: Option<arboard::Clipboard>,
}

impl ClipboardManager {
    /// Create a new clipboard manager.
    pub fn new() -> Self {
        let system = arboard::Clipboard::new().ok();
        Self { internal: None, system }
    }

    /// Copy content to the internal clipboard, the platform custom-MIME clipboard,
    /// and the arboard text-fallback clipboard.
    pub fn copy(&mut self, content: ClipboardContent) {
        let json = serde_json::to_string(&content).unwrap_or_default();

        // Build platform entries (custom MIME always present; PNG secondary for raster).
        let mut entries: Vec<(&str, Vec<u8>)> =
            vec![(LIGHTNINGBEAM_MIME, json.as_bytes().to_vec())];
        if let ClipboardContent::RasterPixels { pixels, width, height } = &content {
            if let Some(png) = encode_raster_as_png(pixels, *width, *height) {
                entries.push(("image/png", png));
            }
        }

        clipboard_platform::set(
            &entries.iter().map(|(m, d)| (*m, d.as_slice())).collect::<Vec<_>>(),
        );

        self.internal = Some(content);
    }

    /// Try to paste content.
    ///
    /// Checks the platform custom MIME type first.  If our content is still on
    /// the clipboard the internal cache is returned (avoids re-deserializing).
    /// If another app has taken the clipboard since we last copied, the internal
    /// cache is cleared and `None` is returned so the caller can try other
    /// sources (e.g. `try_get_raster_image`).
    pub fn paste(&mut self) -> Option<ClipboardContent> {
        match clipboard_platform::get(&[LIGHTNINGBEAM_MIME]) {
            Some((_, data)) => {
                // Our MIME type is still on the clipboard — prefer the internal
                // cache to avoid a round-trip through JSON.
                if let Some(content) = &self.internal {
                    return Some(content.clone());
                }
                // Cross-process paste (internal cache absent): deserialize.
                if let Ok(s) = std::str::from_utf8(&data) {
                    if let Ok(content) = serde_json::from_str::<ClipboardContent>(s) {
                        return Some(content);
                    }
                }
                None
            }
            None => {
                // Another app owns the clipboard — internal cache is stale.
                self.internal = None;
                None
            }
        }
    }

    /// Copy raster pixels to the system clipboard as an image.
    ///
    /// `pixels` must be sRGB-encoded premultiplied RGBA (`w × h × 4` bytes).
    /// Converts to straight-alpha RGBA8 for arboard.  Silently ignores errors.
    pub fn try_set_raster_image(&mut self, pixels: &[u8], width: u32, height: u32) {
        let Some(system) = self.system.as_mut() else { return };
        let straight = premul_to_straight(pixels);
        let img = arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(straight),
        };
        let _ = system.set_image(img);
    }

    /// Try to read an image from the system clipboard.
    ///
    /// Returns sRGB-encoded premultiplied RGBA pixels on success, or `None` if
    /// no image is available.  Silently ignores errors.
    pub fn try_get_raster_image(&mut self) -> Option<(Vec<u8>, u32, u32)> {
        // On Linux arboard's get_image() does not reliably read clipboard images
        // set by other apps on Wayland.  Use clipboard_platform (wl-clipboard-rs /
        // x11-clipboard) to read the raw image bytes then decode with the image crate.
        #[cfg(target_os = "linux")]
        {
            let (_, data) = clipboard_platform::get(&[
                "image/png",
                "image/jpeg",
                "image/bmp",
                "image/tiff",
            ])?;
            let img = image::load_from_memory(&data).ok()?.into_rgba8();
            let (width, height) = img.dimensions();
            let premul = straight_to_premul(img.as_raw());
            return Some((premul, width, height));
        }

        // macOS / Windows: arboard handles image clipboard natively.
        #[cfg(not(target_os = "linux"))]
        {
            let img = self.system.as_mut()?.get_image().ok()?;
            let premul = straight_to_premul(&img.bytes);
            Some((premul, img.width as u32, img.height as u32))
        }
    }

    /// Check if there is content available to paste.
    pub fn has_content(&self) -> bool {
        self.internal.is_some()
    }
}
