//! Modify shape path action
//!
//! Handles modifying a shape's bezier path (for vector editing operations)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;
use vello::kurbo::BezPath;

/// Action that modifies a shape's path
///
/// This action is used for vector editing operations like dragging vertices,
/// reshaping curves, or manipulating control points.
pub struct ModifyShapePathAction {
    /// Layer containing the shape
    layer_id: Uuid,

    /// Shape to modify
    shape_id: Uuid,

    /// The version index being modified (for shapes with multiple versions)
    version_index: usize,

    /// New path
    new_path: BezPath,

    /// Old path (stored after first execution for undo)
    old_path: Option<BezPath>,
}

impl ModifyShapePathAction {
    /// Create a new action to modify a shape's path
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The layer containing the shape
    /// * `shape_id` - The shape to modify
    /// * `version_index` - The version index to modify (usually 0)
    /// * `new_path` - The new path to set
    pub fn new(layer_id: Uuid, shape_id: Uuid, version_index: usize, new_path: BezPath) -> Self {
        Self {
            layer_id,
            shape_id,
            version_index,
            new_path,
            old_path: None,
        }
    }

    /// Create action with old path already known (for optimization)
    pub fn with_old_path(
        layer_id: Uuid,
        shape_id: Uuid,
        version_index: usize,
        old_path: BezPath,
        new_path: BezPath,
    ) -> Self {
        Self {
            layer_id,
            shape_id,
            version_index,
            new_path,
            old_path: Some(old_path),
        }
    }
}

impl Action for ModifyShapePathAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            if let AnyLayer::Vector(vector_layer) = layer {
                if let Some(shape) = vector_layer.shapes.get_mut(&self.shape_id) {
                    // Check if version exists
                    if self.version_index >= shape.versions.len() {
                        return Err(format!(
                            "Version index {} out of bounds (shape has {} versions)",
                            self.version_index,
                            shape.versions.len()
                        ));
                    }

                    // Store old path if not already stored
                    if self.old_path.is_none() {
                        self.old_path = Some(shape.versions[self.version_index].path.clone());
                    }

                    // Apply new path
                    shape.versions[self.version_index].path = self.new_path.clone();

                    return Ok(());
                }
            }
        }

        Err(format!(
            "Could not find shape {} in layer {}",
            self.shape_id, self.layer_id
        ))
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(old_path) = &self.old_path {
            if let Some(layer) = document.get_layer_mut(&self.layer_id) {
                if let AnyLayer::Vector(vector_layer) = layer {
                    if let Some(shape) = vector_layer.shapes.get_mut(&self.shape_id) {
                        if self.version_index < shape.versions.len() {
                            shape.versions[self.version_index].path = old_path.clone();
                            return Ok(());
                        }
                    }
                }
            }
        }

        Err(format!(
            "Could not rollback shape path modification for shape {} in layer {}",
            self.shape_id, self.layer_id
        ))
    }

    fn description(&self) -> String {
        "Modify shape path".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;
    use crate::shape::Shape;

    fn create_test_path() -> BezPath {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((100.0, 0.0));
        path.line_to((100.0, 100.0));
        path.line_to((0.0, 100.0));
        path.close_path();
        path
    }

    fn create_modified_path() -> BezPath {
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((150.0, 0.0)); // Modified
        path.line_to((150.0, 150.0)); // Modified
        path.line_to((0.0, 150.0)); // Modified
        path.close_path();
        path
    }

    #[test]
    fn test_modify_shape_path() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = Shape::new(create_test_path());
        let shape_id = shape.id;
        layer.shapes.insert(shape_id, shape);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Verify initial path
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            let bbox = shape.versions[0].path.bounding_box();
            assert_eq!(bbox.width(), 100.0);
            assert_eq!(bbox.height(), 100.0);
        }

        // Create and execute action
        let new_path = create_modified_path();
        let mut action = ModifyShapePathAction::new(layer_id, shape_id, 0, new_path);
        action.execute(&mut document).unwrap();

        // Verify path changed
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            let bbox = shape.versions[0].path.bounding_box();
            assert_eq!(bbox.width(), 150.0);
            assert_eq!(bbox.height(), 150.0);
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        // Verify restored
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let shape = vl.shapes.get(&shape_id).unwrap();
            let bbox = shape.versions[0].path.bounding_box();
            assert_eq!(bbox.width(), 100.0);
            assert_eq!(bbox.height(), 100.0);
        }
    }

    #[test]
    fn test_invalid_version_index() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let shape = Shape::new(create_test_path());
        let shape_id = shape.id;
        layer.shapes.insert(shape_id, shape);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Try to modify non-existent version
        let new_path = create_modified_path();
        let mut action = ModifyShapePathAction::new(layer_id, shape_id, 5, new_path);
        let result = action.execute(&mut document);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("out of bounds"));
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();
        let shape_id = Uuid::new_v4();
        let action = ModifyShapePathAction::new(layer_id, shape_id, 0, create_test_path());
        assert_eq!(action.description(), "Modify shape path");
    }
}
