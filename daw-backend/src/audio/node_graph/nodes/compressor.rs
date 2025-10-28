use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_THRESHOLD: u32 = 0;
const PARAM_RATIO: u32 = 1;
const PARAM_ATTACK: u32 = 2;
const PARAM_RELEASE: u32 = 3;
const PARAM_MAKEUP_GAIN: u32 = 4;
const PARAM_KNEE: u32 = 5;

/// Compressor node for dynamic range compression
pub struct CompressorNode {
    name: String,
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_gain_db: f32,
    knee_db: f32,

    // State
    envelope: f32,
    attack_coeff: f32,
    release_coeff: f32,
    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl CompressorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_THRESHOLD, "Threshold", -60.0, 0.0, -20.0, ParameterUnit::Decibels),
            Parameter::new(PARAM_RATIO, "Ratio", 1.0, 20.0, 4.0, ParameterUnit::Generic),
            Parameter::new(PARAM_ATTACK, "Attack", 0.1, 100.0, 5.0, ParameterUnit::Time),
            Parameter::new(PARAM_RELEASE, "Release", 10.0, 1000.0, 50.0, ParameterUnit::Time),
            Parameter::new(PARAM_MAKEUP_GAIN, "Makeup", 0.0, 24.0, 0.0, ParameterUnit::Decibels),
            Parameter::new(PARAM_KNEE, "Knee", 0.0, 12.0, 3.0, ParameterUnit::Decibels),
        ];

        let sample_rate = 44100;
        let attack_coeff = Self::ms_to_coeff(5.0, sample_rate);
        let release_coeff = Self::ms_to_coeff(50.0, sample_rate);

        Self {
            name,
            threshold_db: -20.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 50.0,
            makeup_gain_db: 0.0,
            knee_db: 3.0,
            envelope: 0.0,
            attack_coeff,
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
        self.attack_coeff = Self::ms_to_coeff(self.attack_ms, self.sample_rate);
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

    /// Calculate gain reduction for a given input level
    fn calculate_gain_reduction(&self, input_db: f32) -> f32 {
        let threshold = self.threshold_db;
        let knee = self.knee_db;
        let ratio = self.ratio;

        // Soft knee implementation
        if input_db < threshold - knee / 2.0 {
            // Below threshold - no compression
            0.0
        } else if input_db > threshold + knee / 2.0 {
            // Above threshold - full compression
            let overshoot = input_db - threshold;
            overshoot * (1.0 - 1.0 / ratio)
        } else {
            // In knee region - gradual compression
            let overshoot = input_db - threshold + knee / 2.0;
            let knee_factor = overshoot / knee;
            overshoot * knee_factor * (1.0 - 1.0 / ratio) / 2.0
        }
    }

    fn process_sample(&mut self, input: f32) -> f32 {
        // Detect input level (using absolute value as simple peak detector)
        let input_level = input.abs();

        // Convert to dB
        let input_db = Self::linear_to_db(input_level);

        // Calculate target gain reduction
        let target_gr_db = self.calculate_gain_reduction(input_db);
        let target_gr_linear = Self::db_to_linear(-target_gr_db);

        // Smooth envelope with attack/release
        let coeff = if target_gr_linear < self.envelope {
            self.attack_coeff // Attack (faster response to louder signal)
        } else {
            self.release_coeff // Release (slower response when signal gets quieter)
        };

        self.envelope = target_gr_linear + coeff * (self.envelope - target_gr_linear);

        // Apply compression and makeup gain
        let makeup_linear = Self::db_to_linear(self.makeup_gain_db);
        input * self.envelope * makeup_linear
    }
}

impl AudioNode for CompressorNode {
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
            PARAM_RATIO => self.ratio = value,
            PARAM_ATTACK => {
                self.attack_ms = value;
                self.update_coefficients();
            }
            PARAM_RELEASE => {
                self.release_ms = value;
                self.update_coefficients();
            }
            PARAM_MAKEUP_GAIN => self.makeup_gain_db = value,
            PARAM_KNEE => self.knee_db = value,
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_THRESHOLD => self.threshold_db,
            PARAM_RATIO => self.ratio,
            PARAM_ATTACK => self.attack_ms,
            PARAM_RELEASE => self.release_ms,
            PARAM_MAKEUP_GAIN => self.makeup_gain_db,
            PARAM_KNEE => self.knee_db,
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
        "Compressor"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            threshold_db: self.threshold_db,
            ratio: self.ratio,
            attack_ms: self.attack_ms,
            release_ms: self.release_ms,
            makeup_gain_db: self.makeup_gain_db,
            knee_db: self.knee_db,
            envelope: 0.0, // Reset state for clone
            attack_coeff: self.attack_coeff,
            release_coeff: self.release_coeff,
            sample_rate: self.sample_rate,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
