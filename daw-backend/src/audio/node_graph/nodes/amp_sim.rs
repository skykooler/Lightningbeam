use crate::audio::midi::MidiEvent;
use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use nam_ffi::NamModel;
use std::path::Path;

const PARAM_INPUT_GAIN: u32 = 0;
const PARAM_OUTPUT_GAIN: u32 = 1;
const PARAM_MIX: u32 = 2;

/// Guitar amp simulator node using Neural Amp Modeler (.nam) models.
pub struct AmpSimNode {
    name: String,
    input_gain: f32,
    output_gain: f32,
    mix: f32,

    model: Option<NamModel>,
    model_path: Option<String>,

    // Mono scratch buffers for NAM processing (NAM is mono-only)
    mono_in: Vec<f32>,
    mono_out: Vec<f32>,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl AmpSimNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![NodePort::new("Audio In", SignalType::Audio, 0)];
        let outputs = vec![NodePort::new("Audio Out", SignalType::Audio, 0)];

        let parameters = vec![
            Parameter::new(PARAM_INPUT_GAIN, "Input Gain", 0.0, 4.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_OUTPUT_GAIN, "Output Gain", 0.0, 4.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_MIX, "Mix", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            input_gain: 1.0,
            output_gain: 1.0,
            mix: 1.0,
            model: None,
            model_path: None,
            mono_in: Vec::new(),
            mono_out: Vec::new(),
            inputs,
            outputs,
            parameters,
        }
    }

    /// Load a .nam model file. Call from the audio thread via command dispatch.
    pub fn load_model(&mut self, path: &str) -> Result<(), String> {
        let model_path = Path::new(path);
        let mut model =
            NamModel::from_file(model_path).map_err(|e| format!("{}", e))?;
        model.set_max_buffer_size(1024);
        self.model = Some(model);
        self.model_path = Some(path.to_string());
        Ok(())
    }

    /// Load a bundled NAM model by name (e.g. "BossSD1").
    pub fn load_bundled_model(&mut self, name: &str) -> Result<(), String> {
        let mut model = super::bundled_models::load_bundled_model(name)
            .ok_or_else(|| format!("Unknown bundled model: {}", name))??;
        model.set_max_buffer_size(1024);
        self.model = Some(model);
        self.model_path = Some(format!("bundled:{}", name));
        Ok(())
    }

    /// Load a .nam model from in-memory bytes (used when loading from a .lbins bundle).
    /// `zip_path` is the ZIP-relative path stored back in `model_path` for serialization.
    pub fn load_model_from_bytes(&mut self, zip_path: &str, bytes: &[u8]) -> Result<(), String> {
        let basename = std::path::Path::new(zip_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(zip_path);
        let mut model = nam_ffi::NamModel::from_bytes(basename, bytes)
            .map_err(|e| format!("{}", e))?;
        model.set_max_buffer_size(1024);
        self.model = Some(model);
        self.model_path = Some(zip_path.to_string());
        Ok(())
    }

    /// Get the loaded model path (for preset serialization).
    pub fn model_path(&self) -> Option<&str> {
        self.model_path.as_deref()
    }
}

impl AudioNode for AmpSimNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Effect
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
            PARAM_INPUT_GAIN => self.input_gain = value.clamp(0.0, 4.0),
            PARAM_OUTPUT_GAIN => self.output_gain = value.clamp(0.0, 4.0),
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_INPUT_GAIN => self.input_gain,
            PARAM_OUTPUT_GAIN => self.output_gain,
            PARAM_MIX => self.mix,
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
        let output = &mut outputs[0];

        let frames = input.len() / 2;
        let output_frames = output.len() / 2;
        let frames_to_process = frames.min(output_frames);

        if let Some(ref mut model) = self.model {
            // Ensure scratch buffers are large enough
            if self.mono_in.len() < frames_to_process {
                self.mono_in.resize(frames_to_process, 0.0);
                self.mono_out.resize(frames_to_process, 0.0);
            }

            // Deinterleave stereo to mono (average L+R) and apply input gain
            for frame in 0..frames_to_process {
                let left = input[frame * 2];
                let right = input[frame * 2 + 1];
                self.mono_in[frame] = (left + right) * 0.5 * self.input_gain;
            }

            // Process through NAM model
            model.process(
                &self.mono_in[..frames_to_process],
                &mut self.mono_out[..frames_to_process],
            );

            // Apply output gain, mix wet/dry, copy mono back to stereo
            for frame in 0..frames_to_process {
                let dry = (input[frame * 2] + input[frame * 2 + 1]) * 0.5;
                let wet = self.mono_out[frame] * self.output_gain;
                let mixed = dry * (1.0 - self.mix) + wet * self.mix;
                output[frame * 2] = mixed;
                output[frame * 2 + 1] = mixed;
            }
        } else {
            // No model loaded — pass through unchanged
            let samples = frames_to_process * 2;
            output[..samples].copy_from_slice(&input[..samples]);
        }
    }

    fn reset(&mut self) {
        // No persistent filter state to reset
    }

    fn node_type(&self) -> &str {
        "AmpSim"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        // Cannot clone the NAM model (C++ pointer), so clone without model.
        // The model will need to be reloaded via command if needed.
        Box::new(Self {
            name: self.name.clone(),
            input_gain: self.input_gain,
            output_gain: self.output_gain,
            mix: self.mix,
            model: None,
            model_path: self.model_path.clone(),
            mono_in: Vec::new(),
            mono_out: Vec::new(),
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
