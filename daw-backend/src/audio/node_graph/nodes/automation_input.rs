use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

/// Interpolation type for automation curves
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum InterpolationType {
    Linear,
    Bezier,
    Step,
    Hold,
}

/// A single keyframe in an automation curve
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationKeyframe {
    /// Time in seconds (absolute project time)
    pub time: f64,
    /// CV output value
    pub value: f32,
    /// Interpolation type to next keyframe
    pub interpolation: InterpolationType,
    /// Bezier ease-out control point (for bezier interpolation)
    pub ease_out: (f32, f32),
    /// Bezier ease-in control point (for bezier interpolation)
    pub ease_in: (f32, f32),
}

impl AutomationKeyframe {
    pub fn new(time: f64, value: f32) -> Self {
        Self {
            time,
            value,
            interpolation: InterpolationType::Linear,
            ease_out: (0.58, 1.0),
            ease_in: (0.42, 0.0),
        }
    }
}

/// Automation Input Node - outputs CV signal controlled by timeline curves
pub struct AutomationInputNode {
    name: String,
    display_name: String, // User-editable name shown in UI
    keyframes: Vec<AutomationKeyframe>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
    /// Shared playback time (set by the graph before processing)
    playback_time: Arc<RwLock<f64>>,
}

impl AutomationInputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        Self {
            name: name.clone(),
            display_name: "Automation".to_string(),
            keyframes: Vec::new(),
            outputs,
            parameters: Vec::new(),
            playback_time: Arc::new(RwLock::new(0.0)),
        }
    }

    /// Set the playback time (called by graph before processing)
    pub fn set_playback_time(&mut self, time: f64) {
        if let Ok(mut playback) = self.playback_time.write() {
            *playback = time;
        }
    }

    /// Get the display name (shown in UI)
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Set the display name
    pub fn set_display_name(&mut self, name: String) {
        self.display_name = name;
    }

    /// Add a keyframe to the curve (maintains sorted order by time)
    pub fn add_keyframe(&mut self, keyframe: AutomationKeyframe) {
        // Find insertion position to maintain sorted order
        let pos = self.keyframes.binary_search_by(|kf| {
            kf.time.partial_cmp(&keyframe.time).unwrap_or(std::cmp::Ordering::Equal)
        });

        match pos {
            Ok(idx) => {
                // Replace existing keyframe at same time
                self.keyframes[idx] = keyframe;
            }
            Err(idx) => {
                // Insert at correct position
                self.keyframes.insert(idx, keyframe);
            }
        }
    }

    /// Remove keyframe at specific time (with tolerance)
    pub fn remove_keyframe_at_time(&mut self, time: f64, tolerance: f64) -> bool {
        if let Some(idx) = self.keyframes.iter().position(|kf| (kf.time - time).abs() < tolerance) {
            self.keyframes.remove(idx);
            true
        } else {
            false
        }
    }

    /// Update an existing keyframe
    pub fn update_keyframe(&mut self, keyframe: AutomationKeyframe) {
        // Remove old keyframe at this time, then add new one
        self.remove_keyframe_at_time(keyframe.time, 0.001);
        self.add_keyframe(keyframe);
    }

    /// Get all keyframes
    pub fn keyframes(&self) -> &[AutomationKeyframe] {
        &self.keyframes
    }

    /// Clear all keyframes
    pub fn clear_keyframes(&mut self) {
        self.keyframes.clear();
    }

    /// Evaluate curve at a specific time
    fn evaluate_at_time(&self, time: f64) -> f32 {
        if self.keyframes.is_empty() {
            return 0.0;
        }

        // Before first keyframe
        if time <= self.keyframes[0].time {
            return self.keyframes[0].value;
        }

        // After last keyframe
        let last_idx = self.keyframes.len() - 1;
        if time >= self.keyframes[last_idx].time {
            return self.keyframes[last_idx].value;
        }

        // Find bracketing keyframes
        for i in 0..self.keyframes.len() - 1 {
            let kf1 = &self.keyframes[i];
            let kf2 = &self.keyframes[i + 1];

            if time >= kf1.time && time <= kf2.time {
                return self.interpolate(kf1, kf2, time);
            }
        }

        0.0
    }

    /// Interpolate between two keyframes
    fn interpolate(&self, kf1: &AutomationKeyframe, kf2: &AutomationKeyframe, time: f64) -> f32 {
        // Calculate normalized position between keyframes (0.0 to 1.0)
        let t = if kf2.time == kf1.time {
            0.0
        } else {
            ((time - kf1.time) / (kf2.time - kf1.time)) as f32
        };

        match kf1.interpolation {
            InterpolationType::Linear => {
                // Simple linear interpolation
                kf1.value + (kf2.value - kf1.value) * t
            }
            InterpolationType::Bezier => {
                // Cubic bezier interpolation using control points
                let eased_t = self.cubic_bezier_ease(t, kf1.ease_out, kf2.ease_in);
                kf1.value + (kf2.value - kf1.value) * eased_t
            }
            InterpolationType::Step | InterpolationType::Hold => {
                // Hold value until next keyframe
                kf1.value
            }
        }
    }

    /// Cubic bezier easing function
    fn cubic_bezier_ease(&self, t: f32, ease_out: (f32, f32), ease_in: (f32, f32)) -> f32 {
        // Simplified cubic bezier for 0,0 -> easeOut -> easeIn -> 1,1
        let u = 1.0 - t;
        3.0 * u * u * t * ease_out.1 +
        3.0 * u * t * t * ease_in.1 +
        t * t * t
    }
}

impl AudioNode for AutomationInputNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Input
    }

    fn inputs(&self) -> &[NodePort] {
        &[] // No inputs
    }

    fn outputs(&self) -> &[NodePort] {
        &self.outputs
    }

    fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    fn set_parameter(&mut self, _id: u32, _value: f32) {
        // No parameters
    }

    fn get_parameter(&self, _id: u32) -> f32 {
        0.0
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let length = output.len();

        // Get the starting playback time
        let playhead = if let Ok(playback) = self.playback_time.read() {
            *playback
        } else {
            0.0
        };

        // Calculate time per sample
        let sample_duration = 1.0 / sample_rate as f64;

        // Evaluate curve for each sample
        for i in 0..length {
            let time = playhead + (i as f64 * sample_duration);
            output[i] = self.evaluate_at_time(time);
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "AutomationInput"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            keyframes: self.keyframes.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
            playback_time: Arc::new(RwLock::new(0.0)),
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
