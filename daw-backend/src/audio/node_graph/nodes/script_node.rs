use crate::audio::node_graph::{AudioNode, NodeCategory, NodePort, Parameter, ParameterUnit, SignalType};
use crate::audio::midi::MidiEvent;
use beamdsp::{ScriptVM, SampleSlot};

/// A user-scriptable audio node powered by the BeamDSP VM
pub struct ScriptNode {
    name: String,
    script_name: String,
    inputs: Vec<NodePort>,
    outputs: Vec<NodePort>,
    parameters: Vec<Parameter>,
    category: NodeCategory,
    vm: ScriptVM,
    source_code: String,
    ui_declaration: beamdsp::UiDeclaration,
}

impl ScriptNode {
    /// Create a default empty Script node (compiles a passthrough on first script set)
    pub fn new(name: impl Into<String>) -> Self {
        let name = name.into();
        // Default: single audio in, single audio out, no params
        let inputs = vec![
            NodePort::new("Audio In", SignalType::Audio, 0),
        ];
        let outputs = vec![
            NodePort::new("Audio Out", SignalType::Audio, 0),
        ];

        // Create a minimal VM that just halts (no-op)
        let vm = ScriptVM::new(
            vec![255], // Halt
            Vec::new(),
            Vec::new(),
            0,
            &[],
            0,
            &[],
            0,
        );

        Self {
            name,
            script_name: "Script".into(),
            inputs,
            outputs,
            parameters: Vec::new(),
            category: NodeCategory::Effect,
            vm,
            source_code: String::new(),
            ui_declaration: beamdsp::UiDeclaration { elements: Vec::new() },
        }
    }

    /// Compile and load a new script, replacing the current one.
    /// Returns Ok(ui_declaration) on success, or Err(error_message) on failure.
    pub fn set_script(&mut self, source: &str) -> Result<beamdsp::UiDeclaration, String> {
        let compiled = beamdsp::compile(source).map_err(|e| e.to_string())?;

        // Update ports
        self.inputs = compiled.input_ports.iter().enumerate().map(|(i, p)| {
            let sig = match p.signal {
                beamdsp::ast::SignalKind::Audio => SignalType::Audio,
                beamdsp::ast::SignalKind::Cv => SignalType::CV,
                beamdsp::ast::SignalKind::Midi => SignalType::Midi,
            };
            NodePort::new(&p.name, sig, i)
        }).collect();

        self.outputs = compiled.output_ports.iter().enumerate().map(|(i, p)| {
            let sig = match p.signal {
                beamdsp::ast::SignalKind::Audio => SignalType::Audio,
                beamdsp::ast::SignalKind::Cv => SignalType::CV,
                beamdsp::ast::SignalKind::Midi => SignalType::Midi,
            };
            NodePort::new(&p.name, sig, i)
        }).collect();

        // Update parameters
        self.parameters = compiled.parameters.iter().enumerate().map(|(i, p)| {
            let unit = if p.unit == "dB" {
                ParameterUnit::Decibels
            } else if p.unit == "Hz" {
                ParameterUnit::Frequency
            } else if p.unit == "s" {
                ParameterUnit::Time
            } else if p.unit == "%" {
                ParameterUnit::Percent
            } else {
                ParameterUnit::Generic
            };
            Parameter::new(i as u32, &p.name, p.min, p.max, p.default, unit)
        }).collect();

        // Update category
        self.category = match compiled.category {
            beamdsp::ast::CategoryKind::Generator => NodeCategory::Generator,
            beamdsp::ast::CategoryKind::Effect => NodeCategory::Effect,
            beamdsp::ast::CategoryKind::Utility => NodeCategory::Utility,
        };

        self.script_name = compiled.name.clone();
        self.vm = compiled.vm;
        self.source_code = compiled.source;
        self.ui_declaration = compiled.ui_declaration.clone();

        Ok(compiled.ui_declaration)
    }

    /// Set audio data for a sample slot
    pub fn set_sample(&mut self, slot_index: usize, data: Vec<f32>, sample_rate: u32, name: String) {
        if slot_index < self.vm.sample_slots.len() {
            let frame_count = data.len() / 2;
            self.vm.sample_slots[slot_index] = SampleSlot {
                data,
                frame_count,
                sample_rate,
                name,
            };
        }
    }

    pub fn source_code(&self) -> &str {
        &self.source_code
    }

    pub fn ui_declaration(&self) -> &beamdsp::UiDeclaration {
        &self.ui_declaration
    }

    pub fn sample_slot_names(&self) -> Vec<String> {
        self.vm.sample_slots.iter().map(|s| s.name.clone()).collect()
    }
}

impl AudioNode for ScriptNode {
    fn category(&self) -> NodeCategory {
        self.category
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
        let idx = id as usize;
        let params = self.vm.params_mut();
        if idx < params.len() {
            params[idx] = value;
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        let idx = id as usize;
        let params = self.vm.params();
        if idx < params.len() {
            params[idx]
        } else {
            0.0
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

        // Determine buffer size from output buffer length
        let buffer_size = outputs[0].len();

        // Execute VM — on error, zero all outputs (fail silent on audio thread)
        if let Err(_) = self.vm.execute(inputs, outputs, sample_rate, buffer_size) {
            for out in outputs.iter_mut() {
                out.fill(0.0);
            }
        }
    }

    fn reset(&mut self) {
        self.vm.reset_state();
    }

    fn node_type(&self) -> &str {
        "Script"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn clone_node(&self) -> Box<dyn AudioNode> {
        let mut cloned = Self {
            name: self.name.clone(),
            script_name: self.script_name.clone(),
            inputs: self.inputs.clone(),
            outputs: self.outputs.clone(),
            parameters: self.parameters.clone(),
            category: self.category,
            vm: self.vm.clone(),
            source_code: self.source_code.clone(),
            ui_declaration: self.ui_declaration.clone(),
        };
        cloned.vm.reset_state();
        Box::new(cloned)
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
