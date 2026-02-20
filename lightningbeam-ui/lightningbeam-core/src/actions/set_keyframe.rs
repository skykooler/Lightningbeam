//! Set keyframe action
//!
//! For vector layers: creates a new ShapeKeyframe at the given time by copying
//! shapes from the current keyframe span (with new UUIDs).
//! For clip instances: adds AnimationData keyframes for transform properties.

use crate::action::Action;
use crate::animation::{AnimationCurve, AnimationTarget, Keyframe, TransformProperty};
use crate::document::Document;
use crate::layer::{AnyLayer, ShapeKeyframe};
use uuid::Uuid;

/// Undo info for a clip animation curve
struct ClipUndoEntry {
    target: AnimationTarget,
    old_keyframe: Option<Keyframe>,
    curve_created: bool,
}

pub struct SetKeyframeAction {
    layer_id: Uuid,
    time: f64,
    /// Clip instance IDs to keyframe (motion tweens)
    clip_instance_ids: Vec<Uuid>,
    /// Whether a shape keyframe was created by this action
    shape_keyframe_created: bool,
    /// The removed keyframe for rollback (if we created one)
    removed_keyframe: Option<ShapeKeyframe>,
    /// Clip animation undo entries
    clip_undo_entries: Vec<ClipUndoEntry>,
}

impl SetKeyframeAction {
    pub fn new(layer_id: Uuid, time: f64, clip_instance_ids: Vec<Uuid>) -> Self {
        Self {
            layer_id,
            time,
            clip_instance_ids,
            shape_keyframe_created: false,
            removed_keyframe: None,
            clip_undo_entries: Vec::new(),
        }
    }
}

const TRANSFORM_PROPERTIES: &[TransformProperty] = &[
    TransformProperty::X,
    TransformProperty::Y,
    TransformProperty::Rotation,
    TransformProperty::ScaleX,
    TransformProperty::ScaleY,
    TransformProperty::SkewX,
    TransformProperty::SkewY,
    TransformProperty::Opacity,
];

fn transform_default(prop: &TransformProperty) -> f64 {
    match prop {
        TransformProperty::ScaleX | TransformProperty::ScaleY => 1.0,
        TransformProperty::Opacity => 1.0,
        _ => 0.0,
    }
}

impl Action for SetKeyframeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.clip_undo_entries.clear();
        self.shape_keyframe_created = false;

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // For vector layers: create a shape keyframe
        if let AnyLayer::Vector(vl) = layer {
            // Check if a keyframe already exists at this exact time
            let already_exists = vl.keyframe_index_at_exact(self.time, 0.001).is_some();
            if !already_exists {
                vl.insert_keyframe_from_current(self.time);
                self.shape_keyframe_created = true;
            }

            // Add clip animation keyframes
            for clip_id in &self.clip_instance_ids {
                for prop in TRANSFORM_PROPERTIES {
                    let target = AnimationTarget::Object {
                        id: *clip_id,
                        property: *prop,
                    };
                    let default = transform_default(prop);
                    let value = vl.layer.animation_data.eval(&target, self.time, default);

                    let curve_created = vl.layer.animation_data.get_curve(&target).is_none();
                    if curve_created {
                        vl.layer
                            .animation_data
                            .set_curve(AnimationCurve::new(target.clone(), default));
                    }

                    let curve = vl.layer.animation_data.get_curve_mut(&target).unwrap();
                    let old_keyframe = curve.get_keyframe_at(self.time, 0.001).cloned();
                    curve.set_keyframe(Keyframe::linear(self.time, value));

                    self.clip_undo_entries.push(ClipUndoEntry {
                        target,
                        old_keyframe,
                        curve_created,
                    });
                }
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            // Undo clip animation keyframes in reverse order
            for entry in self.clip_undo_entries.drain(..).rev() {
                if entry.curve_created {
                    vl.layer.animation_data.remove_curve(&entry.target);
                } else if let Some(curve) = vl.layer.animation_data.get_curve_mut(&entry.target) {
                    curve.remove_keyframe(self.time, 0.001);
                    if let Some(old_kf) = entry.old_keyframe {
                        curve.set_keyframe(old_kf);
                    }
                }
            }

            // Remove the shape keyframe if we created one
            if self.shape_keyframe_created {
                self.removed_keyframe = vl.remove_keyframe_at(self.time, 0.001);
                self.shape_keyframe_created = false;
            }
        }

        Ok(())
    }

    fn description(&self) -> String {
        "New keyframe".to_string()
    }
}
