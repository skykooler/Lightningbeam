//! Transform clip instances action
//!
//! Handles spatial transformation (move, scale, rotate) of clip instances on the stage.
//! Updates both the clip instance's transform and the animation keyframe at the current time.

use crate::action::Action;
use crate::animation::{AnimationTarget, Keyframe, TransformProperty};
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that transforms clip instances spatially on the stage
pub struct TransformClipInstancesAction {
    layer_id: Uuid,
    /// Current time for animation keyframe update
    time: f64,
    /// Map of clip instance ID to (old transform, new transform)
    clip_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
}

impl TransformClipInstancesAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        clip_instance_transforms: HashMap<Uuid, (Transform, Transform)>,
    ) -> Self {
        Self {
            layer_id,
            time,
            clip_instance_transforms,
        }
    }
}

/// Update animation keyframes for a clip instance's transform properties at the given time.
/// If a curve exists for a property, updates the keyframe at that time. If no curve exists, does nothing.
fn update_animation_keyframes(
    animation_data: &mut crate::animation::AnimationData,
    instance_id: Uuid,
    transform: &Transform,
    opacity: f64,
    time: f64,
) {
    let props_and_values = [
        (TransformProperty::X, transform.x),
        (TransformProperty::Y, transform.y),
        (TransformProperty::Rotation, transform.rotation),
        (TransformProperty::ScaleX, transform.scale_x),
        (TransformProperty::ScaleY, transform.scale_y),
        (TransformProperty::SkewX, transform.skew_x),
        (TransformProperty::SkewY, transform.skew_y),
        (TransformProperty::Opacity, opacity),
    ];

    for (prop, value) in props_and_values {
        let target = AnimationTarget::Object {
            id: instance_id,
            property: prop,
        };
        if let Some(curve) = animation_data.get_curve_mut(&target) {
            curve.set_keyframe(Keyframe::linear(time, value));
        }
    }
}

impl Action for TransformClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        match layer {
            AnyLayer::Vector(vl) => {
                for (clip_id, (_old, new)) in &self.clip_instance_transforms {
                    if let Some(clip_instance) = vl.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        let opacity = clip_instance.opacity;
                        clip_instance.transform = new.clone();
                        update_animation_keyframes(
                            &mut vl.layer.animation_data, *clip_id, new, opacity, self.time,
                        );
                    }
                }
            }
            AnyLayer::Audio(al) => {
                for (clip_id, (_old, new)) in &self.clip_instance_transforms {
                    if let Some(ci) = al.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        ci.transform = new.clone();
                    }
                }
            }
            AnyLayer::Video(vl) => {
                for (clip_id, (_old, new)) in &self.clip_instance_transforms {
                    if let Some(ci) = vl.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        ci.transform = new.clone();
                    }
                }
            }
            AnyLayer::Effect(_) => {}
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = match document.get_layer_mut(&self.layer_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        match layer {
            AnyLayer::Vector(vl) => {
                for (clip_id, (old, _new)) in &self.clip_instance_transforms {
                    if let Some(clip_instance) = vl.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        let opacity = clip_instance.opacity;
                        clip_instance.transform = old.clone();
                        update_animation_keyframes(
                            &mut vl.layer.animation_data, *clip_id, old, opacity, self.time,
                        );
                    }
                }
            }
            AnyLayer::Audio(al) => {
                for (clip_id, (old, _new)) in &self.clip_instance_transforms {
                    if let Some(ci) = al.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        ci.transform = old.clone();
                    }
                }
            }
            AnyLayer::Video(vl) => {
                for (clip_id, (old, _new)) in &self.clip_instance_transforms {
                    if let Some(ci) = vl.clip_instances.iter_mut().find(|ci| ci.id == *clip_id) {
                        ci.transform = old.clone();
                    }
                }
            }
            AnyLayer::Effect(_) => {}
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!(
            "Transform {} clip instance(s)",
            self.clip_instance_transforms.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::ClipInstance;
    use crate::layer::{AudioLayer, VectorLayer, VideoLayer};

    #[test]
    fn test_transform_clip_instance_on_vector_layer() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let clip_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform = Transform::with_position(10.0, 20.0);
        layer.clip_instances.push(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let old_transform = Transform::with_position(10.0, 20.0);
        let new_transform = Transform::with_position(100.0, 200.0);
        let mut transforms = HashMap::new();
        transforms.insert(instance_id, (old_transform, new_transform));

        let mut action = TransformClipInstancesAction::new(layer_id, 0.0, transforms);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let inst = vl.clip_instances.iter().find(|ci| ci.id == instance_id).unwrap();
            assert_eq!(inst.transform.x, 100.0);
            assert_eq!(inst.transform.y, 200.0);
        } else {
            panic!("Layer not found");
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let inst = vl.clip_instances.iter().find(|ci| ci.id == instance_id).unwrap();
            assert_eq!(inst.transform.x, 10.0);
            assert_eq!(inst.transform.y, 20.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_clip_instance_on_audio_layer() {
        let mut document = Document::new("Test");
        let mut layer = AudioLayer::new("Audio Layer");

        let clip_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform = Transform::with_position(0.0, 0.0);
        layer.clip_instances.push(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Audio(layer));

        let old_transform = Transform::with_position(0.0, 0.0);
        let new_transform = Transform::with_position(50.0, 75.0);
        let mut transforms = HashMap::new();
        transforms.insert(instance_id, (old_transform, new_transform));

        let mut action = TransformClipInstancesAction::new(layer_id, 0.0, transforms);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Audio(al)) = document.get_layer_mut(&layer_id) {
            let inst = al.clip_instances.iter().find(|ci| ci.id == instance_id).unwrap();
            assert_eq!(inst.transform.x, 50.0);
            assert_eq!(inst.transform.y, 75.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_clip_instance_on_video_layer() {
        let mut document = Document::new("Test");
        let mut layer = VideoLayer::new("Video Layer");

        let clip_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform.rotation = 0.0;
        instance.transform.scale_x = 1.0;
        layer.clip_instances.push(instance);

        let layer_id = document.root_mut().add_child(AnyLayer::Video(layer));

        let mut old_transform = Transform::new();
        old_transform.rotation = 0.0;
        old_transform.scale_x = 1.0;

        let mut new_transform = Transform::new();
        new_transform.rotation = 45.0;
        new_transform.scale_x = 2.0;
        new_transform.scale_y = 2.0;

        let mut transforms = HashMap::new();
        transforms.insert(instance_id, (old_transform, new_transform));

        let mut action = TransformClipInstancesAction::new(layer_id, 0.0, transforms);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Video(vl)) = document.get_layer_mut(&layer_id) {
            let inst = vl.clip_instances.iter().find(|ci| ci.id == instance_id).unwrap();
            assert_eq!(inst.transform.rotation, 45.0);
            assert_eq!(inst.transform.scale_x, 2.0);
            assert_eq!(inst.transform.scale_y, 2.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_transform_multiple_clip_instances() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let clip_id = Uuid::new_v4();
        let instance1_id = Uuid::new_v4();
        let instance2_id = Uuid::new_v4();

        let mut instance1 = ClipInstance::with_id(instance1_id, clip_id);
        instance1.transform = Transform::with_position(0.0, 0.0);

        let mut instance2 = ClipInstance::with_id(instance2_id, clip_id);
        instance2.transform = Transform::with_position(100.0, 100.0);

        layer.clip_instances.push(instance1);
        layer.clip_instances.push(instance2);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let mut transforms = HashMap::new();
        transforms.insert(
            instance1_id,
            (Transform::with_position(0.0, 0.0), Transform::with_position(50.0, 50.0)),
        );
        transforms.insert(
            instance2_id,
            (Transform::with_position(100.0, 100.0), Transform::with_position(150.0, 150.0)),
        );

        let mut action = TransformClipInstancesAction::new(layer_id, 0.0, transforms);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let inst1 = vl.clip_instances.iter().find(|ci| ci.id == instance1_id).unwrap();
            assert_eq!(inst1.transform.x, 50.0);
            assert_eq!(inst1.transform.y, 50.0);

            let inst2 = vl.clip_instances.iter().find(|ci| ci.id == instance2_id).unwrap();
            assert_eq!(inst2.transform.x, 150.0);
            assert_eq!(inst2.transform.y, 150.0);
        } else {
            panic!("Layer not found");
        }

        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&layer_id) {
            let inst1 = vl.clip_instances.iter().find(|ci| ci.id == instance1_id).unwrap();
            assert_eq!(inst1.transform.x, 0.0);
            assert_eq!(inst1.transform.y, 0.0);

            let inst2 = vl.clip_instances.iter().find(|ci| ci.id == instance2_id).unwrap();
            assert_eq!(inst2.transform.x, 100.0);
            assert_eq!(inst2.transform.y, 100.0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_description() {
        let layer_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();

        let mut transforms = HashMap::new();
        transforms.insert(
            instance_id,
            (Transform::new(), Transform::with_position(10.0, 10.0)),
        );

        let action = TransformClipInstancesAction::new(layer_id, 0.0, transforms);
        assert_eq!(action.description(), "Transform 1 clip instance(s)");
    }
}
