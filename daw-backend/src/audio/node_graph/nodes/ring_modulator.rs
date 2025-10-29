use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_MIX: u32 = 0;

/// Ring Modulator - multiplies two signals together
/// Creates metallic, inharmonic timbres by multiplying carrier and modulator
pub struct RingModulatorNode {
    name: String,
    mix: f32,  // 0.0 = dry (carrier only), 1.0 = fully modulated
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl RingModulatorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Carrier", SignalType::Audio, 0),
            NodePort::new("Modulator", SignalType::Audio, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_MIX, "Mix", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            mix: 1.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for RingModulatorNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Effect
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
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_MIX => self.mix,
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

        // Get carrier input
        let carrier = if !inputs.is_empty() && !inputs[0].is_empty() {
            inputs[0]
        } else {
            &[]
        };

        // Get modulator input
        let modulator = if inputs.len() > 1 && !inputs[1].is_empty() {
            inputs[1]
        } else {
            &[]
        };

        // Process each sample
        for i in 0..length {
            let carrier_sample = if i < carrier.len() { carrier[i] } else { 0.0 };
            let modulator_sample = if i < modulator.len() { modulator[i] } else { 0.0 };

            // Ring modulation: multiply the two signals
            let modulated = carrier_sample * modulator_sample;

            // Mix between dry (carrier) and wet (modulated)
            output[i] = carrier_sample * (1.0 - self.mix) + modulated * self.mix;
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "RingModulator"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            mix: self.mix,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
