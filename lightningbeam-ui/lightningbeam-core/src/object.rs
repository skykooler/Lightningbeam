//! Object system for Lightningbeam
//!
//! An Object represents an instance of a Shape with transform properties.
//! Objects can be animated via the animation system.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 2D transform for an object
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Transform {
    /// X position
    pub x: f64,
    /// Y position
    pub y: f64,
    /// Rotation in degrees
    pub rotation: f64,
    /// X scale factor
    pub scale_x: f64,
    /// Y scale factor
    pub scale_y: f64,
    /// X skew in degrees
    pub skew_x: f64,
    /// Y skew in degrees
    pub skew_y: f64,
    /// Opacity (0.0 to 1.0)
    pub opacity: f64,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            rotation: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            skew_x: 0.0,
            skew_y: 0.0,
            opacity: 1.0,
        }
    }
}

impl Transform {
    /// Create a new default transform
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a transform with position
    pub fn with_position(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            ..Default::default()
        }
    }

    /// Create a transform with rotation
    pub fn with_rotation(rotation: f64) -> Self {
        Self {
            rotation,
            ..Default::default()
        }
    }

    /// Set position
    pub fn set_position(&mut self, x: f64, y: f64) {
        self.x = x;
        self.y = y;
    }

    /// Set rotation
    pub fn set_rotation(&mut self, rotation: f64) {
        self.rotation = rotation;
    }

    /// Set scale
    pub fn set_scale(&mut self, scale_x: f64, scale_y: f64) {
        self.scale_x = scale_x;
        self.scale_y = scale_y;
    }

    /// Set uniform scale
    pub fn set_uniform_scale(&mut self, scale: f64) {
        self.scale_x = scale;
        self.scale_y = scale;
    }

    /// Convert to an affine transform matrix
    pub fn to_affine(&self) -> kurbo::Affine {
        use kurbo::Affine;

        // Build transform: translate * rotate * scale * skew
        let translate = Affine::translate((self.x, self.y));
        let rotate = Affine::rotate(self.rotation.to_radians());
        let scale = Affine::scale_non_uniform(self.scale_x, self.scale_y);

        // Skew transforms
        let skew_x = if self.skew_x != 0.0 {
            let tan_skew = self.skew_x.to_radians().tan();
            Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };

        let skew_y = if self.skew_y != 0.0 {
            let tan_skew = self.skew_y.to_radians().tan();
            Affine::new([1.0, tan_skew, 0.0, 1.0, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };

        translate * rotate * scale * skew_x * skew_y
    }
}

/// An object instance (shape with transform)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Object {
    /// Unique identifier
    pub id: Uuid,

    /// Reference to the shape this object uses
    pub shape_id: Uuid,

    /// Transform properties
    pub transform: Transform,

    /// Name for display in UI
    pub name: Option<String>,
}

impl Object {
    /// Create a new object for a shape
    pub fn new(shape_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            shape_id,
            transform: Transform::default(),
            name: None,
        }
    }

    /// Create a new object with a specific ID
    pub fn with_id(id: Uuid, shape_id: Uuid) -> Self {
        Self {
            id,
            shape_id,
            transform: Transform::default(),
            name: None,
        }
    }

    /// Set the name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the transform
    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    /// Set position
    pub fn with_position(mut self, x: f64, y: f64) -> Self {
        self.transform.set_position(x, y);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transform_default() {
        let transform = Transform::default();
        assert_eq!(transform.x, 0.0);
        assert_eq!(transform.y, 0.0);
        assert_eq!(transform.scale_x, 1.0);
        assert_eq!(transform.opacity, 1.0);
    }

    #[test]
    fn test_transform_affine() {
        let mut transform = Transform::default();
        transform.set_position(100.0, 200.0);
        transform.set_rotation(45.0);

        let affine = transform.to_affine();
        // Just verify it doesn't panic
        let _ = affine.as_coeffs();
    }

    #[test]
    fn test_object_creation() {
        let shape_id = Uuid::new_v4();
        let object = Object::new(shape_id);

        assert_eq!(object.shape_id, shape_id);
        assert_eq!(object.transform.x, 0.0);
    }
}
