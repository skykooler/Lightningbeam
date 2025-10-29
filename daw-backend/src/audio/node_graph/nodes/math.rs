use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_OPERATION: u32 = 0;

/// Mathematical and logical operations on CV signals
/// Operations:
/// 0 = Add, 1 = Subtract, 2 = Multiply, 3 = Divide
/// 4 = Min, 5 = Max, 6 = Average
/// 7 = Invert (1.0 - x), 8 = Absolute Value
/// 9 = Clamp (0.0 to 1.0), 10 = Wrap (-1.0 to 1.0)
/// 11 = Greater Than, 12 = Less Than, 13 = Equal (with tolerance)
pub struct MathNode {
    name: String,
    operation: u32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl MathNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("CV In A", SignalType::CV, 0),
            NodePort::new("CV In B", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_OPERATION, "Operation", 0.0, 13.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            operation: 0,
            inputs,
            outputs,
            parameters,
        }
    }

    fn apply_operation(&self, a: f32, b: f32) -> f32 {
        match self.operation {
            0 => a + b,                                    // Add
            1 => a - b,                                    // Subtract
            2 => a * b,                                    // Multiply
            3 => if b.abs() > 0.0001 { a / b } else { 0.0 }, // Divide (with protection)
            4 => a.min(b),                                 // Min
            5 => a.max(b),                                 // Max
            6 => (a + b) * 0.5,                           // Average
            7 => 1.0 - a,                                  // Invert (ignores b)
            8 => a.abs(),                                  // Absolute Value (ignores b)
            9 => a.clamp(0.0, 1.0),                       // Clamp to 0-1 (ignores b)
            10 => {                                        // Wrap -1 to 1
                let mut result = a;
                while result > 1.0 {
                    result -= 2.0;
                }
                while result < -1.0 {
                    result += 2.0;
                }
                result
            },
            11 => if a > b { 1.0 } else { 0.0 },          // Greater Than
            12 => if a < b { 1.0 } else { 0.0 },          // Less Than
            13 => if (a - b).abs() < 0.01 { 1.0 } else { 0.0 }, // Equal (with tolerance)
            _ => a,                                        // Unknown operation - pass through
        }
    }
}

impl AudioNode for MathNode {
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
            PARAM_OPERATION => self.operation = (value as u32).clamp(0, 13),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_OPERATION => self.operation as f32,
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
        let length = output.len();

        // Process each sample
        for i in 0..length {
            // Get input A (or 0.0 if not connected)
            let a = if !inputs.is_empty() && i < inputs[0].len() {
                inputs[0][i]
            } else {
                0.0
            };

            // Get input B (or 0.0 if not connected)
            let b = if inputs.len() > 1 && i < inputs[1].len() {
                inputs[1][i]
            } else {
                0.0
            };

            output[i] = self.apply_operation(a, b);
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Math"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            operation: self.operation,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
