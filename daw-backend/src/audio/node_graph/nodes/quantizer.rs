use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_SCALE: u32 = 0;
const PARAM_ROOT_NOTE: u32 = 1;

/// Quantizer - snaps CV values to musical scales
/// Converts continuous CV into discrete pitch values based on a scale
/// Scale parameter:
/// 0 = Chromatic (all 12 notes)
/// 1 = Major scale
/// 2 = Minor scale (natural)
/// 3 = Pentatonic major
/// 4 = Pentatonic minor
/// 5 = Dorian
/// 6 = Phrygian
/// 7 = Lydian
/// 8 = Mixolydian
/// 9 = Whole tone
/// 10 = Octaves only
pub struct QuantizerNode {
    name: String,
    scale: u32,
    root_note: u32,      // 0-11 (C-B)
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl QuantizerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("CV In", SignalType::CV, 0),
        ];

        let outputs = vec![
            NodePort::new("CV Out", SignalType::CV, 0),
            NodePort::new("Gate Out", SignalType::CV, 1), // Trigger when note changes
        ];

        let parameters = vec![
            Parameter::new(PARAM_SCALE, "Scale", 0.0, 10.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_ROOT_NOTE, "Root", 0.0, 11.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            scale: 0,
            root_note: 0,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Get the scale intervals (semitones from root)
    fn get_scale_intervals(&self) -> Vec<u32> {
        match self.scale {
            0 => vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11], // Chromatic
            1 => vec![0, 2, 4, 5, 7, 9, 11],                  // Major
            2 => vec![0, 2, 3, 5, 7, 8, 10],                  // Minor (natural)
            3 => vec![0, 2, 4, 7, 9],                         // Pentatonic major
            4 => vec![0, 3, 5, 7, 10],                        // Pentatonic minor
            5 => vec![0, 2, 3, 5, 7, 9, 10],                  // Dorian
            6 => vec![0, 1, 3, 5, 7, 8, 10],                  // Phrygian
            7 => vec![0, 2, 4, 6, 7, 9, 11],                  // Lydian
            8 => vec![0, 2, 4, 5, 7, 9, 10],                  // Mixolydian
            9 => vec![0, 2, 4, 6, 8, 10],                     // Whole tone
            10 => vec![0],                                     // Octaves only
            _ => vec![0, 2, 4, 5, 7, 9, 11],                  // Default to major
        }
    }

    /// Quantize a CV value to the nearest note in the scale
    fn quantize(&self, cv: f32) -> f32 {
        // Convert V/Oct to MIDI note (standard: 0V = A4 = MIDI 69)
        // cv = (midi_note - 69) / 12.0
        // midi_note = cv * 12.0 + 69
        let input_midi_note = cv * 12.0 + 69.0;

        // Clamp to reasonable range
        let input_midi_note = input_midi_note.clamp(0.0, 127.0);

        // Get scale intervals (relative to root)
        let intervals = self.get_scale_intervals();

        // Find which octave we're in (relative to C)
        let octave = (input_midi_note / 12.0).floor() as i32;
        let note_in_octave = input_midi_note % 12.0;

        // Adjust note relative to root (e.g., if root is D (2), then C becomes 10, D becomes 0)
        let note_relative_to_root = (note_in_octave - self.root_note as f32 + 12.0) % 12.0;

        // Find the nearest note in the scale (scale intervals are relative to root)
        let mut closest_interval = intervals[0];
        let mut min_distance = (note_relative_to_root - closest_interval as f32).abs();

        for &interval in &intervals {
            let distance = (note_relative_to_root - interval as f32).abs();
            if distance < min_distance {
                min_distance = distance;
                closest_interval = interval;
            }
        }

        // Calculate final MIDI note
        // The scale interval is relative to root, so add root back to get absolute note
        let quantized_note_in_octave = (self.root_note + closest_interval) % 12;
        let quantized_midi_note = (octave * 12) as f32 + quantized_note_in_octave as f32;

        // Clamp result
        let quantized_midi_note = quantized_midi_note.clamp(0.0, 127.0);

        // Convert back to V/Oct: voct = (midi_note - 69) / 12.0
        (quantized_midi_note - 69.0) / 12.0
    }
}

impl AudioNode for QuantizerNode {
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
            PARAM_SCALE => self.scale = (value as u32).clamp(0, 10),
            PARAM_ROOT_NOTE => self.root_note = (value as u32).clamp(0, 11),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_SCALE => self.scale as f32,
            PARAM_ROOT_NOTE => self.root_note as f32,
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
        let length = input.len().min(outputs[0].len());

        // Split outputs to avoid borrow conflicts
        if outputs.len() > 1 {
            let (cv_out, gate_out) = outputs.split_at_mut(1);
            let cv_output = &mut cv_out[0];
            let gate_output = &mut gate_out[0];
            let gate_length = length.min(gate_output.len());

            let mut last_note: Option<f32> = None;

            for i in 0..length {
                let quantized = self.quantize(input[i]);
                cv_output[i] = quantized;

                // Generate gate trigger when note changes
                if i < gate_length {
                    if let Some(prev) = last_note {
                        gate_output[i] = if (quantized - prev).abs() > 0.001 { 1.0 } else { 0.0 };
                    } else {
                        gate_output[i] = 1.0; // First note triggers gate
                    }
                }

                last_note = Some(quantized);
            }
        } else {
            // No gate output, just quantize CV
            let cv_output = &mut outputs[0];
            for i in 0..length {
                cv_output[i] = self.quantize(input[i]);
            }
        }
    }

    fn reset(&mut self) {
        // No state to reset
    }

    fn node_type(&self) -> &str {
        "Quantizer"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            scale: self.scale,
            root_note: self.root_note,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
