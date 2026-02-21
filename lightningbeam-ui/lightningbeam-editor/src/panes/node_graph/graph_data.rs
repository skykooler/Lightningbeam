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

/// Macro that defines `NodeTemplate` enum and generates metadata methods from a single table.
///
/// Each row: `variant, backend_name, display_label, category, in_finder;`
///
/// Generated methods:
/// - `backend_type_name() -> &'static str`
/// - `display_label() -> &'static str` (used by `node_finder_label`)
/// - `category() -> &'static str` (used by `node_finder_categories`)
/// - `in_finder() -> bool`
/// - `from_backend_name(s: &str) -> Option<NodeTemplate>`
/// - `all_finder_kinds() -> Vec<NodeTemplate>` (only variants with `in_finder = true`)
macro_rules! node_templates {
    (
        $( $variant:ident, $backend:literal, $label:literal, $category:literal, $in_finder:literal );+
        $(;)?
    ) => {
        /// Node templates - types of nodes that can be created
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum NodeTemplate {
            $($variant),+
        }

        impl NodeTemplate {
            /// Returns the backend-compatible type name string (matches daw-backend match arms)
            pub fn backend_type_name(&self) -> &'static str {
                match self {
                    $(NodeTemplate::$variant => $backend),+
                }
            }

            /// Display label for the node finder
            fn display_label(&self) -> &'static str {
                match self {
                    $(NodeTemplate::$variant => $label),+
                }
            }

            /// Category for the node finder
            fn category(&self) -> &'static str {
                match self {
                    $(NodeTemplate::$variant => $category),+
                }
            }

            /// Whether this node appears in the node finder
            #[allow(dead_code)]
            fn in_finder(&self) -> bool {
                match self {
                    $(NodeTemplate::$variant => $in_finder),+
                }
            }

            /// Map a backend type name string to a NodeTemplate variant.
            ///
            /// Handles canonical names from the table plus legacy aliases.
            pub fn from_backend_name(s: &str) -> Option<NodeTemplate> {
                match s {
                    $($backend => Some(NodeTemplate::$variant),)+
                    // Legacy / alternate aliases
                    "Delay" => Some(NodeTemplate::Echo),
                    "BPMDetector" => Some(NodeTemplate::BpmDetector),
                    _ => None,
                }
            }

            /// All node templates that should appear in the default node finder
            pub fn all_finder_kinds() -> Vec<NodeTemplate> {
                let mut v = Vec::new();
                $(if $in_finder { v.push(NodeTemplate::$variant); })+
                v
            }
        }
    };
}

node_templates! {
    // Inputs
    MidiInput,          "MidiInput",          "MIDI Input",          "Inputs",       true;
    AudioInput,         "AudioInput",         "Audio Input",         "Inputs",       true;
    AutomationInput,    "AutomationInput",    "Automation Input",    "Inputs",       true;
    Beat,               "Beat",               "Beat",                "Inputs",       true;
    // Generators
    Oscillator,         "Oscillator",         "Oscillator",          "Generators",   true;
    WavetableOscillator,"WavetableOscillator","Wavetable Oscillator","Generators",   true;
    FmSynth,            "FMSynth",            "FM Synth",            "Generators",   true;
    Noise,              "NoiseGenerator",     "Noise Generator",     "Generators",   true;
    SimpleSampler,      "SimpleSampler",      "Simple Sampler",      "Generators",   true;
    MultiSampler,       "MultiSampler",       "Multi Sampler",       "Generators",   true;
    // Effects
    Filter,             "Filter",             "Filter",              "Effects",      true;
    Svf,                "SVF",                "SVF",                 "Effects",      true;
    Gain,               "Gain",               "Gain",                "Effects",      true;
    Echo,               "Echo",               "Echo",                "Effects",      true;
    Reverb,             "Reverb",             "Reverb",              "Effects",      true;
    Chorus,             "Chorus",             "Chorus",              "Effects",      true;
    Flanger,            "Flanger",            "Flanger",             "Effects",      true;
    Phaser,             "Phaser",             "Phaser",              "Effects",      true;
    Distortion,         "Distortion",         "Distortion",          "Effects",      true;
    AmpSim,             "AmpSim",             "Amp Sim",             "Effects",      true;
    BitCrusher,         "BitCrusher",         "Bit Crusher",         "Effects",      true;
    Compressor,         "Compressor",         "Compressor",          "Effects",      true;
    Limiter,            "Limiter",            "Limiter",             "Effects",      true;
    Eq,                 "EQ",                 "EQ",                  "Effects",      true;
    Pan,                "Pan",                "Pan",                 "Effects",      true;
    RingModulator,      "RingModulator",      "Ring Modulator",      "Effects",      true;
    Vocoder,            "Vocoder",            "Vocoder",             "Effects",      true;
    // Utilities
    Adsr,               "ADSR",               "ADSR Envelope",       "Utilities",    true;
    Lfo,                "LFO",                "LFO",                 "Utilities",    true;
    Mixer,              "Mixer",              "Mixer",               "Utilities",    true;
    Splitter,           "Splitter",           "Splitter",            "Utilities",    true;
    Constant,           "Constant",           "Constant",            "Utilities",    true;
    MidiToCv,           "MidiToCV",           "MIDI to CV",          "Utilities",    true;
    AudioToCv,          "AudioToCV",          "Audio to CV",         "Utilities",    true;
    Arpeggiator,        "Arpeggiator",        "Arpeggiator",         "Utilities",    true;
    Sequencer,          "Sequencer",          "Step Sequencer",      "Utilities",    true;
    Math,               "Math",               "Math",                "Utilities",    true;
    SampleHold,         "SampleHold",         "Sample & Hold",       "Utilities",    true;
    SlewLimiter,        "SlewLimiter",        "Slew Limiter",        "Utilities",    true;
    Quantizer,          "Quantizer",          "Quantizer",           "Utilities",    true;
    EnvelopeFollower,   "EnvelopeFollower",   "Envelope Follower",   "Utilities",    true;
    BpmDetector,        "BpmDetector",        "BPM Detector",        "Utilities",    true;
    Mod,                "Mod",                "Modulator",           "Utilities",    true;
    // Analysis
    Oscilloscope,       "Oscilloscope",       "Oscilloscope",        "Analysis",     true;
    // Advanced
    VoiceAllocator,     "VoiceAllocator",     "Voice Allocator",     "Advanced",     true;
    Script,             "Script",             "Script",              "Advanced",     true;
    Group,              "Group",              "Group",               "Advanced",     false;
    // Subgraph I/O
    TemplateInput,      "TemplateInput",      "Template Input",      "Subgraph I/O", false;
    TemplateOutput,     "TemplateOutput",     "Template Output",     "Subgraph I/O", false;
    // Outputs
    AudioOutput,        "AudioOutput",        "Audio Output",        "Outputs",      true;
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
    /// BeamDSP script asset ID (for Script nodes — references a ScriptDefinition in the document)
    #[serde(default)]
    pub script_id: Option<uuid::Uuid>,
    /// Declarative UI from compiled BeamDSP script (for rendering sample pickers, groups)
    #[serde(skip)]
    pub ui_declaration: Option<beamdsp::UiDeclaration>,
    /// Sample slot names from compiled script (index → name, for sample picker mapping)
    #[serde(skip)]
    pub sample_slot_names: Vec<String>,
    /// Display names of loaded samples per slot (slot_index → display name)
    #[serde(skip)]
    pub script_sample_names: HashMap<usize, String>,
    /// Display name of loaded NAM model (for AmpSim nodes)
    #[serde(default)]
    pub nam_model_name: Option<String>,
}

fn default_root_note() -> u8 { 69 }

impl NodeData {
    pub fn new(template: NodeTemplate) -> Self {
        Self {
            template,
            sample_display_name: None,
            root_note: 69,
            script_id: None,
            ui_declaration: None,
            sample_slot_names: Vec::new(),
            script_sample_names: HashMap::new(),
            nam_model_name: None,
        }
    }
}

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

/// Pending script sample load request from bottom_ui(), handled by the node graph pane
pub enum PendingScriptSampleLoad {
    /// Load from audio pool into a script sample slot
    FromPool { node_id: NodeId, backend_node_id: u32, slot_index: usize, pool_index: usize, name: String },
    /// Open file dialog to load into a script sample slot
    FromFile { node_id: NodeId, backend_node_id: u32, slot_index: usize },
}

/// Info about an available NAM model for amp sim selection
pub struct NamModelInfo {
    pub name: String,
    pub path: String,
    pub is_bundled: bool,
}

/// Pending AmpSim model load request from bottom_ui(), handled by the node graph pane
pub enum PendingAmpSimLoad {
    /// Load a known model by path (from bundled list or previously loaded)
    FromPath { node_id: NodeId, backend_node_id: u32, path: String, name: String },
    /// Open file dialog to browse for a .nam file
    FromFile { node_id: NodeId, backend_node_id: u32 },
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
    /// Open a folder dialog for batch import with heuristic mapping
    MultiFromFolderFilesystem { node_id: NodeId, backend_node_id: u32 },
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
    /// Pending sequencer grid changes from bottom_ui (node_id, param_id, new_bitmask_value)
    pub pending_sequencer_changes: Vec<(NodeId, u32, f32)>,
    /// Time scale per oscilloscope node (in milliseconds)
    pub oscilloscope_time_scale: HashMap<NodeId, f32>,
    /// Available scripts for Script node dropdown, populated before draw
    pub available_scripts: Vec<(uuid::Uuid, String)>,
    /// Pending script assignment from dropdown (node_id, script_id)
    pub pending_script_assignment: Option<(NodeId, uuid::Uuid)>,
    /// Pending "New script..." from dropdown (node_id) — create new script and open in editor
    pub pending_new_script: Option<NodeId>,
    /// Pending "Load from file..." from dropdown (node_id) — open file dialog for .bdsp
    pub pending_load_script_file: Option<NodeId>,
    /// Pending script sample load request from bottom_ui sample picker
    pub pending_script_sample_load: Option<PendingScriptSampleLoad>,
    /// Draw VMs for canvas rendering, keyed by node ID
    pub draw_vms: HashMap<NodeId, beamdsp::DrawVM>,
    /// Pending param changes from draw block (node_id, param_index, new_value)
    pub pending_draw_param_changes: Vec<(NodeId, u32, f32)>,
    /// Active sample import dialog (folder import with heuristic mapping)
    pub sample_import_dialog: Option<crate::sample_import_dialog::SampleImportDialog>,
    /// Pending AmpSim model load — triggers file dialog or direct load
    pub pending_amp_sim_load: Option<PendingAmpSimLoad>,
    /// Available NAM models for amp sim selection, populated before draw
    pub available_nam_models: Vec<NamModelInfo>,
    /// Search text for the NAM model picker popup
    pub nam_search_text: String,
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
            pending_sequencer_changes: Vec::new(),
            oscilloscope_time_scale: HashMap::new(),
            available_scripts: Vec::new(),
            pending_script_assignment: None,
            pending_new_script: None,
            pending_load_script_file: None,
            pending_script_sample_load: None,
            draw_vms: HashMap::new(),
            pending_draw_param_changes: Vec::new(),
            sample_import_dialog: None,
            pending_amp_sim_load: None,
            available_nam_models: Vec::new(),
            nam_search_text: String::new(),
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
        self.display_label().into()
    }

    fn node_finder_categories(&self, _user_state: &mut Self::UserState) -> Vec<&'static str> {
        vec![self.category()]
    }

    fn node_graph_label(&self, user_state: &mut Self::UserState) -> String {
        self.node_finder_label(user_state).into()
    }

    fn user_data(&self, _user_state: &mut Self::UserState) -> Self::NodeData {
        NodeData::new(*self)
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
            NodeTemplate::Svf => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Cutoff CV".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Resonance CV".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                // Parameters
                graph.add_input_param(node_id, "Cutoff".into(), DataType::CV,
                    ValueType::float_param(1000.0, 20.0, 20000.0, " Hz", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Resonance".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "Lowpass".into(), DataType::Audio);
                graph.add_output_param(node_id, "Highpass".into(), DataType::Audio);
                graph.add_output_param(node_id, "Bandpass".into(), DataType::Audio);
                graph.add_output_param(node_id, "Notch".into(), DataType::Audio);
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
                graph.add_input_param(node_id, "Gain".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 2.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Attack".into(), DataType::CV,
                    ValueType::float_param(0.01, 0.001, 1.0, " s", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Release".into(), DataType::CV,
                    ValueType::float_param(0.1, 0.01, 5.0, " s", 2, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Transpose".into(), DataType::CV,
                    ValueType::float_param(0.0, -24.0, 24.0, " st", 3, None), InputParamKind::ConstantOnly, true);
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
            NodeTemplate::AmpSim => {
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Input Gain".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 4.0, "", 0, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Output Gain".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 4.0, "", 1, None), InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Mix".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 2, None), InputParamKind::ConstantOnly, true);
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
            NodeTemplate::Arpeggiator => {
                graph.add_input_param(node_id, "MIDI In".into(), DataType::Midi, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Phase".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Mode".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 0,
                        Some(&["One/Cycle", "All/Cycle"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Direction".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 1,
                        Some(&["Up", "Down", "Up/Down", "Random"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Octaves".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 3.0, "", 2,
                        Some(&["1", "2", "3", "4"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Retrigger".into(), DataType::CV,
                    ValueType::float_param(1.0, 0.0, 1.0, "", 3,
                        Some(&["Off", "On"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "V/Oct".into(), DataType::CV);
                graph.add_output_param(node_id, "Gate".into(), DataType::CV);
            }
            NodeTemplate::Sequencer => {
                graph.add_input_param(node_id, "Phase".into(), DataType::CV, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
                graph.add_input_param(node_id, "Mode".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 0,
                        Some(&["One/Cycle", "All/Cycle"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Steps".into(), DataType::CV,
                    ValueType::float_param(2.0, 0.0, 2.0, "", 1,
                        Some(&["4", "8", "16"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Scale".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 1.0, "", 2,
                        Some(&["Chromatic", "Diatonic"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Key".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 11.0, "", 3,
                        Some(&["C","C#","D","D#","E","F","F#","G","G#","A","A#","B"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Scale Type".into(), DataType::CV,
                    ValueType::float_param(0.0, 0.0, 7.0, "", 4,
                        Some(&["Major","Minor","Dorian","Mixolydian",
                                "Penta Maj","Penta Min","Blues","Harm Minor"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Octave".into(), DataType::CV,
                    ValueType::float_param(4.0, 0.0, 8.0, "", 5,
                        Some(&["0","1","2","3","4","5","6","7","8"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_input_param(node_id, "Velocity".into(), DataType::CV,
                    ValueType::float_param(100.0, 1.0, 127.0, "", 6, None),
                    InputParamKind::ConstantOnly, true);
                // Hidden row bitmask parameters (managed by grid UI)
                for row in 0..16u32 {
                    graph.add_input_param(node_id, format!("Row{}", row).into(), DataType::CV,
                        ValueType::float_param(0.0, 0.0, 65535.0, "", 7 + row, None),
                        InputParamKind::ConstantOnly, false);
                }
                graph.add_output_param(node_id, "MIDI Out".into(), DataType::Midi);
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
            NodeTemplate::Beat => {
                graph.add_input_param(node_id, "Resolution".into(), DataType::CV,
                    ValueType::float_param(2.0, 0.0, 6.0, "", 0,
                        Some(&["1/1", "1/2", "1/4", "1/8", "1/16", "1/4T", "1/8T"])),
                    InputParamKind::ConstantOnly, true);
                graph.add_output_param(node_id, "BPM".into(), DataType::CV);
                graph.add_output_param(node_id, "Beat Phase".into(), DataType::CV);
                graph.add_output_param(node_id, "Bar Phase".into(), DataType::CV);
                graph.add_output_param(node_id, "Gate".into(), DataType::CV);
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
            NodeTemplate::Script => {
                // Default Script node: single audio in/out
                // Ports will be rebuilt when a script is compiled
                graph.add_input_param(node_id, "Audio In".into(), DataType::Audio, ValueType::float(0.0), InputParamKind::ConnectionOnly, true);
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
                if is_multi {
                    if ui.button("Import Folder...").clicked() {
                        user_state.pending_sampler_load = Some(PendingSamplerLoad::MultiFromFolderFilesystem {
                            node_id,
                            backend_node_id,
                        });
                        close_popup = true;
                    }
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
        } else if self.template == NodeTemplate::Sequencer {
            // Read grid parameters from graph inputs
            let node = &_graph[node_id];

            let num_steps = {
                let v = node.get_input("Steps").ok()
                    .and_then(|id| if let ValueType::Float { value, .. } = &_graph.get_input(id).value { Some(*value) } else { None })
                    .unwrap_or(2.0);
                match v.round() as i32 { 0 => 4usize, 1 => 8, _ => 16 }
            };
            let scale_mode_val = node.get_input("Scale").ok()
                .and_then(|id| if let ValueType::Float { value, .. } = &_graph.get_input(id).value { Some(*value) } else { None })
                .unwrap_or(0.0);
            let key_val = node.get_input("Key").ok()
                .and_then(|id| if let ValueType::Float { value, .. } = &_graph.get_input(id).value { Some(*value) } else { None })
                .unwrap_or(0.0) as u8;
            let scale_type_val = node.get_input("Scale Type").ok()
                .and_then(|id| if let ValueType::Float { value, .. } = &_graph.get_input(id).value { Some(*value) } else { None })
                .unwrap_or(0.0) as usize;
            let octave_val = node.get_input("Octave").ok()
                .and_then(|id| if let ValueType::Float { value, .. } = &_graph.get_input(id).value { Some(*value) } else { None })
                .unwrap_or(4.0) as u8;
            let is_diatonic = scale_mode_val.round() as i32 >= 1;

            // Read row bitmasks
            let num_rows = 8usize;
            let mut row_patterns = [0u16; 16];
            for row in 0..num_rows {
                let name = format!("Row{}", row);
                if let Ok(input_id) = node.get_input(&name) {
                    if let ValueType::Float { value, .. } = &_graph.get_input(input_id).value {
                        row_patterns[row] = value.round() as u16;
                    }
                }
            }

            // Scale intervals for diatonic mode
            const SCALES: &[&[u8]] = &[
                &[0,2,4,5,7,9,11], &[0,2,3,5,7,8,10], &[0,2,3,5,7,9,10], &[0,2,4,5,7,9,10],
                &[0,2,4,7,9], &[0,3,5,7,10], &[0,3,5,6,7,10], &[0,2,3,5,7,8,11],
            ];
            let row_to_note_name = |row: usize| -> String {
                let base = key_val as u16 + octave_val as u16 * 12;
                let midi_note = if is_diatonic {
                    let scale = SCALES[scale_type_val.min(SCALES.len() - 1)];
                    let oct_off = row / scale.len();
                    let degree = row % scale.len();
                    base + oct_off as u16 * 12 + scale[degree] as u16
                } else {
                    base + row as u16
                };
                let midi_note = (midi_note as u8).min(127);
                let name = NOTE_NAMES[(midi_note % 12) as usize];
                let oct = midi_note / 12;
                format!("{}{}", name, oct)
            };

            // Grid layout
            let label_width = 28.0f32;
            let cell_size = 14.0f32;
            let grid_width = num_steps as f32 * cell_size;
            let grid_height = num_rows as f32 * cell_size;
            let total_width = label_width + grid_width;
            let total_height = grid_height;

            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(total_width, total_height),
                egui::Sense::click(),
            );
            let painter = ui.painter_at(rect);

            let grid_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + label_width, rect.top()),
                egui::vec2(grid_width, grid_height),
            );

            // Background
            painter.rect_filled(grid_rect, 0.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x1a));

            // Draw cells (bottom row = row 0 = lowest pitch)
            let active_color = egui::Color32::from_rgb(0x4C, 0xAF, 0x50);
            let hover_color = egui::Color32::from_rgb(0x66, 0xBB, 0x6A);
            let grid_line_color = egui::Color32::from_rgb(0x33, 0x33, 0x33);

            // Get hover position
            let hover_cell = response.hover_pos().and_then(|pos| {
                if grid_rect.contains(pos) {
                    let col = ((pos.x - grid_rect.left()) / cell_size).floor() as usize;
                    let visual_row = ((pos.y - grid_rect.top()) / cell_size).floor() as usize;
                    if col < num_steps && visual_row < num_rows {
                        Some((num_rows - 1 - visual_row, col))
                    } else {
                        None
                    }
                } else {
                    None
                }
            });

            for visual_row in 0..num_rows {
                let row = num_rows - 1 - visual_row; // flip: top = highest pitch
                for col in 0..num_steps {
                    let cell_rect = egui::Rect::from_min_size(
                        egui::pos2(
                            grid_rect.left() + col as f32 * cell_size,
                            grid_rect.top() + visual_row as f32 * cell_size,
                        ),
                        egui::vec2(cell_size, cell_size),
                    );
                    let active = row_patterns[row] & (1 << col) != 0;
                    let hovered = hover_cell == Some((row, col));

                    if active {
                        let color = if hovered { hover_color } else { active_color };
                        painter.rect_filled(cell_rect.shrink(0.5), 1.0, color);
                    } else if hovered {
                        painter.rect_filled(cell_rect.shrink(0.5), 1.0, egui::Color32::from_rgb(0x2a, 0x2a, 0x2a));
                    }
                }
            }

            // Grid lines
            for i in 0..=num_steps {
                let x = grid_rect.left() + i as f32 * cell_size;
                let color = if i % 4 == 0 { egui::Color32::from_rgb(0x55, 0x55, 0x55) } else { grid_line_color };
                painter.line_segment(
                    [egui::pos2(x, grid_rect.top()), egui::pos2(x, grid_rect.bottom())],
                    egui::Stroke::new(1.0, color),
                );
            }
            for i in 0..=num_rows {
                let y = grid_rect.top() + i as f32 * cell_size;
                painter.line_segment(
                    [egui::pos2(grid_rect.left(), y), egui::pos2(grid_rect.right(), y)],
                    egui::Stroke::new(1.0, grid_line_color),
                );
            }

            // Note labels on the left
            for visual_row in 0..num_rows {
                let row = num_rows - 1 - visual_row;
                let label = row_to_note_name(row);
                let y = grid_rect.top() + visual_row as f32 * cell_size + cell_size * 0.5;
                painter.text(
                    egui::pos2(rect.left() + label_width - 2.0, y),
                    egui::Align2::RIGHT_CENTER,
                    &label,
                    egui::FontId::monospace(8.0),
                    egui::Color32::from_rgb(0x99, 0x99, 0x99),
                );
            }

            // Handle click to toggle cell
            if response.clicked() {
                if let Some((row, col)) = hover_cell {
                    let new_bitmask = row_patterns[row] ^ (1 << col);
                    let param_id = 7 + row as u32;
                    user_state.pending_sequencer_changes.push((node_id, param_id, new_bitmask as f32));
                }
            }
        } else if self.template == NodeTemplate::Script {
            let current_name = self.script_id
                .and_then(|id| user_state.available_scripts.iter().find(|(sid, _)| *sid == id))
                .map(|(_, name)| name.as_str())
                .unwrap_or("No script");

            let button = ui.button(current_name);
            let popup_id = egui::Popup::default_response_id(&button);
            let mut close_popup = false;

            egui::Popup::from_toggle_button_response(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(160.0)
                .show(|ui| {
                    if widgets::list_item(ui, false, "New script...") {
                        user_state.pending_new_script = Some(node_id);
                        close_popup = true;
                    }
                    if widgets::list_item(ui, false, "Load from file...") {
                        user_state.pending_load_script_file = Some(node_id);
                        close_popup = true;
                    }
                    if !user_state.available_scripts.is_empty() {
                        ui.separator();
                    }
                    for (script_id, script_name) in &user_state.available_scripts {
                        let selected = self.script_id == Some(*script_id);
                        if widgets::list_item(ui, selected, script_name) {
                            user_state.pending_script_assignment = Some((node_id, *script_id));
                            close_popup = true;
                        }
                    }
                });

            if close_popup {
                egui::Popup::close_id(ui.ctx(), popup_id);
            }

            // Sync param values from node input ports to draw VM
            if let Some(draw_vm) = user_state.draw_vms.get_mut(&node_id) {
                if let Some(node) = _graph.nodes.get(node_id) {
                    for (_name, input_id) in &node.inputs {
                        if let ValueType::Float { value, backend_param_id: Some(pid), .. } = &_graph.get_input(*input_id).value {
                            let idx = *pid as usize;
                            let params = draw_vm.params_mut();
                            if idx < params.len() {
                                params[idx] = *value;
                            }
                        }
                    }
                }
            }

            // Render declarative UI elements (sample pickers, groups, canvas)
            if let Some(ref ui_decl) = self.ui_declaration {
                let backend_node_id = user_state.node_backend_ids.get(&node_id).copied().unwrap_or(0);
                render_script_ui_elements(
                    ui, node_id, backend_node_id,
                    &ui_decl.elements,
                    &self.sample_slot_names,
                    &self.script_sample_names,
                    &user_state.available_clips,
                    &mut user_state.sampler_search_text,
                    &mut user_state.pending_script_sample_load,
                    &mut user_state.draw_vms,
                    &mut user_state.pending_draw_param_changes,
                );
            }
        } else if self.template == NodeTemplate::AmpSim {
            let backend_node_id = user_state.node_backend_ids.get(&node_id).copied().unwrap_or(0);
            let button_text = self.nam_model_name.as_deref().unwrap_or("Select Model...");

            let button = ui.button(button_text);
            if button.clicked() {
                user_state.nam_search_text.clear();
            }
            let popup_id = egui::Popup::default_response_id(&button);

            let mut close_popup = false;
            egui::Popup::from_toggle_button_response(&button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(200.0)
                .show(|ui| {
                let search_width = ui.available_width();
                ui.add_sized([search_width, 0.0], egui::TextEdit::singleline(&mut user_state.nam_search_text).hint_text("Search..."));
                ui.separator();
                let search = user_state.nam_search_text.to_lowercase();

                let bundled: Vec<&NamModelInfo> = user_state.available_nam_models.iter()
                    .filter(|m| m.is_bundled && (search.is_empty() || m.name.to_lowercase().contains(&search)))
                    .collect();
                let user_models: Vec<&NamModelInfo> = user_state.available_nam_models.iter()
                    .filter(|m| !m.is_bundled && (search.is_empty() || m.name.to_lowercase().contains(&search)))
                    .collect();

                if !bundled.is_empty() {
                    ui.label(egui::RichText::new("Bundled").small().weak());
                    let items = bundled.iter().map(|m| {
                        let selected = self.nam_model_name.as_deref() == Some(m.name.as_str());
                        (selected, m.name.as_str())
                    });
                    if let Some(idx) = widgets::scrollable_list(ui, 200.0, items) {
                        let model = bundled[idx];
                        user_state.pending_amp_sim_load = Some(PendingAmpSimLoad::FromPath {
                            node_id, backend_node_id,
                            path: model.path.clone(),
                            name: model.name.clone(),
                        });
                        close_popup = true;
                    }
                }

                if !user_models.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new("User").small().weak());
                    let items = user_models.iter().map(|m| {
                        let selected = self.nam_model_name.as_deref() == Some(m.name.as_str());
                        (selected, m.name.as_str())
                    });
                    if let Some(idx) = widgets::scrollable_list(ui, 200.0, items) {
                        let model = user_models[idx];
                        user_state.pending_amp_sim_load = Some(PendingAmpSimLoad::FromPath {
                            node_id, backend_node_id,
                            path: model.path.clone(),
                            name: model.name.clone(),
                        });
                        close_popup = true;
                    }
                }

                ui.separator();
                if ui.button("Open...").clicked() {
                    user_state.pending_amp_sim_load = Some(PendingAmpSimLoad::FromFile {
                        node_id, backend_node_id,
                    });
                    close_popup = true;
                }
            });

            if close_popup {
                egui::Popup::close_id(ui.ctx(), popup_id);
            }
        } else {
            ui.label("");
        }
        vec![]
    }
}

/// Convert a u32 RGBA color to egui Color32
fn color_from_u32(c: u32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        ((c >> 24) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

/// Render UiDeclaration elements for Script nodes (sample pickers, groups, canvas, spacers)
fn render_script_ui_elements(
    ui: &mut egui::Ui,
    node_id: NodeId,
    backend_node_id: u32,
    elements: &[beamdsp::UiElement],
    sample_slot_names: &[String],
    script_sample_names: &HashMap<usize, String>,
    available_clips: &[SamplerClipInfo],
    search_text: &mut String,
    pending_load: &mut Option<PendingScriptSampleLoad>,
    draw_vms: &mut HashMap<NodeId, beamdsp::DrawVM>,
    pending_param_changes: &mut Vec<(NodeId, u32, f32)>,
) {
    for element in elements {
        match element {
            beamdsp::UiElement::Canvas { width, height } => {
                let size = egui::vec2(*width, *height);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                let painter = ui.painter_at(rect);

                // Dark background
                painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x1a));

                if let Some(draw_vm) = draw_vms.get_mut(&node_id) {
                    // Set mouse state
                    if let Some(pos) = response.hover_pos() {
                        draw_vm.mouse.x = pos.x - rect.left();
                        draw_vm.mouse.y = pos.y - rect.top();
                    }
                    draw_vm.mouse.down = response.dragged() || response.drag_started();

                    // Save params before execution to detect changes
                    let params_before: Vec<f32> = draw_vm.params().to_vec();

                    // Execute draw block
                    if let Err(e) = draw_vm.execute() {
                        painter.text(
                            rect.center(), egui::Align2::CENTER_CENTER,
                            &format!("draw error: {}", e),
                            egui::FontId::monospace(9.0), egui::Color32::RED,
                        );
                    } else {
                        // Render draw commands
                        for cmd in &draw_vm.draw_commands {
                            match cmd {
                                beamdsp::DrawCommand::FillCircle { cx, cy, r, color } => {
                                    painter.circle_filled(
                                        egui::pos2(rect.left() + cx, rect.top() + cy),
                                        *r, color_from_u32(*color),
                                    );
                                }
                                beamdsp::DrawCommand::StrokeCircle { cx, cy, r, color, width } => {
                                    painter.circle_stroke(
                                        egui::pos2(rect.left() + cx, rect.top() + cy),
                                        *r, egui::Stroke::new(*width, color_from_u32(*color)),
                                    );
                                }
                                beamdsp::DrawCommand::StrokeArc { cx, cy, r, start_deg, end_deg, color, width } => {
                                    // Generate arc as polyline
                                    let center = egui::pos2(rect.left() + cx, rect.top() + cy);
                                    let start_rad = start_deg.to_radians();
                                    let end_rad = end_deg.to_radians();
                                    let arc_len = (end_rad - start_rad).abs();
                                    let segments = ((arc_len * *r / 2.0).ceil() as usize).max(8).min(128);
                                    let points: Vec<egui::Pos2> = (0..=segments)
                                        .map(|i| {
                                            let t = i as f32 / segments as f32;
                                            let angle = start_rad + (end_rad - start_rad) * t;
                                            egui::pos2(
                                                center.x + angle.cos() * r,
                                                center.y + angle.sin() * r,
                                            )
                                        })
                                        .collect();
                                    painter.add(egui::Shape::line(
                                        points,
                                        egui::Stroke::new(*width, color_from_u32(*color)),
                                    ));
                                }
                                beamdsp::DrawCommand::Line { x1, y1, x2, y2, color, width } => {
                                    painter.line_segment(
                                        [
                                            egui::pos2(rect.left() + x1, rect.top() + y1),
                                            egui::pos2(rect.left() + x2, rect.top() + y2),
                                        ],
                                        egui::Stroke::new(*width, color_from_u32(*color)),
                                    );
                                }
                                beamdsp::DrawCommand::FillRect { x, y, w, h, color } => {
                                    painter.rect_filled(
                                        egui::Rect::from_min_size(
                                            egui::pos2(rect.left() + x, rect.top() + y),
                                            egui::vec2(*w, *h),
                                        ),
                                        0.0, color_from_u32(*color),
                                    );
                                }
                                beamdsp::DrawCommand::StrokeRect { x, y, w, h, color, width } => {
                                    painter.rect_stroke(
                                        egui::Rect::from_min_size(
                                            egui::pos2(rect.left() + x, rect.top() + y),
                                            egui::vec2(*w, *h),
                                        ),
                                        0.0,
                                        egui::Stroke::new(*width, color_from_u32(*color)),
                                        egui::StrokeKind::Outside,
                                    );
                                }
                            }
                        }
                    }

                    // Detect param changes from draw block (e.g. knob drag)
                    for (i, (&before, &after)) in params_before.iter().zip(draw_vm.params().iter()).enumerate() {
                        if (after - before).abs() > 1e-10 {
                            pending_param_changes.push((node_id, i as u32, after));
                        }
                    }

                    // Request repaint while interacting
                    if draw_vm.mouse.down || response.hovered() {
                        ui.ctx().request_repaint();
                    }
                }
            }
            beamdsp::UiElement::Sample(slot_name) => {
                // Find the slot index by name
                let slot_index = sample_slot_names.iter().position(|n| n == slot_name);
                let display = script_sample_names
                    .get(&slot_index.unwrap_or(usize::MAX))
                    .map(|s| s.as_str())
                    .unwrap_or("No sample");

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(slot_name).weak());
                    let button = ui.button(display);
                    if let Some(slot_idx) = slot_index {
                        let popup_id = egui::Popup::default_response_id(&button);
                        let mut close = false;
                        egui::Popup::from_toggle_button_response(&button)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .width(160.0)
                            .show(|ui| {
                                let search = search_text.to_lowercase();
                                let filtered: Vec<&SamplerClipInfo> = available_clips.iter()
                                    .filter(|c| search.is_empty() || c.name.to_lowercase().contains(&search))
                                    .collect();
                                let items = filtered.iter().map(|c| (false, c.name.as_str()));
                                if let Some(idx) = widgets::scrollable_list(ui, 200.0, items) {
                                    let clip = filtered[idx];
                                    *pending_load = Some(PendingScriptSampleLoad::FromPool {
                                        node_id,
                                        backend_node_id,
                                        slot_index: slot_idx,
                                        pool_index: clip.pool_index,
                                        name: clip.name.clone(),
                                    });
                                    close = true;
                                }
                                ui.separator();
                                if ui.button("Open...").clicked() {
                                    *pending_load = Some(PendingScriptSampleLoad::FromFile {
                                        node_id,
                                        backend_node_id,
                                        slot_index: slot_idx,
                                    });
                                    close = true;
                                }
                            });
                        if close {
                            egui::Popup::close_id(ui.ctx(), popup_id);
                        }
                    }
                });
            }
            beamdsp::UiElement::Group { label, children } => {
                egui::CollapsingHeader::new(egui::RichText::new(label).weak())
                    .default_open(true)
                    .show(ui, |ui| {
                        render_script_ui_elements(
                            ui, node_id, backend_node_id,
                            children, sample_slot_names, script_sample_names,
                            available_clips, search_text, pending_load,
                            draw_vms, pending_param_changes,
                        );
                    });
            }
            beamdsp::UiElement::Spacer(height) => {
                ui.add_space(*height);
            }
            beamdsp::UiElement::Param(_) => {
                // Params are handled as inline input ports
            }
        }
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
        let mut templates = NodeTemplate::all_finder_kinds();
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
        let mut templates = NodeTemplate::all_finder_kinds();
        templates.push(NodeTemplate::TemplateInput);
        templates.push(NodeTemplate::TemplateOutput);
        templates
    }
}

impl NodeTemplateIter for AllNodeTemplates {
    type Item = NodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        NodeTemplate::all_finder_kinds()
    }
}
