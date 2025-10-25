use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_GAIN: u32 = 0;

/// Gain/volume control node
pub struct GainNode {
    name: String,
    gain: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl GainNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("Gain CV", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_GAIN, "Gain", 0.0, 2.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            gain: 1.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for GainNode {
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
            PARAM_GAIN => self.gain = value.clamp(0.0, 2.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_GAIN => self.gain,
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
        if inputs.is_empty() || outputs.is_empty() {
            return;
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        // Process by frames, not samples
        let frames = input.len().min(output.len()) / 2;

        for frame in 0..frames {
            // Calculate final gain
            let mut final_gain = self.gain;

            // CV input acts as a VCA (voltage-controlled amplifier)
            // CV ranges from 0.0 (silence) to 1.0 (full gain parameter value)
            if inputs.len() > 1 && frame < inputs[1].len() {
                let cv = inputs[1][frame];
                final_gain *= cv;  // Multiply gain by CV (0.0 = silence, 1.0 = full gain)
            }

            // Apply gain to both channels
            output[frame * 2] = input[frame * 2] * final_gain;         // Left
            output[frame * 2 + 1] = input[frame * 2 + 1] * final_gain; // Right
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Gain"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            gain: self.gain,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
