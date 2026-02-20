//! Remove shapes action
//!
//! Handles removing shapes from a vector layer's keyframe (for cut/delete).

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::Shape;
use uuid::Uuid;

/// Action that removes shapes from a vector layer's keyframe
pub struct RemoveShapesAction {
    /// Layer ID containing the shapes
    layer_id: Uuid,
    /// Shape IDs to remove
    shape_ids: Vec<Uuid>,
    /// Time of the keyframe
    time: f64,
    /// Saved shapes for rollback
    saved_shapes: Vec<Shape>,
}

impl RemoveShapesAction {
    pub fn new(layer_id: Uuid, shape_ids: Vec<Uuid>, time: f64) -> Self {
        Self {
            layer_id,
            shape_ids,
            time,
            saved_shapes: Vec::new(),
        }
    }
}

impl Action for RemoveShapesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.saved_shapes.clear();

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vector_layer = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        for shape_id in &self.shape_ids {
            if let Some(shape) = vector_layer.remove_shape_from_keyframe(shape_id, self.time) {
                self.saved_shapes.push(shape);
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vector_layer = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        for shape in self.saved_shapes.drain(..) {
            vector_layer.add_shape_to_keyframe(shape, self.time);
        }

        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_ids.len();
        if count == 1 {
            "Delete shape".to_string()
        } else {
            format!("Delete {} shapes", count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;
    use vello::kurbo::BezPath;

    #[test]
    fn test_remove_shapes() {
        let mut document = Document::new("Test");
        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 100.0));
        let shape = Shape::new(path);
        let shape_id = shape.id;

        vector_layer.add_shape_to_keyframe(shape, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(vector_layer));

        let mut action = RemoveShapesAction::new(layer_id, vec![shape_id], 0.0);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert!(vl.shapes_at_time(0.0).is_empty());
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes_at_time(0.0).len(), 1);
        }
    }
}
