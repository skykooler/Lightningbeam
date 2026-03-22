//! Action that changes the fill of one or more VectorGraph fills.
//!
//! Handles both solid-colour and gradient fills, clearing the other type so they
//! don't coexist on a fill.

use crate::action::Action;
use crate::vector_graph::FillId;
use crate::document::Document;
use crate::gradient::ShapeGradient;
use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use uuid::Uuid;

/// Snapshot of one fill's state (both types) for undo.
#[derive(Clone)]
struct OldFill {
    fill_id:  FillId,
    color:    Option<ShapeColor>,
    gradient: Option<ShapeGradient>,
}

/// Action that sets a solid-colour *or* gradient fill on a set of fills,
/// clearing the other fill type.
pub struct SetFillPaintAction {
    layer_id:     Uuid,
    time:         f64,
    fill_ids:     Vec<FillId>,
    new_color:    Option<ShapeColor>,
    new_gradient: Option<ShapeGradient>,
    old_fills:    Vec<OldFill>,
    description:  &'static str,
}

impl SetFillPaintAction {
    /// Set a solid fill (clears any gradient on the same fills).
    pub fn solid(
        layer_id: Uuid,
        time: f64,
        fill_ids: Vec<FillId>,
        color: Option<ShapeColor>,
    ) -> Self {
        Self {
            layer_id,
            time,
            fill_ids,
            new_color:    color,
            new_gradient: None,
            old_fills:    Vec::new(),
            description:  "Set fill colour",
        }
    }

    /// Set a gradient fill (clears any solid colour on the same fills).
    pub fn gradient(
        layer_id: Uuid,
        time: f64,
        fill_ids: Vec<FillId>,
        gradient: Option<ShapeGradient>,
    ) -> Self {
        Self {
            layer_id,
            time,
            fill_ids,
            new_color:    None,
            new_gradient: gradient,
            old_fills:    Vec::new(),
            description:  "Set gradient fill",
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
        match layer {
            AnyLayer::Vector(vl) => vl
                .graph_at_time_mut(time)
                .ok_or_else(|| format!("No keyframe at time {}", time)),
            _ => Err("Not a vector layer".to_string()),
        }
    }
}

impl Action for SetFillPaintAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;
        self.old_fills.clear();

        for &fid in &self.fill_ids {
            let fill = graph.fill(fid);
            self.old_fills.push(OldFill {
                fill_id:  fid,
                color:    fill.color,
                gradient: fill.gradient_fill.clone(),
            });

            let fill_mut = graph.fill_mut(fid);
            // Setting a gradient clears solid colour and vice-versa.
            if self.new_gradient.is_some() || self.new_color.is_none() {
                fill_mut.color         = self.new_color;
                fill_mut.gradient_fill = self.new_gradient.clone();
            } else {
                fill_mut.color         = self.new_color;
                fill_mut.gradient_fill = None;
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let graph = Self::get_graph_mut(document, &self.layer_id, self.time)?;
        for old in &self.old_fills {
            let fill = graph.fill_mut(old.fill_id);
            fill.color         = old.color;
            fill.gradient_fill = old.gradient.clone();
        }
        Ok(())
    }

    fn description(&self) -> String {
        self.description.to_string()
    }
}
