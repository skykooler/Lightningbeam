//! Resize-text-box action
//!
//! Updates a text layer's box origin and dimensions (which changes the wrap
//! width, so parley re-wraps the text). One undoable step.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use kurbo::Point;
use uuid::Uuid;

pub struct ResizeTextBoxAction {
    layer_id: Uuid,
    new_origin: Point,
    new_width: f64,
    new_height: f64,
    old: Option<(Point, f64, f64)>,
}

impl ResizeTextBoxAction {
    pub fn new(layer_id: Uuid, new_origin: Point, new_width: f64, new_height: f64) -> Self {
        Self { layer_id, new_origin, new_width, new_height, old: None }
    }
}

impl Action for ResizeTextBoxAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let AnyLayer::Text(text_layer) = layer else {
            return Err("ResizeTextBoxAction target is not a text layer".to_string());
        };
        if self.old.is_none() {
            self.old = Some((text_layer.box_origin, text_layer.box_width, text_layer.box_height));
        }
        text_layer.box_origin = self.new_origin;
        text_layer.box_width = self.new_width.max(1.0);
        text_layer.box_height = self.new_height.max(1.0);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some((origin, w, h)) = self.old else { return Ok(()) };
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        if let AnyLayer::Text(text_layer) = layer {
            text_layer.box_origin = origin;
            text_layer.box_width = w;
            text_layer.box_height = h;
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Resize text box".to_string()
    }
}
