//! Paint bucket fill action — sets fill_color on a DCEL face.

use crate::action::Action;
use crate::dcel::FaceId;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that performs a paint bucket fill on a DCEL face.
pub struct PaintBucketAction {
    layer_id: Uuid,
    time: f64,
    click_point: Point,
    fill_color: ShapeColor,
    /// The face that was hit (resolved during execute)
    hit_face: Option<FaceId>,
    /// Previous fill color for undo
    old_fill_color: Option<Option<ShapeColor>>,
}

impl PaintBucketAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        click_point: Point,
        fill_color: ShapeColor,
    ) -> Self {
        Self {
            layer_id,
            time,
            click_point,
            fill_color,
            hit_face: None,
            old_fill_color: None,
        }
    }
}

impl Action for PaintBucketAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        let keyframe = vl.ensure_keyframe_at(self.time);
        let dcel = &mut keyframe.dcel;

        // Record for debug test generation (if recording is active)
        dcel.record_paint_point(self.click_point);

        // Find the enclosing cycle for the click point
        let query = dcel.find_face_at_point(self.click_point);

        // Dump cumulative test to stderr after every paint click (if recording)
        if dcel.is_recording() {
            eprintln!("\n--- DCEL debug test (cumulative, face={:?}) ---", query.face);
            dcel.debug_recorder.as_ref().unwrap().dump_test("test_recorded");
            eprintln!("--- end test ---\n");
        }

        if query.cycle_he.is_none() {
            // No edges at all — nothing to fill
            return Err("No face at click point".to_string());
        }

        // If the cycle is in F0 (no face created yet), create one now
        let face_id = if query.face.0 == 0 {
            dcel.create_face_at_cycle(query.cycle_he)
        } else {
            query.face
        };

        // Store for undo
        self.hit_face = Some(face_id);
        self.old_fill_color = Some(dcel.face(face_id).fill_color.clone());

        // Apply fill
        dcel.face_mut(face_id).fill_color = Some(self.fill_color.clone());

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let face_id = self.hit_face.ok_or("No face to undo")?;

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        let keyframe = vl.ensure_keyframe_at(self.time);
        let dcel = &mut keyframe.dcel;

        dcel.face_mut(face_id).fill_color = self.old_fill_color.take().unwrap_or(None);

        Ok(())
    }

    fn description(&self) -> String {
        "Paint bucket fill".to_string()
    }
}
