use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;
use rand::Rng;

const PARAM_FREQUENCY: u32 = 0;
const PARAM_AMPLITUDE: u32 = 1;
const PARAM_WAVEFORM: u32 = 2;
const PARAM_PHASE_OFFSET: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LFOWaveform {
    Sine = 0,
    Triangle = 1,
    Saw = 2,
    Square = 3,
    Random = 4,
}

impl LFOWaveform {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => LFOWaveform::Triangle,
            2 => LFOWaveform::Saw,
            3 => LFOWaveform::Square,
            4 => LFOWaveform::Random,
            _ => LFOWaveform::Sine,
        }
    }
}

/// Low Frequency Oscillator node for modulation
pub struct LFONode {
    name: String,
    frequency: f32,
    amplitude: f32,
    waveform: LFOWaveform,
    phase_offset: f32,
    phase: f32,
    last_random_value: f32,
    next_random_value: f32,
    random_phase: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl LFONode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_FREQUENCY, "Frequency", 0.01, 20.0, 1.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_AMPLITUDE, "Amplitude", 0.0, 1.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_WAVEFORM, "Waveform", 0.0, 4.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_PHASE_OFFSET, "Phase", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        let mut rng = rand::thread_rng();

        Self {
            name,
            frequency: 1.0,
            amplitude: 1.0,
            waveform: LFOWaveform::Sine,
            phase_offset: 0.0,
            phase: 0.0,
            last_random_value: rng.gen_range(-1.0..1.0),
            next_random_value: rng.gen_range(-1.0..1.0),
            random_phase: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for LFONode {
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
            PARAM_FREQUENCY => self.frequency = value.clamp(0.01, 20.0),
            PARAM_AMPLITUDE => self.amplitude = value.clamp(0.0, 1.0),
            PARAM_WAVEFORM => self.waveform = LFOWaveform::from_f32(value),
            PARAM_PHASE_OFFSET => self.phase_offset = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_FREQUENCY => self.frequency,
            PARAM_AMPLITUDE => self.amplitude,
            PARAM_WAVEFORM => self.waveform as i32 as f32,
            PARAM_PHASE_OFFSET => self.phase_offset,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
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
        for sample_idx in 0..output.len() {
            let current_phase = (self.phase + self.phase_offset) % 1.0;

            // Generate waveform sample based on waveform type
            let raw_sample = match self.waveform {
                LFOWaveform::Sine => (current_phase * 2.0 * PI).sin(),
                LFOWaveform::Triangle => {
                    // Triangle: rises from -1 to 1, falls back to -1
                    4.0 * (current_phase - 0.5).abs() - 1.0
                }
                LFOWaveform::Saw => {
                    // Sawtooth: ramp from -1 to 1
                    2.0 * current_phase - 1.0
                }
                LFOWaveform::Square => {
                    if current_phase < 0.5 { 1.0 } else { -1.0 }
                }
                LFOWaveform::Random => {
                    // Sample & hold random values with smooth interpolation
                    // Interpolate between last and next random value
                    let t = self.random_phase;
                    self.last_random_value * (1.0 - t) + self.next_random_value * t
                }
            };

            // Scale to 0-1 range and apply amplitude
            let sample = (raw_sample * 0.5 + 0.5) * self.amplitude;
            output[sample_idx] = sample;

            // Update phase
            self.phase += self.frequency / sample_rate_f32;
            if self.phase >= 1.0 {
                self.phase -= 1.0;

                // For random waveform, generate new random value at each cycle
                if self.waveform == LFOWaveform::Random {
                    self.last_random_value = self.next_random_value;
                    let mut rng = rand::thread_rng();
                    self.next_random_value = rng.gen_range(-1.0..1.0);
                    self.random_phase = 0.0;
                }
            }

            // Update random interpolation phase
            if self.waveform == LFOWaveform::Random {
                self.random_phase += self.frequency / sample_rate_f32;
                if self.random_phase >= 1.0 {
                    self.random_phase -= 1.0;
                }
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.random_phase = 0.0;
        let mut rng = rand::thread_rng();
        self.last_random_value = rng.gen_range(-1.0..1.0);
        self.next_random_value = rng.gen_range(-1.0..1.0);
    }

    fn node_type(&self) -> &str {
        "LFO"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            frequency: self.frequency,
            amplitude: self.amplitude,
            waveform: self.waveform,
            phase_offset: self.phase_offset,
            phase: 0.0, // Reset phase for new instance
            last_random_value: self.last_random_value,
            next_random_value: self.next_random_value,
            random_phase: 0.0,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
