use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType, cv_input_or_default};
use crate::audio::midi::MidiEvent;
use crate::dsp::svf::SvfFilter;

const PARAM_CUTOFF: u32 = 0;
const PARAM_RESONANCE: u32 = 1;

/// State Variable Filter node — simultaneously outputs lowpass, highpass,
/// bandpass, and notch from one filter, with per-sample CV modulation of
/// cutoff and resonance.
pub struct SVFNode {
    name: String,
    filter: SvfFilter,
    cutoff: f32,
    resonance: f32,
    sample_rate: u32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl SVFNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("Cutoff CV", SignalType::CV, 1),
            NodePort::new("Resonance CV", SignalType::CV, 2),
        ];

        let outputs = vec![
            NodePort::new("Lowpass", SignalType::Audio, 0),
            NodePort::new("Highpass", SignalType::Audio, 1),
            NodePort::new("Bandpass", SignalType::Audio, 2),
            NodePort::new("Notch", SignalType::Audio, 3),
        ];

        let parameters = vec![
            Parameter::new(PARAM_CUTOFF, "Cutoff", 20.0, 20000.0, 1000.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_RESONANCE, "Resonance", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        let mut filter = SvfFilter::new();
        filter.set_params(1000.0, 0.0, 44100.0);

        Self {
            name,
            filter,
            cutoff: 1000.0,
            resonance: 0.0,
            sample_rate: 44100,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for SVFNode {
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
            PARAM_CUTOFF => {
                self.cutoff = value.clamp(20.0, 20000.0);
                self.filter.set_params(self.cutoff, self.resonance, self.sample_rate as f32);
            }
            PARAM_RESONANCE => {
                self.resonance = value.clamp(0.0, 1.0);
                self.filter.set_params(self.cutoff, self.resonance, self.sample_rate as f32);
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_CUTOFF => self.cutoff,
            PARAM_RESONANCE => self.resonance,
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
        if inputs.is_empty() || outputs.len() < 4 {
            return;
        }

        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.filter.set_params(self.cutoff, self.resonance, sample_rate as f32);
        }

        let input = inputs[0];
        // All 4 outputs are stereo interleaved
        let frames = input.len() / 2;
        let sr = self.sample_rate as f32;

        // Check if CV inputs are connected (sample first frame to detect NaN)
        let has_cutoff_cv = !cv_input_or_default(inputs, 1, 0, f32::NAN).is_nan();
        let has_resonance_cv = !cv_input_or_default(inputs, 2, 0, f32::NAN).is_nan();

        let mut last_cutoff = self.cutoff;
        let mut last_resonance = self.resonance;

        for frame in 0..frames {
            // Update coefficients from CV if connected
            if has_cutoff_cv || has_resonance_cv {
                let cutoff = if has_cutoff_cv {
                    let cv = cv_input_or_default(inputs, 1, frame, 0.5);
                    let octave_shift = (cv.clamp(0.0, 1.0) - 0.5) * 4.0;
                    (self.cutoff * 2.0_f32.powf(octave_shift)).clamp(20.0, 20000.0)
                } else {
                    self.cutoff
                };

                let resonance = if has_resonance_cv {
                    cv_input_or_default(inputs, 2, frame, self.resonance).clamp(0.0, 1.0)
                } else {
                    self.resonance
                };

                if cutoff != last_cutoff || resonance != last_resonance {
                    self.filter.set_params(cutoff, resonance, sr);
                    last_cutoff = cutoff;
                    last_resonance = resonance;
                }
            }

            // Process both channels, writing all 4 outputs
            for ch in 0..2 {
                let idx = frame * 2 + ch;
                let (lp, hp, bp, notch) = self.filter.process_sample_quad(input[idx], ch);
                outputs[0][idx] = lp;
                outputs[1][idx] = hp;
                outputs[2][idx] = bp;
                outputs[3][idx] = notch;
            }
        }
    }

    fn reset(&mut self) {
        self.filter.reset();
    }

    fn node_type(&self) -> &str {
        "SVF"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        let mut filter = SvfFilter::new();
        filter.set_params(self.cutoff, self.resonance, self.sample_rate as f32);

        Box::new(Self {
            name: self.name.clone(),
            filter,
            cutoff: self.cutoff,
            resonance: self.resonance,
            sample_rate: self.sample_rate,
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
