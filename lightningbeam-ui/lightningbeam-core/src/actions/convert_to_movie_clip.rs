//! Convert to Movie Clip action
//!
//! Wraps selected shapes and/or clip instances into a new VectorClip
//! with is_group = false, giving it a real internal timeline.
//! Works with 1+ selected items (unlike Group which requires 2+).

use crate::action::Action;
use crate::animation::{AnimationCurve, AnimationTarget, Keyframe, TransformProperty};
use crate::clip::{ClipInstance, VectorClip};
use crate::document::Document;
use crate::layer::{AnyLayer, VectorLayer};
use crate::shape::Shape;
use uuid::Uuid;
use vello::kurbo::{Rect, Shape as KurboShape};

pub struct ConvertToMovieClipAction {
    layer_id: Uuid,
    time: f64,
    shape_ids: Vec<Uuid>,
    clip_instance_ids: Vec<Uuid>,
    instance_id: Uuid,
    created_clip_id: Option<Uuid>,
    removed_shapes: Vec<Shape>,
    removed_clip_instances: Vec<ClipInstance>,
}

impl ConvertToMovieClipAction {
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

impl Action for ConvertToMovieClipAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Convert to Movie Clip is only supported on vector layers".to_string()),
        };

        // Collect shapes
        let shapes_at_time = vl.shapes_at_time(self.time);
        let mut collected_shapes: Vec<Shape> = Vec::new();
        for id in &self.shape_ids {
            if let Some(shape) = shapes_at_time.iter().find(|s| &s.id == id) {
                collected_shapes.push(shape.clone());
            }
        }

        // Collect clip instances
        let mut collected_clip_instances: Vec<ClipInstance> = Vec::new();
        for id in &self.clip_instance_ids {
            if let Some(ci) = vl.clip_instances.iter().find(|ci| &ci.id == id) {
                collected_clip_instances.push(ci.clone());
            }
        }

        let total_items = collected_shapes.len() + collected_clip_instances.len();
        if total_items < 1 {
            return Err("Need at least 1 item to convert to movie clip".to_string());
        }

        // Compute combined bounding box
        let mut combined_bbox: Option<Rect> = None;

        for shape in &collected_shapes {
            let local_bbox = shape.path().bounding_box();
            let transform = shape.transform.to_affine();
            let transformed_bbox = transform.transform_rect_bbox(local_bbox);
            combined_bbox = Some(match combined_bbox {
                Some(existing) => existing.union(transformed_bbox),
                None => transformed_bbox,
            });
        }

        for ci in &collected_clip_instances {
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

        // Offset shapes relative to center
        let mut clip_shapes: Vec<Shape> = collected_shapes.clone();
        for shape in &mut clip_shapes {
            shape.transform.x -= center_x;
            shape.transform.y -= center_y;
        }

        let mut clip_instances_inside: Vec<ClipInstance> = collected_clip_instances.clone();
        for ci in &mut clip_instances_inside {
            ci.transform.x -= center_x;
            ci.transform.y -= center_y;
        }

        // Create VectorClip with real timeline duration
        let mut clip = VectorClip::new("Movie Clip", bbox.width(), bbox.height(), document.duration);
        // is_group defaults to false — movie clips have real timelines
        let clip_id = clip.id;

        let mut inner_layer = VectorLayer::new("Layer 1");
        for shape in clip_shapes {
            inner_layer.add_shape_to_keyframe(shape, 0.0);
        }
        for ci in clip_instances_inside {
            inner_layer.clip_instances.push(ci);
        }
        clip.layers.add_root(AnyLayer::Vector(inner_layer));

        document.add_vector_clip(clip);
        self.created_clip_id = Some(clip_id);

        // Remove originals from the layer
        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        let vl = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => unreachable!(),
        };

        self.removed_shapes.clear();
        for id in &self.shape_ids {
            if let Some(shape) = vl.remove_shape_from_keyframe(id, self.time) {
                self.removed_shapes.push(shape);
            }
        }

        self.removed_clip_instances.clear();
        for id in &self.clip_instance_ids {
            if let Some(pos) = vl.clip_instances.iter().position(|ci| &ci.id == id) {
                self.removed_clip_instances.push(vl.clip_instances.remove(pos));
            }
        }

        // Place the new ClipInstance
        let instance = ClipInstance::with_id(self.instance_id, clip_id)
            .with_position(center_x, center_y)
            .with_name("Movie Clip");
        vl.clip_instances.push(instance);

        // Create default animation curves
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
            // Remove animation curves
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

            // Remove the clip instance
            vl.clip_instances.retain(|ci| ci.id != self.instance_id);

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
        format!("Convert {} object(s) to Movie Clip", count)
    }
}
