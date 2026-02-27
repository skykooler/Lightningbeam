use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_RATE: u32 = 0;
const PARAM_DEPTH: u32 = 1;

const MAX_DELAY_MS: f32 = 7.0;
const BASE_DELAY_MS: f32 = 0.5;

/// Vibrato effect — periodic pitch modulation via a short modulated delay line.
///
/// 100% wet signal (no dry mix). Supports an external Mod CV input that, when
/// connected, replaces the internal sine LFO with the incoming CV signal.
pub struct VibratoNode {
    name: String,
    rate: f32,
    depth: f32,

    delay_buffer_left: Vec<f32>,
    delay_buffer_right: Vec<f32>,
    write_position: usize,
    max_delay_samples: usize,
    sample_rate: u32,

    lfo_phase: f32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl VibratoNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("Mod CV In", SignalType::CV, 1),
            NodePort::new("Rate CV In", SignalType::CV, 2),
            NodePort::new("Depth CV In", SignalType::CV, 3),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_RATE, "Rate", 0.1, 14.0, 5.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_DEPTH, "Depth", 0.0, 1.0, 0.5, ParameterUnit::Generic),
        ];

        let max_delay_samples = ((MAX_DELAY_MS / 1000.0) * 48000.0) as usize;

        Self {
            name,
            rate: 5.0,
            depth: 0.5,
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

impl AudioNode for VibratoNode {
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
                self.rate = value.clamp(0.1, 14.0);
            }
            PARAM_DEPTH => {
                self.depth = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_RATE => self.rate,
            PARAM_DEPTH => self.depth,
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

        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.max_delay_samples = ((MAX_DELAY_MS / 1000.0) * sample_rate as f32) as usize;
            self.delay_buffer_left.resize(self.max_delay_samples, 0.0);
            self.delay_buffer_right.resize(self.max_delay_samples, 0.0);
            self.write_position = 0;
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // CV inputs — unconnected ports are filled with NaN
        let mod_cv = inputs.get(1);
        let rate_cv = inputs.get(2);
        let depth_cv = inputs.get(3);

        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        let base_delay_samples = (BASE_DELAY_MS / 1000.0) * self.sample_rate as f32;
        let max_modulation_samples = (MAX_DELAY_MS - BASE_DELAY_MS) / 1000.0 * self.sample_rate as f32;

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Resolve depth: CV overrides knob when connected
            let depth = if let Some(cv) = depth_cv {
                let cv_val = cv.get(frame).copied().unwrap_or(f32::NAN);
                if cv_val.is_nan() {
                    self.depth
                } else {
                    cv_val.clamp(0.0, 1.0)
                }
            } else {
                self.depth
            };

            // Determine modulation value (0..1 range, pre-depth)
            let mod_value = if let Some(cv) = mod_cv {
                let cv_val = cv.get(frame).copied().unwrap_or(f32::NAN);
                if cv_val.is_nan() {
                    // No external mod — use internal LFO
                    None
                } else {
                    Some(cv_val.clamp(0.0, 1.0))
                }
            } else {
                None
            };

            let modulation = if let Some(ext) = mod_value {
                // External modulation: CV value scaled by depth
                ext * depth
            } else {
                // Internal LFO: resolve rate with CV
                let rate = if let Some(cv) = rate_cv {
                    let cv_val = cv.get(frame).copied().unwrap_or(f32::NAN);
                    if cv_val.is_nan() {
                        self.rate
                    } else {
                        (self.rate + cv_val * 14.0).clamp(0.1, 14.0)
                    }
                } else {
                    self.rate
                };

                let lfo_value = (self.lfo_phase * 2.0 * PI).sin() * 0.5 + 0.5;

                self.lfo_phase += rate / self.sample_rate as f32;
                if self.lfo_phase >= 1.0 {
                    self.lfo_phase -= 1.0;
                }

                lfo_value * depth
            };

            let delay_samples = base_delay_samples + modulation * max_modulation_samples;

            // 100% wet — output is only the delayed signal
            output[frame * 2] = self.read_interpolated_sample(&self.delay_buffer_left, delay_samples);
            output[frame * 2 + 1] = self.read_interpolated_sample(&self.delay_buffer_right, delay_samples);

            self.delay_buffer_left[self.write_position] = left_in;
            self.delay_buffer_right[self.write_position] = right_in;

            self.write_position = (self.write_position + 1) % self.max_delay_samples;
        }
    }

    fn reset(&mut self) {
        self.delay_buffer_left.fill(0.0);
        self.delay_buffer_right.fill(0.0);
        self.write_position = 0;
        self.lfo_phase = 0.0;
    }

    fn node_type(&self) -> &str {
        "Vibrato"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            rate: self.rate,
            depth: self.depth,
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
