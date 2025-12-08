//! Effect system for Lightningbeam
//!
//! Provides GPU-accelerated visual effects with animatable parameters.
//! Effects are defined by WGSL shaders embedded directly in the document.
//!
//! Effect instances are represented as `ClipInstance` objects where `clip_id`
//! references an `EffectDefinition`. Effects are treated as having infinite
//! internal duration (`EFFECT_DURATION`), with timeline duration controlled
//! solely by `timeline_start` and `timeline_duration`.

use crate::animation::AnimationCurve;
use crate::clip::ClipInstance;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Constant representing "infinite" effect duration for clip lookups.
/// Effects don't have an inherent duration like video/audio clips.
/// Their timeline duration is controlled by `ClipInstance.timeline_duration`.
pub const EFFECT_DURATION: f64 = f64::MAX;

/// Category of effect for UI organization
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectCategory {
    /// Color adjustments (brightness, contrast, hue, saturation)
    Color,
    /// Blur effects (gaussian, motion, radial)
    Blur,
    /// Distortion effects (warp, ripple, twirl)
    Distort,
    /// Stylize effects (glow, sharpen, posterize)
    Stylize,
    /// Generate effects (noise, gradients, patterns)
    Generate,
    /// Keying effects (chroma key, luma key)
    Keying,
    /// Transition effects (wipe, dissolve, etc.)
    Transition,
    /// Time-based effects (echo, frame hold)
    Time,
    /// Custom user-defined effect
    Custom,
}

/// Type of effect parameter
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterType {
    /// Floating point value
    Float,
    /// Integer value
    Int,
    /// Boolean toggle
    Bool,
    /// RGBA color
    Color,
    /// 2D point/vector
    Point2D,
    /// Angle in degrees
    Angle,
    /// Enum with named options
    Enum,
}

/// Value of an effect parameter
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParameterValue {
    Float(f64),
    Int(i64),
    Bool(bool),
    Color { r: f64, g: f64, b: f64, a: f64 },
    Point2D { x: f64, y: f64 },
    Angle(f64),
    Enum(u32),
}

impl ParameterValue {
    /// Get as f64 for shader uniform packing (returns 0.0 for non-float types)
    pub fn as_f32(&self) -> f32 {
        match self {
            ParameterValue::Float(v) => *v as f32,
            ParameterValue::Int(v) => *v as f32,
            ParameterValue::Bool(v) => if *v { 1.0 } else { 0.0 },
            ParameterValue::Angle(v) => *v as f32,
            ParameterValue::Enum(v) => *v as f32,
            ParameterValue::Color { r, .. } => *r as f32,
            ParameterValue::Point2D { x, .. } => *x as f32,
        }
    }

    /// Pack color value into 4 f32s [r, g, b, a]
    pub fn as_color_f32(&self) -> [f32; 4] {
        match self {
            ParameterValue::Color { r, g, b, a } => [*r as f32, *g as f32, *b as f32, *a as f32],
            _ => [0.0, 0.0, 0.0, 1.0],
        }
    }

    /// Pack point value into 2 f32s [x, y]
    pub fn as_point_f32(&self) -> [f32; 2] {
        match self {
            ParameterValue::Point2D { x, y } => [*x as f32, *y as f32],
            _ => [0.0, 0.0],
        }
    }
}

impl Default for ParameterValue {
    fn default() -> Self {
        ParameterValue::Float(0.0)
    }
}

/// Definition of a single effect parameter
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectParameterDef {
    /// Internal parameter name (used in shader)
    pub name: String,
    /// Display label for UI
    pub label: String,
    /// Parameter data type
    pub param_type: ParameterType,
    /// Default value
    pub default_value: ParameterValue,
    /// Minimum allowed value (for numeric types)
    pub min_value: Option<ParameterValue>,
    /// Maximum allowed value (for numeric types)
    pub max_value: Option<ParameterValue>,
    /// Whether this parameter can be animated
    pub animatable: bool,
    /// Enum option names (for ParameterType::Enum)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enum_options: Vec<String>,
}

impl EffectParameterDef {
    /// Create a new float parameter definition
    pub fn float(name: impl Into<String>, label: impl Into<String>, default: f64) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Float,
            default_value: ParameterValue::Float(default),
            min_value: None,
            max_value: None,
            animatable: true,
            enum_options: Vec::new(),
        }
    }

    /// Create a float parameter with range constraints
    pub fn float_range(
        name: impl Into<String>,
        label: impl Into<String>,
        default: f64,
        min: f64,
        max: f64,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Float,
            default_value: ParameterValue::Float(default),
            min_value: Some(ParameterValue::Float(min)),
            max_value: Some(ParameterValue::Float(max)),
            animatable: true,
            enum_options: Vec::new(),
        }
    }

    /// Create a boolean parameter definition
    pub fn boolean(name: impl Into<String>, label: impl Into<String>, default: bool) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Bool,
            default_value: ParameterValue::Bool(default),
            min_value: None,
            max_value: None,
            animatable: false,
            enum_options: Vec::new(),
        }
    }

    /// Create a color parameter definition
    pub fn color(name: impl Into<String>, label: impl Into<String>, r: f64, g: f64, b: f64, a: f64) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Color,
            default_value: ParameterValue::Color { r, g, b, a },
            min_value: None,
            max_value: None,
            animatable: true,
            enum_options: Vec::new(),
        }
    }

    /// Create an angle parameter definition (in degrees)
    pub fn angle(name: impl Into<String>, label: impl Into<String>, default: f64) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Angle,
            default_value: ParameterValue::Angle(default),
            min_value: Some(ParameterValue::Angle(0.0)),
            max_value: Some(ParameterValue::Angle(360.0)),
            animatable: true,
            enum_options: Vec::new(),
        }
    }

    /// Create a point parameter definition
    pub fn point(name: impl Into<String>, label: impl Into<String>, x: f64, y: f64) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            param_type: ParameterType::Point2D,
            default_value: ParameterValue::Point2D { x, y },
            min_value: None,
            max_value: None,
            animatable: true,
            enum_options: Vec::new(),
        }
    }
}

/// Type of input an effect can accept
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EffectInputType {
    /// Input from a specific layer
    Layer,
    /// Input from the composition (all layers below, already composited)
    Composition,
    /// Input from another effect in the chain
    Effect,
}

/// Definition of an effect input slot
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectInput {
    /// Name of this input
    pub name: String,
    /// Type of input expected
    pub input_type: EffectInputType,
    /// Whether this input is required
    pub required: bool,
}

impl EffectInput {
    /// Create a required composition input (most common case)
    pub fn composition(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            input_type: EffectInputType::Composition,
            required: true,
        }
    }

    /// Create an optional layer input
    pub fn layer(name: impl Into<String>, required: bool) -> Self {
        Self {
            name: name.into(),
            input_type: EffectInputType::Layer,
            required,
        }
    }
}

/// Complete definition of an effect (embedded shader + metadata)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectDefinition {
    /// Unique identifier for this effect definition
    pub id: Uuid,
    /// Display name
    pub name: String,
    /// Optional description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Effect category for UI organization
    pub category: EffectCategory,
    /// WGSL shader source code (embedded directly)
    pub shader_code: String,
    /// Input slots for this effect
    pub inputs: Vec<EffectInput>,
    /// Parameter definitions
    pub parameters: Vec<EffectParameterDef>,
}

impl EffectDefinition {
    /// Create a new effect definition with a single composition input
    pub fn new(
        name: impl Into<String>,
        category: EffectCategory,
        shader_code: impl Into<String>,
        parameters: Vec<EffectParameterDef>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            description: None,
            category,
            shader_code: shader_code.into(),
            inputs: vec![EffectInput::composition("source")],
            parameters,
        }
    }

    /// Create with a specific ID (for built-in effects with stable IDs)
    pub fn with_id(id: Uuid, name: impl Into<String>, category: EffectCategory, shader_code: impl Into<String>, parameters: Vec<EffectParameterDef>) -> Self {
        Self {
            id,
            name: name.into(),
            description: None,
            category,
            shader_code: shader_code.into(),
            inputs: vec![EffectInput::composition("source")],
            parameters,
        }
    }

    /// Add a description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Add custom inputs
    pub fn with_inputs(mut self, inputs: Vec<EffectInput>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Get a parameter definition by name
    pub fn get_parameter(&self, name: &str) -> Option<&EffectParameterDef> {
        self.parameters.iter().find(|p| p.name == name)
    }

    /// Create a ClipInstance for this effect definition
    ///
    /// The returned ClipInstance references this effect definition via `clip_id`.
    /// Effects use `timeline_duration` to control their length since they have
    /// infinite internal duration.
    ///
    /// # Arguments
    ///
    /// * `timeline_start` - When the effect starts on the timeline (seconds)
    /// * `duration` - How long the effect appears on the timeline (seconds)
    pub fn create_instance(&self, timeline_start: f64, duration: f64) -> ClipInstance {
        ClipInstance::new(self.id)
            .with_timeline_start(timeline_start)
            .with_timeline_duration(duration)
    }
}

/// Connection to an input source for an effect
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputConnection {
    /// Connect to a specific layer (by ID)
    Layer(Uuid),
    /// Connect to the composited result of all layers below
    Composition,
    /// Connect to the output of another effect instance
    Effect(Uuid),
}

/// Animated parameter value for an effect instance
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimatedParameter {
    /// Parameter name (matches EffectParameterDef.name)
    pub name: String,
    /// Current/base value
    pub value: ParameterValue,
    /// Optional animation curve (for animatable parameters)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<AnimationCurve>,
}

impl AnimatedParameter {
    /// Create a new non-animated parameter
    pub fn new(name: impl Into<String>, value: ParameterValue) -> Self {
        Self {
            name: name.into(),
            value,
            animation: None,
        }
    }

    /// Create with animation
    pub fn with_animation(name: impl Into<String>, value: ParameterValue, curve: AnimationCurve) -> Self {
        Self {
            name: name.into(),
            value,
            animation: Some(curve),
        }
    }

    /// Get the value at a specific time
    pub fn value_at(&self, time: f64) -> ParameterValue {
        if let Some(ref curve) = self.animation {
            // Apply animation curve to get animated value
            let animated_value = curve.eval(time);
            // Convert based on parameter type
            match &self.value {
                ParameterValue::Float(_) => ParameterValue::Float(animated_value),
                ParameterValue::Int(_) => ParameterValue::Int(animated_value.round() as i64),
                ParameterValue::Bool(_) => ParameterValue::Bool(animated_value > 0.5),
                ParameterValue::Angle(_) => ParameterValue::Angle(animated_value),
                ParameterValue::Enum(_) => ParameterValue::Enum(animated_value.round() as u32),
                // Color and Point2D would need multiple curves, so just use base value
                ParameterValue::Color { .. } => self.value.clone(),
                ParameterValue::Point2D { .. } => self.value.clone(),
            }
        } else {
            self.value.clone()
        }
    }
}

/// Instance of an effect applied to a layer
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectInstance {
    /// Unique identifier for this instance
    pub id: Uuid,
    /// ID of the effect definition this is an instance of
    pub effect_id: Uuid,
    /// Start time on the timeline (when effect becomes active)
    pub timeline_start: f64,
    /// End time on the timeline (when effect stops)
    pub timeline_end: f64,
    /// Input connections (parallel to EffectDefinition.inputs)
    pub input_connections: Vec<Option<InputConnection>>,
    /// Parameter values (name -> animated value)
    pub parameters: HashMap<String, AnimatedParameter>,
    /// Whether the effect is enabled
    pub enabled: bool,
    /// Mix/blend amount (0.0 = original, 1.0 = full effect)
    pub mix: f64,
}

impl EffectInstance {
    /// Create a new effect instance from a definition
    pub fn new(definition: &EffectDefinition, timeline_start: f64, timeline_end: f64) -> Self {
        // Initialize parameters from definition defaults
        let mut parameters = HashMap::new();
        for param_def in &definition.parameters {
            parameters.insert(
                param_def.name.clone(),
                AnimatedParameter::new(param_def.name.clone(), param_def.default_value.clone()),
            );
        }

        // Initialize input connections (Composition for required, None for optional)
        let input_connections = definition.inputs.iter()
            .map(|input| {
                if input.required && input.input_type == EffectInputType::Composition {
                    Some(InputConnection::Composition)
                } else {
                    None
                }
            })
            .collect();

        Self {
            id: Uuid::new_v4(),
            effect_id: definition.id,
            timeline_start,
            timeline_end,
            input_connections,
            parameters,
            enabled: true,
            mix: 1.0,
        }
    }

    /// Check if the effect is active at a given time
    pub fn is_active_at(&self, time: f64) -> bool {
        self.enabled && time >= self.timeline_start && time < self.timeline_end
    }

    /// Get a parameter value at a specific time
    pub fn get_parameter_at(&self, name: &str, time: f64) -> Option<ParameterValue> {
        self.parameters.get(name).map(|p| p.value_at(time))
    }

    /// Set a parameter value (non-animated)
    pub fn set_parameter(&mut self, name: &str, value: ParameterValue) {
        if let Some(param) = self.parameters.get_mut(name) {
            param.value = value;
            param.animation = None;
        }
    }

    /// Get all parameter values at a specific time as f32 array for shader uniform
    pub fn get_uniform_params(&self, time: f64, definitions: &[EffectParameterDef]) -> Vec<f32> {
        let mut params = Vec::with_capacity(16);
        for def in definitions {
            if let Some(param) = self.parameters.get(&def.name) {
                let value = param.value_at(time);
                match def.param_type {
                    ParameterType::Float | ParameterType::Int | ParameterType::Bool |
                    ParameterType::Angle | ParameterType::Enum => {
                        params.push(value.as_f32());
                    }
                    ParameterType::Color => {
                        let color = value.as_color_f32();
                        params.extend_from_slice(&color);
                    }
                    ParameterType::Point2D => {
                        let point = value.as_point_f32();
                        params.extend_from_slice(&point);
                    }
                }
            }
        }
        // Pad to 16 floats for uniform alignment
        while params.len() < 16 {
            params.push(0.0);
        }
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_definition_creation() {
        let def = EffectDefinition::new(
            "Test Effect",
            EffectCategory::Color,
            "// shader code",
            vec![EffectParameterDef::float_range("intensity", "Intensity", 1.0, 0.0, 2.0)],
        );

        assert_eq!(def.name, "Test Effect");
        assert_eq!(def.category, EffectCategory::Color);
        assert_eq!(def.parameters.len(), 1);
        assert_eq!(def.inputs.len(), 1);
    }

    #[test]
    fn test_effect_instance_creation() {
        let def = EffectDefinition::new(
            "Blur",
            EffectCategory::Blur,
            "// blur shader",
            vec![
                EffectParameterDef::float_range("radius", "Radius", 10.0, 0.0, 100.0),
                EffectParameterDef::float_range("quality", "Quality", 1.0, 0.0, 1.0),
            ],
        );

        let instance = EffectInstance::new(&def, 0.0, 10.0);

        assert_eq!(instance.effect_id, def.id);
        assert!(instance.is_active_at(5.0));
        assert!(!instance.is_active_at(15.0));
        assert_eq!(instance.parameters.len(), 2);
    }

    #[test]
    fn test_parameter_value_as_f32() {
        assert_eq!(ParameterValue::Float(1.5).as_f32(), 1.5);
        assert_eq!(ParameterValue::Int(42).as_f32(), 42.0);
        assert_eq!(ParameterValue::Bool(true).as_f32(), 1.0);
        assert_eq!(ParameterValue::Bool(false).as_f32(), 0.0);
        assert_eq!(ParameterValue::Angle(90.0).as_f32(), 90.0);
    }
}
