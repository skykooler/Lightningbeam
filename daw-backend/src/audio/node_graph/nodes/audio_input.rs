use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;

/// Audio input node - receives audio from audio track clip playback
/// This node acts as the entry point for audio tracks, injecting clip audio into the effects graph
pub struct AudioInputNode {
    name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    /// Internal buffer to hold injected audio from clips
    /// This is filled externally by AudioTrack::render() before graph processing
    audio_buffer: Vec<f32>,
}

impl AudioInputNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        // Audio input node has no inputs - audio is injected externally
        let inputs = vec![];

        // Outputs stereo audio
        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        Self {
            name,
            inputs,
            outputs,
            audio_buffer: Vec::new(),
        }
    }

    /// Inject audio from clip playback into this node
    /// Should be called by AudioTrack::render() before processing the graph
    pub fn inject_audio(&mut self, audio: &[f32]) {
        self.audio_buffer.clear();
        self.audio_buffer.extend_from_slice(audio);
    }

    /// Clear the internal audio buffer
    pub fn clear_buffer(&mut self) {
        self.audio_buffer.clear();
    }
}

impl AudioNode for AudioInputNode {
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
        &[] // No parameters
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
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let len = output.len().min(self.audio_buffer.len());

        // Copy audio from internal buffer to output
        if len > 0 {
            output[..len].copy_from_slice(&self.audio_buffer[..len]);
        }

        // Clear any remaining samples in output
        if output.len() > len {
            output[len..].fill(0.0);
        }
    }

    fn reset(&mut self) {
        self.audio_buffer.clear();
    }

    fn node_type(&self) -> &str {
        "AudioInput"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            audio_buffer: Vec::new(), // Don't clone the buffer, start fresh
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
