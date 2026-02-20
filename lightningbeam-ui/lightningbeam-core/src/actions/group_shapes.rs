//! Group action
//!
//! Groups selected shapes and/or clip instances into a new VectorClip
//! with a ClipInstance placed on the layer. Supports grouping shapes,
//! existing clip instances (groups), or a mix of both.

use crate::action::Action;
use crate::animation::{AnimationCurve, AnimationTarget, Keyframe, TransformProperty};
use crate::clip::{ClipInstance, VectorClip};
use crate::document::Document;
use crate::layer::{AnyLayer, VectorLayer};
use crate::shape::Shape;
use uuid::Uuid;
use vello::kurbo::{Rect, Shape as KurboShape};

/// Action that groups selected shapes and/or clip instances into a VectorClip
pub struct GroupAction {
    /// Layer containing the items to group
    layer_id: Uuid,

    /// Time of the keyframe to operate on (for shape lookup)
    time: f64,

    /// Shape IDs to include in the group
    shape_ids: Vec<Uuid>,

    /// Clip instance IDs to include in the group
    clip_instance_ids: Vec<Uuid>,

    /// Pre-generated clip instance ID for the new group (so caller can update selection)
    instance_id: Uuid,

    /// Created clip ID (for rollback)
    created_clip_id: Option<Uuid>,

    /// Shapes removed from the keyframe (for rollback)
    removed_shapes: Vec<Shape>,

    /// Clip instances removed from the layer (for rollback, preserving original order)
    removed_clip_instances: Vec<ClipInstance>,
}

impl GroupAction {
    pub fn new(
        layer_id: Uuid,
        time: f64,
        shape_ids: Vec<Uuid>,
        clip_instance_ids: Vec<Uuid>,
        instance_id: Uuid,
    ) -> Self {
        Self {
            layer_id,
            time,
            shape_ids,
            clip_instance_ids,
            instance_id,
            created_clip_id: None,
            removed_shapes: Vec::new(),
            removed_clip_instances: Vec::new(),
        }
    }
}

impl Action for GroupAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // --- Phase 1: Collect items and compute bounding box ---

        let layer = document
            .get_layer(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Group is only supported on vector layers".to_string()),
        };

        // Collect shapes
        let shapes_at_time = vl.shapes_at_time(self.time);
        let mut group_shapes: Vec<Shape> = Vec::new();
        for id in &self.shape_ids {
            if let Some(shape) = shapes_at_time.iter().find(|s| &s.id == id) {
                group_shapes.push(shape.clone());
            }
        }

        // Collect clip instances
        let mut group_clip_instances: Vec<ClipInstance> = Vec::new();
        for id in &self.clip_instance_ids {
            if let Some(ci) = vl.clip_instances.iter().find(|ci| &ci.id == id) {
                group_clip_instances.push(ci.clone());
            }
        }

        let total_items = group_shapes.len() + group_clip_instances.len();
        if total_items < 2 {
            return Err("Need at least 2 items to group".to_string());
        }

        // Compute combined bounding box in parent (layer) space
        let mut combined_bbox: Option<Rect> = None;

        // Shape bounding boxes
        for shape in &group_shapes {
            let local_bbox = shape.path().bounding_box();
            let transform = shape.transform.to_affine();
            let transformed_bbox = transform.transform_rect_bbox(local_bbox);
            combined_bbox = Some(match combined_bbox {
                Some(existing) => existing.union(transformed_bbox),
                None => transformed_bbox,
            });
        }

        // Clip instance bounding boxes
        for ci in &group_clip_instances {
            let content_bounds = if let Some(vector_clip) = document.get_vector_clip(&ci.clip_id) {
                let clip_time = ((self.time - ci.timeline_start) * ci.playback_speed) + ci.trim_start;
                vector_clip.calculate_content_bounds(document, clip_time)
            } else if let Some(video_clip) = document.get_video_clip(&ci.clip_id) {
                Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
            } else {
                continue;
            };
            let ci_transform = ci.transform.to_affine();
            let transformed_bbox = ci_transform.transform_rect_bbox(content_bounds);
            combined_bbox = Some(match combined_bbox {
                Some(existing) => existing.union(transformed_bbox),
                None => transformed_bbox,
            });
        }

        let bbox = combined_bbox.ok_or("Could not compute bounding box")?;
        let center_x = (bbox.x0 + bbox.x1) / 2.0;
        let center_y = (bbox.y0 + bbox.y1) / 2.0;

        // --- Phase 2: Build the VectorClip ---

        // Offset shapes so positions are relative to the group center
        let mut clip_shapes: Vec<Shape> = group_shapes.clone();
        for shape in &mut clip_shapes {
            shape.transform.x -= center_x;
            shape.transform.y -= center_y;
        }

        // Offset clip instances similarly
        let mut clip_instances_inside: Vec<ClipInstance> = group_clip_instances.clone();
        for ci in &mut clip_instances_inside {
            ci.transform.x -= center_x;
            ci.transform.y -= center_y;
        }

        // Create VectorClip — groups are static (one frame), not time-based clips
        let frame_duration = 1.0 / document.framerate;
        let mut clip = VectorClip::new("Group", bbox.width(), bbox.height(), frame_duration);
        clip.is_group = true;
        let clip_id = clip.id;

        let mut inner_layer = VectorLayer::new("Layer 1");
        for shape in clip_shapes {
            inner_layer.add_shape_to_keyframe(shape, 0.0);
        }
        for ci in clip_instances_inside {
            inner_layer.clip_instances.push(ci);
        }
        clip.layers.add_root(AnyLayer::Vector(inner_layer));

        // Add clip to document library
        document.add_vector_clip(clip);
        self.created_clip_id = Some(clip_id);

        // --- Phase 3: Remove originals from the layer ---

        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => unreachable!(),
        };

        // Remove shapes
        self.removed_shapes.clear();
        for id in &self.shape_ids {
            if let Some(shape) = vl.remove_shape_from_keyframe(id, self.time) {
                self.removed_shapes.push(shape);
            }
        }

        // Remove clip instances (preserve order for rollback)
        self.removed_clip_instances.clear();
        for id in &self.clip_instance_ids {
            if let Some(pos) = vl.clip_instances.iter().position(|ci| &ci.id == id) {
                self.removed_clip_instances.push(vl.clip_instances.remove(pos));
            }
        }

        // --- Phase 4: Place the new group ClipInstance ---

        let instance = ClipInstance::with_id(self.instance_id, clip_id)
            .with_position(center_x, center_y)
            .with_name("Group");
        vl.clip_instances.push(instance);

        // Register the group in the current keyframe's clip_instance_ids
        if let Some(kf) = vl.keyframe_at_mut(self.time) {
            if !kf.clip_instance_ids.contains(&self.instance_id) {
                kf.clip_instance_ids.push(self.instance_id);
            }
        }

        // --- Phase 5: Create default animation curves with initial keyframe ---

        let props_and_values = [
            (TransformProperty::X, center_x),
            (TransformProperty::Y, center_y),
            (TransformProperty::Rotation, 0.0),
            (TransformProperty::ScaleX, 1.0),
            (TransformProperty::ScaleY, 1.0),
            (TransformProperty::SkewX, 0.0),
            (TransformProperty::SkewY, 0.0),
            (TransformProperty::Opacity, 1.0),
        ];

        for (prop, value) in props_and_values {
            let target = AnimationTarget::Object {
                id: self.instance_id,
                property: prop,
            };
            let mut curve = AnimationCurve::new(target.clone(), value);
            curve.set_keyframe(Keyframe::linear(0.0, value));
            vl.layer.animation_data.set_curve(curve);
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        if let AnyLayer::Vector(vl) = layer {
            // Remove animation curves for the group's clip instance
            for prop in &[
                TransformProperty::X, TransformProperty::Y,
                TransformProperty::Rotation,
                TransformProperty::ScaleX, TransformProperty::ScaleY,
                TransformProperty::SkewX, TransformProperty::SkewY,
                TransformProperty::Opacity,
            ] {
                let target = AnimationTarget::Object {
                    id: self.instance_id,
                    property: *prop,
                };
                vl.layer.animation_data.remove_curve(&target);
            }

            // Remove the group's clip instance
            vl.clip_instances.retain(|ci| ci.id != self.instance_id);

            // Remove the group ID from the keyframe
            if let Some(kf) = vl.keyframe_at_mut(self.time) {
                kf.clip_instance_ids.retain(|id| id != &self.instance_id);
            }

            // Re-insert removed shapes
            for shape in self.removed_shapes.drain(..) {
                vl.add_shape_to_keyframe(shape, self.time);
            }

            // Re-insert removed clip instances
            for ci in self.removed_clip_instances.drain(..) {
                vl.clip_instances.push(ci);
            }
        }

        // Remove the VectorClip from the document
        if let Some(clip_id) = self.created_clip_id.take() {
            document.remove_vector_clip(&clip_id);
        }

        Ok(())
    }

    fn description(&self) -> String {
        let count = self.shape_ids.len() + self.clip_instance_ids.len();
        format!("Group {} objects", count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::ShapeColor;
    use vello::kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_group_shapes() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        let circle1 = Circle::new((0.0, 0.0), 20.0);
        let shape1 = Shape::new(circle1.to_path(0.1))
            .with_fill(ShapeColor::rgb(255, 0, 0))
            .with_position(50.0, 50.0);
        let shape1_id = shape1.id;

        let circle2 = Circle::new((0.0, 0.0), 20.0);
        let shape2 = Shape::new(circle2.to_path(0.1))
            .with_fill(ShapeColor::rgb(0, 255, 0))
            .with_position(150.0, 50.0);
        let shape2_id = shape2.id;

        layer.add_shape_to_keyframe(shape1, 0.0);
        layer.add_shape_to_keyframe(shape2, 0.0);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let instance_id = Uuid::new_v4();
        let mut action = GroupAction::new(
            layer_id, 0.0,
            vec![shape1_id, shape2_id],
            vec![],
            instance_id,
        );
        action.execute(&mut document).unwrap();

        // Shapes removed, clip instance added
        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes_at_time(0.0).len(), 0);
            assert_eq!(vl.clip_instances.len(), 1);
            assert_eq!(vl.clip_instances[0].id, instance_id);
        }
        assert_eq!(document.vector_clips.len(), 1);

        // Rollback
        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes_at_time(0.0).len(), 2);
            assert_eq!(vl.clip_instances.len(), 0);
        }
        assert!(document.vector_clips.is_empty());
    }

    #[test]
    fn test_group_mixed_shapes_and_clips() {
        let mut document = Document::new("Test");
        let mut layer = VectorLayer::new("Test Layer");

        // Add a shape
        let circle = Circle::new((0.0, 0.0), 20.0);
        let shape = Shape::new(circle.to_path(0.1))
            .with_fill(ShapeColor::rgb(255, 0, 0))
            .with_position(50.0, 50.0);
        let shape_id = shape.id;
        layer.add_shape_to_keyframe(shape, 0.0);

        // Add a clip instance (create a clip for it first)
        let mut inner_clip = VectorClip::new("Inner", 40.0, 40.0, 1.0);
        let inner_clip_id = inner_clip.id;
        let mut inner_layer = VectorLayer::new("Inner Layer");
        let inner_shape = Shape::new(Circle::new((20.0, 20.0), 15.0).to_path(0.1))
            .with_fill(ShapeColor::rgb(0, 0, 255));
        inner_layer.add_shape_to_keyframe(inner_shape, 0.0);
        inner_clip.layers.add_root(AnyLayer::Vector(inner_layer));
        document.add_vector_clip(inner_clip);

        let ci = ClipInstance::new(inner_clip_id).with_position(150.0, 50.0);
        let ci_id = ci.id;
        layer.clip_instances.push(ci);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(layer));

        let instance_id = Uuid::new_v4();
        let mut action = GroupAction::new(
            layer_id, 0.0,
            vec![shape_id],
            vec![ci_id],
            instance_id,
        );
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes_at_time(0.0).len(), 0);
            // Only the new group instance remains (the inner clip instance was grouped)
            assert_eq!(vl.clip_instances.len(), 1);
            assert_eq!(vl.clip_instances[0].id, instance_id);
        }
        // Two vector clips: the inner one + the new group
        assert_eq!(document.vector_clips.len(), 2);

        // Rollback
        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.shapes_at_time(0.0).len(), 1);
            assert_eq!(vl.clip_instances.len(), 1);
            assert_eq!(vl.clip_instances[0].id, ci_id);
        }
        // Only the inner clip remains
        assert_eq!(document.vector_clips.len(), 1);
    }

    #[test]
    fn test_group_description() {
        let action = GroupAction::new(
            Uuid::new_v4(), 0.0,
            vec![Uuid::new_v4(), Uuid::new_v4()],
            vec![Uuid::new_v4()],
            Uuid::new_v4(),
        );
        assert_eq!(action.description(), "Group 3 objects");
    }
}
