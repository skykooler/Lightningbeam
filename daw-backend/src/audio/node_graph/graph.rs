use super::node_trait::AudioNode;
use super::types::{ConnectionError, SignalType};
use crate::audio::midi::MidiEvent;
use petgraph::algo::has_path_connecting;
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::{EdgeRef, IntoEdgeReferences};
use petgraph::Direction;

/// Connection information between nodes
#[derive(Debug, Clone)]
pub struct Connection {
    pub from_port: usize,
    pub to_port: usize,
}

/// Wrapper for audio nodes in the graph
pub struct GraphNode {
    pub node: Box<dyn AudioNode>,
    /// Buffers for each audio/CV output port
    pub output_buffers: Vec<Vec<f32>>,
    /// Buffers for each MIDI output port
    pub midi_output_buffers: Vec<Vec<MidiEvent>>,
}

impl std::fmt::Debug for GraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphNode")
            .field("node", &"<AudioNode>")
            .field("output_buffers_len", &self.output_buffers.len())
            .field("midi_output_buffers_len", &self.midi_output_buffers.len())
            .finish()
    }
}

impl GraphNode {
    pub fn new(node: Box<dyn AudioNode>, buffer_size: usize) -> Self {
        let outputs = node.outputs();

        // Allocate buffers based on signal type
        // Audio signals are stereo (2 samples per frame), CV is mono (1 sample per frame)
        let mut output_buffers = Vec::new();
        let mut midi_output_buffers = Vec::new();

        for port in outputs.iter() {
            match port.signal_type {
                SignalType::Audio => {
                    output_buffers.push(vec![0.0; buffer_size * 2]); // Stereo (interleaved L/R)
                }
                SignalType::CV => {
                    output_buffers.push(vec![0.0; buffer_size]); // Mono
                }
                SignalType::Midi => {
                    output_buffers.push(vec![]); // Placeholder for indexing alignment
                    let mut midi_buf = Vec::new();
                    midi_buf.reserve(128); // Max 128 MIDI events per cycle
                    midi_output_buffers.push(midi_buf);
                }
            }
        }

        Self {
            node,
            output_buffers,
            midi_output_buffers,
        }
    }
}

/// Audio processing graph for instruments/effects
#[derive(Debug)]
pub struct AudioGraph {
    /// The audio graph (StableGraph allows node removal without index invalidation)
    graph: StableGraph<GraphNode, Connection>,

    /// MIDI input mapping (which nodes receive MIDI)
    midi_targets: Vec<NodeIndex>,

    /// Audio output node index (where we read final audio)
    output_node: Option<NodeIndex>,

    /// Sample rate
    sample_rate: u32,

    /// Buffer size for internal processing
    buffer_size: usize,

    /// Temporary buffers for node audio/CV inputs during processing
    input_buffers: Vec<Vec<f32>>,

    /// Temporary buffers for node MIDI inputs during processing
    midi_input_buffers: Vec<Vec<MidiEvent>>,

    /// UI positions for nodes (node_index -> (x, y))
    node_positions: std::collections::HashMap<u32, (f32, f32)>,

    /// Current playback time (for automation nodes)
    playback_time: f64,

    /// Project tempo (synced from Engine via SetTempo)
    bpm: f32,
    /// Beats per bar (time signature numerator)
    beats_per_bar: u32,

    /// Cached topological sort order (invalidated on graph mutation)
    topo_cache: Option<Vec<NodeIndex>>,

    /// Frontend-only group definitions (stored opaquely for persistence)
    frontend_groups: Vec<crate::audio::node_graph::preset::SerializedGroup>,
}

impl AudioGraph {
    /// Create a new empty audio graph
    pub fn new(sample_rate: u32, buffer_size: usize) -> Self {
        Self {
            graph: StableGraph::new(),
            midi_targets: Vec::new(),
            output_node: None,
            sample_rate,
            buffer_size,
            // Pre-allocate input buffers with stereo size (2x) to accommodate Audio signals
            // CV signals will only use the first half
            input_buffers: vec![vec![0.0; buffer_size * 2]; 16],
            // Pre-allocate MIDI input buffers (max 128 events per port)
            midi_input_buffers: (0..16).map(|_| Vec::with_capacity(128)).collect(),
            node_positions: std::collections::HashMap::new(),
            playback_time: 0.0,
            bpm: 120.0,
            beats_per_bar: 4,
            topo_cache: None,
            frontend_groups: Vec::new(),
        }
    }

    /// Set the project tempo and time signature for BeatNodes
    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        self.bpm = bpm;
        self.beats_per_bar = beats_per_bar;
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeIndex {
        let graph_node = GraphNode::new(node, self.buffer_size);
        self.topo_cache = None;
        self.graph.add_node(graph_node)
    }

    /// Get the number of nodes in the graph
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Set the UI position for a node
    pub fn set_node_position(&mut self, node: NodeIndex, x: f32, y: f32) {
        self.node_positions.insert(node.index() as u32, (x, y));
    }

    /// Get the UI position for a node
    pub fn get_node_position(&self, node: NodeIndex) -> Option<(f32, f32)> {
        self.node_positions.get(&(node.index() as u32)).copied()
    }

    /// Connect two nodes with type checking
    pub fn connect(
        &mut self,
        from: NodeIndex,
        from_port: usize,
        to: NodeIndex,
        to_port: usize,
    ) -> Result<(), ConnectionError> {
        // Check if this exact connection already exists
        if let Some(edge_idx) = self.graph.find_edge(from, to) {
            let existing_conn = &self.graph[edge_idx];
            if existing_conn.from_port == from_port && existing_conn.to_port == to_port {
                return Ok(()); // Connection already exists, don't create duplicate
            }
        }

        // Validate the connection
        self.validate_connection(from, from_port, to, to_port)?;

        // Remove any existing connection to the same input port (replace semantics).
        // The frontend UI enforces single-connection inputs, so when a new connection
        // targets the same port, the old one should be replaced.
        let edges_to_remove: Vec<_> = self.graph.edges_directed(to, petgraph::Direction::Incoming)
            .filter(|e| e.weight().to_port == to_port)
            .map(|e| e.id())
            .collect();
        for edge_id in edges_to_remove {
            self.graph.remove_edge(edge_id);
        }

        // Add the edge
        self.graph.add_edge(from, to, Connection { from_port, to_port });
        self.topo_cache = None;

        Ok(())
    }

    /// Disconnect two nodes
    pub fn disconnect(
        &mut self,
        from: NodeIndex,
        from_port: usize,
        to: NodeIndex,
        to_port: usize,
    ) {
        // Find and remove the edge
        if let Some(edge_idx) = self.graph.find_edge(from, to) {
            let conn = &self.graph[edge_idx];
            if conn.from_port == from_port && conn.to_port == to_port {
                self.graph.remove_edge(edge_idx);
                self.topo_cache = None;
            }
        }
    }

    /// Remove a node from the graph
    pub fn remove_node(&mut self, node: NodeIndex) {
        self.graph.remove_node(node);
        self.topo_cache = None;

        // Update MIDI targets
        self.midi_targets.retain(|&idx| idx != node);

        // Update output node
        if self.output_node == Some(node) {
            self.output_node = None;
        }
    }

    /// Validate a connection is type-compatible and wouldn't create a cycle
    fn validate_connection(
        &self,
        from: NodeIndex,
        from_port: usize,
        to: NodeIndex,
        to_port: usize,
    ) -> Result<(), ConnectionError> {
        // Check nodes exist
        let from_node = self.graph.node_weight(from).ok_or(ConnectionError::InvalidPort)?;
        let to_node = self.graph.node_weight(to).ok_or(ConnectionError::InvalidPort)?;

        // Check ports are valid
        let from_outputs = from_node.node.outputs();
        let to_inputs = to_node.node.inputs();

        if from_port >= from_outputs.len() || to_port >= to_inputs.len() {
            return Err(ConnectionError::InvalidPort);
        }

        // Check signal types match
        let from_type = from_outputs[from_port].signal_type;
        let to_type = to_inputs[to_port].signal_type;

        if from_type != to_type {
            return Err(ConnectionError::TypeMismatch {
                expected: to_type,
                got: from_type,
            });
        }

        // Check for cycles: if there's already a path from 'to' to 'from',
        // then adding 'from' -> 'to' would create a cycle
        if has_path_connecting(&self.graph, to, from, None) {
            return Err(ConnectionError::WouldCreateCycle);
        }

        Ok(())
    }

    /// Set which node receives MIDI events
    pub fn set_midi_target(&mut self, node: NodeIndex, enabled: bool) {
        if enabled {
            if !self.midi_targets.contains(&node) {
                self.midi_targets.push(node);
            }
        } else {
            self.midi_targets.retain(|&idx| idx != node);
        }
    }

    /// Set the output node (where final audio is read from)
    pub fn set_output_node(&mut self, node: Option<NodeIndex>) {
        self.output_node = node;
    }

    /// Add a node to a VoiceAllocator's template graph
    pub fn add_node_to_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        node: Box<dyn AudioNode>,
    ) -> Result<u32, String> {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        // Get the VoiceAllocator node
        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // We need to downcast to VoiceAllocatorNode
            // This is tricky with trait objects, so we'll need to use Any
            // For now, let's use a different approach - store the node pointer temporarily

            // Downcast to VoiceAllocatorNode using safe Any trait
            let va = graph_node.node.as_any_mut()
                .downcast_mut::<VoiceAllocatorNode>()
                .ok_or_else(|| "Node is not a VoiceAllocator".to_string())?;

            // Add node to template graph
            let node_idx = va.template_graph_mut().add_node(node);
            let node_id = node_idx.index() as u32;

            // Rebuild voice instances from template
            va.rebuild_voices();

            return Ok(node_id);
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Connect nodes in a VoiceAllocator's template graph
    pub fn connect_in_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        from_node: u32,
        from_port: usize,
        to_node: u32,
        to_port: usize,
    ) -> Result<(), String> {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        // Get the VoiceAllocator node
        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // Downcast to VoiceAllocatorNode using safe Any trait
            let va = graph_node.node.as_any_mut()
                .downcast_mut::<VoiceAllocatorNode>()
                .ok_or_else(|| "Node is not a VoiceAllocator".to_string())?;

            // Connect in template graph
            let from_idx = NodeIndex::new(from_node as usize);
            let to_idx = NodeIndex::new(to_node as usize);

            va.template_graph_mut().connect(from_idx, from_port, to_idx, to_port)
                .map_err(|e| format!("{:?}", e))?;

            // Rebuild voice instances from template
            va.rebuild_voices();

            return Ok(());
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Disconnect two nodes in a VoiceAllocator's template graph
    pub fn disconnect_in_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        from_node: u32,
        from_port: usize,
        to_node: u32,
        to_port: usize,
    ) -> Result<(), String> {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // Downcast to VoiceAllocatorNode using safe Any trait
            let va = graph_node.node.as_any_mut()
                .downcast_mut::<VoiceAllocatorNode>()
                .ok_or_else(|| "Node is not a VoiceAllocator".to_string())?;

            let from_idx = NodeIndex::new(from_node as usize);
            let to_idx = NodeIndex::new(to_node as usize);

            va.template_graph_mut().disconnect(from_idx, from_port, to_idx, to_port);
            va.rebuild_voices();

            return Ok(());
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Remove a node from a VoiceAllocator's template graph
    pub fn remove_node_from_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        node_id: u32,
    ) -> Result<(), String> {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // Downcast to VoiceAllocatorNode using safe Any trait
            let va = graph_node.node.as_any_mut()
                .downcast_mut::<VoiceAllocatorNode>()
                .ok_or_else(|| "Node is not a VoiceAllocator".to_string())?;

            let node_idx = NodeIndex::new(node_id as usize);
            va.template_graph_mut().remove_node(node_idx);
            va.rebuild_voices();

            return Ok(());
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Set a parameter on a node in a VoiceAllocator's template graph
    pub fn set_parameter_in_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        node_id: u32,
        param_id: u32,
        value: f32,
    ) -> Result<(), String> {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // Downcast to VoiceAllocatorNode using safe Any trait
            let va = graph_node.node.as_any_mut()
                .downcast_mut::<VoiceAllocatorNode>()
                .ok_or_else(|| "Node is not a VoiceAllocator".to_string())?;

            let node_idx = NodeIndex::new(node_id as usize);
            if let Some(template_node) = va.template_graph_mut().get_graph_node_mut(node_idx) {
                template_node.node.set_parameter(param_id, value);
            } else {
                return Err("Node not found in template".to_string());
            }

            va.rebuild_voices();

            return Ok(());
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Set the position of a node in a VoiceAllocator's template graph
    pub fn set_position_in_voice_allocator_template(
        &mut self,
        voice_allocator_idx: NodeIndex,
        node_id: u32,
        x: f32,
        y: f32,
    ) {
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        if let Some(graph_node) = self.graph.node_weight_mut(voice_allocator_idx) {
            // Downcast to VoiceAllocatorNode using safe Any trait
            if let Some(va) = graph_node.node.as_any_mut().downcast_mut::<VoiceAllocatorNode>() {
                let node_idx = NodeIndex::new(node_id as usize);
                va.template_graph_mut().set_node_position(node_idx, x, y);
            }
        }
    }

    /// Process the graph and produce audio output
    pub fn process(&mut self, output_buffer: &mut [f32], midi_events: &[MidiEvent], playback_time: f64) {
        // Update playback time
        self.playback_time = playback_time;

        // Update playback time for all time-dependent nodes before processing
        use super::nodes::{AutomationInputNode, BeatNode};
        for node in self.graph.node_weights_mut() {
            if let Some(auto_node) = node.node.as_any_mut().downcast_mut::<AutomationInputNode>() {
                auto_node.set_playback_time(playback_time);
            } else if let Some(beat_node) = node.node.as_any_mut().downcast_mut::<BeatNode>() {
                beat_node.set_playback_time(playback_time);
                beat_node.set_tempo(self.bpm, self.beats_per_bar);
            }
        }

        // Use the requested output buffer size for processing
        // process_size is stereo (interleaved L/R), frame_count is mono
        let process_size = output_buffer.len();
        let frame_count = process_size / 2;

        // Clear all output buffers (audio/CV and MIDI)
        for node in self.graph.node_weights_mut() {
            for buffer in &mut node.output_buffers {
                let len = buffer.len();
                buffer[..process_size.min(len)].fill(0.0);
            }
            for midi_buffer in &mut node.midi_output_buffers {
                midi_buffer.clear();
            }
        }

        // Distribute incoming MIDI events to target nodes' MIDI output buffers
        // This puts MIDI into the graph so it can flow through connections
        for &target_idx in &self.midi_targets {
            if let Some(node) = self.graph.node_weight_mut(target_idx) {
                // Find the first MIDI output port and add events there
                if !node.midi_output_buffers.is_empty() {
                    node.midi_output_buffers[0].extend_from_slice(midi_events);
                }
            }
        }

        // Topological sort for processing order (cached, recomputed only on graph mutation)
        if self.topo_cache.is_none() {
            self.topo_cache = Some(
                petgraph::algo::toposort(&self.graph, None)
                    .unwrap_or_else(|_| {
                        // If there's a cycle (shouldn't happen due to validation), just process in index order
                        self.graph.node_indices().collect()
                    })
            );
        }
        let topo_len = self.topo_cache.as_ref().unwrap().len();

        // Process nodes in topological order
        for topo_i in 0..topo_len {
            let node_idx = self.topo_cache.as_ref().unwrap()[topo_i];
            // Get input port information
            let inputs = self.graph[node_idx].node.inputs();
            let num_audio_cv_inputs = inputs.iter().filter(|p| p.signal_type != SignalType::Midi).count();
            let num_midi_inputs = inputs.iter().filter(|p| p.signal_type == SignalType::Midi).count();
            // Collect audio/CV input signal types for correct buffer sizing
            let audio_cv_input_types: Vec<SignalType> = inputs.iter()
                .filter(|p| p.signal_type != SignalType::Midi)
                .map(|p| p.signal_type)
                .collect();

            // Clear input buffers
            // - Audio inputs: fill with 0.0 (silence) when unconnected
            // - CV inputs: fill with NaN to indicate "no connection" (allows nodes to use parameter values)
            let mut audio_cv_idx = 0;
            for port in inputs.iter().filter(|p| p.signal_type != SignalType::Midi) {
                if audio_cv_idx < self.input_buffers.len() {
                    let fill_value = match port.signal_type {
                        SignalType::Audio => 0.0,  // Silence for audio
                        SignalType::CV => f32::NAN, // Sentinel for CV
                        SignalType::Midi => unreachable!(), // Already filtered out
                    };
                    self.input_buffers[audio_cv_idx].fill(fill_value);
                    audio_cv_idx += 1;
                }
            }

            // Clear MIDI input buffers
            for i in 0..num_midi_inputs {
                if i < self.midi_input_buffers.len() {
                    self.midi_input_buffers[i].clear();
                }
            }

            // Collect edge info into stack array to avoid heap allocation
            // (need to collect because we borrow graph immutably for source node data)
            const MAX_EDGES: usize = 32;
            let mut edge_info: [(NodeIndex, usize, usize); MAX_EDGES] = [(NodeIndex::new(0), 0, 0); MAX_EDGES];
            let mut edge_count = 0;
            for edge in self.graph.edges_directed(node_idx, Direction::Incoming) {
                if edge_count < MAX_EDGES {
                    edge_info[edge_count] = (edge.source(), edge.weight().from_port, edge.weight().to_port);
                    edge_count += 1;
                }
            }

            for ei in 0..edge_count {
                let (source_idx, from_port, to_port) = edge_info[ei];
                let source_node = &self.graph[source_idx];

                // Determine source port type
                if from_port < source_node.node.outputs().len() {
                    let source_port_type = source_node.node.outputs()[from_port].signal_type;

                    match source_port_type {
                        SignalType::Audio | SignalType::CV => {
                            // Map from global port index to audio/CV-only port index
                            // (input_buffers only contains audio/CV entries, not MIDI)
                            let audio_cv_port_idx = inputs.iter()
                                .take(to_port + 1)
                                .filter(|p| p.signal_type != SignalType::Midi)
                                .count().saturating_sub(1);

                            // Copy audio/CV data
                            if audio_cv_port_idx < num_audio_cv_inputs && from_port < source_node.output_buffers.len() {
                                let source_buffer = &source_node.output_buffers[from_port];
                                if audio_cv_port_idx < self.input_buffers.len() {
                                    for (dst, src) in self.input_buffers[audio_cv_port_idx].iter_mut().zip(source_buffer.iter()) {
                                        // If dst is NaN (unconnected), replace it; otherwise add (for mixing)
                                        if dst.is_nan() {
                                            *dst = *src;
                                        } else {
                                            *dst += src;
                                        }
                                    }
                                }
                            }
                        }
                        SignalType::Midi => {
                            // Copy MIDI events
                            // Map from global port index to MIDI-only port index
                            let midi_port_idx = inputs.iter()
                                .take(to_port + 1)
                                .filter(|p| p.signal_type == SignalType::Midi)
                                .count() - 1;

                            let source_midi_idx = source_node.node.outputs().iter()
                                .take(from_port + 1)
                                .filter(|p| p.signal_type == SignalType::Midi)
                                .count() - 1;

                            if midi_port_idx < self.midi_input_buffers.len() &&
                               source_midi_idx < source_node.midi_output_buffers.len() {
                                self.midi_input_buffers[midi_port_idx]
                                    .extend_from_slice(&source_node.midi_output_buffers[source_midi_idx]);
                            }
                        }
                    }
                }
            }

            // Prepare audio/CV input slices (Audio=stereo process_size, CV=mono frame_count)
            let input_slices: Vec<&[f32]> = (0..num_audio_cv_inputs)
                .map(|i| {
                    if i < self.input_buffers.len() {
                        let slice_size = match audio_cv_input_types.get(i) {
                            Some(&SignalType::Audio) => process_size,
                            _ => frame_count,
                        };
                        &self.input_buffers[i][..slice_size.min(self.input_buffers[i].len())]
                    } else {
                        &[][..]
                    }
                })
                .collect();

            // Prepare MIDI input slices
            let midi_input_slices: Vec<&[MidiEvent]> = (0..num_midi_inputs)
                .map(|i| {
                    if i < self.midi_input_buffers.len() {
                        &self.midi_input_buffers[i][..]
                    } else {
                        &[][..]
                    }
                })
                .collect();

            // Get mutable access to output buffers
            let node = &mut self.graph[node_idx];
            let outputs = node.node.outputs();
            let num_midi_outputs = outputs.iter().filter(|p| p.signal_type == SignalType::Midi).count();
            // Collect output signal types for correct buffer sizing
            let output_signal_types: Vec<SignalType> = outputs.iter().map(|p| p.signal_type).collect();

            // Create mutable slices for audio/CV outputs (Audio=stereo, CV=mono)
            let mut output_slices: Vec<&mut [f32]> = Vec::new();
            for (i, buf) in node.output_buffers.iter_mut().enumerate() {
                let signal_type = output_signal_types.get(i).copied().unwrap_or(SignalType::CV);
                if signal_type == SignalType::Midi { continue; }
                let slice_size = match signal_type {
                    SignalType::Audio => process_size,
                    _ => frame_count,
                };
                let len = buf.len();
                output_slices.push(&mut buf[..slice_size.min(len)]);
            }

            // Create mutable references for MIDI outputs
            let mut midi_output_refs: Vec<&mut Vec<MidiEvent>> = node.midi_output_buffers
                .iter_mut()
                .take(num_midi_outputs)
                .collect();

            // Process the node with both audio/CV and MIDI
            node.node.process(&input_slices, &mut output_slices, &midi_input_slices, &mut midi_output_refs, self.sample_rate);
        }

        // Mix output node's first output into the provided buffer
        if let Some(output_idx) = self.output_node {
            if let Some(output_node) = self.graph.node_weight(output_idx) {
                if !output_node.output_buffers.is_empty() {
                    let len = output_buffer.len().min(output_node.output_buffers[0].len());
                    for i in 0..len {
                        output_buffer[i] += output_node.output_buffers[0][i];
                    }
                }
            }
        }
    }

    /// Get node by index
    pub fn get_node(&self, idx: NodeIndex) -> Option<&dyn AudioNode> {
        self.graph.node_weight(idx).map(|n| &*n.node)
    }

    pub fn get_node_mut(&mut self, idx: NodeIndex) -> Option<&mut (dyn AudioNode + 'static)> {
        self.graph.node_weight_mut(idx).map(|n| &mut *n.node)
    }

    /// Get oscilloscope data from a specific node
    pub fn get_oscilloscope_data(&self, idx: NodeIndex, sample_count: usize) -> Option<Vec<f32>> {
        self.get_node(idx).and_then(|node| node.get_oscilloscope_data(sample_count))
    }

    /// Get oscilloscope CV data from a specific node
    pub fn get_oscilloscope_cv_data(&self, idx: NodeIndex, sample_count: usize) -> Option<Vec<f32>> {
        self.get_node(idx).and_then(|node| node.get_oscilloscope_cv_data(sample_count))
    }

    /// Get node by index (read-only)
    pub fn get_graph_node(&self, idx: NodeIndex) -> Option<&GraphNode> {
        self.graph.node_weight(idx)
    }

    /// Get node mutably by index
    /// Note: Due to lifetime constraints with trait objects, this returns a mutable reference
    /// to the GraphNode, from which you can access the node
    pub fn get_graph_node_mut(&mut self, idx: NodeIndex) -> Option<&mut GraphNode> {
        self.graph.node_weight_mut(idx)
    }

    /// Get all node indices
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph.node_indices()
    }

    /// Get all connections
    pub fn connections(&self) -> impl Iterator<Item = (NodeIndex, NodeIndex, &Connection)> + '_ {
        self.graph.edge_references().map(|e| (e.source(), e.target(), e.weight()))
    }

    /// Reset all nodes in the graph
    pub fn reset(&mut self) {
        // Collect indices first to avoid borrow checker issues
        let indices: Vec<_> = self.graph.node_indices().collect();
        for node_idx in indices {
            if let Some(node) = self.graph.node_weight_mut(node_idx) {
                node.node.reset();
            }
        }
    }

    /// Clone the graph structure with all nodes and connections
    pub fn clone_graph(&self) -> Self {
        let mut new_graph = Self::new(self.sample_rate, self.buffer_size);

        // Map from old NodeIndex to new NodeIndex
        let mut index_map = std::collections::HashMap::new();

        // Clone all nodes
        for node_idx in self.graph.node_indices() {
            if let Some(graph_node) = self.graph.node_weight(node_idx) {
                let cloned_node = graph_node.node.clone_node();
                let new_idx = new_graph.add_node(cloned_node);
                index_map.insert(node_idx, new_idx);
            }
        }

        // Clone all connections
        for edge in self.graph.edge_references() {
            let source = edge.source();
            let target = edge.target();
            let conn = edge.weight();

            if let (Some(&new_source), Some(&new_target)) = (index_map.get(&source), index_map.get(&target)) {
                let _ = new_graph.connect(new_source, conn.from_port, new_target, conn.to_port);
            }
        }

        // Clone MIDI targets
        for &old_target in &self.midi_targets {
            if let Some(&new_target) = index_map.get(&old_target) {
                new_graph.set_midi_target(new_target, true);
            }
        }

        // Clone output node reference
        if let Some(old_output) = self.output_node {
            if let Some(&new_output) = index_map.get(&old_output) {
                new_graph.output_node = Some(new_output);
            }
        }

        // Clone frontend groups
        new_graph.frontend_groups = self.frontend_groups.clone();

        new_graph
    }

    /// Set frontend-only group definitions (stored opaquely for persistence)
    pub fn set_frontend_groups(&mut self, groups: Vec<crate::audio::node_graph::preset::SerializedGroup>) {
        self.frontend_groups = groups;
    }

    /// Serialize the graph to a preset
    pub fn to_preset(&self, name: impl Into<String>) -> crate::audio::node_graph::preset::GraphPreset {
        use crate::audio::node_graph::preset::{GraphPreset, SerializedConnection, SerializedNode};
        use crate::audio::node_graph::nodes::VoiceAllocatorNode;

        let mut preset = GraphPreset::new(name);

        // Serialize all nodes
        for node_idx in self.graph.node_indices() {
            if let Some(graph_node) = self.graph.node_weight(node_idx) {
                let node = &graph_node.node;
                let node_id = node_idx.index() as u32;

                let mut serialized = SerializedNode::new(node_id, node.node_type());

                // Get all parameters
                for param in node.parameters() {
                    let value = node.get_parameter(param.id);
                    serialized.set_parameter(param.id, value);
                }

                // For VoiceAllocator nodes, serialize the template graph
                if node.node_type() == "VoiceAllocator" {
                    // Downcast using safe Any trait
                    if let Some(va_node) = node.as_any().downcast_ref::<VoiceAllocatorNode>() {
                        let template_preset = va_node.template_graph().to_preset("template");
                        serialized.template_graph = Some(Box::new(template_preset));
                    }
                }

                // For SimpleSampler nodes, serialize the loaded sample
                if node.node_type() == "SimpleSampler" {
                    use crate::audio::node_graph::nodes::SimpleSamplerNode;
                    use crate::audio::node_graph::preset::{EmbeddedSampleData, SampleData};
                    use base64::{Engine as _, engine::general_purpose};

                    // Downcast using safe Any trait
                    if let Some(sampler_node) = node.as_any().downcast_ref::<SimpleSamplerNode>() {
                        if let Some(sample_path) = sampler_node.get_sample_path() {
                            // Check file size
                            let should_embed = std::fs::metadata(sample_path)
                                .map(|m| m.len() < 100_000) // < 100KB
                                .unwrap_or(false);

                            if should_embed {
                                // Embed the sample data
                                let (sample_data, sample_rate) = sampler_node.get_sample_data_for_embedding();

                                // Convert f32 samples to bytes
                                let bytes: Vec<u8> = sample_data
                                    .iter()
                                    .flat_map(|&f| f.to_le_bytes())
                                    .collect();

                                // Encode to base64
                                let data_base64 = general_purpose::STANDARD.encode(&bytes);

                                serialized.sample_data = Some(SampleData::SimpleSampler {
                                    file_path: Some(sample_path.to_string()),
                                    embedded_data: Some(EmbeddedSampleData {
                                        data_base64,
                                        sample_rate: sample_rate as u32,
                                    }),
                                });
                            } else {
                                // Just save the file path
                                serialized.sample_data = Some(SampleData::SimpleSampler {
                                    file_path: Some(sample_path.to_string()),
                                    embedded_data: None,
                                });
                            }
                        }
                    }
                }

                // For MultiSampler nodes, serialize all loaded layers
                if node.node_type() == "MultiSampler" {
                    use crate::audio::node_graph::nodes::MultiSamplerNode;
                    use crate::audio::node_graph::preset::{EmbeddedSampleData, LayerData, SampleData};
                    use base64::{Engine as _, engine::general_purpose};

                    // Downcast using safe Any trait
                    if let Some(multi_sampler_node) = node.as_any().downcast_ref::<MultiSamplerNode>() {
                        let layers_info = multi_sampler_node.get_layers_info();
                        if !layers_info.is_empty() {
                            let layers: Vec<LayerData> = layers_info
                                .iter()
                                .enumerate()
                                .map(|(layer_index, info)| {
                                    // Check if we should embed this layer
                                    let should_embed = std::fs::metadata(&info.file_path)
                                        .map(|m| m.len() < 100_000) // < 100KB
                                        .unwrap_or(false);

                                    let embedded_data = if should_embed {
                                        // Get the sample data for this layer
                                        if let Some((sample_data, sample_rate)) = multi_sampler_node.get_layer_data(layer_index) {
                                            // Convert f32 samples to bytes
                                            let bytes: Vec<u8> = sample_data
                                                .iter()
                                                .flat_map(|&f| f.to_le_bytes())
                                                .collect();

                                            // Encode to base64
                                            let data_base64 = general_purpose::STANDARD.encode(&bytes);

                                            Some(EmbeddedSampleData {
                                                data_base64,
                                                sample_rate: sample_rate as u32,
                                            })
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    };

                                    LayerData {
                                        file_path: Some(info.file_path.clone()),
                                        embedded_data,
                                        key_min: info.key_min,
                                        key_max: info.key_max,
                                        root_key: info.root_key,
                                        velocity_min: info.velocity_min,
                                        velocity_max: info.velocity_max,
                                        loop_start: info.loop_start,
                                        loop_end: info.loop_end,
                                        loop_mode: info.loop_mode,
                                    }
                                })
                                .collect();
                            serialized.sample_data = Some(SampleData::MultiSampler { layers });
                        }
                    }
                }

                // For Script nodes, serialize the source code
                if node.node_type() == "Script" {
                    use crate::audio::node_graph::nodes::ScriptNode;
                    if let Some(script_node) = node.as_any().downcast_ref::<ScriptNode>() {
                        let source = script_node.source_code();
                        if !source.is_empty() {
                            serialized.script_source = Some(source.to_string());
                        }
                    }
                }

                // For AmpSim nodes, serialize the model path
                if node.node_type() == "AmpSim" {
                    use crate::audio::node_graph::nodes::AmpSimNode;
                    if let Some(amp_sim) = node.as_any().downcast_ref::<AmpSimNode>() {
                        serialized.nam_model_path = amp_sim.model_path().map(|s| s.to_string());
                    }
                }

                // Save position if available
                if let Some(pos) = self.get_node_position(node_idx) {
                    serialized.set_position(pos.0, pos.1);
                }

                preset.add_node(serialized);
            }
        }

        // Serialize connections
        for edge in self.graph.edge_references() {
            let source = edge.source();
            let target = edge.target();
            let conn = edge.weight();

            preset.add_connection(SerializedConnection {
                from_node: source.index() as u32,
                from_port: conn.from_port,
                to_node: target.index() as u32,
                to_port: conn.to_port,
            });
        }

        // MIDI targets
        preset.midi_targets = self.midi_targets.iter().map(|idx| idx.index() as u32).collect();

        // Output node
        preset.output_node = self.output_node.map(|idx| idx.index() as u32);

        // Frontend groups (stored opaquely)
        preset.groups = self.frontend_groups.clone();

        preset
    }

    /// Deserialize a preset into the graph
    pub fn from_preset(preset: &crate::audio::node_graph::preset::GraphPreset, sample_rate: u32, buffer_size: usize, preset_base_path: Option<&std::path::Path>) -> Result<Self, String> {
        use crate::audio::node_graph::nodes::*;
        use petgraph::stable_graph::NodeIndex;
        use std::collections::HashMap;

        // Helper function to resolve sample paths relative to preset
        let resolve_sample_path = |path: &str| -> String {
            let path_obj = std::path::Path::new(path);

            // If path is absolute, use it as-is
            if path_obj.is_absolute() {
                return path.to_string();
            }

            // If we have a base path and the path is relative, resolve it
            if let Some(base) = preset_base_path {
                let resolved = base.join(path);
                resolved.to_string_lossy().to_string()
            } else {
                // No base path, use path as-is
                path.to_string()
            }
        };

        let mut graph = Self::new(sample_rate, buffer_size);
        let mut index_map: HashMap<u32, NodeIndex> = HashMap::new();

        // Create all nodes
        for serialized_node in &preset.nodes {
            // Create the node based on type
            let mut node = crate::audio::node_graph::nodes::create_node(&serialized_node.node_type, sample_rate, buffer_size)
                .ok_or_else(|| format!("Unknown node type: {}", serialized_node.node_type))?;

            // VoiceAllocator needs its template graph deserialized and set
            if serialized_node.node_type == "VoiceAllocator" {
                if let Some(ref template_preset) = serialized_node.template_graph {
                    if let Some(va) = node.as_any_mut().downcast_mut::<VoiceAllocatorNode>() {
                        let template_graph = Self::from_preset(template_preset, sample_rate, buffer_size, preset_base_path)?;
                        *va.template_graph_mut() = template_graph;
                        va.rebuild_voices();
                    }
                }
            }

            let node_idx = graph.add_node(node);
            index_map.insert(serialized_node.id, node_idx);

            // Restore script source for Script nodes (must come before parameter setting
            // since set_script rebuilds parameters)
            if let Some(ref source) = serialized_node.script_source {
                if serialized_node.node_type == "Script" {
                    use crate::audio::node_graph::nodes::ScriptNode;
                    if let Some(graph_node) = graph.graph.node_weight_mut(node_idx) {
                        if let Some(script_node) = graph_node.node.as_any_mut().downcast_mut::<ScriptNode>() {
                            if let Err(e) = script_node.set_script(source) {
                                eprintln!("Warning: failed to compile script for node {}: {}", serialized_node.id, e);
                            }
                        }
                    }
                }
            }

            // Set parameters (after script compilation so param slots exist)
            for (&param_id, &value) in &serialized_node.parameters {
                if let Some(graph_node) = graph.graph.node_weight_mut(node_idx) {
                    graph_node.node.set_parameter(param_id, value);
                }
            }

            // Restore sample data for sampler nodes
            if let Some(ref sample_data) = serialized_node.sample_data {
                match sample_data {
                    crate::audio::node_graph::preset::SampleData::SimpleSampler { file_path, embedded_data } => {
                        // Load sample into SimpleSampler
                        if let Some(graph_node) = graph.graph.node_weight_mut(node_idx) {
                            // Downcast using safe Any trait
                            if let Some(sampler_node) = graph_node.node.as_any_mut().downcast_mut::<SimpleSamplerNode>() {

                                // Try embedded data first, then fall back to file path
                                if let Some(ref embedded) = embedded_data {
                                    use base64::{Engine as _, engine::general_purpose};

                                    // Decode base64
                                    if let Ok(bytes) = general_purpose::STANDARD.decode(&embedded.data_base64) {
                                        // Convert bytes back to f32 samples
                                        let samples: Vec<f32> = bytes
                                            .chunks_exact(4)
                                            .map(|chunk| {
                                                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                                            })
                                            .collect();

                                        sampler_node.set_sample(samples, embedded.sample_rate as f32);
                                    }
                                } else if let Some(ref path) = file_path {
                                    // Fall back to loading from file (resolve path relative to preset)
                                    let resolved_path = resolve_sample_path(path);
                                    if let Err(e) = sampler_node.load_sample_from_file(&resolved_path) {
                                        eprintln!("Failed to load sample from {}: {}", resolved_path, e);
                                    }
                                }
                            }
                        }
                    }
                    crate::audio::node_graph::preset::SampleData::MultiSampler { layers } => {
                        // Load layers into MultiSampler
                        if let Some(graph_node) = graph.graph.node_weight_mut(node_idx) {
                            // Downcast using safe Any trait
                            if let Some(multi_sampler_node) = graph_node.node.as_any_mut().downcast_mut::<MultiSamplerNode>() {
                                for layer in layers {
                                    // Try embedded data first, then fall back to file path
                                    if let Some(ref embedded) = layer.embedded_data {
                                        use base64::{Engine as _, engine::general_purpose};

                                        // Decode base64
                                        if let Ok(bytes) = general_purpose::STANDARD.decode(&embedded.data_base64) {
                                            // Convert bytes back to f32 samples
                                            let samples: Vec<f32> = bytes
                                                .chunks_exact(4)
                                                .map(|chunk| {
                                                    f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
                                                })
                                                .collect();

                                            multi_sampler_node.add_layer(
                                                samples,
                                                embedded.sample_rate as f32,
                                                layer.key_min,
                                                layer.key_max,
                                                layer.root_key,
                                                layer.velocity_min,
                                                layer.velocity_max,
                                                layer.loop_start,
                                                layer.loop_end,
                                                layer.loop_mode,
                                            );
                                        }
                                    } else if let Some(ref path) = layer.file_path {
                                        // Fall back to loading from file (resolve path relative to preset)
                                        let resolved_path = resolve_sample_path(path);
                                        if let Err(e) = multi_sampler_node.load_layer_from_file(
                                            &resolved_path,
                                            layer.key_min,
                                            layer.key_max,
                                            layer.root_key,
                                            layer.velocity_min,
                                            layer.velocity_max,
                                            layer.loop_start,
                                            layer.loop_end,
                                            layer.loop_mode,
                                        ) {
                                            eprintln!("Failed to load sample layer from {}: {}", resolved_path, e);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Restore NAM model for AmpSim nodes
            if let Some(ref model_path) = serialized_node.nam_model_path {
                if serialized_node.node_type == "AmpSim" {
                    use crate::audio::node_graph::nodes::AmpSimNode;
                    let resolved_path = resolve_sample_path(model_path);
                    if let Some(graph_node) = graph.graph.node_weight_mut(node_idx) {
                        if let Some(amp_sim) = graph_node.node.as_any_mut().downcast_mut::<AmpSimNode>() {
                            if let Err(e) = amp_sim.load_model(&resolved_path) {
                                eprintln!("Warning: failed to load NAM model {}: {}", resolved_path, e);
                            }
                        }
                    }
                }
            }

            // Restore position
            graph.set_node_position(node_idx, serialized_node.position.0, serialized_node.position.1);
        }

        // Create connections
        for conn in &preset.connections {
            let from_idx = index_map.get(&conn.from_node)
                .ok_or_else(|| format!("Connection from unknown node {}", conn.from_node))?;
            let to_idx = index_map.get(&conn.to_node)
                .ok_or_else(|| format!("Connection to unknown node {}", conn.to_node))?;

            graph.connect(*from_idx, conn.from_port, *to_idx, conn.to_port)
                .map_err(|e| format!("Failed to connect nodes: {:?}", e))?;
        }

        // Set MIDI targets
        for &target_id in &preset.midi_targets {
            if let Some(&target_idx) = index_map.get(&target_id) {
                graph.set_midi_target(target_idx, true);
            }
        }

        // Set output node
        if let Some(output_id) = preset.output_node {
            if let Some(&output_idx) = index_map.get(&output_id) {
                graph.output_node = Some(output_idx);
            }
        }

        // Restore frontend groups (stored opaquely)
        graph.frontend_groups = preset.groups.clone();

        Ok(graph)
    }
}
