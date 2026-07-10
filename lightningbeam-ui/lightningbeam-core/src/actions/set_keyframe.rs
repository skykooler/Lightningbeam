//! Set keyframe action
//!
//! For vector layers: creates a new ShapeKeyframe at the given time by copying
//! shapes from the current keyframe span (with new UUIDs).
//! For clip instances: adds AnimationData keyframes for transform properties.

use crate::action::Action;
use crate::animation::{AnimationCurve, AnimationTarget, Keyframe, TransformProperty};
use crate::document::Document;
use crate::layer::{AnyLayer, ShapeKeyframe};
use crate::object::Transform;
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

/// The clip instance's own value for a property (its base transform / opacity).
fn transform_prop_value(t: &Transform, opacity: f64, prop: &TransformProperty) -> f64 {
    match prop {
        TransformProperty::X => t.x,
        TransformProperty::Y => t.y,
        TransformProperty::Rotation => t.rotation,
        TransformProperty::ScaleX => t.scale_x,
        TransformProperty::ScaleY => t.scale_y,
        TransformProperty::SkewX => t.skew_x,
        TransformProperty::SkewY => t.skew_y,
        TransformProperty::Opacity => opacity,
    }
}

impl Action for SetKeyframeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.clip_undo_entries.clear();
        self.shape_keyframe_created = false;

        // Phase 1 (immutable): for each clip instance, gather its base transform and the
        // start time of its visibility region, so a brand-new curve can be anchored there.
        let mut clip_info: std::collections::HashMap<Uuid, (Transform, f64, f64)> =
            std::collections::HashMap::new(); // id -> (base transform, opacity, start time)
        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&self.layer_id) {
            for clip_id in &self.clip_instance_ids {
                if let Some(ci) = vl.clip_instances.iter().find(|c| c.id == *clip_id) {
                    // `start` is a keyframe time in seconds; group_visibility_start returns seconds,
                    // so the fallback must convert the clip's beats start to seconds too.
                    let start = vl
                        .group_visibility_start(clip_id, self.time)
                        .unwrap_or_else(|| document.tempo_map().beats_to_seconds(ci.timeline_start).seconds_to_f64());
                    clip_info.insert(*clip_id, (ci.transform.clone(), ci.opacity, start));
                }
            }
        }

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
                let (base_transform, base_opacity, start) = clip_info
                    .get(clip_id)
                    .cloned()
                    .unwrap_or((Transform::new(), 1.0, 0.0));
                for prop in TRANSFORM_PROPERTIES {
                    let target = AnimationTarget::Object {
                        id: *clip_id,
                        property: *prop,
                    };
                    // Fall back to the clip's OWN value (not a generic default) so a brand-new
                    // keyframe captures the actual on-stage position, not (0,0)/identity.
                    let base = transform_prop_value(&base_transform, base_opacity, prop);
                    let value = vl.layer.animation_data.eval(&target, self.time, base);

                    let curve_created = vl.layer.animation_data.get_curve(&target).is_none();
                    if curve_created {
                        vl.layer
                            .animation_data
                            .set_curve(AnimationCurve::new(target.clone(), base));
                    }

                    let curve = vl.layer.animation_data.get_curve_mut(&target).unwrap();
                    let old_keyframe = curve.get_keyframe_at(self.time, 0.001).cloned();

                    // When this is the first keyframe of the curve and the clip already existed
                    // before `time`, anchor a keyframe at its start with the original value.
                    // Otherwise a single keyframe would Hold-extrapolate backward and move the
                    // clip on every earlier frame too (the motion-tween first-keyframe bug).
                    if curve_created && start < self.time - 0.001 {
                        curve.set_keyframe(Keyframe::linear(start, base));
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::TransformClipInstancesAction;
    use crate::clip::ClipInstance;
    use crate::layer::VectorLayer;
    use std::collections::HashMap;

    fn x_curve_eval(document: &Document, layer_id: Uuid, instance_id: Uuid, time: f64) -> f64 {
        let target = AnimationTarget::Object { id: instance_id, property: TransformProperty::X };
        match document.get_layer(&layer_id) {
            Some(AnyLayer::Vector(vl)) => vl.layer.animation_data.eval(&target, time, f64::NAN),
            _ => panic!("no layer"),
        }
    }

    #[test]
    fn first_keyframe_then_move_does_not_disturb_earlier_frames() {
        // Group created at frame 0 (clip at x=50), keyframe + move at frame 10 → x=200.
        // Frame 0 must keep x=50 (the motion-tween first-keyframe bug: it used to become 200).
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Layer");
        let clip_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform = Transform::with_position(50.0, 50.0);
        layer.clip_instances.push(instance);
        // The group's visibility starts at a keyframe at time 0 containing the instance.
        layer.ensure_keyframe_at(0.0).clip_instance_ids.push(instance_id);
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create a keyframe at frame 10.
        SetKeyframeAction::new(layer_id, 10.0, vec![instance_id])
            .execute(&mut document)
            .unwrap();

        // The new curve must be anchored at the start (two keyframes, both at x=50 so far).
        assert!((x_curve_eval(&document, layer_id, instance_id, 0.0) - 50.0).abs() < 1e-6);
        assert!((x_curve_eval(&document, layer_id, instance_id, 10.0) - 50.0).abs() < 1e-6);

        // Move the clip at frame 10 to x=200.
        let mut transforms = HashMap::new();
        transforms.insert(
            instance_id,
            (Transform::with_position(50.0, 50.0), Transform::with_position(200.0, 200.0)),
        );
        TransformClipInstancesAction::new(layer_id, 10.0, transforms)
            .execute(&mut document)
            .unwrap();

        // Frame 0 unchanged; frame 10 moved; midpoint tweens.
        assert!((x_curve_eval(&document, layer_id, instance_id, 0.0) - 50.0).abs() < 1e-6, "frame 0 must stay 50");
        assert!((x_curve_eval(&document, layer_id, instance_id, 10.0) - 200.0).abs() < 1e-6, "frame 10 must be 200");
        assert!((x_curve_eval(&document, layer_id, instance_id, 5.0) - 125.0).abs() < 1e-6, "midpoint tweens");
    }

    #[test]
    fn first_keyframe_at_clip_start_is_not_double_anchored() {
        // When the keyframe is created at the clip's own start, there's nothing earlier to
        // anchor — a single keyframe is correct.
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Layer");
        let clip_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform = Transform::with_position(10.0, 0.0);
        layer.clip_instances.push(instance);
        layer.ensure_keyframe_at(0.0).clip_instance_ids.push(instance_id);
        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        SetKeyframeAction::new(layer_id, 0.0, vec![instance_id])
            .execute(&mut document)
            .unwrap();

        let target = AnimationTarget::Object { id: instance_id, property: TransformProperty::X };
        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            let curve = vl.layer.animation_data.get_curve(&target).unwrap();
            assert_eq!(curve.keyframes.len(), 1, "keyframe at clip start needs no anchor");
            assert!((curve.eval(0.0) - 10.0).abs() < 1e-6);
        }
    }
}
