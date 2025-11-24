use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_ATTACK: u32 = 0;
const PARAM_RELEASE: u32 = 1;

/// Audio to CV converter (Envelope Follower)
/// Converts audio amplitude to control voltage
pub struct AudioToCVNode {
    name: String,
    envelope: f32,       // Current envelope value
    attack: f32,         // Attack time in seconds
    release: f32,        // Release time in seconds
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl AudioToCVNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_ATTACK, "Attack", 0.001, 1.0, 0.01, ParameterUnit::Time),
            Parameter::new(PARAM_RELEASE, "Release", 0.001, 1.0, 0.1, ParameterUnit::Time),
        ];

        Self {
            name,
            envelope: 0.0,
            attack: 0.01,
            release: 0.1,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for AudioToCVNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Utility
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
            PARAM_ATTACK => self.attack = value.clamp(0.001, 1.0),
            PARAM_RELEASE => self.release = value.clamp(0.001, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ATTACK => self.attack,
            PARAM_RELEASE => self.release,
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

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio input is stereo (interleaved L/R), CV output is mono
        let audio_frames = input.len() / 2;
        let cv_frames = output.len();
        let frames = audio_frames.min(cv_frames);

        // Calculate attack and release coefficients
        let sample_rate_f32 = sample_rate as f32;
        let attack_coeff = (-1.0 / (self.attack * sample_rate_f32)).exp();
        let release_coeff = (-1.0 / (self.release * sample_rate_f32)).exp();

        for frame in 0..frames {
            // Get stereo samples
            let left = input[frame * 2];
            let right = input[frame * 2 + 1];

            // Calculate RMS-like value (average of absolute values for simplicity)
            let amplitude = (left.abs() + right.abs()) / 2.0;

            // Envelope follower with attack/release
            if amplitude > self.envelope {
                // Attack: follow signal up quickly
                self.envelope = amplitude * (1.0 - attack_coeff) + self.envelope * attack_coeff;
            } else {
                // Release: decay slowly
                self.envelope = amplitude * (1.0 - release_coeff) + self.envelope * release_coeff;
            }

            // Output CV (mono)
            output[frame] = self.envelope;
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn node_type(&self) -> &str {
        "AudioToCV"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            envelope: 0.0,      // Reset envelope
            attack: self.attack,
            release: self.release,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
