//! Set shape properties action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use crate::shape::ShapeColor;
use uuid::Uuid;

/// Property change for a shape
#[derive(Clone, Debug)]
pub enum ShapePropertyChange {
    FillColor(Option<ShapeColor>),
    StrokeColor(Option<ShapeColor>),
    StrokeWidth(f64),
}

/// Action that sets properties on a shape
/// TODO: Replace with DCEL face/edge property changes
pub struct SetShapePropertiesAction {
    layer_id: Uuid,
    shape_id: Uuid,
    time: f64,
    new_value: ShapePropertyChange,
    old_value: Option<ShapePropertyChange>,
}

impl SetShapePropertiesAction {
    pub fn new(layer_id: Uuid, shape_id: Uuid, time: f64, new_value: ShapePropertyChange) -> Self {
        Self {
            layer_id,
            shape_id,
            time,
            new_value,
            old_value: None,
        }
    }

    pub fn set_fill_color(layer_id: Uuid, shape_id: Uuid, time: f64, color: Option<ShapeColor>) -> Self {
        Self::new(layer_id, shape_id, time, ShapePropertyChange::FillColor(color))
    }

    pub fn set_stroke_color(layer_id: Uuid, shape_id: Uuid, time: f64, color: Option<ShapeColor>) -> Self {
        Self::new(layer_id, shape_id, time, ShapePropertyChange::StrokeColor(color))
    }

    pub fn set_stroke_width(layer_id: Uuid, shape_id: Uuid, time: f64, width: f64) -> Self {
        Self::new(layer_id, shape_id, time, ShapePropertyChange::StrokeWidth(width))
    }
}

impl Action for SetShapePropertiesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, &self.shape_id, self.time, &self.new_value);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = &self.old_value;
        Ok(())
    }

    fn description(&self) -> String {
        match &self.new_value {
            ShapePropertyChange::FillColor(_) => "Set fill color".to_string(),
            ShapePropertyChange::StrokeColor(_) => "Set stroke color".to_string(),
            ShapePropertyChange::StrokeWidth(_) => "Set stroke width".to_string(),
        }
    }
}
