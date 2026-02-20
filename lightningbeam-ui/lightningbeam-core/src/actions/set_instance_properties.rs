//! Set shape instance properties action
//!
//! Handles changing individual properties on shapes (position, rotation, scale, etc.)
//! with undo/redo support. In the keyframe model, these operate on Shape's transform
//! and opacity fields within the active keyframe.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Individual property change for a shape instance
#[derive(Clone, Debug)]
pub enum InstancePropertyChange {
    X(f64),
    Y(f64),
    Rotation(f64),
    ScaleX(f64),
    ScaleY(f64),
    SkewX(f64),
    SkewY(f64),
    Opacity(f64),
}

impl InstancePropertyChange {
    /// Extract the f64 value from any variant
    fn value(&self) -> f64 {
        match self {
            InstancePropertyChange::X(v) => *v,
            InstancePropertyChange::Y(v) => *v,
            InstancePropertyChange::Rotation(v) => *v,
            InstancePropertyChange::ScaleX(v) => *v,
            InstancePropertyChange::ScaleY(v) => *v,
            InstancePropertyChange::SkewX(v) => *v,
            InstancePropertyChange::SkewY(v) => *v,
            InstancePropertyChange::Opacity(v) => *v,
        }
    }
}

/// Action that sets a property on one or more shapes in a keyframe
pub struct SetInstancePropertiesAction {
    /// Layer containing the shapes
    layer_id: Uuid,

    /// Time of the keyframe
    time: f64,

    /// Shape IDs to modify and their old values
    shape_changes: Vec<(Uuid, Option<f64>)>,

    /// Property to change
    property: InstancePropertyChange,
}

impl SetInstancePropertiesAction {
    /// Create a new action to set a property on a single shape
    pub fn new(layer_id: Uuid, time: f64, shape_id: Uuid, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            time,
            shape_changes: vec![(shape_id, None)],
            property,
        }
    }

    /// Create a new action to set a property on multiple shapes
    pub fn new_batch(layer_id: Uuid, time: f64, shape_ids: Vec<Uuid>, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            time,
            shape_changes: shape_ids.into_iter().map(|id| (id, None)).collect(),
            property,
        }
    }

    fn get_value_from_shape(shape: &crate::shape::Shape, property: &InstancePropertyChange) -> f64 {
        match property {
            InstancePropertyChange::X(_) => shape.transform.x,
            InstancePropertyChange::Y(_) => shape.transform.y,
            InstancePropertyChange::Rotation(_) => shape.transform.rotation,
            InstancePropertyChange::ScaleX(_) => shape.transform.scale_x,
            InstancePropertyChange::ScaleY(_) => shape.transform.scale_y,
            InstancePropertyChange::SkewX(_) => shape.transform.skew_x,
            InstancePropertyChange::SkewY(_) => shape.transform.skew_y,
            InstancePropertyChange::Opacity(_) => shape.opacity,
        }
    }

    fn set_value_on_shape(shape: &mut crate::shape::Shape, property: &InstancePropertyChange, value: f64) {
        match property {
            InstancePropertyChange::X(_) => shape.transform.x = value,
            InstancePropertyChange::Y(_) => shape.transform.y = value,
            InstancePropertyChange::Rotation(_) => shape.transform.rotation = value,
            InstancePropertyChange::ScaleX(_) => shape.transform.scale_x = value,
            InstancePropertyChange::ScaleY(_) => shape.transform.scale_y = value,
            InstancePropertyChange::SkewX(_) => shape.transform.skew_x = value,
            InstancePropertyChange::SkewY(_) => shape.transform.skew_y = value,
            InstancePropertyChange::Opacity(_) => shape.opacity = value,
        }
    }
}

impl Action for SetInstancePropertiesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let new_value = self.property.value();

        // First pass: collect old values
        if let Some(layer) = document.get_layer(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_id, old_value) in &mut self.shape_changes {
                    if old_value.is_none() {
                        if let Some(shape) = vector_layer.get_shape_in_keyframe(shape_id, self.time) {
                            *old_value = Some(Self::get_value_from_shape(shape, &self.property));
                        }
                    }
                }
            }
        }

        // Second pass: apply new values
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_id, _) in &self.shape_changes {
                    if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                        Self::set_value_on_shape(shape, &self.property, new_value);
                    }
                }
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                for (shape_id, old_value) in &self.shape_changes {
                    if let Some(value) = old_value {
                        if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                            Self::set_value_on_shape(shape, &self.property, *value);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            InstancePropertyChange::X(_) => "X position",
            InstancePropertyChange::Y(_) => "Y position",
            InstancePropertyChange::Rotation(_) => "rotation",
            InstancePropertyChange::ScaleX(_) => "scale X",
            InstancePropertyChange::ScaleY(_) => "scale Y",
            InstancePropertyChange::SkewX(_) => "skew X",
            InstancePropertyChange::SkewY(_) => "skew Y",
            InstancePropertyChange::Opacity(_) => "opacity",
        };

        if self.shape_changes.len() == 1 {
            format!("Set {}", property_name)
        } else {
            format!("Set {} on {} shapes", property_name, self.shape_changes.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;
    use vello::kurbo::BezPath;

    fn make_shape_at(x: f64, y: f64) -> Shape {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((10.0, 10.0));
        Shape::new(path).with_position(x, y)
    }

    #[test]
    fn test_set_x_position() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = make_shape_at(10.0, 20.0);
        let shape_id = shape.id;
        layer.add_shape_to_keyframe(shape, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let mut action = SetInstancePropertiesAction::new(
            layer_id,
            0.0,
            shape_id,
            InstancePropertyChange::X(50.0),
        );
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.transform.x, 50.0);
            assert_eq!(s.transform.y, 20.0);
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.transform.x, 10.0);
        }
    }

    #[test]
    fn test_set_opacity() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = make_shape_at(0.0, 0.0);
        let shape_id = shape.id;
        layer.add_shape_to_keyframe(shape, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let mut action = SetInstancePropertiesAction::new(
            layer_id,
            0.0,
            shape_id,
            InstancePropertyChange::Opacity(0.5),
        );
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.opacity, 0.5);
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let s = vl.get_shape_in_keyframe(&shape_id, 0.0).unwrap();
            assert_eq!(s.opacity, 1.0);
        }
    }

    #[test]
    fn test_batch_set_scale() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape1 = make_shape_at(0.0, 0.0);
        let shape1_id = shape1.id;
        let shape2 = make_shape_at(10.0, 10.0);
        let shape2_id = shape2.id;

        layer.add_shape_to_keyframe(shape1, 0.0);
        layer.add_shape_to_keyframe(shape2, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let mut action = SetInstancePropertiesAction::new_batch(
            layer_id,
            0.0,
            vec![shape1_id, shape2_id],
            InstancePropertyChange::ScaleX(2.0),
        );
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.get_shape_in_keyframe(&shape1_id, 0.0).unwrap().transform.scale_x, 2.0);
            assert_eq!(vl.get_shape_in_keyframe(&shape2_id, 0.0).unwrap().transform.scale_x, 2.0);
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.get_shape_in_keyframe(&shape1_id, 0.0).unwrap().transform.scale_x, 1.0);
            assert_eq!(vl.get_shape_in_keyframe(&shape2_id, 0.0).unwrap().transform.scale_x, 1.0);
        }
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();
        let shape_id = Uuid::new_v4();

        let action1 = SetInstancePropertiesAction::new(
            layer_id, 0.0, shape_id,
            InstancePropertyChange::X(0.0),
        );
        assert_eq!(action1.description(), "Set X position");

        let action2 = SetInstancePropertiesAction::new(
            layer_id, 0.0, shape_id,
            InstancePropertyChange::Rotation(0.0),
        );
        assert_eq!(action2.description(), "Set rotation");

        let action3 = SetInstancePropertiesAction::new_batch(
            layer_id, 0.0,
            vec![Uuid::new_v4(), Uuid::new_v4()],
            InstancePropertyChange::Opacity(1.0),
        );
        assert_eq!(action3.description(), "Set opacity on 2 shapes");
    }
}
