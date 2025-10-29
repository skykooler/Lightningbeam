use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_VALUE: u32 = 0;

/// Constant CV source - outputs a constant voltage
/// Useful for providing fixed values to CV inputs, offsets, etc.
pub struct ConstantNode {
    name: String,
    value: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl ConstantNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_VALUE, "Value", -10.0, 10.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            value: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for ConstantNode {
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
            PARAM_VALUE => self.value = value.clamp(-10.0, 10.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_VALUE => self.value,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let length = output.len();

        // Fill output with constant value
        for i in 0..length {
            output[i] = self.value;
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Constant"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            value: self.value,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
