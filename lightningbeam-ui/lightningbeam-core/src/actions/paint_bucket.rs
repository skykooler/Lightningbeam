//! Paint bucket fill action — creates a fill region in a VectorGraph.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::{FillRule, ShapeColor};
use crate::vector_graph::FillId;
use uuid::Uuid;
use vello::kurbo::Point;

/// Action that performs a paint bucket fill on a VectorGraph region.
pub struct PaintBucketAction {
    layer_id: Uuid,
    time: f64,
    click_point: Point,
    fill_color: ShapeColor,
    /// The fill that was created (resolved during execute)
    hit_fill: Option<FillId>,
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
            hit_fill: None,
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
        let graph = &mut keyframe.graph;

        let fill_id = graph
            .paint_bucket(self.click_point, self.fill_color.clone(), FillRule::NonZero, 2.0)
            .ok_or("No fillable region at click point")?;

        self.hit_fill = Some(fill_id);

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let fill_id = self.hit_fill.ok_or("No fill to undo")?;

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        let keyframe = vl.ensure_keyframe_at(self.time);
        let graph = &mut keyframe.graph;

        graph.free_fill(fill_id);

        Ok(())
    }

    fn description(&self) -> String {
        "Paint bucket fill".to_string()
    }
}
