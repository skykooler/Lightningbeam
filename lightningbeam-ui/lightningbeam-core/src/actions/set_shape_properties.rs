//! Set shape properties action
//!
//! Handles changing shape properties (fill color, stroke color, stroke width)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::{ShapeColor, StrokeStyle};
use uuid::Uuid;

/// Property change for a shape
#[derive(Clone, Debug)]
pub enum ShapePropertyChange {
    FillColor(Option<ShapeColor>),
    StrokeColor(Option<ShapeColor>),
    StrokeWidth(f64),
}

/// Action that sets properties on a shape
pub struct SetShapePropertiesAction {
    /// Layer containing the shape
    layer_id: Uuid,

    /// Shape to modify
    shape_id: Uuid,

    /// New property value
    new_value: ShapePropertyChange,

    /// Old property value (stored after first execution)
    old_value: Option<ShapePropertyChange>,
}

impl SetShapePropertiesAction {
    /// Create a new action to set a property on a shape
    pub fn new(layer_id: Uuid, shape_id: Uuid, new_value: ShapePropertyChange) -> Self {
        Self {
            layer_id,
            shape_id,
            new_value,
            old_value: None,
        }
    }

    /// Create action to set fill color
    pub fn set_fill_color(layer_id: Uuid, shape_id: Uuid, color: Option<ShapeColor>) -> Self {
        Self::new(layer_id, shape_id, ShapePropertyChange::FillColor(color))
    }

    /// Create action to set stroke color
    pub fn set_stroke_color(layer_id: Uuid, shape_id: Uuid, color: Option<ShapeColor>) -> Self {
        Self::new(layer_id, shape_id, ShapePropertyChange::StrokeColor(color))
    }

    /// Create action to set stroke width
    pub fn set_stroke_width(layer_id: Uuid, shape_id: Uuid, width: f64) -> Self {
        Self::new(layer_id, shape_id, ShapePropertyChange::StrokeWidth(width))
    }
}

impl Action for SetShapePropertiesAction {
    fn execute(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                if let Some(shape) = vector_layer.shapes.get_mut(&self.shape_id) {
                    // Store old value if not already stored
                    if self.old_value.is_none() {
                        self.old_value = Some(match &self.new_value {
                            ShapePropertyChange::FillColor(_) => {
                                ShapePropertyChange::FillColor(shape.fill_color)
                            }
                            ShapePropertyChange::StrokeColor(_) => {
                                ShapePropertyChange::StrokeColor(shape.stroke_color)
                            }
                            ShapePropertyChange::StrokeWidth(_) => {
                                let width = shape
                                    .stroke_style
                                    .as_ref()
                                    .map(|s| s.width)
                                    .unwrap_or(1.0);
                                ShapePropertyChange::StrokeWidth(width)
                            }
                        });
                    }

                    // Apply new value
                    match &self.new_value {
                        ShapePropertyChange::FillColor(color) => {
                            shape.fill_color = *color;
                        }
                        ShapePropertyChange::StrokeColor(color) => {
                            shape.stroke_color = *color;
                        }
                        ShapePropertyChange::StrokeWidth(width) => {
                            if let Some(ref mut style) = shape.stroke_style {
                                style.width = *width;
                            } else {
                                // Create stroke style if it doesn't exist
                                shape.stroke_style = Some(StrokeStyle {
                                    width: *width,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if let Some(old_value) = &self.old_value {
            if let Some(layer) = document.get_layer_mut(&self.layer_id) {
                if let AnyLayer::Vector(vector_layer) = layer {
                    if let Some(shape) = vector_layer.shapes.get_mut(&self.shape_id) {
                        match old_value {
                            ShapePropertyChange::FillColor(color) => {
                                shape.fill_color = *color;
                            }
                            ShapePropertyChange::StrokeColor(color) => {
                                shape.stroke_color = *color;
                            }
                            ShapePropertyChange::StrokeWidth(width) => {
                                if let Some(ref mut style) = shape.stroke_style {
                                    style.width = *width;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn description(&self) -> String {
        match &self.new_value {
            ShapePropertyChange::FillColor(_) => "Set fill color".to_string(),
            ShapePropertyChange::StrokeColor(_) => "Set stroke color".to_string(),
            ShapePropertyChange::StrokeWidth(_) => "Set stroke width".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;
    use kurbo::BezPath;

    fn create_test_shape() -> Shape {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        path.line_to((100.0, 100.0));
        path.line_to((0.0, 100.0));
        path.close_path();

        let mut shape = Shape::new(path);
        shape.fill_color = Some(ShapeColor::rgb(255, 0, 0));
        shape.stroke_color = Some(ShapeColor::rgb(0, 0, 0));
        shape.stroke_style = Some(StrokeStyle {
            width: 2.0,
            ..Default::default()
        });
        shape
    }

    #[test]
    fn test_set_fill_color() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = create_test_shape();
        let shape_id = shape.id;
        layer.shapes.insert(shape_id, shape);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Verify initial color
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.fill_color.unwrap().r, 255);
        }

        // Create and execute action
        let new_color = Some(ShapeColor::rgb(0, 255, 0));
        let mut action = SetShapePropertiesAction::set_fill_color(layer_id, shape_id, new_color);
        action.execute(&mut document);

        // Verify color changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.fill_color.unwrap().g, 255);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.fill_color.unwrap().r, 255);
        }
    }

    #[test]
    fn test_set_stroke_width() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = create_test_shape();
        let shape_id = shape.id;
        layer.shapes.insert(shape_id, shape);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Verify initial width
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.stroke_style.as_ref().unwrap().width, 2.0);
        }

        // Create and execute action
        let mut action = SetShapePropertiesAction::set_stroke_width(layer_id, shape_id, 5.0);
        action.execute(&mut document);

        // Verify width changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.stroke_style.as_ref().unwrap().width, 5.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            assert_eq!(shape.stroke_style.as_ref().unwrap().width, 2.0);
        }
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();
        let shape_id = Uuid::new_v4();

        let action1 =
            SetShapePropertiesAction::set_fill_color(layer_id, shape_id, Some(ShapeColor::rgb(0, 0, 0)));
        assert_eq!(action1.description(), "Set fill color");

        let action2 =
            SetShapePropertiesAction::set_stroke_color(layer_id, shape_id, Some(ShapeColor::rgb(0, 0, 0)));
        assert_eq!(action2.description(), "Set stroke color");

        let action3 = SetShapePropertiesAction::set_stroke_width(layer_id, shape_id, 3.0);
        assert_eq!(action3.description(), "Set stroke width");
    }
}
