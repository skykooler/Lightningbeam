use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_FREQUENCY: u32 = 0;
const PARAM_AMPLITUDE: u32 = 1;
const PARAM_WAVEFORM: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Waveform {
    Sine = 0,
    Saw = 1,
    Square = 2,
    Triangle = 3,
}

impl Waveform {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => Waveform::Saw,
            2 => Waveform::Square,
            3 => Waveform::Triangle,
            _ => Waveform::Sine,
        }
    }
}

/// Oscillator node with multiple waveforms
pub struct OscillatorNode {
    name: String,
    frequency: f32,
    amplitude: f32,
    waveform: Waveform,
    phase: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl OscillatorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),
            NodePort::new("FM", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_FREQUENCY, "Frequency", 20.0, 20000.0, 440.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_AMPLITUDE, "Amplitude", 0.0, 1.0, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_WAVEFORM, "Waveform", 0.0, 3.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            frequency: 440.0,
            amplitude: 0.5,
            waveform: Waveform::Sine,
            phase: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for OscillatorNode {
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
            PARAM_FREQUENCY => self.frequency = value.clamp(20.0, 20000.0),
            PARAM_AMPLITUDE => self.amplitude = value.clamp(0.0, 1.0),
            PARAM_WAVEFORM => self.waveform = Waveform::from_f32(value),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_FREQUENCY => self.frequency,
            PARAM_AMPLITUDE => self.amplitude,
            PARAM_WAVEFORM => self.waveform as i32 as f32,
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
        let sample_rate_f32 = sample_rate as f32;

        // Audio signals are stereo (interleaved L/R)
        // Process by frames, not samples
        let frames = output.len() / 2;

        for frame in 0..frames {
            // Start with base frequency
            let mut frequency = self.frequency;

            // V/Oct input: 0.0-1.0 maps to MIDI notes 0-127
            if !inputs.is_empty() && frame < inputs[0].len() {
                let voct = inputs[0][frame]; // Read V/Oct CV (mono)
                if voct > 0.001 {
                    // Convert CV to MIDI note number (0-1 -> 0-127)
                    let midi_note = voct * 127.0;
                    // Convert MIDI note to frequency: f = 440 * 2^((n-69)/12)
                    frequency = 440.0 * 2.0_f32.powf((midi_note - 69.0) / 12.0);
                }
            }

            // FM input: modulates the frequency
            if inputs.len() > 1 && frame < inputs[1].len() {
                let fm = inputs[1][frame]; // Read FM CV (mono)
                frequency *= 1.0 + fm;
            }

            let freq_mod = frequency;

            // Generate waveform sample based on waveform type
            let sample = match self.waveform {
                Waveform::Sine => (self.phase * 2.0 * PI).sin(),
                Waveform::Saw => 2.0 * self.phase - 1.0, // Ramp from -1 to 1
                Waveform::Square => {
                    if self.phase < 0.5 { 1.0 } else { -1.0 }
                }
                Waveform::Triangle => {
                    // Triangle: rises from -1 to 1, falls back to -1
                    4.0 * (self.phase - 0.5).abs() - 1.0
                }
            } * self.amplitude;

            // Write to both channels (mono source duplicated to stereo)
            output[frame * 2] = sample;       // Left
            output[frame * 2 + 1] = sample;   // Right

            // Update phase once per frame
            self.phase += freq_mod / sample_rate_f32;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn node_type(&self) -> &str {
        "Oscillator"
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
            phase: 0.0, // Reset phase for new instance
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
