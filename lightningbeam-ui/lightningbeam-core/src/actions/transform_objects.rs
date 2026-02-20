//! Transform shapes action
//!
//! Applies scale, rotation, and other transformations to shapes in a keyframe.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action to transform multiple shapes in a keyframe
pub struct TransformShapeInstancesAction {
    layer_id: Uuid,
    time: f64,
    /// Map of shape ID to (old transform, new transform)
    shape_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformShapeInstancesAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        shape_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            time,
            shape_transforms,
        }
    }
}

impl Action for TransformShapeInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_id, (_old, new)) in &self.shape_transforms {
                    if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                        shape.transform = new.clone();
                    }
                }
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_id, (old, _new)) in &self.shape_transforms {
                    if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                        shape.transform = old.clone();
                    }
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Transform {} shape(s)", self.shape_transforms.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;
    use vello::kurbo::BezPath;

    #[test]
    fn test_transform_shape() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 100.0));
        let shape = Shape::new(path).with_position(10.0, 20.0);
        let shape_id = shape.id;

        layer.add_shape_to_keyframe(shape, 0.0);
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let old_transform = Transform::with_position(10.0, 20.0);
        let new_transform = Transform::with_position(100.0, 200.0);
        let mut transforms = HashMap::new();
        transforms.insert(shape_id, (old_transform, new_transform));

        let mut action = TransformShapeInstancesAction::new(layer_id, 0.0, transforms);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.transform.x, 100.0);
            assert_eq!(s.transform.y, 200.0);
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.transform.x, 10.0);
            assert_eq!(s.transform.y, 20.0);
        }
    }
}
