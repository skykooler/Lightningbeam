use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_ROOM_SIZE: u32 = 0;
const PARAM_DAMPING: u32 = 1;
const PARAM_WET_DRY: u32 = 2;

// Schroeder reverb uses a parallel bank of comb filters followed by series all-pass filters
// Comb filter delays (in samples at 48kHz)
const COMB_DELAYS: [usize; 8] = [1557, 1617, 1491, 1422, 1277, 1356, 1188, 1116];
// All-pass filter delays (in samples at 48kHz)
const ALLPASS_DELAYS: [usize; 4] = [225, 556, 441, 341];

/// Process a single channel through comb and all-pass filters
fn process_channel(
    input: f32,
    comb_filters: &mut [CombFilter],
    allpass_filters: &mut [AllPassFilter],
) -> f32 {
    // Sum parallel comb filters and scale down to prevent excessive gain
    // With 8 comb filters, we need to scale the output significantly
    let mut output = 0.0;
    for comb in comb_filters.iter_mut() {
        output += comb.process(input);
    }
    output *= 0.015; // Scale down the summed comb output

    // Series all-pass filters
    for allpass in allpass_filters.iter_mut() {
        output = allpass.process(output);
    }

    output
}

/// Single comb filter for reverb
struct CombFilter {
    buffer: Vec<f32>,
    buffer_size: usize,
    filter_store: f32,
    write_pos: usize,
    damp: f32,
    feedback: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            buffer_size: size,
            filter_store: 0.0,
            write_pos: 0,
            damp: 0.5,
            feedback: 0.5,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.write_pos];

        // One-pole lowpass filter
        self.filter_store = output * (1.0 - self.damp) + self.filter_store * self.damp;

        self.buffer[self.write_pos] = input + self.filter_store * self.feedback;

        self.write_pos = (self.write_pos + 1) % self.buffer_size;

        output
    }

    fn mute(&mut self) {
        self.buffer.fill(0.0);
        self.filter_store = 0.0;
    }

    fn set_damp(&mut self, val: f32) {
        self.damp = val;
    }

    fn set_feedback(&mut self, val: f32) {
        self.feedback = val;
    }
}

/// Single all-pass filter for reverb
struct AllPassFilter {
    buffer: Vec<f32>,
    buffer_size: usize,
    write_pos: usize,
}

impl AllPassFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            buffer_size: size,
            write_pos: 0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let delayed = self.buffer[self.write_pos];
        let output = -input + delayed;

        self.buffer[self.write_pos] = input + delayed * 0.5;

        self.write_pos = (self.write_pos + 1) % self.buffer_size;

        output
    }

    fn mute(&mut self) {
        self.buffer.fill(0.0);
    }
}

/// Schroeder reverb node with room size and damping controls
pub struct ReverbNode {
    name: String,
    room_size: f32,      // 0.0 to 1.0
    damping: f32,        // 0.0 to 1.0
    wet_dry: f32,        // 0.0 = dry only, 1.0 = wet only

    // Left channel filters
    comb_filters_left: Vec<CombFilter>,
    allpass_filters_left: Vec<AllPassFilter>,

    // Right channel filters
    comb_filters_right: Vec<CombFilter>,
    allpass_filters_right: Vec<AllPassFilter>,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl ReverbNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_ROOM_SIZE, "Room Size", 0.0, 1.0, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_DAMPING, "Damping", 0.0, 1.0, 0.5, ParameterUnit::Generic),
            Parameter::new(PARAM_WET_DRY, "Wet/Dry", 0.0, 1.0, 0.3, ParameterUnit::Generic),
        ];

        // Create comb filters for both channels
        // Right channel has slightly different delays to create stereo effect
        let comb_filters_left: Vec<CombFilter> = COMB_DELAYS.iter().map(|&d| CombFilter::new(d)).collect();
        let comb_filters_right: Vec<CombFilter> = COMB_DELAYS.iter().map(|&d| CombFilter::new(d + 23)).collect();

        // Create all-pass filters for both channels
        let allpass_filters_left: Vec<AllPassFilter> = ALLPASS_DELAYS.iter().map(|&d| AllPassFilter::new(d)).collect();
        let allpass_filters_right: Vec<AllPassFilter> = ALLPASS_DELAYS.iter().map(|&d| AllPassFilter::new(d + 23)).collect();

        let mut node = Self {
            name,
            room_size: 0.5,
            damping: 0.5,
            wet_dry: 0.3,
            comb_filters_left,
            allpass_filters_left,
            comb_filters_right,
            allpass_filters_right,
            inputs,
            outputs,
            parameters,
        };

        node.update_filters();
        node
    }

    fn update_filters(&mut self) {
        // Room size affects feedback (larger room = more feedback)
        let feedback = 0.28 + self.room_size * 0.7;

        // Update all comb filters
        for comb in &mut self.comb_filters_left {
            comb.set_feedback(feedback);
            comb.set_damp(self.damping);
        }
        for comb in &mut self.comb_filters_right {
            comb.set_feedback(feedback);
            comb.set_damp(self.damping);
        }
    }

}

impl AudioNode for ReverbNode {
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
            PARAM_ROOM_SIZE => {
                self.room_size = value.clamp(0.0, 1.0);
                self.update_filters();
            }
            PARAM_DAMPING => {
                self.damping = value.clamp(0.0, 1.0);
                self.update_filters();
            }
            PARAM_WET_DRY => {
                self.wet_dry = value.clamp(0.0, 1.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_ROOM_SIZE => self.room_size,
            PARAM_DAMPING => self.damping,
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
        _sample_rate: u32,
    ) {
        if inputs.is_empty() || outputs.is_empty() {
            return;
        }

        let input = inputs[0];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        let dry_gain = 1.0 - self.wet_dry;
        let wet_gain = self.wet_dry;

        for frame in 0..frames_to_process {
            let left_in = input[frame * 2];
            let right_in = input[frame * 2 + 1];

            // Process both channels
            let left_wet = process_channel(
                left_in,
                &mut self.comb_filters_left,
                &mut self.allpass_filters_left,
            );
            let right_wet = process_channel(
                right_in,
                &mut self.comb_filters_right,
                &mut self.allpass_filters_right,
            );

            // Mix dry and wet signals
            output[frame * 2] = left_in * dry_gain + left_wet * wet_gain;
            output[frame * 2 + 1] = right_in * dry_gain + right_wet * wet_gain;
        }
    }

    fn reset(&mut self) {
        for comb in &mut self.comb_filters_left {
            comb.mute();
        }
        for comb in &mut self.comb_filters_right {
            comb.mute();
        }
        for allpass in &mut self.allpass_filters_left {
            allpass.mute();
        }
        for allpass in &mut self.allpass_filters_right {
            allpass.mute();
        }
    }

    fn node_type(&self) -> &str {
        "Reverb"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self::new(self.name.clone()))
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
