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
        }
    }

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
                "Delay" => graph_data::NodeTemplate::Delay,
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
                user_data: graph_data::NodeData,
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

            // Set parameter values
            for (&param_id, &value) in &node.parameters {
                // Find the input param in the graph and set its value
                if let Some(node_data) = self.state.graph.nodes.get_mut(frontend_id) {
                    // TODO: Set parameter values on the node's input params
                    // This requires matching param_id to the input param by index
                    let _ = (param_id, value); // Silence unused warning for now
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
                                // Add connection to graph - connections map is InputId -> Vec<OutputId>
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

        Ok(())
    }

    fn handle_graph_response(
        &mut self,
        response: egui_node_graph2::GraphResponse<
            graph_data::UserResponse,
            graph_data::NodeData,
        >,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        use egui_node_graph2::NodeResponse;

        for node_response in response.node_responses {
            match node_response {
                NodeResponse::CreatedNode(node_id) => {
                    // Node was created from the node finder
                    // Get node label which is the node type string
                    if let Some(node) = self.state.graph.nodes.get(node_id) {
                        let node_type = node.label.clone();
                        let position = self.state.node_positions.get(node_id)
                            .map(|pos| (pos.x, pos.y))
                            .unwrap_or((0.0, 0.0));

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
                    // Node was moved - we'll handle this on drag end
                    // For now, just update the position (no action needed during drag)
                    self.user_state.active_node = Some(node);
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
                                            eprintln!("[DEBUG] Mapped new node: frontend {:?} -> backend {:?}", frontend_id, backend_id);
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
        let mut checked_count = 0;
        let mut connection_only_count = 0;
        let mut non_float_count = 0;

        for (input_id, input_param) in &self.state.graph.inputs {
            // Only check parameters that can have constant values (not ConnectionOnly)
            if matches!(input_param.kind, InputParamKind::ConnectionOnly) {
                connection_only_count += 1;
                continue;
            }

            // Get current value
            let current_value = match &input_param.value {
                ValueType::Float { value } => {
                    checked_count += 1;
                    *value
                },
                other => {
                    non_float_count += 1;
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

                    // Get backend node ID
                    if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                        // Get parameter index (position in node's inputs array)
                        if let Some(node) = self.state.graph.nodes.get(node_id) {
                            if let Some(param_index) = node.inputs.iter().position(|(_, id)| *id == input_id) {
                                eprintln!("[DEBUG] Parameter changed: node {:?} param {} from {:?} to {}",
                                    backend_id, param_index, previous_value, current_value);
                                // Create action to update backend
                                let action = Box::new(actions::NodeGraphAction::SetParameter(
                                    actions::SetParameterAction::new(
                                        track_id,
                                        backend_id,
                                        param_index as u32,
                                        current_value as f64,
                                    )
                                ));
                                self.pending_action = Some(action);
                            }
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
}

impl crate::panes::PaneRenderer for NodeGraphPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        // Check if we need to reload for a different track
        let current_track = *shared.active_layer_id;

        // If selected track changed, reload the graph
        if self.track_id != current_track {
            if let Some(new_track_id) = current_track {
                // Get backend track ID
                if let Some(&backend_track_id) = shared.layer_to_track_map.get(&new_track_id) {
                    // Check if track is MIDI or Audio
                    if let Some(audio_controller) = &shared.audio_controller {
                        let is_valid_track = {
                            let controller = audio_controller.lock().unwrap();
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
        ui.allocate_ui_at_rect(rect, |ui| {
            // Check for scroll input to override library's default zoom behavior
            let modifiers = ui.input(|i| i.modifiers);
            let has_ctrl = modifiers.ctrl || modifiers.command;

            // When ctrl is held, check for raw scroll events in the events list
            let scroll_delta = if has_ctrl {
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
            self.handle_graph_response(graph_response, shared);

            // Check for parameter value changes and send updates to backend
            self.check_parameter_changes();

            // Execute any parameter change actions
            self.execute_pending_action(shared);

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
            ui.allocate_ui_at_rect(
                egui::Rect::from_min_size(button_pos, egui::vec2(100.0, 24.0)),
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
