use crate::audio::midi::MidiEvent;
use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};

const PARAM_PITCH_BEND_RANGE: u32 = 0;

/// MIDI to CV converter
/// Converts MIDI note events to control voltage signals
pub struct MidiToCVNode {
    name: String,
    note: u8,               // Current MIDI note number
    gate: f32,              // Gate CV (1.0 when note on, 0.0 when off)
    velocity: f32,          // Velocity CV (0.0-1.0)
    pitch_cv: f32,          // Pitch CV (V/Oct: 0V = A4, ±1V per octave), without bend
    pitch_bend_range: f32,  // Pitch bend range in semitones (default 2.0)
    current_bend: f32,      // Current pitch bend, normalised -1.0..=1.0 (0 = centre)
    current_mod: f32,       // Current modulation (CC1), 0.0..=1.0
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl MidiToCVNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("MIDI In", SignalType::Midi, 0),
            NodePort::new("Bend CV", SignalType::CV, 0),  // External pitch bend in semitones
            NodePort::new("Mod CV", SignalType::CV, 1),   // External modulation 0.0..=1.0
        ];

        let outputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),     // V/Oct: 0V = A4, ±1V per octave (with bend applied)
            NodePort::new("Gate", SignalType::CV, 1),      // 1.0 = on, 0.0 = off
            NodePort::new("Velocity", SignalType::CV, 2),  // 0.0-1.0
            NodePort::new("Bend", SignalType::CV, 3),      // Total pitch bend in semitones (MIDI + CV)
            NodePort::new("Mod", SignalType::CV, 4),       // Total modulation 0.0..=1.0 (MIDI CC1 + CV)
        ];

        let parameters = vec![
            Parameter::new(
                PARAM_PITCH_BEND_RANGE,
                "Pitch Bend Range",
                0.0, 48.0, 2.0,
                ParameterUnit::Generic,
            ),
        ];

        Self {
            name,
            note: 60,
            gate: 0.0,
            velocity: 0.0,
            pitch_cv: Self::midi_note_to_voct(60),
            pitch_bend_range: 2.0,
            current_bend: 0.0,
            current_mod: 0.0,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Convert MIDI note to V/oct CV (proper V/Oct standard)
    /// 0V = A4 (MIDI 69), ±1V per octave
    /// Middle C (MIDI 60) = -0.75V, A5 (MIDI 81) = +1.0V
    fn midi_note_to_voct(note: u8) -> f32 {
        // Standard V/Oct: 0V at A4, 1V per octave (12 semitones)
        (note as f32 - 69.0) / 12.0
    }

    fn apply_midi_event(&mut self, event: &MidiEvent) {
        let status = event.status & 0xF0;
        match status {
            0x90 if event.data2 > 0 => {
                // Note on — reset per-note expression so previous note's bend doesn't bleed in
                self.note = event.data1;
                self.pitch_cv = Self::midi_note_to_voct(self.note);
                self.velocity = event.data2 as f32 / 127.0;
                self.gate = 1.0;
                self.current_bend = 0.0;
                self.current_mod = 0.0;
            }
            0x80 | 0x90 => {
                // Note off (or note on with velocity 0)
                if event.data1 == self.note {
                    self.gate = 0.0;
                }
            }
            0xE0 => {
                // Pitch bend: 14-bit value, center = 8192
                let bend_raw = ((event.data2 as i16) << 7) | (event.data1 as i16);
                self.current_bend = (bend_raw - 8192) as f32 / 8192.0;
            }
            0xB0 if event.data1 == 1 => {
                // CC1 (modulation wheel)
                self.current_mod = event.data2 as f32 / 127.0;
            }
            _ => {}
        }
    }
}

impl AudioNode for MidiToCVNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Input
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
        if id == PARAM_PITCH_BEND_RANGE {
            self.pitch_bend_range = value.clamp(0.0, 48.0);
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        if id == PARAM_PITCH_BEND_RANGE {
            self.pitch_bend_range
        } else {
            0.0
        }
    }

    fn handle_midi(&mut self, event: &MidiEvent) {
        self.apply_midi_event(event);
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        // Process MIDI events from input buffer
        if !midi_inputs.is_empty() {
            for event in midi_inputs[0] {
                self.apply_midi_event(event);
            }
        }

        if outputs.len() < 5 {
            return;
        }

        // Read CV inputs (use first sample of buffer). NaN = unconnected port → treat as 0.
        let bend_cv = inputs.get(0).and_then(|b| b.first().copied())
            .filter(|v| v.is_finite()).unwrap_or(0.0);
        let mod_cv  = inputs.get(1).and_then(|b| b.first().copied())
            .filter(|v| v.is_finite()).unwrap_or(0.0);

        // Total bend in semitones: MIDI bend + CV bend
        let bend_semitones = self.current_bend * self.pitch_bend_range + bend_cv;
        // Total mod: MIDI CC1 + CV mod, clamped to 0..1
        let total_mod = (self.current_mod + mod_cv).clamp(0.0, 1.0);
        // Pitch output includes bend
        let pitch_out_val = self.pitch_cv + bend_semitones / 12.0;

        // Use split_at_mut to get multiple mutable references
        let (v0, rest) = outputs.split_at_mut(1);
        let (v1, rest) = rest.split_at_mut(1);
        let (v2, rest) = rest.split_at_mut(1);
        let (v3, v4_slice) = rest.split_at_mut(1);

        let pitch_out    = &mut v0[0];
        let gate_out     = &mut v1[0];
        let velocity_out = &mut v2[0];
        let bend_out     = &mut v3[0];
        let mod_out      = &mut v4_slice[0];

        let frames = pitch_out.len();

        // Output constant CV values for the entire buffer
        for frame in 0..frames {
            pitch_out[frame]    = pitch_out_val;
            gate_out[frame]     = self.gate;
            velocity_out[frame] = self.velocity;
            bend_out[frame]     = bend_semitones;
            mod_out[frame]      = total_mod;
        }
    }

    fn reset(&mut self) {
        self.gate = 0.0;
        self.velocity = 0.0;
        self.current_bend = 0.0;
        self.current_mod = 0.0;
    }

    fn node_type(&self) -> &str {
        "MidiToCV"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            note: 60,
            gate: 0.0,
            velocity: 0.0,
            pitch_cv: Self::midi_note_to_voct(60),
            pitch_bend_range: self.pitch_bend_range,
            current_bend: 0.0,
            current_mod: 0.0,
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
