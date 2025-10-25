use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// Audio output node - collects audio and passes it to the main output
pub struct AudioOutputNode {
    name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
}

impl AudioOutputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        // Output node has an output for graph consistency, but it's typically the final node
        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        Self {
            name,
            inputs,
            outputs,
        }
    }
}

impl AudioNode for AudioOutputNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Output
    }

    fn inputs(&self) -> &[NodePort] {
        &self.inputs
    }

    fn outputs(&self) -> &[NodePort] {
        &self.outputs
    }

    fn parameters(&self) -> &[Parameter] {
        &[] // No parameters
    }

    fn set_parameter(&mut self, _id: u32, _value: f32) {
        // No parameters
    }

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

        // Simply pass through the input to the output
        let input = inputs[0];
        let output = &mut outputs[0];
        let len = input.len().min(output.len());

        output[..len].copy_from_slice(&input[..len]);
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "AudioOutput"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
        })
    }
}
