//! Remove shapes action
//!
//! Handles removing shapes and shape instances from a vector layer (for cut/delete).

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::ShapeInstance;
use crate::shape::Shape;
use uuid::Uuid;

/// Action that removes shapes and their instances from a vector layer
pub struct RemoveShapesAction {
    /// Layer ID containing the shapes
    layer_id: Uuid,
    /// Shape IDs to remove
    shape_ids: Vec<Uuid>,
    /// Shape instance IDs to remove
    instance_ids: Vec<Uuid>,
    /// Saved shapes for rollback
    saved_shapes: Vec<(Uuid, Shape)>,
    /// Saved instances for rollback
    saved_instances: Vec<ShapeInstance>,
}

impl RemoveShapesAction {
    /// Create a new remove shapes action
    pub fn new(layer_id: Uuid, shape_ids: Vec<Uuid>, instance_ids: Vec<Uuid>) -> Self {
        Self {
            layer_id,
            shape_ids,
            instance_ids,
            saved_shapes: Vec::new(),
            saved_instances: Vec::new(),
        }
    }
}

impl Action for RemoveShapesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.saved_shapes.clear();
        self.saved_instances.clear();

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vector_layer = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        // Remove and save shape instances
        let mut remaining_instances = Vec::new();
        for inst in vector_layer.shape_instances.drain(..) {
            if self.instance_ids.contains(&inst.id) {
                self.saved_instances.push(inst);
            } else {
                remaining_instances.push(inst);
            }
        }
        vector_layer.shape_instances = remaining_instances;

        // Remove and save shape definitions
        for shape_id in &self.shape_ids {
            if let Some(shape) = vector_layer.shapes.remove(shape_id) {
                self.saved_shapes.push((*shape_id, shape));
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

        // Restore shapes
        for (id, shape) in self.saved_shapes.drain(..) {
            vector_layer.shapes.insert(id, shape);
        }

        // Restore instances
        for inst in self.saved_instances.drain(..) {
            vector_layer.shape_instances.push(inst);
        }

        Ok(())
    }

    fn description(&self) -> String {
        let count = self.instance_ids.len();
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
    use crate::object::ShapeInstance;
    use crate::shape::Shape;
    use vello::kurbo::BezPath;

    #[test]
    fn test_remove_shapes() {
        let mut document = Document::new("Test");

        let mut vector_layer = VectorLayer::new("Layer 1");

        // Add a shape and instance
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 100.0));
        let shape = Shape::new(path);
        let shape_id = shape.id;
        let instance = ShapeInstance::new(shape_id);
        let instance_id = instance.id;

        vector_layer.shapes.insert(shape_id, shape);
        vector_layer.shape_instances.push(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(vector_layer));

        // Remove
        let mut action = RemoveShapesAction::new(layer_id, vec![shape_id], vec![instance_id]);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert!(vl.shapes.is_empty());
            assert!(vl.shape_instances.is_empty());
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes.len(), 1);
            assert_eq!(vl.shape_instances.len(), 1);
        }
    }
}
