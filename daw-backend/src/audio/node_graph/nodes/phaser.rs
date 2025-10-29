use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;
const PARAM_STAGES: u32 = 2;
const PARAM_FEEDBACK: u32 = 3;
const PARAM_WET_DRY: u32 = 4;

const MAX_STAGES: usize = 8;

/// First-order all-pass filter for phaser
struct AllPassFilter {
    a1: f32,
    zm1_left: f32,
    zm1_right: f32,
}

impl AllPassFilter {
    fn new() -> Self {
        Self {
            a1: 0.0,
            zm1_left: 0.0,
            zm1_right: 0.0,
        }
    }

    fn set_coefficient(&mut self, frequency: f32, sample_rate: f32) {
        // First-order all-pass coefficient
        // a1 = (tan(π*f/fs) - 1) / (tan(π*f/fs) + 1)
        let tan_val = ((PI * frequency) / sample_rate).tan();
        self.a1 = (tan_val - 1.0) / (tan_val + 1.0);
    }

    fn process(&mut self, input: f32, is_left: bool) -> f32 {
        let zm1 = if is_left {
            &mut self.zm1_left
        } else {
            &mut self.zm1_right
        };

        // All-pass filter: y[n] = a1*x[n] + x[n-1] - a1*y[n-1]
        let output = self.a1 * input + *zm1;
        *zm1 = input - self.a1 * output;
        output
    }

    fn reset(&mut self) {
        self.zm1_left = 0.0;
        self.zm1_right = 0.0;
    }
}

/// Phaser effect using cascaded all-pass filters
pub struct PhaserNode {
    name: String,
    rate: f32,           // LFO rate in Hz (0.1 to 10 Hz)
    depth: f32,          // Modulation depth 0.0 to 1.0
    stages: usize,       // Number of all-pass stages (2, 4, 6, or 8)
    feedback: f32,       // Feedback amount -0.95 to 0.95
    wet_dry: f32,        // 0.0 = dry only, 1.0 = wet only

    // All-pass filters
    filters: Vec<AllPassFilter>,

    // Feedback buffers
    feedback_left: f32,
    feedback_right: f32,

    // LFO state
    lfo_phase: f32,

    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl PhaserNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_RATE, "Rate", 0.1, 10.0, 0.5, ParameterUnit::Frequency),
            Parameter::new(PARAM_DEPTH, "Depth", 0.0, 1.0, 0.7, ParameterUnit::Generic),
            Parameter::new(PARAM_STAGES, "Stages", 2.0, 8.0, 6.0, ParameterUnit::Generic),
            Parameter::new(PARAM_FEEDBACK, "Feedback", -0.95, 0.95, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_WET_DRY, "Wet/Dry", 0.0, 1.0, 0.5, ParameterUnit::Generic),
        ];

        let mut filters = Vec::with_capacity(MAX_STAGES);
        for _ in 0..MAX_STAGES {
            filters.push(AllPassFilter::new());
        }

        Self {
            name,
            rate: 0.5,
            depth: 0.7,
            stages: 6,
            feedback: 0.5,
            wet_dry: 0.5,
            filters,
            feedback_left: 0.0,
            feedback_right: 0.0,
            lfo_phase: 0.0,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for PhaserNode {
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
            PARAM_RATE => {
                self.rate = value.clamp(0.1, 10.0);
            }
            PARAM_DEPTH => {
                self.depth = value.clamp(0.0, 1.0);
            }
            PARAM_STAGES => {
                // Round to even numbers: 2, 4, 6, 8
                let stages = (value.round() as usize).clamp(2, 8);
                self.stages = if stages % 2 == 0 { stages } else { stages + 1 };
            }
            PARAM_FEEDBACK => {
                self.feedback = value.clamp(-0.95, 0.95);
            }
            PARAM_WET_DRY => {
                self.wet_dry = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_RATE => self.rate,
            PARAM_DEPTH => self.depth,
            PARAM_STAGES => self.stages as f32,
            PARAM_FEEDBACK => self.feedback,
            PARAM_WET_DRY => self.wet_dry,
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
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        let dry_gain = 1.0 - self.wet_dry;
        let wet_gain = self.wet_dry;

        // Frequency range for all-pass filters (200 Hz to 2000 Hz)
        let min_freq = 200.0;
        let max_freq = 2000.0;

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Generate LFO value (sine wave, 0 to 1)
            let lfo_value = (self.lfo_phase * 2.0 * PI).sin() * 0.5 + 0.5;

            // Calculate modulated frequency
            let frequency = min_freq + (max_freq - min_freq) * lfo_value * self.depth;

            // Update all filter coefficients
            for filter in self.filters.iter_mut().take(self.stages) {
                filter.set_coefficient(frequency, self.sample_rate as f32);
            }

            // Add feedback
            let mut left_sig = left_in + self.feedback_left * self.feedback;
            let mut right_sig = right_in + self.feedback_right * self.feedback;

            // Process through all-pass filter chain
            for i in 0..self.stages {
                left_sig = self.filters[i].process(left_sig, true);
                right_sig = self.filters[i].process(right_sig, false);
            }

            // Store feedback
            self.feedback_left = left_sig;
            self.feedback_right = right_sig;

            // Mix dry and wet signals
            output[frame * 2] = left_in * dry_gain + left_sig * wet_gain;
            output[frame * 2 + 1] = right_in * dry_gain + right_sig * wet_gain;

            // Advance LFO phase
            self.lfo_phase += self.rate / self.sample_rate as f32;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        for filter in &mut self.filters {
            filter.reset();
        }
        self.feedback_left = 0.0;
        self.feedback_right = 0.0;
        self.lfo_phase = 0.0;
    }

    fn node_type(&self) -> &str {
        "Phaser"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        let mut filters = Vec::with_capacity(MAX_STAGES);
        for _ in 0..MAX_STAGES {
            filters.push(AllPassFilter::new());
        }

        Box::new(Self {
            name: self.name.clone(),
            rate: self.rate,
            depth: self.depth,
            stages: self.stages,
            feedback: self.feedback,
            wet_dry: self.wet_dry,
            filters,
            feedback_left: 0.0,
            feedback_right: 0.0,
            lfo_phase: 0.0,
            sample_rate: self.sample_rate,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
