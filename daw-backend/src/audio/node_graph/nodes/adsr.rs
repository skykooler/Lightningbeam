use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType, cv_input_or_default};
use crate::audio::midi::MidiEvent;

const PARAM_ATTACK: u32 = 0;
const PARAM_DECAY: u32 = 1;
const PARAM_SUSTAIN: u32 = 2;
const PARAM_RELEASE: u32 = 3;
const PARAM_CURVE: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
enum EnvelopeStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Curve shape for envelope segments
#[derive(Debug, Clone, Copy, PartialEq)]
enum CurveType {
    Linear,
    Exponential,
}

impl CurveType {
    fn from_f32(v: f32) -> Self {
        if v >= 0.5 { CurveType::Exponential } else { CurveType::Linear }
    }
}

/// ADSR Envelope Generator
/// Outputs a CV signal (0.0-1.0) based on gate input and ADSR parameters
pub struct ADSRNode {
    name: String,
    attack: f32,    // seconds
    decay: f32,     // seconds
    sustain: f32,   // level (0.0-1.0)
    release: f32,   // seconds
    curve: CurveType,
    stage: EnvelopeStage,
    level: f32,     // current envelope level
    /// For exponential curves: the coefficient per sample (computed on stage entry)
    exp_coeff: f32,
    /// For exponential curves: the base level when the stage started
    exp_base: f32,
    /// For exponential curves: the target level
    exp_target: f32,
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
            Parameter::new(PARAM_CURVE, "Curve", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
            curve: CurveType::Linear,
            stage: EnvelopeStage::Idle,
            level: 0.0,
            exp_coeff: 0.0,
            exp_base: 0.0,
            exp_target: 0.0,
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
            PARAM_CURVE => self.curve = CurveType::from_f32(value),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ATTACK => self.attack,
            PARAM_DECAY => self.decay,
            PARAM_SUSTAIN => self.sustain,
            PARAM_RELEASE => self.release,
            PARAM_CURVE => match self.curve { CurveType::Linear => 0.0, CurveType::Exponential => 1.0 },
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
            // Gate input: when unconnected, defaults to 0.0 (off)
            let gate_cv = cv_input_or_default(inputs, 0, frame, 0.0);
            let gate_high = gate_cv > 0.5;

            // Detect gate transitions
            if gate_high && !self.gate_was_high {
                // Note on: Start attack
                self.stage = EnvelopeStage::Attack;
                if self.curve == CurveType::Exponential {
                    // For exponential attack, compute coefficient for ~5 time constants
                    // We overshoot the target slightly so the curve reaches 1.0 naturally
                    let samples = self.attack * sample_rate_f32;
                    self.exp_coeff = (-5.0 / samples).exp();
                    self.exp_base = self.level;
                    self.exp_target = 1.0;
                }
            } else if !gate_high && self.gate_was_high {
                // Note off: Start release
                self.stage = EnvelopeStage::Release;
                if self.curve == CurveType::Exponential {
                    let samples = self.release * sample_rate_f32;
                    self.exp_coeff = (-5.0 / samples).exp();
                    self.exp_base = self.level;
                    self.exp_target = 0.0;
                }
            }
            self.gate_was_high = gate_high;

            // Process envelope stage
            match self.stage {
                EnvelopeStage::Idle => {
                    self.level = 0.0;
                }
                EnvelopeStage::Attack => {
                    match self.curve {
                        CurveType::Linear => {
                            let increment = 1.0 / (self.attack * sample_rate_f32);
                            self.level += increment;
                            if self.level >= 1.0 {
                                self.level = 1.0;
                                self.stage = EnvelopeStage::Decay;
                            }
                        }
                        CurveType::Exponential => {
                            // Asymptotic approach: level moves toward overshoot target
                            // Using target of 1.0 + small overshoot so we actually reach 1.0
                            let overshoot_target = 1.0 + (1.0 - self.exp_base) * 0.01;
                            self.level = overshoot_target - (overshoot_target - self.level) * self.exp_coeff;
                            if self.level >= 1.0 {
                                self.level = 1.0;
                                self.stage = EnvelopeStage::Decay;
                                // Set up decay exponential
                                let samples = self.decay * sample_rate_f32;
                                self.exp_coeff = (-5.0 / samples).exp();
                                self.exp_base = 1.0;
                                self.exp_target = self.sustain;
                            }
                        }
                    }
                }
                EnvelopeStage::Decay => {
                    let target = self.sustain;
                    match self.curve {
                        CurveType::Linear => {
                            let decrement = (1.0 - target) / (self.decay * sample_rate_f32);
                            self.level -= decrement;
                            if self.level <= target {
                                self.level = target;
                                self.stage = EnvelopeStage::Sustain;
                            }
                        }
                        CurveType::Exponential => {
                            // Exponential decay toward sustain level
                            self.level = target + (self.level - target) * self.exp_coeff;
                            if (self.level - target).abs() < 0.001 {
                                self.level = target;
                                self.stage = EnvelopeStage::Sustain;
                            }
                        }
                    }
                }
                EnvelopeStage::Sustain => {
                    // Hold at sustain level
                    self.level = self.sustain;
                }
                EnvelopeStage::Release => {
                    match self.curve {
                        CurveType::Linear => {
                            let decrement = self.level / (self.release * sample_rate_f32);
                            self.level -= decrement;
                            if self.level <= 0.001 {
                                self.level = 0.0;
                                self.stage = EnvelopeStage::Idle;
                            }
                        }
                        CurveType::Exponential => {
                            // Exponential decay toward 0
                            self.level *= self.exp_coeff;
                            if self.level <= 0.001 {
                                self.level = 0.0;
                                self.stage = EnvelopeStage::Idle;
                            }
                        }
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
        self.exp_coeff = 0.0;
        self.exp_base = 0.0;
        self.exp_target = 0.0;
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
            curve: self.curve,
            stage: EnvelopeStage::Idle,
            level: 0.0,
            exp_coeff: 0.0,
            exp_base: 0.0,
            exp_target: 0.0,
            gate_was_high: false,
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
