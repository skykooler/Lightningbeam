//! Animation system for Lightningbeam
//!
//! Provides keyframe-based animation curves with support for different
//! interpolation types and property targets.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Interpolation type for keyframes
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum InterpolationType {
    /// Linear interpolation between keyframes
    Linear,
    /// Smooth bezier interpolation with handles
    Bezier,
    /// Hold value until next keyframe (step function)
    Hold,
}

/// Extrapolation type for values outside keyframe range
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExtrapolationType {
    /// Hold the first/last keyframe value
    Hold,
    /// Continue with the slope from the first/last segment
    Linear,
    /// Repeat the curve pattern cyclically
    Cyclic,
    /// Repeat the curve, but offset each cycle by the change in the previous cycle
    /// (each cycle starts where the previous one ended)
    CyclicOffset,
}

impl Default for ExtrapolationType {
    fn default() -> Self {
        ExtrapolationType::Hold
    }
}

/// A single keyframe in an animation curve
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Time in seconds
    pub time: f64,
    /// Value at this keyframe
    pub value: f64,
    /// Interpolation type to use after this keyframe
    pub interpolation: InterpolationType,
    /// Bezier handle for smooth curves (in and out tangents)
    /// Format: (in_time, in_value, out_time, out_value)
    pub bezier_handles: Option<(f64, f64, f64, f64)>,
}

impl Keyframe {
    /// Create a new linear keyframe
    pub fn linear(time: f64, value: f64) -> Self {
        Self {
            time,
            value,
            interpolation: InterpolationType::Linear,
            bezier_handles: None,
        }
    }

    /// Create a new hold keyframe
    pub fn hold(time: f64, value: f64) -> Self {
        Self {
            time,
            value,
            interpolation: InterpolationType::Hold,
            bezier_handles: None,
        }
    }

    /// Create a new bezier keyframe with handles
    pub fn bezier(
        time: f64,
        value: f64,
        in_time: f64,
        in_value: f64,
        out_time: f64,
        out_value: f64,
    ) -> Self {
        Self {
            time,
            value,
            interpolation: InterpolationType::Bezier,
            bezier_handles: Some((in_time, in_value, out_time, out_value)),
        }
    }
}

/// Transform properties that can be animated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransformProperty {
    X,
    Y,
    Rotation,
    ScaleX,
    ScaleY,
    SkewX,
    SkewY,
    Opacity,
}

/// Shape properties that can be animated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum ShapeProperty {
    /// Whether the shape is visible (0 or 1, for animation)
    Exists,
    /// Z-order within the layer
    ZOrder,
    /// Morph between shape versions (fractional index)
    ShapeIndex,
}

/// Layer-level properties that can be animated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum LayerProperty {
    /// Layer opacity (0.0 to 1.0)
    Opacity,
    /// Layer visibility (0 or 1, for animation)
    Visibility,
}

/// Audio-specific properties that can be automated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum AudioProperty {
    /// Volume in dB (-60 to +12 typical range)
    Volume,
    /// Pan position (-1.0 left to +1.0 right)
    Pan,
    /// Pitch shift in semitones
    Pitch,
}

/// Video-specific properties that can be animated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum VideoProperty {
    /// Fade/opacity (0.0 to 1.0)
    Fade,
    /// X position
    PositionX,
    /// Y position
    PositionY,
    /// Scale factor
    Scale,
    /// Rotation in degrees
    Rotation,
}

/// Effect-specific properties that can be animated
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum EffectProperty {
    /// Effect intensity (0.0 to 1.0)
    Intensity,
    /// Mix/blend amount (0.0 to 1.0)
    Mix,
    /// Custom effect parameter (effect-specific)
    Custom(u32),
}

/// Target for an animation curve (type-safe property identification)
#[derive(Clone, Debug, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum AnimationTarget {
    /// Object transform property
    Object {
        id: Uuid,
        property: TransformProperty,
    },
    /// Shape property
    Shape { id: Uuid, property: ShapeProperty },
    /// Layer property
    Layer { property: LayerProperty },
    /// Audio automation
    Audio { id: Uuid, property: AudioProperty },
    /// Video property
    Video { id: Uuid, property: VideoProperty },
    /// Effect parameter
    Effect {
        id: Uuid,
        property: EffectProperty,
    },
    /// Generic automation node parameter
    Automation { node_id: u32, parameter: String },
}

/// An animation curve with keyframes
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimationCurve {
    /// What this curve animates
    pub target: AnimationTarget,
    /// Keyframes in chronological order
    pub keyframes: Vec<Keyframe>,
    /// Default value when no keyframes are present
    pub default_value: f64,
    /// How to extrapolate before the first keyframe
    #[serde(default)]
    pub pre_extrapolation: ExtrapolationType,
    /// How to extrapolate after the last keyframe
    #[serde(default)]
    pub post_extrapolation: ExtrapolationType,
}

impl AnimationCurve {
    /// Create a new animation curve
    pub fn new(target: AnimationTarget, default_value: f64) -> Self {
        Self {
            target,
            keyframes: Vec::new(),
            default_value,
            pre_extrapolation: ExtrapolationType::Hold,
            post_extrapolation: ExtrapolationType::Hold,
        }
    }

    /// Get the time range of keyframes (min, max)
    fn get_keyframe_range(&self) -> Option<(f64, f64)> {
        if self.keyframes.is_empty() {
            None
        } else {
            Some((
                self.keyframes.first().unwrap().time,
                self.keyframes.last().unwrap().time,
            ))
        }
    }

    /// Get the keyframes that bracket the given time
    /// Returns (before, after) where:
    /// - (None, Some(kf)) if time is before the first keyframe
    /// - (Some(kf), None) if time is after the last keyframe
    /// - (Some(before), Some(after)) if time is between two keyframes
    /// - (None, None) if there are no keyframes
    pub fn get_bracketing_keyframes(&self, time: f64) -> (Option<&Keyframe>, Option<&Keyframe>) {
        if self.keyframes.is_empty() {
            return (None, None);
        }

        // Find the first keyframe after the given time
        let after_idx = self.keyframes.iter().position(|kf| kf.time > time);

        match after_idx {
            None => {
                // Time is after all keyframes
                (self.keyframes.last(), None)
            }
            Some(0) => {
                // Time is before all keyframes
                (None, self.keyframes.first())
            }
            Some(idx) => {
                // Time is between two keyframes
                (Some(&self.keyframes[idx - 1]), Some(&self.keyframes[idx]))
            }
        }
    }

    /// Interpolate between two keyframes
    fn interpolate(&self, before_kf: &Keyframe, after_kf: &Keyframe, time: f64) -> f64 {
        let t = (time - before_kf.time) / (after_kf.time - before_kf.time);

        match before_kf.interpolation {
            InterpolationType::Linear => {
                // Linear interpolation
                before_kf.value + t * (after_kf.value - before_kf.value)
            }
            InterpolationType::Bezier => {
                // Bezier interpolation using handles
                if let Some((_, in_val, _, out_val)) = before_kf.bezier_handles {
                    // Cubic bezier interpolation
                    let p0 = before_kf.value;
                    let p1 = out_val;
                    let p2 = in_val;
                    let p3 = after_kf.value;

                    let t2 = t * t;
                    let t3 = t2 * t;
                    let mt = 1.0 - t;
                    let mt2 = mt * mt;
                    let mt3 = mt2 * mt;

                    mt3 * p0 + 3.0 * mt2 * t * p1 + 3.0 * mt * t2 * p2 + t3 * p3
                } else {
                    // Fallback to linear if no handles
                    before_kf.value + t * (after_kf.value - before_kf.value)
                }
            }
            InterpolationType::Hold => {
                // Hold until next keyframe
                before_kf.value
            }
        }
    }

    /// Evaluate the curve at a given time
    pub fn eval(&self, time: f64) -> f64 {
        if self.keyframes.is_empty() {
            return self.default_value;
        }

        let (before, after) = self.get_bracketing_keyframes(time);

        match (before, after) {
            (None, None) => self.default_value,

            (None, Some(first_kf)) => {
                // Before first keyframe - use pre-extrapolation
                self.extrapolate_pre(time, first_kf)
            }

            (Some(last_kf), None) => {
                // After last keyframe - use post-extrapolation
                self.extrapolate_post(time, last_kf)
            }

            (Some(before_kf), Some(after_kf)) => {
                // Between keyframes - interpolate
                self.interpolate(before_kf, after_kf, time)
            }
        }
    }

    /// True when `time` lies strictly between two keyframes — an in-between frame of a
    /// tween (not on a keyframe, not in the pre/post-extrapolation tails).
    pub fn is_tween_inbetween(&self, time: f64, tol: f64) -> bool {
        if self.keyframes.len() < 2 {
            return false;
        }
        let first = self.keyframes.first().unwrap().time;
        let last = self.keyframes.last().unwrap().time;
        if time <= first + tol || time >= last - tol {
            return false;
        }
        !self.keyframes.iter().any(|kf| (kf.time - time).abs() <= tol)
    }

    /// Extrapolate before the first keyframe
    fn extrapolate_pre(&self, time: f64, first_kf: &Keyframe) -> f64 {
        match self.pre_extrapolation {
            ExtrapolationType::Hold => first_kf.value,

            ExtrapolationType::Linear => {
                // Use slope from first segment if available
                if self.keyframes.len() >= 2 {
                    let second_kf = &self.keyframes[1];
                    let slope = (second_kf.value - first_kf.value)
                              / (second_kf.time - first_kf.time);
                    first_kf.value + slope * (time - first_kf.time)
                } else {
                    first_kf.value
                }
            }

            ExtrapolationType::Cyclic => {
                let (start_time, end_time) = self.get_keyframe_range().unwrap();
                let duration = end_time - start_time;
                if duration <= 0.0 {
                    return first_kf.value;
                }

                // Map time into the keyframe range
                let offset = ((start_time - time) / duration).ceil() * duration;
                let mapped_time = time + offset;
                self.eval(mapped_time)
            }

            ExtrapolationType::CyclicOffset => {
                let (start_time, end_time) = self.get_keyframe_range().unwrap();
                let duration = end_time - start_time;
                if duration <= 0.0 {
                    return first_kf.value;
                }

                let first_val = self.keyframes.first().unwrap().value;
                let last_val = self.keyframes.last().unwrap().value;
                let cycle_delta = last_val - first_val;

                // Calculate which cycle we're in (negative for pre-extrapolation)
                let cycles = ((start_time - time) / duration).ceil();
                let offset = cycles * duration;
                let mapped_time = time + offset;

                // Evaluate and offset by accumulated cycles
                self.eval(mapped_time) - cycles * cycle_delta
            }
        }
    }

    /// Extrapolate after the last keyframe
    fn extrapolate_post(&self, time: f64, last_kf: &Keyframe) -> f64 {
        match self.post_extrapolation {
            ExtrapolationType::Hold => last_kf.value,

            ExtrapolationType::Linear => {
                // Use slope from last segment if available
                let n = self.keyframes.len();
                if n >= 2 {
                    let second_last_kf = &self.keyframes[n - 2];
                    let slope = (last_kf.value - second_last_kf.value)
                              / (last_kf.time - second_last_kf.time);
                    last_kf.value + slope * (time - last_kf.time)
                } else {
                    last_kf.value
                }
            }

            ExtrapolationType::Cyclic => {
                let (start_time, end_time) = self.get_keyframe_range().unwrap();
                let duration = end_time - start_time;
                if duration <= 0.0 {
                    return last_kf.value;
                }

                // Map time into the keyframe range
                let offset = ((time - start_time) / duration).floor() * duration;
                let mapped_time = time - offset;
                self.eval(mapped_time)
            }

            ExtrapolationType::CyclicOffset => {
                let (start_time, end_time) = self.get_keyframe_range().unwrap();
                let duration = end_time - start_time;
                if duration <= 0.0 {
                    return last_kf.value;
                }

                let first_val = self.keyframes.first().unwrap().value;
                let last_val = self.keyframes.last().unwrap().value;
                let cycle_delta = last_val - first_val;

                // Calculate which cycle we're in
                let cycles = ((time - start_time) / duration).floor();
                let offset = cycles * duration;
                let mapped_time = time - offset;

                // Evaluate and offset by accumulated cycles
                self.eval(mapped_time) + cycles * cycle_delta
            }
        }
    }

    /// Add or update a keyframe
    pub fn set_keyframe(&mut self, keyframe: Keyframe) {
        // Find existing keyframe at this time or insert new one
        if let Some(existing) = self
            .keyframes
            .iter_mut()
            .find(|kf| (kf.time - keyframe.time).abs() < 0.001)
        {
            *existing = keyframe;
        } else {
            self.keyframes.push(keyframe);
            // Keep keyframes sorted by time
            self.keyframes
                .sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
        }
    }

    /// Remove a keyframe at the given time (within tolerance)
    pub fn remove_keyframe(&mut self, time: f64, tolerance: f64) -> bool {
        if let Some(idx) = self
            .keyframes
            .iter()
            .position(|kf| (kf.time - time).abs() < tolerance)
        {
            self.keyframes.remove(idx);
            true
        } else {
            false
        }
    }

    /// Get the keyframe closest to the given time, if within tolerance
    pub fn get_keyframe_at(&self, time: f64, tolerance: f64) -> Option<&Keyframe> {
        let (before, after) = self.get_bracketing_keyframes(time);

        // Check if before keyframe is within tolerance
        if let Some(kf) = before {
            if (kf.time - time).abs() < tolerance {
                return Some(kf);
            }
        }

        // Check if after keyframe is within tolerance
        if let Some(kf) = after {
            if (kf.time - time).abs() < tolerance {
                return Some(kf);
            }
        }

        None
    }
}

/// Collection of animation curves for a layer
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnimationData {
    /// Map of animation targets to their curves
    pub curves: HashMap<AnimationTarget, AnimationCurve>,
}

impl AnimationData {
    /// Create new empty animation data
    pub fn new() -> Self {
        Self {
            curves: HashMap::new(),
        }
    }

    /// Get a curve for a specific target
    pub fn get_curve(&self, target: &AnimationTarget) -> Option<&AnimationCurve> {
        self.curves.get(target)
    }

    /// Get a mutable curve for a specific target
    pub fn get_curve_mut(&mut self, target: &AnimationTarget) -> Option<&mut AnimationCurve> {
        self.curves.get_mut(target)
    }

    /// Add or replace a curve
    pub fn set_curve(&mut self, curve: AnimationCurve) {
        let target = curve.target.clone();
        self.curves.insert(target, curve);
    }

    /// Remove a curve
    pub fn remove_curve(&mut self, target: &AnimationTarget) -> Option<AnimationCurve> {
        self.curves.remove(target)
    }

    /// True when the object (e.g. a clip instance) is mid motion-tween at `time` — any of
    /// its curves has `time` strictly between two keyframes. Used to lock out editing on
    /// in-between frames (editing there would silently insert a keyframe and disturb the tween).
    pub fn is_object_tweened_at(&self, id: uuid::Uuid, time: f64) -> bool {
        const TOL: f64 = 0.001;
        self.curves.iter().any(|(target, curve)| {
            matches!(target, AnimationTarget::Object { id: oid, .. } if *oid == id)
                && curve.is_tween_inbetween(time, TOL)
        })
    }

    /// Evaluate a property at a given time
    pub fn eval(&self, target: &AnimationTarget, time: f64, default: f64) -> f64 {
        self.curves
            .get(target)
            .map(|curve| curve.eval(time))
            .unwrap_or(default)
    }

    /// Evaluate the effective transform for a clip instance at a given time.
    /// Uses animation curves if they exist, falling back to the clip instance's base transform.
    pub fn eval_clip_instance_transform(
        &self,
        instance_id: uuid::Uuid,
        time: f64,
        base: &crate::object::Transform,
        base_opacity: f64,
    ) -> (crate::object::Transform, f64) {
        let mut t = base.clone();
        t.x = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::X }, time, base.x);
        t.y = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::Y }, time, base.y);
        t.rotation = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::Rotation }, time, base.rotation);
        t.scale_x = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::ScaleX }, time, base.scale_x);
        t.scale_y = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::ScaleY }, time, base.scale_y);
        t.skew_x = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::SkewX }, time, base.skew_x);
        t.skew_y = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::SkewY }, time, base.skew_y);
        let opacity = self.eval(&AnimationTarget::Object { id: instance_id, property: TransformProperty::Opacity }, time, base_opacity);
        (t, opacity)
    }
}

#[cfg(test)]
mod tween_lock_tests {
    use super::*;

    #[test]
    fn curve_in_between_detection() {
        let mut c = AnimationCurve::new(
            AnimationTarget::Object { id: uuid::Uuid::nil(), property: TransformProperty::X },
            0.0,
        );
        c.set_keyframe(Keyframe::linear(0.0, 0.0));
        c.set_keyframe(Keyframe::linear(10.0, 100.0));

        assert!(c.is_tween_inbetween(5.0, 0.001), "strictly between keyframes");
        assert!(!c.is_tween_inbetween(0.0, 0.001), "on a keyframe");
        assert!(!c.is_tween_inbetween(10.0, 0.001), "on a keyframe");
        assert!(!c.is_tween_inbetween(15.0, 0.001), "past the last keyframe (extrapolation tail)");
    }

    #[test]
    fn single_keyframe_is_never_in_between() {
        let mut c = AnimationCurve::new(
            AnimationTarget::Object { id: uuid::Uuid::nil(), property: TransformProperty::X },
            0.0,
        );
        c.set_keyframe(Keyframe::linear(10.0, 100.0));
        assert!(!c.is_tween_inbetween(5.0, 0.001));
        assert!(!c.is_tween_inbetween(20.0, 0.001));
    }

    #[test]
    fn object_tweened_when_any_curve_is_in_between() {
        let id = uuid::Uuid::new_v4();
        let mut data = AnimationData::new();
        let mut cx = AnimationCurve::new(
            AnimationTarget::Object { id, property: TransformProperty::X },
            0.0,
        );
        cx.set_keyframe(Keyframe::linear(0.0, 0.0));
        cx.set_keyframe(Keyframe::linear(10.0, 100.0));
        data.set_curve(cx);

        assert!(data.is_object_tweened_at(id, 5.0));
        assert!(!data.is_object_tweened_at(id, 0.0));
        assert!(!data.is_object_tweened_at(uuid::Uuid::new_v4(), 5.0), "different object");
    }
}
