use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_RISE_TIME: u32 = 0;
const PARAM_FALL_TIME: u32 = 1;

/// Slew limiter - limits the rate of change of a CV signal
/// Useful for creating portamento/glide effects and smoothing control signals
pub struct SlewLimiterNode {
    name: String,
    rise_time: f32,  // Time in seconds to rise from 0 to 1
    fall_time: f32,  // Time in seconds to fall from 1 to 0
    last_value: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl SlewLimiterNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("CV In", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_RISE_TIME, "Rise Time", 0.0, 5.0, 0.01, ParameterUnit::Time),
            Parameter::new(PARAM_FALL_TIME, "Fall Time", 0.0, 5.0, 0.01, ParameterUnit::Time),
        ];

        Self {
            name,
            rise_time: 0.01,
            fall_time: 0.01,
            last_value: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for SlewLimiterNode {
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
            PARAM_RISE_TIME => self.rise_time = value.clamp(0.0, 5.0),
            PARAM_FALL_TIME => self.fall_time = value.clamp(0.0, 5.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_RISE_TIME => self.rise_time,
            PARAM_FALL_TIME => self.fall_time,
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
        let length = input.len().min(output.len());

        // Calculate maximum change per sample
        let sample_duration = 1.0 / sample_rate as f32;

        // Rise/fall rates (units per second)
        let rise_rate = if self.rise_time > 0.0001 {
            1.0 / self.rise_time
        } else {
            f32::MAX // No limiting
        };

        let fall_rate = if self.fall_time > 0.0001 {
            1.0 / self.fall_time
        } else {
            f32::MAX // No limiting
        };

        for i in 0..length {
            let target = input[i];
            let difference = target - self.last_value;

            let max_change = if difference > 0.0 {
                // Rising
                rise_rate * sample_duration
            } else {
                // Falling
                fall_rate * sample_duration
            };

            // Limit the change
            let limited_difference = difference.clamp(-max_change, max_change);
            self.last_value += limited_difference;

            output[i] = self.last_value;
        }
    }

    fn reset(&mut self) {
        self.last_value = 0.0;
    }

    fn node_type(&self) -> &str {
        "SlewLimiter"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            rise_time: self.rise_time,
            fall_time: self.fall_time,
            last_value: self.last_value,
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
