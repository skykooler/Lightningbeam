use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

/// Mixer node — combines N audio inputs with independent gain controls.
///
/// The number of input ports is dynamic: one spare unconnected port is always present
/// beyond however many are currently wired, so users can keep patching in without
/// manually adding inputs. Port count is managed by `AudioGraph::connect` /
/// `AudioGraph::disconnect` calling `ensure_min_ports` / `resize`.
///
/// Gain values are stored separately from the port list so they survive resize
/// operations and can be set via `set_parameter` before the port is visible.
pub struct MixerNode {
    name: String,
    /// Displayed input ports. Length = num_ports (connected + 1 spare).
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    /// Per-channel gains, indexed by port. May be longer than `inputs` if gains
    /// were set before ports were created (handled gracefully).
    gains: Vec<f32>,
    /// Mirrored parameter list so `parameters()` stays in sync with `inputs`.
    parameters: Vec<Parameter>,
}

impl MixerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let mut node = Self {
            name: name.into(),
            inputs: Vec::new(),
            outputs: vec![NodePort::new("Mixed Out", SignalType::Audio, 0)],
            gains: Vec::new(),
            parameters: Vec::new(),
        };
        node.resize(1); // start with one spare input
        node
    }

    /// Return the current number of input ports (connected + 1 spare).
    pub fn num_inputs(&self) -> usize {
        self.inputs.len()
    }

    /// Set the exact number of input ports.
    ///
    /// Existing gain values are preserved. Truncates spare gains when shrinking,
    /// but gain slots that have already been written survive a grow-shrink-grow cycle.
    pub fn resize(&mut self, n: usize) {
        let n = n.max(1); // always at least one spare

        self.inputs = (0..n)
            .map(|i| NodePort::new(format!("Input {}", i + 1).as_str(), SignalType::Audio, i))
            .collect();

        // Extend gains with 1.0 for new slots; preserve existing values.
        if self.gains.len() < n {
            self.gains.resize(n, 1.0);
        }

        self.parameters = (0..n)
            .map(|i| {
                Parameter::new(i as u32, format!("Gain {}", i + 1).as_str(), 0.0, 2.0, 1.0, ParameterUnit::Generic)
            })
            .collect();
    }

    /// Ensure at least `n` input ports exist, growing if needed but never shrinking.
    ///
    /// Called by `AudioGraph::connect` after adding a connection.
    pub fn ensure_min_ports(&mut self, n: usize) {
        if n > self.inputs.len() {
            self.resize(n);
        }
    }
}

impl AudioNode for MixerNode {
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
        let idx = id as usize;
        // Extend gains if this port hasn't been created yet (e.g. loaded from preset
        // before connections are restored).
        if idx >= self.gains.len() {
            self.gains.resize(idx + 1, 1.0);
        }
        self.gains[idx] = value.clamp(0.0, 2.0);
    }

    fn get_parameter(&self, id: u32) -> f32 {
        self.gains.get(id as usize).copied().unwrap_or(1.0)
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        _sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let frames = output.len() / 2;
        output.fill(0.0);

        for (input_idx, input) in inputs.iter().enumerate() {
            let gain = self.gains.get(input_idx).copied().unwrap_or(1.0);
            let input_frames = input.len() / 2;
            let process_frames = frames.min(input_frames);

            for frame in 0..process_frames {
                output[frame * 2]     += input[frame * 2]     * gain; // Left
                output[frame * 2 + 1] += input[frame * 2 + 1] * gain; // Right
            }
        }
    }

    fn reset(&mut self) {
        // No per-frame state
    }

    fn node_type(&self) -> &str {
        "Mixer"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            gains: self.gains.clone(),
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
