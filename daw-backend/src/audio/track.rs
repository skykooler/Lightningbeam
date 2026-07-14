use super::automation::{AutomationLane, AutomationLaneId, ParameterId};
use super::clip::{AudioClipInstance, AudioClipInstanceId};
use super::midi::{MidiClipId, MidiClipInstance, MidiClipInstanceId, MidiEvent};
use super::midi_pool::MidiClipPool;
use super::node_graph::AudioGraph;
use super::node_graph::nodes::{AudioInputNode, AudioOutputNode};
use super::node_graph::preset::GraphPreset;
use super::pool::AudioClipPool;
use crate::tempo_map::TempoMap;
use crate::time::{Beats, Seconds};
use serde::{Serialize, Deserialize};
use std::collections::{HashMap, HashSet};

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
#[derive(Clone, Copy)]
pub struct RenderContext<'a> {
    /// Current playhead position in seconds (in transformed time)
    pub playhead_seconds: Seconds,
    /// Tempo map for beat ↔ second conversion
    pub tempo_map: &'a TempoMap,
    /// Audio sample rate
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u32,
    /// Size of the buffer being rendered (in interleaved samples)
    pub buffer_size: usize,
    /// Accumulated time stretch factor (1.0 = normal, 0.5 = half speed, 2.0 = double speed)
    pub time_stretch: f32,
    /// When true: skip clip event collection; only render instrument state and live MIDI queue.
    /// Used after pause/stop to route note-off tails through the normal group hierarchy
    /// without re-triggering notes from clips at the paused position.
    pub live_only: bool,
    /// The MIDI recording in progress, if any: (track being recorded to, clip being recorded into).
    ///
    /// On that track, every OTHER clip is silenced for the duration of the recording. You're playing
    /// a part into this region — hearing what's already there (a previous take, say) fighting with
    /// what you're playing now is just noise. The clip being recorded into is exempt, because in
    /// merge mode that's exactly what you DO want to hear: the overdub you've been building up.
    pub recording_midi: Option<(TrackId, MidiClipId)>,
}

impl<'a> RenderContext<'a> {
    pub fn new(
        playhead_seconds: Seconds,
        tempo_map: &'a TempoMap,
        sample_rate: u32,
        channels: u32,
        buffer_size: usize,
    ) -> Self {
        Self {
            playhead_seconds,
            tempo_map,
            sample_rate,
            channels,
            buffer_size,
            time_stretch: 1.0,
            live_only: false,
            recording_midi: None,
        }
    }

    pub fn buffer_duration(&self) -> Seconds {
        Seconds(self.buffer_size as f64 / (self.sample_rate as f64 * self.channels as f64))
    }

    pub fn buffer_end(&self) -> Seconds {
        self.playhead_seconds + self.buffer_duration()
    }

    pub fn playhead_beats(&self) -> Beats {
        self.tempo_map.seconds_to_beats(self.playhead_seconds)
    }

    pub fn buffer_end_beats(&self) -> Beats {
        self.tempo_map.seconds_to_beats(self.buffer_end())
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
#[derive(Debug, Serialize, Deserialize)]
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
    /// Time offset (shift content forward/backward in time)
    pub offset: Seconds,
    /// Trim start: offset into the metatrack's internal content
    /// Children will see time starting from this point
    pub trim_start: Seconds,
    /// Trim end: offset into the metatrack's internal content
    /// None means no end trim (play until content ends)
    pub trim_end: Option<Seconds>,
    /// Automation lanes for this metatrack
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,
    /// Audio node graph for effects processing (input → output)
    #[serde(skip, default = "default_audio_graph")]
    pub audio_graph: AudioGraph,
    /// Saved graph preset for serialization
    audio_graph_preset: Option<GraphPreset>,
    /// True while the mixing graph is still the auto-generated default (no user edits).
    /// Used to auto-connect new subtracks and to prompt before loading a preset.
    #[serde(default)]
    pub graph_is_default: bool,
}

impl Clone for Metatrack {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            name: self.name.clone(),
            children: self.children.clone(),
            volume: self.volume,
            muted: self.muted,
            solo: self.solo,
            time_stretch: self.time_stretch,
            pitch_shift: self.pitch_shift,
            offset: self.offset,
            trim_start: self.trim_start,
            trim_end: self.trim_end,
            automation_lanes: self.automation_lanes.clone(),
            next_automation_id: self.next_automation_id,
            audio_graph: default_audio_graph(), // Create fresh graph, not cloned
            audio_graph_preset: self.audio_graph_preset.clone(),
            graph_is_default: self.graph_is_default,
        }
    }
}

impl Metatrack {
    /// Create a new metatrack. The mixing graph is set up later via `set_subtrack_graph`
    /// once the child track list is known.
    pub fn new(id: TrackId, name: String, sample_rate: u32) -> Self {
        let default_buffer_size = 8192;
        let audio_graph = Self::create_empty_graph(sample_rate, default_buffer_size);

        Self {
            id,
            name,
            children: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
            time_stretch: 1.0,
            pitch_shift: 0.0,
            offset: Seconds::ZERO,
            trim_start: Seconds::ZERO,
            trim_end: None,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
            audio_graph,
            audio_graph_preset: None,
            graph_is_default: true,
        }
    }

    /// Minimal graph used before subtracks are known (just an AudioOutput node).
    fn create_empty_graph(sample_rate: u32, buffer_size: usize) -> AudioGraph {
        let mut graph = AudioGraph::new(sample_rate, buffer_size);
        let output_node = Box::new(AudioOutputNode::new("Audio Output"));
        let output_id = graph.add_node(output_node);
        graph.set_node_position(output_id, 500.0, 150.0);
        graph.set_output_node(Some(output_id));
        graph
    }

    /// Build the default subtrack mixing graph: SubtrackInputs → Mixer → Gain → AudioOutput,
    /// with an AutomationInput ("Volume", range 0..2) feeding the Gain's CV port.
    ///
    /// Existing Volume keyframes are preserved across rebuilds so that adding/removing
    /// a child track doesn't reset the automation.
    ///
    /// `subtracks` is an ordered list of (backend TrackId, display name) for each child.
    /// Replaces the current graph and marks `graph_is_default = true`.
    pub fn set_subtrack_graph(
        &mut self,
        subtracks: Vec<(TrackId, String)>,
        sample_rate: u32,
        buffer_size: usize,
    ) {
        use super::node_graph::nodes::{SubtrackInputsNode, MixerNode, GainNode, AutomationInputNode};
        use super::node_graph::nodes::AutomationKeyframe;
        use crate::time::Beats;

        // Preserve existing Volume keyframes before rebuilding.
        let existing_volume_kfs = self.get_volume_automation_keyframes();

        let n = subtracks.len();
        let mut graph = AudioGraph::new(sample_rate, buffer_size);

        // SubtrackInputs node (N outputs, one per child)
        let mut inputs_node = SubtrackInputsNode::new("Subtrack Inputs", subtracks);
        let subtracks_copy = inputs_node.subtracks().to_vec();
        inputs_node.update_subtracks(subtracks_copy, buffer_size);
        let inputs_id = graph.add_node(Box::new(inputs_node));
        graph.set_node_position(inputs_id, 100.0, 150.0);

        // Mixer node
        let mixer_node = Box::new(MixerNode::new("Mixer"));
        let mixer_id = graph.add_node(mixer_node);
        graph.set_node_position(mixer_id, 330.0, 150.0);

        // Gain node — group volume control
        let gain_id = graph.add_node(Box::new(GainNode::new("Volume")));
        graph.set_node_position(gain_id, 520.0, 150.0);

        // AutomationInput — drives the Gain's CV port
        let mut auto_node = AutomationInputNode::new("Volume CV");
        auto_node.set_display_name("Volume".to_string());
        auto_node.value_min = 0.0;
        auto_node.value_max = 2.0;
        auto_node.clear_keyframes();
        if existing_volume_kfs.is_empty() {
            auto_node.add_keyframe(AutomationKeyframe::new(Beats::ZERO, 1.0));
        } else {
            for kf in existing_volume_kfs {
                auto_node.add_keyframe(kf);
            }
        }
        let auto_id = graph.add_node(Box::new(auto_node));
        graph.set_node_position(auto_id, 520.0, 320.0);

        // AudioOutput node
        let output_node = Box::new(AudioOutputNode::new("Audio Output"));
        let output_id = graph.add_node(output_node);
        graph.set_node_position(output_id, 720.0, 150.0);

        // Connect SubtrackInputs[i] → Mixer[i] for each subtrack
        for i in 0..n {
            let _ = graph.connect(inputs_id, i, mixer_id, i);
        }
        let _ = graph.connect(mixer_id, 0, gain_id, 0);    // Mixer → Gain audio
        let _ = graph.connect(auto_id, 0, gain_id, 1);     // AutomationInput → Gain CV
        let _ = graph.connect(gain_id, 0, output_id, 0);   // Gain → Audio Out
        graph.set_output_node(Some(output_id));

        self.audio_graph = graph;
        self.audio_graph_preset = None;
        self.graph_is_default = true;
    }

    /// Extract Volume AutomationInput keyframes from the current graph (if any),
    /// so they can be preserved across `set_subtrack_graph` rebuilds.
    fn get_volume_automation_keyframes(&self) -> Vec<super::node_graph::nodes::AutomationKeyframe> {
        use super::node_graph::nodes::AutomationInputNode;
        for idx in self.audio_graph.node_indices() {
            if let Some(node) = self.audio_graph.get_graph_node(idx) {
                if node.node.node_type() == "AutomationInput" {
                    if let Some(auto_node) = node.node.as_any().downcast_ref::<AutomationInputNode>() {
                        return auto_node.keyframes().to_vec();
                    }
                }
            }
        }
        Vec::new()
    }

    /// Add a new subtrack port to the existing graph.
    ///
    /// If `graph_is_default`: also connects the new port to a new Mixer input.
    /// If the user has modified the graph: just adds the port (unconnected).
    pub fn add_subtrack_to_graph(&mut self, track_id: TrackId, name: String, buffer_size: usize) {
        use super::node_graph::nodes::SubtrackInputsNode;

        // Find SubtrackInputs node index
        let si_idx = self.audio_graph.node_indices()
            .find(|&idx| self.audio_graph.get_graph_node(idx)
                .map(|n| n.node.node_type() == "SubtrackInputs")
                .unwrap_or(false));

        let si_idx = match si_idx {
            Some(idx) => idx,
            None => return, // No subtrack graph set up yet
        };

        // Get current subtrack count (= new port index after adding)
        let new_slot = {
            let gn = self.audio_graph.get_graph_node_mut(si_idx).unwrap();
            let si = gn.node.as_any_mut().downcast_mut::<SubtrackInputsNode>().unwrap();
            let mut subtracks = si.subtracks().to_vec();
            subtracks.push((track_id, name));
            let n = subtracks.len();
            si.update_subtracks(subtracks, buffer_size);
            // Rebuild output buffers for the new port count
            n - 1 // index of the newly added slot
        };
        // Reallocate GraphNode output buffers to match new port count
        self.audio_graph.reallocate_node_output_buffers(si_idx, buffer_size);

        if self.graph_is_default {
            // Find the Mixer node and connect the new subtrack port to a new Mixer input
            let mixer_idx = self.audio_graph.node_indices()
                .find(|&idx| self.audio_graph.get_graph_node(idx)
                    .map(|n| n.node.node_type() == "Mixer")
                    .unwrap_or(false));

            if let Some(mixer_idx) = mixer_idx {
                // n_incoming after connecting = new_slot + 1; auto-grow handled by connect()
                let _ = self.audio_graph.connect(si_idx, new_slot, mixer_idx, new_slot);
            }
        }
    }

    /// Remove a subtrack from the graph (by TrackId).
    ///
    /// Always disconnects any connections from the removed port and removes the port.
    /// If `graph_is_default`: also reshuffles Mixer connections to stay compact.
    pub fn remove_subtrack_from_graph(&mut self, track_id: TrackId, buffer_size: usize) {
        use super::node_graph::nodes::SubtrackInputsNode;

        let si_idx = self.audio_graph.node_indices()
            .find(|&idx| self.audio_graph.get_graph_node(idx)
                .map(|n| n.node.node_type() == "SubtrackInputs")
                .unwrap_or(false));

        let si_idx = match si_idx {
            Some(idx) => idx,
            None => return,
        };

        // Find the slot index for this track
        let slot = {
            let gn = self.audio_graph.get_graph_node(si_idx).unwrap();
            let si = gn.node.as_any().downcast_ref::<SubtrackInputsNode>().unwrap();
            si.subtrack_index_for(track_id)
        };
        let slot = match slot {
            Some(s) => s,
            None => return,
        };

        // Remove all connections from this output port
        self.audio_graph.disconnect_output_port(si_idx, slot);

        // Update the SubtrackInputsNode's subtrack list
        {
            let gn = self.audio_graph.get_graph_node_mut(si_idx).unwrap();
            let si = gn.node.as_any_mut().downcast_mut::<SubtrackInputsNode>().unwrap();
            let mut subtracks = si.subtracks().to_vec();
            subtracks.remove(slot);
            si.update_subtracks(subtracks, buffer_size);
        }
        self.audio_graph.reallocate_node_output_buffers(si_idx, buffer_size);

        if self.graph_is_default {
            // Rebuild default Mixer connections (they've shifted after removal)
            let mixer_idx = self.audio_graph.node_indices()
                .find(|&idx| self.audio_graph.get_graph_node(idx)
                    .map(|n| n.node.node_type() == "Mixer")
                    .unwrap_or(false));

            if let Some(mixer_idx) = mixer_idx {
                // Clear all connections TO mixer
                self.audio_graph.disconnect_all_inputs(mixer_idx);
                // Get new subtrack count
                let n = {
                    let gn = self.audio_graph.get_graph_node(si_idx).unwrap();
                    gn.node.as_any().downcast_ref::<SubtrackInputsNode>().unwrap().num_subtracks()
                };
                // Resize mixer and reconnect
                {
                    let gn = self.audio_graph.get_graph_node_mut(mixer_idx).unwrap();
                    let mixer = gn.node.as_any_mut().downcast_mut::<super::node_graph::nodes::MixerNode>().unwrap();
                    mixer.resize(n + 1);
                }
                for i in 0..n {
                    let _ = self.audio_graph.connect(si_idx, i, mixer_idx, i);
                }
            }
        }
    }

    /// Return the current ordered subtrack list from SubtrackInputsNode, or empty vec if none.
    pub fn current_subtracks(&self) -> Vec<(TrackId, String)> {
        use super::node_graph::nodes::SubtrackInputsNode;
        for idx in self.audio_graph.node_indices().collect::<Vec<_>>() {
            if let Some(gn) = self.audio_graph.get_graph_node(idx) {
                if let Some(si) = gn.node.as_any().downcast_ref::<SubtrackInputsNode>() {
                    return si.subtracks().to_vec();
                }
            }
        }
        Vec::new()
    }

    /// Prepare for serialization by saving the audio graph as a preset
    pub fn prepare_for_save(&mut self) {
        self.audio_graph_preset = Some(self.audio_graph.to_preset("Metatrack Graph"));
    }

    /// Rebuild the audio graph from preset after deserialization.
    ///
    /// After loading, the caller must call `update_subtrack_ids` to re-associate
    /// backend TrackIds with the SubtrackInputsNode's port slots.
    pub fn rebuild_audio_graph(&mut self, sample_rate: u32, buffer_size: usize) -> Result<(), String> {
        if let Some(preset) = &self.audio_graph_preset {
            if !preset.nodes.is_empty() && preset.output_node.is_some() {
                self.audio_graph = AudioGraph::from_preset(preset, sample_rate, buffer_size, None, None)?;
                // graph_is_default remains as serialized (false for user-modified graphs)
            } else {
                self.audio_graph = Self::create_empty_graph(sample_rate, buffer_size);
                self.graph_is_default = true;
            }
        } else {
            self.audio_graph = Self::create_empty_graph(sample_rate, buffer_size);
            self.graph_is_default = true;
        }
        Ok(())
    }

    /// Re-associate backend TrackIds with the SubtrackInputsNode's port slots after reload.
    ///
    /// The preset stores placeholder TrackId=0 entries; this call fills in the real IDs.
    pub fn update_subtrack_ids(&mut self, subtracks: Vec<(TrackId, String)>, buffer_size: usize) {
        use super::node_graph::nodes::SubtrackInputsNode;

        for idx in self.audio_graph.node_indices().collect::<Vec<_>>() {
            if let Some(gn) = self.audio_graph.get_graph_node_mut(idx) {
                if let Some(si) = gn.node.as_any_mut().downcast_mut::<SubtrackInputsNode>() {
                    si.update_subtracks(subtracks, buffer_size);
                    return;
                }
            }
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
    pub fn evaluate_automation_at_time(&self, time: Beats) -> (f32, f32, Seconds) {
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
                        offset = Seconds(automated_value as f64);
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

    /// Check whether this metatrack should produce audio at the given parent time.
    /// Returns false if the playhead is outside the trim window.
    pub fn is_active_at_time(&self, parent_playhead: Seconds) -> bool {
        let local_time = (parent_playhead - self.offset) * self.time_stretch as f64;
        if local_time < self.trim_start {
            return false;
        }
        if let Some(end) = self.trim_end {
            if local_time >= end {
                return false;
            }
        }
        true
    }

    /// Transform a render context for this metatrack's children
    ///
    /// Applies time stretching, offset, and trim transformations.
    /// Time stretch affects how fast content plays: 0.5 = half speed, 2.0 = double speed
    /// Offset shifts content forward/backward in time
    /// Trim start offsets into the internal content
    pub fn transform_context<'a>(&self, ctx: RenderContext<'a>) -> RenderContext<'a> {
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
        let stretched = adjusted_playhead * self.time_stretch as f64;

        // 3. Add trim_start so children see time starting from the trim point
        //    If trim_start=2.0s, children start seeing time 2.0s when parent reaches offset
        transformed.playhead_seconds = stretched + self.trim_start;

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
    /// Clip instances that were active (overlapping playhead) in the previous render buffer.
    /// Used to detect when the playhead exits a clip, so we can send all-notes-off.
    #[serde(skip)]
    prev_active_instances: HashSet<MidiClipInstanceId>,

    /// Peak level of last render() call (for VU metering)
    #[serde(skip, default)]
    pub peak_level: f32,

    /// True while the instrument graph is still the auto-generated default (no user edits).
    /// Used to prompt before loading a preset.
    #[serde(default)]
    pub graph_is_default: bool,
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
            prev_active_instances: HashSet::new(),
            peak_level: 0.0,
            graph_is_default: self.graph_is_default,
        }
    }
}

impl MidiTrack {
    /// Create a new MIDI track with default settings
    pub fn new(id: TrackId, name: String, sample_rate: u32) -> Self {
        // Use a large buffer size that can accommodate any callback
        let default_buffer_size = 8192;

        // Start with empty graph — the frontend loads a default instrument preset
        // (bass.json) via graph_load_preset which replaces the entire graph
        let instrument_graph = AudioGraph::new(sample_rate, default_buffer_size);

        Self {
            id,
            name,
            clip_instances: Vec::new(),
            instrument_graph_preset: None,
            instrument_graph,
            volume: 1.0,
            muted: false,
            solo: false,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
            live_midi_queue: Vec::new(),
            prev_active_instances: HashSet::new(),
            peak_level: 0.0,
            graph_is_default: true,
        }
    }

    /// Prepare for serialization by saving the instrument graph as a preset
    pub fn prepare_for_save(&mut self) {
        self.instrument_graph_preset = Some(self.instrument_graph.to_preset("Instrument Graph"));
    }

    /// Rebuild the instrument graph from preset after deserialization
    pub fn rebuild_audio_graph(&mut self, sample_rate: u32, buffer_size: usize) -> Result<(), String> {
        if let Some(preset) = &self.instrument_graph_preset {
            self.instrument_graph = AudioGraph::from_preset(preset, sample_rate, buffer_size, None, None)?;
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
            note_offs.push(MidiEvent::note_off(Beats::ZERO, 0, note, 0));
        }

        // Create a silent buffer to process the note-offs
        let buffer_size = 512 * 2; // stereo
        let mut silent_buffer = vec![0.0f32; buffer_size];
        self.instrument_graph.process(&mut silent_buffer, &note_offs, Beats::ZERO);
    }

    /// Queue a live MIDI event (from virtual keyboard or MIDI controller)
    pub fn queue_live_midi(&mut self, event: MidiEvent) {
        self.live_midi_queue.push(event);
    }

    /// Clear the live MIDI queue
    pub fn clear_live_midi_queue(&mut self) {
        self.live_midi_queue.clear();
    }

    /// Render this MIDI track into the output buffer.
    ///
    /// When `ctx.live_only` is true, clip event collection is skipped and only the live MIDI
    /// queue is processed. This lets note-off tails (and live keyboard input) route through
    /// the normal group hierarchy without re-triggering notes from clips at the paused position.
    pub fn render(
        &mut self,
        output: &mut [f32],
        midi_pool: &MidiClipPool,
        ctx: RenderContext,
    ) {
        let mut midi_events = Vec::new();

        if !ctx.live_only {
            let playhead_beats = ctx.playhead_beats();
            let buffer_end_beats = ctx.buffer_end_beats();

            // While recording into this track, every clip EXCEPT the one being recorded into is
            // silenced. Otherwise a take folder already sitting in the cycle region would play its
            // active take underneath you on every pass, fighting the part you're trying to record.
            // The recording clip itself is exempt: in merge mode that's the overdub monitoring.
            let muted_clip = match ctx.recording_midi {
                Some((track_id, clip_id)) if track_id == self.id => Some(clip_id),
                _ => None,
            };

            // Collect MIDI events from all clip instances that overlap with current beat range
            let mut currently_active = HashSet::new();
            for instance in &self.clip_instances {
                if muted_clip.is_some_and(|recording| instance.clip_id != recording) {
                    continue;
                }
                if instance.overlaps_range(playhead_beats, buffer_end_beats) {
                    currently_active.insert(instance.id);
                }
                if let Some(clip) = midi_pool.get_clip(instance.clip_id) {
                    let events = instance.get_events_in_range(clip, playhead_beats, buffer_end_beats);
                    midi_events.extend(events);
                }
            }

            // Send all-notes-off for clip instances that just became inactive
            for prev_id in &self.prev_active_instances {
                if !currently_active.contains(prev_id) {
                    for note in 0..128u8 {
                        midi_events.push(MidiEvent::note_off(playhead_beats, 0, note, 0));
                    }
                    break;
                }
            }
            self.prev_active_instances = currently_active;
        }

        // Add live MIDI events (from virtual keyboard or MIDI controllers)
        midi_events.extend(self.live_midi_queue.drain(..));

        // Generate audio using instrument graph
        self.instrument_graph.process(output, &midi_events, ctx.playhead_beats());

        // Evaluate and apply automation (skip automation in live_only mode — no playhead to evaluate at)
        let effective_volume = if ctx.live_only { self.volume } else { self.evaluate_automation_at_time(ctx.playhead_beats()) };

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= effective_volume;
        }
    }

    /// Evaluate automation at a specific time and return the effective volume
    fn evaluate_automation_at_time(&self, time: Beats) -> f32 {
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

    /// Pre-allocated buffer for clip rendering (avoids heap allocation per callback)
    #[serde(skip, default)]
    clip_render_buffer: Vec<f32>,

    /// Peak level of last render() call (for VU metering)
    #[serde(skip, default)]
    pub peak_level: f32,

    /// True while the effects graph is still the auto-generated default (no user edits).
    /// Used to prompt before loading a preset.
    #[serde(default)]
    pub graph_is_default: bool,
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
            clip_render_buffer: Vec::new(),
            peak_level: 0.0,
            graph_is_default: self.graph_is_default,
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
            clip_render_buffer: Vec::new(),
            peak_level: 0.0,
            graph_is_default: true,
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
                self.effects_graph = AudioGraph::from_preset(preset, sample_rate, buffer_size, None, None)?;
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
        ctx: RenderContext<'_>,
    ) -> usize {
        let buffer_end = ctx.buffer_end();

        // Split borrow: take clip_render_buffer out to avoid borrow conflict with &self methods
        let mut clip_buffer = std::mem::take(&mut self.clip_render_buffer);
        clip_buffer.resize(output.len(), 0.0);
        clip_buffer.fill(0.0);
        let mut rendered = 0;

        // Render all active clip instances into the buffer
        for clip in &self.clips {
            if clip.external_start_secs(ctx.tempo_map) < buffer_end && clip.external_end_secs(ctx.tempo_map) > ctx.playhead_seconds {
                rendered += self.render_clip(clip, &mut clip_buffer, pool, ctx);
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
        self.effects_graph.process(output, &[], ctx.playhead_beats());

        // Put the buffer back for reuse next callback
        self.clip_render_buffer = clip_buffer;

        // Evaluate and apply automation
        let effective_volume = self.evaluate_automation_at_time(ctx.playhead_beats());

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= effective_volume;
        }

        rendered
    }

    /// Evaluate automation at a specific time and return the effective volume
    fn evaluate_automation_at_time(&self, time: Beats) -> f32 {
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
        ctx: RenderContext<'_>,
    ) -> usize {
        let playhead = ctx.playhead_seconds;
        let buffer_end = ctx.buffer_end();
        let tempo_map = ctx.tempo_map;
        let sample_rate = ctx.sample_rate;
        let channels = ctx.channels;

        // Determine the time range we need to render (intersection of buffer and clip external bounds)
        let render_start = playhead.max(clip.external_start_secs(tempo_map));
        let render_end = buffer_end.min(clip.external_end_secs(tempo_map));

        if render_start >= render_end {
            return 0;
        }

        let internal_duration = clip.internal_duration();
        if internal_duration <= Seconds::ZERO {
            return 0;
        }

        let combined_gain = clip.gain;
        let mut total_rendered = 0;
        let samples_per_second = sample_rate as f64 * channels as f64;

        let output_start_offset = ((render_start - playhead).0 * samples_per_second + 0.5) as usize;
        let output_end_offset = ((render_end - playhead).0 * samples_per_second + 0.5) as usize;

        if output_end_offset > output.len() || output_start_offset > output.len() {
            return 0;
        }

        if !clip.is_looping(tempo_map) {
            let content_start = clip.get_content_position(render_start, tempo_map).unwrap_or(clip.internal_start);
            let output_len = output.len();
            let output_slice = &mut output[output_start_offset..output_end_offset.min(output_len)];

            total_rendered = pool.render_from_file(
                clip.audio_pool_index,
                output_slice,
                content_start,
                combined_gain,
                sample_rate,
                channels,
                clip.read_ahead.as_deref(),
            );
        } else {
            // Looping case: handle wrap-around at loop boundaries
            let mut timeline_pos = render_start;
            let mut output_offset = output_start_offset;

            while timeline_pos < render_end && output_offset < output.len() {
                let relative_pos = timeline_pos - clip.external_start_secs(tempo_map);
                let loop_offset = relative_pos.0 % internal_duration.0;
                let content_pos = clip.internal_start + Seconds(loop_offset);

                let time_to_loop_end = Seconds(internal_duration.0 - loop_offset);
                let time_to_render_end = render_end - timeline_pos;
                let chunk_duration = time_to_loop_end.min(time_to_render_end);

                let chunk_samples = (chunk_duration.0 * samples_per_second) as usize;
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
                    clip.read_ahead.as_deref(),
                );

                total_rendered += rendered;
                output_offset += chunk_samples;
                timeline_pos = timeline_pos + chunk_duration;
            }
        }

        total_rendered
    }
}
