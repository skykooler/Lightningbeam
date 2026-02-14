//! Audio Graph Backend Implementation
//!
//! Wraps daw_backend's EngineController for audio node graph operations

use super::backend::{BackendNodeId, GraphBackend, GraphState};
use daw_backend::EngineController;
use petgraph::stable_graph::NodeIndex;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Audio graph backend wrapping daw_backend
pub struct AudioGraphBackend {
    /// Track ID this graph belongs to
    track_id: u32,

    /// Shared audio controller (thread-safe)
    audio_controller: Arc<Mutex<EngineController>>,

    /// Maps backend NodeIndex to stable IDs for round-trip serialization
    _node_index_to_stable: HashMap<NodeIndex, u32>,
    _next_stable_id: u32,
}

impl AudioGraphBackend {
    pub fn new(track_id: u32, audio_controller: Arc<Mutex<EngineController>>) -> Self {
        Self {
            track_id,
            audio_controller,
            _node_index_to_stable: HashMap::new(),
            _next_stable_id: 0,
        }
    }
}

impl GraphBackend for AudioGraphBackend {
    fn add_node(&mut self, node_type: &str, x: f32, y: f32) -> Result<BackendNodeId, String> {
        // TODO: Call EngineController.graph_add_node_sync() once implemented
        // For now, return placeholder
        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_add_node(self.track_id, node_type.to_string(), x, y);

        // Generate placeholder node ID
        // This will be replaced with actual backend NodeIndex from sync query
        let stable_id = self._next_stable_id;
        self._next_stable_id += 1;

        // Placeholder: use stable_id as backend index (will be wrong, but compiles)
        let node_idx = NodeIndex::new(stable_id as usize);
        self._node_index_to_stable.insert(node_idx, stable_id);

        Ok(BackendNodeId::Audio(node_idx))
    }

    fn remove_node(&mut self, backend_id: BackendNodeId) -> Result<(), String> {
        let BackendNodeId::Audio(node_idx) = backend_id;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_remove_node(self.track_id, node_idx.index() as u32);

        self._node_index_to_stable.remove(&node_idx);

        Ok(())
    }

    fn connect(
        &mut self,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String> {
        let BackendNodeId::Audio(from_idx) = output_node;
        let BackendNodeId::Audio(to_idx) = input_node;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_connect(
            self.track_id,
            from_idx.index() as u32,
            output_port,
            to_idx.index() as u32,
            input_port,
        );

        Ok(())
    }

    fn disconnect(
        &mut self,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String> {
        let BackendNodeId::Audio(from_idx) = output_node;
        let BackendNodeId::Audio(to_idx) = input_node;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_disconnect(
            self.track_id,
            from_idx.index() as u32,
            output_port,
            to_idx.index() as u32,
            input_port,
        );

        Ok(())
    }

    fn set_parameter(
        &mut self,
        backend_id: BackendNodeId,
        param_id: u32,
        value: f64,
    ) -> Result<(), String> {
        let BackendNodeId::Audio(node_idx) = backend_id;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_set_parameter(
            self.track_id,
            node_idx.index() as u32,
            param_id,
            value as f32,
        );

        Ok(())
    }

    fn get_state(&self) -> Result<GraphState, String> {
        let mut controller = self.audio_controller.lock().unwrap();
        let json = controller.query_graph_state(self.track_id)?;

        // Parse the GraphPreset JSON from backend
        let preset: daw_backend::audio::node_graph::GraphPreset =
            serde_json::from_str(&json)
                .map_err(|e| format!("Failed to parse graph state: {}", e))?;

        // Convert to our GraphState format
        let nodes = preset.nodes.iter().map(|n| {
            super::backend::SerializedNode {
                id: n.id,
                node_type: n.node_type.clone(),
                position: n.position,
                parameters: n.parameters.iter().map(|(&k, &v)| (k, v as f64)).collect(),
            }
        }).collect();

        let connections = preset.connections.iter().map(|c| {
            super::backend::SerializedConnection {
                from_node: c.from_node,
                from_port: c.from_port,
                to_node: c.to_node,
                to_port: c.to_port,
            }
        }).collect();

        Ok(GraphState { nodes, connections })
    }

    fn load_state(&mut self, _state: &GraphState) -> Result<(), String> {
        // TODO: Implement graph state loading
        Ok(())
    }

    fn add_node_to_template(
        &mut self,
        voice_allocator_id: BackendNodeId,
        node_type: &str,
        x: f32,
        y: f32,
    ) -> Result<BackendNodeId, String> {
        let BackendNodeId::Audio(allocator_idx) = voice_allocator_id;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_add_node_to_template(
            self.track_id,
            allocator_idx.index() as u32,
            node_type.to_string(),
            x,
            y,
        );

        // Placeholder return
        let stable_id = self._next_stable_id;
        self._next_stable_id += 1;
        let node_idx = NodeIndex::new(stable_id as usize);

        Ok(BackendNodeId::Audio(node_idx))
    }

    fn connect_in_template(
        &mut self,
        voice_allocator_id: BackendNodeId,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String> {
        let BackendNodeId::Audio(allocator_idx) = voice_allocator_id;
        let BackendNodeId::Audio(from_idx) = output_node;
        let BackendNodeId::Audio(to_idx) = input_node;

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_connect_in_template(
            self.track_id,
            allocator_idx.index() as u32,
            from_idx.index() as u32,
            output_port,
            to_idx.index() as u32,
            input_port,
        );

        Ok(())
    }
}
