use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

// Parameters for the FM synth
const PARAM_ALGORITHM: u32 = 0;
const PARAM_OP1_RATIO: u32 = 1;
const PARAM_OP1_LEVEL: u32 = 2;
const PARAM_OP2_RATIO: u32 = 3;
const PARAM_OP2_LEVEL: u32 = 4;
const PARAM_OP3_RATIO: u32 = 5;
const PARAM_OP3_LEVEL: u32 = 6;
const PARAM_OP4_RATIO: u32 = 7;
const PARAM_OP4_LEVEL: u32 = 8;

/// FM Algorithm types (inspired by DX7)
/// Algorithm determines how operators modulate each other
#[derive(Debug, Clone, Copy, PartialEq)]
enum FMAlgorithm {
    /// Stack: 1->2->3->4 (most harmonic)
    Stack = 0,
    /// Parallel: All operators to output (organ-like)
    Parallel = 1,
    /// Bell: 1->2, 3->4, both to output
    Bell = 2,
    /// Dual: 1->2->output, 3->4->output
    Dual = 3,
}

impl FMAlgorithm {
    fn from_u32(value: u32) -> Self {
        match value {
            0 => FMAlgorithm::Stack,
            1 => FMAlgorithm::Parallel,
            2 => FMAlgorithm::Bell,
            3 => FMAlgorithm::Dual,
            _ => FMAlgorithm::Stack,
        }
    }
}

/// Single FM operator (oscillator)
struct FMOperator {
    phase: f32,
    frequency_ratio: f32,  // Multiplier of base frequency (e.g., 1.0, 2.0, 0.5)
    level: f32,            // Output amplitude 0.0-1.0
}

impl FMOperator {
    fn new() -> Self {
        Self {
            phase: 0.0,
            frequency_ratio: 1.0,
            level: 1.0,
        }
    }

    /// Process one sample with optional frequency modulation
    fn process(&mut self, base_freq: f32, modulation: f32, sample_rate: f32) -> f32 {
        let freq = base_freq * self.frequency_ratio;

        // Phase modulation (PM, which sounds like FM)
        let output = (self.phase * 2.0 * PI + modulation).sin() * self.level;

        // Advance phase
        self.phase += freq / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        output
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// 4-operator FM synthesizer node
pub struct FMSynthNode {
    name: String,
    algorithm: FMAlgorithm,

    // Four operators
    operators: [FMOperator; 4],

    // Current frequency from V/oct input
    current_frequency: f32,
    gate_active: bool,

    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl FMSynthNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),
            NodePort::new("Gate", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_ALGORITHM, "Algorithm", 0.0, 3.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP1_RATIO, "Op1 Ratio", 0.25, 16.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP1_LEVEL, "Op1 Level", 0.0, 1.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP2_RATIO, "Op2 Ratio", 0.25, 16.0, 2.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP2_LEVEL, "Op2 Level", 0.0, 1.0, 0.8, ParameterUnit::Generic),
            Parameter::new(PARAM_OP3_RATIO, "Op3 Ratio", 0.25, 16.0, 3.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP3_LEVEL, "Op3 Level", 0.0, 1.0, 0.6, ParameterUnit::Generic),
            Parameter::new(PARAM_OP4_RATIO, "Op4 Ratio", 0.25, 16.0, 4.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OP4_LEVEL, "Op4 Level", 0.0, 1.0, 0.4, ParameterUnit::Generic),
        ];

        Self {
            name,
            algorithm: FMAlgorithm::Stack,
            operators: [
                FMOperator::new(),
                FMOperator::new(),
                FMOperator::new(),
                FMOperator::new(),
            ],
            current_frequency: 440.0,
            gate_active: false,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Convert V/oct CV to frequency
    fn voct_to_freq(voct: f32) -> f32 {
        440.0 * 2.0_f32.powf(voct)
    }

    /// Process FM synthesis based on current algorithm
    fn process_algorithm(&mut self) -> f32 {
        if !self.gate_active {
            return 0.0;
        }

        let base_freq = self.current_frequency;
        let sr = self.sample_rate as f32;

        match self.algorithm {
            FMAlgorithm::Stack => {
                // 1 -> 2 -> 3 -> 4 -> output
                let op4_out = self.operators[3].process(base_freq, 0.0, sr);
                let op3_out = self.operators[2].process(base_freq, op4_out * 2.0, sr);
                let op2_out = self.operators[1].process(base_freq, op3_out * 2.0, sr);
                let op1_out = self.operators[0].process(base_freq, op2_out * 2.0, sr);
                op1_out
            }
            FMAlgorithm::Parallel => {
                // All operators output directly (no modulation)
                let op1_out = self.operators[0].process(base_freq, 0.0, sr);
                let op2_out = self.operators[1].process(base_freq, 0.0, sr);
                let op3_out = self.operators[2].process(base_freq, 0.0, sr);
                let op4_out = self.operators[3].process(base_freq, 0.0, sr);
                (op1_out + op2_out + op3_out + op4_out) * 0.25
            }
            FMAlgorithm::Bell => {
                // 1 -> 2, 3 -> 4, both to output
                let op2_out = self.operators[1].process(base_freq, 0.0, sr);
                let op1_out = self.operators[0].process(base_freq, op2_out * 2.0, sr);
                let op4_out = self.operators[3].process(base_freq, 0.0, sr);
                let op3_out = self.operators[2].process(base_freq, op4_out * 2.0, sr);
                (op1_out + op3_out) * 0.5
            }
            FMAlgorithm::Dual => {
                // 1 -> 2 -> output, 3 -> 4 -> output
                let op2_out = self.operators[1].process(base_freq, 0.0, sr);
                let op1_out = self.operators[0].process(base_freq, op2_out * 2.0, sr);
                let op4_out = self.operators[3].process(base_freq, 0.0, sr);
                let op3_out = self.operators[2].process(base_freq, op4_out * 2.0, sr);
                (op1_out + op3_out) * 0.5
            }
        }
    }
}

impl AudioNode for FMSynthNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Generator
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
            PARAM_ALGORITHM => {
                self.algorithm = FMAlgorithm::from_u32(value as u32);
            }
            PARAM_OP1_RATIO => self.operators[0].frequency_ratio = value.clamp(0.25, 16.0),
            PARAM_OP1_LEVEL => self.operators[0].level = value.clamp(0.0, 1.0),
            PARAM_OP2_RATIO => self.operators[1].frequency_ratio = value.clamp(0.25, 16.0),
            PARAM_OP2_LEVEL => self.operators[1].level = value.clamp(0.0, 1.0),
            PARAM_OP3_RATIO => self.operators[2].frequency_ratio = value.clamp(0.25, 16.0),
            PARAM_OP3_LEVEL => self.operators[2].level = value.clamp(0.0, 1.0),
            PARAM_OP4_RATIO => self.operators[3].frequency_ratio = value.clamp(0.25, 16.0),
            PARAM_OP4_LEVEL => self.operators[3].level = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ALGORITHM => self.algorithm as u32 as f32,
            PARAM_OP1_RATIO => self.operators[0].frequency_ratio,
            PARAM_OP1_LEVEL => self.operators[0].level,
            PARAM_OP2_RATIO => self.operators[1].frequency_ratio,
            PARAM_OP2_LEVEL => self.operators[1].level,
            PARAM_OP3_RATIO => self.operators[2].frequency_ratio,
            PARAM_OP3_LEVEL => self.operators[2].level,
            PARAM_OP4_RATIO => self.operators[3].frequency_ratio,
            PARAM_OP4_LEVEL => self.operators[3].level,
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

        self.sample_rate = sample_rate;

        let output = &mut outputs[0];
        let frames = output.len() / 2;

        for frame in 0..frames {
            // Read CV inputs
            let voct = if inputs.len() > 0 && !inputs[0].is_empty() {
                inputs[0][frame.min(inputs[0].len() / 2 - 1) * 2]
            } else {
                0.0
            };

            let gate = if inputs.len() > 1 && !inputs[1].is_empty() {
                inputs[1][frame.min(inputs[1].len() / 2 - 1) * 2]
            } else {
                0.0
            };

            // Update state
            self.current_frequency = Self::voct_to_freq(voct);
            self.gate_active = gate > 0.5;

            // Generate sample
            let sample = self.process_algorithm() * 0.3; // Scale down to prevent clipping

            // Output stereo (same signal to both channels)
            output[frame * 2] = sample;
            output[frame * 2 + 1] = sample;
        }
    }

    fn reset(&mut self) {
        for op in &mut self.operators {
            op.reset();
        }
        self.gate_active = false;
    }

    fn node_type(&self) -> &str {
        "FMSynth"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self::new(self.name.clone()))
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
