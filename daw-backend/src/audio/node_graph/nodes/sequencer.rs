use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType, cv_input_or_default};
use crate::audio::midi::MidiEvent;
use crate::time::Beats;

const PARAM_MODE: u32 = 0;
const PARAM_STEPS: u32 = 1;
const PARAM_SCALE_MODE: u32 = 2;
const PARAM_KEY: u32 = 3;
const PARAM_SCALE_TYPE: u32 = 4;
const PARAM_OCTAVE: u32 = 5;
const PARAM_VELOCITY: u32 = 6;
const PARAM_ROW_BASE: u32 = 7;
const NUM_ROWS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
enum SeqMode {
    OnePerCycle = 0,
    AllPerCycle = 1,
}

impl SeqMode {
    fn from_f32(v: f32) -> Self {
        if v.round() as i32 >= 1 { SeqMode::AllPerCycle } else { SeqMode::OnePerCycle }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ScaleMode {
    Chromatic = 0,
    Diatonic = 1,
}

impl ScaleMode {
    fn from_f32(v: f32) -> Self {
        if v.round() as i32 >= 1 { ScaleMode::Diatonic } else { ScaleMode::Chromatic }
    }
}

/// Scale interval patterns (semitones from root)
const SCALES: &[&[u8]] = &[
    &[0, 2, 4, 5, 7, 9, 11],       // Major
    &[0, 2, 3, 5, 7, 8, 10],       // Minor
    &[0, 2, 3, 5, 7, 9, 10],       // Dorian
    &[0, 2, 4, 5, 7, 9, 10],       // Mixolydian
    &[0, 2, 4, 7, 9],              // Pentatonic Major
    &[0, 3, 5, 7, 10],             // Pentatonic Minor
    &[0, 3, 5, 6, 7, 10],          // Blues
    &[0, 2, 3, 5, 7, 8, 11],       // Harmonic Minor
];

/// Step Sequencer node — MxN grid of note triggers with CV phase input and MIDI output.
pub struct SequencerNode {
    name: String,
    /// Grid state: row_patterns[row] is a u16 bitmask (bit N = step N active)
    row_patterns: [u16; 16],
    num_steps: usize,
    /// Scale mapping
    scale_mode: ScaleMode,
    key: u8,
    scale_type: usize,
    base_octave: u8,
    velocity: u8,
    /// Playback state
    mode: SeqMode,
    current_step: usize,
    prev_phase: f32,
    /// Notes currently "on" from the previous step
    prev_active_notes: Vec<u8>,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl SequencerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let inputs = vec![
            NodePort::new("Phase", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("MIDI Out", SignalType::Midi, 0),
        ];

        let mut parameters = vec![
            Parameter::new(PARAM_MODE, "Mode", 0.0, 1.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_STEPS, "Steps", 0.0, 2.0, 2.0, ParameterUnit::Generic),
            Parameter::new(PARAM_SCALE_MODE, "Scale Mode", 0.0, 1.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_KEY, "Key", 0.0, 11.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_SCALE_TYPE, "Scale", 0.0, 7.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OCTAVE, "Octave", 0.0, 8.0, 4.0, ParameterUnit::Generic),
            Parameter::new(PARAM_VELOCITY, "Velocity", 1.0, 127.0, 100.0, ParameterUnit::Generic),
        ];

        // Row bitmask parameters
        for row in 0..16u32 {
            parameters.push(Parameter::new(
                PARAM_ROW_BASE + row,
                "Row",
                0.0,
                65535.0,
                0.0,
                ParameterUnit::Generic,
            ));
        }

        Self {
            name: name.into(),
            row_patterns: [0u16; 16],
            num_steps: 16,
            scale_mode: ScaleMode::Chromatic,
            key: 0,
            scale_type: 0,
            base_octave: 4,
            velocity: 100,
            mode: SeqMode::OnePerCycle,
            current_step: 0,
            prev_phase: 0.0,
            prev_active_notes: Vec::new(),
            inputs,
            outputs,
            parameters,
        }
    }

    fn steps_from_param(v: f32) -> usize {
        match v.round() as i32 {
            0 => 4,
            1 => 8,
            _ => 16,
        }
    }

    fn row_to_midi_note(&self, row: usize) -> u8 {
        let base = self.key as u16 + self.base_octave as u16 * 12;
        let note = match self.scale_mode {
            ScaleMode::Chromatic => base + row as u16,
            ScaleMode::Diatonic => {
                let scale = SCALES[self.scale_type.min(SCALES.len() - 1)];
                let octave_offset = row / scale.len();
                let degree = row % scale.len();
                base + octave_offset as u16 * 12 + scale[degree] as u16
            }
        };
        (note as u8).min(127)
    }
}

impl AudioNode for SequencerNode {
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
            PARAM_MODE => self.mode = SeqMode::from_f32(value),
            PARAM_STEPS => self.num_steps = Self::steps_from_param(value),
            PARAM_SCALE_MODE => self.scale_mode = ScaleMode::from_f32(value),
            PARAM_KEY => self.key = (value.round() as u8).min(11),
            PARAM_SCALE_TYPE => self.scale_type = (value.round() as usize).min(SCALES.len() - 1),
            PARAM_OCTAVE => self.base_octave = (value.round() as u8).min(8),
            PARAM_VELOCITY => self.velocity = (value.round() as u8).clamp(1, 127),
            id if id >= PARAM_ROW_BASE && id < PARAM_ROW_BASE + 16 => {
                let row = (id - PARAM_ROW_BASE) as usize;
                self.row_patterns[row] = value.round() as u16;
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_MODE => self.mode as i32 as f32,
            PARAM_STEPS => match self.num_steps {
                4 => 0.0,
                8 => 1.0,
                _ => 2.0,
            },
            PARAM_SCALE_MODE => self.scale_mode as i32 as f32,
            PARAM_KEY => self.key as f32,
            PARAM_SCALE_TYPE => self.scale_type as f32,
            PARAM_OCTAVE => self.base_octave as f32,
            PARAM_VELOCITY => self.velocity as f32,
            id if id >= PARAM_ROW_BASE && id < PARAM_ROW_BASE + 16 => {
                let row = (id - PARAM_ROW_BASE) as usize;
                self.row_patterns[row] as f32
            }
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if midi_outputs.is_empty() {
            return;
        }

        let len = if !inputs.is_empty() { inputs[0].len() } else { return };

        for i in 0..len {
            let phase = cv_input_or_default(inputs, 0, i, 0.0).clamp(0.0, 1.0);

            let new_step = match self.mode {
                SeqMode::OnePerCycle => {
                    if self.prev_phase > 0.7 && phase < 0.3 {
                        (self.current_step + 1) % self.num_steps
                    } else {
                        self.current_step
                    }
                }
                SeqMode::AllPerCycle => {
                    ((phase * self.num_steps as f32).floor() as usize)
                        .min(self.num_steps - 1)
                }
            };

            if new_step != self.current_step {
                // Compute active notes for the new step
                let mut new_notes = Vec::new();
                for row in 0..NUM_ROWS {
                    if self.row_patterns[row] & (1 << new_step) != 0 {
                        let note = self.row_to_midi_note(row);
                        new_notes.push(note);
                    }
                }

                // Note-off for notes no longer active
                for &note in &self.prev_active_notes {
                    if !new_notes.contains(&note) {
                        midi_outputs[0].push(MidiEvent::note_off(Beats::ZERO, 0, note, 0));
                    }
                }

                // Note-on for newly active notes
                for &note in &new_notes {
                    if !self.prev_active_notes.contains(&note) {
                        midi_outputs[0].push(MidiEvent::note_on(Beats::ZERO, 0, note, self.velocity));
                    }
                }

                self.prev_active_notes = new_notes;
                self.current_step = new_step;
            }

            self.prev_phase = phase;
        }
    }

    fn reset(&mut self) {
        self.current_step = 0;
        self.prev_phase = 0.0;
        self.prev_active_notes.clear();
    }

    fn node_type(&self) -> &str {
        "Sequencer"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            row_patterns: self.row_patterns,
            num_steps: self.num_steps,
            scale_mode: self.scale_mode,
            key: self.key,
            scale_type: self.scale_type,
            base_octave: self.base_octave,
            velocity: self.velocity,
            mode: self.mode,
            current_step: 0,
            prev_phase: 0.0,
            prev_active_notes: Vec::new(),
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
