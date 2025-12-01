//! Document structure for Lightningbeam
//!
//! The Document represents a complete animation project with settings
//! and a root graphics object containing the scene graph.

use crate::clip::{AudioClip, ImageAsset, VideoClip, VectorClip};
use crate::layer::AnyLayer;
use crate::layout::LayoutNode;
use crate::shape::ShapeColor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Root graphics object containing all layers in the scene
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphicsObject {
    /// Unique identifier
    pub id: Uuid,

    /// Name of this graphics object
    pub name: String,

    /// Child layers
    pub children: Vec<AnyLayer>,
}

impl GraphicsObject {
    /// Create a new graphics object
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            children: Vec::new(),
        }
    }

    /// Add a layer as a child
    pub fn add_child(&mut self, layer: AnyLayer) -> Uuid {
        let id = layer.id();
        self.children.push(layer);
        id
    }

    /// Get a child layer by ID
    pub fn get_child(&self, id: &Uuid) -> Option<&AnyLayer> {
        self.children.iter().find(|l| &l.id() == id)
    }

    /// Get a mutable child layer by ID
    pub fn get_child_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        self.children.iter_mut().find(|l| &l.id() == id)
    }

    /// Remove a child layer by ID
    pub fn remove_child(&mut self, id: &Uuid) -> Option<AnyLayer> {
        if let Some(index) = self.children.iter().position(|l| &l.id() == id) {
            Some(self.children.remove(index))
        } else {
            None
        }
    }
}

impl Default for GraphicsObject {
    fn default() -> Self {
        Self::new("Root")
    }
}

/// Document settings and scene
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    /// Unique identifier for this document
    pub id: Uuid,

    /// Document name
    pub name: String,

    /// Background color
    pub background_color: ShapeColor,

    /// Canvas width in pixels
    pub width: f64,

    /// Canvas height in pixels
    pub height: f64,

    /// Framerate (frames per second)
    pub framerate: f64,

    /// Duration in seconds
    pub duration: f64,

    /// Root graphics object containing all layers
    pub root: GraphicsObject,

    /// Clip libraries - reusable clip definitions
    /// VectorClips can be instantiated multiple times with different transforms/timing
    pub vector_clips: HashMap<Uuid, VectorClip>,

    /// Video clip library - references to video files
    pub video_clips: HashMap<Uuid, VideoClip>,

    /// Audio clip library - sampled audio and MIDI clips
    pub audio_clips: HashMap<Uuid, AudioClip>,

    /// Image asset library - static images for fill textures
    pub image_assets: HashMap<Uuid, ImageAsset>,

    /// Current UI layout state (serialized for save/load)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout: Option<LayoutNode>,

    /// Name of base layout this was derived from (for reference only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_layout_base: Option<String>,

    /// Current playback time in seconds
    #[serde(skip)]
    pub current_time: f64,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Untitled".to_string(),
            background_color: ShapeColor::rgb(255, 255, 255), // White background
            width: 1920.0,
            height: 1080.0,
            framerate: 60.0,
            duration: 10.0,
            root: GraphicsObject::default(),
            vector_clips: HashMap::new(),
            video_clips: HashMap::new(),
            audio_clips: HashMap::new(),
            image_assets: HashMap::new(),
            ui_layout: None,
            ui_layout_base: None,
            current_time: 0.0,
        }
    }
}

impl Document {
    /// Create a new document with default settings
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Create a document with custom dimensions
    pub fn with_size(name: impl Into<String>, width: f64, height: f64) -> Self {
        Self {
            name: name.into(),
            width,
            height,
            ..Default::default()
        }
    }

    /// Set the background color
    pub fn with_background(mut self, color: ShapeColor) -> Self {
        self.background_color = color;
        self
    }

    /// Set the framerate
    pub fn with_framerate(mut self, framerate: f64) -> Self {
        self.framerate = framerate;
        self
    }

    /// Set the duration
    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = duration;
        self
    }

    /// Get the aspect ratio
    pub fn aspect_ratio(&self) -> f64 {
        self.width / self.height
    }

    /// Set the current playback time
    pub fn set_time(&mut self, time: f64) {
        self.current_time = time.max(0.0).min(self.duration);
    }

    /// Get visible layers from the root graphics object
    pub fn visible_layers(&self) -> impl Iterator<Item = &AnyLayer> {
        self.root
            .children
            .iter()
            .filter(|layer| layer.layer().visible)
    }

    /// Get a layer by ID
    pub fn get_layer(&self, id: &Uuid) -> Option<&AnyLayer> {
        self.root.get_child(id)
    }

    // === MUTATION METHODS (pub(crate) - only accessible to action module) ===

    /// Get mutable access to the root graphics object
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn root_mut(&mut self) -> &mut GraphicsObject {
        &mut self.root
    }

    /// Get mutable access to a layer by ID
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub fn get_layer_mut(&mut self, id: &Uuid) -> Option<&mut AnyLayer> {
        self.root.get_child_mut(id)
    }

    // === CLIP LIBRARY METHODS ===

    /// Add a vector clip to the library
    pub fn add_vector_clip(&mut self, clip: VectorClip) -> Uuid {
        let id = clip.id;
        self.vector_clips.insert(id, clip);
        id
    }

    /// Add a video clip to the library
    pub fn add_video_clip(&mut self, clip: VideoClip) -> Uuid {
        let id = clip.id;
        self.video_clips.insert(id, clip);
        id
    }

    /// Add an audio clip to the library
    pub fn add_audio_clip(&mut self, clip: AudioClip) -> Uuid {
        let id = clip.id;
        self.audio_clips.insert(id, clip);
        id
    }

    /// Get a vector clip by ID
    pub fn get_vector_clip(&self, id: &Uuid) -> Option<&VectorClip> {
        self.vector_clips.get(id)
    }

    /// Get a video clip by ID
    pub fn get_video_clip(&self, id: &Uuid) -> Option<&VideoClip> {
        self.video_clips.get(id)
    }

    /// Get an audio clip by ID
    pub fn get_audio_clip(&self, id: &Uuid) -> Option<&AudioClip> {
        self.audio_clips.get(id)
    }

    /// Get a mutable vector clip by ID
    pub fn get_vector_clip_mut(&mut self, id: &Uuid) -> Option<&mut VectorClip> {
        self.vector_clips.get_mut(id)
    }

    /// Get a mutable video clip by ID
    pub fn get_video_clip_mut(&mut self, id: &Uuid) -> Option<&mut VideoClip> {
        self.video_clips.get_mut(id)
    }

    /// Get a mutable audio clip by ID
    pub fn get_audio_clip_mut(&mut self, id: &Uuid) -> Option<&mut AudioClip> {
        self.audio_clips.get_mut(id)
    }

    /// Remove a vector clip from the library
    pub fn remove_vector_clip(&mut self, id: &Uuid) -> Option<VectorClip> {
        self.vector_clips.remove(id)
    }

    /// Remove a video clip from the library
    pub fn remove_video_clip(&mut self, id: &Uuid) -> Option<VideoClip> {
        self.video_clips.remove(id)
    }

    /// Remove an audio clip from the library
    pub fn remove_audio_clip(&mut self, id: &Uuid) -> Option<AudioClip> {
        self.audio_clips.remove(id)
    }

    // === IMAGE ASSET METHODS ===

    /// Add an image asset to the library
    pub fn add_image_asset(&mut self, asset: ImageAsset) -> Uuid {
        let id = asset.id;
        self.image_assets.insert(id, asset);
        id
    }

    /// Get an image asset by ID
    pub fn get_image_asset(&self, id: &Uuid) -> Option<&ImageAsset> {
        self.image_assets.get(id)
    }

    /// Get a mutable image asset by ID
    pub fn get_image_asset_mut(&mut self, id: &Uuid) -> Option<&mut ImageAsset> {
        self.image_assets.get_mut(id)
    }

    /// Remove an image asset from the library
    pub fn remove_image_asset(&mut self, id: &Uuid) -> Option<ImageAsset> {
        self.image_assets.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{LayerTrait, VectorLayer};

    #[test]
    fn test_document_creation() {
        let doc = Document::new("Test Project");
        assert_eq!(doc.name, "Test Project");
        assert_eq!(doc.width, 1920.0);
        assert_eq!(doc.height, 1080.0);
        assert_eq!(doc.root.children.len(), 0);
    }

    #[test]
    fn test_graphics_object() {
        let mut root = GraphicsObject::new("Root");
        let vector_layer = VectorLayer::new("Layer 1");
        let layer_id = root.add_child(AnyLayer::Vector(vector_layer));

        assert_eq!(root.children.len(), 1);
        assert!(root.get_child(&layer_id).is_some());
    }

    #[test]
    fn test_document_with_layers() {
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        // Hide layer2 to test visibility filtering
        layer2.layer.visible = false;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Only visible layers should be returned
        assert_eq!(doc.visible_layers().count(), 1);

        // Update layer2 to be visible via root access
        let ids: Vec<_> = doc.root.children.iter().map(|n| n.id()).collect();
        if let Some(layer) = doc.root.get_child_mut(&ids[1]) {
            layer.set_visible(true);
        }

        assert_eq!(doc.visible_layers().count(), 2);
    }
}
