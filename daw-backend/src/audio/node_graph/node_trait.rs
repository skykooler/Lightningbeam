use super::types::{NodeCategory, NodePort, Parameter};
use crate::audio::midi::MidiEvent;

/// Custom node trait for audio processing nodes
///
/// All nodes must be Send to be usable in the audio thread.
/// Nodes should be real-time safe: no allocations, no blocking operations.
pub trait AudioNode: Send {
    /// Node category for UI organization
    fn category(&self) -> NodeCategory;

    /// Input port definitions
    fn inputs(&self) -> &[NodePort];

    /// Output port definitions
    fn outputs(&self) -> &[NodePort];

    /// User-facing parameters
    fn parameters(&self) -> &[Parameter];

    /// Set parameter by ID
    fn set_parameter(&mut self, id: u32, value: f32);

    /// Get parameter by ID
    fn get_parameter(&self, id: u32) -> f32;

    /// Process audio buffers
    ///
    /// # Arguments
    /// * `inputs` - Audio/CV input buffers for each input port
    /// * `outputs` - Audio/CV output buffers for each output port
    /// * `midi_inputs` - MIDI event buffers for each MIDI input port
    /// * `midi_outputs` - MIDI event buffers for each MIDI output port
    /// * `sample_rate` - Current sample rate in Hz
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_inputs: &[&[MidiEvent]],
        midi_outputs: &mut [&mut Vec<MidiEvent>],
        sample_rate: u32,
    );

    /// Handle MIDI events (for nodes with MIDI inputs)
    fn handle_midi(&mut self, _event: &MidiEvent) {
        // Default: do nothing
    }

    /// Reset internal state (clear delays, resonances, etc.)
    fn reset(&mut self);

    /// Get the node type name (for serialization)
    fn node_type(&self) -> &str;

    /// Get a unique identifier for this node instance
    fn name(&self) -> &str;

    /// Clone this node into a new boxed instance
    /// Required for VoiceAllocator to create multiple instances
    fn clone_node(&self) -> Box<dyn AudioNode>;

    /// Get oscilloscope data if this is an oscilloscope node
    /// Returns None for non-oscilloscope nodes
    fn get_oscilloscope_data(&self, _sample_count: usize) -> Option<Vec<f32>> {
        None
    }

    /// Get oscilloscope CV data if this is an oscilloscope node
    /// Returns None for non-oscilloscope nodes
    fn get_oscilloscope_cv_data(&self, _sample_count: usize) -> Option<Vec<f32>> {
        None
    }

    /// Downcast to `&mut dyn Any` for type-specific operations
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Downcast to `&dyn Any` for type-specific read-only operations
    fn as_any(&self) -> &dyn std::any::Any;
}

/// Helper function for CV inputs with optional connections
///
/// Returns the input value if connected (not NaN), otherwise returns the default value.
/// This implements "Blender-style" input behavior where parameters are replaced by
/// connected inputs.
///
/// # Arguments
/// * `inputs` - Input buffer array from process()
/// * `port` - Input port index
/// * `frame` - Current frame index
/// * `default` - Default value to use when input is unconnected
///
/// # Returns
/// The input value if connected, otherwise the default value
#[inline]
pub fn cv_input_or_default(inputs: &[&[f32]], port: usize, frame: usize, default: f32) -> f32 {
    if port < inputs.len() && frame < inputs[port].len() {
        let value = inputs[port][frame];
        if value.is_nan() {
            // Unconnected: use default parameter value
            default
        } else {
            // Connected: use input signal
            value
        }
    } else {
        // No input buffer: use default
        default
    }
}
