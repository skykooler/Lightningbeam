use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_DRIVE: u32 = 0;
const PARAM_TYPE: u32 = 1;
const PARAM_TONE: u32 = 2;
const PARAM_MIX: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DistortionType {
    SoftClip = 0,
    HardClip = 1,
    Tanh = 2,
    Asymmetric = 3,
}

impl DistortionType {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => DistortionType::HardClip,
            2 => DistortionType::Tanh,
            3 => DistortionType::Asymmetric,
            _ => DistortionType::SoftClip,
        }
    }
}

/// Distortion node with multiple waveshaping algorithms
pub struct DistortionNode {
    name: String,
    drive: f32,              // 0.01 to 20.0 (linear gain)
    distortion_type: DistortionType,
    tone: f32,               // 0.0 to 1.0 (low-pass filter cutoff)
    mix: f32,                // 0.0 to 1.0 (dry/wet)

    // Tone filter state (simple one-pole low-pass)
    filter_state_left: f32,
    filter_state_right: f32,
    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl DistortionNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_DRIVE, "Drive", 0.01, 20.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_TYPE, "Type", 0.0, 3.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_TONE, "Tone", 0.0, 1.0, 0.7, ParameterUnit::Generic),
            Parameter::new(PARAM_MIX, "Mix", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            drive: 1.0,
            distortion_type: DistortionType::SoftClip,
            tone: 0.7,
            mix: 1.0,
            filter_state_left: 0.0,
            filter_state_right: 0.0,
            sample_rate: 44100,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Soft clipping using cubic waveshaping
    fn soft_clip(&self, x: f32) -> f32 {
        let x = x.clamp(-2.0, 2.0);
        if x.abs() <= 1.0 {
            x
        } else {
            let sign = x.signum();
            sign * (2.0 - (2.0 - x.abs()).powi(2)) / 2.0
        }
    }

    /// Hard clipping
    fn hard_clip(&self, x: f32) -> f32 {
        x.clamp(-1.0, 1.0)
    }

    /// Hyperbolic tangent waveshaping
    fn tanh_distortion(&self, x: f32) -> f32 {
        x.tanh()
    }

    /// Asymmetric waveshaping (different curves for positive/negative)
    fn asymmetric(&self, x: f32) -> f32 {
        if x >= 0.0 {
            // Positive: soft clip
            self.soft_clip(x)
        } else {
            // Negative: harder clip
            self.hard_clip(x * 1.5) / 1.5
        }
    }

    /// Apply waveshaping based on type
    fn apply_waveshaping(&self, x: f32) -> f32 {
        match self.distortion_type {
            DistortionType::SoftClip => self.soft_clip(x),
            DistortionType::HardClip => self.hard_clip(x),
            DistortionType::Tanh => self.tanh_distortion(x),
            DistortionType::Asymmetric => self.asymmetric(x),
        }
    }

    /// Simple one-pole low-pass filter for tone control
    fn apply_tone_filter(&mut self, input: f32, is_left: bool) -> f32 {
        // Tone parameter controls cutoff frequency (0 = dark, 1 = bright)
        // Map tone to filter coefficient (0.1 to 0.99)
        let coeff = 0.1 + self.tone * 0.89;

        let state = if is_left {
            &mut self.filter_state_left
        } else {
            &mut self.filter_state_right
        };

        *state = *state * coeff + input * (1.0 - coeff);
        *state
    }

    fn process_sample(&mut self, input: f32, is_left: bool) -> f32 {
        // Apply drive (input gain)
        let driven = input * self.drive;

        // Apply waveshaping
        let distorted = self.apply_waveshaping(driven);

        // Apply tone control (low-pass filter to tame harshness)
        let filtered = self.apply_tone_filter(distorted, is_left);

        // Apply output gain compensation and mix
        let output_gain = 1.0 / (1.0 + self.drive * 0.2); // Compensate for loudness increase
        let wet = filtered * output_gain;
        let dry = input;

        // Mix dry and wet
        dry * (1.0 - self.mix) + wet * self.mix
    }
}

impl AudioNode for DistortionNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Effect
    }

    fn inputs(&self) -> &[NodePort] {
        &self.inputs
    }

    fn outputs(&self) -> &[NodePort] {
        &self.outputs
    }

    fn parameters(&self) -> &[Parameter] {
        &self.parameters
    }

    fn set_parameter(&mut self, id: u32, value: f32) {
        match id {
            PARAM_DRIVE => self.drive = value.clamp(0.01, 20.0),
            PARAM_TYPE => self.distortion_type = DistortionType::from_f32(value),
            PARAM_TONE => self.tone = value.clamp(0.0, 1.0),
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_DRIVE => self.drive,
            PARAM_TYPE => self.distortion_type as i32 as f32,
            PARAM_TONE => self.tone,
            PARAM_MIX => self.mix,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        sample_rate: u32,
    ) {
        if inputs.is_empty() || outputs.is_empty() {
            return;
        }

        // Update sample rate if changed
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            output[frame * 2] = self.process_sample(left_in, true);
            output[frame * 2 + 1] = self.process_sample(right_in, false);
        }
    }

    fn reset(&mut self) {
        self.filter_state_left = 0.0;
        self.filter_state_right = 0.0;
    }

    fn node_type(&self) -> &str {
        "Distortion"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            drive: self.drive,
            distortion_type: self.distortion_type,
            tone: self.tone,
            mix: self.mix,
            filter_state_left: 0.0, // Reset state for clone
            filter_state_right: 0.0,
            sample_rate: self.sample_rate,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
