use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_ATTACK: u32 = 0;
const PARAM_RELEASE: u32 = 1;

/// Envelope Follower - extracts amplitude envelope from audio signal
/// Outputs a CV signal that follows the loudness of the input
pub struct EnvelopeFollowerNode {
    name: String,
    attack_time: f32,   // seconds
    release_time: f32,  // seconds
    envelope: f32,      // current envelope level
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl EnvelopeFollowerNode {
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
            attack_time: 0.01,
            release_time: 0.1,
            envelope: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for EnvelopeFollowerNode {
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
            PARAM_ATTACK => self.attack_time = value.clamp(0.001, 1.0),
            PARAM_RELEASE => self.release_time = value.clamp(0.001, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ATTACK => self.attack_time,
            PARAM_RELEASE => self.release_time,
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
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let length = output.len();

        let input = if !inputs.is_empty() && !inputs[0].is_empty() {
            inputs[0]
        } else {
            &[]
        };

        // Calculate filter coefficients
        // One-pole filter: y[n] = y[n-1] + coefficient * (x[n] - y[n-1])
        let sample_duration = 1.0 / sample_rate as f32;

        // Time constant τ = time to reach ~63% of target
        // Coefficient = 1 - e^(-1/(τ * sample_rate))
        // Simplified approximation: coefficient ≈ sample_duration / time_constant
        let attack_coeff = (sample_duration / self.attack_time).min(1.0);
        let release_coeff = (sample_duration / self.release_time).min(1.0);

        // Process each sample
        for i in 0..length {
            // Get absolute value of input (rectify)
            let input_level = if i < input.len() {
                input[i].abs()
            } else {
                0.0
            };

            // Apply attack or release
            let coeff = if input_level > self.envelope {
                attack_coeff  // Rising - use attack time
            } else {
                release_coeff // Falling - use release time
            };

            // One-pole filter
            self.envelope += (input_level - self.envelope) * coeff;

            output[i] = self.envelope;
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn node_type(&self) -> &str {
        "EnvelopeFollower"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            attack_time: self.attack_time,
            release_time: self.release_time,
            envelope: self.envelope,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
