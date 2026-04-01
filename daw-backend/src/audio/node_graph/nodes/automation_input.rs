use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
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
    /// Time in beats (derived; canonical in Measures mode)
    #[serde(default)]
    pub time_beats: f64,
    /// Time in frames (derived; canonical in Frames mode)
    #[serde(default)]
    pub time_frames: f64,
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
            time_beats: 0.0,
            time_frames: 0.0,
            value,
            interpolation: InterpolationType::Linear,
            ease_out: (0.58, 1.0),
            ease_in: (0.42, 0.0),
        }
    }

    /// Populate beats/frames from the current seconds value.
    pub fn sync_from_seconds(&mut self, bpm: f64, fps: f64) {
        self.time_beats = self.time * bpm / 60.0;
        self.time_frames = self.time * fps;
    }

    /// BPM changed; beats are canonical → recompute seconds and frames.
    pub fn apply_beats(&mut self, bpm: f64, fps: f64) {
        self.time = self.time_beats * 60.0 / bpm;
        self.time_frames = self.time * fps;
    }

    /// FPS changed; frames are canonical → recompute seconds and beats.
    pub fn apply_frames(&mut self, fps: f64, bpm: f64) {
        self.time = self.time_frames / fps;
        self.time_beats = self.time * bpm / 60.0;
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
    /// Minimum output value (for UI display range)
    pub value_min: f32,
    /// Maximum output value (for UI display range)
    pub value_max: f32,
}

impl AutomationInputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(0, "Min", f32::NEG_INFINITY, f32::INFINITY, -1.0, ParameterUnit::Generic),
            Parameter::new(1, "Max", f32::NEG_INFINITY, f32::INFINITY,  1.0, ParameterUnit::Generic),
        ];

        Self {
            name: name.clone(),
            display_name: "Automation".to_string(),
            keyframes: vec![AutomationKeyframe::new(0.0, 0.0)],
            outputs,
            parameters,
            playback_time: Arc::new(RwLock::new(0.0)),
            value_min: -1.0,
            value_max: 1.0,
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

    /// Populate beats/frames on all keyframes from their current seconds values.
    pub fn sync_keyframes_from_seconds(&mut self, bpm: f64, fps: f64) {
        for kf in &mut self.keyframes {
            kf.sync_from_seconds(bpm, fps);
        }
    }

    /// BPM changed: for each keyframe, bootstrap beats from seconds (using `from_bpm`) if not yet
    /// set, then re-derive seconds and frames from beats using `to_bpm`.
    pub fn apply_beats_to_keyframes(&mut self, from_bpm: f64, to_bpm: f64, fps: f64) {
        for kf in &mut self.keyframes {
            if kf.time_beats == 0.0 && kf.time.abs() > 1e-9 {
                kf.sync_from_seconds(from_bpm, fps);
            }
            kf.apply_beats(to_bpm, fps);
        }
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

    fn set_parameter(&mut self, id: u32, value: f32) {
        match id {
            0 => self.value_min = value,
            1 => self.value_max = value,
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            0 => self.value_min,
            1 => self.value_max,
            _ => 0.0,
        }
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
            value_min: self.value_min,
            value_max: self.value_max,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
