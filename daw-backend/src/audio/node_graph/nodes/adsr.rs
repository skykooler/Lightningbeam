use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_ATTACK: u32 = 0;
const PARAM_DECAY: u32 = 1;
const PARAM_SUSTAIN: u32 = 2;
const PARAM_RELEASE: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// ADSR Envelope Generator
/// Outputs a CV signal (0.0-1.0) based on gate input and ADSR parameters
pub struct ADSRNode {
    name: String,
    attack: f32,    // seconds
    decay: f32,     // seconds
    sustain: f32,   // level (0.0-1.0)
    release: f32,   // seconds
    stage: EnvelopeStage,
    level: f32,     // current envelope level
    gate_was_high: bool,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl ADSRNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Gate", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("Envelope Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_ATTACK, "Attack", 0.001, 5.0, 0.01, ParameterUnit::Time),
            Parameter::new(PARAM_DECAY, "Decay", 0.001, 5.0, 0.1, ParameterUnit::Time),
            Parameter::new(PARAM_SUSTAIN, "Sustain", 0.0, 1.0, 0.7, ParameterUnit::Generic),
            Parameter::new(PARAM_RELEASE, "Release", 0.001, 5.0, 0.2, ParameterUnit::Time),
        ];

        Self {
            name,
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
            stage: EnvelopeStage::Idle,
            level: 0.0,
            gate_was_high: false,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for ADSRNode {
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
            PARAM_ATTACK => self.attack = value.clamp(0.001, 5.0),
            PARAM_DECAY => self.decay = value.clamp(0.001, 5.0),
            PARAM_SUSTAIN => self.sustain = value.clamp(0.0, 1.0),
            PARAM_RELEASE => self.release = value.clamp(0.001, 5.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ATTACK => self.attack,
            PARAM_DECAY => self.decay,
            PARAM_SUSTAIN => self.sustain,
            PARAM_RELEASE => self.release,
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
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let sample_rate_f32 = sample_rate as f32;

        // CV signals are mono
        let frames = output.len();

        for frame in 0..frames {
            // Read gate input (if available)
            let gate_high = if !inputs.is_empty() && frame < inputs[0].len() {
                inputs[0][frame] > 0.5 // Gate is high if CV > 0.5
            } else {
                false
            };

            // Detect gate transitions
            if gate_high && !self.gate_was_high {
                // Note on: Start attack
                self.stage = EnvelopeStage::Attack;
            } else if !gate_high && self.gate_was_high {
                // Note off: Start release
                self.stage = EnvelopeStage::Release;
            }
            self.gate_was_high = gate_high;

            // Process envelope stage
            match self.stage {
                EnvelopeStage::Idle => {
                    self.level = 0.0;
                }
                EnvelopeStage::Attack => {
                    // Rise from current level to 1.0
                    let increment = 1.0 / (self.attack * sample_rate_f32);
                    self.level += increment;
                    if self.level >= 1.0 {
                        self.level = 1.0;
                        self.stage = EnvelopeStage::Decay;
                    }
                }
                EnvelopeStage::Decay => {
                    // Fall from 1.0 to sustain level
                    let target = self.sustain;
                    let decrement = (1.0 - target) / (self.decay * sample_rate_f32);
                    self.level -= decrement;
                    if self.level <= target {
                        self.level = target;
                        self.stage = EnvelopeStage::Sustain;
                    }
                }
                EnvelopeStage::Sustain => {
                    // Hold at sustain level
                    self.level = self.sustain;
                }
                EnvelopeStage::Release => {
                    // Fall from current level to 0.0
                    let decrement = self.level / (self.release * sample_rate_f32);
                    self.level -= decrement;
                    if self.level <= 0.001 {
                        self.level = 0.0;
                        self.stage = EnvelopeStage::Idle;
                    }
                }
            }

            // Write envelope value (CV is mono)
            output[frame] = self.level;
        }
    }

    fn reset(&mut self) {
        self.stage = EnvelopeStage::Idle;
        self.level = 0.0;
        self.gate_was_high = false;
    }

    fn node_type(&self) -> &str {
        "ADSR"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            attack: self.attack,
            decay: self.decay,
            sustain: self.sustain,
            release: self.release,
            stage: EnvelopeStage::Idle, // Reset state
            level: 0.0,                 // Reset level
            gate_was_high: false,       // Reset gate
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
