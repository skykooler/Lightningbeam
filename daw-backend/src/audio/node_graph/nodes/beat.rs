use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_RESOLUTION: u32 = 0;

const DEFAULT_BPM: f32 = 120.0;
const DEFAULT_BEATS_PER_BAR: u32 = 4;

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
/// BPM and time signature are synced from the project document via SetTempo.
/// When playing: synced to timeline position.
/// When stopped: free-runs continuously at the project BPM.
///
/// Outputs:
/// - BPM: constant CV proportional to tempo (bpm / 240)
/// - Beat Phase: sawtooth 0→1 per beat subdivision
/// - Bar Phase: sawtooth 0→1 per bar (uses project time signature)
/// - Gate: 1.0 for first half of each subdivision, 0.0 otherwise
pub struct BeatNode {
    name: String,
    bpm: f32,
    beats_per_bar: u32,
    resolution: BeatResolution,
    /// Playback time in seconds, set by the graph before process()
    playback_time: f64,
    /// Previous playback_time to detect paused state
    prev_playback_time: f64,
    /// Free-running time accumulator for when playback is stopped
    free_run_time: f64,
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
            beats_per_bar: DEFAULT_BEATS_PER_BAR,
            resolution: BeatResolution::Quarter,
            playback_time: 0.0,
            prev_playback_time: -1.0,
            free_run_time: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    pub fn set_playback_time(&mut self, time: f64) {
        self.playback_time = time;
    }

    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        self.bpm = bpm;
        self.beats_per_bar = beats_per_bar;
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
        let sample_period = 1.0 / sample_rate as f64;

        // Detect paused: playback_time hasn't changed since last process()
        let paused = self.playback_time == self.prev_playback_time;
        self.prev_playback_time = self.playback_time;

        let beats_per_second = self.bpm as f64 / 60.0;
        let subs_per_beat = self.resolution.subdivisions_per_beat();

        // Choose time source: timeline when playing, free-running when stopped
        let base_time = if paused { self.free_run_time } else { self.playback_time };

        for i in 0..len {
            let time = base_time + i as f64 * sample_period;
            let beat_pos = time * beats_per_second;

            // Beat subdivision phase: 0→1 sawtooth
            let sub_phase = ((beat_pos * subs_per_beat) % 1.0) as f32;

            // Bar phase: 0→1 over one bar (beats_per_bar beats)
            let bar_phase = ((beat_pos / self.beats_per_bar as f64) % 1.0) as f32;

            // Gate: high for first half of each subdivision
            let gate = if sub_phase < 0.5 { 1.0f32 } else { 0.0 };

            outputs[0][i] = bpm_cv;
            outputs[1][i] = sub_phase;
            outputs[2][i] = bar_phase;
            outputs[3][i] = gate;
        }

        // Advance free-run time (always ticks, so it's ready when playback stops)
        self.free_run_time += len as f64 * sample_period;
    }

    fn reset(&mut self) {
        self.playback_time = 0.0;
        self.prev_playback_time = -1.0;
        self.free_run_time = 0.0;
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
            beats_per_bar: self.beats_per_bar,
            resolution: self.resolution,
            playback_time: 0.0,
            prev_playback_time: -1.0,
            free_run_time: 0.0,
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
