//! Document structure for Lightningbeam
//!
//! The Document represents a complete animation project with settings
//! and a root graphics object containing the scene graph.

use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use serde::{Deserialize, Serialize};
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

    /// Get visible layers at the current time from the root graphics object
    pub fn visible_layers(&self) -> impl Iterator<Item = &AnyLayer> {
        self.root
            .children
            .iter()
            .filter(|layer| {
                let layer = layer.layer();
                layer.visible && layer.contains_time(self.current_time)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;

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

        let mut layer1 = VectorLayer::new("Layer 1");
        layer1.layer.start_time = 0.0;
        layer1.layer.end_time = 5.0;

        let mut layer2 = VectorLayer::new("Layer 2");
        layer2.layer.start_time = 3.0;
        layer2.layer.end_time = 8.0;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        doc.set_time(4.0);
        assert_eq!(doc.visible_layers().count(), 2);

        doc.set_time(6.0);
        assert_eq!(doc.visible_layers().count(), 1);
    }
}
