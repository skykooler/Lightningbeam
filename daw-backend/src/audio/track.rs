use super::automation::{AutomationLane, AutomationLaneId, ParameterId};
use super::clip::Clip;
use super::midi::{MidiClip, MidiEvent};
use super::node_graph::AudioGraph;
use super::node_graph::nodes::{AudioInputNode, AudioOutputNode};
use super::pool::AudioPool;
use std::collections::HashMap;

/// Track ID type
pub type TrackId = u32;

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
}

/// Metatrack that contains other tracks with time transformation capabilities
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

/// MIDI track with MIDI clips and a node-based instrument
pub struct MidiTrack {
    pub id: TrackId,
    pub name: String,
    pub clips: Vec<MidiClip>,
    pub instrument_graph: AudioGraph,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
    /// Automation lanes for this track
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,
    /// Queue for live MIDI input (virtual keyboard, MIDI controllers)
    live_midi_queue: Vec<MidiEvent>,
}

impl MidiTrack {
    /// Create a new MIDI track with default settings
    pub fn new(id: TrackId, name: String, sample_rate: u32) -> Self {
        // Use a large buffer size that can accommodate any callback
        let default_buffer_size = 8192;

        Self {
            id,
            name,
            clips: Vec::new(),
            instrument_graph: AudioGraph::new(sample_rate, default_buffer_size),
            volume: 1.0,
            muted: false,
            solo: false,
            automation_lanes: HashMap::new(),
            next_automation_id: 0,
            live_midi_queue: Vec::new(),
        }
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

    /// Add a MIDI clip to this track
    pub fn add_clip(&mut self, clip: MidiClip) {
        self.clips.push(clip);
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
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Collect MIDI events from all clips that overlap with current time range
        let mut midi_events = Vec::new();
        for clip in &self.clips {
            let events = clip.get_events_in_range(
                playhead_seconds,
                buffer_end_seconds,
                sample_rate,
            );

            // Events now have timestamps in seconds relative to clip start
            midi_events.extend(events);
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

/// Audio track with clips
pub struct AudioTrack {
    pub id: TrackId,
    pub name: String,
    pub clips: Vec<Clip>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
    /// Automation lanes for this track
    pub automation_lanes: HashMap<AutomationLaneId, AutomationLane>,
    next_automation_id: AutomationLaneId,
    /// Effects processing graph for this audio track
    pub effects_graph: AudioGraph,
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
            effects_graph,
        }
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

    /// Add a clip to this track
    pub fn add_clip(&mut self, clip: Clip) {
        self.clips.push(clip);
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
        pool: &AudioPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Create a temporary buffer for clip rendering
        let mut clip_buffer = vec![0.0f32; output.len()];
        let mut rendered = 0;

        // Render all active clips into the temporary buffer
        for clip in &self.clips {
            // Check if clip overlaps with current buffer time range
            if clip.start_time < buffer_end_seconds && clip.end_time() > playhead_seconds {
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

    /// Render a single clip into the output buffer
    fn render_clip(
        &self,
        clip: &Clip,
        output: &mut [f32],
        pool: &AudioPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) -> usize {
        let buffer_duration_seconds = output.len() as f64 / (sample_rate as f64 * channels as f64);
        let buffer_end_seconds = playhead_seconds + buffer_duration_seconds;

        // Determine the time range we need to render (intersection of buffer and clip)
        let render_start_seconds = playhead_seconds.max(clip.start_time);
        let render_end_seconds = buffer_end_seconds.min(clip.end_time());

        // If no overlap, return early
        if render_start_seconds >= render_end_seconds {
            return 0;
        }

        // Calculate offset into the output buffer (in interleaved samples)
        let output_offset_seconds = render_start_seconds - playhead_seconds;
        let output_offset_samples = (output_offset_seconds * sample_rate as f64 * channels as f64) as usize;

        // Calculate position within the clip's audio file (in seconds)
        let clip_position_seconds = render_start_seconds - clip.start_time + clip.offset;

        // Calculate how many samples to render in the output
        let render_duration_seconds = render_end_seconds - render_start_seconds;
        let samples_to_render = (render_duration_seconds * sample_rate as f64 * channels as f64) as usize;
        let samples_to_render = samples_to_render.min(output.len() - output_offset_samples);

        // Get the slice of output buffer to write to
        if output_offset_samples + samples_to_render > output.len() {
            return 0;
        }

        let output_slice = &mut output[output_offset_samples..output_offset_samples + samples_to_render];

        // Calculate combined gain
        let combined_gain = clip.gain * self.volume;

        // Render from pool with sample rate conversion
        // Pass the time position in seconds, let the pool handle sample rate conversion
        pool.render_from_file(
            clip.audio_pool_index,
            output_slice,
            clip_position_seconds,
            combined_gain,
            sample_rate,
            channels,
        )
    }
}
