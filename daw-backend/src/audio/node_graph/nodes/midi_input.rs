use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// MIDI Input node - receives MIDI events from the track and passes them through
pub struct MidiInputNode {
    name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
    pending_events: Vec<MidiEvent>,
}

impl MidiInputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![];
        let outputs = vec![
            NodePort::new("MIDI Out", SignalType::Midi, 0),
        ];

        Self {
            name,
            inputs,
            outputs,
            parameters: vec![],
            pending_events: Vec::new(),
        }
    }

    /// Add MIDI events to be processed
    pub fn add_midi_events(&mut self, events: Vec<MidiEvent>) {
        self.pending_events.extend(events);
    }

    /// Get pending MIDI events (used for routing to connected nodes)
    pub fn take_midi_events(&mut self) -> Vec<MidiEvent> {
        std::mem::take(&mut self.pending_events)
    }
}

impl AudioNode for MidiInputNode {
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

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        _outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        // MidiInput receives MIDI from external sources (marked as MIDI target)
        // and outputs it through the graph
        // The MIDI was already placed in midi_outputs by the graph before calling process()
    }

    fn reset(&mut self) {
        self.pending_events.clear();
    }

    fn node_type(&self) -> &str {
        "MidiInput"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
            pending_events: Vec::new(),
        })
    }

    fn handle_midi(&mut self, event: &MidiEvent) {
        self.pending_events.push(*event);
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
