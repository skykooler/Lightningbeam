use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

// Parameters
const PARAM_GAIN: u32 = 0;
const PARAM_ATTACK: u32 = 1;
const PARAM_RELEASE: u32 = 2;
const PARAM_TRANSPOSE: u32 = 3;

/// Loop playback mode
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoopMode {
    /// Play sample once, no looping
    OneShot,
    /// Loop continuously between loop_start and loop_end
    Continuous,
}

/// Metadata about a loaded sample layer (for preset serialization)
#[derive(Clone, Debug)]
pub struct LayerInfo {
    pub file_path: String,
    pub key_min: u8,
    pub key_max: u8,
    pub root_key: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub loop_start: Option<usize>,  // Loop start point in samples
    pub loop_end: Option<usize>,    // Loop end point in samples
    pub loop_mode: LoopMode,
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

    // Loop points (in samples)
    loop_start: Option<usize>,
    loop_end: Option<usize>,
    loop_mode: LoopMode,
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
        loop_start: Option<usize>,
        loop_end: Option<usize>,
        loop_mode: LoopMode,
    ) -> Self {
        Self {
            sample_data,
            sample_rate,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
            loop_start,
            loop_end,
            loop_mode,
        }
    }

    /// Check if this layer matches the given key and velocity
    fn matches(&self, key: u8, velocity: u8) -> bool {
        key >= self.key_min
            && key <= self.key_max
            && velocity >= self.velocity_min
            && velocity <= self.velocity_max
    }

    /// Auto-detect loop points using autocorrelation to find a good loop region
    /// Returns (loop_start, loop_end) in samples
    fn detect_loop_points(sample_data: &[f32], sample_rate: f32) -> Option<(usize, usize)> {
        if sample_data.len() < (sample_rate * 0.5) as usize {
            return None; // Need at least 0.5 seconds of audio
        }

        // Look for loop in the sustain region (skip attack/decay, avoid release)
        // For sustained instruments, look in the middle 50% of the sample
        let search_start = (sample_data.len() as f32 * 0.25) as usize;
        let search_end = (sample_data.len() as f32 * 0.75) as usize;

        if search_end <= search_start {
            return None;
        }

        // Find the best loop point using autocorrelation
        // For sustained instruments like brass/woodwind, we want longer loops
        let min_loop_length = (sample_rate * 0.1) as usize; // Min 0.1s loop (more stable)
        let max_loop_length = (sample_rate * 10.0) as usize; // Max 10 second loop

        let mut best_correlation = -1.0;
        let mut best_loop_start = search_start;
        let mut best_loop_end = search_end;

        // Try different loop lengths from LONGEST to SHORTEST
        // This way we prefer longer loops and stop early if we find a good one
        let length_step = ((sample_rate * 0.05) as usize).max(512); // 50ms steps
        let actual_max_length = max_loop_length.min(search_end - search_start);

        // Manually iterate backwards since step_by().rev() doesn't work on RangeInclusive<usize>
        let mut loop_length = actual_max_length;
        while loop_length >= min_loop_length {
            // Try different starting points in the sustain region (finer steps)
            let start_step = ((sample_rate * 0.02) as usize).max(256); // 20ms steps
            for start in (search_start..search_end - loop_length).step_by(start_step) {
                let end = start + loop_length;
                if end > search_end {
                    break;
                }

                // Calculate correlation between loop end and loop start
                let correlation = Self::calculate_loop_correlation(sample_data, start, end);

                if correlation > best_correlation {
                    best_correlation = correlation;
                    best_loop_start = start;
                    best_loop_end = end;
                }
            }

            // If we found a good enough loop, stop searching shorter ones
            if best_correlation > 0.8 {
                break;
            }

            // Decrement loop_length, with underflow protection
            if loop_length < length_step {
                break;
            }
            loop_length -= length_step;
        }

        // Lower threshold since longer loops are harder to match perfectly
        if best_correlation > 0.6 {
            Some((best_loop_start, best_loop_end))
        } else {
            // Fallback: use a reasonable chunk of the sustain region
            let fallback_length = ((search_end - search_start) / 2).max(min_loop_length);
            Some((search_start, search_start + fallback_length))
        }
    }

    /// Calculate how well the audio loops at the given points
    /// Returns correlation value between -1.0 and 1.0 (higher is better)
    fn calculate_loop_correlation(sample_data: &[f32], loop_start: usize, loop_end: usize) -> f32 {
        let loop_length = loop_end - loop_start;
        let window_size = (loop_length / 10).max(128).min(2048); // Compare last 10% of loop

        if loop_end + window_size >= sample_data.len() {
            return -1.0;
        }

        // Compare the end of the loop region with the beginning
        let region1_start = loop_end - window_size;
        let region2_start = loop_start;

        let mut sum_xy = 0.0;
        let mut sum_x2 = 0.0;
        let mut sum_y2 = 0.0;

        for i in 0..window_size {
            let x = sample_data[region1_start + i];
            let y = sample_data[region2_start + i];
            sum_xy += x * y;
            sum_x2 += x * x;
            sum_y2 += y * y;
        }

        // Pearson correlation coefficient
        let denominator = (sum_x2 * sum_y2).sqrt();
        if denominator > 0.0 {
            sum_xy / denominator
        } else {
            -1.0
        }
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

    // Loop crossfade state
    crossfade_buffer: Vec<f32>,  // Stores samples from before loop_start for crossfading
    crossfade_length: usize,     // Length of crossfade in samples (e.g., 100 samples = ~2ms @ 48kHz)
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
            crossfade_buffer: Vec::new(),
            crossfade_length: 1000,  // ~20ms at 48kHz (longer for smoother loops)
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
        loop_start: Option<usize>,
        loop_end: Option<usize>,
        loop_mode: LoopMode,
    ) {
        let layer = SampleLayer::new(
            sample_data,
            sample_rate,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
            loop_start,
            loop_end,
            loop_mode,
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
        loop_start: Option<usize>,
        loop_end: Option<usize>,
        loop_mode: LoopMode,
    ) -> Result<(), String> {
        use crate::audio::sample_loader::load_audio_file;

        let sample_data = load_audio_file(path)?;

        // Auto-detect loop points if not provided and mode is Continuous
        let (final_loop_start, final_loop_end) = if loop_mode == LoopMode::Continuous && loop_start.is_none() && loop_end.is_none() {
            if let Some((start, end)) = SampleLayer::detect_loop_points(&sample_data.samples, sample_data.sample_rate as f32) {
                (Some(start), Some(end))
            } else {
                (None, None)
            }
        } else {
            (loop_start, loop_end)
        };

        self.add_layer(
            sample_data.samples,
            sample_data.sample_rate as f32,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
            final_loop_start,
            final_loop_end,
            loop_mode,
        );

        // Store layer metadata for preset serialization
        self.layer_infos.push(LayerInfo {
            file_path: path.to_string(),
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
            loop_start: final_loop_start,
            loop_end: final_loop_end,
            loop_mode,
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
        loop_start: Option<usize>,
        loop_end: Option<usize>,
        loop_mode: LoopMode,
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
        self.layers[layer_index].loop_start = loop_start;
        self.layers[layer_index].loop_end = loop_end;
        self.layers[layer_index].loop_mode = loop_mode;

        // Update the layer info
        if layer_index < self.layer_infos.len() {
            self.layer_infos[layer_index].key_min = key_min;
            self.layer_infos[layer_index].key_max = key_max;
            self.layer_infos[layer_index].root_key = root_key;
            self.layer_infos[layer_index].velocity_min = velocity_min;
            self.layer_infos[layer_index].velocity_max = velocity_max;
            self.layer_infos[layer_index].loop_start = loop_start;
            self.layer_infos[layer_index].loop_end = loop_end;
            self.layer_infos[layer_index].loop_mode = loop_mode;
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
                // Read sample with linear interpolation and loop handling
                let playhead = voice.playhead;
                let mut sample = 0.0;

                if !layer.sample_data.is_empty() && playhead >= 0.0 {
                    let index = playhead.floor() as usize;

                    // Check if we need to handle looping
                    if layer.loop_mode == LoopMode::Continuous {
                        if let (Some(loop_start), Some(loop_end)) = (layer.loop_start, layer.loop_end) {
                            // Validate loop points
                            if loop_start < loop_end && loop_end <= layer.sample_data.len() {
                                // Fill crossfade buffer on first loop with samples just before loop_start
                                // These will be crossfaded with the beginning of the loop for seamless looping
                                if voice.crossfade_buffer.is_empty() && loop_start >= voice.crossfade_length {
                                    let crossfade_start = loop_start.saturating_sub(voice.crossfade_length);
                                    voice.crossfade_buffer = layer.sample_data[crossfade_start..loop_start].to_vec();
                                }

                                // Check if we've reached the loop end
                                if index >= loop_end {
                                    // Wrap around to loop start
                                    let loop_length = loop_end - loop_start;
                                    let offset_from_end = index - loop_end;
                                    let wrapped_index = loop_start + (offset_from_end % loop_length);
                                    voice.playhead = wrapped_index as f32 + (playhead - playhead.floor());
                                }

                                // Read sample at current position
                                let current_index = voice.playhead.floor() as usize;
                                if current_index < layer.sample_data.len() {
                                    let frac = voice.playhead - voice.playhead.floor();
                                    let sample1 = layer.sample_data[current_index];
                                    let sample2 = if current_index + 1 < layer.sample_data.len() {
                                        layer.sample_data[current_index + 1]
                                    } else {
                                        layer.sample_data[loop_start]  // Wrap to loop start for interpolation
                                    };
                                    sample = sample1 + (sample2 - sample1) * frac;

                                    // Apply crossfade only at the END of loop
                                    // Crossfade the end of loop with samples BEFORE loop_start
                                    if current_index >= loop_start && current_index < loop_end {
                                        if !voice.crossfade_buffer.is_empty() {
                                            let crossfade_len = voice.crossfade_length.min(voice.crossfade_buffer.len());

                                            // Only crossfade at loop end (last crossfade_length samples)
                                            // This blends end samples (i,j,k) with pre-loop samples (a,b,c)
                                            if current_index >= loop_end - crossfade_len && current_index < loop_end {
                                                let crossfade_pos = current_index - (loop_end - crossfade_len);
                                                if crossfade_pos < voice.crossfade_buffer.len() {
                                                    let end_sample = sample; // Current sample at end of loop (i, j, or k)
                                                    let pre_loop_sample = voice.crossfade_buffer[crossfade_pos]; // Corresponding pre-loop sample (a, b, or c)
                                                    // Equal-power crossfade: fade out end, fade in pre-loop
                                                    let fade_ratio = crossfade_pos as f32 / crossfade_len as f32;
                                                    let fade_out = (1.0 - fade_ratio).sqrt();
                                                    let fade_in = fade_ratio.sqrt();
                                                    sample = end_sample * fade_out + pre_loop_sample * fade_in;
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Invalid loop points, play normally
                                if index < layer.sample_data.len() {
                                    let frac = playhead - playhead.floor();
                                    let sample1 = layer.sample_data[index];
                                    let sample2 = if index + 1 < layer.sample_data.len() {
                                        layer.sample_data[index + 1]
                                    } else {
                                        0.0
                                    };
                                    sample = sample1 + (sample2 - sample1) * frac;
                                }
                            }
                        } else {
                            // No loop points defined, play normally
                            if index < layer.sample_data.len() {
                                let frac = playhead - playhead.floor();
                                let sample1 = layer.sample_data[index];
                                let sample2 = if index + 1 < layer.sample_data.len() {
                                    layer.sample_data[index + 1]
                                } else {
                                    0.0
                                };
                                sample = sample1 + (sample2 - sample1) * frac;
                            }
                        }
                    } else {
                        // OneShot mode - play normally without looping
                        if index < layer.sample_data.len() {
                            let frac = playhead - playhead.floor();
                            let sample1 = layer.sample_data[index];
                            let sample2 = if index + 1 < layer.sample_data.len() {
                                layer.sample_data[index + 1]
                            } else {
                                0.0
                            };
                            sample = sample1 + (sample2 - sample1) * frac;
                        }
                    }
                }

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

                // Stop if we've reached the end (only for OneShot mode)
                if layer.loop_mode == LoopMode::OneShot {
                    if voice.playhead >= layer.sample_data.len() as f32 {
                        voice.is_active = false;
                        break;
                    }
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
