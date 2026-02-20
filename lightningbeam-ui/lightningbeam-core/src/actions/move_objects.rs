//! Move shapes action
//!
//! Handles moving one or more shapes to new positions within a keyframe.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that moves shapes to new positions within a keyframe
pub struct MoveShapeInstancesAction {
    layer_id: Uuid,
    time: f64,
    /// Map of shape IDs to their old and new positions
    shape_positions: HashMap<Uuid, (Point, Point)>,
}

impl MoveShapeInstancesAction {
    pub fn new(layer_id: Uuid, time: f64, shape_positions: HashMap<Uuid, (Point, Point)>) -> Self {
        Self {
            layer_id,
            time,
            shape_positions,
        }
    }
}

impl Action for MoveShapeInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            for (shape_id, (_old, new)) in &self.shape_positions {
                if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                    shape.transform.x = new.x;
                    shape.transform.y = new.y;
                }
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        if let AnyLayer::Vector(vector_layer) = layer {
            for (shape_id, (old, _new)) in &self.shape_positions {
                if let Some(shape) = vector_layer.get_shape_in_keyframe_mut(shape_id, self.time) {
                    shape.transform.x = old.x;
                    shape.transform.y = old.y;
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_positions.len();
        if count == 1 {
            "Move shape".to_string()
        } else {
            format!("Move {} shapes", count)
        }
    }
}
