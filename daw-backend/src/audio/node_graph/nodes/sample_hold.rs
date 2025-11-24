use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// Sample & Hold - samples input CV when triggered by a gate signal
/// Classic modular synth utility for creating stepped sequences
pub struct SampleHoldNode {
    name: String,
    held_value: f32,
    last_gate: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl SampleHoldNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("CV In", SignalType::CV, 0),
            NodePort::new("Gate In", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![];

        Self {
            name,
            held_value: 0.0,
            last_gate: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for SampleHoldNode {
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
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let length = output.len();

        // Get CV input
        let cv_input = if !inputs.is_empty() && !inputs[0].is_empty() {
            inputs[0]
        } else {
            &[]
        };

        // Get Gate input
        let gate_input = if inputs.len() > 1 && !inputs[1].is_empty() {
            inputs[1]
        } else {
            &[]
        };

        // Process each sample
        for i in 0..length {
            let cv = if i < cv_input.len() { cv_input[i] } else { 0.0 };
            let gate = if i < gate_input.len() { gate_input[i] } else { 0.0 };

            // Detect rising edge (trigger)
            let gate_active = gate > 0.5;
            let last_gate_active = self.last_gate > 0.5;

            if gate_active && !last_gate_active {
                // Rising edge detected - sample the input
                self.held_value = cv;
            }

            self.last_gate = gate;
            output[i] = self.held_value;
        }
    }

    fn reset(&mut self) {
        self.held_value = 0.0;
        self.last_gate = 0.0;
    }

    fn node_type(&self) -> &str {
        "SampleHold"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            held_value: self.held_value,
            last_gate: self.last_gate,
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
