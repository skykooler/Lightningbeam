use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_PAN: u32 = 0;

/// Stereo panning node using constant-power panning law
/// Converts mono audio to stereo with controllable pan position
pub struct PanNode {
    name: String,
    pan: f32,
    left_gain: f32,
    right_gain: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl PanNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("Pan CV", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_PAN, "Pan", -1.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        let mut node = Self {
            name,
            pan: 0.0,
            left_gain: 1.0,
            right_gain: 1.0,
            inputs,
            outputs,
            parameters,
        };

        node.update_gains();
        node
    }

    /// Update left/right gains using constant-power panning law
    fn update_gains(&mut self) {
        // Constant-power panning: pan from -1 to +1 maps to angle 0 to PI/2
        let angle = (self.pan + 1.0) * 0.5 * PI / 2.0;

        self.left_gain = angle.cos();
        self.right_gain = angle.sin();
    }
}

impl AudioNode for PanNode {
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
            PARAM_PAN => {
                self.pan = value.clamp(-1.0, 1.0);
                self.update_gains();
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_PAN => self.pan,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if inputs.is_empty() || outputs.is_empty() {
            return;
        }

        let audio_input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        // Process by frames, not samples
        let frames = audio_input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        for frame in 0..frames_to_process {
            // Get base pan position
            let mut pan = self.pan;

            // Add CV modulation if connected
            if inputs.len() > 1 && frame < inputs[1].len() {
                let cv = inputs[1][frame]; // CV is mono
                // CV is 0-1, map to -1 to +1 range
                pan += cv * 2.0 - 1.0;
                pan = pan.clamp(-1.0, 1.0);
            }

            // Update gains if pan changed from CV
            let angle = (pan + 1.0) * 0.5 * PI / 2.0;
            let left_gain = angle.cos();
            let right_gain = angle.sin();

            // Read stereo input
            let left_in = audio_input[frame * 2];
            let right_in = audio_input[frame * 2 + 1];

            // Mix both input channels with panning
            // When pan is -1 (full left), left gets full signal, right gets nothing
            // When pan is 0 (center), both get equal signal
            // When pan is +1 (full right), right gets full signal, left gets nothing
            output[frame * 2] = (left_in + right_in) * left_gain;        // Left
            output[frame * 2 + 1] = (left_in + right_in) * right_gain;   // Right
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Pan"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            pan: self.pan,
            left_gain: self.left_gain,
            right_gain: self.right_gain,
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
