//! Graph Backend Trait
//!
//! Provides an abstraction layer for different graph backends (audio, VFX shaders, etc.)

use petgraph::stable_graph::NodeIndex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Backend node identifier (abstraction over different backend types)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendNodeId {
    Audio(NodeIndex),
    // Future: Vfx(u32),
}

impl BackendNodeId {
    /// Get the backend node index as a u32
    pub fn index(self) -> u32 {
        match self {
            BackendNodeId::Audio(idx) => idx.index() as u32,
        }
    }

}

/// Abstract backend for node graph operations
///
/// Implementations:
/// - AudioGraphBackend: Wraps daw_backend::AudioGraph via EngineController
/// - VfxGraphBackend (future): GPU-based shader graph
#[allow(dead_code)]
pub trait GraphBackend: Send {
    /// Add a node to the backend graph
    fn add_node(&mut self, node_type: &str, x: f32, y: f32) -> Result<BackendNodeId, String>;

    /// Remove a node from the backend
    fn remove_node(&mut self, backend_id: BackendNodeId) -> Result<(), String>;

    /// Connect two nodes
    fn connect(
        &mut self,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String>;

    /// Disconnect two nodes
    fn disconnect(
        &mut self,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String>;

    /// Set a node parameter
    fn set_parameter(
        &mut self,
        backend_id: BackendNodeId,
        param_id: u32,
        value: f64,
    ) -> Result<(), String>;

    /// Get current graph state (for serialization)
    fn get_state(&self) -> Result<GraphState, String>;

    /// Get current graph state as raw JSON (GraphPreset format from backend)
    fn get_state_json(&self) -> Result<String, String>;

    /// Load graph state (for presets)
    fn load_state(&mut self, state: &GraphState) -> Result<(), String>;

    /// Add node to VoiceAllocator template (Phase 2)
    fn add_node_to_template(
        &mut self,
        voice_allocator_id: BackendNodeId,
        node_type: &str,
        x: f32,
        y: f32,
    ) -> Result<BackendNodeId, String>;

    /// Connect nodes inside VoiceAllocator template (Phase 2)
    fn connect_in_template(
        &mut self,
        voice_allocator_id: BackendNodeId,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String>;

    /// Get the state of a VoiceAllocator's template graph as JSON
    fn query_template_state(&self, voice_allocator_id: u32) -> Result<String, String>;
}

/// Serializable graph state (for presets and save/load)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct GraphState {
    pub nodes: Vec<SerializedNode>,
    pub connections: Vec<SerializedConnection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SerializedNode {
    pub id: u32, // Frontend node ID (stable)
    pub node_type: String,
    pub position: (f32, f32),
    pub parameters: HashMap<u32, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct SerializedConnection {
    pub from_node: u32,
    pub from_port: usize,
    pub to_node: u32,
    pub to_port: usize,
}
