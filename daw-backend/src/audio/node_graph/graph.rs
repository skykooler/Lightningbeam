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
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: Box<dyn AudioNode>) -> NodeIndex {
        let graph_node = GraphNode::new(node, self.buffer_size);
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

        // Add the edge
        self.graph.add_edge(from, to, Connection { from_port, to_port });

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
            }
        }
    }

    /// Remove a node from the graph
    pub fn remove_node(&mut self, node: NodeIndex) {
        self.graph.remove_node(node);

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

            // Check node type first
            if graph_node.node.node_type() != "VoiceAllocator" {
                return Err("Node is not a VoiceAllocator".to_string());
            }

            // Get mutable reference and downcast using raw pointers
            let node_ptr = &mut *graph_node.node as *mut dyn AudioNode;

            // SAFETY: We just checked that this is a VoiceAllocator
            // This is safe because we know the concrete type
            unsafe {
                let va_ptr = node_ptr as *mut VoiceAllocatorNode;
                let va = &mut *va_ptr;

                // Add node to template graph
                let node_idx = va.template_graph_mut().add_node(node);
                let node_id = node_idx.index() as u32;

                // Rebuild voice instances from template
                va.rebuild_voices();

                return Ok(node_id);
            }
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
            // Check node type first
            if graph_node.node.node_type() != "VoiceAllocator" {
                return Err("Node is not a VoiceAllocator".to_string());
            }

            // Get mutable reference and downcast using raw pointers
            let node_ptr = &mut *graph_node.node as *mut dyn AudioNode;

            // SAFETY: We just checked that this is a VoiceAllocator
            unsafe {
                let va_ptr = node_ptr as *mut VoiceAllocatorNode;
                let va = &mut *va_ptr;

                // Connect in template graph
                let from_idx = NodeIndex::new(from_node as usize);
                let to_idx = NodeIndex::new(to_node as usize);

                va.template_graph_mut().connect(from_idx, from_port, to_idx, to_port)
                    .map_err(|e| format!("{:?}", e))?;

                // Rebuild voice instances from template
                va.rebuild_voices();

                return Ok(());
            }
        }

        Err("VoiceAllocator node not found".to_string())
    }

    /// Process the graph and produce audio output
    pub fn process(&mut self, output_buffer: &mut [f32], midi_events: &[MidiEvent], playback_time: f64) {
        // Update playback time
        self.playback_time = playback_time;

        // Update playback time for all automation nodes before processing
        use super::nodes::AutomationInputNode;
        for node in self.graph.node_weights_mut() {
            // Try to downcast to AutomationInputNode and update its playback time
            if let Some(auto_node) = node.node.as_any_mut().downcast_mut::<AutomationInputNode>() {
                auto_node.set_playback_time(playback_time);
            }
        }

        // Use the requested output buffer size for processing
        let process_size = output_buffer.len();

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

        // Topological sort for processing order
        let topo = petgraph::algo::toposort(&self.graph, None)
            .unwrap_or_else(|_| {
                // If there's a cycle (shouldn't happen due to validation), just process in index order
                self.graph.node_indices().collect()
            });

        // Process nodes in topological order
        for node_idx in topo {
            // Get input port information
            let inputs = self.graph[node_idx].node.inputs();
            let num_audio_cv_inputs = inputs.iter().filter(|p| p.signal_type != SignalType::Midi).count();
            let num_midi_inputs = inputs.iter().filter(|p| p.signal_type == SignalType::Midi).count();

            // Clear audio/CV input buffers
            for i in 0..num_audio_cv_inputs {
                if i < self.input_buffers.len() {
                    self.input_buffers[i].fill(0.0);
                }
            }

            // Clear MIDI input buffers
            for i in 0..num_midi_inputs {
                if i < self.midi_input_buffers.len() {
                    self.midi_input_buffers[i].clear();
                }
            }

            // Collect inputs from connected nodes
            let incoming = self.graph.edges_directed(node_idx, Direction::Incoming).collect::<Vec<_>>();

            for edge in incoming {
                let source_idx = edge.source();
                let conn = edge.weight();
                let source_node = &self.graph[source_idx];

                // Determine source port type
                if conn.from_port < source_node.node.outputs().len() {
                    let source_port_type = source_node.node.outputs()[conn.from_port].signal_type;

                    match source_port_type {
                        SignalType::Audio | SignalType::CV => {
                            // Copy audio/CV data
                            if conn.to_port < num_audio_cv_inputs && conn.from_port < source_node.output_buffers.len() {
                                let source_buffer = &source_node.output_buffers[conn.from_port];
                                if conn.to_port < self.input_buffers.len() {
                                    for (dst, src) in self.input_buffers[conn.to_port].iter_mut().zip(source_buffer.iter()) {
                                        *dst += src;
                                    }
                                }
                            }
                        }
                        SignalType::Midi => {
                            // Copy MIDI events
                            // Map from global port index to MIDI-only port index
                            let midi_port_idx = inputs.iter()
                                .take(conn.to_port + 1)
                                .filter(|p| p.signal_type == SignalType::Midi)
                                .count() - 1;

                            let source_midi_idx = source_node.node.outputs().iter()
                                .take(conn.from_port + 1)
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

            // Prepare audio/CV input slices
            let input_slices: Vec<&[f32]> = (0..num_audio_cv_inputs)
                .map(|i| {
                    if i < self.input_buffers.len() {
                        &self.input_buffers[i][..process_size.min(self.input_buffers[i].len())]
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
            let num_audio_cv_outputs = outputs.iter().filter(|p| p.signal_type != SignalType::Midi).count();
            let num_midi_outputs = outputs.iter().filter(|p| p.signal_type == SignalType::Midi).count();

            // Create mutable slices for audio/CV outputs
            let mut output_slices: Vec<&mut [f32]> = Vec::with_capacity(num_audio_cv_outputs);
            for i in 0..num_audio_cv_outputs {
                if i < node.output_buffers.len() {
                    // Safety: We need to work around borrowing rules here
                    // This is safe because each output buffer is independent
                    let buffer = &mut node.output_buffers[i] as *mut Vec<f32>;
                    unsafe {
                        let slice = &mut (&mut *buffer)[..process_size.min((*buffer).len())];
                        output_slices.push(slice);
                    }
                }
            }

            // Create mutable references for MIDI outputs
            let mut midi_output_refs: Vec<&mut Vec<MidiEvent>> = Vec::with_capacity(num_midi_outputs);
            for i in 0..num_midi_outputs {
                if i < node.midi_output_buffers.len() {
                    // Safety: Similar to above
                    let buffer = &mut node.midi_output_buffers[i] as *mut Vec<MidiEvent>;
                    unsafe {
                        midi_output_refs.push(&mut *buffer);
                    }
                }
            }

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

        new_graph
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
                // We need to downcast to access template_graph()
                // This is safe because we know the node type
                if node.node_type() == "VoiceAllocator" {
                    // Use Any to downcast
                    let node_ptr = &**node as *const dyn crate::audio::node_graph::AudioNode;
                    let node_ptr = node_ptr as *const VoiceAllocatorNode;
                    unsafe {
                        let va_node = &*node_ptr;
                        let template_preset = va_node.template_graph().to_preset("template");
                        serialized.template_graph = Some(Box::new(template_preset));
                    }
                }

                // For SimpleSampler nodes, serialize the loaded sample
                if node.node_type() == "SimpleSampler" {
                    use crate::audio::node_graph::nodes::SimpleSamplerNode;
                    use crate::audio::node_graph::preset::{EmbeddedSampleData, SampleData};
                    use base64::{Engine as _, engine::general_purpose};

                    let node_ptr = &**node as *const dyn crate::audio::node_graph::AudioNode;
                    let node_ptr = node_ptr as *const SimpleSamplerNode;
                    unsafe {
                        let sampler_node = &*node_ptr;
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

                    let node_ptr = &**node as *const dyn crate::audio::node_graph::AudioNode;
                    let node_ptr = node_ptr as *const MultiSamplerNode;
                    unsafe {
                        let multi_sampler_node = &*node_ptr;
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
            let node: Box<dyn crate::audio::node_graph::AudioNode> = match serialized_node.node_type.as_str() {
                "Oscillator" => Box::new(OscillatorNode::new("Oscillator")),
                "Gain" => Box::new(GainNode::new("Gain")),
                "Mixer" => Box::new(MixerNode::new("Mixer")),
                "Filter" => Box::new(FilterNode::new("Filter")),
                "ADSR" => Box::new(ADSRNode::new("ADSR")),
                "LFO" => Box::new(LFONode::new("LFO")),
                "NoiseGenerator" => Box::new(NoiseGeneratorNode::new("Noise")),
                "Splitter" => Box::new(SplitterNode::new("Splitter")),
                "Pan" => Box::new(PanNode::new("Pan")),
                "Quantizer" => Box::new(QuantizerNode::new("Quantizer")),
                "Delay" => Box::new(DelayNode::new("Delay")),
                "Distortion" => Box::new(DistortionNode::new("Distortion")),
                "Reverb" => Box::new(ReverbNode::new("Reverb")),
                "Chorus" => Box::new(ChorusNode::new("Chorus")),
                "Compressor" => Box::new(CompressorNode::new("Compressor")),
                "Constant" => Box::new(ConstantNode::new("Constant")),
                "EnvelopeFollower" => Box::new(EnvelopeFollowerNode::new("Envelope Follower")),
                "Limiter" => Box::new(LimiterNode::new("Limiter")),
                "Math" => Box::new(MathNode::new("Math")),
                "EQ" => Box::new(EQNode::new("EQ")),
                "Flanger" => Box::new(FlangerNode::new("Flanger")),
                "FMSynth" => Box::new(FMSynthNode::new("FM Synth")),
                "Phaser" => Box::new(PhaserNode::new("Phaser")),
                "BitCrusher" => Box::new(BitCrusherNode::new("Bit Crusher")),
                "Vocoder" => Box::new(VocoderNode::new("Vocoder")),
                "RingModulator" => Box::new(RingModulatorNode::new("Ring Modulator")),
                "SampleHold" => Box::new(SampleHoldNode::new("Sample & Hold")),
                "WavetableOscillator" => Box::new(WavetableOscillatorNode::new("Wavetable")),
                "SimpleSampler" => Box::new(SimpleSamplerNode::new("Sampler")),
                "SlewLimiter" => Box::new(SlewLimiterNode::new("Slew Limiter")),
                "MultiSampler" => Box::new(MultiSamplerNode::new("Multi Sampler")),
                "MidiInput" => Box::new(MidiInputNode::new("MIDI Input")),
                "MidiToCV" => Box::new(MidiToCVNode::new("MIDI→CV")),
                "AudioToCV" => Box::new(AudioToCVNode::new("Audio→CV")),
                "AudioInput" => Box::new(AudioInputNode::new("Audio Input")),
                "AutomationInput" => Box::new(AutomationInputNode::new("Automation")),
                "Oscilloscope" => Box::new(OscilloscopeNode::new("Oscilloscope")),
                "TemplateInput" => Box::new(TemplateInputNode::new("Template Input")),
                "TemplateOutput" => Box::new(TemplateOutputNode::new("Template Output")),
                "VoiceAllocator" => {
                    let mut va = VoiceAllocatorNode::new("VoiceAllocator", sample_rate, buffer_size);

                    // If there's a template graph, deserialize and set it
                    if let Some(ref template_preset) = serialized_node.template_graph {
                        let template_graph = Self::from_preset(template_preset, sample_rate, buffer_size, preset_base_path)?;
                        // Set the template graph (we'll need to add this method to VoiceAllocator)
                        *va.template_graph_mut() = template_graph;
                        va.rebuild_voices();
                    }

                    Box::new(va)
                }
                "AudioOutput" => Box::new(AudioOutputNode::new("Output")),
                _ => return Err(format!("Unknown node type: {}", serialized_node.node_type)),
            };

            let node_idx = graph.add_node(node);
            index_map.insert(serialized_node.id, node_idx);

            // Set parameters
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
                            let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                            let node_ptr = node_ptr as *mut SimpleSamplerNode;
                            unsafe {
                                let sampler_node = &mut *node_ptr;

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
                            let node_ptr = &mut *graph_node.node as *mut dyn crate::audio::node_graph::AudioNode;
                            let node_ptr = node_ptr as *mut MultiSamplerNode;
                            unsafe {
                                let multi_sampler_node = &mut *node_ptr;
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

        Ok(graph)
    }
}
