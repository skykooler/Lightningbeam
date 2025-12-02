use super::automation::{AutomationLane, AutomationLaneId, ParameterId};
use super::clip::{AudioClipInstance, AudioClipInstanceId};
use super::midi::{MidiClipInstance, MidiClipInstanceId, MidiEvent};
use super::midi_pool::MidiClipPool;
use super::node_graph::AudioGraph;
use super::node_graph::nodes::{AudioInputNode, AudioOutputNode};
use super::node_graph::preset::GraphPreset;
use super::pool::AudioClipPool;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Track ID type
pub type TrackId = u32;

/// Default function for creating empty AudioGraph during deserialization
fn default_audio_graph() -> AudioGraph {
    AudioGraph::new(48000, 8192)
}

/// Type alias for backwards compatibility
pub type Track = AudioTrack;

/// Rendering context that carries timing information through the track hierarchy
///
/// This allows metatracks to transform time for their children (time stretch, offset, etc.)
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    /// Current playhead position in seconds (in transformed time)
    pub playhead_seconds: f64,
    /// Audio sample rate
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u32,
    /// Size of the buffer being rendered (in interleaved samples)
    pub buffer_size: usize,
    /// Accumulated time stretch factor (1.0 = normal, 0.5 = half speed, 2.0 = double speed)
    pub time_stretch: f32,
}

impl RenderContext {
    /// Create a new render context
    pub fn new(
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
        buffer_size: usize,
    ) -> Self {
        Self {
            playhead_seconds,
            sample_rate,
            channels,
            buffer_size,
            time_stretch: 1.0,
        }
    }

    /// Get the duration of the buffer in seconds
    pub fn buffer_duration(&self) -> f64 {
        self.buffer_size as f64 / (self.sample_rate as f64 * self.channels as f64)
    }

    /// Get the end time of the buffer
    pub fn buffer_end(&self) -> f64 {
        self.playhead_seconds + self.buffer_duration()
    }
}

/// Node in the track hierarchy - can be an audio track, MIDI track, or a metatrack
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TrackNode {
    Audio(AudioTrack),
    Midi(MidiTrack),
    Group(Metatrack),
}

impl TrackNode {
    /// Get the track ID
    pub fn id(&self) -> TrackId {
        match self {
            TrackNode::Audio(track) => track.id,
            TrackNode::Midi(track) => track.id,
            TrackNode::Group(group) => group.id,
        }
    }

    /// Get the track name
    pub fn name(&self) -> &str {
        match self {
            TrackNode::Audio(track) => &track.name,
            TrackNode::Midi(track) => &track.name,
            TrackNode::Group(group) => &group.name,
        }
    }

    /// Get muted state
    pub fn is_muted(&self) -> bool {
        match self {
            TrackNode::Audio(track) => track.muted,
            TrackNode::Midi(track) => track.muted,
            TrackNode::Group(group) => group.muted,
        }
    }

    /// Get solo state
    pub fn is_solo(&self) -> bool {
        match self {
            TrackNode::Audio(track) => track.solo,
            TrackNode::Midi(track) => track.solo,
            TrackNode::Group(group) => group.solo,
        }
    }

    /// Set volume
    pub fn set_volume(&mut self, volume: f32) {
        match self {
            TrackNode::Audio(track) => track.set_volume(volume),
            TrackNode::Midi(track) => track.set_volume(volume),
            TrackNode::Group(group) => group.set_volume(volume),
        }
    }

    /// Set muted state
    pub fn set_muted(&mut self, muted: bool) {
        match self {
            TrackNode::Audio(track) => track.set_muted(muted),
            TrackNode::Midi(track) => track.set_muted(muted),
            TrackNode::Group(group) => group.set_muted(muted),
        }
    }

    /// Set solo state
    pub fn set_solo(&mut self, solo: bool) {
        match self {
            TrackNode::Audio(track) => track.set_solo(solo),
            TrackNode::Midi(track) => track.set_solo(solo),
            TrackNode::Group(group) => group.set_solo(solo),
        }
    }

    /// Remove a MIDI clip instance (only works on MIDI tracks)
    pub fn remove_midi_clip_instance(&mut self, instance_id: MidiClipInstanceId) {
        if let TrackNode::Midi(track) = self {
            track.remove_midi_clip_instance(instance_id);
        }
    }

    /// Remove an audio clip instance (only works on audio tracks)
    pub fn remove_audio_clip_instance(&mut self, instance_id: AudioClipInstanceId) {
        if let TrackNode::Audio(track) = self {
            track.remove_audio_clip_instance(instance_id);
        }
    }
}

/// Metatrack that contains other tracks with time transformation capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metatrack {
    pub id: TrackId,
    pub name: String,
    pub children: Vec<TrackId>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
    /// Time stretch factor (0.5 = half speed, 1.0 = normal, 2.0 = double speed)
    pub time_stretch: f32,
    /// Pitch shift in semitones (for future implementation)
    pub pitch_shift: f32,
    /// Time offset in seconds (shift content forward/backward in time)
    pub offset: f64,
    /// Automation lanes for this metatrack
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,
}

impl Metatrack {
    /// Create a new metatrack
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            name,
            children: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
            time_stretch: 1.0,
            pitch_shift: 0.0,
            offset: 0.0,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
        }
    }

    /// Add an automation lane to this metatrack
    pub fn add_automation_lane(&mut self, parameter_id: ParameterId) -> AutomationLaneId {
        let lane_id = self.next_automation_id;
        self.next_automation_id += 1;

        let lane = AutomationLane::new(lane_id, parameter_id);
        self.automation_lanes.insert(lane_id, lane);
        lane_id
    }

    /// Get an automation lane by ID
    pub fn get_automation_lane(&self, lane_id: AutomationLaneId) -> Option<&AutomationLane> {
        self.automation_lanes.get(&lane_id)
    }

    /// Get a mutable automation lane by ID
    pub fn get_automation_lane_mut(&mut self, lane_id: AutomationLaneId) -> Option<&mut AutomationLane> {
        self.automation_lanes.get_mut(&lane_id)
    }

    /// Remove an automation lane
    pub fn remove_automation_lane(&mut self, lane_id: AutomationLaneId) -> bool {
        self.automation_lanes.remove(&lane_id).is_some()
    }

    /// Evaluate automation at a specific time and return effective parameters
    pub fn evaluate_automation_at_time(&self, time: f64) -> (f32, f32, f64) {
        let mut volume = self.volume;
        let mut time_stretch = self.time_stretch;
        let mut offset = self.offset;

        // Check for automation
        for lane in self.automation_lanes.values() {
            if !lane.enabled {
                continue;
            }

            match lane.parameter_id {
                ParameterId::TrackVolume => {
                    if let Some(automated_value) = lane.evaluate(time) {
                        volume = automated_value;
                    }
                }
                ParameterId::TimeStretch => {
                    if let Some(automated_value) = lane.evaluate(time) {
                        time_stretch = automated_value;
                    }
                }
                ParameterId::TimeOffset => {
                    if let Some(automated_value) = lane.evaluate(time) {
                        offset = automated_value as f64;
                    }
                }
                _ => {}
            }
        }

        (volume, time_stretch, offset)
    }

    /// Add a child track to this group
    pub fn add_child(&mut self, track_id: TrackId) {
        if !self.children.contains(&track_id) {
            self.children.push(track_id);
        }
    }

    /// Remove a child track from this group
    pub fn remove_child(&mut self, track_id: TrackId) {
        self.children.retain(|&id| id != track_id);
    }

    /// Set group volume
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.max(0.0);
    }

    /// Set mute state
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Set solo state
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    /// Check if this group should be audible given the solo state
    pub fn is_active(&self, any_solo: bool) -> bool {
        !self.muted && (!any_solo || self.solo)
    }

    /// Transform a render context for this metatrack's children
    ///
    /// Applies time stretching and offset transformations.
    /// Time stretch affects how fast content plays: 0.5 = half speed, 2.0 = double speed
    /// Offset shifts content forward/backward in time
    pub fn transform_context(&self, ctx: RenderContext) -> RenderContext {
        let mut transformed = ctx;

        // Apply transformations in order:
        // 1. First, subtract offset (positive offset = content appears later)
        //    At parent time 0.0s with offset=2.0s, child sees -2.0s (before content starts)
        //    At parent time 2.0s with offset=2.0s, child sees 0.0s (content starts)
        let adjusted_playhead = transformed.playhead_seconds - self.offset;

        // 2. Then apply time stretch (< 1.0 = slower/half speed, > 1.0 = faster/double speed)
        //    With stretch=0.5, when parent time is 2.0s, child reads from 1.0s (plays slower, pitches down)
        //    With stretch=2.0, when parent time is 2.0s, child reads from 4.0s (plays faster, pitches up)
        //    Note: This creates pitch shift as well - true time stretching would require resampling
        transformed.playhead_seconds = adjusted_playhead * self.time_stretch as f64;

        // Accumulate time stretch for nested metatracks
        transformed.time_stretch *= self.time_stretch;

        transformed
    }
}

/// MIDI track with MIDI clip instances and a node-based instrument
#[derive(Debug, Serialize, Deserialize)]
pub struct MidiTrack {
    pub id: TrackId,
    pub name: String,
    /// Clip instances placed on this track (reference clips in the MidiClipPool)
    pub clip_instances: Vec<MidiClipInstance>,

    /// Serialized instrument graph (used for save/load)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instrument_graph_preset: Option<GraphPreset>,

    /// Runtime instrument graph (rebuilt from preset on load)
    #[serde(skip, default = "default_audio_graph")]
    pub instrument_graph: AudioGraph,

    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
    /// Automation lanes for this track
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,
    /// Queue for live MIDI input (virtual keyboard, MIDI controllers)
    #[serde(skip)]
    live_midi_queue: Vec<MidiEvent>,
}

impl Clone for MidiTrack {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            clip_instances: self.clip_instances.clone(),
            instrument_graph_preset: self.instrument_graph_preset.clone(),
            instrument_graph: default_audio_graph(), // Create fresh graph, not cloned
            volume: self.volume,
            muted: self.muted,
            solo: self.solo,
            automation_lanes: self.automation_lanes.clone(),
            next_automation_id: self.next_automation_id,
            live_midi_queue: Vec::new(), // Don't clone live MIDI queue
        }
    }
}

impl MidiTrack {
    /// Create a new MIDI track with default settings
    pub fn new(id: TrackId, name: String, sample_rate: u32) -> Self {
        // Use a large buffer size that can accommodate any callback
        let default_buffer_size = 8192;

        Self {
            id,
            name,
            clip_instances: Vec::new(),
            instrument_graph_preset: None,
            instrument_graph: AudioGraph::new(sample_rate, default_buffer_size),
            volume: 1.0,
            muted: false,
            solo: false,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
            live_midi_queue: Vec::new(),
        }
    }

    /// Prepare for serialization by saving the instrument graph as a preset
    pub fn prepare_for_save(&mut self) {
        self.instrument_graph_preset = Some(self.instrument_graph.to_preset("Instrument Graph"));
    }

    /// Rebuild the instrument graph from preset after deserialization
    pub fn rebuild_audio_graph(&mut self, sample_rate: u32, buffer_size: usize) -> Result<(), String> {
        if let Some(preset) = &self.instrument_graph_preset {
            self.instrument_graph = AudioGraph::from_preset(preset, sample_rate, buffer_size, None)?;
        } else {
            // No preset - create default graph
            self.instrument_graph = AudioGraph::new(sample_rate, buffer_size);
        }
        Ok(())
    }

    /// Add an automation lane to this track
    pub fn add_automation_lane(&mut self, parameter_id: ParameterId) -> AutomationLaneId {
        let lane_id = self.next_automation_id;
        self.next_automation_id += 1;

        let lane = AutomationLane::new(lane_id, parameter_id);
        self.automation_lanes.insert(lane_id, lane);
        lane_id
    }

    /// Get an automation lane by ID
    pub fn get_automation_lane(&self, lane_id: AutomationLaneId) -> Option<&AutomationLane> {
        self.automation_lanes.get(&lane_id)
    }

    /// Get a mutable automation lane by ID
    pub fn get_automation_lane_mut(&mut self, lane_id: AutomationLaneId) -> Option<&mut AutomationLane> {
        self.automation_lanes.get_mut(&lane_id)
    }

    /// Remove an automation lane
    pub fn remove_automation_lane(&mut self, lane_id: AutomationLaneId) -> bool {
        self.automation_lanes.remove(&lane_id).is_some()
    }

    /// Add a MIDI clip instance to this track
    pub fn add_clip_instance(&mut self, instance: MidiClipInstance) {
        self.clip_instances.push(instance);
    }

    /// Remove a MIDI clip instance from this track by instance ID (for undo/redo support)
    pub fn remove_midi_clip_instance(&mut self, instance_id: MidiClipInstanceId) {
        self.clip_instances.retain(|instance| instance.id != instance_id);
    }

    /// Set track volume
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.max(0.0);
    }

    /// Set mute state
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Set solo state
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    /// Check if this track should be audible given the solo state
    pub fn is_active(&self, any_solo: bool) -> bool {
        !self.muted && (!any_solo || self.solo)
    }

    /// Stop all currently playing notes on this track's instrument
    /// Note: With node-based instruments, stopping is handled by ceasing MIDI input
    pub fn stop_all_notes(&mut self) {
        // Send note-off for all 128 possible MIDI notes to silence the instrument
        let mut note_offs = Vec::new();
        for note in 0..128 {
            note_offs.push(MidiEvent::note_off(0.0, 0, note, 0));
        }

        // Create a silent buffer to process the note-offs
        let buffer_size = 512 * 2; // stereo
        let mut silent_buffer = vec![0.0f32; buffer_size];
        self.instrument_graph.process(&mut silent_buffer, &note_offs, 0.0);
    }

    /// Queue a live MIDI event (from virtual keyboard or MIDI controller)
    pub fn queue_live_midi(&mut self, event: MidiEvent) {
        self.live_midi_queue.push(event);
    }

    /// Clear the live MIDI queue
    pub fn clear_live_midi_queue(&mut self) {
        self.live_midi_queue.clear();
    }

    /// Process only live MIDI input (queued events) without rendering clips
    /// This is used when playback is stopped but we want to hear live input
    pub fn process_live_input(
        &mut self,
        output: &mut [f32],
        _sample_rate: u32,
        _channels: u32,
    ) {
        // Generate audio using instrument graph with live MIDI events
        self.instrument_graph.process(output, &self.live_midi_queue, 0.0);

        // Clear the queue after processing
        self.live_midi_queue.clear();

        // Apply track volume (no automation during live input)
        for sample in output.iter_mut() {
            *sample *= self.volume;
        }
    }

    /// Render this MIDI track into the output buffer
    pub fn render(
        &mut self,
        output: &mut [f32],
        midi_pool: &MidiClipPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Collect MIDI events from all clip instances that overlap with current time range
        let mut midi_events = Vec::new();
        for instance in &self.clip_instances {
            // Get the clip content from the pool
            if let Some(clip) = midi_pool.get_clip(instance.clip_id) {
                let events = instance.get_events_in_range(
                    clip,
                    playhead_seconds,
                    buffer_end_seconds,
                );
                midi_events.extend(events);
            }
        }

        // Add live MIDI events (from virtual keyboard or MIDI controllers)
        // This allows real-time input to be heard during playback/recording
        midi_events.extend(self.live_midi_queue.drain(..));

        // Generate audio using instrument graph
        self.instrument_graph.process(output, &midi_events, playhead_seconds);

        // Evaluate and apply automation
        let effective_volume = self.evaluate_automation_at_time(playhead_seconds);

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= effective_volume;
        }
    }

    /// Evaluate automation at a specific time and return the effective volume
    fn evaluate_automation_at_time(&self, time: f64) -> f32 {
        let mut volume = self.volume;

        // Check for volume automation
        for lane in self.automation_lanes.values() {
            if !lane.enabled {
                continue;
            }

            match lane.parameter_id {
                ParameterId::TrackVolume => {
                    if let Some(automated_value) = lane.evaluate(time) {
                        volume = automated_value;
                    }
                }
                _ => {}
            }
        }

        volume
    }
}

/// Audio track with audio clip instances
#[derive(Debug, Serialize, Deserialize)]
pub struct AudioTrack {
    pub id: TrackId,
    pub name: String,
    /// Audio clip instances (reference content in the AudioClipPool)
    pub clips: Vec<AudioClipInstance>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
    /// Automation lanes for this track
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,

    /// Serialized effects graph (used for save/load)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effects_graph_preset: Option<GraphPreset>,

    /// Runtime effects processing graph (rebuilt from preset on load)
    #[serde(skip, default = "default_audio_graph")]
    pub effects_graph: AudioGraph,
}

impl Clone for AudioTrack {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            clips: self.clips.clone(),
            volume: self.volume,
            muted: self.muted,
            solo: self.solo,
            automation_lanes: self.automation_lanes.clone(),
            next_automation_id: self.next_automation_id,
            effects_graph_preset: self.effects_graph_preset.clone(),
            effects_graph: default_audio_graph(), // Create fresh graph, not cloned
        }
    }
}

impl AudioTrack {
    /// Create a new audio track with default settings
    pub fn new(id: TrackId, name: String, sample_rate: u32) -> Self {
        // Use a large buffer size that can accommodate any callback
        let default_buffer_size = 8192;

        // Create the effects graph with default AudioInput -> AudioOutput chain
        let mut effects_graph = AudioGraph::new(sample_rate, default_buffer_size);

        // Add AudioInput node
        let input_node = Box::new(AudioInputNode::new("Audio Input"));
        let input_id = effects_graph.add_node(input_node);
        // Set position for AudioInput (left side, similar to instrument preset spacing)
        effects_graph.set_node_position(input_id, 100.0, 150.0);

        // Add AudioOutput node
        let output_node = Box::new(AudioOutputNode::new("Audio Output"));
        let output_id = effects_graph.add_node(output_node);
        // Set position for AudioOutput (right side, spaced apart)
        effects_graph.set_node_position(output_id, 500.0, 150.0);

        // Connect AudioInput -> AudioOutput
        let _ = effects_graph.connect(input_id, 0, output_id, 0);

        // Set the AudioOutput node as the graph's output
        effects_graph.set_output_node(Some(output_id));

        Self {
            id,
            name,
            clips: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
            effects_graph_preset: None,
            effects_graph,
        }
    }

    /// Prepare for serialization by saving the effects graph as a preset
    pub fn prepare_for_save(&mut self) {
        self.effects_graph_preset = Some(self.effects_graph.to_preset("Effects Graph"));
    }

    /// Rebuild the effects graph from preset after deserialization
    pub fn rebuild_audio_graph(&mut self, sample_rate: u32, buffer_size: usize) -> Result<(), String> {
        if let Some(preset) = &self.effects_graph_preset {
            // Check if preset is empty or missing required nodes
            let has_nodes = !preset.nodes.is_empty();
            let has_output = preset.output_node.is_some();

            if has_nodes && has_output {
                // Valid preset - rebuild from it
                self.effects_graph = AudioGraph::from_preset(preset, sample_rate, buffer_size, None)?;
            } else {
                // Empty or invalid preset - create default graph
                self.effects_graph = Self::create_default_graph(sample_rate, buffer_size);
            }
        } else {
            // No preset - create default graph
            self.effects_graph = Self::create_default_graph(sample_rate, buffer_size);
        }
        Ok(())
    }

    /// Create a default effects graph with AudioInput -> AudioOutput
    fn create_default_graph(sample_rate: u32, buffer_size: usize) -> AudioGraph {
        let mut effects_graph = AudioGraph::new(sample_rate, buffer_size);

        // Add AudioInput node
        let input_node = Box::new(AudioInputNode::new("Audio Input"));
        let input_id = effects_graph.add_node(input_node);
        effects_graph.set_node_position(input_id, 100.0, 150.0);

        // Add AudioOutput node
        let output_node = Box::new(AudioOutputNode::new("Audio Output"));
        let output_id = effects_graph.add_node(output_node);
        effects_graph.set_node_position(output_id, 500.0, 150.0);

        // Connect AudioInput -> AudioOutput
        let _ = effects_graph.connect(input_id, 0, output_id, 0);

        // Set the AudioOutput node as the graph's output
        effects_graph.set_output_node(Some(output_id));

        effects_graph
    }

    /// Add an automation lane to this track
    pub fn add_automation_lane(&mut self, parameter_id: ParameterId) -> AutomationLaneId {
        let lane_id = self.next_automation_id;
        self.next_automation_id += 1;

        let lane = AutomationLane::new(lane_id, parameter_id);
        self.automation_lanes.insert(lane_id, lane);
        lane_id
    }

    /// Get an automation lane by ID
    pub fn get_automation_lane(&self, lane_id: AutomationLaneId) -> Option<&AutomationLane> {
        self.automation_lanes.get(&lane_id)
    }

    /// Get a mutable automation lane by ID
    pub fn get_automation_lane_mut(&mut self, lane_id: AutomationLaneId) -> Option<&mut AutomationLane> {
        self.automation_lanes.get_mut(&lane_id)
    }

    /// Remove an automation lane
    pub fn remove_automation_lane(&mut self, lane_id: AutomationLaneId) -> bool {
        self.automation_lanes.remove(&lane_id).is_some()
    }

    /// Add an audio clip instance to this track
    pub fn add_clip(&mut self, clip: AudioClipInstance) {
        self.clips.push(clip);
    }

    /// Remove an audio clip instance from this track by instance ID (for undo/redo support)
    pub fn remove_audio_clip_instance(&mut self, instance_id: AudioClipInstanceId) {
        self.clips.retain(|instance| instance.id != instance_id);
    }

    /// Set track volume (0.0 = silence, 1.0 = unity gain, >1.0 = amplification)
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.max(0.0);
    }

    /// Set mute state
    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    /// Set solo state
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
    }

    /// Check if this track should be audible given the solo state of all tracks
    pub fn is_active(&self, any_solo: bool) -> bool {
        !self.muted && (!any_solo || self.solo)
    }

    /// Render this track into the output buffer at a given timeline position
    /// Returns the number of samples actually rendered
    pub fn render(
        &mut self,
        output: &mut [f32],
        pool: &AudioClipPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Create a temporary buffer for clip rendering
        let mut clip_buffer = vec![0.0f32; output.len()];
        let mut rendered = 0;

        // Render all active clip instances into the temporary buffer
        for clip in &self.clips {
            // Check if clip overlaps with current buffer time range
            if clip.external_start < buffer_end_seconds && clip.external_end() > playhead_seconds {
                rendered += self.render_clip(
                    clip,
                    &mut clip_buffer,
                    pool,
                    playhead_seconds,
                    sample_rate,
                    channels,
                );
            }
        }

        // Find and inject audio into the AudioInputNode
        let node_indices: Vec<_> = self.effects_graph.node_indices().collect();
        for node_idx in node_indices {
            if let Some(graph_node) = self.effects_graph.get_graph_node_mut(node_idx) {
                if graph_node.node.node_type() == "AudioInput" {
                    if let Some(input_node) = graph_node.node.as_any_mut().downcast_mut::<AudioInputNode>() {
                        input_node.inject_audio(&clip_buffer);
                        break;
                    }
                }
            }
        }

        // Process through the effects graph (this will write to output buffer)
        self.effects_graph.process(output, &[], playhead_seconds);

        // Evaluate and apply automation
        let effective_volume = self.evaluate_automation_at_time(playhead_seconds);

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= effective_volume;
        }

        rendered
    }

    /// Evaluate automation at a specific time and return the effective volume
    fn evaluate_automation_at_time(&self, time: f64) -> f32 {
        let mut volume = self.volume;

        // Check for volume automation
        for lane in self.automation_lanes.values() {
            if !lane.enabled {
                continue;
            }

            match lane.parameter_id {
                ParameterId::TrackVolume => {
                    if let Some(automated_value) = lane.evaluate(time) {
                        volume = automated_value;
                    }
                }
                _ => {}
            }
        }

        volume
    }

    /// Render a single audio clip instance into the output buffer
    /// Handles looping when external_duration > internal_duration
    fn render_clip(
        &self,
        clip: &AudioClipInstance,
        output: &mut [f32],
        pool: &AudioClipPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Determine the time range we need to render (intersection of buffer and clip external bounds)
        let render_start_seconds = playhead_seconds.max(clip.external_start);
        let render_end_seconds = buffer_end_seconds.min(clip.external_end());

        // If no overlap, return early
        if render_start_seconds >= render_end_seconds {
            return 0;
        }

        let internal_duration = clip.internal_duration();
        if internal_duration <= 0.0 {
            return 0;
        }

        // Calculate combined gain
        let combined_gain = clip.gain * self.volume;

        let mut total_rendered = 0;

        // Process the render range sample by sample (or in chunks for efficiency)
        // For looping clips, we need to handle wrap-around at the loop boundary
        let samples_per_second = sample_rate as f64 * channels as f64;

        // For now, render in a simpler way - iterate through the timeline range
        // and use get_content_position for each sample position
        let output_start_offset = ((render_start_seconds - playhead_seconds) * samples_per_second) as usize;
        let output_end_offset = ((render_end_seconds - playhead_seconds) * samples_per_second) as usize;

        if output_end_offset > output.len() || output_start_offset > output.len() {
            return 0;
        }

        // If not looping, we can render in one chunk (more efficient)
        if !clip.is_looping() {
            // Simple case: no looping
            let content_start = clip.get_content_position(render_start_seconds).unwrap_or(clip.internal_start);
            let output_len = output.len();
            let output_slice = &mut output[output_start_offset..output_end_offset.min(output_len)];

            total_rendered = pool.render_from_file(
                clip.audio_pool_index,
                output_slice,
                content_start,
                combined_gain,
                sample_rate,
                channels,
            );
        } else {
            // Looping case: need to handle wrap-around at loop boundaries
            // Render in segments, one per loop iteration
            let mut timeline_pos = render_start_seconds;
            let mut output_offset = output_start_offset;

            while timeline_pos < render_end_seconds && output_offset < output.len() {
                // Calculate position within the loop
                let relative_pos = timeline_pos - clip.external_start;
                let loop_offset = relative_pos % internal_duration;
                let content_pos = clip.internal_start + loop_offset;

                // Calculate how much we can render before hitting the loop boundary
                let time_to_loop_end = internal_duration - loop_offset;
                let time_to_render_end = render_end_seconds - timeline_pos;
                let chunk_duration = time_to_loop_end.min(time_to_render_end);

                let chunk_samples = (chunk_duration * samples_per_second) as usize;
                let chunk_samples = chunk_samples.min(output.len() - output_offset);

                if chunk_samples == 0 {
                    break;
                }

                let output_slice = &mut output[output_offset..output_offset + chunk_samples];

                let rendered = pool.render_from_file(
                    clip.audio_pool_index,
                    output_slice,
                    content_pos,
                    combined_gain,
                    sample_rate,
                    channels,
                );

                total_rendered += rendered;
                output_offset += chunk_samples;
                timeline_pos += chunk_duration;
            }
        }

        total_rendered
    }
}
