//! Add shape action — inserts strokes into the VectorGraph.
//!
//! Converts a BezPath into cubic segments and inserts them via
//! `VectorGraph::insert_stroke()`. Undo is handled by snapshotting the graph.

use crate::action::Action;
use crate::vector_graph::bezpath_to_cubic_segments;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::{FillRule, ShapeColor, StrokeStyle};
use crate::vector_graph::VectorGraph;
use kurbo::{BezPath, Shape as _};
use uuid::Uuid;

const DEFAULT_SNAP_EPSILON: f64 = 0.5;

/// Action that inserts a drawn path into a vector layer's VectorGraph keyframe.
pub struct AddShapeAction {
    layer_id: Uuid,
    time: f64,
    path: BezPath,
    stroke_style: Option<StrokeStyle>,
    stroke_color: Option<ShapeColor>,
    fill_color: Option<ShapeColor>,
    is_closed: bool,
    description_text: String,
    /// Snapshot of the graph before insertion (for undo).
    graph_before: Option<VectorGraph>,
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
            graph_before: None,
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
        let graph = &mut keyframe.graph;

        // Snapshot for undo
        self.graph_before = Some(graph.clone());

        let subpaths = bezpath_to_cubic_segments(&self.path);

        for segments in &subpaths {
            if segments.is_empty() {
                continue;
            }
            let _new_edges = graph.insert_stroke(
                segments,
                self.stroke_style.clone(),
                self.stroke_color.clone(),
                DEFAULT_SNAP_EPSILON,
            );

            // Apply fill if this is a closed shape with fill
            if self.is_closed {
                if let Some(ref fill) = self.fill_color {
                    // Compute centroid of the path's bounding box and paint-bucket fill
                    let bbox = self.path.bounding_box();
                    let centroid = kurbo::Point::new(
                        (bbox.x0 + bbox.x1) / 2.0,
                        (bbox.y0 + bbox.y1) / 2.0,
                    );
                    graph.paint_bucket(centroid, fill.clone(), FillRule::NonZero, 0.0);
                }
            }
        }

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
        keyframe.graph = self
            .graph_before
            .take()
            .ok_or_else(|| "No graph snapshot for undo".to_string())?;

        Ok(())
    }

    fn description(&self) -> String {
        self.description_text.clone()
    }
}
