use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_BIT_DEPTH: u32 = 0;
const PARAM_SAMPLE_RATE_REDUCTION: u32 = 1;
const PARAM_MIX: u32 = 2;

/// Bit Crusher effect - reduces bit depth and sample rate for lo-fi sound
pub struct BitCrusherNode {
    name: String,
    bit_depth: f32,              // 1 to 16 bits
    sample_rate_reduction: f32,  // 1 to 48000 Hz (crushed sample rate)
    mix: f32,                    // 0.0 to 1.0 (dry/wet)

    // State for sample rate reduction
    hold_left: f32,
    hold_right: f32,
    hold_counter: f32,

    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl BitCrusherNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_BIT_DEPTH, "Bit Depth", 1.0, 16.0, 8.0, ParameterUnit::Generic),
            Parameter::new(PARAM_SAMPLE_RATE_REDUCTION, "Sample Rate", 100.0, 48000.0, 8000.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_MIX, "Mix", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            bit_depth: 8.0,
            sample_rate_reduction: 8000.0,
            mix: 1.0,
            hold_left: 0.0,
            hold_right: 0.0,
            hold_counter: 0.0,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Quantize sample to specified bit depth
    fn quantize(&self, sample: f32) -> f32 {
        // Calculate number of quantization levels
        let levels = 2.0_f32.powf(self.bit_depth);

        // Quantize: scale up, round, scale back down
        let scaled = sample * levels / 2.0;
        let quantized = scaled.round();
        quantized * 2.0 / levels
    }
}

impl AudioNode for BitCrusherNode {
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
            PARAM_BIT_DEPTH => self.bit_depth = value.clamp(1.0, 16.0),
            PARAM_SAMPLE_RATE_REDUCTION => self.sample_rate_reduction = value.clamp(100.0, 48000.0),
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_BIT_DEPTH => self.bit_depth,
            PARAM_SAMPLE_RATE_REDUCTION => self.sample_rate_reduction,
            PARAM_MIX => self.mix,
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

        // Calculate sample hold period
        let hold_period = self.sample_rate as f32 / self.sample_rate_reduction;

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Sample rate reduction: hold samples
            if self.hold_counter <= 0.0 {
                // Time to sample a new value
                self.hold_left = self.quantize(left_in);
                self.hold_right = self.quantize(right_in);
                self.hold_counter = hold_period;
            }

            self.hold_counter -= 1.0;

            // Mix dry and wet
            let wet_left = self.hold_left;
            let wet_right = self.hold_right;

            output[frame * 2] = left_in * (1.0 - self.mix) + wet_left * self.mix;
            output[frame * 2 + 1] = right_in * (1.0 - self.mix) + wet_right * self.mix;
        }
    }

    fn reset(&mut self) {
        self.hold_left = 0.0;
        self.hold_right = 0.0;
        self.hold_counter = 0.0;
    }

    fn node_type(&self) -> &str {
        "BitCrusher"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            bit_depth: self.bit_depth,
            sample_rate_reduction: self.sample_rate_reduction,
            mix: self.mix,
            hold_left: 0.0,
            hold_right: 0.0,
            hold_counter: 0.0,
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
