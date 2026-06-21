//! Set the tween type on the keyframe at-or-before a time (e.g. "Add Shape Tween").
//!
//! The keyframe's `tween_after` controls how the span between it and the next keyframe is
//! rendered: `None` holds, `Shape` morphs the geometry (when the two keyframes share
//! topology — otherwise rendering falls back to holding).

use crate::action::Action;
use crate::document::Document;
use crate::layer::{AnyLayer, TweenType};
use uuid::Uuid;

pub struct SetTweenAction {
    layer_id: Uuid,
    time: f64,
    new_tween: TweenType,
    old_tween: Option<TweenType>,
}

impl SetTweenAction {
    pub fn new(layer_id: Uuid, time: f64, new_tween: TweenType) -> Self {
        Self { layer_id, time, new_tween, old_tween: None }
    }
}

impl Action for SetTweenAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&self.layer_id) {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                self.old_tween = Some(kf.tween_after);
                kf.tween_after = self.new_tween;
            } else {
                return Err("No keyframe at-or-before this time".to_string());
            }
        } else {
            return Err("Not a vector layer".to_string());
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let (Some(old), Some(AnyLayer::Vector(vl))) =
            (self.old_tween, document.get_layer_mut(&self.layer_id))
        {
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.tween_after = old;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        match self.new_tween {
            TweenType::Shape => "Add shape tween".to_string(),
            TweenType::None => "Remove tween".to_string(),
        }
    }
}
