use crate::audio::midi::MidiEvent;
use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};

/// MIDI to CV converter
/// Converts MIDI note events to control voltage signals
pub struct MidiToCVNode {
    name: String,
    note: u8,           // Current MIDI note number
    gate: f32,          // Gate CV (1.0 when note on, 0.0 when off)
    velocity: f32,      // Velocity CV (0.0-1.0)
    pitch_cv: f32,      // Pitch CV (V/Oct: 0V = A4, ±1V per octave)
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl MidiToCVNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        // MIDI input port for receiving MIDI through graph connections
        let inputs = vec![
            NodePort::new("MIDI In", SignalType::Midi, 0),
        ];

        let outputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),     // V/Oct: 0V = A4, ±1V per octave
            NodePort::new("Gate", SignalType::CV, 1),      // 1.0 = on, 0.0 = off
            NodePort::new("Velocity", SignalType::CV, 2),  // 0.0-1.0
        ];

        Self {
            name,
            note: 60, // Middle C
            gate: 0.0,
            velocity: 0.0,
            pitch_cv: Self::midi_note_to_voct(60),
            inputs,
            outputs,
            parameters: vec![], // No user parameters
        }
    }

    /// Convert MIDI note to V/oct CV (proper V/Oct standard)
    /// 0V = A4 (MIDI 69), ±1V per octave
    /// Middle C (MIDI 60) = -0.75V, A5 (MIDI 81) = +1.0V
    fn midi_note_to_voct(note: u8) -> f32 {
        // Standard V/Oct: 0V at A4, 1V per octave (12 semitones)
        (note as f32 - 69.0) / 12.0
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

    fn set_parameter(&mut self, _id: u32, _value: f32) {
        // No parameters
    }

    fn get_parameter(&self, _id: u32) -> f32 {
        0.0
    }

    fn handle_midi(&mut self, event: &MidiEvent) {
        let status = event.status & 0xF0;

        match status {
            0x90 => {
                // Note on
                if event.data2 > 0 {
                    // Velocity > 0 means note on
                    self.note = event.data1;
                    self.pitch_cv = Self::midi_note_to_voct(self.note);
                    self.velocity = event.data2 as f32 / 127.0;
                    self.gate = 1.0;
                } else {
                    // Velocity = 0 means note off
                    if event.data1 == self.note {
                        self.gate = 0.0;
                    }
                }
            }
            0x80 => {
                // Note off
                if event.data1 == self.note {
                    self.gate = 0.0;
                }
            }
            _ => {}
        }
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        // Process MIDI events from input buffer
        if !midi_inputs.is_empty() {
            for event in midi_inputs[0] {
                let status = event.status & 0xF0;
                match status {
                    0x90 if event.data2 > 0 => {
                        // Note on
                        self.note = event.data1;
                        self.pitch_cv = Self::midi_note_to_voct(self.note);
                        self.velocity = event.data2 as f32 / 127.0;
                        self.gate = 1.0;
                    }
                    0x80 | 0x90 => {
                        // Note off (or note on with velocity 0)
                        if event.data1 == self.note {
                            self.gate = 0.0;
                        }
                    }
                    _ => {}
                }
            }
        }

        if outputs.len() < 3 {
            return;
        }

        // CV signals are mono
        // Use split_at_mut to get multiple mutable references
        let (pitch_and_rest, rest) = outputs.split_at_mut(1);
        let (gate_and_rest, velocity_slice) = rest.split_at_mut(1);

        let pitch_out = &mut pitch_and_rest[0];
        let gate_out = &mut gate_and_rest[0];
        let velocity_out = &mut velocity_slice[0];

        let frames = pitch_out.len();

        // Output constant CV values for the entire buffer
        for frame in 0..frames {
            pitch_out[frame] = self.pitch_cv;
            gate_out[frame] = self.gate;
            velocity_out[frame] = self.velocity;
        }
    }

    fn reset(&mut self) {
        self.gate = 0.0;
        self.velocity = 0.0;
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
            note: 60,           // Reset to middle C
            gate: 0.0,          // Reset gate
            velocity: 0.0,      // Reset velocity
            pitch_cv: Self::midi_note_to_voct(60), // Reset pitch
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
