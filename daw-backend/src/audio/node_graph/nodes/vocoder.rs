use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

const PARAM_BANDS: u32 = 0;
const PARAM_ATTACK: u32 = 1;
const PARAM_RELEASE: u32 = 2;
const PARAM_MIX: u32 = 3;

const MAX_BANDS: usize = 32;

/// Simple bandpass filter using biquad
struct BandpassFilter {
    // Biquad coefficients
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,

    // State variables (separate for modulator and carrier, L/R channels)
    mod_z1_left: f32,
    mod_z2_left: f32,
    mod_z1_right: f32,
    mod_z2_right: f32,
    car_z1_left: f32,
    car_z2_left: f32,
    car_z1_right: f32,
    car_z2_right: f32,
}

impl BandpassFilter {
    fn new() -> Self {
        Self {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            mod_z1_left: 0.0,
            mod_z2_left: 0.0,
            mod_z1_right: 0.0,
            mod_z2_right: 0.0,
            car_z1_left: 0.0,
            car_z2_left: 0.0,
            car_z1_right: 0.0,
            car_z2_right: 0.0,
        }
    }

    fn set_bandpass(&mut self, frequency: f32, q: f32, sample_rate: f32) {
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);

        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.b1 = 0.0;
        self.b2 = -alpha / a0;
        self.a1 = -2.0 * cos_omega / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process_modulator(&mut self, input: f32, is_left: bool) -> f32 {
        let (z1, z2) = if is_left {
            (&mut self.mod_z1_left, &mut self.mod_z2_left)
        } else {
            (&mut self.mod_z1_right, &mut self.mod_z2_right)
        };

        let output = self.b0 * input + self.b1 * *z1 + self.b2 * *z2 - self.a1 * *z1 - self.a2 * *z2;
        *z2 = *z1;
        *z1 = output;
        output
    }

    fn process_carrier(&mut self, input: f32, is_left: bool) -> f32 {
        let (z1, z2) = if is_left {
            (&mut self.car_z1_left, &mut self.car_z2_left)
        } else {
            (&mut self.car_z1_right, &mut self.car_z2_right)
        };

        let output = self.b0 * input + self.b1 * *z1 + self.b2 * *z2 - self.a1 * *z1 - self.a2 * *z2;
        *z2 = *z1;
        *z1 = output;
        output
    }

    fn reset(&mut self) {
        self.mod_z1_left = 0.0;
        self.mod_z2_left = 0.0;
        self.mod_z1_right = 0.0;
        self.mod_z2_right = 0.0;
        self.car_z1_left = 0.0;
        self.car_z2_left = 0.0;
        self.car_z1_right = 0.0;
        self.car_z2_right = 0.0;
    }
}

/// Vocoder band with filter and envelope follower
struct VocoderBand {
    filter: BandpassFilter,
    envelope_left: f32,
    envelope_right: f32,
}

impl VocoderBand {
    fn new() -> Self {
        Self {
            filter: BandpassFilter::new(),
            envelope_left: 0.0,
            envelope_right: 0.0,
        }
    }

    fn reset(&mut self) {
        self.filter.reset();
        self.envelope_left = 0.0;
        self.envelope_right = 0.0;
    }
}

/// Vocoder effect - imposes spectral envelope of modulator onto carrier
pub struct VocoderNode {
    name: String,
    num_bands: usize,     // 8 to 32 bands
    attack_time: f32,     // 0.001 to 0.1 seconds
    release_time: f32,    // 0.001 to 1.0 seconds
    mix: f32,             // 0.0 to 1.0

    bands: Vec<VocoderBand>,

    sample_rate: u32,

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl VocoderNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Modulator", SignalType::Audio, 0),
            NodePort::new("Carrier", SignalType::Audio, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_BANDS, "Bands", 8.0, 32.0, 16.0, ParameterUnit::Generic),
            Parameter::new(PARAM_ATTACK, "Attack", 0.001, 0.1, 0.01, ParameterUnit::Time),
            Parameter::new(PARAM_RELEASE, "Release", 0.001, 1.0, 0.05, ParameterUnit::Time),
            Parameter::new(PARAM_MIX, "Mix", 0.0, 1.0, 1.0, ParameterUnit::Generic),
        ];

        let mut bands = Vec::with_capacity(MAX_BANDS);
        for _ in 0..MAX_BANDS {
            bands.push(VocoderBand::new());
        }

        Self {
            name,
            num_bands: 16,
            attack_time: 0.01,
            release_time: 0.05,
            mix: 1.0,
            bands,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }

    fn setup_bands(&mut self) {
        // Distribute bands logarithmically from 200 Hz to 5000 Hz
        let min_freq: f32 = 200.0;
        let max_freq: f32 = 5000.0;
        let q: f32 = 4.0; // Fairly narrow bands

        for i in 0..self.num_bands {
            let t = i as f32 / (self.num_bands - 1) as f32;
            let freq = min_freq * (max_freq / min_freq).powf(t);
            self.bands[i].filter.set_bandpass(freq, q, self.sample_rate as f32);
        }
    }
}

impl AudioNode for VocoderNode {
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
            PARAM_BANDS => {
                let bands = (value.round() as usize).clamp(8, 32);
                if bands != self.num_bands {
                    self.num_bands = bands;
                    self.setup_bands();
                }
            }
            PARAM_ATTACK => self.attack_time = value.clamp(0.001, 0.1),
            PARAM_RELEASE => self.release_time = value.clamp(0.001, 1.0),
            PARAM_MIX => self.mix = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_BANDS => self.num_bands as f32,
            PARAM_ATTACK => self.attack_time,
            PARAM_RELEASE => self.release_time,
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
        sample_rate: u32,
    ) {
        if inputs.len() < 2 || outputs.is_empty() {
            return;
        }

        // Update sample rate if changed
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.setup_bands();
        }

        let modulator = inputs[0];
        let carrier = inputs[1];
        let output = &mut outputs[0];

        // Audio signals are stereo (interleaved L/R)
        let mod_frames = modulator.len() / 2;
        let car_frames = carrier.len() / 2;
        let out_frames = output.len() / 2;
        let frames_to_process = mod_frames.min(car_frames).min(out_frames);

        // Calculate envelope follower coefficients
        let sample_duration = 1.0 / self.sample_rate as f32;
        let attack_coeff = (sample_duration / self.attack_time).min(1.0);
        let release_coeff = (sample_duration / self.release_time).min(1.0);

        for frame in 0..frames_to_process {
            let mod_left = modulator[frame * 2];
            let mod_right = modulator[frame * 2 + 1];
            let car_left = carrier[frame * 2];
            let car_right = carrier[frame * 2 + 1];

            let mut out_left = 0.0;
            let mut out_right = 0.0;

            // Process each band
            for i in 0..self.num_bands {
                let band = &mut self.bands[i];

                // Filter modulator and carrier through bandpass
                let mod_band_left = band.filter.process_modulator(mod_left, true);
                let mod_band_right = band.filter.process_modulator(mod_right, false);
                let car_band_left = band.filter.process_carrier(car_left, true);
                let car_band_right = band.filter.process_carrier(car_right, false);

                // Extract envelope from modulator band (rectify + smooth)
                let mod_level_left = mod_band_left.abs();
                let mod_level_right = mod_band_right.abs();

                // Envelope follower
                let coeff_left = if mod_level_left > band.envelope_left {
                    attack_coeff
                } else {
                    release_coeff
                };
                let coeff_right = if mod_level_right > band.envelope_right {
                    attack_coeff
                } else {
                    release_coeff
                };

                band.envelope_left += (mod_level_left - band.envelope_left) * coeff_left;
                band.envelope_right += (mod_level_right - band.envelope_right) * coeff_right;

                // Apply envelope to carrier band
                out_left += car_band_left * band.envelope_left;
                out_right += car_band_right * band.envelope_right;
            }

            // Normalize output (roughly compensate for band summing)
            let norm_factor = 1.0 / (self.num_bands as f32).sqrt();
            out_left *= norm_factor;
            out_right *= norm_factor;

            // Mix with carrier (dry signal)
            output[frame * 2] = car_left * (1.0 - self.mix) + out_left * self.mix;
            output[frame * 2 + 1] = car_right * (1.0 - self.mix) + out_right * self.mix;
        }
    }

    fn reset(&mut self) {
        for band in &mut self.bands {
            band.reset();
        }
    }

    fn node_type(&self) -> &str {
        "Vocoder"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        let mut bands = Vec::with_capacity(MAX_BANDS);
        for _ in 0..MAX_BANDS {
            bands.push(VocoderBand::new());
        }

        let mut node = Self {
            name: self.name.clone(),
            num_bands: self.num_bands,
            attack_time: self.attack_time,
            release_time: self.release_time,
            mix: self.mix,
            bands,
            sample_rate: self.sample_rate,
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        };

        node.setup_bands();
        Box::new(node)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
