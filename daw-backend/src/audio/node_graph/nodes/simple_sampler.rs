use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType, cv_input_or_default};
use crate::audio::midi::MidiEvent;
use std::sync::{Arc, Mutex};

// Parameters
const PARAM_GAIN: u32 = 0;
const PARAM_LOOP: u32 = 1;
const PARAM_PITCH_SHIFT: u32 = 2;

/// Simple single-sample playback node with pitch shifting
pub struct SimpleSamplerNode {
    name: String,

    // Sample data (shared, can be set externally)
    sample_data: Arc<Mutex<Vec<f32>>>,
    sample_rate_original: f32,
    sample_path: Option<String>,  // Path to loaded sample file

    // Playback state
    playhead: f32,          // Fractional position in sample
    is_playing: bool,
    gate_prev: bool,

    // Parameters
    gain: f32,
    loop_enabled: bool,
    pitch_shift: f32,       // Additional pitch shift in semitones
    root_note: u8,          // MIDI note for original pitch playback (default 69 = A4)

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl SimpleSamplerNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("V/Oct", SignalType::CV, 0),
            NodePort::new("Gate", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_GAIN, "Gain", 0.0, 2.0, 1.0, ParameterUnit::Generic),
            Parameter::new(PARAM_LOOP, "Loop", 0.0, 1.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_PITCH_SHIFT, "Pitch Shift", -12.0, 12.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            sample_data: Arc::new(Mutex::new(Vec::new())),
            sample_rate_original: 48000.0,
            sample_path: None,
            playhead: 0.0,
            is_playing: false,
            gate_prev: false,
            gain: 1.0,
            loop_enabled: false,
            pitch_shift: 0.0,
            root_note: 69, // A4 — V/Oct 0.0 from MIDI-to-CV
            inputs,
            outputs,
            parameters,
        }
    }

    /// Set the sample data (mono)
    pub fn set_sample(&mut self, data: Vec<f32>, sample_rate: f32) {
        let mut sample = self.sample_data.lock().unwrap();
        *sample = data;
        self.sample_rate_original = sample_rate;
    }

    /// Get the sample data reference (for external loading)
    pub fn get_sample_data(&self) -> Arc<Mutex<Vec<f32>>> {
        Arc::clone(&self.sample_data)
    }

    /// Load a sample from a file path
    pub fn load_sample_from_file(&mut self, path: &str) -> Result<(), String> {
        use crate::audio::sample_loader::load_audio_file;

        let sample_data = load_audio_file(path)?;
        self.set_sample(sample_data.samples, sample_data.sample_rate as f32);
        self.sample_path = Some(path.to_string());
        Ok(())
    }

    /// Get the currently loaded sample path
    pub fn get_sample_path(&self) -> Option<&str> {
        self.sample_path.as_deref()
    }

    /// Get the current sample data and sample rate (for preset embedding)
    pub fn get_sample_data_for_embedding(&self) -> (Vec<f32>, f32) {
        let sample = self.sample_data.lock().unwrap();
        (sample.clone(), self.sample_rate_original)
    }

    /// Convert V/oct CV to playback speed multiplier
    /// Accounts for root_note: when the incoming MIDI note matches root_note,
    /// the sample plays at original speed. V/Oct 0.0 = A4 (MIDI 69) by convention.
    fn voct_to_speed(&self, voct: f32) -> f32 {
        // Offset so root_note plays at original speed
        let root_offset = (self.root_note as f32 - 69.0) / 12.0;
        let total_semitones = (voct - root_offset) * 12.0 + self.pitch_shift;
        2.0_f32.powf(total_semitones / 12.0)
    }

    /// Set the root note (MIDI note number for original-pitch playback)
    pub fn set_root_note(&mut self, note: u8) {
        self.root_note = note.min(127);
    }

    /// Get the current root note
    pub fn root_note(&self) -> u8 {
        self.root_note
    }

    /// Read sample at playhead with linear interpolation
    fn read_sample(&self, playhead: f32, sample: &[f32]) -> f32 {
        if sample.is_empty() {
            return 0.0;
        }

        let index = playhead.floor() as usize;
        let frac = playhead - playhead.floor();

        if index >= sample.len() {
            return 0.0;
        }

        let sample1 = sample[index];
        let sample2 = if index + 1 < sample.len() {
            sample[index + 1]
        } else if self.loop_enabled {
            sample[0] // Loop back to start
        } else {
            0.0
        };

        // Linear interpolation
        sample1 + (sample2 - sample1) * frac
    }
}

impl AudioNode for SimpleSamplerNode {
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
            PARAM_LOOP => {
                self.loop_enabled = value > 0.5;
            }
            PARAM_PITCH_SHIFT => {
                self.pitch_shift = value.clamp(-12.0, 12.0);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_GAIN => self.gain,
            PARAM_LOOP => if self.loop_enabled { 1.0 } else { 0.0 },
            PARAM_PITCH_SHIFT => self.pitch_shift,
            _ => 0.0,
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _midi_inputs: &[&[MidiEvent]],
        _midi_outputs: &mut [&mut Vec<MidiEvent>],
        sample_rate: u32,
    ) {
        if outputs.is_empty() {
            return;
        }

        // Lock the sample data
        let sample_data = self.sample_data.lock().unwrap();
        if sample_data.is_empty() {
            // No sample loaded, output silence
            for output in outputs.iter_mut() {
                output.fill(0.0);
            }
            return;
        }

        let output = &mut outputs[0];
        let frames = output.len() / 2;

        for frame in 0..frames {
            // Read CV inputs (both are mono signals)
            // V/Oct: when unconnected, defaults to 0.0 (original pitch)
            let voct = cv_input_or_default(inputs, 0, frame, 0.0);
            // Gate: when unconnected, defaults to 0.0 (off)
            let gate = cv_input_or_default(inputs, 1, frame, 0.0);

            // Detect gate trigger (rising edge)
            let gate_active = gate > 0.5;
            if gate_active && !self.gate_prev {
                // Trigger: start playback from beginning
                self.playhead = 0.0;
                self.is_playing = true;
            }
            self.gate_prev = gate_active;

            // Generate sample
            let sample = if self.is_playing {
                let s = self.read_sample(self.playhead, &sample_data);

                // Calculate playback speed from V/Oct
                let speed = self.voct_to_speed(voct);

                // Advance playhead with resampling
                let speed_adjusted = speed * (self.sample_rate_original / sample_rate as f32);
                self.playhead += speed_adjusted;

                // Check if we've reached the end
                if self.playhead >= sample_data.len() as f32 {
                    if self.loop_enabled {
                        // Loop back to start
                        self.playhead = self.playhead % sample_data.len() as f32;
                    } else {
                        // Stop playback
                        self.is_playing = false;
                        self.playhead = 0.0;
                    }
                }

                s * self.gain
            } else {
                0.0
            };

            // Output stereo (same signal to both channels)
            output[frame * 2] = sample;
            output[frame * 2 + 1] = sample;
        }
    }

    fn reset(&mut self) {
        self.playhead = 0.0;
        self.is_playing = false;
        self.gate_prev = false;
    }

    fn node_type(&self) -> &str {
        "SimpleSampler"
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
