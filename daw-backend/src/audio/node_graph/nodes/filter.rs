use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use crate::dsp::biquad::BiquadFilter;

const PARAM_CUTOFF: u32 = 0;
const PARAM_RESONANCE: u32 = 1;
const PARAM_TYPE: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    Lowpass = 0,
    Highpass = 1,
}

impl FilterType {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => FilterType::Highpass,
            _ => FilterType::Lowpass,
        }
    }
}

/// Filter node using biquad implementation
pub struct FilterNode {
    name: String,
    filter: BiquadFilter,
    cutoff: f32,
    resonance: f32,
    filter_type: FilterType,
    sample_rate: u32,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl FilterNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("Cutoff CV", SignalType::CV, 1),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_CUTOFF, "Cutoff", 20.0, 20000.0, 1000.0, ParameterUnit::Frequency),
            Parameter::new(PARAM_RESONANCE, "Resonance", 0.1, 10.0, 0.707, ParameterUnit::Generic),
            Parameter::new(PARAM_TYPE, "Type", 0.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        let filter = BiquadFilter::lowpass(1000.0, 0.707, 44100.0);

        Self {
            name,
            filter,
            cutoff: 1000.0,
            resonance: 0.707,
            filter_type: FilterType::Lowpass,
            sample_rate: 44100,
            inputs,
            outputs,
            parameters,
        }
    }

    fn update_filter(&mut self) {
        match self.filter_type {
            FilterType::Lowpass => {
                self.filter.set_lowpass(self.cutoff, self.resonance, self.sample_rate as f32);
            }
            FilterType::Highpass => {
                self.filter.set_highpass(self.cutoff, self.resonance, self.sample_rate as f32);
            }
        }
    }
}

impl AudioNode for FilterNode {
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
                self.update_filter();
            }
            PARAM_RESONANCE => {
                self.resonance = value.clamp(0.1, 10.0);
                self.update_filter();
            }
            PARAM_TYPE => {
                self.filter_type = FilterType::from_f32(value);
                self.update_filter();
            }
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_CUTOFF => self.cutoff,
            PARAM_RESONANCE => self.resonance,
            PARAM_TYPE => self.filter_type as i32 as f32,
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
        if inputs.is_empty() || outputs.is_empty() {
            return;
        }

        // Update sample rate if changed
        if self.sample_rate != sample_rate {
            self.sample_rate = sample_rate;
            self.update_filter();
        }

        let input = inputs[0];
        let output = &mut outputs[0];
        let len = input.len().min(output.len());

        // Copy input to output
        output[..len].copy_from_slice(&input[..len]);

        // Check for CV modulation (modulates cutoff)
        if inputs.len() > 1 && !inputs[1].is_empty() {
            // CV input modulates cutoff frequency
            // For now, just use the base cutoff - per-sample modulation would be expensive
            // TODO: Sample CV at frame rate and update filter periodically
        }

        // Apply filter (processes stereo interleaved)
        self.filter.process_buffer(&mut output[..len], 2);
    }

    fn reset(&mut self) {
        self.filter.reset();
    }

    fn node_type(&self) -> &str {
        "Filter"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        // Create new filter with same parameters but reset state
        let mut new_filter = BiquadFilter::new();

        // Set filter to match current type
        match self.filter_type {
            FilterType::Lowpass => {
                new_filter.set_lowpass(self.sample_rate as f32, self.cutoff, self.resonance);
            }
            FilterType::Highpass => {
                new_filter.set_highpass(self.sample_rate as f32, self.cutoff, self.resonance);
            }
        }

        Box::new(Self {
            name: self.name.clone(),
            filter: new_filter,
            cutoff: self.cutoff,
            resonance: self.resonance,
            filter_type: self.filter_type,
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
