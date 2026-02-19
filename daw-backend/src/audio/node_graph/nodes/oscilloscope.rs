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
    buffer: Arc<Mutex<CircularBuffer>>,       // Audio buffer (mono downmix)
    cv_buffer: Arc<Mutex<CircularBuffer>>,    // CV buffer
    mono_buf: Vec<f32>,                       // Scratch buffer for stereo-to-mono downmix

    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
}

impl OscilloscopeNode {
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();

        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
            NodePort::new("CV In", SignalType::CV, 1),
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
            mono_buf: vec![0.0; 2048],
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
        let stereo_len = input.len().min(output.len());
        let frame_count = stereo_len / 2;

        // Read CV input if available (port 1) — used for both display and V/Oct triggering
        if inputs.len() > 1 && !inputs[1].is_empty() {
            let cv_input = inputs[1];
            let cv_len = frame_count.min(cv_input.len());

            // Check if connected (not NaN sentinel)
            if cv_len > 0 && !cv_input[0].is_nan() {
                // Update V/Oct trigger period from CV value
                self.voct_value = cv_input[0];
                let frequency = Self::voct_to_frequency(self.voct_value);
                let period_samples = (sample_rate as f32 / frequency).max(1.0);
                self.trigger_period = period_samples as usize;

                // Capture CV samples to buffer
                if let Ok(mut cv_buffer) = self.cv_buffer.lock() {
                    cv_buffer.write(&cv_input[..cv_len]);
                }
            }
        }

        // Update sample counter for V/oct triggering
        if self.trigger_mode == TriggerMode::VoltPerOctave {
            self.sample_counter = (self.sample_counter + frame_count) % self.trigger_period;
        }

        // Pass through audio (copy input to output)
        output[..stereo_len].copy_from_slice(&input[..stereo_len]);

        // Capture audio as mono downmix to match CV time scale
        if let Ok(mut buffer) = self.buffer.lock() {
            for frame in 0..frame_count {
                let left = input[frame * 2];
                let right = input[frame * 2 + 1];
                self.mono_buf[frame] = (left + right) * 0.5;
            }
            buffer.write(&self.mono_buf[..frame_count]);
        }

        // Update last sample for trigger detection
        if frame_count > 0 {
            self.last_sample = (input[0] + input[1]) * 0.5;
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
            mono_buf: vec![0.0; 2048],
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

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
