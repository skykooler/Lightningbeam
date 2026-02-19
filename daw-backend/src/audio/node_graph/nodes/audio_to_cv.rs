use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// Audio to CV converter
/// Directly converts a stereo audio signal to mono CV (averages L+R channels)
pub struct AudioToCVNode {
    name: String,
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

        Self {
            name,
            inputs,
            outputs,
            parameters: Vec::new(),
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

    fn set_parameter(&mut self, _id: u32, _value: f32) {}

    fn get_parameter(&self, _id: u32) -> f32 {
        0.0
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

        // Audio input is stereo (interleaved L/R), CV output is mono
        let audio_frames = input.len() / 2;
        let frames = audio_frames.min(output.len());

        for frame in 0..frames {
            let left = input[frame * 2];
            let right = input[frame * 2 + 1];
            output[frame] = (left + right) * 0.5;
        }
    }

    fn reset(&mut self) {}

    fn node_type(&self) -> &str {
        "AudioToCV"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
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
