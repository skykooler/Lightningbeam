use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// Template Input node - represents the MIDI input for one voice in a VoiceAllocator
pub struct TemplateInputNode {
    name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl TemplateInputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![];
        let outputs = vec![
            NodePort::new("MIDI Out", SignalType::Midi, 0),
        ];

        Self {
            name,
            inputs,
            outputs,
            parameters: vec![],
        }
    }
}

impl AudioNode for TemplateInputNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Input
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
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        // TemplateInput receives MIDI from VoiceAllocator and outputs it
        // The MIDI was already placed in midi_outputs by the graph before calling process()
        // So there's nothing to do here - the MIDI is already in the output buffer
    }

    fn reset(&mut self) {}

    fn node_type(&self) -> &str {
        "TemplateInput"
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

    fn handle_midi(&mut self, _event: &MidiEvent) {
        // Pass through to connected nodes
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Template Output node - represents the audio output from one voice in a VoiceAllocator
pub struct TemplateOutputNode {
    name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl TemplateOutputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];
        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        Self {
            name,
            inputs,
            outputs,
            parameters: vec![],
        }
    }
}

impl AudioNode for TemplateOutputNode {
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
        // Copy input to output - the graph reads from output buffers
        if !inputs.is_empty() && !outputs.is_empty() {
            let input = inputs[0];
            let output = &mut outputs[0];
            let len = input.len().min(output.len());
            output[..len].copy_from_slice(&input[..len]);
        }
    }

    fn reset(&mut self) {}

    fn node_type(&self) -> &str {
        "TemplateOutput"
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
