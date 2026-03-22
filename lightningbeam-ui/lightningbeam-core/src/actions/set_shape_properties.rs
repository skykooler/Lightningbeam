//! Set shape properties action — operates on VectorGraph edge/fill IDs.

use crate::action::Action;
use crate::vector_graph::{EdgeId, FillId};
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use uuid::Uuid;

/// Action that sets fill/stroke properties on VectorGraph elements.
pub struct SetShapePropertiesAction {
    layer_id: Uuid,
    time: f64,
    change: PropertyChange,
    old_edge_values: Vec<(EdgeId, Option<ShapeColor>, Option<f64>)>,
    old_fill_values: Vec<(FillId, Option<ShapeColor>)>,
}

enum PropertyChange {
    FillColor {
        fill_ids: Vec<FillId>,
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
        fill_ids: Vec<FillId>,
        color: Option<ShapeColor>,
    ) -> Self {
        Self {
            layer_id,
            time,
            change: PropertyChange::FillColor { fill_ids, color },
            old_edge_values: Vec::new(),
            old_fill_values: Vec::new(),
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
            old_fill_values: Vec::new(),
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
            old_fill_values: Vec::new(),
        }
    }

    fn get_graph_mut<'a>(
        document: &'a mut Document,
        layer_id: &Uuid,
        time: f64,
    ) -> Result<&'a mut crate::vector_graph::VectorGraph, String> {
        let layer = document
            .get_layer_mut(layer_id)
            .ok_or_else(|| format!("Layer {} not found", layer_id))?;
        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };
        vl.graph_at_time_mut(time)
            .ok_or_else(|| format!("No keyframe at time {}", time))
    }
}

impl Action for SetShapePropertiesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;

        match &self.change {
            PropertyChange::FillColor { fill_ids, color } => {
                self.old_fill_values.clear();
                for &fid in fill_ids {
                    let fill = graph.fill(fid);
                    self.old_fill_values.push((fid, fill.color));
                    graph.fill_mut(fid).color = *color;
                }
            }
            PropertyChange::StrokeColor { edge_ids, color } => {
                self.old_edge_values.clear();
                for &eid in edge_ids {
                    let edge = graph.edge(eid);
                    let old_width = edge.stroke_style.as_ref().map(|s| s.width);
                    self.old_edge_values.push((eid, edge.stroke_color, old_width));
                    graph.edge_mut(eid).stroke_color = *color;
                }
            }
            PropertyChange::StrokeWidth { edge_ids, width } => {
                self.old_edge_values.clear();
                for &eid in edge_ids {
                    let edge = graph.edge(eid);
                    let old_width = edge.stroke_style.as_ref().map(|s| s.width);
                    self.old_edge_values.push((eid, edge.stroke_color, old_width));
                    if let Some(ref mut style) = graph.edge_mut(eid).stroke_style {
                        style.width = *width;
                    }
                }
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;

        match &self.change {
            PropertyChange::FillColor { .. } => {
                for &(fid, old_color) in &self.old_fill_values {
                    graph.fill_mut(fid).color = old_color;
                }
            }
            PropertyChange::StrokeColor { .. } => {
                for &(eid, old_color, _) in &self.old_edge_values {
                    graph.edge_mut(eid).stroke_color = old_color;
                }
            }
            PropertyChange::StrokeWidth { .. } => {
                for &(eid, _, old_width) in &self.old_edge_values {
                    if let Some(w) = old_width {
                        if let Some(ref mut style) = graph.edge_mut(eid).stroke_style {
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
