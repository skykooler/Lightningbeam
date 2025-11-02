use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

// Parameters
const PARAM_GAIN: u32 = 0;
const PARAM_ATTACK: u32 = 1;
const PARAM_RELEASE: u32 = 2;
const PARAM_TRANSPOSE: u32 = 3;

/// Metadata about a loaded sample layer (for preset serialization)
#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub file_path: String,
    pub key_min: u8,
    pub key_max: u8,
    pub root_key: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
}

/// Single sample with velocity range and key range
#[derive(Clone)]
struct SampleLayer {
    sample_data: Vec<f32>,
    sample_rate: f32,

    // Key range: C-1 = 0, C0 = 12, middle C (C4) = 60, C9 = 120
    key_min: u8,
    key_max: u8,
    root_key: u8,  // The original pitch of the sample

    // Velocity range: 0-127
    velocity_min: u8,
    velocity_max: u8,
}

impl SampleLayer {
    fn new(
        sample_data: Vec<f32>,
        sample_rate: f32,
        key_min: u8,
        key_max: u8,
        root_key: u8,
        velocity_min: u8,
        velocity_max: u8,
    ) -> Self {
        Self {
            sample_data,
            sample_rate,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        }
    }

    /// Check if this layer matches the given key and velocity
    fn matches(&self, key: u8, velocity: u8) -> bool {
        key >= self.key_min
            && key <= self.key_max
            && velocity >= self.velocity_min
            && velocity <= self.velocity_max
    }
}

/// Active voice playing a sample
struct Voice {
    layer_index: usize,
    playhead: f32,
    note: u8,
    velocity: u8,
    is_active: bool,

    // Envelope
    envelope_phase: EnvelopePhase,
    envelope_value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum EnvelopePhase {
    Attack,
    Sustain,
    Release,
}

impl Voice {
    fn new(layer_index: usize, note: u8, velocity: u8) -> Self {
        Self {
            layer_index,
            playhead: 0.0,
            note,
            velocity,
            is_active: true,
            envelope_phase: EnvelopePhase::Attack,
            envelope_value: 0.0,
        }
    }
}

/// Multi-sample instrument with velocity layers and key zones
pub struct MultiSamplerNode {
    name: String,

    // Sample layers
    layers: Vec<SampleLayer>,
    layer_infos: Vec<LayerInfo>,  // Metadata about loaded layers

    // Voice management
    voices: Vec<Voice>,
    max_voices: usize,

    // Parameters
    gain: f32,
    attack_time: f32,   // seconds
    release_time: f32,  // seconds
    transpose: i8,      // semitones

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl MultiSamplerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("MIDI In", SignalType::Midi, 0),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_GAIN, "Gain", 0.0, 2.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_ATTACK, "Attack", 0.001, 1.0, 0.01, ParameterUnit::Time),
            Parameter::new(PARAM_RELEASE, "Release", 0.01, 5.0, 0.1, ParameterUnit::Time),
            Parameter::new(PARAM_TRANSPOSE, "Transpose", -24.0, 24.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            layers: Vec::new(),
            layer_infos: Vec::new(),
            voices: Vec::new(),
            max_voices: 16,
            gain: 1.0,
            attack_time: 0.01,
            release_time: 0.1,
            transpose: 0,
            inputs,
            outputs,
            parameters,
        }
    }

    /// Add a sample layer
    pub fn add_layer(
        &mut self,
        sample_data: Vec<f32>,
        sample_rate: f32,
        key_min: u8,
        key_max: u8,
        root_key: u8,
        velocity_min: u8,
        velocity_max: u8,
    ) {
        let layer = SampleLayer::new(
            sample_data,
            sample_rate,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        );
        self.layers.push(layer);
    }

    /// Load a sample layer from a file path
    pub fn load_layer_from_file(
        &mut self,
        path: &str,
        key_min: u8,
        key_max: u8,
        root_key: u8,
        velocity_min: u8,
        velocity_max: u8,
    ) -> Result<(), String> {
        use crate::audio::sample_loader::load_audio_file;

        let sample_data = load_audio_file(path)?;
        self.add_layer(
            sample_data.samples,
            sample_data.sample_rate as f32,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        );

        // Store layer metadata for preset serialization
        self.layer_infos.push(LayerInfo {
            file_path: path.to_string(),
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        });

        Ok(())
    }

    /// Get information about all loaded layers
    pub fn get_layers_info(&self) -> &[LayerInfo] {
        &self.layer_infos
    }

    /// Get sample data for a specific layer (for preset embedding)
    pub fn get_layer_data(&self, layer_index: usize) -> Option<(Vec<f32>, f32)> {
        self.layers.get(layer_index).map(|layer| {
            (layer.sample_data.clone(), layer.sample_rate)
        })
    }

    /// Update a layer's configuration
    pub fn update_layer(
        &mut self,
        layer_index: usize,
        key_min: u8,
        key_max: u8,
        root_key: u8,
        velocity_min: u8,
        velocity_max: u8,
    ) -> Result<(), String> {
        if layer_index >= self.layers.len() {
            return Err("Layer index out of bounds".to_string());
        }

        // Update the layer data
        self.layers[layer_index].key_min = key_min;
        self.layers[layer_index].key_max = key_max;
        self.layers[layer_index].root_key = root_key;
        self.layers[layer_index].velocity_min = velocity_min;
        self.layers[layer_index].velocity_max = velocity_max;

        // Update the layer info
        if layer_index < self.layer_infos.len() {
            self.layer_infos[layer_index].key_min = key_min;
            self.layer_infos[layer_index].key_max = key_max;
            self.layer_infos[layer_index].root_key = root_key;
            self.layer_infos[layer_index].velocity_min = velocity_min;
            self.layer_infos[layer_index].velocity_max = velocity_max;
        }

        Ok(())
    }

    /// Remove a layer
    pub fn remove_layer(&mut self, layer_index: usize) -> Result<(), String> {
        if layer_index >= self.layers.len() {
            return Err("Layer index out of bounds".to_string());
        }

        self.layers.remove(layer_index);
        if layer_index < self.layer_infos.len() {
            self.layer_infos.remove(layer_index);
        }

        // Stop any voices playing this layer
        for voice in &mut self.voices {
            if voice.layer_index == layer_index {
                voice.is_active = false;
            } else if voice.layer_index > layer_index {
                // Adjust indices for layers that were shifted down
                voice.layer_index -= 1;
            }
        }

        Ok(())
    }

    /// Find the best matching layer for a given note and velocity
    fn find_layer(&self, note: u8, velocity: u8) -> Option<usize> {
        self.layers
            .iter()
            .enumerate()
            .find(|(_, layer)| layer.matches(note, velocity))
            .map(|(index, _)| index)
    }

    /// Trigger a note
    fn note_on(&mut self, note: u8, velocity: u8) {
        let transposed_note = (note as i16 + self.transpose as i16).clamp(0, 127) as u8;

        if let Some(layer_index) = self.find_layer(transposed_note, velocity) {
            // Find an inactive voice or reuse the oldest one
            let voice_index = self
                .voices
                .iter()
                .position(|v| !v.is_active)
                .unwrap_or_else(|| {
                    // All voices active, reuse the first one
                    if self.voices.len() < self.max_voices {
                        self.voices.len()
                    } else {
                        0
                    }
                });

            let voice = Voice::new(layer_index, note, velocity);

            if voice_index < self.voices.len() {
                self.voices[voice_index] = voice;
            } else {
                self.voices.push(voice);
            }
        }
    }

    /// Release a note
    fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.note == note && voice.is_active {
                voice.envelope_phase = EnvelopePhase::Release;
            }
        }
    }
}

impl AudioNode for MultiSamplerNode {
    fn category(&self) -> NodeCategory {
        NodeCategory::Generator
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
            PARAM_GAIN => {
                self.gain = value.clamp(0.0, 2.0);
            }
            PARAM_ATTACK => {
                self.attack_time = value.clamp(0.001, 1.0);
            }
            PARAM_RELEASE => {
                self.release_time = value.clamp(0.01, 5.0);
            }
            PARAM_TRANSPOSE => {
                self.transpose = value.clamp(-24.0, 24.0) as i8;
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_GAIN => self.gain,
            PARAM_ATTACK => self.attack_time,
            PARAM_RELEASE => self.release_time,
            PARAM_TRANSPOSE => self.transpose as f32,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        _inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let frames = output.len() / 2;

        // Clear output
        output.fill(0.0);

        // Process MIDI events
        if !midi_inputs.is_empty() {
            for event in midi_inputs[0].iter() {
                if event.is_note_on() {
                    self.note_on(event.data1, event.data2);
                } else if event.is_note_off() {
                    self.note_off(event.data1);
                }
            }
        }

        // Extract parameters needed for processing
        let gain = self.gain;
        let attack_time = self.attack_time;
        let release_time = self.release_time;

        // Process all active voices
        for voice in &mut self.voices {
            if !voice.is_active {
                continue;
            }

            if voice.layer_index >= self.layers.len() {
                continue;
            }

            let layer = &self.layers[voice.layer_index];

            // Calculate playback speed
            let semitone_diff = voice.note as i16 - layer.root_key as i16;
            let speed = 2.0_f32.powf(semitone_diff as f32 / 12.0);
            let speed_adjusted = speed * (layer.sample_rate / sample_rate as f32);

            for frame in 0..frames {
                // Read sample with linear interpolation
                let playhead = voice.playhead;
                let sample = if !layer.sample_data.is_empty() && playhead >= 0.0 {
                    let index = playhead.floor() as usize;
                    if index < layer.sample_data.len() {
                        let frac = playhead - playhead.floor();
                        let sample1 = layer.sample_data[index];
                        let sample2 = if index + 1 < layer.sample_data.len() {
                            layer.sample_data[index + 1]
                        } else {
                            0.0
                        };
                        sample1 + (sample2 - sample1) * frac
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };

                // Process envelope
                match voice.envelope_phase {
                    EnvelopePhase::Attack => {
                        let attack_samples = attack_time * sample_rate as f32;
                        voice.envelope_value += 1.0 / attack_samples;
                        if voice.envelope_value >= 1.0 {
                            voice.envelope_value = 1.0;
                            voice.envelope_phase = EnvelopePhase::Sustain;
                        }
                    }
                    EnvelopePhase::Sustain => {
                        voice.envelope_value = 1.0;
                    }
                    EnvelopePhase::Release => {
                        let release_samples = release_time * sample_rate as f32;
                        voice.envelope_value -= 1.0 / release_samples;
                        if voice.envelope_value <= 0.0 {
                            voice.envelope_value = 0.0;
                            voice.is_active = false;
                        }
                    }
                }
                let envelope = voice.envelope_value.clamp(0.0, 1.0);

                // Apply velocity scaling (0-127 -> 0-1)
                let velocity_scale = voice.velocity as f32 / 127.0;

                // Mix into output
                let final_sample = sample * envelope * velocity_scale * gain;
                output[frame * 2] += final_sample;
                output[frame * 2 + 1] += final_sample;

                // Advance playhead
                voice.playhead += speed_adjusted;

                // Stop if we've reached the end
                if voice.playhead >= layer.sample_data.len() as f32 {
                    voice.is_active = false;
                    break;
                }
            }
        }
    }

    fn reset(&mut self) {
        self.voices.clear();
    }

    fn node_type(&self) -> &str {
        "MultiSampler"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self::new(self.name.clone()))
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
