use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_RESOLUTION: u32 = 0;

/// Hardcoded BPM until project tempo is implemented
const DEFAULT_BPM: f32 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeatResolution {
    Whole = 0,      // 1/1
    Half = 1,       // 1/2
    Quarter = 2,    // 1/4
    Eighth = 3,     // 1/8
    Sixteenth = 4,  // 1/16
    QuarterT = 5,   // 1/4 triplet
    EighthT = 6,    // 1/8 triplet
}

impl BeatResolution {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            0 => BeatResolution::Whole,
            1 => BeatResolution::Half,
            2 => BeatResolution::Quarter,
            3 => BeatResolution::Eighth,
            4 => BeatResolution::Sixteenth,
            5 => BeatResolution::QuarterT,
            6 => BeatResolution::EighthT,
            _ => BeatResolution::Quarter,
        }
    }

    /// How many subdivisions per quarter note beat
    fn subdivisions_per_beat(&self) -> f64 {
        match self {
            BeatResolution::Whole => 0.25,     // 1 per 4 beats
            BeatResolution::Half => 0.5,       // 1 per 2 beats
            BeatResolution::Quarter => 1.0,    // 1 per beat
            BeatResolution::Eighth => 2.0,     // 2 per beat
            BeatResolution::Sixteenth => 4.0,  // 4 per beat
            BeatResolution::QuarterT => 1.5,   // 3 per 2 beats (triplet)
            BeatResolution::EighthT => 3.0,    // 3 per beat (triplet)
        }
    }
}

/// Beat clock node — generates tempo-synced CV signals.
///
/// Outputs:
/// - BPM: constant CV proportional to tempo (bpm / 240)
/// - Beat Phase: sawtooth 0→1 per beat subdivision
/// - Bar Phase: sawtooth 0→1 per bar (4 beats)
/// - Gate: 1.0 for first half of each subdivision, 0.0 otherwise
pub struct BeatNode {
    name: String,
    bpm: f32,
    resolution: BeatResolution,
    /// Playback time in seconds, set by the graph before process()
    playback_time: f64,
    /// Previous playback_time to detect paused state
    prev_playback_time: f64,
    /// Cached output values held when paused
    held_beat_phase: f32,
    held_bar_phase: f32,
    held_gate: f32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl BeatNode {
    pub fn new(name: impl Into<String>) -> Self {
        let inputs = vec![];

        let outputs = vec![
            NodePort::new("BPM", SignalType::CV, 0),
            NodePort::new("Beat Phase", SignalType::CV, 1),
            NodePort::new("Bar Phase", SignalType::CV, 2),
            NodePort::new("Gate", SignalType::CV, 3),
        ];

        let parameters = vec![
            Parameter::new(PARAM_RESOLUTION, "Resolution", 0.0, 6.0, 2.0, ParameterUnit::Generic),
        ];

        Self {
            name: name.into(),
            bpm: DEFAULT_BPM,
            resolution: BeatResolution::Quarter,
            playback_time: 0.0,
            prev_playback_time: -1.0,
            held_beat_phase: 0.0,
            held_bar_phase: 0.0,
            held_gate: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    pub fn set_playback_time(&mut self, time: f64) {
        self.playback_time = time;
    }
}

impl AudioNode for BeatNode {
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
            PARAM_RESOLUTION => self.resolution = BeatResolution::from_f32(value),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_RESOLUTION => self.resolution as i32 as f32,
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
        if outputs.len() < 4 {
            return;
        }

        let bpm_cv = (self.bpm / 240.0).clamp(0.0, 1.0);
        let len = outputs[0].len();

        // Detect paused: playback_time hasn't changed since last process()
        let paused = self.playback_time == self.prev_playback_time;
        self.prev_playback_time = self.playback_time;

        if paused {
            // Hold last values
            for i in 0..len {
                outputs[0][i] = bpm_cv;
                outputs[1][i] = self.held_beat_phase;
                outputs[2][i] = self.held_bar_phase;
                outputs[3][i] = self.held_gate;
            }
            return;
        }

        let beats_per_second = self.bpm as f64 / 60.0;
        let sample_period = 1.0 / sample_rate as f64;
        let subs_per_beat = self.resolution.subdivisions_per_beat();

        for i in 0..len {
            // Derive beat position from timeline playback time
            let time = self.playback_time + i as f64 * sample_period;
            let beat_pos = time * beats_per_second;

            // Beat subdivision phase: 0→1 sawtooth
            let sub_phase = ((beat_pos * subs_per_beat) % 1.0) as f32;

            // Bar phase: 0→1 over 4 quarter-note beats
            let bar_phase = ((beat_pos / 4.0) % 1.0) as f32;

            // Gate: high for first half of each subdivision
            let gate = if sub_phase < 0.5 { 1.0f32 } else { 0.0 };

            outputs[0][i] = bpm_cv;
            outputs[1][i] = sub_phase;
            outputs[2][i] = bar_phase;
            outputs[3][i] = gate;
        }

        // Cache last sample's values for hold when paused
        if len > 0 {
            self.held_beat_phase = outputs[1][len - 1];
            self.held_bar_phase = outputs[2][len - 1];
            self.held_gate = outputs[3][len - 1];
        }
    }

    fn reset(&mut self) {
        self.playback_time = 0.0;
        self.prev_playback_time = -1.0;
        self.held_beat_phase = 0.0;
        self.held_bar_phase = 0.0;
        self.held_gate = 0.0;
    }

    fn node_type(&self) -> &str {
        "Beat"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            bpm: self.bpm,
            resolution: self.resolution,
            playback_time: 0.0,
            prev_playback_time: -1.0,
            held_beat_phase: 0.0,
            held_bar_phase: 0.0,
            held_gate: 0.0,
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
