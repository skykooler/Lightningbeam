use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;
const PARAM_FEEDBACK: u32 = 2;
const PARAM_WET_DRY: u32 = 3;

const MAX_DELAY_MS: f32 = 10.0;
const BASE_DELAY_MS: f32 = 1.0;

/// Flanger effect using modulated delay lines with feedback
pub struct FlangerNode {
    name: String,
    rate: f32,           // LFO rate in Hz (0.1 to 10 Hz)
    depth: f32,          // Modulation depth 0.0 to 1.0
    feedback: f32,       // Feedback amount -0.95 to 0.95
    wet_dry: f32,        // 0.0 = dry only, 1.0 = wet only

    // Delay buffers for left and right channels
    delay_buffer_left: Vec<f32>,
    delay_buffer_right: Vec<f32>,
    write_position: usize,
    max_delay_samples: usize,
    sample_rate: u32,

    // LFO state
    lfo_phase: f32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl FlangerNode {
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
            Parameter::new(PARAM_FEEDBACK, "Feedback", -0.95, 0.95, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_WET_DRY, "Wet/Dry", 0.0, 1.0, 0.5, ParameterUnit::Generic),
        ];

        // Allocate max delay buffer size
        let max_delay_samples = ((MAX_DELAY_MS / 1000.0) * 48000.0) as usize;

        Self {
            name,
            rate: 0.5,
            depth: 0.7,
            feedback: 0.5,
            wet_dry: 0.5,
            delay_buffer_left: vec![0.0; max_delay_samples],
            delay_buffer_right: vec![0.0; max_delay_samples],
            write_position: 0,
            max_delay_samples,
            sample_rate: 48000,
            lfo_phase: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    fn read_interpolated_sample(&self, buffer: &[f32], delay_samples: f32) -> f32 {
        // Linear interpolation for smooth delay modulation
        let delay_samples = delay_samples.clamp(0.0, (self.max_delay_samples - 1) as f32);

        let read_pos_float = self.write_position as f32 - delay_samples;
        let read_pos_float = if read_pos_float < 0.0 {
            read_pos_float + self.max_delay_samples as f32
        } else {
            read_pos_float
        };

        let read_pos_int = read_pos_float.floor() as usize;
        let frac = read_pos_float - read_pos_int as f32;

        let sample1 = buffer[read_pos_int % self.max_delay_samples];
        let sample2 = buffer[(read_pos_int + 1) % self.max_delay_samples];

        sample1 * (1.0 - frac) + sample2 * frac
    }
}

impl AudioNode for FlangerNode {
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
            self.max_delay_samples = ((MAX_DELAY_MS / 1000.0) * sample_rate as f32) as usize;
            self.delay_buffer_left.resize(self.max_delay_samples, 0.0);
            self.delay_buffer_right.resize(self.max_delay_samples, 0.0);
            self.write_position = 0;
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        let dry_gain = 1.0 - self.wet_dry;
        let wet_gain = self.wet_dry;

        let base_delay_samples = (BASE_DELAY_MS / 1000.0) * self.sample_rate as f32;
        let max_modulation_samples = (MAX_DELAY_MS - BASE_DELAY_MS) / 1000.0 * self.sample_rate as f32;

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Generate LFO value (sine wave, 0 to 1)
            let lfo_value = ((self.lfo_phase * 2.0 * PI).sin() * 0.5 + 0.5) * self.depth;

            // Calculate modulated delay time
            let delay_samples = base_delay_samples + lfo_value * max_modulation_samples;

            // Read delayed samples with interpolation
            let left_delayed = self.read_interpolated_sample(&self.delay_buffer_left, delay_samples);
            let right_delayed = self.read_interpolated_sample(&self.delay_buffer_right, delay_samples);

            // Mix dry and wet signals
            output[frame * 2] = left_in * dry_gain + left_delayed * wet_gain;
            output[frame * 2 + 1] = right_in * dry_gain + right_delayed * wet_gain;

            // Write to delay buffer with feedback
            self.delay_buffer_left[self.write_position] = left_in + left_delayed * self.feedback;
            self.delay_buffer_right[self.write_position] = right_in + right_delayed * self.feedback;

            // Advance write position
            self.write_position = (self.write_position + 1) % self.max_delay_samples;

            // Advance LFO phase
            self.lfo_phase += self.rate / self.sample_rate as f32;
            if self.lfo_phase >= 1.0 {
                self.lfo_phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.delay_buffer_left.fill(0.0);
        self.delay_buffer_right.fill(0.0);
        self.write_position = 0;
        self.lfo_phase = 0.0;
    }

    fn node_type(&self) -> &str {
        "Flanger"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            rate: self.rate,
            depth: self.depth,
            feedback: self.feedback,
            wet_dry: self.wet_dry,
            delay_buffer_left: vec![0.0; self.max_delay_samples],
            delay_buffer_right: vec![0.0; self.max_delay_samples],
            write_position: 0,
            max_delay_samples: self.max_delay_samples,
            sample_rate: self.sample_rate,
            lfo_phase: 0.0,
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
