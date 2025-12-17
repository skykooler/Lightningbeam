//! Graph Data Types for egui_node_graph2
//!
//! Node definitions and trait implementations for audio/MIDI node graph

use eframe::egui;
use egui_node_graph2::*;
use serde::{Deserialize, Serialize};

/// Signal types for audio node graph
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    Audio,
    Midi,
    CV,
}

/// Node templates - types of nodes that can be created
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeTemplate {
    // Inputs
    MidiInput,
    AudioInput,
    AutomationInput,

    // Generators
    Oscillator,
    WavetableOscillator,
    FmSynth,
    Noise,
    SimpleSampler,
    MultiSampler,

    // Effects
    Filter,
    Gain,
    Delay,
    Reverb,
    Chorus,
    Flanger,
    Phaser,
    Distortion,
    BitCrusher,
    Compressor,
    Limiter,
    Eq,
    Pan,
    RingModulator,
    Vocoder,

    // Utilities
    Adsr,
    Lfo,
    Mixer,
    Splitter,
    Constant,
    MidiToCv,
    AudioToCv,
    Math,
    SampleHold,
    SlewLimiter,
    Quantizer,
    EnvelopeFollower,
    BpmDetector,
    Mod,

    // Analysis
    Oscilloscope,

    // Advanced
    VoiceAllocator,

    // Outputs
    AudioOutput,
}

/// Custom node data - empty for now, can be extended
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeData;

/// Custom graph state - can track selected nodes, etc.
#[derive(Default)]
pub struct GraphState {
    pub active_node: Option<NodeId>,
}

/// User response type (empty for now)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserResponse {}

impl UserResponseTrait for UserResponse {}

/// Value types for inline parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ValueType {
    Float { value: f32 },
    String { value: String },
}

impl Default for ValueType {
    fn default() -> Self {
        ValueType::Float { value: 0.0 }
    }
}

// Implement DataTypeTrait for our signal types
impl DataTypeTrait<GraphState> for DataType {
    fn data_type_color(&self, _user_state: &mut GraphState) -> egui::Color32 {
        match self {
            DataType::Audio => egui::Color32::from_rgb(100, 150, 255),  // Blue
            DataType::Midi => egui::Color32::from_rgb(100, 255, 100),   // Green
            DataType::CV => egui::Color32::from_rgb(255, 150, 100),     // Orange
        }
    }

    fn name(&self) -> std::borrow::Cow<'_, str> {
        match self {
            DataType::Audio => "Audio".into(),
            DataType::Midi => "MIDI".into(),
            DataType::CV => "CV".into(),
        }
    }
}

// Implement NodeTemplateTrait for our node types
impl NodeTemplateTrait for NodeTemplate {
    type NodeData = NodeData;
    type DataType = DataType;
    type ValueType = ValueType;
    type UserState = GraphState;
    type CategoryType = &'static str;

    fn node_finder_label(&self, _user_state: &mut Self::UserState) -> std::borrow::Cow<'_, str> {
        match self {
            // Inputs
            NodeTemplate::MidiInput => "MIDI Input".into(),
            NodeTemplate::AudioInput => "Audio Input".into(),
            NodeTemplate::AutomationInput => "Automation Input".into(),
            // Generators
            NodeTemplate::Oscillator => "Oscillator".into(),
            NodeTemplate::WavetableOscillator => "Wavetable Oscillator".into(),
            NodeTemplate::FmSynth => "FM Synth".into(),
            NodeTemplate::Noise => "Noise Generator".into(),
            NodeTemplate::SimpleSampler => "Simple Sampler".into(),
            NodeTemplate::MultiSampler => "Multi Sampler".into(),
            // Effects
            NodeTemplate::Filter => "Filter".into(),
            NodeTemplate::Gain => "Gain".into(),
            NodeTemplate::Delay => "Delay".into(),
            NodeTemplate::Reverb => "Reverb".into(),
            NodeTemplate::Chorus => "Chorus".into(),
            NodeTemplate::Flanger => "Flanger".into(),
            NodeTemplate::Phaser => "Phaser".into(),
            NodeTemplate::Distortion => "Distortion".into(),
            NodeTemplate::BitCrusher => "Bit Crusher".into(),
            NodeTemplate::Compressor => "Compressor".into(),
            NodeTemplate::Limiter => "Limiter".into(),
            NodeTemplate::Eq => "EQ".into(),
            NodeTemplate::Pan => "Pan".into(),
            NodeTemplate::RingModulator => "Ring Modulator".into(),
            NodeTemplate::Vocoder => "Vocoder".into(),
            // Utilities
            NodeTemplate::Adsr => "ADSR Envelope".into(),
            NodeTemplate::Lfo => "LFO".into(),
            NodeTemplate::Mixer => "Mixer".into(),
            NodeTemplate::Splitter => "Splitter".into(),
            NodeTemplate::Constant => "Constant".into(),
            NodeTemplate::MidiToCv => "MIDI to CV".into(),
            NodeTemplate::AudioToCv => "Audio to CV".into(),
            NodeTemplate::Math => "Math".into(),
            NodeTemplate::SampleHold => "Sample & Hold".into(),
            NodeTemplate::SlewLimiter => "Slew Limiter".into(),
            NodeTemplate::Quantizer => "Quantizer".into(),
            NodeTemplate::EnvelopeFollower => "Envelope Follower".into(),
            NodeTemplate::BpmDetector => "BPM Detector".into(),
            NodeTemplate::Mod => "Modulator".into(),
            // Analysis
            NodeTemplate::Oscilloscope => "Oscilloscope".into(),
            // Advanced
            NodeTemplate::VoiceAllocator => "Voice Allocator".into(),
            // Outputs
            NodeTemplate::AudioOutput => "Audio Output".into(),
        }
    }

    fn node_finder_categories(&self, _user_state: &mut Self::UserState) -> Vec<&'static str> {
        match self {
            NodeTemplate::MidiInput | NodeTemplate::AudioInput | NodeTemplate::AutomationInput => vec!["Inputs"],
            NodeTemplate::Oscillator | NodeTemplate::WavetableOscillator | NodeTemplate::FmSynth
            | NodeTemplate::Noise | NodeTemplate::SimpleSampler | NodeTemplate::MultiSampler => vec!["Generators"],
            NodeTemplate::Filter | NodeTemplate::Gain | NodeTemplate::Delay | NodeTemplate::Reverb
            | NodeTemplate::Chorus | NodeTemplate::Flanger | NodeTemplate::Phaser | NodeTemplate::Distortion
            | NodeTemplate::BitCrusher | NodeTemplate::Compressor | NodeTemplate::Limiter | NodeTemplate::Eq
            | NodeTemplate::Pan | NodeTemplate::RingModulator | NodeTemplate::Vocoder => vec!["Effects"],
            NodeTemplate::Adsr | NodeTemplate::Lfo | NodeTemplate::Mixer | NodeTemplate::Splitter
            | NodeTemplate::Constant | NodeTemplate::MidiToCv | NodeTemplate::AudioToCv | NodeTemplate::Math
            | NodeTemplate::SampleHold | NodeTemplate::SlewLimiter | NodeTemplate::Quantizer
            | NodeTemplate::EnvelopeFollower | NodeTemplate::BpmDetector | NodeTemplate::Mod => vec!["Utilities"],
            NodeTemplate::Oscilloscope => vec!["Analysis"],
            NodeTemplate::VoiceAllocator => vec!["Advanced"],
            NodeTemplate::AudioOutput => vec!["Outputs"],
        }
    }

    fn node_graph_label(&self, user_state: &mut Self::UserState) -> String {
        self.node_finder_label(user_state).into()
    }

    fn user_data(&self, _user_state: &mut Self::UserState) -> Self::NodeData {
        NodeData
    }

    fn build_node(
        &self,
        graph: &mut Graph<Self::NodeData, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
        node_id: NodeId,
    ) {
        match self {
            NodeTemplate::Oscillator => {
                // V/Oct input (pitch control voltage)
                graph.add_input_param(
                    node_id,
                    "V/Oct".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                // FM input (frequency modulation)
                graph.add_input_param(
                    node_id,
                    "FM".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                // Audio output
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Noise => {
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Filter => {
                graph.add_input_param(
                    node_id,
                    "Audio In".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Cutoff CV".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Gain => {
                graph.add_input_param(
                    node_id,
                    "Audio In".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Gain CV".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Adsr => {
                graph.add_input_param(
                    node_id,
                    "Gate".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                // Parameters
                graph.add_input_param(
                    node_id,
                    "Attack".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.01 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Decay".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.1 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Sustain".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.7 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Release".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.2 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Envelope Out".into(), DataType::CV);
            }
            NodeTemplate::Lfo => {
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::AudioOutput => {
                graph.add_input_param(
                    node_id,
                    "Audio In".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
            }
            NodeTemplate::AudioInput => {
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::MidiInput => {
                graph.add_output_param(node_id, "MIDI Out".into(), DataType::Midi);
            }
            NodeTemplate::Delay => {
                graph.add_input_param(
                    node_id,
                    "Audio In".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                // Parameters
                graph.add_input_param(
                    node_id,
                    "Delay Time".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.5 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Feedback".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.5 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Wet/Dry".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.5 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Mixer => {
                graph.add_input_param(
                    node_id,
                    "Input 1".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Input 2".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Input 3".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Input 4".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_output_param(node_id, "Mixed Out".into(), DataType::Audio);
            }
            NodeTemplate::Splitter => {
                graph.add_input_param(
                    node_id,
                    "Audio In".into(),
                    DataType::Audio,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_output_param(node_id, "Out 1".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 2".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 3".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 4".into(), DataType::Audio);
            }
            NodeTemplate::Constant => {
                // No inputs - value is set via parameter
                graph.add_input_param(
                    node_id,
                    "Value".into(),
                    DataType::CV,
                    ValueType::Float { value: 0.0 },
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::MidiToCv => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "V/Oct".into(), DataType::CV);
                graph.add_output_param(node_id, "Gate".into(), DataType::CV);
                graph.add_output_param(node_id, "Velocity".into(), DataType::CV);
            }
            // Stub implementations for all other nodes - add proper ports as needed
            NodeTemplate::AutomationInput => {
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::WavetableOscillator => {
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::FmSynth => {
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::SimpleSampler => {
                graph.add_input_param(node_id, "Gate".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::MultiSampler => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Reverb | NodeTemplate::Chorus | NodeTemplate::Flanger | NodeTemplate::Phaser
            | NodeTemplate::Distortion | NodeTemplate::BitCrusher | NodeTemplate::Compressor
            | NodeTemplate::Limiter | NodeTemplate::Eq | NodeTemplate::Pan | NodeTemplate::RingModulator
            | NodeTemplate::Vocoder => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::AudioToCv => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::Math => {
                graph.add_input_param(node_id, "A".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "B".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOrConstant, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::SampleHold | NodeTemplate::SlewLimiter | NodeTemplate::Quantizer | NodeTemplate::EnvelopeFollower => {
                graph.add_input_param(node_id, "In".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::BpmDetector => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "BPM".into(), DataType::CV);
            }
            NodeTemplate::Mod => {
                graph.add_input_param(node_id, "Carrier".into(), DataType::Audio, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Modulator".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::Audio);
            }
            NodeTemplate::Oscilloscope => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "CV In".into(), DataType::CV, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
            }
            NodeTemplate::VoiceAllocator => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::Float { value: 0.0 }, InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
        }
    }
}

// Implement WidgetValueTrait for parameter editing
impl WidgetValueTrait for ValueType {
    type Response = UserResponse;
    type UserState = GraphState;
    type NodeData = NodeData;

    fn value_widget(
        &mut self,
        param_name: &str,
        _node_id: NodeId,
        ui: &mut egui::Ui,
        _user_state: &mut Self::UserState,
        _node_data: &Self::NodeData,
    ) -> Vec<Self::Response> {
        match self {
            ValueType::Float { value } => {
                ui.horizontal(|ui| {
                    ui.label(param_name);
                    ui.add(egui::DragValue::new(value).speed(0.1));
                });
            }
            ValueType::String { value } => {
                ui.horizontal(|ui| {
                    ui.label(param_name);
                    ui.text_edit_singleline(value);
                });
            }
        }
        vec![]
    }
}

// Implement NodeDataTrait for custom node UI (optional)
impl NodeDataTrait for NodeData {
    type Response = UserResponse;
    type UserState = GraphState;
    type DataType = DataType;
    type ValueType = ValueType;

    fn bottom_ui(
        &self,
        ui: &mut egui::Ui,
        _node_id: NodeId,
        _graph: &Graph<NodeData, DataType, ValueType>,
        _user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, NodeData>>
    where
        Self::Response: UserResponseTrait,
    {
        // No custom UI for now
        ui.label("");
        vec![]
    }
}

// Iterator for all node templates
pub struct AllNodeTemplates;

impl NodeTemplateIter for AllNodeTemplates {
    type Item = NodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        vec![
            // Inputs
            NodeTemplate::MidiInput,
            NodeTemplate::AudioInput,
            NodeTemplate::AutomationInput,
            // Generators
            NodeTemplate::Oscillator,
            NodeTemplate::WavetableOscillator,
            NodeTemplate::FmSynth,
            NodeTemplate::Noise,
            NodeTemplate::SimpleSampler,
            NodeTemplate::MultiSampler,
            // Effects
            NodeTemplate::Filter,
            NodeTemplate::Gain,
            NodeTemplate::Delay,
            NodeTemplate::Reverb,
            NodeTemplate::Chorus,
            NodeTemplate::Flanger,
            NodeTemplate::Phaser,
            NodeTemplate::Distortion,
            NodeTemplate::BitCrusher,
            NodeTemplate::Compressor,
            NodeTemplate::Limiter,
            NodeTemplate::Eq,
            NodeTemplate::Pan,
            NodeTemplate::RingModulator,
            NodeTemplate::Vocoder,
            // Utilities
            NodeTemplate::Adsr,
            NodeTemplate::Lfo,
            NodeTemplate::Mixer,
            NodeTemplate::Splitter,
            NodeTemplate::Constant,
            NodeTemplate::MidiToCv,
            NodeTemplate::AudioToCv,
            NodeTemplate::Math,
            NodeTemplate::SampleHold,
            NodeTemplate::SlewLimiter,
            NodeTemplate::Quantizer,
            NodeTemplate::EnvelopeFollower,
            NodeTemplate::BpmDetector,
            NodeTemplate::Mod,
            // Analysis
            NodeTemplate::Oscilloscope,
            // Advanced
            NodeTemplate::VoiceAllocator,
            // Outputs
            NodeTemplate::AudioOutput,
        ]
    }
}
