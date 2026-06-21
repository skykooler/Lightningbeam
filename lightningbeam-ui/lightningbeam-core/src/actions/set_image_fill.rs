//! Action that sets or clears the image fill on one or more VectorGraph fills.
//!
//! `image_fill` is an asset id the renderer maps onto the fill's bounding box; it
//! takes priority over colour/gradient. Setting `None` clears it (the colour/gradient
//! underneath shows again).

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::vector_graph::FillId;
use uuid::Uuid;

pub struct SetImageFillAction {
    layer_id: Uuid,
    time: f64,
    fill_ids: Vec<FillId>,
    /// `Some(asset_id)` to set, `None` to clear.
    new_image: Option<Uuid>,
    /// Per-fill previous `image_fill`, for undo.
    old: Vec<(FillId, Option<Uuid>)>,
}

impl SetImageFillAction {
    pub fn new(layer_id: Uuid, time: f64, fill_ids: Vec<FillId>, image: Option<Uuid>) -> Self {
        Self { layer_id, time, fill_ids, new_image: image, old: Vec::new() }
    }

    fn get_graph_mut<'a>(
        document: &'a mut Document,
        layer_id: &Uuid,
        time: f64,
    ) -> Result<&'a mut crate::vector_graph::VectorGraph, String> {
        let layer = document
            .get_layer_mut(layer_id)
            .ok_or_else(|| format!("Layer {} not found", layer_id))?;
        match layer {
            AnyLayer::Vector(vl) => vl
                .graph_at_time_mut(time)
                .ok_or_else(|| format!("No keyframe at time {}", time)),
            _ => Err("Not a vector layer".to_string()),
        }
    }
}

impl Action for SetImageFillAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;
        self.old.clear();
        for &fid in &self.fill_ids {
            self.old.push((fid, graph.fill(fid).image_fill));
            graph.fill_mut(fid).image_fill = self.new_image;
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;
        for &(fid, old) in &self.old {
            graph.fill_mut(fid).image_fill = old;
        }
        Ok(())
    }

    fn description(&self) -> String {
        if self.new_image.is_some() { "Set image fill" } else { "Clear image fill" }.to_string()
    }
}
