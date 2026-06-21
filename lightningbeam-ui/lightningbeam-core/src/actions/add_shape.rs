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
    /// When set, the enclosed region is filled with this image asset (instead of a
    /// solid colour). The renderer prioritises `image_fill` over colour/gradient.
    image_fill: Option<Uuid>,
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
            image_fill: None,
            description_text: "Add shape".to_string(),
            graph_before: None,
        }
    }

    /// A borderless, axis-aligned rectangle filled with an image asset — the result
    /// of importing/dropping an image onto a vector layer.
    pub fn image_rect(
        layer_id: Uuid,
        time: f64,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        asset_id: Uuid,
    ) -> Self {
        let mut path = BezPath::new();
        path.move_to((x, y));
        path.line_to((x + w, y));
        path.line_to((x + w, y + h));
        path.line_to((x, y + h));
        path.close_path();
        Self {
            layer_id,
            time,
            path,
            stroke_style: None, // invisible edges — just the image
            stroke_color: None,
            fill_color: None,
            is_closed: true,
            image_fill: Some(asset_id),
            description_text: "Add image".to_string(),
            graph_before: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description_text = desc.into();
        self
    }

    /// Fill the created region with an image asset (image takes render priority).
    pub fn with_image_fill(mut self, asset_id: Uuid) -> Self {
        self.image_fill = Some(asset_id);
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

            // Apply fill if this is a closed shape with a colour and/or image fill.
            if self.is_closed && (self.fill_color.is_some() || self.image_fill.is_some()) {
                // Compute centroid of the path's bounding box and paint-bucket fill.
                let bbox = self.path.bounding_box();
                let centroid = kurbo::Point::new(
                    (bbox.x0 + bbox.x1) / 2.0,
                    (bbox.y0 + bbox.y1) / 2.0,
                );
                // paint_bucket needs a colour; an image-only fill uses a placeholder
                // that the image overrides (cleared below).
                let color = self.fill_color.clone().unwrap_or_else(|| ShapeColor::rgba(255, 255, 255, 255));
                if let Some(fid) = graph.paint_bucket(centroid, color, FillRule::NonZero, 0.0) {
                    if let Some(asset_id) = self.image_fill {
                        let fill = graph.fill_mut(fid);
                        fill.image_fill = Some(asset_id);
                        if self.fill_color.is_none() {
                            fill.color = None; // image-only: don't double-paint a colour
                        }
                    }
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
