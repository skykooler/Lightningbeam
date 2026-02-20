//! Clipboard management for cut/copy/paste operations
//!
//! Supports multiple content types (clip instances, shapes) with
//! cross-platform clipboard integration via arboard.

use crate::clip::{AudioClip, ClipInstance, ImageAsset, VectorClip, VideoClip};
use crate::layer::{AudioLayerType, AnyLayer};
use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Layer type tag for clipboard, so paste knows where clips can go
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ClipboardLayerType {
    Vector,
    Video,
    AudioSampled,
    AudioMidi,
    Effect,
}

impl ClipboardLayerType {
    /// Determine the clipboard layer type from a document layer
    pub fn from_layer(layer: &AnyLayer) -> Self {
        match layer {
            AnyLayer::Vector(_) => ClipboardLayerType::Vector,
            AnyLayer::Video(_) => ClipboardLayerType::Video,
            AnyLayer::Audio(al) => match al.audio_layer_type {
                AudioLayerType::Sampled => ClipboardLayerType::AudioSampled,
                AudioLayerType::Midi => ClipboardLayerType::AudioMidi,
            },
            AnyLayer::Effect(_) => ClipboardLayerType::Effect,
        }
    }

    /// Check if a layer is compatible with this clipboard layer type
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

/// Content stored in the clipboard
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClipboardContent {
    /// Clip instances with their referenced clip definitions
    ClipInstances {
        /// Source layer type (for paste compatibility)
        layer_type: ClipboardLayerType,
        /// The clip instances (IDs will be regenerated on paste)
        instances: Vec<ClipInstance>,
        /// Referenced audio clip definitions
        audio_clips: Vec<(Uuid, AudioClip)>,
        /// Referenced video clip definitions
        video_clips: Vec<(Uuid, VideoClip)>,
        /// Referenced vector clip definitions
        vector_clips: Vec<(Uuid, VectorClip)>,
        /// Referenced image assets
        image_assets: Vec<(Uuid, ImageAsset)>,
    },
    /// Shapes from a vector layer's keyframe
    Shapes {
        /// Shapes (with embedded transforms)
        shapes: Vec<Shape>,
    },
}

impl ClipboardContent {
    /// Create a clone of this content with all UUIDs regenerated
    /// Returns the new content and a mapping from old to new IDs
    pub fn with_regenerated_ids(&self) -> (Self, HashMap<Uuid, Uuid>) {
        let mut id_map = HashMap::new();

        match self {
            ClipboardContent::ClipInstances {
                layer_type,
                instances,
                audio_clips,
                video_clips,
                vector_clips,
                image_assets,
            } => {
                // Regenerate clip definition IDs
                let new_audio_clips: Vec<(Uuid, AudioClip)> = audio_clips
                    .iter()
                    .map(|(old_id, clip)| {
                        let new_id = Uuid::new_v4();
                        id_map.insert(*old_id, new_id);
                        let mut new_clip = clip.clone();
                        new_clip.id = new_id;
                        (new_id, new_clip)
                    })
                    .collect();

                let new_video_clips: Vec<(Uuid, VideoClip)> = video_clips
                    .iter()
                    .map(|(old_id, clip)| {
                        let new_id = Uuid::new_v4();
                        id_map.insert(*old_id, new_id);
                        let mut new_clip = clip.clone();
                        new_clip.id = new_id;
                        (new_id, new_clip)
                    })
                    .collect();

                let new_vector_clips: Vec<(Uuid, VectorClip)> = vector_clips
                    .iter()
                    .map(|(old_id, clip)| {
                        let new_id = Uuid::new_v4();
                        id_map.insert(*old_id, new_id);
                        let mut new_clip = clip.clone();
                        new_clip.id = new_id;
                        (new_id, new_clip)
                    })
                    .collect();

                let new_image_assets: Vec<(Uuid, ImageAsset)> = image_assets
                    .iter()
                    .map(|(old_id, asset)| {
                        let new_id = Uuid::new_v4();
                        id_map.insert(*old_id, new_id);
                        let mut new_asset = asset.clone();
                        new_asset.id = new_id;
                        (new_id, new_asset)
                    })
                    .collect();

                // Regenerate clip instance IDs and remap clip_id references
                let new_instances: Vec<ClipInstance> = instances
                    .iter()
                    .map(|inst| {
                        let new_instance_id = Uuid::new_v4();
                        id_map.insert(inst.id, new_instance_id);
                        let mut new_inst = inst.clone();
                        new_inst.id = new_instance_id;
                        // Remap clip_id to new definition ID
                        if let Some(new_clip_id) = id_map.get(&inst.clip_id) {
                            new_inst.clip_id = *new_clip_id;
                        }
                        new_inst
                    })
                    .collect();

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
            ClipboardContent::Shapes { shapes } => {
                // Regenerate shape IDs
                let new_shapes: Vec<Shape> = shapes
                    .iter()
                    .map(|shape| {
                        let new_id = Uuid::new_v4();
                        id_map.insert(shape.id, new_id);
                        let mut new_shape = shape.clone();
                        new_shape.id = new_id;
                        new_shape
                    })
                    .collect();

                (
                    ClipboardContent::Shapes {
                        shapes: new_shapes,
                    },
                    id_map,
                )
            }
        }
    }
}

/// JSON prefix for clipboard text to identify Lightningbeam content
const CLIPBOARD_PREFIX: &str = "LIGHTNINGBEAM_CLIPBOARD:";

/// Manages clipboard operations with internal + system clipboard
pub struct ClipboardManager {
    /// Internal clipboard (preserves rich data without serialization loss)
    internal: Option<ClipboardContent>,
    /// System clipboard handle (lazy-initialized)
    system: Option<arboard::Clipboard>,
}

impl ClipboardManager {
    /// Create a new clipboard manager
    pub fn new() -> Self {
        let system = arboard::Clipboard::new().ok();
        Self {
            internal: None,
            system,
        }
    }

    /// Copy content to both internal and system clipboard
    pub fn copy(&mut self, content: ClipboardContent) {
        // Serialize to system clipboard as JSON text
        if let Some(system) = self.system.as_mut() {
            if let Ok(json) = serde_json::to_string(&content) {
                let clipboard_text = format!("{}{}", CLIPBOARD_PREFIX, json);
                let _ = system.set_text(clipboard_text);
            }
        }

        // Store internally for rich access
        self.internal = Some(content);
    }

    /// Try to paste content
    /// Returns internal clipboard if available, falls back to system clipboard JSON
    pub fn paste(&mut self) -> Option<ClipboardContent> {
        // Try internal clipboard first
        if let Some(content) = &self.internal {
            return Some(content.clone());
        }

        // Fall back to system clipboard
        if let Some(system) = self.system.as_mut() {
            if let Ok(text) = system.get_text() {
                if let Some(json) = text.strip_prefix(CLIPBOARD_PREFIX) {
                    if let Ok(content) = serde_json::from_str::<ClipboardContent>(json) {
                        return Some(content);
                    }
                }
            }
        }

        None
    }

    /// Check if there's content available to paste
    pub fn has_content(&mut self) -> bool {
        if self.internal.is_some() {
            return true;
        }

        if let Some(system) = self.system.as_mut() {
            if let Ok(text) = system.get_text() {
                return text.starts_with(CLIPBOARD_PREFIX);
            }
        }

        false
    }
}
