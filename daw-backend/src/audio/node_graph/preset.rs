use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::nodes::LoopMode;

/// Sample data for preset serialization
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SampleData {
    #[serde(rename = "simple_sampler")]
    SimpleSampler {
        #[serde(skip_serializing_if = "Option::is_none")]
        file_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        embedded_data: Option<EmbeddedSampleData>,
    },
    #[serde(rename = "multi_sampler")]
    MultiSampler { layers: Vec<LayerData> },
}

/// Embedded sample data (base64-encoded for JSON compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedSampleData {
    /// Base64-encoded audio samples (f32 little-endian)
    pub data_base64: String,
    /// Original sample rate
    pub sample_rate: u32,
}

/// Layer data for MultiSampler
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_data: Option<EmbeddedSampleData>,
    pub key_min: u8,
    pub key_max: u8,
    pub root_key: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_end: Option<usize>,
    #[serde(default = "default_loop_mode")]
    pub loop_mode: LoopMode,
}

fn default_loop_mode() -> LoopMode {
    LoopMode::OneShot
}

/// Serializable representation of a node graph preset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPreset {
    /// Preset metadata
    pub metadata: PresetMetadata,

    /// Nodes in the graph
    pub nodes: Vec<SerializedNode>,

    /// Connections between nodes
    pub connections: Vec<SerializedConnection>,

    /// Which node indices are MIDI targets
    pub midi_targets: Vec<u32>,

    /// Which node index is the audio output (None if not set)
    pub output_node: Option<u32>,
}

/// Metadata about the preset
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetMetadata {
    /// Preset name
    pub name: String,

    /// Description of what the preset sounds like
    #[serde(default)]
    pub description: String,

    /// Preset author
    #[serde(default)]
    pub author: String,

    /// Preset version (for compatibility)
    #[serde(default = "default_version")]
    pub version: u32,

    /// Tags for categorization (e.g., "bass", "lead", "pad")
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_version() -> u32 {
    1
}

/// Serialized node representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedNode {
    /// Unique ID (node index in the graph)
    pub id: u32,

    /// Node type (e.g., "Oscillator", "Filter", "ADSR")
    pub node_type: String,

    /// Parameter values (param_id -> value)
    pub parameters: HashMap<u32, f32>,

    /// UI position (for visual editor)
    #[serde(default)]
    pub position: (f32, f32),

    /// For VoiceAllocator nodes: the nested template graph
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_graph: Option<Box<GraphPreset>>,

    /// For sampler nodes: loaded sample data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_data: Option<SampleData>,
}

/// Serialized connection between nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedConnection {
    /// Source node ID
    pub from_node: u32,

    /// Source port index
    pub from_port: usize,

    /// Destination node ID
    pub to_node: u32,

    /// Destination port index
    pub to_port: usize,
}

impl GraphPreset {
    /// Create a new preset with the given name
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            metadata: PresetMetadata {
                name: name.into(),
                description: String::new(),
                author: String::new(),
                version: 1,
                tags: Vec::new(),
            },
            nodes: Vec::new(),
            connections: Vec::new(),
            midi_targets: Vec::new(),
            output_node: None,
        }
    }

    /// Serialize to JSON string
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Add a node to the preset
    pub fn add_node(&mut self, node: SerializedNode) {
        self.nodes.push(node);
    }

    /// Add a connection to the preset
    pub fn add_connection(&mut self, connection: SerializedConnection) {
        self.connections.push(connection);
    }
}

impl SerializedNode {
    /// Create a new serialized node
    pub fn new(id: u32, node_type: impl Into<String>) -> Self {
        Self {
            id,
            node_type: node_type.into(),
            parameters: HashMap::new(),
            position: (0.0, 0.0),
            template_graph: None,
            sample_data: None,
        }
    }

    /// Set a parameter value
    pub fn set_parameter(&mut self, param_id: u32, value: f32) {
        self.parameters.insert(param_id, value);
    }

    /// Set UI position
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.position = (x, y);
    }
}
