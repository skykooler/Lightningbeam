use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const WAVETABLE_SIZE: usize = 256;

// Parameters
const PARAM_WAVETABLE: u32 = 0;
const PARAM_FINE_TUNE: u32 = 1;
const PARAM_POSITION: u32 = 2;

/// Types of preset wavetables
#[derive(Debug, Clone, Copy, PartialEq)]
enum WavetableType {
    Sine = 0,
    Saw = 1,
    Square = 2,
    Triangle = 3,
    PWM = 4,           // Pulse Width Modulated
    Harmonic = 5,      // Rich harmonics
    Inharmonic = 6,    // Metallic/bell-like
    Digital = 7,       // Stepped/digital artifacts
}

impl WavetableType {
    fn from_u32(value: u32) -> Self {
        match value {
            0 => WavetableType::Sine,
            1 => WavetableType::Saw,
            2 => WavetableType::Square,
            3 => WavetableType::Triangle,
            4 => WavetableType::PWM,
            5 => WavetableType::Harmonic,
            6 => WavetableType::Inharmonic,
            7 => WavetableType::Digital,
            _ => WavetableType::Sine,
        }
    }
}

/// Generate a wavetable of the specified type
fn generate_wavetable(wave_type: WavetableType) -> Vec<f32> {
    let mut table = vec![0.0; WAVETABLE_SIZE];

    match wave_type {
        WavetableType::Sine => {
            for i in 0..WAVETABLE_SIZE {
                let phase = (i as f32 / WAVETABLE_SIZE as f32) * 2.0 * PI;
                table[i] = phase.sin();
            }
        }
        WavetableType::Saw => {
            for i in 0..WAVETABLE_SIZE {
                let t = i as f32 / WAVETABLE_SIZE as f32;
                table[i] = 2.0 * t - 1.0;
            }
        }
        WavetableType::Square => {
            for i in 0..WAVETABLE_SIZE {
                table[i] = if i < WAVETABLE_SIZE / 2 { 1.0 } else { -1.0 };
            }
        }
        WavetableType::Triangle => {
            for i in 0..WAVETABLE_SIZE {
                let t = i as f32 / WAVETABLE_SIZE as f32;
                table[i] = if t < 0.5 {
                    4.0 * t - 1.0
                } else {
                    -4.0 * t + 3.0
                };
            }
        }
        WavetableType::PWM => {
            // Variable pulse width
            for i in 0..WAVETABLE_SIZE {
                let duty = 0.25; // 25% duty cycle
                table[i] = if (i as f32 / WAVETABLE_SIZE as f32) < duty { 1.0 } else { -1.0 };
            }
        }
        WavetableType::Harmonic => {
            // Multiple harmonics for rich sound
            for i in 0..WAVETABLE_SIZE {
                let phase = (i as f32 / WAVETABLE_SIZE as f32) * 2.0 * PI;
                table[i] = phase.sin() * 0.5
                    + (phase * 2.0).sin() * 0.25
                    + (phase * 3.0).sin() * 0.125
                    + (phase * 4.0).sin() * 0.0625;
            }
        }
        WavetableType::Inharmonic => {
            // Non-integer harmonics for metallic/bell-like sounds
            for i in 0..WAVETABLE_SIZE {
                let phase = (i as f32 / WAVETABLE_SIZE as f32) * 2.0 * PI;
                table[i] = phase.sin() * 0.4
                    + (phase * 2.13).sin() * 0.3
                    + (phase * 3.76).sin() * 0.2
                    + (phase * 5.41).sin() * 0.1;
            }
        }
        WavetableType::Digital => {
            // Stepped waveform with digital artifacts
            for i in 0..WAVETABLE_SIZE {
                let steps = 8;
                let step = (i * steps / WAVETABLE_SIZE) as f32 / steps as f32;
                table[i] = step * 2.0 - 1.0;
            }
        }
    }

    table
}

/// Wavetable oscillator node
pub struct WavetableOscillatorNode {
    name: String,

    // Current wavetable
    wavetable_type: WavetableType,
    wavetable: Vec<f32>,

    // Oscillator state
    phase: f32,
    fine_tune: f32,     // -1.0 to 1.0 semitones
    position: f32,      // 0.0 to 1.0 (for future multi-cycle wavetables)

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl WavetableOscillatorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_WAVETABLE, "Wavetable", 0.0, 7.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_FINE_TUNE, "Fine Tune", -1.0, 1.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_POSITION, "Position", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        let wavetable_type = WavetableType::Sine;
        let wavetable = generate_wavetable(wavetable_type);

        Self {
            name,
            wavetable_type,
            wavetable,
            phase: 0.0,
            fine_tune: 0.0,
            position: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Convert V/oct CV to frequency with fine tune
    fn voct_to_freq(&self, voct: f32) -> f32 {
        let semitones = voct * 12.0 + self.fine_tune;
        440.0 * 2.0_f32.powf(semitones / 12.0)
    }

    /// Read from wavetable with linear interpolation
    fn read_wavetable(&self, phase: f32) -> f32 {
        let index = phase * WAVETABLE_SIZE as f32;
        let index_floor = index.floor() as usize % WAVETABLE_SIZE;
        let index_ceil = (index_floor + 1) % WAVETABLE_SIZE;
        let frac = index - index.floor();

        // Linear interpolation
        let sample1 = self.wavetable[index_floor];
        let sample2 = self.wavetable[index_ceil];
        sample1 + (sample2 - sample1) * frac
    }
}

impl AudioNode for WavetableOscillatorNode {
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
            PARAM_WAVETABLE => {
                let new_type = WavetableType::from_u32(value as u32);
                if new_type != self.wavetable_type {
                    self.wavetable_type = new_type;
                    self.wavetable = generate_wavetable(new_type);
                }
            }
            PARAM_FINE_TUNE => {
                self.fine_tune = value.clamp(-1.0, 1.0);
            }
            PARAM_POSITION => {
                self.position = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_WAVETABLE => self.wavetable_type as u32 as f32,
            PARAM_FINE_TUNE => self.fine_tune,
            PARAM_POSITION => self.position,
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
        let frames = output.len() / 2;

        for frame in 0..frames {
            // Read V/Oct input
            let voct = if !inputs.is_empty() && !inputs[0].is_empty() {
                inputs[0][frame.min(inputs[0].len() / 2 - 1) * 2]
            } else {
                0.0 // Default to A4 (440 Hz)
            };

            // Calculate frequency
            let freq = self.voct_to_freq(voct);

            // Read from wavetable
            let sample = self.read_wavetable(self.phase);

            // Advance phase
            self.phase += freq / sample_rate as f32;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }

            // Output stereo (same signal to both channels)
            output[frame * 2] = sample * 0.5; // Scale down to prevent clipping
            output[frame * 2 + 1] = sample * 0.5;
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn node_type(&self) -> &str {
        "WavetableOscillator"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self::new(self.name.clone()))
    }
}
