use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_GAIN_1: u32 = 0;
const PARAM_GAIN_2: u32 = 1;
const PARAM_GAIN_3: u32 = 2;
const PARAM_GAIN_4: u32 = 3;

/// Mixer node - combines multiple audio inputs with independent gain controls
pub struct MixerNode {
    name: String,
    gains: [f32; 4],
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl MixerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Input 1", SignalType::Audio, 0),
            NodePort::new("Input 2", SignalType::Audio, 1),
            NodePort::new("Input 3", SignalType::Audio, 2),
            NodePort::new("Input 4", SignalType::Audio, 3),
        ];

        let outputs = vec![
            NodePort::new("Mixed Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_GAIN_1, "Gain 1", 0.0, 2.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_GAIN_2, "Gain 2", 0.0, 2.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_GAIN_3, "Gain 3", 0.0, 2.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_GAIN_4, "Gain 4", 0.0, 2.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            gains: [1.0, 1.0, 1.0, 1.0],
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for MixerNode {
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
            PARAM_GAIN_1 => self.gains[0] = value.clamp(0.0, 2.0),
            PARAM_GAIN_2 => self.gains[1] = value.clamp(0.0, 2.0),
            PARAM_GAIN_3 => self.gains[2] = value.clamp(0.0, 2.0),
            PARAM_GAIN_4 => self.gains[3] = value.clamp(0.0, 2.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_GAIN_1 => self.gains[0],
            PARAM_GAIN_2 => self.gains[1],
            PARAM_GAIN_3 => self.gains[2],
            PARAM_GAIN_4 => self.gains[3],
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = output.len() / 2;

        // Clear output buffer first
        output.fill(0.0);

        // Mix each input with its gain
        for (input_idx, input) in inputs.iter().enumerate().take(4) {
            if input_idx >= self.gains.len() {
                break;
            }

            let gain = self.gains[input_idx];
            let input_frames = input.len() / 2;
            let process_frames = frames.min(input_frames);

            for frame in 0..process_frames {
                output[frame * 2] += input[frame * 2] * gain;         // Left
                output[frame * 2 + 1] += input[frame * 2 + 1] * gain; // Right
            }
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Mixer"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            gains: self.gains,
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
