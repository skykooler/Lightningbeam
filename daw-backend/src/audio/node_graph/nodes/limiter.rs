use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_THRESHOLD: u32 = 0;
const PARAM_RELEASE: u32 = 1;
const PARAM_CEILING: u32 = 2;

/// Limiter node for preventing audio peaks from exceeding a threshold
/// Essentially a compressor with infinite ratio and very fast attack
pub struct LimiterNode {
    name: String,
    threshold_db: f32,
    release_ms: f32,
    ceiling_db: f32,

    // State
    envelope: f32,
    release_coeff: f32,
    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl LimiterNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_THRESHOLD, "Threshold", -60.0, 0.0, -1.0, ParameterUnit::Decibels),
            Parameter::new(PARAM_RELEASE, "Release", 1.0, 500.0, 50.0, ParameterUnit::Time),
            Parameter::new(PARAM_CEILING, "Ceiling", -60.0, 0.0, 0.0, ParameterUnit::Decibels),
        ];

        let sample_rate = 44100;
        let release_coeff = Self::ms_to_coeff(50.0, sample_rate);

        Self {
            name,
            threshold_db: -1.0,
            release_ms: 50.0,
            ceiling_db: 0.0,
            envelope: 0.0,
            release_coeff,
            sample_rate,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Convert milliseconds to exponential smoothing coefficient
    fn ms_to_coeff(time_ms: f32, sample_rate: u32) -> f32 {
        let time_seconds = time_ms / 1000.0;
        let samples = time_seconds * sample_rate as f32;
        (-1.0 / samples).exp()
    }

    fn update_coefficients(&mut self) {
        self.release_coeff = Self::ms_to_coeff(self.release_ms, self.sample_rate);
    }

    /// Convert linear amplitude to dB
    fn linear_to_db(linear: f32) -> f32 {
        if linear > 0.0 {
            20.0 * linear.log10()
        } else {
            -160.0
        }
    }

    /// Convert dB to linear gain
    fn db_to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    fn process_sample(&mut self, input: f32) -> f32 {
        // Detect input level (using absolute value as peak detector)
        let input_level = input.abs();

        // Convert to dB
        let input_db = Self::linear_to_db(input_level);

        // Calculate gain reduction needed
        // If above threshold, apply infinite ratio (hard limit)
        let target_gr_db = if input_db > self.threshold_db {
            input_db - self.threshold_db  // Amount of overshoot to reduce
        } else {
            0.0
        };

        let target_gr_linear = Self::db_to_linear(-target_gr_db);

        // Very fast attack (instant for limiter), but slower release
        // Attack coeff is very close to 0 for near-instant response
        let attack_coeff = 0.0001; // Extremely fast attack

        let coeff = if target_gr_linear < self.envelope {
            attack_coeff // Attack (instant response to louder signal)
        } else {
            self.release_coeff // Release (slower recovery)
        };

        self.envelope = target_gr_linear + coeff * (self.envelope - target_gr_linear);

        // Apply limiting and output ceiling
        let limited = input * self.envelope;
        let ceiling_linear = Self::db_to_linear(self.ceiling_db);

        // Hard clip at ceiling
        limited.clamp(-ceiling_linear, ceiling_linear)
    }
}

impl AudioNode for LimiterNode {
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
            PARAM_THRESHOLD => self.threshold_db = value,
            PARAM_RELEASE => {
                self.release_ms = value;
                self.update_coefficients();
            }
            PARAM_CEILING => self.ceiling_db = value,
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_THRESHOLD => self.threshold_db,
            PARAM_RELEASE => self.release_ms,
            PARAM_CEILING => self.ceiling_db,
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

        // Update sample rate if changed
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.update_coefficients();
        }

        let input = inputs[0];
        let output = &mut outputs[0];
        let len = input.len().min(output.len());

        for i in 0..len {
            output[i] = self.process_sample(input[i]);
        }
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
    }

    fn node_type(&self) -> &str {
        "Limiter"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            threshold_db: self.threshold_db,
            release_ms: self.release_ms,
            ceiling_db: self.ceiling_db,
            envelope: 0.0, // Reset state for clone
            release_coeff: self.release_coeff,
            sample_rate: self.sample_rate,
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
