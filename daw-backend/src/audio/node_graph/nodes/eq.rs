use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use crate::dsp::biquad::BiquadFilter;

// Low band (shelving)
const PARAM_LOW_FREQ: u32 = 0;
const PARAM_LOW_GAIN: u32 = 1;

// Mid band (peaking)
const PARAM_MID_FREQ: u32 = 2;
const PARAM_MID_GAIN: u32 = 3;
const PARAM_MID_Q: u32 = 4;

// High band (shelving)
const PARAM_HIGH_FREQ: u32 = 5;
const PARAM_HIGH_GAIN: u32 = 6;

/// 3-Band Parametric EQ Node
/// All three bands use peaking filters at different frequencies
pub struct EQNode {
    name: String,

    // Parameters
    low_freq: f32,
    low_gain_db: f32,
    low_q: f32,
    mid_freq: f32,
    mid_gain_db: f32,
    mid_q: f32,
    high_freq: f32,
    high_gain_db: f32,
    high_q: f32,

    // Filters (stereo)
    low_filter_left: BiquadFilter,
    low_filter_right: BiquadFilter,
    mid_filter_left: BiquadFilter,
    mid_filter_right: BiquadFilter,
    high_filter_left: BiquadFilter,
    high_filter_right: BiquadFilter,

    sample_rate: u32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl EQNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_LOW_FREQ, "Low Freq", 20.0, 500.0, 100.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_LOW_GAIN, "Low Gain", -24.0, 24.0, 0.0, ParameterUnit::Decibels),
            Parameter::new(PARAM_MID_FREQ, "Mid Freq", 200.0, 5000.0, 1000.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_MID_GAIN, "Mid Gain", -24.0, 24.0, 0.0, ParameterUnit::Decibels),
            Parameter::new(PARAM_MID_Q, "Mid Q", 0.1, 10.0, 0.707, ParameterUnit::Generic),
            Parameter::new(PARAM_HIGH_FREQ, "High Freq", 2000.0, 20000.0, 8000.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_HIGH_GAIN, "High Gain", -24.0, 24.0, 0.0, ParameterUnit::Decibels),
        ];

        let sample_rate = 44100;

        // Initialize filters - all peaking
        let low_filter_left = BiquadFilter::peaking(100.0, 1.0, 0.0, sample_rate as f32);
        let low_filter_right = BiquadFilter::peaking(100.0, 1.0, 0.0, sample_rate as f32);
        let mid_filter_left = BiquadFilter::peaking(1000.0, 0.707, 0.0, sample_rate as f32);
        let mid_filter_right = BiquadFilter::peaking(1000.0, 0.707, 0.0, sample_rate as f32);
        let high_filter_left = BiquadFilter::peaking(8000.0, 1.0, 0.0, sample_rate as f32);
        let high_filter_right = BiquadFilter::peaking(8000.0, 1.0, 0.0, sample_rate as f32);

        Self {
            name,
            low_freq: 100.0,
            low_gain_db: 0.0,
            low_q: 1.0,
            mid_freq: 1000.0,
            mid_gain_db: 0.0,
            mid_q: 0.707,
            high_freq: 8000.0,
            high_gain_db: 0.0,
            high_q: 1.0,
            low_filter_left,
            low_filter_right,
            mid_filter_left,
            mid_filter_right,
            high_filter_left,
            high_filter_right,
            sample_rate,
            inputs,
            outputs,
            parameters,
        }
    }

    fn update_filters(&mut self) {
        let sr = self.sample_rate as f32;

        // Update low band peaking filter
        self.low_filter_left.set_peaking(self.low_freq, self.low_q, self.low_gain_db, sr);
        self.low_filter_right.set_peaking(self.low_freq, self.low_q, self.low_gain_db, sr);

        // Update mid band peaking filter
        self.mid_filter_left.set_peaking(self.mid_freq, self.mid_q, self.mid_gain_db, sr);
        self.mid_filter_right.set_peaking(self.mid_freq, self.mid_q, self.mid_gain_db, sr);

        // Update high band peaking filter
        self.high_filter_left.set_peaking(self.high_freq, self.high_q, self.high_gain_db, sr);
        self.high_filter_right.set_peaking(self.high_freq, self.high_q, self.high_gain_db, sr);
    }
}

impl AudioNode for EQNode {
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
            PARAM_LOW_FREQ => {
                self.low_freq = value;
                self.update_filters();
            }
            PARAM_LOW_GAIN => {
                self.low_gain_db = value;
                self.update_filters();
            }
            PARAM_MID_FREQ => {
                self.mid_freq = value;
                self.update_filters();
            }
            PARAM_MID_GAIN => {
                self.mid_gain_db = value;
                self.update_filters();
            }
            PARAM_MID_Q => {
                self.mid_q = value;
                self.update_filters();
            }
            PARAM_HIGH_FREQ => {
                self.high_freq = value;
                self.update_filters();
            }
            PARAM_HIGH_GAIN => {
                self.high_gain_db = value;
                self.update_filters();
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_LOW_FREQ => self.low_freq,
            PARAM_LOW_GAIN => self.low_gain_db,
            PARAM_MID_FREQ => self.mid_freq,
            PARAM_MID_GAIN => self.mid_gain_db,
            PARAM_MID_Q => self.mid_q,
            PARAM_HIGH_FREQ => self.high_freq,
            PARAM_HIGH_GAIN => self.high_gain_db,
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
            self.update_filters();
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        for frame in 0..frames_to_process {
            let mut left = input[frame * 2];
            let mut right = input[frame * 2 + 1];

            // Process through all three bands
            left = self.low_filter_left.process_sample(left, 0);
            left = self.mid_filter_left.process_sample(left, 0);
            left = self.high_filter_left.process_sample(left, 0);

            right = self.low_filter_right.process_sample(right, 1);
            right = self.mid_filter_right.process_sample(right, 1);
            right = self.high_filter_right.process_sample(right, 1);

            output[frame * 2] = left;
            output[frame * 2 + 1] = right;
        }
    }

    fn reset(&mut self) {
        self.low_filter_left.reset();
        self.low_filter_right.reset();
        self.mid_filter_left.reset();
        self.mid_filter_right.reset();
        self.high_filter_left.reset();
        self.high_filter_right.reset();
    }

    fn node_type(&self) -> &str {
        "EQ"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        let mut node = Self::new(self.name.clone());
        node.low_freq = self.low_freq;
        node.low_gain_db = self.low_gain_db;
        node.mid_freq = self.mid_freq;
        node.mid_gain_db = self.mid_gain_db;
        node.mid_q = self.mid_q;
        node.high_freq = self.high_freq;
        node.high_gain_db = self.high_gain_db;
        node.sample_rate = self.sample_rate;
        node.update_filters();
        Box::new(node)
    }
}
