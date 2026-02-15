//! Node Graph Pane
//!
//! Audio/MIDI node graph editor for modular synthesis and effects processing

pub mod actions;
pub mod audio_backend;
pub mod backend;
pub mod graph_data;
pub mod node_types;

use backend::{BackendNodeId, GraphBackend};
use graph_data::{AllNodeTemplates, DataType, GraphState, NodeData, NodeTemplate, ValueType};
use super::NodePath;
use eframe::egui;
use egui_node_graph2::*;
use std::collections::HashMap;
use uuid::Uuid;

/// Node graph pane with egui_node_graph2 integration
pub struct NodeGraphPane {
    /// The graph editor state
    state: GraphEditorState<NodeData, DataType, ValueType, NodeTemplate, GraphState>,

    /// User state for the graph
    user_state: GraphState,

    /// Backend integration
    #[allow(dead_code)]
    backend: Option<Box<dyn GraphBackend>>,

    /// Maps frontend node IDs to backend node IDs
    node_id_map: HashMap<NodeId, BackendNodeId>,

    /// Maps backend node IDs to frontend node IDs (reverse mapping)
    backend_to_frontend_map: HashMap<BackendNodeId, NodeId>,

    /// Track ID this graph belongs to
    track_id: Option<Uuid>,

    /// Pending action to execute
    #[allow(dead_code)]
    pending_action: Option<Box<dyn lightningbeam_core::action::Action>>,

    /// Track newly added nodes to update ID mappings after action execution
    /// (frontend_id, node_type, position)
    pending_node_addition: Option<(NodeId, String, (f32, f32))>,

    /// Track parameter values to detect changes
    /// Maps InputId -> last known value
    parameter_values: HashMap<InputId, f32>,

    /// Last seen project generation (to detect project reloads)
    last_project_generation: u64,

    /// Node currently being dragged (for insert-on-connection-drop)
    dragging_node: Option<NodeId>,
    /// Connection that would be targeted for insertion (highlighted during drag)
    insert_target: Option<(InputId, OutputId)>,
}

impl NodeGraphPane {
    pub fn new() -> Self {
        let state = GraphEditorState::new(1.0);

        Self {
            state,
            user_state: GraphState::default(),
            backend: None,
            node_id_map: HashMap::new(),
            backend_to_frontend_map: HashMap::new(),
            track_id: None,
            pending_action: None,
            pending_node_addition: None,
            parameter_values: HashMap::new(),
            last_project_generation: 0,
            dragging_node: None,

            insert_target: None,
        }
    }

    #[allow(dead_code)]
    pub fn with_track_id(
        track_id: Uuid,
        audio_controller: std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>,
        backend_track_id: u32,
    ) -> Self {
        let backend = Box::new(audio_backend::AudioGraphBackend::new(
            backend_track_id,
            audio_controller,
        ));

        let mut pane = Self {
            state: GraphEditorState::new(1.0),
            user_state: GraphState::default(),
            backend: Some(backend),
            node_id_map: HashMap::new(),
            backend_to_frontend_map: HashMap::new(),
            track_id: Some(track_id),
            pending_action: None,
            pending_node_addition: None,
            parameter_values: HashMap::new(),
            last_project_generation: 0,
            dragging_node: None,

            insert_target: None,
        };

        // Load existing graph from backend
        if let Err(e) = pane.load_graph_from_backend() {
            eprintln!("Failed to load graph from backend: {}", e);
        }

        pane
    }

    /// Load the graph state from the backend and populate the frontend
    fn load_graph_from_backend(&mut self) -> Result<(), String> {
        let graph_state = if let Some(backend) = &self.backend {
            backend.get_state()?
        } else {
            return Err("No backend available".to_string());
        };

        // Clear existing graph
        self.state.graph.nodes.clear();
        self.state.graph.inputs.clear();
        self.state.graph.outputs.clear();
        self.state.graph.connections.clear();
        self.state.node_order.clear();
        self.state.node_positions.clear();
        self.state.selected_nodes.clear();
        self.state.connection_in_progress = None;
        self.state.ongoing_box_selection = None;
        self.node_id_map.clear();
        self.backend_to_frontend_map.clear();

        // Create nodes in frontend
        for node in &graph_state.nodes {
            // Parse node type from string (e.g., "Oscillator" -> NodeTemplate::Oscillator)
            let node_template = match node.node_type.as_str() {
                // Inputs
                "MidiInput" => graph_data::NodeTemplate::MidiInput,
                "AudioInput" => graph_data::NodeTemplate::AudioInput,
                "AutomationInput" => graph_data::NodeTemplate::AutomationInput,
                // Generators
                "Oscillator" => graph_data::NodeTemplate::Oscillator,
                "WavetableOscillator" => graph_data::NodeTemplate::WavetableOscillator,
                "FMSynth" => graph_data::NodeTemplate::FmSynth,
                "NoiseGenerator" => graph_data::NodeTemplate::Noise,
                "SimpleSampler" => graph_data::NodeTemplate::SimpleSampler,
                "MultiSampler" => graph_data::NodeTemplate::MultiSampler,
                // Effects
                "Filter" => graph_data::NodeTemplate::Filter,
                "Gain" => graph_data::NodeTemplate::Gain,
                "Echo" | "Delay" => graph_data::NodeTemplate::Echo,
                "Reverb" => graph_data::NodeTemplate::Reverb,
                "Chorus" => graph_data::NodeTemplate::Chorus,
                "Flanger" => graph_data::NodeTemplate::Flanger,
                "Phaser" => graph_data::NodeTemplate::Phaser,
                "Distortion" => graph_data::NodeTemplate::Distortion,
                "BitCrusher" => graph_data::NodeTemplate::BitCrusher,
                "Compressor" => graph_data::NodeTemplate::Compressor,
                "Limiter" => graph_data::NodeTemplate::Limiter,
                "EQ" => graph_data::NodeTemplate::Eq,
                "Pan" => graph_data::NodeTemplate::Pan,
                "RingModulator" => graph_data::NodeTemplate::RingModulator,
                "Vocoder" => graph_data::NodeTemplate::Vocoder,
                // Utilities
                "ADSR" => graph_data::NodeTemplate::Adsr,
                "LFO" => graph_data::NodeTemplate::Lfo,
                "Mixer" => graph_data::NodeTemplate::Mixer,
                "Splitter" => graph_data::NodeTemplate::Splitter,
                "Constant" => graph_data::NodeTemplate::Constant,
                "MidiToCV" => graph_data::NodeTemplate::MidiToCv,
                "AudioToCV" => graph_data::NodeTemplate::AudioToCv,
                "Math" => graph_data::NodeTemplate::Math,
                "SampleHold" => graph_data::NodeTemplate::SampleHold,
                "SlewLimiter" => graph_data::NodeTemplate::SlewLimiter,
                "Quantizer" => graph_data::NodeTemplate::Quantizer,
                "EnvelopeFollower" => graph_data::NodeTemplate::EnvelopeFollower,
                "BPMDetector" => graph_data::NodeTemplate::BpmDetector,
                "Mod" => graph_data::NodeTemplate::Mod,
                // Analysis
                "Oscilloscope" => graph_data::NodeTemplate::Oscilloscope,
                // Advanced
                "VoiceAllocator" => graph_data::NodeTemplate::VoiceAllocator,
                // Outputs
                "AudioOutput" => graph_data::NodeTemplate::AudioOutput,
                _ => {
                    eprintln!("Unknown node type: {}", node.node_type);
                    continue;
                }
            };

            // Create node directly in the graph
            use egui_node_graph2::Node;
            let frontend_id = self.state.graph.nodes.insert(Node {
                id: egui_node_graph2::NodeId::default(), // Will be replaced by insert
                label: node.node_type.clone(),
                inputs: vec![],
                outputs: vec![],
                user_data: graph_data::NodeData { template: node_template },
            });

            // Build the node's inputs and outputs (this adds them to graph.inputs and graph.outputs)
            // build_node() automatically populates the node's inputs/outputs vectors with correct names and order
            node_template.build_node(&mut self.state.graph, &mut self.user_state, frontend_id);

            // Set position
            self.state.node_positions.insert(
                frontend_id,
                egui::pos2(node.position.0, node.position.1),
            );

            // Add to node order for rendering
            self.state.node_order.push(frontend_id);

            // Map frontend ID to backend ID
            let backend_id = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(node.id as usize));
            self.node_id_map.insert(frontend_id, backend_id);
            self.backend_to_frontend_map.insert(backend_id, frontend_id);

            // Set parameter values from backend
            if let Some(node_data) = self.state.graph.nodes.get(frontend_id) {
                let input_ids: Vec<InputId> = node_data.inputs.iter().map(|(_, id)| *id).collect();
                for input_id in input_ids {
                    if let Some(input_param) = self.state.graph.inputs.get_mut(input_id) {
                        if let ValueType::Float { value, backend_param_id: Some(pid), .. } = &mut input_param.value {
                            if let Some(&backend_value) = node.parameters.get(pid) {
                                *value = backend_value as f32;
                            }
                        }
                    }
                }
            }
        }

        // Create connections in frontend
        for conn in &graph_state.connections {
            let from_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(conn.from_node as usize));
            let to_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(conn.to_node as usize));

            if let (Some(&from_id), Some(&to_id)) = (
                self.backend_to_frontend_map.get(&from_backend),
                self.backend_to_frontend_map.get(&to_backend),
            ) {
                // Find output param on from_node
                if let Some(from_node) = self.state.graph.nodes.get(from_id) {
                    if let Some((_name, output_id)) = from_node.outputs.get(conn.from_port) {
                        // Find input param on to_node
                        if let Some(to_node) = self.state.graph.nodes.get(to_id) {
                            if let Some((_name, input_id)) = to_node.inputs.get(conn.to_port) {
                                // Check max_connections to avoid panic in egui_node_graph2 rendering
                                let max_conns = self.state.graph.inputs.get(*input_id)
                                    .and_then(|p| p.max_connections)
                                    .map(|n| n.get() as usize)
                                    .unwrap_or(usize::MAX);

                                let current_count = self.state.graph.connections.get(*input_id)
                                    .map(|c| c.len())
                                    .unwrap_or(0);

                                if current_count < max_conns {
                                    if let Some(connections) = self.state.graph.connections.get_mut(*input_id) {
                                        connections.push(*output_id);
                                    } else {
                                        self.state.graph.connections.insert(*input_id, vec![*output_id]);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn handle_graph_response(
        &mut self,
        response: egui_node_graph2::GraphResponse<
            graph_data::UserResponse,
            graph_data::NodeData,
        >,
        shared: &mut crate::panes::SharedPaneState,
        pane_rect: egui::Rect,
    ) {
        use egui_node_graph2::NodeResponse;

        for node_response in response.node_responses {
            match node_response {
                NodeResponse::CreatedNode(node_id) => {
                    // Node was created from the node finder
                    // Reposition to the center of the pane (in graph coordinates)
                    let center_graph = (pane_rect.center().to_vec2()
                        - self.state.pan_zoom.pan
                        - pane_rect.min.to_vec2())
                        / self.state.pan_zoom.zoom;
                    self.state.node_positions.insert(node_id, center_graph.to_pos2());

                    if let Some(node) = self.state.graph.nodes.get(node_id) {
                        let node_type = node.user_data.template.backend_type_name().to_string();
                        let position = (center_graph.x, center_graph.y);

                        if let Some(track_id) = self.track_id {
                            let action = Box::new(actions::NodeGraphAction::AddNode(
                                actions::AddNodeAction::new(track_id, node_type.clone(), position)
                            ));
                            self.pending_action = Some(action);
                            // Track this addition so we can update ID mappings after execution
                            self.pending_node_addition = Some((node_id, node_type, position));
                        }
                    }
                }
                NodeResponse::ConnectEventEnded { output, input, .. } => {
                    // Connection was made between output and input
                    if let Some(track_id) = self.track_id {
                        // Get the nodes that own these params
                        let from_node = self.state.graph.outputs.get(output).map(|o| o.node);
                        let to_node = self.state.graph.inputs.get(input).map(|i| i.node);

                        if let (Some(from_node_id), Some(to_node_id)) = (from_node, to_node) {
                            // Find port indices
                            let from_port = self.state.graph.nodes.get(from_node_id)
                                .and_then(|n| n.outputs.iter().position(|(_, id)| *id == output))
                                .unwrap_or(0);
                            let to_port = self.state.graph.nodes.get(to_node_id)
                                .and_then(|n| n.inputs.iter().position(|(_, id)| *id == input))
                                .unwrap_or(0);

                            // Map frontend IDs to backend IDs
                            let from_backend = self.node_id_map.get(&from_node_id);
                            let to_backend = self.node_id_map.get(&to_node_id);

                            if let (Some(&from_id), Some(&to_id)) = (from_backend, to_backend) {
                                let action = Box::new(actions::NodeGraphAction::Connect(
                                    actions::ConnectAction::new(
                                        track_id,
                                        from_id,
                                        from_port,
                                        to_id,
                                        to_port,
                                    )
                                ));
                                self.pending_action = Some(action);
                            }
                        }
                    }
                }
                NodeResponse::DisconnectEvent { output, input } => {
                    // Connection was removed
                    if let Some(track_id) = self.track_id {
                        // Get the nodes that own these params
                        let from_node = self.state.graph.outputs.get(output).map(|o| o.node);
                        let to_node = self.state.graph.inputs.get(input).map(|i| i.node);

                        if let (Some(from_node_id), Some(to_node_id)) = (from_node, to_node) {
                            // Find port indices
                            let from_port = self.state.graph.nodes.get(from_node_id)
                                .and_then(|n| n.outputs.iter().position(|(_, id)| *id == output))
                                .unwrap_or(0);
                            let to_port = self.state.graph.nodes.get(to_node_id)
                                .and_then(|n| n.inputs.iter().position(|(_, id)| *id == input))
                                .unwrap_or(0);

                            // Map frontend IDs to backend IDs
                            let from_backend = self.node_id_map.get(&from_node_id);
                            let to_backend = self.node_id_map.get(&to_node_id);

                            if let (Some(&from_id), Some(&to_id)) = (from_backend, to_backend) {
                                let action = Box::new(actions::NodeGraphAction::Disconnect(
                                    actions::DisconnectAction::new(
                                        track_id,
                                        from_id,
                                        from_port,
                                        to_id,
                                        to_port,
                                    )
                                ));
                                self.pending_action = Some(action);
                            }
                        }
                    }
                }
                NodeResponse::DeleteNodeFull { node_id, .. } => {
                    // Node was deleted
                    if let Some(track_id) = self.track_id {
                        if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                            let action = Box::new(actions::NodeGraphAction::RemoveNode(
                                actions::RemoveNodeAction::new(track_id, backend_id)
                            ));
                            self.pending_action = Some(action);

                            // Remove from ID map
                            self.node_id_map.remove(&node_id);
                            self.backend_to_frontend_map.remove(&backend_id);
                        }
                    }
                }
                NodeResponse::MoveNode { node, drag_delta: _ } => {
                    self.user_state.active_node = Some(node);
                    self.dragging_node = Some(node);

                    // Sync updated position to backend
                    if let Some(&backend_id) = self.node_id_map.get(&node) {
                        if let Some(pos) = self.state.node_positions.get(node) {
                            let node_index = match backend_id {
                                BackendNodeId::Audio(idx) => idx.index() as u32,
                            };
                            if let Some(audio_controller) = &shared.audio_controller {
                                if let Some(&backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid)) {
                                    let mut controller = audio_controller.lock().unwrap();
                                    controller.graph_set_node_position(
                                        backend_track_id,
                                        node_index,
                                        pos.x,
                                        pos.y,
                                    );
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Ignore other events (SelectNode, RaiseNode, etc.)
                }
            }
        }

        // Execute any pending action created during response handling
        self.execute_pending_action(shared);
    }

    fn execute_pending_action(&mut self, shared: &mut crate::panes::SharedPaneState) {
        // Execute pending action if any
        if let Some(action) = self.pending_action.take() {
            // Node graph actions need to update the backend, so use execute_with_backend
            if let Some(ref audio_controller) = shared.audio_controller {
                let mut controller = audio_controller.lock().unwrap();
                // Node graph actions don't use clip instances, so we use an empty map
                let mut empty_clip_map = std::collections::HashMap::new();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: shared.layer_to_track_map,
                    clip_instance_to_backend_map: &mut empty_clip_map,
                };

                if let Err(e) = shared.action_executor.execute_with_backend(action, &mut backend_context) {
                    eprintln!("Failed to execute node graph action: {}", e);
                } else {
                    // If this was a node addition, query backend to get the new node's ID
                    if let Some((frontend_id, node_type, position)) = self.pending_node_addition.take() {
                        if let Some(track_id) = self.track_id {
                            if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                // Query graph state to find the new node
                                if let Ok(json) = controller.query_graph_state(backend_track_id) {
                                    if let Ok(state) = serde_json::from_str::<daw_backend::audio::node_graph::GraphPreset>(&json) {
                                        // Find node by type and position (approximate match for position)
                                        if let Some(backend_node) = state.nodes.iter().find(|n| {
                                            n.node_type == node_type &&
                                            (n.position.0 - position.0).abs() < 1.0 &&
                                            (n.position.1 - position.1).abs() < 1.0
                                        }) {
                                            let backend_id = BackendNodeId::Audio(
                                                petgraph::stable_graph::NodeIndex::new(backend_node.id as usize)
                                            );
                                            self.node_id_map.insert(frontend_id, backend_id);
                                            self.backend_to_frontend_map.insert(backend_id, frontend_id);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                eprintln!("Cannot execute node graph action: no audio controller");
            }
        }
    }

    fn check_parameter_changes(&mut self) {
        // Check all input parameters for value changes
        let mut _checked_count = 0;
        let mut _connection_only_count = 0;
        let mut _non_float_count = 0;

        for (input_id, input_param) in &self.state.graph.inputs {
            // Only check parameters that can have constant values (not ConnectionOnly)
            if matches!(input_param.kind, InputParamKind::ConnectionOnly) {
                _connection_only_count += 1;
                continue;
            }

            // Get current value and backend param ID
            let (current_value, backend_param_id) = match &input_param.value {
                ValueType::Float { value, backend_param_id, .. } => {
                    _checked_count += 1;
                    (*value, *backend_param_id)
                },
                other => {
                    _non_float_count += 1;
                    eprintln!("[DEBUG] Non-float parameter type: {:?}", std::mem::discriminant(other));
                    continue;
                }
            };

            // Check if value has changed
            let previous_value = self.parameter_values.get(&input_id).copied();
            let has_changed = if let Some(prev) = previous_value {
                (prev - current_value).abs() > 0.0001
            } else {
                // First time seeing this parameter - don't send update, just store it
                self.parameter_values.insert(input_id, current_value);
                false
            };

            if has_changed {
                // Value has changed, create SetParameterAction
                if let Some(track_id) = self.track_id {
                    let node_id = input_param.node;

                    // Get backend node ID and use stored param ID
                    if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                        if let Some(param_id) = backend_param_id {
                            eprintln!("[DEBUG] Parameter changed: node {:?} param {} from {:?} to {}",
                                backend_id, param_id, previous_value, current_value);
                            let action = Box::new(actions::NodeGraphAction::SetParameter(
                                actions::SetParameterAction::new(
                                    track_id,
                                    backend_id,
                                    param_id,
                                    current_value as f64,
                                )
                            ));
                            self.pending_action = Some(action);
                        }
                    }
                }

                // Update stored value
                self.parameter_values.insert(input_id, current_value);
            }
        }

    }

    fn draw_dot_grid_background(
        ui: &mut egui::Ui,
        rect: egui::Rect,
        bg_color: egui::Color32,
        dot_color: egui::Color32,
        pan_zoom: &egui_node_graph2::PanZoom,
    ) {
        let painter = ui.painter();

        // Draw background
        painter.rect_filled(rect, 0.0, bg_color);

        // Draw grid dots with pan/zoom transform
        let grid_spacing = 20.0;
        let dot_radius = 1.0 * pan_zoom.zoom;

        // Get pan offset and zoom
        let pan = pan_zoom.pan;
        let zoom = pan_zoom.zoom;

        // Calculate zoom center (same as nodes - they zoom relative to viewport center)
        let half_size = rect.size() / 2.0;
        let zoom_center = rect.min.to_vec2() + half_size + pan;

        // Calculate grid bounds in graph space
        // Screen to graph: (screen_pos - zoom_center) / zoom
        let graph_min = egui::pos2(
            (rect.min.x - zoom_center.x) / zoom,
            (rect.min.y - zoom_center.y) / zoom,
        );
        let graph_max = egui::pos2(
            (rect.max.x - zoom_center.x) / zoom,
            (rect.max.y - zoom_center.y) / zoom,
        );

        let start_x = (graph_min.x / grid_spacing).floor() * grid_spacing;
        let start_y = (graph_min.y / grid_spacing).floor() * grid_spacing;

        let mut y = start_y;
        while y < graph_max.y {
            let mut x = start_x;
            while x < graph_max.x {
                // Transform to screen space: graph_pos * zoom + zoom_center
                let screen_pos = egui::pos2(
                    x * zoom + zoom_center.x,
                    y * zoom + zoom_center.y,
                );
                if rect.contains(screen_pos) {
                    painter.circle_filled(screen_pos, dot_radius, dot_color);
                }
                x += grid_spacing;
            }
            y += grid_spacing;
        }
    }

    /// Evaluate a cubic bezier curve at parameter t ∈ [0, 1]
    fn bezier_point(p0: egui::Pos2, p1: egui::Pos2, p2: egui::Pos2, p3: egui::Pos2, t: f32) -> egui::Pos2 {
        let u = 1.0 - t;
        let tt = t * t;
        let uu = u * u;
        egui::pos2(
            uu * u * p0.x + 3.0 * uu * t * p1.x + 3.0 * u * tt * p2.x + tt * t * p3.x,
            uu * u * p0.y + 3.0 * uu * t * p1.y + 3.0 * u * tt * p2.y + tt * t * p3.y,
        )
    }

    /// Find the nearest compatible connection for inserting the dragged node.
    /// Returns (input_id, output_id, src_graph_pos, dst_graph_pos) — positions in graph space.
    fn find_insert_target(
        &self,
        dragged_node: NodeId,
    ) -> Option<(InputId, OutputId, egui::Pos2, egui::Pos2)> {
        let node_pos = *self.state.node_positions.get(dragged_node)?;

        // Collect which InputIds are connected (to find free ports on dragged node)
        let mut connected_inputs: std::collections::HashSet<InputId> = std::collections::HashSet::new();
        let mut connected_outputs: std::collections::HashSet<OutputId> = std::collections::HashSet::new();
        for (input_id, outputs) in self.state.graph.iter_connection_groups() {
            connected_inputs.insert(input_id);
            for output_id in outputs {
                connected_outputs.insert(output_id);
            }
        }

        // Get dragged node's free input types and free output types
        let dragged_data = self.state.graph.nodes.get(dragged_node)?;
        let free_input_types: Vec<DataType> = dragged_data.inputs.iter()
            .filter(|(_, id)| !connected_inputs.contains(id))
            .filter_map(|(_, id)| {
                let param = self.state.graph.inputs.get(*id)?;
                if matches!(param.kind, InputParamKind::ConstantOnly) { return None; }
                Some(param.typ.clone())
            })
            .collect();
        let free_output_types: Vec<DataType> = dragged_data.outputs.iter()
            .filter(|(_, id)| !connected_outputs.contains(id))
            .filter_map(|(_, id)| Some(self.state.graph.outputs.get(*id)?.typ.clone()))
            .collect();

        if free_input_types.is_empty() || free_output_types.is_empty() {
            return None;
        }

        let threshold = 50.0; // graph-space distance threshold

        let mut best: Option<(InputId, OutputId, egui::Pos2, egui::Pos2, f32)> = None;

        for (input_id, outputs) in self.state.graph.iter_connection_groups() {
            for output_id in outputs {
                // Skip connections involving the dragged node
                let input_node = self.state.graph.inputs.get(input_id).map(|p| p.node);
                let output_node = self.state.graph.outputs.get(output_id).map(|p| p.node);
                if input_node == Some(dragged_node) || output_node == Some(dragged_node) {
                    continue;
                }

                // Check data type compatibility
                let conn_type = match self.state.graph.outputs.get(output_id) {
                    Some(p) => p.typ.clone(),
                    None => continue,
                };
                let has_matching_input = free_input_types.iter().any(|t| *t == conn_type);
                let has_matching_output = free_output_types.iter().any(|t| *t == conn_type);
                if !has_matching_input || !has_matching_output {
                    continue;
                }

                // Get source and dest node positions (graph space)
                let src_node_id = output_node.unwrap();
                let dst_node_id = input_node.unwrap();
                let src_node_pos = match self.state.node_positions.get(src_node_id) {
                    Some(p) => *p,
                    None => continue,
                };
                let dst_node_pos = match self.state.node_positions.get(dst_node_id) {
                    Some(p) => *p,
                    None => continue,
                };

                // Approximate port positions in graph space (output on right, input on left)
                let src_port = egui::pos2(src_node_pos.x + 80.0, src_node_pos.y + 30.0);
                let dst_port = egui::pos2(dst_node_pos.x - 10.0, dst_node_pos.y + 30.0);

                // Compute bezier in graph space
                let control_scale = ((dst_port.x - src_port.x) / 2.0).max(30.0);
                let src_ctrl = egui::pos2(src_port.x + control_scale, src_port.y);
                let dst_ctrl = egui::pos2(dst_port.x - control_scale, dst_port.y);

                // Sample bezier and find min distance to dragged node center
                let mut min_dist = f32::MAX;
                for i in 0..=20 {
                    let t = i as f32 / 20.0;
                    let p = Self::bezier_point(src_port, src_ctrl, dst_ctrl, dst_port, t);
                    let d = node_pos.distance(p);
                    if d < min_dist {
                        min_dist = d;
                    }
                }

                if min_dist < threshold {
                    if best.is_none() || min_dist < best.as_ref().unwrap().4 {
                        best = Some((input_id, output_id, src_port, dst_port, min_dist));
                    }
                }
            }
        }

        best.map(|(input, output, src, dst, _)| (input, output, src, dst))
    }

    /// Draw a highlight over a connection to indicate insertion target.
    /// src/dst are in graph space — converted to screen space here.
    fn draw_connection_highlight(
        ui: &egui::Ui,
        src_graph: egui::Pos2,
        dst_graph: egui::Pos2,
        zoom: f32,
        pan: egui::Vec2,
        editor_offset: egui::Vec2,
    ) {
        // Convert graph space to screen space
        let to_screen = |p: egui::Pos2| -> egui::Pos2 {
            egui::pos2(p.x * zoom + pan.x + editor_offset.x, p.y * zoom + pan.y + editor_offset.y)
        };
        let src = to_screen(src_graph);
        let dst = to_screen(dst_graph);

        let control_scale = ((dst.x - src.x) / 2.0).max(30.0 * zoom);
        let src_ctrl = egui::pos2(src.x + control_scale, src.y);
        let dst_ctrl = egui::pos2(dst.x - control_scale, dst.y);

        let bezier = egui::epaint::CubicBezierShape::from_points_stroke(
            [src, src_ctrl, dst_ctrl, dst],
            false,
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(7.0 * zoom, egui::Color32::from_rgb(100, 220, 100)),
        );
        ui.painter().add(bezier);
    }

    /// Execute the insert-node-on-connection action
    fn execute_insert_on_connection(
        &mut self,
        dragged_node: NodeId,
        target_input: InputId,
        target_output: OutputId,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let track_id = match self.track_id {
            Some(id) => id,
            None => return,
        };
        let backend_track_id = match shared.layer_to_track_map.get(&track_id) {
            Some(&id) => id,
            None => return,
        };
        let audio_controller = match shared.audio_controller {
            Some(ref c) => (*c).clone(),
            None => return,
        };

        // Get the connection's data type to find matching ports on dragged node
        let conn_type = match self.state.graph.outputs.get(target_output) {
            Some(p) => p.typ.clone(),
            None => return,
        };

        // Get the source and dest nodes/ports of the existing connection
        let src_frontend_node = match self.state.graph.outputs.get(target_output) {
            Some(p) => p.node,
            None => return,
        };
        let dst_frontend_node = match self.state.graph.inputs.get(target_input) {
            Some(p) => p.node,
            None => return,
        };

        let src_port_idx = self.state.graph.nodes.get(src_frontend_node)
            .and_then(|n| n.outputs.iter().position(|(_, id)| *id == target_output))
            .unwrap_or(0);
        let dst_port_idx = self.state.graph.nodes.get(dst_frontend_node)
            .and_then(|n| n.inputs.iter().position(|(_, id)| *id == target_input))
            .unwrap_or(0);

        // Find matching free input and output on the dragged node
        let dragged_data = match self.state.graph.nodes.get(dragged_node) {
            Some(d) => d,
            None => return,
        };

        // Collect connected ports
        let mut connected_inputs: std::collections::HashSet<InputId> = std::collections::HashSet::new();
        let mut connected_outputs: std::collections::HashSet<OutputId> = std::collections::HashSet::new();
        for (input_id, outputs) in self.state.graph.iter_connection_groups() {
            connected_inputs.insert(input_id);
            for output_id in outputs {
                connected_outputs.insert(output_id);
            }
        }

        // Find first free input with matching type
        let drag_input = dragged_data.inputs.iter()
            .find(|(_, id)| {
                if connected_inputs.contains(id) { return false; }
                self.state.graph.inputs.get(*id)
                    .map(|p| {
                        !matches!(p.kind, InputParamKind::ConstantOnly) && p.typ == conn_type
                    })
                    .unwrap_or(false)
            })
            .map(|(_, id)| *id);

        let drag_output = dragged_data.outputs.iter()
            .find(|(_, id)| {
                if connected_outputs.contains(id) { return false; }
                self.state.graph.outputs.get(*id)
                    .map(|p| p.typ == conn_type)
                    .unwrap_or(false)
            })
            .map(|(_, id)| *id);

        let (drag_input_id, drag_output_id) = match (drag_input, drag_output) {
            (Some(i), Some(o)) => (i, o),
            _ => return,
        };

        let drag_input_port_idx = dragged_data.inputs.iter()
            .position(|(_, id)| *id == drag_input_id)
            .unwrap_or(0);
        let drag_output_port_idx = dragged_data.outputs.iter()
            .position(|(_, id)| *id == drag_output_id)
            .unwrap_or(0);

        // Get backend node IDs
        let src_backend = match self.node_id_map.get(&src_frontend_node) {
            Some(&id) => id,
            None => return,
        };
        let dst_backend = match self.node_id_map.get(&dst_frontend_node) {
            Some(&id) => id,
            None => return,
        };
        let drag_backend = match self.node_id_map.get(&dragged_node) {
            Some(&id) => id,
            None => return,
        };

        let BackendNodeId::Audio(src_idx) = src_backend;
        let BackendNodeId::Audio(dst_idx) = dst_backend;
        let BackendNodeId::Audio(drag_idx) = drag_backend;

        // Send commands to backend: disconnect old, connect source→drag, connect drag→dest
        {
            let mut controller = audio_controller.lock().unwrap();
            controller.graph_disconnect(
                backend_track_id,
                src_idx.index() as u32, src_port_idx,
                dst_idx.index() as u32, dst_port_idx,
            );
            controller.graph_connect(
                backend_track_id,
                src_idx.index() as u32, src_port_idx,
                drag_idx.index() as u32, drag_input_port_idx,
            );
            controller.graph_connect(
                backend_track_id,
                drag_idx.index() as u32, drag_output_port_idx,
                dst_idx.index() as u32, dst_port_idx,
            );
        }

        // Update frontend connections
        // Remove old connection
        if let Some(conns) = self.state.graph.connections.get_mut(target_input) {
            conns.retain(|&o| o != target_output);
        }
        // Add source → drag_input
        if let Some(conns) = self.state.graph.connections.get_mut(drag_input_id) {
            conns.push(target_output);
        } else {
            self.state.graph.connections.insert(drag_input_id, vec![target_output]);
        }
        // Add drag_output → dest
        if let Some(conns) = self.state.graph.connections.get_mut(target_input) {
            conns.push(drag_output_id);
        } else {
            self.state.graph.connections.insert(target_input, vec![drag_output_id]);
        }
    }
}

impl crate::panes::PaneRenderer for NodeGraphPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        // Check if we need to reload for a different track or project reload
        let current_track = *shared.active_layer_id;
        let generation_changed = shared.project_generation != self.last_project_generation;
        if generation_changed {
            self.last_project_generation = shared.project_generation;
        }

        // If selected track changed or project was reloaded, reload the graph
        if self.track_id != current_track || (generation_changed && current_track.is_some()) {
            if let Some(new_track_id) = current_track {
                // Get backend track ID
                if let Some(&backend_track_id) = shared.layer_to_track_map.get(&new_track_id) {
                    // Check if track is MIDI or Audio
                    if let Some(audio_controller) = &shared.audio_controller {
                        let is_valid_track = {
                            let _controller = audio_controller.lock().unwrap();
                            // TODO: Query track type from backend
                            // For now, assume it's valid if we have a track ID mapping
                            true
                        };

                        if is_valid_track {
                            // Reload graph for new track
                            self.track_id = Some(new_track_id);

                            // Recreate backend
                            self.backend = Some(Box::new(audio_backend::AudioGraphBackend::new(
                                backend_track_id,
                                (*audio_controller).clone(),
                            )));

                            // Load graph from backend
                            if let Err(e) = self.load_graph_from_backend() {
                                eprintln!("Failed to load graph from backend: {}", e);
                            }
                        }
                    }
                }
            } else {
                self.track_id = None;
            }
        }

        // Check if we have a valid track
        if self.track_id.is_none() || self.backend.is_none() {
            // Show message that no valid track is selected
            let painter = ui.painter();
            let bg_color = egui::Color32::from_gray(30);
            painter.rect_filled(rect, 0.0, bg_color);

            let text = "Select a MIDI or Audio track to view its node graph";
            let font_id = egui::FontId::proportional(16.0);
            let text_color = egui::Color32::from_gray(150);

            let galley = painter.layout_no_wrap(text.to_string(), font_id, text_color);
            let text_pos = rect.center() - galley.size() / 2.0;
            painter.galley(text_pos, galley, text_color);
            return;
        }
        // Get colors from theme
        let bg_style = shared.theme.style(".node-graph-background", ui.ctx());
        let grid_style = shared.theme.style(".node-graph-grid", ui.ctx());

        let bg_color = bg_style.background_color.unwrap_or(egui::Color32::from_gray(45));
        let grid_color = grid_style.background_color.unwrap_or(egui::Color32::from_gray(55));

        // Allocate the rect and render the graph editor within it
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            // Check for scroll input to override library's default zoom behavior
            // Only handle scroll when mouse is over the node graph area
            let pointer_over_graph = ui.rect_contains_pointer(rect);
            let modifiers = ui.input(|i| i.modifiers);
            let has_ctrl = modifiers.ctrl || modifiers.command;

            // When ctrl is held, check for raw scroll events in the events list
            let scroll_delta = if !pointer_over_graph {
                egui::Vec2::ZERO
            } else if has_ctrl {
                // Sum up scroll events from the raw event list
                ui.input(|i| {
                    let mut total_scroll = egui::Vec2::ZERO;
                    for event in &i.events {
                        if let egui::Event::MouseWheel { delta, .. } = event {
                            total_scroll += *delta;
                        }
                    }
                    total_scroll
                })
            } else {
                ui.input(|i| i.smooth_scroll_delta)
            };
            let has_scroll = scroll_delta != egui::Vec2::ZERO;


            // Save current zoom to detect if library changed it
            let zoom_before = self.state.pan_zoom.zoom;
            let pan_before = self.state.pan_zoom.pan;

            // Draw dot grid background with pan/zoom
            let pan_zoom = &self.state.pan_zoom;
            Self::draw_dot_grid_background(ui, rect, bg_color, grid_color, pan_zoom);

            // Draw the graph editor (library will process scroll as zoom by default)
            let graph_response = self.state.draw_graph_editor(
                ui,
                AllNodeTemplates,
                &mut self.user_state,
                Vec::default(),
            );

            // Handle graph events and create actions
            self.handle_graph_response(graph_response, shared, rect);

            // Check for parameter value changes and send updates to backend
            self.check_parameter_changes();

            // Execute any parameter change actions
            self.execute_pending_action(shared);

            // Insert-node-on-connection: find target during drag, highlight, and execute on drop
            let primary_down = ui.input(|i| i.pointer.primary_down());
            if let Some(dragged) = self.dragging_node {
                if primary_down {
                    // Still dragging — check for nearby compatible connection
                    if let Some((input_id, output_id, src_graph, dst_graph)) = self.find_insert_target(dragged) {
                        self.insert_target = Some((input_id, output_id));
                        Self::draw_connection_highlight(
                            ui,
                            src_graph,
                            dst_graph,
                            self.state.pan_zoom.zoom,
                            self.state.pan_zoom.pan,
                            rect.min.to_vec2(),
                        );
                    } else {
                        self.insert_target = None;
                    }
                } else {
                    // Drag ended — execute insertion if we have a target
                    if let Some((target_input, target_output)) = self.insert_target.take() {
                        self.execute_insert_on_connection(dragged, target_input, target_output, shared);
                    }
                    self.dragging_node = None;
                }
            }

            // Override library's default scroll behavior:
            // - Library uses scroll for zoom
            // - We want: scroll = pan, ctrl+scroll = zoom
            if has_scroll {
                if has_ctrl {
                    // Ctrl+scroll: zoom (explicitly handle it instead of relying on library)
                    // First undo any zoom the library applied
                    if self.state.pan_zoom.zoom != zoom_before {
                        let undo_zoom = zoom_before / self.state.pan_zoom.zoom;
                        self.state.zoom(ui, undo_zoom);
                    }
                    // Now apply zoom based on scroll
                    let zoom_delta = (scroll_delta.y * 0.002).exp();
                    self.state.zoom(ui, zoom_delta);
                } else {
                    // Scroll without ctrl: library zoomed, but we want pan instead
                    // Undo the zoom and apply pan
                    if self.state.pan_zoom.zoom != zoom_before {
                        // Library changed zoom - revert it
                        let undo_zoom = zoom_before / self.state.pan_zoom.zoom;
                        self.state.zoom(ui, undo_zoom);
                    }
                    // Apply pan
                    self.state.pan_zoom.pan = pan_before + scroll_delta;
                }
            }

            // Draw menu button in top-left corner
            let button_pos = rect.min + egui::vec2(8.0, 8.0);
            ui.scope_builder(
                egui::UiBuilder::new().max_rect(egui::Rect::from_min_size(button_pos, egui::vec2(100.0, 24.0))),
                |ui| {
                    if ui.button("➕ Add Node").clicked() {
                        // Open node finder at button's top-left position
                        self.state.node_finder = Some(egui_node_graph2::NodeFinder::new_at(button_pos));
                    }
                },
            );
        });

        // TODO: Handle node responses and sync with backend
    }

    fn name(&self) -> &str {
        "Node Graph"
    }
}

impl Default for NodeGraphPane {
    fn default() -> Self {
        Self::new()
    }
}
