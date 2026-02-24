//! Paint bucket fill action — STUB: needs DCEL rewrite
//!
//! With DCEL, paint bucket simply hit-tests faces and sets fill_color.

use crate::action::Action;
use crate::document::Document;
use crate::gap_handling::GapHandlingMode;
use crate::shape::ShapeColor;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that performs a paint bucket fill operation
/// TODO: Rewrite to use DCEL face hit-testing
pub struct PaintBucketAction {
    layer_id: Uuid,
    time: f64,
    click_point: Point,
    fill_color: ShapeColor,
    _tolerance: f64,
    _gap_mode: GapHandlingMode,
    created_shape_id: Option<Uuid>,
}

impl PaintBucketAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        click_point: Point,
        fill_color: ShapeColor,
        tolerance: f64,
        gap_mode: GapHandlingMode,
    ) -> Self {
        Self {
            layer_id,
            time,
            click_point,
            fill_color,
            _tolerance: tolerance,
            _gap_mode: gap_mode,
            created_shape_id: None,
        }
    }
}

impl Action for PaintBucketAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time, self.click_point, self.fill_color);
        // TODO: Hit-test DCEL faces, set face.fill_color
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        self.created_shape_id = None;
        Ok(())
    }

    fn description(&self) -> String {
        "Paint bucket fill".to_string()
    }
}
