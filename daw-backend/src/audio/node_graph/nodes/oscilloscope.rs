use crate::audio::midi::MidiEvent;
use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use std::sync::{Arc, Mutex};

const PARAM_TIME_SCALE: u32 = 0;
const PARAM_TRIGGER_MODE: u32 = 1;
const PARAM_TRIGGER_LEVEL: u32 = 2;

const BUFFER_SIZE: usize = 96000; // 2 seconds at 48kHz (stereo)

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TriggerMode {
    FreeRunning = 0,
    RisingEdge = 1,
    FallingEdge = 2,
    VoltPerOctave = 3,
}

impl TriggerMode {
    fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => TriggerMode::RisingEdge,
            2 => TriggerMode::FallingEdge,
            3 => TriggerMode::VoltPerOctave,
            _ => TriggerMode::FreeRunning,
        }
    }
}

/// Circular buffer for storing audio samples
pub struct CircularBuffer {
    buffer: Vec<f32>,
    write_pos: usize,
    capacity: usize,
}

impl CircularBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            write_pos: 0,
            capacity,
        }
    }

    fn write(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.capacity;
        }
    }

    fn read(&self, count: usize) -> Vec<f32> {
        let count = count.min(self.capacity);
        let mut result = Vec::with_capacity(count);

        // Read backwards from current write position
        let start_pos = if self.write_pos >= count {
            self.write_pos - count
        } else {
            self.capacity - (count - self.write_pos)
        };

        for i in 0..count {
            let pos = (start_pos + i) % self.capacity;
            result.push(self.buffer[pos]);
        }

        result
    }

    fn clear(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}

/// Oscilloscope node for visualizing audio and CV signals
pub struct OscilloscopeNode {
    name: String,
    time_scale: f32,      // Milliseconds to display (10-1000ms)
    trigger_mode: TriggerMode,
    trigger_level: f32,   // -1.0 to 1.0
    last_sample: f32,     // For edge detection
    voct_value: f32,      // Current V/oct input value
    sample_counter: usize, // Counter for V/oct triggering
    trigger_period: usize, // Period in samples for V/oct triggering

    // Shared buffers for reading from Tauri commands
    buffer: Arc<Mutex<CircularBuffer>>,       // Audio buffer
    cv_buffer: Arc<Mutex<CircularBuffer>>,    // CV buffer

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl OscilloscopeNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("V/oct", SignalType::CV, 1),
            NodePort::new("CV In", SignalType::CV, 2),
        ];

        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        let parameters = vec![
            Parameter::new(PARAM_TIME_SCALE, "Time Scale", 10.0, 1000.0, 100.0, ParameterUnit::Time),
            Parameter::new(PARAM_TRIGGER_MODE, "Trigger", 0.0, 3.0, 0.0, ParameterUnit::Generic),
            Parameter::new(PARAM_TRIGGER_LEVEL, "Trigger Level", -1.0, 1.0, 0.0, ParameterUnit::Generic),
        ];

        Self {
            name,
            time_scale: 100.0,
            trigger_mode: TriggerMode::FreeRunning,
            trigger_level: 0.0,
            last_sample: 0.0,
            voct_value: 0.0,
            sample_counter: 0,
            trigger_period: 480, // Default to ~100Hz at 48kHz
            buffer: Arc::new(Mutex::new(CircularBuffer::new(BUFFER_SIZE))),
            cv_buffer: Arc::new(Mutex::new(CircularBuffer::new(BUFFER_SIZE))),
            inputs,
            outputs,
            parameters,
        }
    }

    /// Get a clone of the buffer Arc for reading from external code (Tauri commands)
    pub fn get_buffer(&self) -> Arc<Mutex<CircularBuffer>> {
        Arc::clone(&self.buffer)
    }

    /// Read samples from the buffer (for Tauri commands)
    pub fn read_samples(&self, count: usize) -> Vec<f32> {
        if let Ok(buffer) = self.buffer.lock() {
            buffer.read(count)
        } else {
            vec![0.0; count]
        }
    }

    /// Read CV samples from the CV buffer (for Tauri commands)
    pub fn read_cv_samples(&self, count: usize) -> Vec<f32> {
        if let Ok(buffer) = self.cv_buffer.lock() {
            buffer.read(count)
        } else {
            vec![0.0; count]
        }
    }

    /// Clear the buffer
    pub fn clear_buffer(&self) {
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.clear();
        }
        if let Ok(mut cv_buffer) = self.cv_buffer.lock() {
            cv_buffer.clear();
        }
    }

    /// Convert V/oct to frequency in Hz (matches oscillator convention)
    /// 0V = A4 (440 Hz), ±1V per octave
    fn voct_to_frequency(voct: f32) -> f32 {
        440.0 * 2.0_f32.powf(voct)
    }
}

impl AudioNode for OscilloscopeNode {
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
            PARAM_TIME_SCALE => self.time_scale = value.clamp(10.0, 1000.0),
            PARAM_TRIGGER_MODE => self.trigger_mode = TriggerMode::from_f32(value),
            PARAM_TRIGGER_LEVEL => self.trigger_level = value.clamp(-1.0, 1.0),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            PARAM_TIME_SCALE => self.time_scale,
            PARAM_TRIGGER_MODE => self.trigger_mode as i32 as f32,
            PARAM_TRIGGER_LEVEL => self.trigger_level,
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

        let input = inputs[0];
        let output = &mut outputs[0];
        let len = input.len().min(output.len());

        // Read V/oct input if available and update trigger period
        if inputs.len() > 1 && !inputs[1].is_empty() {
            self.voct_value = inputs[1][0]; // Use first sample of V/oct input
            let frequency = Self::voct_to_frequency(self.voct_value);
            // Calculate period in samples, clamped to reasonable range
            let period_samples = (sample_rate as f32 / frequency).max(1.0);
            self.trigger_period = period_samples as usize;
        }

        // Update sample counter for V/oct triggering
        if self.trigger_mode == TriggerMode::VoltPerOctave {
            self.sample_counter = (self.sample_counter + len) % self.trigger_period;
        }

        // Pass through audio (copy input to output)
        output[..len].copy_from_slice(&input[..len]);

        // Capture audio samples to buffer
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.write(&input[..len]);
        }

        // Capture CV samples if CV input is connected (input 2)
        if inputs.len() > 2 && !inputs[2].is_empty() {
            let cv_input = inputs[2];
            if let Ok(mut cv_buffer) = self.cv_buffer.lock() {
                cv_buffer.write(&cv_input[..len.min(cv_input.len())]);
            }
        }

        // Update last sample for trigger detection (use left channel, frame 0)
        if !input.is_empty() {
            self.last_sample = input[0];
        }
    }

    fn reset(&mut self) {
        self.last_sample = 0.0;
        self.voct_value = 0.0;
        self.sample_counter = 0;
        self.clear_buffer();
    }

    fn node_type(&self) -> &str {
        "Oscilloscope"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        Box::new(Self {
            name: self.name.clone(),
            time_scale: self.time_scale,
            trigger_mode: self.trigger_mode,
            trigger_level: self.trigger_level,
            last_sample: 0.0,
            voct_value: 0.0,
            sample_counter: 0,
            trigger_period: 480,
            buffer: Arc::new(Mutex::new(CircularBuffer::new(BUFFER_SIZE))),
            cv_buffer: Arc::new(Mutex::new(CircularBuffer::new(BUFFER_SIZE))),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
        })
    }

    fn get_oscilloscope_data(&self, sample_count: usize) -> Option<Vec<f32>> {
        Some(self.read_samples(sample_count))
    }

    fn get_oscilloscope_cv_data(&self, sample_count: usize) -> Option<Vec<f32>> {
        Some(self.read_cv_samples(sample_count))
    }
}
