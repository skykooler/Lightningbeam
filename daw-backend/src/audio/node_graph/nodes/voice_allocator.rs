use crate::audio::midi::MidiEvent;
use crate::audio::node_graph::{AudioNode, InstrumentGraph, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};

const PARAM_VOICE_COUNT: u32 = 0;
const MAX_VOICES: usize = 16; // Maximum allowed voices
const DEFAULT_VOICES: usize = 8;

/// Voice state for voice allocation
#[derive(Clone)]
struct VoiceState {
    active: bool,
    note: u8,
    age: u32, // For voice stealing
    pending_events: Vec<MidiEvent>, // MIDI events to send to this voice
}

impl VoiceState {
    fn new() -> Self {
        Self {
            active: false,
            note: 0,
            age: 0,
            pending_events: Vec::new(),
        }
    }
}

/// VoiceAllocatorNode - A group node that creates N polyphonic instances of its internal graph
///
/// This node acts as a container for a "voice template" graph. At runtime, it creates
/// N instances of that graph (one per voice) and routes MIDI note events to them.
/// All voice outputs are mixed together into a single output.
pub struct VoiceAllocatorNode {
    name: String,

    /// The template graph (edited by user via UI)
    template_graph: InstrumentGraph,

    /// Runtime voice instances (clones of template)
    voice_instances: Vec<InstrumentGraph>,

    /// Voice allocation state
    voices: [VoiceState; MAX_VOICES],

    /// Number of active voices (configurable parameter)
    voice_count: usize,

    /// Mix buffer for combining voice outputs
    mix_buffer: Vec<f32>,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl VoiceAllocatorNode {
    pub fn new(name: impl Into<String>, sample_rate: u32, buffer_size: usize) -> Self {
        let name = name.into();

        // MIDI input for receiving note events
        let inputs = vec![
            NodePort::new("MIDI In", SignalType::Midi, 0),
        ];

        // Single mixed audio output
        let outputs = vec![
            NodePort::new("Mixed Out", SignalType::Audio, 0),
        ];

        // Voice count parameter
        let parameters = vec![
            Parameter::new(PARAM_VOICE_COUNT, "Voices", 1.0, MAX_VOICES as f32, DEFAULT_VOICES as f32, ParameterUnit::Generic),
        ];

        // Create empty template graph
        let template_graph = InstrumentGraph::new(sample_rate, buffer_size);

        // Create voice instances (initially empty clones of template)
        let voice_instances: Vec<InstrumentGraph> = (0..MAX_VOICES)
            .map(|_| InstrumentGraph::new(sample_rate, buffer_size))
            .collect();

        Self {
            name,
            template_graph,
            voice_instances,
            voices: std::array::from_fn(|_| VoiceState::new()),
            voice_count: DEFAULT_VOICES,
            mix_buffer: vec![0.0; buffer_size * 2], // Stereo
            inputs,
            outputs,
            parameters,
        }
    }

    /// Get mutable reference to template graph (for UI editing)
    pub fn template_graph_mut(&mut self) -> &mut InstrumentGraph {
        &mut self.template_graph
    }

    /// Get reference to template graph (for serialization)
    pub fn template_graph(&self) -> &InstrumentGraph {
        &self.template_graph
    }

    /// Rebuild voice instances from template (called after template is edited)
    pub fn rebuild_voices(&mut self) {
        // Clone template to all voice instances
        for voice in &mut self.voice_instances {
            *voice = self.template_graph.clone_graph();

            // Find TemplateInput and TemplateOutput nodes
            let mut template_input_idx = None;
            let mut template_output_idx = None;

            for node_idx in voice.node_indices() {
                if let Some(node) = voice.get_node(node_idx) {
                    match node.node_type() {
                        "TemplateInput" => template_input_idx = Some(node_idx),
                        "TemplateOutput" => template_output_idx = Some(node_idx),
                        _ => {}
                    }
                }
            }

            // Mark ONLY TemplateInput as a MIDI target
            // MIDI will flow through graph connections to other nodes (like MidiToCV)
            if let Some(input_idx) = template_input_idx {
                voice.set_midi_target(input_idx, true);
            }

            // Set TemplateOutput as output node
            voice.set_output_node(template_output_idx);
        }
    }

    /// Find a free voice, or steal the oldest one
    fn find_voice_for_note_on(&mut self) -> usize {
        // Only search within active voice_count
        // First, look for an inactive voice
        for (i, voice) in self.voices[..self.voice_count].iter().enumerate() {
            if !voice.active {
                return i;
            }
        }

        // No free voices, steal the oldest one within voice_count
        self.voices[..self.voice_count]
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Find all voices playing a specific note
    fn find_voices_for_note_off(&self, note: u8) -> Vec<usize> {
        self.voices[..self.voice_count]
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                if v.active && v.note == note {
                    Some(i)
                } else {
                    None
                }
            })
            .collect()
    }
}

impl AudioNode for VoiceAllocatorNode {
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
            PARAM_VOICE_COUNT => {
                let new_count = (value.round() as usize).clamp(1, MAX_VOICES);
                if new_count != self.voice_count {
                    self.voice_count = new_count;
                    // Stop voices beyond the new count
                    for voice in &mut self.voices[new_count..] {
                        voice.active = false;
                    }
                }
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_VOICE_COUNT => self.voice_count as f32,
            _ => 0.0,
        }
    }

    fn handle_midi(&mut self, event: &MidiEvent) {
        let status = event.status & 0xF0;

        match status {
            0x90 => {
                // Note on
                if event.data2 > 0 {
                    let voice_idx = self.find_voice_for_note_on();
                    self.voices[voice_idx].active = true;
                    self.voices[voice_idx].note = event.data1;
                    self.voices[voice_idx].age = 0;

                    // Store MIDI event for this voice to process
                    self.voices[voice_idx].pending_events.push(*event);
                } else {
                    // Velocity = 0 means note off - send to ALL voices playing this note
                    let voice_indices = self.find_voices_for_note_off(event.data1);
                    for voice_idx in voice_indices {
                        self.voices[voice_idx].active = false;
                        self.voices[voice_idx].pending_events.push(*event);
                    }
                }
            }
            0x80 => {
                // Note off - send to ALL voices playing this note
                let voice_indices = self.find_voices_for_note_off(event.data1);
                for voice_idx in voice_indices {
                    self.voices[voice_idx].active = false;
                    self.voices[voice_idx].pending_events.push(*event);
                }
            }
            _ => {
                // Other MIDI events (CC, pitch bend, etc.) - send to all active voices
                for voice_idx in 0..self.voice_count {
                    if self.voices[voice_idx].active {
                        self.voices[voice_idx].pending_events.push(*event);
                    }
                }
            }
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
        // Process MIDI events from input (allocate notes to voices)
        if !midi_inputs.is_empty() {
            for event in midi_inputs[0] {
                self.handle_midi(event);
            }
        }

        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let output_len = output.len();

        // Process each active voice and mix (only up to voice_count)
        for voice_idx in 0..self.voice_count {
            let voice_state = &mut self.voices[voice_idx];
            if voice_state.active {
                voice_state.age = voice_state.age.saturating_add(1);

                // Get pending MIDI events for this voice
                let midi_events = std::mem::take(&mut voice_state.pending_events);

                // IMPORTANT: Process only the slice of mix_buffer that matches output size
                // This prevents phase discontinuities in oscillators
                let mix_slice = &mut self.mix_buffer[..output_len];
                mix_slice.fill(0.0);

                // Process this voice's graph with its MIDI events
                self.voice_instances[voice_idx].process(mix_slice, &midi_events);

                // Mix into output (accumulate)
                for (i, sample) in mix_slice.iter().enumerate() {
                    output[i] += sample;
                }
            }
        }

        // Apply normalization to prevent clipping (divide by active voice count)
        let active_count = self.voices[..self.voice_count].iter().filter(|v| v.active).count();
        if active_count > 1 {
            let scale = 1.0 / (active_count as f32).sqrt(); // Use sqrt for better loudness perception
            for sample in output.iter_mut() {
                *sample *= scale;
            }
        }
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
            voice.pending_events.clear();
        }
        for graph in &mut self.voice_instances {
            graph.reset();
        }
        self.template_graph.reset();
    }

    fn node_type(&self) -> &str {
        "VoiceAllocator"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        // Clone creates a new VoiceAllocator with the same template graph
        // Voice instances will be rebuilt when rebuild_voices() is called
        Box::new(Self {
            name: self.name.clone(),
            template_graph: self.template_graph.clone_graph(),
            voice_instances: self.voice_instances.iter().map(|g| g.clone_graph()).collect(),
            voices: std::array::from_fn(|_| VoiceState::new()), // Reset voices
            voice_count: self.voice_count,
            mix_buffer: vec![0.0; self.mix_buffer.len()],
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }
}
