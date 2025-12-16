//! Node Graph Actions
//!
//! Implements the Action trait for node graph operations, ensuring undo/redo support

use super::backend::BackendNodeId;
use lightningbeam_core::action::{Action, BackendContext};
use lightningbeam_core::document::Document;
use uuid::Uuid;

/// Node graph action variants
///
/// Note: Node graph state is managed by the audio backend, not the document.
/// Therefore, execute() and rollback() are no-ops, and all work happens in
/// execute_backend() and rollback_backend().
pub enum NodeGraphAction {
    AddNode(AddNodeAction),
    RemoveNode(RemoveNodeAction),
    MoveNode(MoveNodeAction),
    Connect(ConnectAction),
    Disconnect(DisconnectAction),
    SetParameter(SetParameterAction),
}

impl Action for NodeGraphAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        // Node graph state is in the audio backend, not document
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        match self {
            NodeGraphAction::AddNode(a) => a.description(),
            NodeGraphAction::RemoveNode(a) => a.description(),
            NodeGraphAction::MoveNode(a) => a.description(),
            NodeGraphAction::Connect(a) => a.description(),
            NodeGraphAction::Disconnect(a) => a.description(),
            NodeGraphAction::SetParameter(a) => a.description(),
        }
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        match self {
            NodeGraphAction::AddNode(a) => a.execute_backend(backend, document),
            NodeGraphAction::RemoveNode(a) => a.execute_backend(backend, document),
            NodeGraphAction::MoveNode(a) => a.execute_backend(backend, document),
            NodeGraphAction::Connect(a) => a.execute_backend(backend, document),
            NodeGraphAction::Disconnect(a) => a.execute_backend(backend, document),
            NodeGraphAction::SetParameter(a) => a.execute_backend(backend, document),
        }
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        match self {
            NodeGraphAction::AddNode(a) => a.rollback_backend(backend, document),
            NodeGraphAction::RemoveNode(a) => a.rollback_backend(backend, document),
            NodeGraphAction::MoveNode(a) => a.rollback_backend(backend, document),
            NodeGraphAction::Connect(a) => a.rollback_backend(backend, document),
            NodeGraphAction::Disconnect(a) => a.rollback_backend(backend, document),
            NodeGraphAction::SetParameter(a) => a.rollback_backend(backend, document),
        }
    }
}

// ============================================================================
// AddNodeAction
// ============================================================================

pub struct AddNodeAction {
    /// Layer ID (maps to backend track ID)
    layer_id: Uuid,

    /// Node type to add
    node_type: String,

    /// Position in canvas coordinates
    position: (f32, f32),

    /// Backend node ID (stored after execute for rollback)
    backend_node_id: Option<BackendNodeId>,
}

impl AddNodeAction {
    pub fn new(layer_id: Uuid, node_type: String, position: (f32, f32)) -> Self {
        Self {
            layer_id,
            node_type,
            position,
            backend_node_id: None,
        }
    }

    fn description(&self) -> String {
        format!("Add {} node", self.node_type)
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        // Get graph state before adding node to see what nodes exist
        let before_json = controller.query_graph_state(*track_id)?;
        let before_state: daw_backend::audio::node_graph::GraphPreset =
            serde_json::from_str(&before_json)
                .map_err(|e| format!("Failed to parse before graph state: {}", e))?;

        // Add node to backend (using async API)
        controller.graph_add_node(*track_id, self.node_type.clone(), self.position.0, self.position.1);

        // Query graph state after to find the new node ID
        let after_json = controller.query_graph_state(*track_id)?;
        let after_state: daw_backend::audio::node_graph::GraphPreset =
            serde_json::from_str(&after_json)
                .map_err(|e| format!("Failed to parse after graph state: {}", e))?;

        // Find the new node by comparing before and after states
        // The new node should have an ID that wasn't in the before state
        let before_ids: std::collections::HashSet<_> = before_state.nodes.iter().map(|n| n.id).collect();
        let new_node = after_state.nodes.iter()
            .find(|n| !before_ids.contains(&n.id))
            .ok_or("Failed to find newly added node in graph state")?;

        // Store the backend node ID
        self.backend_node_id = Some(BackendNodeId::Audio(
            petgraph::stable_graph::NodeIndex::new(new_node.id as usize)
        ));

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        if let Some(backend_id) = self.backend_node_id {
            let controller = backend
                .audio_controller
                .as_mut()
                .ok_or("Audio controller not available")?;

            let track_id = backend
                .layer_to_track_map
                .get(&self.layer_id)
                .ok_or("Track not found")?;

            let BackendNodeId::Audio(node_idx) = backend_id;
            controller.graph_remove_node(*track_id, node_idx.index() as u32);
        }

        Ok(())
    }
}

// ============================================================================
// RemoveNodeAction
// ============================================================================

pub struct RemoveNodeAction {
    layer_id: Uuid,
    backend_node_id: BackendNodeId,

    // Store node state for undo (TODO: implement when we have graph state query)
    #[allow(dead_code)]
    node_type: Option<String>,
    #[allow(dead_code)]
    position: Option<(f32, f32)>,
}

impl RemoveNodeAction {
    pub fn new(layer_id: Uuid, backend_node_id: BackendNodeId) -> Self {
        Self {
            layer_id,
            backend_node_id,
            node_type: None,
            position: None,
        }
    }

    fn description(&self) -> String {
        "Remove node".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // TODO: Query and store node state before removing for undo

        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(node_idx) = self.backend_node_id;
        controller.graph_remove_node(*track_id, node_idx.index() as u32);

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        _backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // TODO: Re-add node using stored state
        Ok(())
    }
}

// ============================================================================
// MoveNodeAction
// ============================================================================

pub struct MoveNodeAction {
    layer_id: Uuid,
    backend_node_id: BackendNodeId,
    new_position: (f32, f32),
    old_position: Option<(f32, f32)>,
}

impl MoveNodeAction {
    pub fn new(layer_id: Uuid, backend_node_id: BackendNodeId, new_position: (f32, f32)) -> Self {
        Self {
            layer_id,
            backend_node_id,
            new_position,
            old_position: None,
        }
    }

    fn description(&self) -> String {
        "Move node".to_string()
    }

    fn execute_backend(
        &mut self,
        _backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // TODO: Query old position and call graph_move_node() when available
        Ok(())
    }

    fn rollback_backend(
        &mut self,
        _backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // TODO: Restore old position
        Ok(())
    }
}

// ============================================================================
// ConnectAction
// ============================================================================

pub struct ConnectAction {
    layer_id: Uuid,
    from_node: BackendNodeId,
    from_port: usize,
    to_node: BackendNodeId,
    to_port: usize,
}

impl ConnectAction {
    pub fn new(
        layer_id: Uuid,
        from_node: BackendNodeId,
        from_port: usize,
        to_node: BackendNodeId,
        to_port: usize,
    ) -> Self {
        Self {
            layer_id,
            from_node,
            from_port,
            to_node,
            to_port,
        }
    }

    fn description(&self) -> String {
        "Connect nodes".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(from_idx) = self.from_node;
        let BackendNodeId::Audio(to_idx) = self.to_node;

        controller.graph_connect(
            *track_id,
            from_idx.index() as u32,
            self.from_port,
            to_idx.index() as u32,
            self.to_port,
        );

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(from_idx) = self.from_node;
        let BackendNodeId::Audio(to_idx) = self.to_node;

        controller.graph_disconnect(
            *track_id,
            from_idx.index() as u32,
            self.from_port,
            to_idx.index() as u32,
            self.to_port,
        );

        Ok(())
    }
}

// ============================================================================
// DisconnectAction
// ============================================================================

pub struct DisconnectAction {
    layer_id: Uuid,
    from_node: BackendNodeId,
    from_port: usize,
    to_node: BackendNodeId,
    to_port: usize,
}

impl DisconnectAction {
    pub fn new(
        layer_id: Uuid,
        from_node: BackendNodeId,
        from_port: usize,
        to_node: BackendNodeId,
        to_port: usize,
    ) -> Self {
        Self {
            layer_id,
            from_node,
            from_port,
            to_node,
            to_port,
        }
    }

    fn description(&self) -> String {
        "Disconnect nodes".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(from_idx) = self.from_node;
        let BackendNodeId::Audio(to_idx) = self.to_node;

        controller.graph_disconnect(
            *track_id,
            from_idx.index() as u32,
            self.from_port,
            to_idx.index() as u32,
            self.to_port,
        );

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // Undo disconnect by reconnecting
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(from_idx) = self.from_node;
        let BackendNodeId::Audio(to_idx) = self.to_node;

        controller.graph_connect(
            *track_id,
            from_idx.index() as u32,
            self.from_port,
            to_idx.index() as u32,
            self.to_port,
        );

        Ok(())
    }
}

// ============================================================================
// SetParameterAction
// ============================================================================

pub struct SetParameterAction {
    layer_id: Uuid,
    backend_node_id: BackendNodeId,
    param_id: u32,
    new_value: f64,
    old_value: Option<f64>,
}

impl SetParameterAction {
    pub fn new(layer_id: Uuid, backend_node_id: BackendNodeId, param_id: u32, new_value: f64) -> Self {
        Self {
            layer_id,
            backend_node_id,
            param_id,
            new_value,
            old_value: None,
        }
    }

    fn description(&self) -> String {
        "Set parameter".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // TODO: Query and store old value before changing

        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or("Audio controller not available")?;

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or("Track not found")?;

        let BackendNodeId::Audio(node_idx) = self.backend_node_id;

        controller.graph_set_parameter(
            *track_id,
            node_idx.index() as u32,
            self.param_id,
            self.new_value as f32,
        );

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        if let Some(old_value) = self.old_value {
            let controller = backend
                .audio_controller
                .as_mut()
                .ok_or("Audio controller not available")?;

            let track_id = backend
                .layer_to_track_map
                .get(&self.layer_id)
                .ok_or("Track not found")?;

            let BackendNodeId::Audio(node_idx) = self.backend_node_id;

            controller.graph_set_parameter(
                *track_id,
                node_idx.index() as u32,
                self.param_id,
                old_value as f32,
            );
        }

        Ok(())
    }
}
