//! Action that changes the fill of one or more DCEL faces.
//!
//! Handles both solid-colour and gradient fills, clearing the other type so they
//! don't coexist on a face.

use crate::action::Action;
use crate::dcel::FaceId;
use crate::document::Document;
use crate::gradient::ShapeGradient;
use crate::layer::AnyLayer;
use crate::shape::ShapeColor;
use uuid::Uuid;

/// Snapshot of one face's fill state (both types) for undo.
#[derive(Clone)]
struct OldFill {
    face_id:  FaceId,
    color:    Option<ShapeColor>,
    gradient: Option<ShapeGradient>,
}

/// Action that sets a solid-colour *or* gradient fill on a set of faces,
/// clearing the other fill type.
pub struct SetFillPaintAction {
    layer_id:     Uuid,
    time:         f64,
    face_ids:     Vec<FaceId>,
    new_color:    Option<ShapeColor>,
    new_gradient: Option<ShapeGradient>,
    old_fills:    Vec<OldFill>,
    description:  &'static str,
}

impl SetFillPaintAction {
    /// Set a solid fill (clears any gradient on the same faces).
    pub fn solid(
        layer_id: Uuid,
        time: f64,
        face_ids: Vec<FaceId>,
        color: Option<ShapeColor>,
    ) -> Self {
        Self {
            layer_id,
            time,
            face_ids,
            new_color:    color,
            new_gradient: None,
            old_fills:    Vec::new(),
            description:  "Set fill colour",
        }
    }

    /// Set a gradient fill (clears any solid colour on the same faces).
    pub fn gradient(
        layer_id: Uuid,
        time: f64,
        face_ids: Vec<FaceId>,
        gradient: Option<ShapeGradient>,
    ) -> Self {
        Self {
            layer_id,
            time,
            face_ids,
            new_color:    None,
            new_gradient: gradient,
            old_fills:    Vec::new(),
            description:  "Set gradient fill",
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
        match layer {
            AnyLayer::Vector(vl) => vl
                .dcel_at_time_mut(time)
                .ok_or_else(|| format!("No keyframe at time {}", time)),
            _ => Err("Not a vector layer".to_string()),
        }
    }
}

impl Action for SetFillPaintAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel = Self::get_dcel_mut(document, &self.layer_id, self.time)?;
        self.old_fills.clear();

        for &fid in &self.face_ids {
            let face = dcel.face(fid);
            self.old_fills.push(OldFill {
                face_id:  fid,
                color:    face.fill_color,
                gradient: face.gradient_fill.clone(),
            });

            let face_mut = dcel.face_mut(fid);
            // Setting a gradient clears solid colour and vice-versa.
            if self.new_gradient.is_some() || self.new_color.is_none() {
                face_mut.fill_color    = self.new_color;
                face_mut.gradient_fill = self.new_gradient.clone();
            } else {
                face_mut.fill_color    = self.new_color;
                face_mut.gradient_fill = None;
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let dcel = Self::get_dcel_mut(document, &self.layer_id, self.time)?;
        for old in &self.old_fills {
            let face = dcel.face_mut(old.face_id);
            face.fill_color    = old.color;
            face.gradient_fill = old.gradient.clone();
        }
        Ok(())
    }

    fn description(&self) -> String {
        self.description.to_string()
    }
}
