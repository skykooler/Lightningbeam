use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use rand::Rng;

const PARAM_AMPLITUDE: u32 = 0;
const PARAM_COLOR: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoiseColor {
    White = 0,
    Pink = 1,
}

impl NoiseColor {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => NoiseColor::Pink,
            _ => NoiseColor::White,
        }
    }
}

/// Noise generator node with white and pink noise
pub struct NoiseGeneratorNode {
    name: String,
    amplitude: f32,
    color: NoiseColor,
    // Pink noise state (Paul Kellet's pink noise algorithm)
    pink_b0: f32,
    pink_b1: f32,
    pink_b2: f32,
    pink_b3: f32,
    pink_b4: f32,
    pink_b5: f32,
    pink_b6: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl NoiseGeneratorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_AMPLITUDE, "Amplitude", 0.0, 1.0, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_COLOR, "Color", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            amplitude: 0.5,
            color: NoiseColor::White,
            pink_b0: 0.0,
            pink_b1: 0.0,
            pink_b2: 0.0,
            pink_b3: 0.0,
            pink_b4: 0.0,
            pink_b5: 0.0,
            pink_b6: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Generate white noise sample
    fn generate_white(&self) -> f32 {
        let mut rng = rand::thread_rng();
        rng.gen_range(-1.0..1.0)
    }

    /// Generate pink noise sample using Paul Kellet's algorithm
    fn generate_pink(&mut self) -> f32 {
        let mut rng = rand::thread_rng();
        let white: f32 = rng.gen_range(-1.0..1.0);

        self.pink_b0 = 0.99886 * self.pink_b0 + white * 0.0555179;
        self.pink_b1 = 0.99332 * self.pink_b1 + white * 0.0750759;
        self.pink_b2 = 0.96900 * self.pink_b2 + white * 0.1538520;
        self.pink_b3 = 0.86650 * self.pink_b3 + white * 0.3104856;
        self.pink_b4 = 0.55000 * self.pink_b4 + white * 0.5329522;
        self.pink_b5 = -0.7616 * self.pink_b5 - white * 0.0168980;

        let pink = self.pink_b0 + self.pink_b1 + self.pink_b2 + self.pink_b3 + self.pink_b4 + self.pink_b5 + self.pink_b6 + white * 0.5362;
        self.pink_b6 = white * 0.115926;

        // Scale to approximately -1 to 1
        pink * 0.11
    }
}

impl AudioNode for NoiseGeneratorNode {
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
            PARAM_AMPLITUDE => self.amplitude = value.clamp(0.0, 1.0),
            PARAM_COLOR => self.color = NoiseColor::from_f32(value),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_AMPLITUDE => self.amplitude,
            PARAM_COLOR => self.color as i32 as f32,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        // Process by frames, not samples
        let frames = output.len() / 2;

        for frame in 0..frames {
            let sample = match self.color {
                NoiseColor::White => self.generate_white(),
                NoiseColor::Pink => self.generate_pink(),
            } * self.amplitude;

            // Write to both channels (mono source duplicated to stereo)
            output[frame * 2] = sample;       // Left
            output[frame * 2 + 1] = sample;   // Right
        }
    }

    fn reset(&mut self) {
        self.pink_b0 = 0.0;
        self.pink_b1 = 0.0;
        self.pink_b2 = 0.0;
        self.pink_b3 = 0.0;
        self.pink_b4 = 0.0;
        self.pink_b5 = 0.0;
        self.pink_b6 = 0.0;
    }

    fn node_type(&self) -> &str {
        "NoiseGenerator"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            amplitude: self.amplitude,
            color: self.color,
            pink_b0: 0.0,
            pink_b1: 0.0,
            pink_b2: 0.0,
            pink_b3: 0.0,
            pink_b4: 0.0,
            pink_b5: 0.0,
            pink_b6: 0.0,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
