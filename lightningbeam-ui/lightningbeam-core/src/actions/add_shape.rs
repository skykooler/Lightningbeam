//! Add shape action — inserts strokes into the DCEL.
//!
//! Converts a BezPath into cubic segments and inserts them via
//! `Dcel::insert_stroke()`. Undo is handled by snapshotting the DCEL.

use crate::action::Action;
use crate::dcel::{bezpath_to_cubic_segments, Dcel, DEFAULT_SNAP_EPSILON};
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::{ShapeColor, StrokeStyle};
use kurbo::BezPath;
use uuid::Uuid;

/// Action that inserts a drawn path into a vector layer's DCEL keyframe.
pub struct AddShapeAction {
    layer_id: Uuid,
    time: f64,
    path: BezPath,
    stroke_style: Option<StrokeStyle>,
    stroke_color: Option<ShapeColor>,
    fill_color: Option<ShapeColor>,
    is_closed: bool,
    description_text: String,
    /// Snapshot of the DCEL before insertion (for undo).
    dcel_before: Option<Dcel>,
}

impl AddShapeAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        path: BezPath,
        stroke_style: Option<StrokeStyle>,
        stroke_color: Option<ShapeColor>,
        fill_color: Option<ShapeColor>,
        is_closed: bool,
    ) -> Self {
        Self {
            layer_id,
            time,
            path,
            stroke_style,
            stroke_color,
            fill_color,
            is_closed,
            description_text: "Add shape".to_string(),
            dcel_before: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description_text = desc.into();
        self
    }
}

impl Action for AddShapeAction {
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

        // Snapshot for undo
        self.dcel_before = Some(dcel.clone());

        let subpaths = bezpath_to_cubic_segments(&self.path);

        for segments in &subpaths {
            if segments.is_empty() {
                continue;
            }
            let result = dcel.insert_stroke(
                segments,
                self.stroke_style.clone(),
                self.stroke_color.clone(),
                DEFAULT_SNAP_EPSILON,
            );

            // Apply fill to new faces if this is a closed shape with fill
            if self.is_closed {
                if let Some(ref fill) = self.fill_color {
                    for face_id in &result.new_faces {
                        dcel.face_mut(*face_id).fill_color = Some(fill.clone());
                    }
                }
            }
        }

        dcel.rebuild_spatial_index();

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        let keyframe = vl.ensure_keyframe_at(self.time);
        keyframe.dcel = self
            .dcel_before
            .take()
            .ok_or_else(|| "No DCEL snapshot for undo".to_string())?;

        Ok(())
    }

    fn description(&self) -> String {
        self.description_text.clone()
    }
}
