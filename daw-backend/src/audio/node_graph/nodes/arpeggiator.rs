use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType, cv_input_or_default};
use crate::audio::midi::MidiEvent;

const PARAM_MODE: u32 = 0;
const PARAM_DIRECTION: u32 = 1;
const PARAM_OCTAVES: u32 = 2;
const PARAM_RETRIGGER: u32 = 3;

/// ~1ms gate-off for re-triggering at 48kHz
const RETRIGGER_SAMPLES: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArpMode {
    OnePerCycle = 0,
    AllPerCycle = 1,
}

impl ArpMode {
    fn from_f32(v: f32) -> Self {
        if v.round() as i32 >= 1 { ArpMode::AllPerCycle } else { ArpMode::OnePerCycle }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ArpDirection {
    Up = 0,
    Down = 1,
    UpDown = 2,
    Random = 3,
}

impl ArpDirection {
    fn from_f32(v: f32) -> Self {
        match v.round() as i32 {
            1 => ArpDirection::Down,
            2 => ArpDirection::UpDown,
            3 => ArpDirection::Random,
            _ => ArpDirection::Up,
        }
    }
}

/// Arpeggiator node — takes MIDI input (held chord) and a CV phase input,
/// outputs CV V/Oct + Gate stepping through the held notes.
pub struct ArpeggiatorNode {
    name: String,
    /// Currently held notes: (note, velocity), kept sorted by pitch
    held_notes: Vec<(u8, u8)>,
    /// Expanded sequence after applying direction + octaves
    sequence: Vec<(u8, u8)>,
    /// Current position in the sequence (for OnePerCycle mode)
    current_step: usize,
    /// Previous phase value for wraparound detection
    prev_phase: f32,
    /// Countdown for gate re-trigger gap
    retrigger_countdown: u32,
    /// Current output values
    current_voct: f32,
    current_gate: f32,
    /// Parameters
    mode: ArpMode,
    direction: ArpDirection,
    octaves: u32,
    retrigger: bool,
    /// For Up/Down direction tracking
    going_up: bool,
    /// Track whether sequence needs rebuilding
    sequence_dirty: bool,
    /// Stateful PRNG for random direction
    rng_state: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl ArpeggiatorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let inputs = vec![
            NodePort::new("MIDI In", SignalType::Midi, 0),
            NodePort::new("Phase", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),
            NodePort::new("Gate", SignalType::CV, 1),
        ];

        let parameters = vec![
            Parameter::new(PARAM_MODE, "Mode", 0.0, 1.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_DIRECTION, "Direction", 0.0, 3.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OCTAVES, "Octaves", 1.0, 4.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_RETRIGGER, "Retrigger", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name: name.into(),
            held_notes: Vec::new(),
            sequence: Vec::new(),
            current_step: 0,
            prev_phase: 0.0,
            retrigger_countdown: 0,
            current_voct: 0.0,
            current_gate: 0.0,
            mode: ArpMode::OnePerCycle,
            direction: ArpDirection::Up,
            octaves: 1,
            retrigger: true,
            going_up: true,
            sequence_dirty: false,
            rng_state: 12345,
            inputs,
            outputs,
            parameters,
        }
    }

    fn midi_note_to_voct(note: u8) -> f32 {
        (note as f32 - 69.0) / 12.0
    }

    fn rebuild_sequence(&mut self) {
        self.sequence.clear();
        if self.held_notes.is_empty() {
            return;
        }

        // Build base sequence sorted by pitch (held_notes is already sorted)
        let base: Vec<(u8, u8)> = self.held_notes.clone();

        // Expand across octaves
        let mut expanded = Vec::new();
        for oct in 0..self.octaves {
            for &(note, vel) in &base {
                let transposed = note.saturating_add((oct * 12) as u8);
                if transposed <= 127 {
                    expanded.push((transposed, vel));
                }
            }
        }

        // Apply direction
        match self.direction {
            ArpDirection::Up => {
                self.sequence = expanded;
            }
            ArpDirection::Down => {
                expanded.reverse();
                self.sequence = expanded;
            }
            ArpDirection::UpDown => {
                if expanded.len() > 1 {
                    let mut up_down = expanded.clone();
                    // Go back down, skipping the top and bottom notes to avoid doubles
                    for i in (1..expanded.len() - 1).rev() {
                        up_down.push(expanded[i]);
                    }
                    self.sequence = up_down;
                } else {
                    self.sequence = expanded;
                }
            }
            ArpDirection::Random => {
                // For random, keep the expanded list; we'll pick randomly in process()
                self.sequence = expanded;
            }
        }

        // Clamp current_step to valid range and update V/Oct immediately
        if !self.sequence.is_empty() {
            self.current_step = self.current_step % self.sequence.len();
            let (note, _vel) = self.sequence[self.current_step];
            self.current_voct = Self::midi_note_to_voct(note);
        } else {
            self.current_step = 0;
        }

        self.sequence_dirty = false;
    }

    fn advance_step(&mut self) {
        if self.sequence.is_empty() {
            return;
        }

        if self.direction == ArpDirection::Random {
            // Stateful xorshift32 PRNG — evolves independently of current_step
            let mut x = self.rng_state;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            self.rng_state = x;
            // Use upper bits (better distribution) and exclude current note
            if self.sequence.len() > 1 {
                let pick = ((x >> 16) as usize) % (self.sequence.len() - 1);
                self.current_step = if pick >= self.current_step { pick + 1 } else { pick };
            }
        } else {
            self.current_step = (self.current_step + 1) % self.sequence.len();
        }
    }

    fn step_changed(&mut self, new_step: usize) {
        let old_step = self.current_step;
        self.current_step = new_step;

        if !self.sequence.is_empty() {
            let (note, _vel) = self.sequence[self.current_step];
            self.current_voct = Self::midi_note_to_voct(note);
        }

        // Start retrigger gap if enabled and the step actually changed
        if self.retrigger && old_step != new_step {
            self.retrigger_countdown = RETRIGGER_SAMPLES;
        }
    }
}

impl AudioNode for ArpeggiatorNode {
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
            PARAM_MODE => self.mode = ArpMode::from_f32(value),
            PARAM_DIRECTION => {
                let new_dir = ArpDirection::from_f32(value);
                if new_dir != self.direction {
                    self.direction = new_dir;
                    self.going_up = true;
                    self.sequence_dirty = true;
                }
            }
            PARAM_OCTAVES => {
                // UI sends 0-3 (combo box index), map to 1-4 octaves
                let new_oct = (value.round() as u32 + 1).clamp(1, 4);
                if new_oct != self.octaves {
                    self.octaves = new_oct;
                    self.sequence_dirty = true;
                }
            }
            PARAM_RETRIGGER => self.retrigger = value.round() as i32 >= 1,
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_MODE => self.mode as i32 as f32,
            PARAM_DIRECTION => self.direction as i32 as f32,
            PARAM_OCTAVES => (self.octaves - 1) as f32,
            PARAM_RETRIGGER => if self.retrigger { 1.0 } else { 0.0 },
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        // Process incoming MIDI to build held_notes
        if !midi_inputs.is_empty() {
            for event in midi_inputs[0] {
                let status = event.status & 0xF0;
                match status {
                    0x90 if event.data2 > 0 => {
                        // Note on — add to held notes (sorted by pitch)
                        let note = event.data1;
                        let vel = event.data2;
                        // Remove if already held (avoid duplicates)
                        self.held_notes.retain(|&(n, _)| n != note);
                        // Insert sorted by pitch
                        let pos = self.held_notes.partition_point(|&(n, _)| n < note);
                        self.held_notes.insert(pos, (note, vel));
                        self.sequence_dirty = true;
                    }
                    0x80 | 0x90 => {
                        // Note off
                        let note = event.data1;
                        self.held_notes.retain(|&(n, _)| n != note);
                        self.sequence_dirty = true;
                    }
                    _ => {}
                }
            }
        }

        // Rebuild sequence if needed
        if self.sequence_dirty {
            self.rebuild_sequence();
        }

        if outputs.len() < 2 {
            return;
        }

        let len = outputs[0].len();

        // If no notes held, output silence
        if self.sequence.is_empty() {
            for i in 0..len {
                outputs[0][i] = self.current_voct;
                outputs[1][i] = 0.0;
            }
            self.current_gate = 0.0;
            return;
        }

        for i in 0..len {
            let phase = cv_input_or_default(inputs, 0, i, 0.0).clamp(0.0, 1.0);

            match self.mode {
                ArpMode::OnePerCycle => {
                    // Detect phase wraparound (high → low = new cycle)
                    if self.prev_phase > 0.7 && phase < 0.3 {
                        self.advance_step();
                        let step = self.current_step;
                        self.step_changed(step);
                    }
                }
                ArpMode::AllPerCycle => {
                    // Phase 0→1 maps across all sequence notes
                    let new_step = ((phase * self.sequence.len() as f32).floor() as usize)
                        .min(self.sequence.len() - 1);
                    if new_step != self.current_step {
                        self.step_changed(new_step);
                    }
                }
            }

            self.prev_phase = phase;

            // Gate: off if retriggering, on otherwise
            if self.retrigger_countdown > 0 {
                self.retrigger_countdown -= 1;
                self.current_gate = 0.0;
            } else {
                self.current_gate = 1.0;
            }

            outputs[0][i] = self.current_voct;
            outputs[1][i] = self.current_gate;
        }
    }

    fn reset(&mut self) {
        self.held_notes.clear();
        self.sequence.clear();
        self.current_step = 0;
        self.prev_phase = 0.0;
        self.retrigger_countdown = 0;
        self.current_voct = 0.0;
        self.current_gate = 0.0;
        self.going_up = true;
    }

    fn node_type(&self) -> &str {
        "Arpeggiator"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            held_notes: Vec::new(),
            sequence: Vec::new(),
            current_step: 0,
            prev_phase: 0.0,
            retrigger_countdown: 0,
            current_voct: 0.0,
            current_gate: 0.0,
            mode: self.mode,
            direction: self.direction,
            octaves: self.octaves,
            retrigger: self.retrigger,
            going_up: true,
            sequence_dirty: false,
            rng_state: 12345,
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
