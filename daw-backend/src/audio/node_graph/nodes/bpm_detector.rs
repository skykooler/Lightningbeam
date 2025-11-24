use crate::audio::bpm_detector::BpmDetectorRealtime;
use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;

const PARAM_SMOOTHING: u32 = 0;

/// BPM Detector Node - analyzes audio input and outputs tempo as CV
/// CV output represents BPM (e.g., 0.12 = 120 BPM when scaled appropriately)
pub struct BpmDetectorNode {
    name: String,
    detector: BpmDetectorRealtime,
    smoothing: f32,  // Smoothing factor for output (0-1)
    last_output: f32, // For smooth transitions
    sample_rate: u32, // Current sample rate
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl BpmDetectorNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];

        let outputs = vec![
            NodePort::new("BPM CV", SignalType::CV, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_SMOOTHING, "Smoothing", 0.0, 1.0, 0.9, ParameterUnit::Percent),
        ];

        // Use 10 second buffer for analysis
        let detector = BpmDetectorRealtime::new(48000, 10.0);

        Self {
            name,
            detector,
            smoothing: 0.9,
            last_output: 120.0,
            sample_rate: 48000,
            inputs,
            outputs,
            parameters,
        }
    }
}

impl AudioNode for BpmDetectorNode {
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
        match id {
            PARAM_SMOOTHING => self.smoothing = value.clamp(0.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_SMOOTHING => self.smoothing,
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
        // Recreate detector if sample rate changed
        if sample_rate != self.sample_rate {
            self.sample_rate = sample_rate;
            self.detector = BpmDetectorRealtime::new(sample_rate, 10.0);
        }

        if outputs.is_empty() {
            return;
        }

        let output = &mut outputs[0];
        let length = output.len();

        let input = if !inputs.is_empty() && !inputs[0].is_empty() {
            inputs[0]
        } else {
            // Fill output with last known BPM
            for i in 0..length {
                output[i] = self.last_output / 1000.0; // Scale BPM for CV (e.g., 120 BPM -> 0.12)
            }
            return;
        };

        // Process audio through detector
        let detected_bpm = self.detector.process(input);

        // Apply smoothing
        let target_bpm = detected_bpm;
        let smoothed_bpm = self.last_output * self.smoothing + target_bpm * (1.0 - self.smoothing);
        self.last_output = smoothed_bpm;

        // Output BPM as CV (scaled down for typical CV range)
        // BPM / 1000 gives us reasonable CV values (60-180 BPM -> 0.06-0.18)
        let cv_value = smoothed_bpm / 1000.0;

        // Fill entire output buffer with current BPM value
        for i in 0..length {
            output[i] = cv_value;
        }
    }

    fn reset(&mut self) {
        self.detector.reset();
        self.last_output = 120.0;
    }

    fn node_type(&self) -> &str {
        "BpmDetector"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            detector: BpmDetectorRealtime::new(self.sample_rate, 10.0),
            smoothing: self.smoothing,
            last_output: self.last_output,
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
