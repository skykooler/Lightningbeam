//! Node Graph Pane
//!
//! Audio/MIDI node graph editor for modular synthesis and effects processing

pub mod actions;
pub mod audio_backend;
pub mod backend;
pub mod graph_data;

use backend::{BackendNodeId, GraphBackend};
use graph_data::{AllNodeTemplates, SubgraphNodeTemplates, VoiceAllocatorNodeTemplates, DataType, GraphState, NamModelInfo, NodeData, NodeTemplate, PendingAmpSimLoad, ValueType};
use super::NodePath;
use eframe::egui;
use egui_node_graph2::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

type GroupId = u32;

/// A connection that crosses a group boundary
#[derive(Clone, Debug)]
struct BoundaryConnection {
    /// Node outside the group (backend ID)
    external_node: u32,
    /// Port index on the external node
    external_port: usize,
    /// Node inside the group (backend ID)
    internal_node: u32,
    /// Port index on the internal node
    internal_port: usize,
    /// Display name for the group port
    port_name: String,
    /// Signal type for the port
    data_type: DataType,
}

/// A group of nodes collapsed into a single placeholder
#[derive(Clone, Debug)]
struct GroupDef {
    id: GroupId,
    name: String,
    /// Backend node IDs of nodes belonging to this group
    member_nodes: Vec<u32>,
    /// Position of the group placeholder node
    position: (f32, f32),
    /// Connections from outside → inside the group
    boundary_inputs: Vec<BoundaryConnection>,
    /// Connections from inside → outside the group
    boundary_outputs: Vec<BoundaryConnection>,
    /// Parent group ID for nested groups (None = top-level group)
    parent_group_id: Option<GroupId>,
}

/// What kind of container we've entered for subgraph editing
#[derive(Clone, Debug)]
enum SubgraphContext {
    VoiceAllocator { backend_id: BackendNodeId },
    Group { group_id: GroupId, name: String },
}

/// One level of subgraph editing — stores the parent state we'll restore on exit
struct SubgraphFrame {
    context: SubgraphContext,
    saved_state: SavedGraphState,
}

/// Saved graph editor state for restoring when exiting a subgraph
struct SavedGraphState {
    state: GraphEditorState<NodeData, DataType, ValueType, NodeTemplate, GraphState>,
    user_state: GraphState,
    node_id_map: HashMap<NodeId, BackendNodeId>,
    backend_to_frontend_map: HashMap<BackendNodeId, NodeId>,
    parameter_values: HashMap<InputId, f32>,
    /// Groups are only saved/restored for VA transitions. For Group transitions,
    /// groups persist in self (so sub-groups aren't lost on exit).
    groups: Option<Vec<GroupDef>>,
    next_group_id: Option<GroupId>,
    group_placeholder_map: HashMap<NodeId, GroupId>,
}

/// Node graph pane with egui_node_graph2 integration
pub struct NodeGraphPane {
    /// The graph editor state
    state: GraphEditorState<NodeData, DataType, ValueType, NodeTemplate, GraphState>,

    /// User state for the graph
    user_state: GraphState,

    /// Backend integration
    backend: Option<Box<dyn GraphBackend>>,

    /// Maps frontend node IDs to backend node IDs
    node_id_map: HashMap<NodeId, BackendNodeId>,

    /// Maps backend node IDs to frontend node IDs (reverse mapping)
    backend_to_frontend_map: HashMap<BackendNodeId, NodeId>,

    /// Track ID this graph belongs to
    track_id: Option<Uuid>,

    /// Pending action to execute
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

    /// Stack of subgraph contexts — empty = editing track-level graph,
    /// non-empty = editing nested subgraph(s). Supports arbitrary nesting depth.
    subgraph_stack: Vec<SubgraphFrame>,

    /// Group definitions (frontend-only — backend graph stays flat)
    groups: Vec<GroupDef>,
    /// Next group ID to assign
    next_group_id: GroupId,
    /// Maps frontend NodeId → GroupId for group placeholder nodes
    group_placeholder_map: HashMap<NodeId, GroupId>,
    /// Group currently being renamed (shows text edit popup)
    renaming_group: Option<(GroupId, String)>,
    /// Right-click context menu state: (node_id, screen_pos)
    node_context_menu: Option<(NodeId, egui::Pos2)>,
    /// Cached node screen rects from last frame (for hit-testing)
    last_node_rects: std::collections::HashMap<NodeId, egui::Rect>,

    /// Script nodes loaded from preset that need script_id resolution
    /// (frontend_node_id, script_source) — processed in render loop where document is available
    pending_script_resolutions: Vec<(NodeId, String)>,

    /// Last time we polled oscilloscope data (~20 FPS)
    last_oscilloscope_poll: std::time::Instant,
    /// Backend track ID (u32) for oscilloscope queries
    backend_track_id: Option<u32>,
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
            subgraph_stack: Vec::new(),
            groups: Vec::new(),
            next_group_id: 1,
            group_placeholder_map: HashMap::new(),
            renaming_group: None,
            node_context_menu: None,
            last_node_rects: HashMap::new(),
            pending_script_resolutions: Vec::new(),
            last_oscilloscope_poll: std::time::Instant::now(),
            backend_track_id: None,
        }
    }

    /// Load the graph state from the backend and populate the frontend
    fn load_graph_from_backend(&mut self) -> Result<(), String> {
        let json = if let Some(backend) = &self.backend {
            backend.get_state_json()?
        } else {
            return Err("No backend available".to_string());
        };

        self.load_graph_from_json(&json)
    }

    /// Rebuild a Script node's ports and parameters to match a compiled script.
    /// Performs a diff: ports with matching name+type keep their connections,
    /// removed ports lose connections, new ports are added.
    /// Parameters are added as ConnectionOrConstant inputs with inline widgets.
    fn rebuild_script_node_ports(&mut self, node_id: NodeId, compiled: &beamdsp::CompiledScript) {
        let signal_to_data_type = |sig: beamdsp::ast::SignalKind| match sig {
            beamdsp::ast::SignalKind::Audio => DataType::Audio,
            beamdsp::ast::SignalKind::Cv => DataType::CV,
            beamdsp::ast::SignalKind::Midi => DataType::Midi,
        };

        let unit_str = |u: &str| -> &'static str {
            match u { "Hz" => " Hz", "s" => " s", "dB" => " dB", "%" => "%", _ => "" }
        };

        // Collect what the new inputs should be: signal ports + param ports
        // Signal ports use DataType matching their signal kind, ConnectionOnly
        // Param ports use DataType::CV, ConnectionOrConstant with float_param value
        let num_signal_inputs = compiled.input_ports.len();
        let num_params = compiled.parameters.len();
        let num_signal_outputs = compiled.output_ports.len();

        // Check if everything already matches (ports + params + outputs)
        let already_matches = if let Some(node) = self.state.graph.nodes.get(node_id) {
            let expected_inputs = num_signal_inputs + num_params;
            if node.inputs.len() != expected_inputs || node.outputs.len() != num_signal_outputs {
                false
            } else {
                // Check signal inputs
                let signals_match = node.inputs[..num_signal_inputs].iter()
                    .zip(&compiled.input_ports)
                    .all(|((name, id), port)| {
                        name == &port.name
                            && self.state.graph.inputs.get(*id)
                                .map_or(false, |p| p.typ == signal_to_data_type(port.signal))
                    });
                // Check param inputs
                let params_match = node.inputs[num_signal_inputs..].iter()
                    .zip(&compiled.parameters)
                    .all(|((name, id), param)| {
                        name == &param.name
                            && self.state.graph.inputs.get(*id)
                                .map_or(false, |p| p.typ == DataType::CV)
                    });
                // Check outputs
                let outputs_match = node.outputs.iter()
                    .zip(&compiled.output_ports)
                    .all(|((name, id), port)| {
                        name == &port.name
                            && self.state.graph.outputs.get(*id)
                                .map_or(false, |p| p.typ == signal_to_data_type(port.signal))
                    });
                signals_match && params_match && outputs_match
            }
        } else {
            return;
        };

        if already_matches {
            if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                node.label = compiled.name.clone();
            }
            return;
        }

        // Build lookup of existing inputs: (name, type, kind) → InputId
        let old_inputs: Vec<(String, InputId, DataType, InputParamKind)> = self.state.graph.nodes.get(node_id)
            .map(|n| n.inputs.iter().filter_map(|(name, id)| {
                let p = self.state.graph.inputs.get(*id)?;
                Some((name.clone(), *id, p.typ, p.kind))
            }).collect())
            .unwrap_or_default();

        let old_outputs: Vec<(String, OutputId, DataType)> = self.state.graph.nodes.get(node_id)
            .map(|n| n.outputs.iter().filter_map(|(name, id)| {
                let typ = self.state.graph.outputs.get(*id)?.typ;
                Some((name.clone(), *id, typ))
            }).collect())
            .unwrap_or_default();

        // Match signal inputs
        let mut used_old_inputs: HashSet<InputId> = HashSet::new();
        let mut new_input_ids: Vec<(String, InputId)> = Vec::new();

        for port in &compiled.input_ports {
            let dt = signal_to_data_type(port.signal);
            if let Some((_, old_id, _, _)) = old_inputs.iter().find(|(name, id, typ, kind)| {
                name == &port.name && *typ == dt
                    && matches!(kind, InputParamKind::ConnectionOnly)
                    && !used_old_inputs.contains(id)
            }) {
                used_old_inputs.insert(*old_id);
                new_input_ids.push((port.name.clone(), *old_id));
            } else {
                let id = self.state.graph.add_input_param(
                    node_id, port.name.clone(), dt,
                    ValueType::float(0.0), InputParamKind::ConnectionOnly, true,
                );
                new_input_ids.push((port.name.clone(), id));
            }
        }

        // Match param inputs
        for (i, param) in compiled.parameters.iter().enumerate() {
            if let Some((_, old_id, _, _)) = old_inputs.iter().find(|(name, id, typ, kind)| {
                name == &param.name && *typ == DataType::CV
                    && matches!(kind, InputParamKind::ConnectionOrConstant)
                    && !used_old_inputs.contains(id)
            }) {
                used_old_inputs.insert(*old_id);
                new_input_ids.push((param.name.clone(), *old_id));
            } else {
                let id = self.state.graph.add_input_param(
                    node_id, param.name.clone(), DataType::CV,
                    ValueType::float_param(param.default, param.min, param.max, unit_str(&param.unit), i as u32, None),
                    InputParamKind::ConnectionOrConstant, true,
                );
                new_input_ids.push((param.name.clone(), id));
            }
        }

        // Remove old inputs that weren't reused
        for (_, old_id, _, _) in &old_inputs {
            if !used_old_inputs.contains(old_id) {
                self.state.graph.remove_input_param(*old_id);
            }
        }

        // Match outputs
        let mut used_old_outputs: HashSet<OutputId> = HashSet::new();
        let mut new_output_ids: Vec<(String, OutputId)> = Vec::new();

        for port in &compiled.output_ports {
            let dt = signal_to_data_type(port.signal);
            if let Some((_, old_id, _)) = old_outputs.iter().find(|(name, id, typ)| {
                name == &port.name && *typ == dt && !used_old_outputs.contains(id)
            }) {
                used_old_outputs.insert(*old_id);
                new_output_ids.push((port.name.clone(), *old_id));
            } else {
                let id = self.state.graph.add_output_param(node_id, port.name.clone(), dt);
                new_output_ids.push((port.name.clone(), id));
            }
        }

        for (_, old_id, _) in &old_outputs {
            if !used_old_outputs.contains(old_id) {
                self.state.graph.remove_output_param(*old_id);
            }
        }

        // Set the node's port ordering and UI metadata
        if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
            node.inputs = new_input_ids;
            node.outputs = new_output_ids;
            node.label = compiled.name.clone();
            node.user_data.ui_declaration = Some(compiled.ui_declaration.clone());
            node.user_data.sample_slot_names = compiled.sample_slots.clone();
        }
        // Store draw VM in GraphState (needs &mut, can't live on immutable NodeData)
        if let Some(draw_vm) = &compiled.draw_vm {
            self.user_state.draw_vms.insert(node_id, draw_vm.clone());
        } else {
            self.user_state.draw_vms.remove(&node_id);
        }
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
                            if let Some(va_id) = self.va_context() {
                                // Inside VA template — call template command directly
                                if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                    if let Some(audio_controller) = &shared.audio_controller {
                                        let mut controller = audio_controller.lock().unwrap();
                                        controller.graph_add_node_to_template(
                                            backend_track_id, va_id, node_type.clone(),
                                            position.0, position.1,
                                        );
                                        // Query template state to get the new node's backend ID
                                        std::thread::sleep(std::time::Duration::from_millis(10));
                                        if let Ok(json) = controller.query_template_state(backend_track_id, va_id) {
                                            if let Ok(state) = serde_json::from_str::<daw_backend::audio::node_graph::GraphPreset>(&json) {
                                                // Find the new node by type and position
                                                if let Some(backend_node) = state.nodes.iter().find(|n| {
                                                    n.node_type == node_type &&
                                                    (n.position.0 - position.0).abs() < 1.0 &&
                                                    (n.position.1 - position.1).abs() < 1.0
                                                }) {
                                                    let backend_id = BackendNodeId::Audio(
                                                        petgraph::stable_graph::NodeIndex::new(backend_node.id as usize)
                                                    );
                                                    self.node_id_map.insert(node_id, backend_id);
                                                    self.backend_to_frontend_map.insert(backend_id, node_id);

                                                    // Auto-load default NAM model for new AmpSim nodes
                                                    if node_type == "AmpSim" {
                                                        if let Some(model) = self.user_state.available_nam_models.iter().find(|m| m.is_bundled) {
                                                            controller.amp_sim_load_model(
                                                                backend_track_id,
                                                                backend_node.id,
                                                                model.path.clone(),
                                                            );
                                                            if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                                                                node.user_data.nam_model_name = Some(model.name.clone());
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                // Normal track graph — use action system
                                let action = Box::new(actions::NodeGraphAction::AddNode(
                                    actions::AddNodeAction::new(track_id, node_type.clone(), position)
                                ));
                                self.pending_action = Some(action);
                                self.pending_node_addition = Some((node_id, node_type, position));
                            }
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
                                if let Some(va_id) = self.va_context() {
                                    // Inside VA template
                                    if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                        if let Some(audio_controller) = &shared.audio_controller {
                                            let mut controller = audio_controller.lock().unwrap();
                                            controller.graph_connect_in_template(
                                                backend_track_id, va_id,
                                                from_id.index(), from_port,
                                                to_id.index(), to_port,
                                            );
                                        }
                                    }
                                } else {
                                    let action = Box::new(actions::NodeGraphAction::Connect(
                                        actions::ConnectAction::new(
                                            track_id, from_id, from_port, to_id, to_port,
                                        )
                                    ));
                                    self.pending_action = Some(action);
                                }
                            }
                        }
                    }
                }
                NodeResponse::DisconnectEvent { output, input } => {
                    // Connection was removed
                    if let Some(track_id) = self.track_id {
                        let from_node = self.state.graph.outputs.get(output).map(|o| o.node);
                        let to_node = self.state.graph.inputs.get(input).map(|i| i.node);

                        if let (Some(from_node_id), Some(to_node_id)) = (from_node, to_node) {
                            let from_port = self.state.graph.nodes.get(from_node_id)
                                .and_then(|n| n.outputs.iter().position(|(_, id)| *id == output))
                                .unwrap_or(0);
                            let to_port = self.state.graph.nodes.get(to_node_id)
                                .and_then(|n| n.inputs.iter().position(|(_, id)| *id == input))
                                .unwrap_or(0);

                            let from_backend = self.node_id_map.get(&from_node_id);
                            let to_backend = self.node_id_map.get(&to_node_id);

                            if let (Some(&from_id), Some(&to_id)) = (from_backend, to_backend) {
                                if let Some(va_id) = self.va_context() {
                                    // Inside VA template
                                    if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                        if let Some(audio_controller) = &shared.audio_controller {
                                            let mut controller = audio_controller.lock().unwrap();
                                            controller.graph_disconnect_in_template(
                                                backend_track_id, va_id,
                                                from_id.index(), from_port,
                                                to_id.index(), to_port,
                                            );
                                        }
                                    }
                                } else {
                                    let action = Box::new(actions::NodeGraphAction::Disconnect(
                                        actions::DisconnectAction::new(
                                            track_id, from_id, from_port, to_id, to_port,
                                        )
                                    ));
                                    self.pending_action = Some(action);
                                }
                            }
                        }
                    }
                }
                NodeResponse::DeleteNodeFull { node_id, .. } => {
                    // If this is a group placeholder, ungroup instead of deleting
                    if let Some(&group_id) = self.group_placeholder_map.get(&node_id) {
                        self.groups.retain(|g| g.id != group_id);
                        // Will rebuild view after response handling
                        self.rebuild_view();
                        self.sync_groups_to_backend(shared);
                        continue;
                    }

                    // Node was deleted
                    if let Some(track_id) = self.track_id {
                        if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                            if let Some(va_id) = self.va_context() {
                                // Inside VA template
                                if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                    if let Some(audio_controller) = &shared.audio_controller {
                                        let mut controller = audio_controller.lock().unwrap();
                                        controller.graph_remove_node_from_template(
                                            backend_track_id, va_id, backend_id.index(),
                                        );
                                    }
                                }
                            } else {
                                let action = Box::new(actions::NodeGraphAction::RemoveNode(
                                    actions::RemoveNodeAction::new(track_id, backend_id)
                                ));
                                self.pending_action = Some(action);
                            }

                            // Remove from ID map
                            self.node_id_map.remove(&node_id);
                            self.backend_to_frontend_map.remove(&backend_id);
                        }
                    }
                }
                NodeResponse::MoveNode { node, drag_delta: _ } => {
                    self.user_state.active_node = Some(node);
                    self.dragging_node = Some(node);

                    // Update group placeholder position (frontend-only, no backend sync)
                    if let Some(&group_id) = self.group_placeholder_map.get(&node) {
                        if let Some(pos) = self.state.node_positions.get(node) {
                            if let Some(group) = self.groups.iter_mut().find(|g| g.id == group_id) {
                                group.position = (pos.x, pos.y);
                            }
                        }
                        continue;
                    }

                    // Sync updated position to backend
                    if let Some(&backend_id) = self.node_id_map.get(&node) {
                        if let Some(pos) = self.state.node_positions.get(node) {
                            let node_index = backend_id.index();
                            if let Some(audio_controller) = &shared.audio_controller {
                                if let Some(&backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid)) {
                                    let mut controller = audio_controller.lock().unwrap();
                                    if let Some(va_id) = self.va_context() {
                                        controller.graph_set_node_position_in_template(
                                            backend_track_id,
                                            va_id,
                                            node_index,
                                            pos.x,
                                            pos.y,
                                        );
                                    } else {
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
                }
                NodeResponse::DoubleClick(node_id) => {
                    // Check if this is a container node we can enter
                    if let Some(node) = self.state.graph.nodes.get(node_id) {
                        match node.user_data.template {
                            NodeTemplate::VoiceAllocator => {
                                // VA can only be entered at track level (depth 0)
                                if !self.in_subgraph() {
                                    if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                                        self.enter_subgraph(
                                            SubgraphContext::VoiceAllocator {
                                                backend_id,
                                            },
                                            shared,
                                        );
                                    }
                                }
                            }
                            NodeTemplate::Group => {
                                // Groups can nest arbitrarily deep
                                if let Some(&group_id) = self.group_placeholder_map.get(&node_id) {
                                    let name = node.label.clone();
                                    self.enter_subgraph(
                                        SubgraphContext::Group {
                                            group_id,
                                            name,
                                        },
                                        shared,
                                    );
                                }
                            }
                            _ => {}
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
                let empty_metatrack_map = std::collections::HashMap::new();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: shared.layer_to_track_map,
                    clip_instance_to_backend_map: &mut empty_clip_map,
                    clip_to_metatrack_map: &empty_metatrack_map,
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

                                            // Auto-load default NAM model for new AmpSim nodes
                                            if node_type == "AmpSim" {
                                                if let Some(model) = self.user_state.available_nam_models.iter().find(|m| m.is_bundled) {
                                                    controller.amp_sim_load_model(
                                                        backend_track_id,
                                                        backend_node.id,
                                                        model.path.clone(),
                                                    );
                                                    if let Some(node) = self.state.graph.nodes.get_mut(frontend_id) {
                                                        node.user_data.nam_model_name = Some(model.name.clone());
                                                    }
                                                }
                                            }
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

    fn handle_pending_sampler_load(
        &mut self,
        load: graph_data::PendingSamplerLoad,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let backend_track_id = match self.backend_track_id {
            Some(id) => id,
            None => return,
        };
        let controller_arc = match &shared.audio_controller {
            Some(c) => std::sync::Arc::clone(c),
            None => return,
        };

        match load {
            graph_data::PendingSamplerLoad::SimpleFromPool { node_id, backend_node_id, pool_index, name } => {
                let mut controller = controller_arc.lock().unwrap();
                controller.sampler_load_from_pool(backend_track_id, backend_node_id, pool_index);
                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                    node.user_data.sample_display_name = Some(name);
                }
            }
            graph_data::PendingSamplerLoad::SimpleFromFile { node_id, backend_node_id } => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aiff"])
                    .pick_file()
                {
                    let file_name = path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Sample".to_string());

                    // Import into audio pool + asset library, then load from pool
                    let mut controller = controller_arc.lock().unwrap();
                    match controller.import_audio_sync(path.to_path_buf()) {
                        Ok(pool_index) => {
                            // Add to document asset library
                            let metadata = daw_backend::io::read_metadata(&path).ok();
                            let duration = metadata.as_ref().map(|m| m.duration).unwrap_or(0.0);
                            let clip = lightningbeam_core::clip::AudioClip::new_sampled(&file_name, pool_index, duration);
                            shared.action_executor.document_mut().add_audio_clip(clip);

                            // Load into sampler from pool
                            controller.sampler_load_from_pool(backend_track_id, backend_node_id, pool_index);
                        }
                        Err(e) => {
                            eprintln!("Failed to import audio '{}': {}", path.display(), e);
                        }
                    }
                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                        node.user_data.sample_display_name = Some(file_name);
                    }
                }
            }
            graph_data::PendingSamplerLoad::MultiFromPool { node_id, backend_node_id, pool_index, name } => {
                let mut controller = controller_arc.lock().unwrap();
                // Add as a single layer spanning full key range, root_key = 60 (C4)
                controller.multi_sampler_add_layer_from_pool(
                    backend_track_id, backend_node_id, pool_index,
                    0, 127, 60,
                );
                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                    node.user_data.sample_display_name = Some(name);
                }
            }
            graph_data::PendingSamplerLoad::MultiFromFolder { node_id, folder_id } => {
                // Find folder clips from available_folders
                let folder_clips: Vec<(String, usize)> = self.user_state.available_folders.iter()
                    .find(|f| f.folder_id == folder_id)
                    .map(|f| f.clip_pool_indices.clone())
                    .unwrap_or_default();

                if !folder_clips.is_empty() {
                    // TODO: Add MultiSamplerLoadFromPool command to avoid disk re-reads.
                    // For now, folder loading is a placeholder — the UI is wired up but
                    // loading multi-sampler layers from pool requires a new backend command.
                    let folder_name = self.user_state.available_folders.iter()
                        .find(|f| f.folder_id == folder_id)
                        .map(|f| f.name.clone())
                        .unwrap_or_else(|| "Folder".to_string());
                    eprintln!("MultiSampler folder load not yet implemented for folder: {}", folder_name);
                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                        node.user_data.sample_display_name = Some(format!("📁 {}", folder_name));
                    }
                }
            }
            graph_data::PendingSamplerLoad::MultiFromFilesystem { node_id, backend_node_id } => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aiff"])
                    .pick_file()
                {
                    let file_name = path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Sample".to_string());
                    let mut controller = controller_arc.lock().unwrap();
                    // Import into audio pool + asset library, then load from pool
                    match controller.import_audio_sync(path.to_path_buf()) {
                        Ok(pool_index) => {
                            let metadata = daw_backend::io::read_metadata(&path).ok();
                            let duration = metadata.as_ref().map(|m| m.duration).unwrap_or(0.0);
                            let clip = lightningbeam_core::clip::AudioClip::new_sampled(&file_name, pool_index, duration);
                            shared.action_executor.document_mut().add_audio_clip(clip);

                            // Add as layer spanning full key range
                            controller.multi_sampler_add_layer_from_pool(
                                backend_track_id, backend_node_id, pool_index,
                                0, 127, 60,
                            );
                        }
                        Err(e) => {
                            eprintln!("Failed to import audio '{}': {}", path.display(), e);
                        }
                    }
                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                        node.user_data.sample_display_name = Some(file_name);
                    }
                }
            }
            graph_data::PendingSamplerLoad::MultiFromFolderFilesystem { node_id, backend_node_id } => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    match crate::sample_import::scan_folder(&path) {
                        Ok(samples) => {
                            let scan_result = crate::sample_import::build_import_layers(samples);
                            let track_id = backend_track_id;
                            let dialog = crate::sample_import_dialog::SampleImportDialog::new(
                                path, scan_result, track_id, backend_node_id, node_id,
                            );
                            self.user_state.sample_import_dialog = Some(dialog);
                        }
                        Err(e) => {
                            eprintln!("Failed to scan folder '{}': {}", path.display(), e);
                        }
                    }
                }
            }
        }
    }

    fn handle_pending_script_sample_load(
        &mut self,
        load: graph_data::PendingScriptSampleLoad,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let backend_track_id = match self.backend_track_id {
            Some(id) => id,
            None => return,
        };
        let controller_arc = match &shared.audio_controller {
            Some(c) => std::sync::Arc::clone(c),
            None => return,
        };

        match load {
            graph_data::PendingScriptSampleLoad::FromPool { node_id, backend_node_id, slot_index, pool_index, name } => {
                let mut controller = controller_arc.lock().unwrap();
                match controller.get_pool_audio_samples(pool_index) {
                    Ok((samples, sample_rate, _channels)) => {
                        controller.send_command(daw_backend::Command::GraphSetScriptSample(
                            backend_track_id, backend_node_id, slot_index,
                            samples, sample_rate, name.clone(),
                        ));
                    }
                    Err(e) => {
                        eprintln!("Failed to get pool audio for script sample: {}", e);
                        return;
                    }
                }
                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                    node.user_data.script_sample_names.insert(slot_index, name);
                }
            }
            graph_data::PendingScriptSampleLoad::FromFile { node_id, backend_node_id, slot_index } => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Audio", &["wav", "flac", "mp3", "ogg", "aiff"])
                    .pick_file()
                {
                    let file_name = path.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Sample".to_string());

                    let mut controller = controller_arc.lock().unwrap();
                    match controller.import_audio_sync(path.to_path_buf()) {
                        Ok(pool_index) => {
                            // Add to document asset library
                            let metadata = daw_backend::io::read_metadata(&path).ok();
                            let duration = metadata.as_ref().map(|m| m.duration).unwrap_or(0.0);
                            let clip = lightningbeam_core::clip::AudioClip::new_sampled(&file_name, pool_index, duration);
                            shared.action_executor.document_mut().add_audio_clip(clip);

                            // Get the audio data and send to script node
                            match controller.get_pool_audio_samples(pool_index) {
                                Ok((samples, sample_rate, _channels)) => {
                                    controller.send_command(daw_backend::Command::GraphSetScriptSample(
                                        backend_track_id, backend_node_id, slot_index,
                                        samples, sample_rate, file_name.clone(),
                                    ));
                                }
                                Err(e) => {
                                    eprintln!("Failed to get pool audio for script sample: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to import audio '{}': {}", path.display(), e);
                        }
                    }
                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                        node.user_data.script_sample_names.insert(slot_index, file_name);
                    }
                }
            }
        }
    }

    fn check_parameter_changes(&mut self, shared: &mut crate::panes::SharedPaneState) {
        // Check all input parameters for value changes
        for (input_id, input_param) in &self.state.graph.inputs {
            // Only check parameters that can have constant values (not ConnectionOnly)
            if matches!(input_param.kind, InputParamKind::ConnectionOnly) {
                continue;
            }

            // Get current value and backend param ID
            let (current_value, backend_param_id) = match &input_param.value {
                ValueType::Float { value, backend_param_id, .. } => {
                    (*value, *backend_param_id)
                },
                _ => {
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
                // Value has changed — send update to backend
                if let Some(track_id) = self.track_id {
                    let node_id = input_param.node;

                    if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                        if let Some(param_id) = backend_param_id {
                            if let Some(va_id) = self.va_context() {
                                // Inside VA template — call template command directly
                                if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                    if let Some(audio_controller) = &shared.audio_controller {
                                        let mut controller = audio_controller.lock().unwrap();
                                        controller.graph_set_parameter_in_template(
                                            backend_track_id, va_id,
                                            backend_id.index(), param_id, current_value,
                                        );
                                    }
                                }
                            } else {
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

        // Send commands to backend: disconnect old, connect source→drag, connect drag→dest
        {
            let mut controller = audio_controller.lock().unwrap();
            controller.graph_disconnect(
                backend_track_id,
                src_backend.index(), src_port_idx,
                dst_backend.index(), dst_port_idx,
            );
            controller.graph_connect(
                backend_track_id,
                src_backend.index(), src_port_idx,
                drag_backend.index(), drag_input_port_idx,
            );
            controller.graph_connect(
                backend_track_id,
                drag_backend.index(), drag_output_port_idx,
                dst_backend.index(), dst_port_idx,
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

    /// Enter a subgraph for editing (VA template or Group internals)
    fn enter_subgraph(
        &mut self,
        context: SubgraphContext,
        shared: &mut crate::panes::SharedPaneState,
    ) {
        let is_va = matches!(context, SubgraphContext::VoiceAllocator { .. });

        // Only save/restore groups for VA transitions.
        // For Group transitions, groups persist in self so sub-groups aren't lost.
        let (saved_groups, saved_next_group_id) = if is_va {
            (Some(std::mem::take(&mut self.groups)), Some(std::mem::replace(&mut self.next_group_id, 1)))
        } else {
            (None, None)
        };

        // Save current editor state
        let saved = SavedGraphState {
            state: std::mem::replace(&mut self.state, GraphEditorState::new(1.0)),
            user_state: std::mem::replace(&mut self.user_state, GraphState::default()),
            node_id_map: std::mem::take(&mut self.node_id_map),
            backend_to_frontend_map: std::mem::take(&mut self.backend_to_frontend_map),
            parameter_values: std::mem::take(&mut self.parameter_values),
            groups: saved_groups,
            next_group_id: saved_next_group_id,
            group_placeholder_map: std::mem::take(&mut self.group_placeholder_map),
        };

        self.subgraph_stack.push(SubgraphFrame {
            context: context.clone(),
            saved_state: saved,
        });

        // Load the subgraph state from backend
        match &context {
            SubgraphContext::VoiceAllocator { backend_id, .. } => {
                if let Some(track_id) = self.track_id {
                    if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                        if let Some(audio_controller) = &shared.audio_controller {
                            let mut controller = audio_controller.lock().unwrap();
                            match controller.query_template_state(backend_track_id, backend_id.index()) {
                                Ok(json) => {
                                    if let Err(e) = self.load_graph_from_json(&json) {
                                        eprintln!("Failed to load template state: {}", e);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Failed to query template state: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            SubgraphContext::Group { .. } => {
                // Groups are frontend-only. Rebuild the view scoped to this group,
                // showing member nodes, sub-group placeholders, and boundary indicators.
                self.rebuild_view();
            }
        }
    }

    /// Exit the current subgraph level, restoring parent state
    fn exit_subgraph(&mut self) {
        if let Some(frame) = self.subgraph_stack.pop() {
            self.state = frame.saved_state.state;
            self.user_state = frame.saved_state.user_state;
            self.node_id_map = frame.saved_state.node_id_map;
            self.backend_to_frontend_map = frame.saved_state.backend_to_frontend_map;
            self.parameter_values = frame.saved_state.parameter_values;
            // Only restore groups if they were saved (VA transitions save them, Group transitions don't)
            if let Some(groups) = frame.saved_state.groups {
                self.groups = groups;
            }
            if let Some(next_id) = frame.saved_state.next_group_id {
                self.next_group_id = next_id;
            }
            self.group_placeholder_map = frame.saved_state.group_placeholder_map;
        }
    }

    /// Exit to a specific depth in the subgraph stack (0 = track level)
    fn exit_to_level(&mut self, target_depth: usize) {
        while self.subgraph_stack.len() > target_depth {
            self.exit_subgraph();
        }
    }

    /// Load graph state from a JSON string (used for both track graphs and subgraphs)
    fn load_graph_from_json(&mut self, json: &str) -> Result<(), String> {
        let graph_state: daw_backend::audio::node_graph::GraphPreset =
            serde_json::from_str(json).map_err(|e| format!("Failed to parse graph state: {}", e))?;

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
        self.pending_script_resolutions.clear();
        for node in &graph_state.nodes {
            let node_template = match NodeTemplate::from_backend_name(&node.node_type) {
                Some(t) => t,
                None => {
                    eprintln!("Unknown node type: {}", node.node_type);
                    continue;
                }
            };

            let frontend_id = self.add_node_to_editor(node_template, &node.node_type, node.position, node.id, &node.parameters);

            // For Script nodes: rebuild ports now (before connections), defer script_id resolution
            if node.node_type == "Script" {
                if let Some(ref source) = node.script_source {
                    if let Some(fid) = frontend_id {
                        // Rebuild ports/params immediately so connections map correctly
                        if let Ok(compiled) = beamdsp::compile(source) {
                            self.rebuild_script_node_ports(fid, &compiled);
                        }
                        // Defer script_id resolution to render loop (needs document access)
                        self.pending_script_resolutions.push((fid, source.clone()));
                    }
                }
            }
        }

        // Create connections in frontend
        for conn in &graph_state.connections {
            self.add_connection_to_editor(conn.from_node, conn.from_port, conn.to_node, conn.to_port);
        }

        // Restore groups from preset
        self.groups.clear();
        self.group_placeholder_map.clear();
        self.next_group_id = 1;
        if !graph_state.groups.is_empty() {
            for sg in &graph_state.groups {
                let group = GroupDef {
                    id: sg.id,
                    name: sg.name.clone(),
                    member_nodes: sg.member_nodes.clone(),
                    position: sg.position,
                    boundary_inputs: sg.boundary_inputs.iter().map(|bc| BoundaryConnection {
                        external_node: bc.external_node,
                        external_port: bc.external_port,
                        internal_node: bc.internal_node,
                        internal_port: bc.internal_port,
                        port_name: bc.port_name.clone(),
                        data_type: match bc.data_type.as_str() {
                            "Midi" => DataType::Midi,
                            "CV" => DataType::CV,
                            _ => DataType::Audio,
                        },
                    }).collect(),
                    boundary_outputs: sg.boundary_outputs.iter().map(|bc| BoundaryConnection {
                        external_node: bc.external_node,
                        external_port: bc.external_port,
                        internal_node: bc.internal_node,
                        internal_port: bc.internal_port,
                        port_name: bc.port_name.clone(),
                        data_type: match bc.data_type.as_str() {
                            "Midi" => DataType::Midi,
                            "CV" => DataType::CV,
                            _ => DataType::Audio,
                        },
                    }).collect(),
                    parent_group_id: sg.parent_group_id,
                };
                if sg.id >= self.next_group_id {
                    self.next_group_id = sg.id + 1;
                }
                self.groups.push(group);
            }
            // Rebuild the view to show group placeholders instead of member nodes
            self.rebuild_view();
        }

        Ok(())
    }

    /// Serialize a GroupDef to backend format
    fn serialize_group(g: &GroupDef) -> daw_backend::audio::node_graph::SerializedGroup {
        daw_backend::audio::node_graph::SerializedGroup {
            id: g.id,
            name: g.name.clone(),
            member_nodes: g.member_nodes.clone(),
            position: g.position,
            boundary_inputs: g.boundary_inputs.iter().map(|bc| {
                daw_backend::audio::node_graph::SerializedBoundaryConnection {
                    external_node: bc.external_node,
                    external_port: bc.external_port,
                    internal_node: bc.internal_node,
                    internal_port: bc.internal_port,
                    port_name: bc.port_name.clone(),
                    data_type: match bc.data_type {
                        DataType::Audio => "Audio".to_string(),
                        DataType::Midi => "Midi".to_string(),
                        DataType::CV => "CV".to_string(),
                    },
                }
            }).collect(),
            boundary_outputs: g.boundary_outputs.iter().map(|bc| {
                daw_backend::audio::node_graph::SerializedBoundaryConnection {
                    external_node: bc.external_node,
                    external_port: bc.external_port,
                    internal_node: bc.internal_node,
                    internal_port: bc.internal_port,
                    port_name: bc.port_name.clone(),
                    data_type: match bc.data_type {
                        DataType::Audio => "Audio".to_string(),
                        DataType::Midi => "Midi".to_string(),
                        DataType::CV => "CV".to_string(),
                    },
                }
            }).collect(),
            parent_group_id: g.parent_group_id,
        }
    }

    /// Serialize frontend groups to backend format and send to backend for persistence
    fn sync_groups_to_backend(&self, shared: &crate::panes::SharedPaneState) {
        let Some(track_id) = self.track_id else { return };
        let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) else { return };
        let Some(audio_controller) = &shared.audio_controller else { return };

        let serialized: Vec<_> = self.groups.iter().map(Self::serialize_group).collect();

        let mut controller = audio_controller.lock().unwrap();
        if let Some(va_id) = self.va_context() {
            controller.graph_set_groups_in_template(backend_track_id, va_id, serialized);
        } else {
            controller.graph_set_groups(backend_track_id, serialized);
        }
    }

    /// Get the VA backend node ID if we're editing inside a VoiceAllocator template.
    /// Searches the entire subgraph stack, not just the top — so a Group inside a VA
    /// still finds the VA context.
    fn va_context(&self) -> Option<u32> {
        for frame in self.subgraph_stack.iter().rev() {
            if let SubgraphContext::VoiceAllocator { backend_id, .. } = &frame.context {
                return Some(backend_id.index());
            }
        }
        None
    }

    /// Whether we're currently editing inside a subgraph
    fn in_subgraph(&self) -> bool {
        !self.subgraph_stack.is_empty()
    }

    /// True if any frame in the subgraph stack is a VoiceAllocator
    fn inside_voice_allocator(&self) -> bool {
        self.subgraph_stack.iter().any(|frame| {
            matches!(&frame.context, SubgraphContext::VoiceAllocator { .. })
        })
    }

    /// Get the GroupId of the current group scope (if inside a group), for filtering sub-groups.
    fn current_group_scope(&self) -> Option<GroupId> {
        self.subgraph_stack.last().and_then(|frame| {
            if let SubgraphContext::Group { group_id, .. } = &frame.context {
                Some(*group_id)
            } else {
                None
            }
        })
    }

    /// Build breadcrumb segments for the current subgraph stack
    fn breadcrumb_segments(&self) -> Vec<String> {
        let mut segments = vec!["Track Graph".to_string()];
        for frame in &self.subgraph_stack {
            match &frame.context {
                SubgraphContext::VoiceAllocator { .. } => segments.push("Voice Allocator".to_string()),
                SubgraphContext::Group { name, .. } => segments.push(format!("Group '{}'", name)),
            }
        }
        segments
    }

    /// Group the currently selected nodes into a new group
    fn group_selected_nodes(&mut self, shared: &mut crate::panes::SharedPaneState) {
        if self.state.selected_nodes.len() < 2 {
            return;
        }

        // Don't allow grouping group placeholders
        if self.state.selected_nodes.iter().any(|id| self.group_placeholder_map.contains_key(id)) {
            return;
        }

        // Collect selected backend IDs
        let selected_backend_ids: Vec<u32> = self.state.selected_nodes.iter()
            .filter_map(|fid| self.node_id_map.get(fid))
            .map(|bid| bid.index())
            .collect();

        if selected_backend_ids.is_empty() {
            return;
        }

        let selected_set: HashSet<u32> = selected_backend_ids.iter().copied().collect();

        // Find boundary connections by scanning all connections in the editor
        let mut boundary_inputs: Vec<BoundaryConnection> = Vec::new();
        let mut boundary_outputs: Vec<BoundaryConnection> = Vec::new();

        // Collect connection info: (input_id, vec of output_ids)
        let connections: Vec<(InputId, Vec<OutputId>)> = self.state.graph.connections.iter()
            .map(|(iid, oids)| (iid, oids.clone()))
            .collect();

        for (input_id, output_ids) in &connections {
            let to_node_fid = self.state.graph.inputs.get(*input_id).map(|p| p.node);
            for &output_id in output_ids {
                let from_node_fid = self.state.graph.outputs.get(output_id).map(|p| p.node);

                if let (Some(from_fid), Some(to_fid)) = (from_node_fid, to_node_fid) {
                    let from_bid = self.node_id_map.get(&from_fid)
                        .map(|b| b.index());
                    let to_bid = self.node_id_map.get(&to_fid)
                        .map(|b| b.index());

                    if let (Some(from_b), Some(to_b)) = (from_bid, to_bid) {
                        let from_in_group = selected_set.contains(&from_b);
                        let to_in_group = selected_set.contains(&to_b);

                        if !from_in_group && to_in_group {
                            // Boundary input: external → internal
                            let from_port = self.state.graph.nodes.get(from_fid)
                                .and_then(|n| n.outputs.iter().position(|(_, id)| *id == output_id))
                                .unwrap_or(0);
                            let to_port = self.state.graph.nodes.get(to_fid)
                                .and_then(|n| n.inputs.iter().position(|(_, id)| *id == *input_id))
                                .unwrap_or(0);

                            // Get port name from the input node's input label, and data type
                            let (port_name, data_type) = self.state.graph.nodes.get(to_fid)
                                .and_then(|n| n.inputs.get(to_port))
                                .map(|(name, iid)| {
                                    let dt = self.state.graph.inputs.get(*iid)
                                        .map(|p| p.typ)
                                        .unwrap_or(DataType::Audio);
                                    (name.clone(), dt)
                                })
                                .unwrap_or_else(|| ("In".to_string(), DataType::Audio));

                            boundary_inputs.push(BoundaryConnection {
                                external_node: from_b,
                                external_port: from_port,
                                internal_node: to_b,
                                internal_port: to_port,
                                port_name,
                                data_type,
                            });
                        } else if from_in_group && !to_in_group {
                            // Boundary output: internal → external
                            let from_port = self.state.graph.nodes.get(from_fid)
                                .and_then(|n| n.outputs.iter().position(|(_, id)| *id == output_id))
                                .unwrap_or(0);
                            let to_port = self.state.graph.nodes.get(to_fid)
                                .and_then(|n| n.inputs.iter().position(|(_, id)| *id == *input_id))
                                .unwrap_or(0);

                            // Get port name from the output node's output label, and data type
                            let (port_name, data_type) = self.state.graph.nodes.get(from_fid)
                                .and_then(|n| n.outputs.get(from_port))
                                .map(|(name, oid)| {
                                    let dt = self.state.graph.outputs.get(*oid)
                                        .map(|p| p.typ)
                                        .unwrap_or(DataType::Audio);
                                    (name.clone(), dt)
                                })
                                .unwrap_or_else(|| ("Out".to_string(), DataType::Audio));

                            boundary_outputs.push(BoundaryConnection {
                                external_node: to_b,
                                external_port: to_port,
                                internal_node: from_b,
                                internal_port: from_port,
                                port_name,
                                data_type,
                            });
                        }
                    }
                }
            }
        }

        // Calculate average position of selected nodes
        let mut sum_x = 0.0f32;
        let mut sum_y = 0.0f32;
        let mut count = 0;
        for &fid in &self.state.selected_nodes {
            if let Some(pos) = self.state.node_positions.get(fid) {
                sum_x += pos.x;
                sum_y += pos.y;
                count += 1;
            }
        }
        let position = if count > 0 {
            (sum_x / count as f32, sum_y / count as f32)
        } else {
            (0.0, 0.0)
        };

        // Inherit boundary connections from the parent group for any internal nodes
        // that are being included in this sub-group. This handles the case where
        // connections pass through the parent's Group Input/Output synthetic nodes
        // (which don't have backend IDs and are invisible to the editor scan above).
        if let Some(parent_gid) = self.current_group_scope() {
            if let Some(parent_group) = self.groups.iter().find(|g| g.id == parent_gid).cloned() {
                for bc in &parent_group.boundary_inputs {
                    if selected_set.contains(&bc.internal_node) {
                        // Check we don't already have this boundary from the editor scan
                        let already_exists = boundary_inputs.iter().any(|existing|
                            existing.internal_node == bc.internal_node &&
                            existing.internal_port == bc.internal_port &&
                            existing.external_node == bc.external_node &&
                            existing.external_port == bc.external_port
                        );
                        if !already_exists {
                            boundary_inputs.push(bc.clone());
                        }
                    }
                }
                for bc in &parent_group.boundary_outputs {
                    if selected_set.contains(&bc.internal_node) {
                        let already_exists = boundary_outputs.iter().any(|existing|
                            existing.internal_node == bc.internal_node &&
                            existing.internal_port == bc.internal_port &&
                            existing.external_node == bc.external_node &&
                            existing.external_port == bc.external_port
                        );
                        if !already_exists {
                            boundary_outputs.push(bc.clone());
                        }
                    }
                }
            }
        }

        let group = GroupDef {
            id: self.next_group_id,
            name: format!("Group {}", self.next_group_id),
            member_nodes: selected_backend_ids,
            position,
            boundary_inputs,
            boundary_outputs,
            parent_group_id: self.current_group_scope(),
        };
        self.next_group_id += 1;
        self.groups.push(group);

        // Rebuild the view to show the group placeholder
        self.rebuild_view();

        // Sync groups to backend for persistence
        self.sync_groups_to_backend(shared);
    }

    /// Ungroup a group, restoring member nodes to the current view.
    /// Also promotes any child groups to the current scope.
    fn ungroup(&mut self, group_id: GroupId, shared: &crate::panes::SharedPaneState) {
        let parent = self.groups.iter().find(|g| g.id == group_id).and_then(|g| g.parent_group_id);
        // Promote child groups: any group whose parent was the ungrouped group
        // now becomes a child of the ungrouped group's parent
        for g in &mut self.groups {
            if g.parent_group_id == Some(group_id) {
                g.parent_group_id = parent;
            }
        }
        self.groups.retain(|g| g.id != group_id);
        self.rebuild_view();
        self.sync_groups_to_backend(shared);
    }

    /// Rebuild the graph view, scope-aware for nested groups.
    /// - At top level (no group scope): shows all ungrouped nodes + root group placeholders
    /// - Inside a group: shows that group's member nodes (minus sub-group members) + sub-group placeholders + boundary indicators
    /// Context-aware: queries the template graph when inside a VA subgraph.
    fn rebuild_view(&mut self) {
        let backend = match &self.backend {
            Some(b) => b,
            None => return,
        };
        let json = if let Some(va_id) = self.va_context() {
            match backend.query_template_state(va_id) {
                Ok(json) => json,
                Err(e) => { eprintln!("Failed to query template state: {}", e); return; }
            }
        } else {
            match backend.get_state_json() {
                Ok(json) => json,
                Err(e) => { eprintln!("Failed to query backend: {}", e); return; }
            }
        };

        let graph_state: daw_backend::audio::node_graph::GraphPreset = match serde_json::from_str(&json) {
            Ok(state) => state,
            Err(e) => { eprintln!("Failed to parse graph state: {}", e); return; }
        };

        let current_scope = self.current_group_scope();

        // Determine which nodes are "in scope" (visible universe)
        let scope_members: Option<HashSet<u32>> = current_scope.and_then(|gid| {
            self.groups.iter().find(|g| g.id == gid)
                .map(|g| g.member_nodes.iter().copied().collect())
        });

        // Get groups relevant to this scope (direct children)
        let relevant_groups: Vec<GroupDef> = self.groups.iter()
            .filter(|g| g.parent_group_id == current_scope)
            .cloned()
            .collect();

        // Build set of node IDs hidden behind sub-group placeholders
        let sub_grouped_ids: HashSet<u32> = relevant_groups.iter()
            .flat_map(|g| g.member_nodes.iter().copied())
            .collect();

        // Clear editor state
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
        self.group_placeholder_map.clear();
        self.parameter_values.clear();

        // Add visible nodes: in scope, not hidden by sub-groups
        for node in &graph_state.nodes {
            // If inside a group, only include nodes that are members of that group
            if let Some(ref members) = scope_members {
                if !members.contains(&node.id) {
                    continue;
                }
            }
            // Skip nodes hidden behind sub-group placeholders
            if sub_grouped_ids.contains(&node.id) {
                continue;
            }

            let node_template = match NodeTemplate::from_backend_name(&node.node_type) {
                Some(t) => t,
                None => {
                    eprintln!("Unknown node type: {}", node.node_type);
                    continue;
                }
            };

            self.add_node_to_editor(node_template, &node.node_type, node.position, node.id, &node.parameters);
        }

        // Add sub-group placeholder nodes
        for group in &relevant_groups {
            let frontend_id = self.state.graph.nodes.insert(egui_node_graph2::Node {
                id: NodeId::default(),
                label: group.name.clone(),
                inputs: vec![],
                outputs: vec![],
                user_data: NodeData::new(NodeTemplate::Group),
            });

            // Add dynamic input ports based on boundary inputs
            for (i, bc) in group.boundary_inputs.iter().enumerate() {
                let name = if group.boundary_inputs.len() == 1 {
                    bc.port_name.clone()
                } else {
                    format!("{} {}", bc.port_name, i + 1)
                };
                self.state.graph.add_input_param(
                    frontend_id,
                    name.into(),
                    bc.data_type,
                    ValueType::float(0.0),
                    InputParamKind::ConnectionOnly,
                    true,
                );
            }

            // Add dynamic output ports based on boundary outputs
            for (i, bc) in group.boundary_outputs.iter().enumerate() {
                let name = if group.boundary_outputs.len() == 1 {
                    bc.port_name.clone()
                } else {
                    format!("{} {}", bc.port_name, i + 1)
                };
                self.state.graph.add_output_param(frontend_id, name.into(), bc.data_type);
            }

            self.state.node_positions.insert(frontend_id, egui::pos2(group.position.0, group.position.1));
            self.state.node_order.push(frontend_id);
            self.group_placeholder_map.insert(frontend_id, group.id);
        }

        // Add connections between visible nodes (skip connections involving sub-grouped nodes)
        for conn in &graph_state.connections {
            // If scoped, both endpoints must be in scope
            if let Some(ref members) = scope_members {
                if !members.contains(&conn.from_node) || !members.contains(&conn.to_node) {
                    continue;
                }
            }
            // Skip connections involving sub-grouped nodes
            if sub_grouped_ids.contains(&conn.from_node) || sub_grouped_ids.contains(&conn.to_node) {
                continue;
            }

            self.add_connection_to_editor(conn.from_node, conn.from_port, conn.to_node, conn.to_port);
        }

        // If inside a group, add synthetic Group Input / Group Output boundary indicator nodes
        // BEFORE wiring sub-group boundaries, so sub-groups can wire to these nodes.
        let mut group_input_fid: Option<NodeId> = None;
        let mut group_output_fid: Option<NodeId> = None;
        let scope_group = current_scope.and_then(|gid| {
            self.groups.iter().find(|g| g.id == gid).cloned()
        });

        if let Some(ref scope_group) = scope_group {
            // Group Input (for boundary inputs)
            if !scope_group.boundary_inputs.is_empty() {
                let min_x = graph_state.nodes.iter()
                    .filter(|n| scope_group.member_nodes.contains(&n.id))
                    .map(|n| n.position.0)
                    .fold(f32::INFINITY, f32::min);

                let gi_fid = self.state.graph.nodes.insert(egui_node_graph2::Node {
                    id: NodeId::default(),
                    label: "Group Input".to_string(),
                    inputs: vec![],
                    outputs: vec![],
                    user_data: NodeData::new(NodeTemplate::Group),
                });

                for bc in &scope_group.boundary_inputs {
                    self.state.graph.add_output_param(gi_fid, bc.port_name.clone().into(), bc.data_type);
                }

                self.state.node_positions.insert(gi_fid, egui::pos2(min_x - 250.0, 0.0));
                self.state.node_order.push(gi_fid);
                group_input_fid = Some(gi_fid);

                // Wire Group Input outputs to visible internal nodes (not sub-grouped)
                for (port_idx, bc) in scope_group.boundary_inputs.iter().enumerate() {
                    if sub_grouped_ids.contains(&bc.internal_node) {
                        continue; // Will be wired through sub-group placeholder below
                    }
                    let to_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(bc.internal_node as usize));
                    if let Some(&to_fid) = self.backend_to_frontend_map.get(&to_backend) {
                        if let Some(to_node) = self.state.graph.nodes.get(to_fid) {
                            if let Some((_name, input_id)) = to_node.inputs.get(bc.internal_port) {
                                if let Some(gi_node) = self.state.graph.nodes.get(gi_fid) {
                                    if let Some((_name, output_id)) = gi_node.outputs.get(port_idx) {
                                        if let Some(conns) = self.state.graph.connections.get_mut(*input_id) {
                                            conns.push(*output_id);
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

            // Group Output (for boundary outputs)
            if !scope_group.boundary_outputs.is_empty() {
                let max_x = graph_state.nodes.iter()
                    .filter(|n| scope_group.member_nodes.contains(&n.id))
                    .map(|n| n.position.0)
                    .fold(f32::NEG_INFINITY, f32::max);

                let go_fid = self.state.graph.nodes.insert(egui_node_graph2::Node {
                    id: NodeId::default(),
                    label: "Group Output".to_string(),
                    inputs: vec![],
                    outputs: vec![],
                    user_data: NodeData::new(NodeTemplate::Group),
                });

                for bc in &scope_group.boundary_outputs {
                    self.state.graph.add_input_param(
                        go_fid,
                        bc.port_name.clone().into(),
                        bc.data_type,
                        ValueType::float(0.0),
                        InputParamKind::ConnectionOnly,
                        true,
                    );
                }

                self.state.node_positions.insert(go_fid, egui::pos2(max_x + 250.0, 0.0));
                self.state.node_order.push(go_fid);
                group_output_fid = Some(go_fid);

                // Wire visible internal nodes to Group Output inputs (not sub-grouped)
                for (port_idx, bc) in scope_group.boundary_outputs.iter().enumerate() {
                    if sub_grouped_ids.contains(&bc.internal_node) {
                        continue; // Will be wired through sub-group placeholder below
                    }
                    let from_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(bc.internal_node as usize));
                    if let Some(&from_fid) = self.backend_to_frontend_map.get(&from_backend) {
                        if let Some(from_node) = self.state.graph.nodes.get(from_fid) {
                            if let Some((_name, output_id)) = from_node.outputs.get(bc.internal_port) {
                                if let Some(go_node) = self.state.graph.nodes.get(go_fid) {
                                    if let Some((_name, input_id)) = go_node.inputs.get(port_idx) {
                                        self.state.graph.connections.insert(*input_id, vec![*output_id]);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Add boundary connections to/from sub-group placeholders
        for group in &relevant_groups {
            let placeholder_fid = self.group_placeholder_map.iter()
                .find(|(_, gid)| **gid == group.id)
                .map(|(fid, _)| *fid);

            if let Some(placeholder_fid) = placeholder_fid {
                // Boundary inputs: external_node output → group input port
                for (port_idx, bc) in group.boundary_inputs.iter().enumerate() {
                    let from_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(bc.external_node as usize));
                    if let Some(&from_fid) = self.backend_to_frontend_map.get(&from_backend) {
                        // External node is visible in this scope — wire directly
                        if let Some(from_node) = self.state.graph.nodes.get(from_fid) {
                            if let Some((_name, output_id)) = from_node.outputs.get(bc.external_port) {
                                if let Some(placeholder_node) = self.state.graph.nodes.get(placeholder_fid) {
                                    if let Some((_name, input_id)) = placeholder_node.inputs.get(port_idx) {
                                        self.state.graph.connections.insert(*input_id, vec![*output_id]);
                                    }
                                }
                            }
                        }
                    } else if let (Some(ref sg), Some(gi_fid)) = (&scope_group, group_input_fid) {
                        // External node is outside scope — wire from Group Input instead.
                        // Find which Group Input port matches this boundary connection.
                        if let Some(gi_port_idx) = sg.boundary_inputs.iter().position(|sbc|
                            sbc.external_node == bc.external_node &&
                            sbc.external_port == bc.external_port &&
                            sbc.internal_node == bc.internal_node &&
                            sbc.internal_port == bc.internal_port
                        ) {
                            if let Some(gi_node) = self.state.graph.nodes.get(gi_fid) {
                                if let Some((_name, output_id)) = gi_node.outputs.get(gi_port_idx) {
                                    if let Some(placeholder_node) = self.state.graph.nodes.get(placeholder_fid) {
                                        if let Some((_name, input_id)) = placeholder_node.inputs.get(port_idx) {
                                            self.state.graph.connections.insert(*input_id, vec![*output_id]);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Boundary outputs: group output port → external_node input
                for (port_idx, bc) in group.boundary_outputs.iter().enumerate() {
                    let to_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(bc.external_node as usize));
                    if let Some(&to_fid) = self.backend_to_frontend_map.get(&to_backend) {
                        // External node is visible in this scope — wire directly
                        if let Some(to_node) = self.state.graph.nodes.get(to_fid) {
                            if let Some((_name, input_id)) = to_node.inputs.get(bc.external_port) {
                                if let Some(placeholder_node) = self.state.graph.nodes.get(placeholder_fid) {
                                    if let Some((_name, output_id)) = placeholder_node.outputs.get(port_idx) {
                                        if let Some(connections) = self.state.graph.connections.get_mut(*input_id) {
                                            connections.push(*output_id);
                                        } else {
                                            self.state.graph.connections.insert(*input_id, vec![*output_id]);
                                        }
                                    }
                                }
                            }
                        }
                    } else if let (Some(ref sg), Some(go_fid)) = (&scope_group, group_output_fid) {
                        // External node is outside scope — wire to Group Output instead.
                        if let Some(go_port_idx) = sg.boundary_outputs.iter().position(|sbc|
                            sbc.external_node == bc.external_node &&
                            sbc.external_port == bc.external_port &&
                            sbc.internal_node == bc.internal_node &&
                            sbc.internal_port == bc.internal_port
                        ) {
                            if let Some(placeholder_node) = self.state.graph.nodes.get(placeholder_fid) {
                                if let Some((_name, output_id)) = placeholder_node.outputs.get(port_idx) {
                                    if let Some(go_node) = self.state.graph.nodes.get(go_fid) {
                                        if let Some((_name, input_id)) = go_node.inputs.get(go_port_idx) {
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
            }
        }
    }

    /// Helper: add a node to the editor state and return its frontend ID
    fn add_node_to_editor(
        &mut self,
        node_template: NodeTemplate,
        label: &str,
        position: (f32, f32),
        backend_node_id: u32,
        parameters: &std::collections::HashMap<u32, f32>,
    ) -> Option<NodeId> {
        let frontend_id = self.state.graph.nodes.insert(egui_node_graph2::Node {
            id: NodeId::default(),
            label: label.to_string(),
            inputs: vec![],
            outputs: vec![],
            user_data: NodeData::new(node_template),
        });

        node_template.build_node(&mut self.state.graph, &mut self.user_state, frontend_id);

        self.state.node_positions.insert(frontend_id, egui::pos2(position.0, position.1));
        self.state.node_order.push(frontend_id);

        let backend_id = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(backend_node_id as usize));
        self.node_id_map.insert(frontend_id, backend_id);
        self.backend_to_frontend_map.insert(backend_id, frontend_id);

        // Set parameter values from backend
        if let Some(node_data) = self.state.graph.nodes.get(frontend_id) {
            let input_ids: Vec<InputId> = node_data.inputs.iter().map(|(_, id)| *id).collect();
            for input_id in input_ids {
                if let Some(input_param) = self.state.graph.inputs.get_mut(input_id) {
                    if let ValueType::Float { value, backend_param_id: Some(pid), .. } = &mut input_param.value {
                        if let Some(&backend_value) = parameters.get(pid) {
                            *value = backend_value as f32;
                        }
                    }
                }
            }
        }

        Some(frontend_id)
    }

    /// Helper: add a connection to the editor state
    fn add_connection_to_editor(&mut self, from_node: u32, from_port: usize, to_node: u32, to_port: usize) {
        let from_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(from_node as usize));
        let to_backend = BackendNodeId::Audio(petgraph::stable_graph::NodeIndex::new(to_node as usize));

        if let (Some(&from_fid), Some(&to_fid)) = (
            self.backend_to_frontend_map.get(&from_backend),
            self.backend_to_frontend_map.get(&to_backend),
        ) {
            if let Some(from_node) = self.state.graph.nodes.get(from_fid) {
                if let Some((_name, output_id)) = from_node.outputs.get(from_port) {
                    if let Some(to_node) = self.state.graph.nodes.get(to_fid) {
                        if let Some((_name, input_id)) = to_node.inputs.get(to_port) {
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
        let generation_changed = *shared.project_generation != self.last_project_generation;
        if generation_changed {
            self.last_project_generation = *shared.project_generation;
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
                            // Reload graph for new track — exit any subgraph editing and clear groups
                            self.subgraph_stack.clear();
                            self.groups.clear();
                            self.next_group_id = 1;
                            self.group_placeholder_map.clear();
                            self.track_id = Some(new_track_id);

                            // Recreate backend
                            self.backend_track_id = Some(backend_track_id);
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
            let bg_color = shared.theme.bg_color(&["#node-editor", ".pane-content"], ui.ctx(), egui::Color32::from_gray(30));
            painter.rect_filled(rect, 0.0, bg_color);

            let text = "Select a MIDI or Audio track to view its node graph";
            let font_id = egui::FontId::proportional(16.0);
            let text_color = shared.theme.text_color(&["#node-editor", ".text-secondary"], ui.ctx(), egui::Color32::from_gray(150));

            let galley = painter.layout_no_wrap(text.to_string(), font_id, text_color);
            let text_pos = rect.center() - galley.size() / 2.0;
            painter.galley(text_pos, galley, text_color);
            return;
        }
        // Poll oscilloscope data at ~20 FPS
        let has_oscilloscopes;
        if self.last_oscilloscope_poll.elapsed() >= std::time::Duration::from_millis(50) {
            self.last_oscilloscope_poll = std::time::Instant::now();

            // Find all Oscilloscope nodes in the current graph
            let oscilloscope_nodes: Vec<(NodeId, u32)> = self.state.graph.iter_nodes()
                .filter(|&node_id| {
                    self.state.graph.nodes.get(node_id)
                        .map(|n| n.user_data.template == NodeTemplate::Oscilloscope)
                        .unwrap_or(false)
                })
                .filter_map(|node_id| {
                    self.node_id_map.get(&node_id).and_then(|backend_id| {
                        match backend_id {
                            BackendNodeId::Audio(idx) => Some((node_id, idx.index() as u32)),
                        }
                    })
                })
                .collect();

            has_oscilloscopes = !oscilloscope_nodes.is_empty();

            if has_oscilloscopes {
                if let (Some(backend_track_id), Some(audio_controller)) = (self.backend_track_id, &shared.audio_controller) {
                    // Check if we're inside a VoiceAllocator subgraph
                    let va_backend_id = self.subgraph_stack.iter().rev().find_map(|frame| {
                        if let SubgraphContext::VoiceAllocator { backend_id } = &frame.context {
                            match backend_id {
                                BackendNodeId::Audio(idx) => Some(idx.index() as u32),
                            }
                        } else {
                            None
                        }
                    });

                    let mut controller = audio_controller.lock().unwrap();
                    for (node_id, backend_node_id) in oscilloscope_nodes {
                        // Calculate sample count from per-node time scale (default 100ms)
                        let time_ms = self.user_state.oscilloscope_time_scale
                            .get(&node_id).copied().unwrap_or(100.0);
                        let sample_count = ((time_ms / 1000.0) * 48000.0) as usize;
                        let result = if let Some(va_id) = va_backend_id {
                            controller.query_voice_oscilloscope_data(backend_track_id, va_id, backend_node_id, sample_count)
                        } else {
                            controller.query_oscilloscope_data(backend_track_id, backend_node_id, sample_count)
                        };
                        if let Ok(data) = result {
                            self.user_state.oscilloscope_data.insert(node_id, graph_data::OscilloscopeCache {
                                audio: data.audio,
                                cv: data.cv,
                            });
                        }
                    }
                }
            }
        } else {
            // Between polls, check if we have cached oscilloscope data
            has_oscilloscopes = !self.user_state.oscilloscope_data.is_empty();
        }

        // Continuously repaint when oscilloscopes are present
        if has_oscilloscopes {
            ui.ctx().request_repaint();
        }

        // Get colors from theme
        let bg_style = shared.theme.style(".node-graph-background", ui.ctx());
        let grid_style = shared.theme.style(".node-graph-grid", ui.ctx());

        let bg_color = bg_style.background_color().unwrap_or(egui::Color32::from_gray(45));
        let grid_color = grid_style.background_color().unwrap_or(egui::Color32::from_gray(55));

        // Draw breadcrumb bar when editing a subgraph
        let breadcrumb_height = if self.in_subgraph() { 28.0 } else { 0.0 };
        let graph_rect = if self.in_subgraph() {
            // Draw breadcrumb bar at top
            let breadcrumb_rect = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(rect.width(), breadcrumb_height),
            );
            let painter = ui.painter();
            let bc_bg = shared.theme.bg_color(&["#node-editor", ".pane-header"], ui.ctx(), egui::Color32::from_gray(35));
            painter.rect_filled(breadcrumb_rect, 0.0, bc_bg);
            let bc_border = shared.theme.border_color(&["#node-editor", ".pane-header"], ui.ctx(), egui::Color32::from_gray(60));
            painter.line_segment(
                [breadcrumb_rect.left_bottom(), breadcrumb_rect.right_bottom()],
                egui::Stroke::new(1.0, bc_border),
            );

            // Draw clickable breadcrumb segments
            let segments = self.breadcrumb_segments();
            let mut x = rect.min.x + 8.0;
            let y = rect.min.y + 6.0;
            let mut clicked_level: Option<usize> = None;

            for (i, segment) in segments.iter().enumerate() {
                let is_last = i == segments.len() - 1;
                let text_color = if is_last {
                    shared.theme.text_color(&["#node-editor", ".text-primary"], ui.ctx(), egui::Color32::from_gray(220))
                } else {
                    shared.theme.text_color(&["#node-editor", ".text-secondary"], ui.ctx(), egui::Color32::from_rgb(100, 180, 255))
                };

                let font_id = egui::FontId::proportional(13.0);
                let galley = painter.layout_no_wrap(segment.clone(), font_id, text_color);
                let text_rect = egui::Rect::from_min_size(egui::pos2(x, y), galley.size());

                if !is_last {
                    let response = ui.interact(text_rect, ui.id().with(("breadcrumb", i)), egui::Sense::click());
                    if response.clicked() {
                        clicked_level = Some(i);
                    }
                    if response.hovered() {
                        painter.rect_stroke(text_rect.expand(2.0), 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(80)), egui::StrokeKind::Outside);
                    }
                }

                painter.galley(egui::pos2(x, y), galley, text_color);
                x += text_rect.width();

                if !is_last {
                    let sep = " > ";
                    let sep_galley = painter.layout_no_wrap(sep.to_string(), egui::FontId::proportional(13.0), egui::Color32::from_gray(100));
                    painter.galley(egui::pos2(x, y), sep_galley, egui::Color32::from_gray(100));
                    x += 20.0;
                }
            }

            if let Some(level) = clicked_level {
                self.exit_to_level(level);
            }

            // Shrink graph rect to below breadcrumb
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, rect.min.y + breadcrumb_height),
                rect.max,
            )
        } else {
            rect
        };

        // Allocate the rect and render the graph editor within it
        ui.scope_builder(egui::UiBuilder::new().max_rect(graph_rect), |ui| {
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

            // Populate sampler clip list and node backend ID map for bottom_ui()
            {
                use lightningbeam_core::clip::AudioClipType;

                let doc = shared.action_executor.document();

                // Available audio clips
                self.user_state.available_clips = doc.audio_clips.values()
                    .filter_map(|clip| match &clip.clip_type {
                        AudioClipType::Sampled { audio_pool_index } => Some(graph_data::SamplerClipInfo {
                            name: clip.name.clone(),
                            pool_index: *audio_pool_index,
                        }),
                        _ => None,
                    })
                    .collect();
                self.user_state.available_clips.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

                // Available folders (with their contained audio clips)
                self.user_state.available_folders = doc.audio_folders.folders.values()
                    .map(|folder| {
                        let clips_in_folder: Vec<(String, usize)> = doc.audio_clips.values()
                            .filter(|clip| clip.folder_id == Some(folder.id))
                            .filter_map(|clip| match &clip.clip_type {
                                AudioClipType::Sampled { audio_pool_index } => Some((clip.name.clone(), *audio_pool_index)),
                                _ => None,
                            })
                            .collect();
                        graph_data::SamplerFolderInfo {
                            folder_id: folder.id,
                            name: folder.name.clone(),
                            clip_pool_indices: clips_in_folder,
                        }
                    })
                    .filter(|f| !f.clip_pool_indices.is_empty())
                    .collect();
                self.user_state.available_folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

                // Available scripts for Script node dropdown
                self.user_state.available_scripts = doc.script_definitions()
                    .map(|s| (s.id, s.name.clone()))
                    .collect();
                self.user_state.available_scripts.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

                // Bundled NAM models — populate from embedded registry
                if self.user_state.available_nam_models.is_empty() {
                    for name in daw_backend::audio::node_graph::nodes::bundled_models::bundled_model_names() {
                        self.user_state.available_nam_models.push(NamModelInfo {
                            name: name.to_string(),
                            path: format!("bundled:{}", name),
                            is_bundled: true,
                        });
                    }
                    self.user_state.available_nam_models.sort_by(|a, b| a.name.cmp(&b.name));
                }

                // Node backend ID map
                self.user_state.node_backend_ids = self.node_id_map.iter()
                    .map(|(&node_id, backend_id)| {
                        let id = match backend_id {
                            BackendNodeId::Audio(idx) => idx.index() as u32,
                        };
                        (node_id, id)
                    })
                    .collect();
            }

            // Draw dot grid background with pan/zoom
            let pan_zoom = &self.state.pan_zoom;
            Self::draw_dot_grid_background(ui, graph_rect, bg_color, grid_color, pan_zoom);

            // Draw the graph editor with context-aware node templates
            let graph_response = if self.inside_voice_allocator() {
                self.state.draw_graph_editor(
                    ui,
                    VoiceAllocatorNodeTemplates,
                    &mut self.user_state,
                    Vec::default(),
                )
            } else if self.in_subgraph() {
                self.state.draw_graph_editor(
                    ui,
                    SubgraphNodeTemplates,
                    &mut self.user_state,
                    Vec::default(),
                )
            } else {
                self.state.draw_graph_editor(
                    ui,
                    AllNodeTemplates,
                    &mut self.user_state,
                    Vec::default(),
                )
            };

            // Cache node rects for hit-testing, then handle response
            self.last_node_rects = graph_response.node_rects.clone();
            self.handle_graph_response(graph_response, shared, graph_rect);

            // Sync document-level focus with node graph selection
            if !self.state.selected_nodes.is_empty() {
                let node_indices: Vec<u32> = self.state.selected_nodes.iter()
                    .filter_map(|nid| self.node_id_map.get(nid))
                    .map(|bid| bid.index())
                    .collect();
                if !node_indices.is_empty() {
                    *shared.focus = lightningbeam_core::selection::FocusSelection::Nodes(node_indices);
                }
            }

            // Handle pending sampler load requests from bottom_ui()
            if let Some(load) = self.user_state.pending_sampler_load.take() {
                self.handle_pending_sampler_load(load, shared);
            }

            // Handle pending AmpSim model load from bottom_ui()
            if let Some(load) = self.user_state.pending_amp_sim_load.take() {
                if let Some(backend_track_id) = self.backend_track_id {
                    if let Some(controller_arc) = &shared.audio_controller {
                        match load {
                            PendingAmpSimLoad::FromPath { node_id, backend_node_id, path, name } => {
                                controller_arc.lock().unwrap().amp_sim_load_model(
                                    backend_track_id, backend_node_id, path,
                                );
                                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                                    node.user_data.nam_model_name = Some(name);
                                }
                            }
                            PendingAmpSimLoad::FromFile { node_id, backend_node_id } => {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("NAM Model", &["nam"])
                                    .pick_file()
                                {
                                    let model_name = path.file_stem()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Model".to_string());
                                    controller_arc.lock().unwrap().amp_sim_load_model(
                                        backend_track_id,
                                        backend_node_id,
                                        path.to_string_lossy().to_string(),
                                    );
                                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                                        node.user_data.nam_model_name = Some(model_name.clone());
                                    }
                                    // Add user-loaded model to the available list if not already present
                                    let path_str = path.to_string_lossy().to_string();
                                    if !self.user_state.available_nam_models.iter().any(|m| m.path == path_str) {
                                        self.user_state.available_nam_models.push(NamModelInfo {
                                            name: model_name,
                                            path: path_str,
                                            is_bundled: false,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Render sample import dialog if active
            if let Some(dialog) = &mut self.user_state.sample_import_dialog {
                let still_open = dialog.show(ui.ctx());
                if !still_open {
                    // Dialog closed — check if confirmed
                    let dialog = self.user_state.sample_import_dialog.take().unwrap();
                    if dialog.confirmed {
                        let backend_track_id = dialog.track_id;
                        let backend_node_id = dialog.backend_node_id;
                        let node_id = dialog.node_id;
                        let loop_mode = dialog.loop_mode;
                        let enabled_layers: Vec<_> = dialog.scan_result.layers.iter()
                            .filter(|l| l.enabled)
                            .collect();
                        let layer_count = enabled_layers.len();
                        let folder_name = dialog.folder_path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Folder".to_string());

                        if let Some(controller_arc) = &shared.audio_controller {
                            let mut controller = controller_arc.lock().unwrap();
                            // Clear existing layers before importing new ones
                            controller.multi_sampler_clear_layers(backend_track_id, backend_node_id);
                            for layer in &enabled_layers {
                                controller.multi_sampler_add_layer(
                                    backend_track_id,
                                    backend_node_id,
                                    layer.path.to_string_lossy().to_string(),
                                    layer.key_min,
                                    layer.key_max,
                                    layer.root_key,
                                    layer.velocity_min,
                                    layer.velocity_max,
                                    None, None, // loop points auto-detected by backend
                                    loop_mode,
                                );
                            }
                        }

                        if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                            node.user_data.sample_display_name = Some(
                                format!("{} ({} layers)", folder_name, layer_count)
                            );
                        }
                    }
                }
            }

            // Handle pending script sample load requests from bottom_ui()
            if let Some(load) = self.user_state.pending_script_sample_load.take() {
                self.handle_pending_script_sample_load(load, shared);
            }

            // Handle pending root note changes
            if !self.user_state.pending_root_note_changes.is_empty() {
                let changes: Vec<_> = self.user_state.pending_root_note_changes.drain(..).collect();
                if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                    if let Some(controller_arc) = &shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        for (node_id, backend_node_id, root_note) in changes {
                            controller.sampler_set_root_note(backend_track_id, backend_node_id, root_note);
                            if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                                node.user_data.root_note = root_note;
                            }
                        }
                    }
                }
            }

            // Handle pending sequencer grid changes
            if !self.user_state.pending_sequencer_changes.is_empty() {
                let changes: Vec<_> = self.user_state.pending_sequencer_changes.drain(..).collect();
                if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                    if let Some(controller_arc) = &shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        for (node_id, param_id, value) in changes {
                            // Send to backend
                            if let Some(backend_id) = self.node_id_map.get(&node_id) {
                                controller.graph_set_parameter(backend_track_id, backend_id.index(), param_id, value);
                            }
                            // Update frontend graph value
                            let row_name = format!("Row{}", param_id - 7);
                            if let Ok(input_id) = self.state.graph[node_id].get_input(&row_name) {
                                if let ValueType::Float { value: ref mut v, .. } = self.state.graph.inputs[input_id].value {
                                    *v = value;
                                }
                            }
                        }
                    }
                }
            }

            // Handle param changes from draw block (canvas knob drag etc.)
            if !self.user_state.pending_draw_param_changes.is_empty() {
                let changes: Vec<_> = self.user_state.pending_draw_param_changes.drain(..).collect();
                if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                    if let Some(controller_arc) = &shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        for (node_id, param_id, value) in changes {
                            // Send to backend
                            if let Some(backend_id) = self.node_id_map.get(&node_id) {
                                controller.graph_set_parameter(backend_track_id, backend_id.index(), param_id, value);
                            }
                            // Update frontend graph input port value
                            if let Some(node) = self.state.graph.nodes.get(node_id) {
                                for (_name, input_id) in &node.inputs {
                                    if let ValueType::Float { value: ref mut v, backend_param_id: Some(pid), .. } = self.state.graph.inputs[*input_id].value {
                                        if pid == param_id {
                                            *v = value;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Resolve Script nodes loaded from preset: find or create ScriptDefinitions
            // (ports were already rebuilt during load_graph_from_json, this just sets script_id)
            if !self.pending_script_resolutions.is_empty() {
                let resolutions = std::mem::take(&mut self.pending_script_resolutions);
                for (node_id, source) in resolutions {
                    // Try to find an existing ScriptDefinition with matching source
                    let existing_id = shared.action_executor.document()
                        .script_definitions()
                        .find(|s| s.source == source)
                        .map(|s| s.id);

                    let script_id = if let Some(id) = existing_id {
                        id
                    } else {
                        // Create a new ScriptDefinition from the source
                        use lightningbeam_core::script::ScriptDefinition;
                        let name = beamdsp::compile(&source)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|_| "Imported Script".to_string());
                        let script = ScriptDefinition::new(name, source.clone());
                        let id = script.id;
                        shared.action_executor.document_mut().add_script_definition(script);
                        id
                    };

                    // Set script_id on the node
                    if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                        node.user_data.script_id = Some(script_id);
                    }
                }
            }

            // Handle pending script assignment from Script node dropdown
            if let Some((node_id, script_id)) = self.user_state.pending_script_assignment.take() {
                // Update the node's script_id
                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                    node.user_data.script_id = Some(script_id);
                }
                // Look up script source, compile locally to rebuild ports, and send to backend
                let source = shared.action_executor.document()
                    .get_script_definition(&script_id)
                    .map(|s| s.source.clone());
                if let Some(source) = source {
                    // Compile locally to get port info and rebuild the node UI
                    if let Ok(compiled) = beamdsp::compile(&source) {
                        self.rebuild_script_node_ports(node_id, &compiled);
                    }
                    if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                        if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                            if let Some(controller_arc) = &shared.audio_controller {
                                let mut controller = controller_arc.lock().unwrap();
                                controller.send_command(daw_backend::Command::GraphSetScript(
                                    backend_track_id, backend_id.index(), source,
                                ));
                            }
                        }
                    }
                }
            }

            // Handle "New script..." from dropdown
            if let Some(node_id) = self.user_state.pending_new_script.take() {
                use lightningbeam_core::script::ScriptDefinition;
                let script = ScriptDefinition::new(
                    "New Script".to_string(),
                    "name \"New Script\"\ncategory effect\n\ninputs {\n    audio_in: audio\n}\n\noutputs {\n    audio_out: audio\n}\n\nprocess {\n    for i in 0..buffer_size {\n        audio_out[i * 2] = audio_in[i * 2];\n        audio_out[i * 2 + 1] = audio_in[i * 2 + 1];\n    }\n}\n".to_string(),
                );
                let script_id = script.id;
                shared.action_executor.document_mut().add_script_definition(script);
                if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                    node.user_data.script_id = Some(script_id);
                }
                // Open in editor
                *shared.script_to_edit = Some(script_id);
            }

            // Handle "Load from file..." from dropdown
            if let Some(node_id) = self.user_state.pending_load_script_file.take() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Load BeamDSP Script")
                    .add_filter("BeamDSP Script", &["bdsp"])
                    .pick_file()
                {
                    if let Ok(source) = std::fs::read_to_string(&path) {
                        use lightningbeam_core::script::ScriptDefinition;
                        let name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Imported Script")
                            .to_string();
                        let script = ScriptDefinition::new(name, source.clone());
                        let script_id = script.id;
                        shared.action_executor.document_mut().add_script_definition(script);
                        if let Some(node) = self.state.graph.nodes.get_mut(node_id) {
                            node.user_data.script_id = Some(script_id);
                        }
                        // Compile locally to rebuild ports, then send to backend
                        if let Ok(compiled) = beamdsp::compile(&source) {
                            self.rebuild_script_node_ports(node_id, &compiled);
                        }
                        if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                            if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                                if let Some(controller_arc) = &shared.audio_controller {
                                    let mut controller = controller_arc.lock().unwrap();
                                    controller.send_command(daw_backend::Command::GraphSetScript(
                                        backend_track_id, backend_id.index(), source,
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // Handle script_saved: auto-recompile all Script nodes using that script
            if let Some(saved_script_id) = shared.script_saved.take() {
                let source = shared.action_executor.document()
                    .get_script_definition(&saved_script_id)
                    .map(|s| s.source.clone());
                if let Some(source) = source {
                    // Compile locally to get updated port info
                    let compiled = beamdsp::compile(&source).ok();

                    // Collect matching node IDs first (can't mutate graph while iterating)
                    let matching_nodes: Vec<NodeId> = self.state.graph.nodes.iter()
                        .filter(|(_, node)| node.user_data.script_id == Some(saved_script_id))
                        .map(|(id, _)| id)
                        .collect();

                    // Rebuild ports for all matching nodes
                    if let Some(ref compiled) = compiled {
                        for &node_id in &matching_nodes {
                            self.rebuild_script_node_ports(node_id, compiled);
                        }
                    }

                    // Send to backend
                    if let Some(backend_track_id) = self.track_id.and_then(|tid| shared.layer_to_track_map.get(&tid).copied()) {
                        if let Some(controller_arc) = &shared.audio_controller {
                            let mut controller = controller_arc.lock().unwrap();
                            for &node_id in &matching_nodes {
                                if let Some(&backend_id) = self.node_id_map.get(&node_id) {
                                    controller.send_command(daw_backend::Command::GraphSetScript(
                                        backend_track_id, backend_id.index(), source.clone(),
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // Detect right-click on nodes — intercept the library's node finder and show our context menu instead
            {
                let secondary_clicked = ui.input(|i| i.pointer.secondary_released());
                if secondary_clicked {
                    if let Some(cursor_pos) = ui.input(|i| i.pointer.latest_pos()) {
                        // Hit-test against actual rendered node rects
                        for (&fid, &node_rect) in &self.last_node_rects {
                            if node_rect.contains(cursor_pos) {
                                self.state.node_finder = None;
                                self.node_context_menu = Some((fid, cursor_pos));
                                break;
                            }
                        }
                    }
                }
            }

            // Draw node context menu
            if let Some((ctx_node_id, menu_pos)) = self.node_context_menu {
                let is_group = self.group_placeholder_map.contains_key(&ctx_node_id);
                let group_id = self.group_placeholder_map.get(&ctx_node_id).copied();
                let mut close_menu = false;
                let mut action_delete = false;
                let mut action_ungroup = false;
                let mut action_rename = false;
                let mut action_edit_script = false;

                let is_script_node = self.state.graph.nodes.get(ctx_node_id)
                    .map(|n| n.user_data.template == NodeTemplate::Script)
                    .unwrap_or(false);

                let menu_response = egui::Area::new(ui.id().with("node_context_menu"))
                    .fixed_pos(menu_pos)
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(120.0);
                            if is_group {
                                if ui.button("Rename Group").clicked() {
                                    action_rename = true;
                                    close_menu = true;
                                }
                                if ui.button("Ungroup").clicked() {
                                    action_ungroup = true;
                                    close_menu = true;
                                }
                                ui.separator();
                            }
                            if is_script_node {
                                if ui.button("Edit Script").clicked() {
                                    action_edit_script = true;
                                    close_menu = true;
                                }
                                ui.separator();
                            }
                            if ui.button("Delete").clicked() {
                                action_delete = true;
                                close_menu = true;
                            }
                        });
                    });

                // Close menu on click outside the menu area
                let menu_rect = menu_response.response.rect;
                let clicked_outside = ui.input(|i| {
                    i.pointer.any_pressed()
                        && i.pointer.latest_pos()
                            .map(|p| !menu_rect.contains(p))
                            .unwrap_or(false)
                });
                if clicked_outside {
                    close_menu = true;
                }

                if action_rename {
                    if let Some(gid) = group_id {
                        if let Some(group) = self.groups.iter().find(|g| g.id == gid) {
                            self.renaming_group = Some((gid, group.name.clone()));
                        }
                    }
                }
                if action_ungroup {
                    if let Some(gid) = group_id {
                        self.ungroup(gid, shared);
                    }
                }
                if action_delete {
                    if is_group {
                        if let Some(gid) = group_id {
                            self.groups.retain(|g| g.id != gid);
                            self.rebuild_view();
                            self.sync_groups_to_backend(shared);
                        }
                    } else {
                        // Delete the node via the graph - queue the deletion
                        if let Some(track_id) = self.track_id {
                            if let Some(&backend_id) = self.node_id_map.get(&ctx_node_id) {
                                if let Some(va_id) = self.va_context() {
                                    if let Some(&backend_track_id) = shared.layer_to_track_map.get(&track_id) {
                                        if let Some(audio_controller) = &shared.audio_controller {
                                            let mut controller = audio_controller.lock().unwrap();
                                            controller.graph_remove_node_from_template(
                                                backend_track_id, va_id, backend_id.index(),
                                            );
                                        }
                                    }
                                } else {
                                    let action = Box::new(actions::NodeGraphAction::RemoveNode(
                                        actions::RemoveNodeAction::new(track_id, backend_id)
                                    ));
                                    self.pending_action = Some(action);
                                }
                                // Remove from editor state
                                self.state.graph.nodes.remove(ctx_node_id);
                                self.node_id_map.remove(&ctx_node_id);
                                self.backend_to_frontend_map.remove(&backend_id);
                            }
                        }
                    }
                }
                if action_edit_script {
                    if let Some(script_id) = self.state.graph.nodes.get(ctx_node_id)
                        .and_then(|n| n.user_data.script_id)
                    {
                        *shared.script_to_edit = Some(script_id);
                    }
                }
                if close_menu {
                    self.node_context_menu = None;
                }
            }

            // Draw group rename popup
            if let Some((group_id, ref mut new_name)) = self.renaming_group.clone() {
                let mut close_rename = false;
                let mut apply_rename = false;
                let mut name_buf = new_name.clone();

                let center = rect.center();
                egui::Area::new(ui.id().with("group_rename_popup"))
                    .fixed_pos(egui::pos2(center.x - 100.0, center.y - 30.0))
                    .order(egui::Order::Foreground)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.set_min_width(200.0);
                            ui.label("Rename Group:");
                            let response = ui.text_edit_singleline(&mut name_buf);
                            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                apply_rename = true;
                            }
                            ui.horizontal(|ui| {
                                if ui.button("OK").clicked() {
                                    apply_rename = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close_rename = true;
                                }
                            });
                            // Auto-focus the text field
                            response.request_focus();
                        });
                    });

                if apply_rename {
                    if let Some(group) = self.groups.iter_mut().find(|g| g.id == group_id) {
                        group.name = name_buf.clone();
                    }
                    // Update the placeholder node's label
                    for (&fid, &gid) in &self.group_placeholder_map {
                        if gid == group_id {
                            if let Some(node) = self.state.graph.nodes.get_mut(fid) {
                                node.label = name_buf;
                            }
                            break;
                        }
                    }
                    self.renaming_group = None;
                    self.sync_groups_to_backend(shared);
                } else if close_rename {
                    self.renaming_group = None;
                } else {
                    self.renaming_group = Some((group_id, name_buf));
                }
            }

            // Handle group/ungroup commands from global MenuAction dispatch
            if *shared.pending_node_group {
                *shared.pending_node_group = false;
                if !self.state.selected_nodes.is_empty() {
                    self.group_selected_nodes(shared);
                }
            }
            if *shared.pending_node_ungroup {
                *shared.pending_node_ungroup = false;
                let group_ids_to_ungroup: Vec<GroupId> = self.state.selected_nodes.iter()
                    .filter_map(|fid| self.group_placeholder_map.get(fid).copied())
                    .collect();
                for gid in group_ids_to_ungroup {
                    self.ungroup(gid, shared);
                }
            }

            // Handle pane-local keyboard shortcuts (only when pointer is over this pane)
            if ui.rect_contains_pointer(rect) {
                // F2 to rename selected group
                let f2 = ui.input(|i| shared.keymap.action_pressed(crate::keymap::AppAction::NodeGraphRename, i));
                if f2 && self.renaming_group.is_none() {
                    // Find the first selected group placeholder
                    if let Some(group_id) = self.state.selected_nodes.iter()
                        .find_map(|fid| self.group_placeholder_map.get(fid).copied())
                    {
                        if let Some(group) = self.groups.iter().find(|g| g.id == group_id) {
                            self.renaming_group = Some((group_id, group.name.clone()));
                        }
                    }
                }
            }

            // Check for parameter value changes and send updates to backend
            self.check_parameter_changes(shared);

            // Execute any parameter change actions
            self.execute_pending_action(shared);

            // Insert-node-on-connection: find target during drag, highlight, and execute on drop
            let primary_down = ui.input(|i| i.pointer.primary_down());
            if let Some(dragged) = self.dragging_node {
                if primary_down {
                    // Still dragging — check for nearby compatible connection
                    if let Some((input_id, output_id, _src_graph, _dst_graph)) = self.find_insert_target(dragged) {
                        self.insert_target = Some((input_id, output_id));
                        self.state.highlighted_connection = Some((input_id, output_id));
                    } else {
                        self.insert_target = None;
                        self.state.highlighted_connection = None;
                    }
                } else {
                    // Drag ended — execute insertion if we have a target
                    self.state.highlighted_connection = None;
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
