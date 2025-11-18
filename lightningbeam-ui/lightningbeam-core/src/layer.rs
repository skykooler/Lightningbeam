//! Layer system for Lightningbeam
//!
//! Layers organize objects and shapes, and contain animation data.

use crate::animation::AnimationData;
use crate::object::Object;
use crate::shape::Shape;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Layer type
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    /// Vector graphics layer (shapes and objects)
    Vector,
    /// Audio track
    Audio,
    /// Video clip
    Video,
    /// Generic automation layer
    Automation,
}

/// Base layer structure
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    /// Unique identifier
    pub id: Uuid,

    /// Layer type
    pub layer_type: LayerType,

    /// Layer name
    pub name: String,

    /// Whether the layer is visible
    pub visible: bool,

    /// Layer opacity (0.0 to 1.0)
    pub opacity: f64,

    /// Start time in seconds
    pub start_time: f64,

    /// End time in seconds
    pub end_time: f64,

    /// Animation data for this layer
    pub animation_data: AnimationData,
}

impl Layer {
    /// Create a new layer
    pub fn new(layer_type: LayerType, name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            layer_type,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            start_time: 0.0,
            end_time: 10.0, // Default 10 second duration
            animation_data: AnimationData::new(),
        }
    }

    /// Create with a specific ID
    pub fn with_id(id: Uuid, layer_type: LayerType, name: impl Into<String>) -> Self {
        Self {
            id,
            layer_type,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            start_time: 0.0,
            end_time: 10.0,
            animation_data: AnimationData::new(),
        }
    }

    /// Set the time range
    pub fn with_time_range(mut self, start: f64, end: f64) -> Self {
        self.start_time = start;
        self.end_time = end;
        self
    }

    /// Set visibility
    pub fn with_visibility(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Get duration
    pub fn duration(&self) -> f64 {
        self.end_time - self.start_time
    }

    /// Check if a time is within this layer's range
    pub fn contains_time(&self, time: f64) -> bool {
        time >= self.start_time && time <= self.end_time
    }
}

/// Vector layer containing shapes and objects
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VectorLayer {
    /// Base layer properties
    pub layer: Layer,

    /// Shapes defined in this layer
    pub shapes: Vec<Shape>,

    /// Object instances (references to shapes with transforms)
    pub objects: Vec<Object>,
}

impl VectorLayer {
    /// Create a new vector layer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::new(LayerType::Vector, name),
            shapes: Vec::new(),
            objects: Vec::new(),
        }
    }

    /// Add a shape to this layer
    pub fn add_shape(&mut self, shape: Shape) -> Uuid {
        let id = shape.id;
        self.shapes.push(shape);
        id
    }

    /// Add an object to this layer
    pub fn add_object(&mut self, object: Object) -> Uuid {
        let id = object.id;
        self.objects.push(object);
        id
    }

    /// Find a shape by ID
    pub fn get_shape(&self, id: &Uuid) -> Option<&Shape> {
        self.shapes.iter().find(|s| &s.id == id)
    }

    /// Find a mutable shape by ID
    pub fn get_shape_mut(&mut self, id: &Uuid) -> Option<&mut Shape> {
        self.shapes.iter_mut().find(|s| &s.id == id)
    }

    /// Find an object by ID
    pub fn get_object(&self, id: &Uuid) -> Option<&Object> {
        self.objects.iter().find(|o| &o.id == id)
    }

    /// Find a mutable object by ID
    pub fn get_object_mut(&mut self, id: &Uuid) -> Option<&mut Object> {
        self.objects.iter_mut().find(|o| &o.id == id)
    }

    // === MUTATION METHODS (pub(crate) - only accessible to action module) ===

    /// Add a shape to this layer (internal, for actions only)
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn add_shape_internal(&mut self, shape: Shape) -> Uuid {
        let id = shape.id;
        self.shapes.push(shape);
        id
    }

    /// Add an object to this layer (internal, for actions only)
    ///
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn add_object_internal(&mut self, object: Object) -> Uuid {
        let id = object.id;
        self.objects.push(object);
        id
    }

    /// Remove a shape from this layer (internal, for actions only)
    ///
    /// Returns the removed shape if found.
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn remove_shape_internal(&mut self, id: &Uuid) -> Option<Shape> {
        if let Some(index) = self.shapes.iter().position(|s| &s.id == id) {
            Some(self.shapes.remove(index))
        } else {
            None
        }
    }

    /// Remove an object from this layer (internal, for actions only)
    ///
    /// Returns the removed object if found.
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn remove_object_internal(&mut self, id: &Uuid) -> Option<Object> {
        if let Some(index) = self.objects.iter().position(|o| &o.id == id) {
            Some(self.objects.remove(index))
        } else {
            None
        }
    }

    /// Modify an object in place (internal, for actions only)
    ///
    /// Applies the given function to the object if found.
    /// This method is intentionally `pub(crate)` to ensure mutations
    /// only happen through the action system.
    pub(crate) fn modify_object_internal<F>(&mut self, id: &Uuid, f: F)
    where
        F: FnOnce(&mut Object),
    {
        if let Some(object) = self.get_object_mut(id) {
            f(object);
        }
    }
}

/// Audio layer (placeholder for future implementation)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudioLayer {
    /// Base layer properties
    pub layer: Layer,

    /// Audio file path or data reference
    pub audio_source: Option<String>,
}

impl AudioLayer {
    /// Create a new audio layer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::new(LayerType::Audio, name),
            audio_source: None,
        }
    }
}

/// Video layer (placeholder for future implementation)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoLayer {
    /// Base layer properties
    pub layer: Layer,

    /// Video file path or data reference
    pub video_source: Option<String>,
}

impl VideoLayer {
    /// Create a new video layer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::new(LayerType::Video, name),
            video_source: None,
        }
    }
}

/// Unified layer enum for polymorphic handling
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnyLayer {
    Vector(VectorLayer),
    Audio(AudioLayer),
    Video(VideoLayer),
}

impl AnyLayer {
    /// Get a reference to the base layer
    pub fn layer(&self) -> &Layer {
        match self {
            AnyLayer::Vector(l) => &l.layer,
            AnyLayer::Audio(l) => &l.layer,
            AnyLayer::Video(l) => &l.layer,
        }
    }

    /// Get a mutable reference to the base layer
    pub fn layer_mut(&mut self) -> &mut Layer {
        match self {
            AnyLayer::Vector(l) => &mut l.layer,
            AnyLayer::Audio(l) => &mut l.layer,
            AnyLayer::Video(l) => &mut l.layer,
        }
    }

    /// Get the layer ID
    pub fn id(&self) -> Uuid {
        self.layer().id
    }

    /// Get the layer name
    pub fn name(&self) -> &str {
        &self.layer().name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_creation() {
        let layer = Layer::new(LayerType::Vector, "Test Layer");
        assert_eq!(layer.layer_type, LayerType::Vector);
        assert_eq!(layer.name, "Test Layer");
        assert_eq!(layer.opacity, 1.0);
    }

    #[test]
    fn test_vector_layer() {
        let vector_layer = VectorLayer::new("My Layer");
        assert_eq!(vector_layer.shapes.len(), 0);
        assert_eq!(vector_layer.objects.len(), 0);
    }

    #[test]
    fn test_layer_time_range() {
        let layer = Layer::new(LayerType::Vector, "Test")
            .with_time_range(5.0, 15.0);

        assert_eq!(layer.duration(), 10.0);
        assert!(layer.contains_time(10.0));
        assert!(!layer.contains_time(3.0));
        assert!(!layer.contains_time(20.0));
    }
}
