use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, SignalType};
use crate::audio::midi::MidiEvent;
use crate::audio::track::TrackId;

/// Subtrack inputs node for metatracks.
///
/// Exposes one output port per child track so users can route individual subtracks
/// independently in the mixing graph (e.g., for sidechain effects).
///
/// Audio is injected into pre-allocated per-slot buffers by the render system before
/// the graph is processed — no heap allocation occurs during audio rendering.
pub struct SubtrackInputsNode {
    name: String,
    /// Ordered list of (TrackId, display_name) for each subtrack slot.
    /// TrackId is used by the render system to match the right buffer to the right slot.
    subtracks: Vec<(TrackId, String)>,
    /// Output port descriptors — rebuilt whenever subtracks changes.
    outputs: Vec<NodePort>,
    /// Pre-allocated audio buffers, one per subtrack slot (stereo interleaved, length = buffer_size * 2).
    /// Filled by `inject_subtrack_audio` before graph processing; no alloc per frame.
    buffers: Vec<Vec<f32>>,
    /// The buffer size this node was last sized for.
    buffer_size: usize,
}

impl SubtrackInputsNode {
    pub fn new(name: impl Into<String>, subtracks: Vec<(TrackId, String)>) -> Self {
        let outputs = Self::build_outputs(&subtracks);
        let n = subtracks.len();
        Self {
            name: name.into(),
            subtracks,
            outputs,
            buffers: vec![Vec::new(); n],
            buffer_size: 0,
        }
    }

    fn build_outputs(subtracks: &[(TrackId, String)]) -> Vec<NodePort> {
        subtracks
            .iter()
            .enumerate()
            .map(|(i, (_, name))| NodePort::new(name.as_str(), SignalType::Audio, i))
            .collect()
    }

    /// Inject audio from a child track into its pre-allocated slot.
    ///
    /// `idx` is the slot index (matching the order in `subtracks`).
    /// Called by the render system once per child per frame — no allocation.
    pub fn inject_subtrack_audio(&mut self, idx: usize, audio: &[f32]) {
        if let Some(buf) = self.buffers.get_mut(idx) {
            let len = buf.len().min(audio.len());
            buf[..len].copy_from_slice(&audio[..len]);
            // Zero any remaining samples if audio is shorter than the buffer
            if audio.len() < buf.len() {
                buf[audio.len()..].fill(0.0);
            }
        }
    }

    /// Rebuild ports and resize pre-allocated buffers.
    ///
    /// Only reallocates when the subtrack list actually changes in size or content.
    /// Pass `buffer_size` in frames (stereo buffers will be `buffer_size * 2` samples).
    pub fn update_subtracks(&mut self, subtracks: Vec<(TrackId, String)>, buffer_size: usize) {
        let n = subtracks.len();
        self.outputs = Self::build_outputs(&subtracks);
        self.subtracks = subtracks;
        self.buffer_size = buffer_size;

        // Resize buffers: keep existing allocations where possible
        self.buffers.resize_with(n, Vec::new);
        for buf in &mut self.buffers {
            let target = buffer_size * 2; // stereo interleaved
            if buf.len() != target {
                buf.resize(target, 0.0);
            }
        }
    }

    /// Return the slot index for the given TrackId, or None if not found.
    pub fn subtrack_index_for(&self, track_id: TrackId) -> Option<usize> {
        self.subtracks.iter().position(|(id, _)| *id == track_id)
    }

    /// Return the number of subtrack slots.
    pub fn num_subtracks(&self) -> usize {
        self.subtracks.len()
    }

    /// Return the ordered subtrack list.
    pub fn subtracks(&self) -> &[(TrackId, String)] {
        &self.subtracks
    }
}

impl AudioNode for SubtrackInputsNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Input
    }

    fn inputs(&self) -> &[NodePort] {
        &[] // No inputs — audio is injected externally
    }

    fn outputs(&self) -> &[NodePort] {
        &self.outputs
    }

    fn parameters(&self) -> &[Parameter] {
        &[] // No user-facing parameters; port count is stored via num_ports in serialization
    }

    fn set_parameter(&mut self, _id: u32, _value: f32) {}

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
        // Copy each pre-filled buffer to its output port
        for (i, output) in outputs.iter_mut().enumerate() {
            if let Some(buf) = self.buffers.get(i) {
                let len = output.len().min(buf.len());
                if len > 0 {
                    output[..len].copy_from_slice(&buf[..len]);
                }
                if output.len() > len {
                    output[len..].fill(0.0);
                }
            } else {
                output.fill(0.0);
            }
        }
    }

    fn reset(&mut self) {
        for buf in &mut self.buffers {
            buf.fill(0.0);
        }
    }

    fn node_type(&self) -> &str {
        "SubtrackInputs"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            subtracks: self.subtracks.clone(),
            outputs: self.outputs.clone(),
            // Don't clone audio buffers; fresh node starts silent
            buffers: vec![vec![0.0; self.buffer_size * 2]; self.subtracks.len()],
            buffer_size: self.buffer_size,
        })
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
