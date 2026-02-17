//! Graph Data Types for egui_node_graph2
//!
//! Node definitions and trait implementations for audio/MIDI node graph

use eframe::egui;
use egui_node_graph2::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::widgets;

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
    Echo,
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
    Group,

    // Subgraph I/O (only visible when editing inside a container node)
    TemplateInput,
    TemplateOutput,

    // Outputs
    AudioOutput,
}

impl NodeTemplate {
    /// Returns the backend-compatible type name string (matches daw-backend match arms)
    pub fn backend_type_name(&self) -> &'static str {
        match self {
            NodeTemplate::MidiInput => "MidiInput",
            NodeTemplate::AudioInput => "AudioInput",
            NodeTemplate::AutomationInput => "AutomationInput",
            NodeTemplate::Oscillator => "Oscillator",
            NodeTemplate::WavetableOscillator => "WavetableOscillator",
            NodeTemplate::FmSynth => "FMSynth",
            NodeTemplate::Noise => "NoiseGenerator",
            NodeTemplate::SimpleSampler => "SimpleSampler",
            NodeTemplate::MultiSampler => "MultiSampler",
            NodeTemplate::Filter => "Filter",
            NodeTemplate::Gain => "Gain",
            NodeTemplate::Echo => "Echo",
            NodeTemplate::Reverb => "Reverb",
            NodeTemplate::Chorus => "Chorus",
            NodeTemplate::Flanger => "Flanger",
            NodeTemplate::Phaser => "Phaser",
            NodeTemplate::Distortion => "Distortion",
            NodeTemplate::BitCrusher => "BitCrusher",
            NodeTemplate::Compressor => "Compressor",
            NodeTemplate::Limiter => "Limiter",
            NodeTemplate::Eq => "EQ",
            NodeTemplate::Pan => "Pan",
            NodeTemplate::RingModulator => "RingModulator",
            NodeTemplate::Vocoder => "Vocoder",
            NodeTemplate::Adsr => "ADSR",
            NodeTemplate::Lfo => "LFO",
            NodeTemplate::Mixer => "Mixer",
            NodeTemplate::Splitter => "Splitter",
            NodeTemplate::Constant => "Constant",
            NodeTemplate::MidiToCv => "MidiToCV",
            NodeTemplate::AudioToCv => "AudioToCV",
            NodeTemplate::Math => "Math",
            NodeTemplate::SampleHold => "SampleHold",
            NodeTemplate::SlewLimiter => "SlewLimiter",
            NodeTemplate::Quantizer => "Quantizer",
            NodeTemplate::EnvelopeFollower => "EnvelopeFollower",
            NodeTemplate::BpmDetector => "BpmDetector",
            NodeTemplate::Mod => "Mod",
            NodeTemplate::Oscilloscope => "Oscilloscope",
            NodeTemplate::VoiceAllocator => "VoiceAllocator",
            NodeTemplate::Group => "Group",
            NodeTemplate::TemplateInput => "TemplateInput",
            NodeTemplate::TemplateOutput => "TemplateOutput",
            NodeTemplate::AudioOutput => "AudioOutput",
        }
    }
}

/// Custom node data
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeData {
    pub template: NodeTemplate,
    /// Display name of loaded sample (for SimpleSampler/MultiSampler nodes)
    #[serde(default)]
    pub sample_display_name: Option<String>,
    /// Root note (MIDI note number) for original-pitch playback (default 69 = A4)
    #[serde(default = "default_root_note")]
    pub root_note: u8,
}

fn default_root_note() -> u8 { 69 }

/// Cached oscilloscope waveform data for rendering in node body
pub struct OscilloscopeCache {
    pub audio: Vec<f32>,
    pub cv: Vec<f32>,
}

/// Info about an audio clip available for sampler selection
pub struct SamplerClipInfo {
    pub name: String,
    pub pool_index: usize,
}

/// Info about an asset folder available for multi-sampler
pub struct SamplerFolderInfo {
    pub folder_id: uuid::Uuid,
    pub name: String,
    /// Pool indices of audio clips in this folder
    pub clip_pool_indices: Vec<(String, usize)>,
}

/// Pending sampler load request from bottom_ui(), handled by the node graph pane
pub enum PendingSamplerLoad {
    /// Load a single clip from the audio pool into a SimpleSampler
    SimpleFromPool { node_id: NodeId, backend_node_id: u32, pool_index: usize, name: String },
    /// Open a file dialog to load into a SimpleSampler
    SimpleFromFile { node_id: NodeId, backend_node_id: u32 },
    /// Load a single clip from the audio pool as a MultiSampler layer
    MultiFromPool { node_id: NodeId, backend_node_id: u32, pool_index: usize, name: String },
    /// Load all clips in a folder as MultiSampler layers
    MultiFromFolder { node_id: NodeId, folder_id: uuid::Uuid },
    /// Open a file/folder dialog to load into a MultiSampler
    MultiFromFilesystem { node_id: NodeId, backend_node_id: u32 },
}

/// Custom graph state - can track selected nodes, etc.
pub struct GraphState {
    pub active_node: Option<NodeId>,
    /// Oscilloscope data cached per node, populated before draw_graph_editor()
    pub oscilloscope_data: HashMap<NodeId, OscilloscopeCache>,
    /// Audio clips available for sampler selection, populated before draw
    pub available_clips: Vec<SamplerClipInfo>,
    /// Asset folders available for multi-sampler, populated before draw
    pub available_folders: Vec<SamplerFolderInfo>,
    /// Pending sample load request from bottom_ui popup
    pub pending_sampler_load: Option<PendingSamplerLoad>,
    /// Search text for the sampler clip picker popup
    pub sampler_search_text: String,
    /// Mapping from frontend NodeId to backend node index, populated before draw
    pub node_backend_ids: HashMap<NodeId, u32>,
    /// Pending root note changes from bottom_ui (node_id, backend_node_id, new_root_note)
    pub pending_root_note_changes: Vec<(NodeId, u32, u8)>,
    /// Time scale per oscilloscope node (in milliseconds)
    pub oscilloscope_time_scale: HashMap<NodeId, f32>,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            active_node: None,
            oscilloscope_data: HashMap::new(),
            available_clips: Vec::new(),
            available_folders: Vec::new(),
            pending_sampler_load: None,
            sampler_search_text: String::new(),
            node_backend_ids: HashMap::new(),
            pending_root_note_changes: Vec::new(),
            oscilloscope_time_scale: HashMap::new(),
        }
    }
}

/// User response type (empty for now)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserResponse {}

impl UserResponseTrait for UserResponse {}

fn default_unit() -> &'static str { "" }

/// Value types for inline parameters
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ValueType {
    Float {
        value: f32,
        #[serde(skip, default)]
        min: f32,
        #[serde(skip, default)]
        max: f32,
        #[serde(skip, default = "default_unit")]
        unit: &'static str,
        #[serde(skip)]
        backend_param_id: Option<u32>,
        #[serde(skip)]
        enum_labels: Option<&'static [&'static str]>,
    },
    String { value: String },
}

impl ValueType {
    /// Plain float value (for connection inputs, no parameter metadata)
    pub fn float(value: f32) -> Self {
        ValueType::Float {
            value,
            min: 0.0,
            max: 0.0,
            unit: "",
            backend_param_id: None,
            enum_labels: None,
        }
    }

    /// Float parameter with full metadata for inline editing
    pub fn float_param(
        value: f32,
        min: f32,
        max: f32,
        unit: &'static str,
        param_id: u32,
        enum_labels: Option<&'static [&'static str]>,
    ) -> Self {
        ValueType::Float {
            value,
            min,
            max,
            unit,
            backend_param_id: Some(param_id),
            enum_labels,
        }
    }
}

impl Default for ValueType {
    fn default() -> Self {
        ValueType::Float {
            value: 0.0,
            min: 0.0,
            max: 0.0,
            unit: "",
            backend_param_id: None,
            enum_labels: None,
        }
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
            NodeTemplate::Echo => "Echo".into(),
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
            NodeTemplate::Group => "Group".into(),
            // Subgraph I/O
            NodeTemplate::TemplateInput => "Template Input".into(),
            NodeTemplate::TemplateOutput => "Template Output".into(),
            // Outputs
            NodeTemplate::AudioOutput => "Audio Output".into(),
        }
    }

    fn node_finder_categories(&self, _user_state: &mut Self::UserState) -> Vec<&'static str> {
        match self {
            NodeTemplate::MidiInput | NodeTemplate::AudioInput | NodeTemplate::AutomationInput => vec!["Inputs"],
            NodeTemplate::Oscillator | NodeTemplate::WavetableOscillator | NodeTemplate::FmSynth
            | NodeTemplate::Noise | NodeTemplate::SimpleSampler | NodeTemplate::MultiSampler => vec!["Generators"],
            NodeTemplate::Filter | NodeTemplate::Gain | NodeTemplate::Echo | NodeTemplate::Reverb
            | NodeTemplate::Chorus | NodeTemplate::Flanger | NodeTemplate::Phaser | NodeTemplate::Distortion
            | NodeTemplate::BitCrusher | NodeTemplate::Compressor | NodeTemplate::Limiter | NodeTemplate::Eq
            | NodeTemplate::Pan | NodeTemplate::RingModulator | NodeTemplate::Vocoder => vec!["Effects"],
            NodeTemplate::Adsr | NodeTemplate::Lfo | NodeTemplate::Mixer | NodeTemplate::Splitter
            | NodeTemplate::Constant | NodeTemplate::MidiToCv | NodeTemplate::AudioToCv | NodeTemplate::Math
            | NodeTemplate::SampleHold | NodeTemplate::SlewLimiter | NodeTemplate::Quantizer
            | NodeTemplate::EnvelopeFollower | NodeTemplate::BpmDetector | NodeTemplate::Mod => vec!["Utilities"],
            NodeTemplate::Oscilloscope => vec!["Analysis"],
            NodeTemplate::VoiceAllocator | NodeTemplate::Group => vec!["Advanced"],
            NodeTemplate::TemplateInput | NodeTemplate::TemplateOutput => vec!["Subgraph I/O"],
            NodeTemplate::AudioOutput => vec!["Outputs"],
        }
    }

    fn node_graph_label(&self, user_state: &mut Self::UserState) -> String {
        self.node_finder_label(user_state).into()
    }

    fn user_data(&self, _user_state: &mut Self::UserState) -> Self::NodeData {
        NodeData { template: *self, sample_display_name: None, root_note: 69 }
    }

    fn build_node(
        &self,
        graph: &mut Graph<Self::NodeData, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
        node_id: NodeId,
    ) {
        match self {
            NodeTemplate::Oscillator => {
                // Connection inputs
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "FM".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Frequency".into(), DataType::CV,
                    ValueType::float_param(440.0, 20.0, 20000.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Amplitude".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Waveform".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 2, Some(&["Sine", "Saw", "Square", "Triangle"])), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Noise => {
                graph.add_input_param(node_id, "Color".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 2.0, "", 0, Some(&["White", "Pink", "Brown"])), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Filter => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Cutoff CV".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Cutoff".into(), DataType::CV,
                    ValueType::float_param(1000.0, 20.0, 20000.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Resonance".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Type".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 2, Some(&["LPF", "HPF", "BPF", "Notch"])), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Gain => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Gain CV".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Gain".into(), DataType::CV,
                    ValueType::float_param(0.0, -60.0, 12.0, " dB", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Adsr => {
                graph.add_input_param(node_id, "Gate".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Attack".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.001, 5.0, " s", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Decay".into(), DataType::CV,
                    ValueType::float_param(0.1, 0.001, 5.0, " s", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Sustain".into(), DataType::CV,
                    ValueType::float_param(0.7, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(0.2, 0.001, 5.0, " s", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Envelope Out".into(), DataType::CV);
            }
            NodeTemplate::Lfo => {
                // Parameters
                graph.add_input_param(node_id, "Rate".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.01, 20.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Amplitude".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Waveform".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 4.0, "", 2, Some(&["Sine", "Triangle", "Square", "Saw", "Random"])), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Phase".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::AudioOutput => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
            }
            NodeTemplate::AudioInput => {
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::MidiInput => {
                graph.add_output_param(node_id, "MIDI Out".into(), DataType::Midi);
            }
            NodeTemplate::Echo => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Delay Time".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.01, 2.0, " s", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Feedback".into(), DataType::CV,
                    ValueType::float_param(0.3, 0.0, 0.95, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Mixer => {
                graph.add_input_param(node_id, "Input 1".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Input 2".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Input 3".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Input 4".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Level parameters
                graph.add_input_param(node_id, "Level 1".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Level 2".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Level 3".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Level 4".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Mixed Out".into(), DataType::Audio);
            }
            NodeTemplate::Splitter => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Out 1".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 2".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 3".into(), DataType::Audio);
                graph.add_output_param(node_id, "Out 4".into(), DataType::Audio);
            }
            NodeTemplate::Constant => {
                graph.add_input_param(node_id, "Value".into(), DataType::CV,
                    ValueType::float_param(0.0, -1.0, 1.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::MidiToCv => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "V/Oct".into(), DataType::CV);
                graph.add_output_param(node_id, "Gate".into(), DataType::CV);
                graph.add_output_param(node_id, "Velocity".into(), DataType::CV);
            }
            // Stub implementations for all other nodes - add proper ports as needed
            NodeTemplate::AutomationInput => {
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::WavetableOscillator => {
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "Wavetable".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 7.0, "", 0, Some(&["Sine", "Saw", "Square", "Triangle", "Pulse", "Noise", "FM", "Additive"])), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Fine Tune".into(), DataType::CV,
                    ValueType::float_param(0.0, -1.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Position".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::FmSynth => {
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "Gate".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Algorithm".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 0, Some(&["Stack", "Parallel", "Diamond", "Feedback"])), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op1 Ratio".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.25, 16.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op1 Level".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op2 Ratio".into(), DataType::CV,
                    ValueType::float_param(2.0, 0.25, 16.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op2 Level".into(), DataType::CV,
                    ValueType::float_param(0.8, 0.0, 1.0, "", 4, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op3 Ratio".into(), DataType::CV,
                    ValueType::float_param(3.0, 0.25, 16.0, "", 5, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op3 Level".into(), DataType::CV,
                    ValueType::float_param(0.6, 0.0, 1.0, "", 6, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op4 Ratio".into(), DataType::CV,
                    ValueType::float_param(4.0, 0.25, 16.0, "", 7, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Op4 Level".into(), DataType::CV,
                    ValueType::float_param(0.4, 0.0, 1.0, "", 8, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::SimpleSampler => {
                graph.add_input_param(node_id, "V/Oct".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Gate".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::MultiSampler => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Reverb => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Room Size".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Damping".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Wet/Dry".into(), DataType::CV,
                    ValueType::float_param(0.3, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Chorus => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Rate".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.1, 5.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Depth".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Wet/Dry".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Flanger => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Rate".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.1, 10.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Depth".into(), DataType::CV,
                    ValueType::float_param(0.7, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Feedback".into(), DataType::CV,
                    ValueType::float_param(0.5, -0.95, 0.95, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Wet/Dry".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Phaser => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Rate".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.1, 10.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Depth".into(), DataType::CV,
                    ValueType::float_param(0.7, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Stages".into(), DataType::CV,
                    ValueType::float_param(6.0, 2.0, 8.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Feedback".into(), DataType::CV,
                    ValueType::float_param(0.5, -0.95, 0.95, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Wet/Dry".into(), DataType::CV,
                    ValueType::float_param(0.5, 0.0, 1.0, "", 4, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Distortion => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Drive".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.01, 20.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Type".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 1, Some(&["Soft", "Hard", "Foldback", "Bitcrush"])), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Tone".into(), DataType::CV,
                    ValueType::float_param(0.7, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::BitCrusher => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Bit Depth".into(), DataType::CV,
                    ValueType::float_param(8.0, 1.0, 16.0, " bits", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Sample Rate".into(), DataType::CV,
                    ValueType::float_param(8000.0, 100.0, 48000.0, " Hz", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Compressor => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Sidechain".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Threshold".into(), DataType::CV,
                    ValueType::float_param(-20.0, -60.0, 0.0, " dB", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Ratio".into(), DataType::CV,
                    ValueType::float_param(4.0, 1.0, 20.0, ":1", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Attack".into(), DataType::CV,
                    ValueType::float_param(5.0, 0.1, 100.0, " ms", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(50.0, 10.0, 1000.0, " ms", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Makeup".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 24.0, " dB", 4, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Knee".into(), DataType::CV,
                    ValueType::float_param(3.0, 0.0, 12.0, " dB", 5, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Limiter => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Threshold".into(), DataType::CV,
                    ValueType::float_param(-1.0, -60.0, 0.0, " dB", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(50.0, 1.0, 500.0, " ms", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Ceiling".into(), DataType::CV,
                    ValueType::float_param(0.0, -60.0, 0.0, " dB", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Eq => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Low Freq".into(), DataType::CV,
                    ValueType::float_param(100.0, 20.0, 500.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Low Gain".into(), DataType::CV,
                    ValueType::float_param(0.0, -24.0, 24.0, " dB", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mid Freq".into(), DataType::CV,
                    ValueType::float_param(1000.0, 200.0, 5000.0, " Hz", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mid Gain".into(), DataType::CV,
                    ValueType::float_param(0.0, -24.0, 24.0, " dB", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mid Q".into(), DataType::CV,
                    ValueType::float_param(0.707, 0.1, 10.0, "", 4, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "High Freq".into(), DataType::CV,
                    ValueType::float_param(8000.0, 2000.0, 20000.0, " Hz", 5, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "High Gain".into(), DataType::CV,
                    ValueType::float_param(0.0, -24.0, 24.0, " dB", 6, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Pan => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Pan".into(), DataType::CV,
                    ValueType::float_param(0.0, -1.0, 1.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::RingModulator => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Modulator".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Vocoder => {
                graph.add_input_param(node_id, "Modulator".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Carrier".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Bands".into(), DataType::CV,
                    ValueType::float_param(16.0, 8.0, 32.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Attack".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.001, 0.1, " s", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(0.05, 0.001, 1.0, " s", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 3, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::AudioToCv => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "CV Out".into(), DataType::CV);
            }
            NodeTemplate::Math => {
                graph.add_input_param(node_id, "A".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOrConstant, true);
                graph.add_input_param(node_id, "B".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOrConstant, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::SampleHold | NodeTemplate::Quantizer => {
                graph.add_input_param(node_id, "In".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::SlewLimiter => {
                graph.add_input_param(node_id, "In".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Rise Time".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.0, 5.0, " s", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Fall Time".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.0, 5.0, " s", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::EnvelopeFollower => {
                graph.add_input_param(node_id, "In".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Attack".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.001, 1.0, " s", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(0.1, 0.001, 1.0, " s", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::CV);
            }
            NodeTemplate::BpmDetector => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "BPM".into(), DataType::CV);
            }
            NodeTemplate::Mod => {
                graph.add_input_param(node_id, "Carrier".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Modulator".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Out".into(), DataType::Audio);
            }
            NodeTemplate::Oscilloscope => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "CV In".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
            }
            NodeTemplate::VoiceAllocator => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Voices".into(), DataType::CV,
                    ValueType::float_param(8.0, 1.0, 16.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::Group => {
                // Ports are dynamic based on subgraph TemplateInput/Output nodes.
                // Start with one audio pass-through by default.
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_output_param(node_id, "Audio Out".into(), DataType::Audio);
            }
            NodeTemplate::TemplateInput => {
                // Inside a VA template: provides MIDI from the allocator
                graph.add_output_param(node_id, "MIDI Out".into(), DataType::Midi);
            }
            NodeTemplate::TemplateOutput => {
                // Inside a VA template: sends audio back to the allocator
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
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
            ValueType::Float { value, min, max, unit, enum_labels, .. } => {
                let has_range = *max > *min;
                if let Some(labels) = enum_labels {
                    // Enum parameter: render as ComboBox dropdown
                    let mut selected = (*value as usize).min(labels.len().saturating_sub(1));
                    ui.horizontal(|ui| {
                        ui.label(param_name);
                        egui::ComboBox::from_id_salt(param_name)
                            .selected_text(labels.get(selected).copied().unwrap_or("?"))
                            .width(90.0)
                            .show_ui(ui, |ui| {
                                for (i, label) in labels.iter().enumerate() {
                                    ui.selectable_value(&mut selected, i, *label);
                                }
                            });
                    });
                    *value = selected as f32;
                } else if has_range {
                    // Ranged parameter: render clamped DragValue with unit suffix
                    let range = *max - *min;
                    let speed = if range > 1000.0 {
                        // Logarithmic-ish speed for large ranges (frequency, time)
                        (*value).max(1.0) * 0.01
                    } else {
                        range / 300.0
                    };
                    ui.horizontal(|ui| {
                        ui.label(param_name);
                        let mut dv = egui::DragValue::new(value)
                            .speed(speed)
                            .range(*min..=*max);
                        if !unit.is_empty() {
                            dv = dv.suffix(*unit);
                        }
                        ui.add(dv);
                    });
                } else {
                    // Plain float (no metadata)
                    ui.horizontal(|ui| {
                        ui.label(param_name);
                        ui.add(egui::DragValue::new(value).speed(0.1));
                    });
                }
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

const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

fn midi_note_name(note: u8) -> String {
    let octave = (note as i32 / 12) - 1;
    let name = NOTE_NAMES[note as usize % 12];
    format!("{}{}", name, octave)
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
        node_id: NodeId,
        _graph: &Graph<NodeData, DataType, ValueType>,
        user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, NodeData>>
    where
        Self::Response: UserResponseTrait,
    {
        if self.template == NodeTemplate::SimpleSampler || self.template == NodeTemplate::MultiSampler {
            let is_multi = self.template == NodeTemplate::MultiSampler;
            let backend_node_id = user_state.node_backend_ids.get(&node_id).copied().unwrap_or(0);
            let default_text = if is_multi { "Select samples..." } else { "Select sample..." };
            let button_text = self.sample_display_name.as_deref().unwrap_or(default_text);

            let button = ui.button(button_text);
            if button.clicked() {
                user_state.sampler_search_text.clear();
            }
            let popup_id = egui::Popup::default_response_id(&button);

            let mut close_popup = false;
            egui::Popup::from_toggle_button_response(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(160.0)
                .show(|ui| {
                let search_width = ui.available_width();
                ui.add_sized([search_width, 0.0], egui::TextEdit::singleline(&mut user_state.sampler_search_text).hint_text("Search..."));
                ui.separator();
                let search = user_state.sampler_search_text.to_lowercase();

                // Folders section (multi-sampler only)
                if is_multi && !user_state.available_folders.is_empty() {
                    ui.label(egui::RichText::new("Folders").small().weak());
                    for folder in &user_state.available_folders {
                        if !search.is_empty() && !folder.name.to_lowercase().contains(&search) {
                            continue;
                        }
                        let label = format!("📁 {} ({} clips)", folder.name, folder.clip_pool_indices.len());
                        if widgets::list_item(ui, false, &label) {
                            user_state.pending_sampler_load = Some(PendingSamplerLoad::MultiFromFolder {
                                node_id,
                                folder_id: folder.folder_id,
                            });
                            close_popup = true;
                        }
                    }
                    ui.separator();
                }

                // Audio clips list
                if is_multi {
                    ui.label(egui::RichText::new("Audio Clips").small().weak());
                }
                let filtered_clips: Vec<&SamplerClipInfo> = user_state.available_clips.iter()
                    .filter(|clip| search.is_empty() || clip.name.to_lowercase().contains(&search))
                    .collect();
                let items = filtered_clips.iter().map(|clip| (false, clip.name.as_str()));
                if let Some(idx) = widgets::scrollable_list(ui, 200.0, items) {
                    let clip = filtered_clips[idx];
                    if is_multi {
                        user_state.pending_sampler_load = Some(PendingSamplerLoad::MultiFromPool {
                            node_id,
                            backend_node_id,
                            pool_index: clip.pool_index,
                            name: clip.name.clone(),
                        });
                    } else {
                        user_state.pending_sampler_load = Some(PendingSamplerLoad::SimpleFromPool {
                            node_id,
                            backend_node_id,
                            pool_index: clip.pool_index,
                            name: clip.name.clone(),
                        });
                    }
                    close_popup = true;
                }
                ui.separator();
                if ui.button("Open...").clicked() {
                    if is_multi {
                        user_state.pending_sampler_load = Some(PendingSamplerLoad::MultiFromFilesystem {
                            node_id,
                            backend_node_id,
                        });
                    } else {
                        user_state.pending_sampler_load = Some(PendingSamplerLoad::SimpleFromFile {
                            node_id,
                            backend_node_id,
                        });
                    }
                    close_popup = true;
                }
            });

            if close_popup {
                egui::Popup::close_id(ui.ctx(), popup_id);
            }

            // Root note selector
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Root:").weak());
                let note_name = midi_note_name(self.root_note);
                let root_btn = ui.button(&note_name);
                let root_popup_id = egui::Popup::default_response_id(&root_btn);
                let mut close_root = false;
                egui::Popup::from_toggle_button_response(&root_btn)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .width(80.0)
                    .show(|ui| {
                    let notes: Vec<(u8, String)> = (24..=96).rev()
                        .map(|n| (n, midi_note_name(n)))
                        .collect();
                    let items = notes.iter().map(|(n, name)| (*n == self.root_note, name.as_str()));
                    if let Some(idx) = widgets::scrollable_list(ui, 200.0, items) {
                        let (note, _) = &notes[idx];
                        user_state.pending_root_note_changes.push((node_id, backend_node_id, *note));
                        close_root = true;
                    }
                });
                if close_root {
                    egui::Popup::close_id(ui.ctx(), root_popup_id);
                }
            });
        } else if self.template == NodeTemplate::Oscilloscope {
            let size = egui::vec2(200.0, 80.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let painter = ui.painter_at(rect);

            // Background
            painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x1a));

            // Center line
            let center_y = rect.center().y;
            painter.line_segment(
                [egui::pos2(rect.left(), center_y), egui::pos2(rect.right(), center_y)],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x2a, 0x2a, 0x2a)),
            );

            if let Some(cache) = user_state.oscilloscope_data.get(&node_id) {
                // Draw audio waveform (green)
                if cache.audio.len() >= 2 {
                    let points: Vec<egui::Pos2> = cache.audio.iter().enumerate().map(|(i, &sample)| {
                        let x = rect.left() + (i as f32 / (cache.audio.len() - 1) as f32) * rect.width();
                        let y = center_y - sample.clamp(-1.0, 1.0) * (rect.height() / 2.0);
                        egui::pos2(x, y)
                    }).collect();
                    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, egui::Color32::from_rgb(0x4C, 0xAF, 0x50))));
                }

                // Draw CV waveform (orange) if present
                if cache.cv.len() >= 2 {
                    let points: Vec<egui::Pos2> = cache.cv.iter().enumerate().map(|(i, &sample)| {
                        let x = rect.left() + (i as f32 / (cache.cv.len() - 1) as f32) * rect.width();
                        let y = center_y - sample.clamp(-1.0, 1.0) * (rect.height() / 2.0);
                        egui::pos2(x, y)
                    }).collect();
                    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, egui::Color32::from_rgb(0xFF, 0x98, 0x00))));
                }
            }

            // Time window slider
            let time_ms = user_state.oscilloscope_time_scale.entry(node_id).or_insert(100.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().slider_width = 140.0;
                ui.add(egui::Slider::new(time_ms, 10.0..=1000.0)
                    .suffix(" ms")
                    .logarithmic(true));
            });
        } else {
            ui.label("");
        }
        vec![]
    }
}

// Iterator for all node templates (track-level graph)
pub struct AllNodeTemplates;

/// Iterator for subgraph node templates (includes TemplateInput/Output)
pub struct SubgraphNodeTemplates;

/// Node templates available inside a VoiceAllocator subgraph (no nested VA)
pub struct VoiceAllocatorNodeTemplates;

impl NodeTemplateIter for VoiceAllocatorNodeTemplates {
    type Item = NodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        let mut templates = AllNodeTemplates.all_kinds();
        // VA nodes can't be nested — signals inside a VA are monophonic
        templates.retain(|t| *t != NodeTemplate::VoiceAllocator);
        templates.push(NodeTemplate::TemplateInput);
        templates.push(NodeTemplate::TemplateOutput);
        templates
    }
}

impl NodeTemplateIter for SubgraphNodeTemplates {
    type Item = NodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        let mut templates = AllNodeTemplates.all_kinds();
        templates.push(NodeTemplate::TemplateInput);
        templates.push(NodeTemplate::TemplateOutput);
        templates
    }
}

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
            NodeTemplate::Echo,
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
            // Note: Group is not in the node finder — groups are created via Ctrl+G selection.
            // Note: TemplateInput/TemplateOutput are excluded from the default finder.
            // They are added dynamically when editing inside a subgraph.
            // Outputs
            NodeTemplate::AudioOutput,
        ]
    }
}
