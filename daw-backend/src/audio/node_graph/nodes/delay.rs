use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_DELAY_TIME: u32 = 0;
const PARAM_FEEDBACK: u32 = 1;
const PARAM_WET_DRY: u32 = 2;

const MAX_DELAY_SECONDS: f32 = 2.0;

/// Stereo delay node with feedback
pub struct DelayNode {
    name: String,
    delay_time: f32,     // seconds
    feedback: f32,       // 0.0 to 0.95
    wet_dry: f32,        // 0.0 = dry only, 1.0 = wet only

    // Delay buffers for left and right channels
    delay_buffer_left: Vec<f32>,
    delay_buffer_right: Vec<f32>,
    write_position: usize,
    max_delay_samples: usize,
    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl DelayNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_DELAY_TIME, "Delay Time", 0.001, MAX_DELAY_SECONDS, 0.5, ParameterUnit::Time),
            Parameter::new(PARAM_FEEDBACK, "Feedback", 0.0, 0.95, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_WET_DRY, "Wet/Dry", 0.0, 1.0, 0.5, ParameterUnit::Generic),
        ];

        // Allocate max delay buffer size (will be initialized properly when we get sample rate)
        let max_delay_samples = (MAX_DELAY_SECONDS * 48000.0) as usize; // Assume max 48kHz

        Self {
            name,
            delay_time: 0.5,
            feedback: 0.5,
            wet_dry: 0.5,
            delay_buffer_left: vec![0.0; max_delay_samples],
            delay_buffer_right: vec![0.0; max_delay_samples],
            write_position: 0,
            max_delay_samples,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }

    fn get_delay_samples(&self) -> usize {
        (self.delay_time * self.sample_rate as f32) as usize
    }

    fn read_delayed_sample(&self, buffer: &[f32], delay_samples: usize) -> f32 {
        // Calculate read position (wrap around)
        let read_pos = if self.write_position >= delay_samples {
            self.write_position - delay_samples
        } else {
            self.max_delay_samples + self.write_position - delay_samples
        };

        buffer[read_pos % self.max_delay_samples]
    }
}

impl AudioNode for DelayNode {
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
            PARAM_DELAY_TIME => {
                self.delay_time = value.clamp(0.001, MAX_DELAY_SECONDS);
            }
            PARAM_FEEDBACK => {
                self.feedback = value.clamp(0.0, 0.95);
            }
            PARAM_WET_DRY => {
                self.wet_dry = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_DELAY_TIME => self.delay_time,
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
            self.max_delay_samples = (MAX_DELAY_SECONDS * sample_rate as f32) as usize;
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

        let delay_samples = self.get_delay_samples().max(1).min(self.max_delay_samples - 1);

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Read delayed samples
            let left_delayed = self.read_delayed_sample(&self.delay_buffer_left, delay_samples);
            let right_delayed = self.read_delayed_sample(&self.delay_buffer_right, delay_samples);

            // Mix dry and wet signals
            let dry_gain = 1.0 - self.wet_dry;
            let wet_gain = self.wet_dry;

            let left_out = left_in * dry_gain + left_delayed * wet_gain;
            let right_out = right_in * dry_gain + right_delayed * wet_gain;

            output[frame * 2] = left_out;
            output[frame * 2 + 1] = right_out;

            // Write to delay buffer with feedback
            self.delay_buffer_left[self.write_position] = left_in + left_delayed * self.feedback;
            self.delay_buffer_right[self.write_position] = right_in + right_delayed * self.feedback;

            // Advance write position
            self.write_position = (self.write_position + 1) % self.max_delay_samples;
        }
    }

    fn reset(&mut self) {
        self.delay_buffer_left.fill(0.0);
        self.delay_buffer_right.fill(0.0);
        self.write_position = 0;
    }

    fn node_type(&self) -> &str {
        "Delay"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            delay_time: self.delay_time,
            feedback: self.feedback,
            wet_dry: self.wet_dry,
            delay_buffer_left: vec![0.0; self.max_delay_samples],
            delay_buffer_right: vec![0.0; self.max_delay_samples],
            write_position: 0,
            max_delay_samples: self.max_delay_samples,
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
