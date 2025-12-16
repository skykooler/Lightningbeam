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
    node_index_to_stable: HashMap<NodeIndex, u32>,
    next_stable_id: u32,
}

impl AudioGraphBackend {
    pub fn new(track_id: u32, audio_controller: Arc<Mutex<EngineController>>) -> Self {
        Self {
            track_id,
            audio_controller,
            node_index_to_stable: HashMap::new(),
            next_stable_id: 0,
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
        let stable_id = self.next_stable_id;
        self.next_stable_id += 1;

        // Placeholder: use stable_id as backend index (will be wrong, but compiles)
        let node_idx = NodeIndex::new(stable_id as usize);
        self.node_index_to_stable.insert(node_idx, stable_id);

        Ok(BackendNodeId::Audio(node_idx))
    }

    fn remove_node(&mut self, backend_id: BackendNodeId) -> Result<(), String> {
        let BackendNodeId::Audio(node_idx) = backend_id else {
            return Err("Invalid backend node type".to_string());
        };

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_remove_node(self.track_id, node_idx.index() as u32);

        self.node_index_to_stable.remove(&node_idx);

        Ok(())
    }

    fn connect(
        &mut self,
        output_node: BackendNodeId,
        output_port: usize,
        input_node: BackendNodeId,
        input_port: usize,
    ) -> Result<(), String> {
        let BackendNodeId::Audio(from_idx) = output_node else {
            return Err("Invalid output node type".to_string());
        };
        let BackendNodeId::Audio(to_idx) = input_node else {
            return Err("Invalid input node type".to_string());
        };

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
        let BackendNodeId::Audio(from_idx) = output_node else {
            return Err("Invalid output node type".to_string());
        };
        let BackendNodeId::Audio(to_idx) = input_node else {
            return Err("Invalid input node type".to_string());
        };

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
        let BackendNodeId::Audio(node_idx) = backend_id else {
            return Err("Invalid backend node type".to_string());
        };

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
        // TODO: Implement graph state query
        // For now, return empty state
        Ok(GraphState {
            nodes: vec![],
            connections: vec![],
        })
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
        let BackendNodeId::Audio(allocator_idx) = voice_allocator_id else {
            return Err("Invalid voice allocator node type".to_string());
        };

        let mut controller = self.audio_controller.lock().unwrap();
        controller.graph_add_node_to_template(
            self.track_id,
            allocator_idx.index() as u32,
            node_type.to_string(),
            x,
            y,
        );

        // Placeholder return
        let stable_id = self.next_stable_id;
        self.next_stable_id += 1;
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
        let BackendNodeId::Audio(allocator_idx) = voice_allocator_id else {
            return Err("Invalid voice allocator node type".to_string());
        };
        let BackendNodeId::Audio(from_idx) = output_node else {
            return Err("Invalid output node type".to_string());
        };
        let BackendNodeId::Audio(to_idx) = input_node else {
            return Err("Invalid input node type".to_string());
        };

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
