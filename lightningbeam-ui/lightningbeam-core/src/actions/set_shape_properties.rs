//! Set shape properties action — operates on DCEL edge/face IDs.

use crate::action::Action;
use crate::dcel::{EdgeId, FaceId};
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use uuid::Uuid;

/// Action that sets fill/stroke properties on DCEL elements.
pub struct SetShapePropertiesAction {
    layer_id: Uuid,
    time: f64,
    change: PropertyChange,
    old_edge_values: Vec<(EdgeId, Option<ShapeColor>, Option<f64>)>,
    old_face_values: Vec<(FaceId, Option<ShapeColor>)>,
}

enum PropertyChange {
    FillColor {
        face_ids: Vec<FaceId>,
        color: Option<ShapeColor>,
    },
    StrokeColor {
        edge_ids: Vec<EdgeId>,
        color: Option<ShapeColor>,
    },
    StrokeWidth {
        edge_ids: Vec<EdgeId>,
        width: f64,
    },
}

impl SetShapePropertiesAction {
    pub fn set_fill_color(
        layer_id: Uuid,
        time: f64,
        face_ids: Vec<FaceId>,
        color: Option<ShapeColor>,
    ) -> Self {
        Self {
            layer_id,
            time,
            change: PropertyChange::FillColor { face_ids, color },
            old_edge_values: Vec::new(),
            old_face_values: Vec::new(),
        }
    }

    pub fn set_stroke_color(
        layer_id: Uuid,
        time: f64,
        edge_ids: Vec<EdgeId>,
        color: Option<ShapeColor>,
    ) -> Self {
        Self {
            layer_id,
            time,
            change: PropertyChange::StrokeColor { edge_ids, color },
            old_edge_values: Vec::new(),
            old_face_values: Vec::new(),
        }
    }

    pub fn set_stroke_width(
        layer_id: Uuid,
        time: f64,
        edge_ids: Vec<EdgeId>,
        width: f64,
    ) -> Self {
        Self {
            layer_id,
            time,
            change: PropertyChange::StrokeWidth { edge_ids, width },
            old_edge_values: Vec::new(),
            old_face_values: Vec::new(),
        }
    }

    fn get_dcel_mut<'a>(
        document: &'a mut Document,
        layer_id: &Uuid,
        time: f64,
    ) -> Result<&'a mut crate::dcel::Dcel, String> {
        let layer = document
            .get_layer_mut(layer_id)
            .ok_or_else(|| format!("Layer {} not found", layer_id))?;
        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };
        vl.dcel_at_time_mut(time)
            .ok_or_else(|| format!("No keyframe at time {}", time))
    }
}

impl Action for SetShapePropertiesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel = Self::get_dcel_mut(document, &self.layer_id, self.time)?;

        match &self.change {
            PropertyChange::FillColor { face_ids, color } => {
                self.old_face_values.clear();
                for &fid in face_ids {
                    let face = dcel.face(fid);
                    self.old_face_values.push((fid, face.fill_color));
                    dcel.face_mut(fid).fill_color = *color;
                }
            }
            PropertyChange::StrokeColor { edge_ids, color } => {
                self.old_edge_values.clear();
                for &eid in edge_ids {
                    let edge = dcel.edge(eid);
                    let old_width = edge.stroke_style.as_ref().map(|s| s.width);
                    self.old_edge_values.push((eid, edge.stroke_color, old_width));
                    dcel.edge_mut(eid).stroke_color = *color;
                }
            }
            PropertyChange::StrokeWidth { edge_ids, width } => {
                self.old_edge_values.clear();
                for &eid in edge_ids {
                    let edge = dcel.edge(eid);
                    let old_width = edge.stroke_style.as_ref().map(|s| s.width);
                    self.old_edge_values.push((eid, edge.stroke_color, old_width));
                    if let Some(ref mut style) = dcel.edge_mut(eid).stroke_style {
                        style.width = *width;
                    }
                }
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel = Self::get_dcel_mut(document, &self.layer_id, self.time)?;

        match &self.change {
            PropertyChange::FillColor { .. } => {
                for &(fid, old_color) in &self.old_face_values {
                    dcel.face_mut(fid).fill_color = old_color;
                }
            }
            PropertyChange::StrokeColor { .. } => {
                for &(eid, old_color, _) in &self.old_edge_values {
                    dcel.edge_mut(eid).stroke_color = old_color;
                }
            }
            PropertyChange::StrokeWidth { .. } => {
                for &(eid, _, old_width) in &self.old_edge_values {
                    if let Some(w) = old_width {
                        if let Some(ref mut style) = dcel.edge_mut(eid).stroke_style {
                            style.width = w;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn description(&self) -> String {
        match &self.change {
            PropertyChange::FillColor { .. } => "Set fill color".to_string(),
            PropertyChange::StrokeColor { .. } => "Set stroke color".to_string(),
            PropertyChange::StrokeWidth { .. } => "Set stroke width".to_string(),
        }
    }
}
