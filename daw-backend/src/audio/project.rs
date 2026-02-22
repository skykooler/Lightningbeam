use super::buffer_pool::BufferPool;
use super::clip::{AudioClipInstanceId, Clip};
use super::midi::{MidiClip, MidiClipId, MidiClipInstance, MidiClipInstanceId, MidiEvent};
use super::midi_pool::MidiClipPool;
use super::pool::AudioClipPool;
use super::track::{AudioTrack, Metatrack, MidiTrack, RenderContext, TrackId, TrackNode};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Project manages the hierarchical track structure and clip pools
///
/// Tracks are stored in a flat HashMap but can be organized into groups,
/// forming a tree structure. Groups render their children recursively.
///
/// Clip content is stored in pools (MidiClipPool), while tracks store
/// clip instances that reference the pool content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    tracks: HashMap<TrackId, TrackNode>,
    next_track_id: TrackId,
    root_tracks: Vec<TrackId>, // Top-level tracks (not in any group)
    sample_rate: u32, // System sample rate
    /// Pool for MIDI clip content
    pub midi_clip_pool: MidiClipPool,
    /// Next MIDI clip instance ID (for generating unique IDs)
    next_midi_clip_instance_id: MidiClipInstanceId,
}

impl Project {
    /// Create a new empty project
    pub fn new(sample_rate: u32) -> Self {
        Self {
            tracks: HashMap::new(),
            next_track_id: 0,
            root_tracks: Vec::new(),
            sample_rate,
            midi_clip_pool: MidiClipPool::new(),
            next_midi_clip_instance_id: 1,
        }
    }

    /// Generate a new unique track ID
    fn next_id(&mut self) -> TrackId {
        let id = self.next_track_id;
        self.next_track_id += 1;
        id
    }

    /// Add an audio track to the project
    ///
    /// # Arguments
    /// * `name` - Track name
    /// * `parent_id` - Optional parent group ID
    ///
    /// # Returns
    /// The new track's ID
    pub fn add_audio_track(&mut self, name: String, parent_id: Option<TrackId>) -> TrackId {
        let id = self.next_id();
        let track = AudioTrack::new(id, name, self.sample_rate);
        self.tracks.insert(id, TrackNode::Audio(track));

        if let Some(parent) = parent_id {
            // Add to parent group
            if let Some(TrackNode::Group(group)) = self.tracks.get_mut(&parent) {
                group.add_child(id);
            }
        } else {
            // Add to root level
            self.root_tracks.push(id);
        }

        id
    }

    /// Add a group track to the project
    ///
    /// # Arguments
    /// * `name` - Group name
    /// * `parent_id` - Optional parent group ID
    ///
    /// # Returns
    /// The new group's ID
    pub fn add_group_track(&mut self, name: String, parent_id: Option<TrackId>) -> TrackId {
        let id = self.next_id();
        let group = Metatrack::new(id, name, self.sample_rate);
        self.tracks.insert(id, TrackNode::Group(group));

        if let Some(parent) = parent_id {
            // Add to parent group
            if let Some(TrackNode::Group(parent_group)) = self.tracks.get_mut(&parent) {
                parent_group.add_child(id);
            }
        } else {
            // Add to root level
            self.root_tracks.push(id);
        }

        id
    }

    /// Add a MIDI track to the project
    ///
    /// # Arguments
    /// * `name` - Track name
    /// * `parent_id` - Optional parent group ID
    ///
    /// # Returns
    /// The new track's ID
    pub fn add_midi_track(&mut self, name: String, parent_id: Option<TrackId>) -> TrackId {
        let id = self.next_id();
        let track = MidiTrack::new(id, name, self.sample_rate);
        self.tracks.insert(id, TrackNode::Midi(track));

        if let Some(parent) = parent_id {
            // Add to parent group
            if let Some(TrackNode::Group(group)) = self.tracks.get_mut(&parent) {
                group.add_child(id);
            }
        } else {
            // Add to root level
            self.root_tracks.push(id);
        }

        id
    }

    /// Remove a track from the project
    ///
    /// If the track is a group, all children are moved to the parent (or root)
    pub fn remove_track(&mut self, track_id: TrackId) {
        if let Some(node) = self.tracks.remove(&track_id) {
            // If it's a group, handle its children
            if let TrackNode::Group(group) = node {
                // Find the parent of this group
                let parent_id = self.find_parent(track_id);

                // Move children to parent or root
                for child_id in group.children {
                    if let Some(parent) = parent_id {
                        if let Some(TrackNode::Group(parent_group)) = self.tracks.get_mut(&parent) {
                            parent_group.add_child(child_id);
                        }
                    } else {
                        self.root_tracks.push(child_id);
                    }
                }
            }

            // Remove from parent or root
            if let Some(parent_id) = self.find_parent(track_id) {
                if let Some(TrackNode::Group(parent)) = self.tracks.get_mut(&parent_id) {
                    parent.remove_child(track_id);
                }
            } else {
                self.root_tracks.retain(|&id| id != track_id);
            }
        }
    }

    /// Find the parent group of a track
    fn find_parent(&self, track_id: TrackId) -> Option<TrackId> {
        for (id, node) in &self.tracks {
            if let TrackNode::Group(group) = node {
                if group.children.contains(&track_id) {
                    return Some(*id);
                }
            }
        }
        None
    }

    /// Move a track to a different group
    pub fn move_to_group(&mut self, track_id: TrackId, new_parent_id: TrackId) {
        // First remove from current parent
        if let Some(old_parent_id) = self.find_parent(track_id) {
            if let Some(TrackNode::Group(parent)) = self.tracks.get_mut(&old_parent_id) {
                parent.remove_child(track_id);
            }
        } else {
            // Remove from root
            self.root_tracks.retain(|&id| id != track_id);
        }

        // Add to new parent
        if let Some(TrackNode::Group(new_parent)) = self.tracks.get_mut(&new_parent_id) {
            new_parent.add_child(track_id);
        }
    }

    /// Move a track to the root level (remove from any group)
    pub fn move_to_root(&mut self, track_id: TrackId) {
        // Remove from current parent if any
        if let Some(parent_id) = self.find_parent(track_id) {
            if let Some(TrackNode::Group(parent)) = self.tracks.get_mut(&parent_id) {
                parent.remove_child(track_id);
            }
            // Add to root if not already there
            if !self.root_tracks.contains(&track_id) {
                self.root_tracks.push(track_id);
            }
        }
    }

    /// Get a reference to a track node
    pub fn get_track(&self, track_id: TrackId) -> Option<&TrackNode> {
        self.tracks.get(&track_id)
    }

    /// Get a mutable reference to a track node
    pub fn get_track_mut(&mut self, track_id: TrackId) -> Option<&mut TrackNode> {
        self.tracks.get_mut(&track_id)
    }

    /// Get oscilloscope data from a node in a track's graph
    pub fn get_oscilloscope_data(&self, track_id: TrackId, node_id: u32, sample_count: usize) -> Option<(Vec<f32>, Vec<f32>)> {
        if let Some(TrackNode::Midi(track)) = self.tracks.get(&track_id) {
            let graph = &track.instrument_graph;
            let node_idx = petgraph::stable_graph::NodeIndex::new(node_id as usize);

            // Get audio data
            let audio = graph.get_oscilloscope_data(node_idx, sample_count)?;

            // Get CV data (may be empty if no CV input or not an oscilloscope node)
            let cv = graph.get_oscilloscope_cv_data(node_idx, sample_count).unwrap_or_default();

            return Some((audio, cv));
        }
        None
    }

    /// Get oscilloscope data from a node inside a VoiceAllocator's best voice
    pub fn get_voice_oscilloscope_data(&self, track_id: TrackId, va_node_id: u32, inner_node_id: u32, sample_count: usize) -> Option<(Vec<f32>, Vec<f32>)> {
        if let Some(TrackNode::Midi(track)) = self.tracks.get(&track_id) {
            let graph = &track.instrument_graph;
            let va_idx = petgraph::stable_graph::NodeIndex::new(va_node_id as usize);
            let node = graph.get_node(va_idx)?;
            let va = node.as_any().downcast_ref::<crate::audio::node_graph::nodes::VoiceAllocatorNode>()?;
            return va.get_voice_oscilloscope_data(inner_node_id, sample_count);
        }
        None
    }

    /// Get all root-level track IDs
    pub fn root_tracks(&self) -> &[TrackId] {
        &self.root_tracks
    }

    /// Get the number of tracks in the project
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Check if any track is soloed
    pub fn any_solo(&self) -> bool {
        self.tracks.values().any(|node| node.is_solo())
    }

    /// Add a clip to an audio track
    pub fn add_clip(&mut self, track_id: TrackId, clip: Clip) -> Result<AudioClipInstanceId, &'static str> {
        if let Some(TrackNode::Audio(track)) = self.tracks.get_mut(&track_id) {
            let instance_id = clip.id;
            track.add_clip(clip);
            Ok(instance_id)
        } else {
            Err("Track not found or is not an audio track")
        }
    }

    /// Add a MIDI clip instance to a MIDI track
    /// The clip content should already exist in the midi_clip_pool
    pub fn add_midi_clip_instance(&mut self, track_id: TrackId, instance: MidiClipInstance) -> Result<(), &'static str> {
        if let Some(TrackNode::Midi(track)) = self.tracks.get_mut(&track_id) {
            track.add_clip_instance(instance);
            Ok(())
        } else {
            Err("Track not found or is not a MIDI track")
        }
    }

    /// Create a new MIDI clip in the pool and add an instance to a track
    /// Returns (clip_id, instance_id) on success
    pub fn create_midi_clip_with_instance(
        &mut self,
        track_id: TrackId,
        events: Vec<MidiEvent>,
        duration: f64,
        name: String,
        external_start: f64,
    ) -> Result<(MidiClipId, MidiClipInstanceId), &'static str> {
        // Verify track exists and is a MIDI track
        if !matches!(self.tracks.get(&track_id), Some(TrackNode::Midi(_))) {
            return Err("Track not found or is not a MIDI track");
        }

        // Create clip in pool
        let clip_id = self.midi_clip_pool.add_clip(events, duration, name);

        // Create instance
        let instance_id = self.next_midi_clip_instance_id;
        self.next_midi_clip_instance_id += 1;

        let instance = MidiClipInstance::from_full_clip(instance_id, clip_id, duration, external_start);

        // Add instance to track
        if let Some(TrackNode::Midi(track)) = self.tracks.get_mut(&track_id) {
            track.add_clip_instance(instance);
        }

        Ok((clip_id, instance_id))
    }

    /// Generate a new unique MIDI clip instance ID
    pub fn next_midi_clip_instance_id(&mut self) -> MidiClipInstanceId {
        let id = self.next_midi_clip_instance_id;
        self.next_midi_clip_instance_id += 1;
        id
    }

    /// Legacy method for backwards compatibility - creates clip and instance from old MidiClip format
    pub fn add_midi_clip(&mut self, track_id: TrackId, clip: MidiClip) -> Result<MidiClipInstanceId, &'static str> {
        self.add_midi_clip_at(track_id, clip, 0.0)
    }

    /// Add a MIDI clip to the pool and create an instance at the given timeline position
    pub fn add_midi_clip_at(&mut self, track_id: TrackId, clip: MidiClip, start_time: f64) -> Result<MidiClipInstanceId, &'static str> {
        // Add the clip to the pool (it already has events and duration)
        let duration = clip.duration;
        let clip_id = clip.id;
        self.midi_clip_pool.add_existing_clip(clip);

        // Create an instance that uses the full clip at the given position
        let instance_id = self.next_midi_clip_instance_id();
        let instance = MidiClipInstance::from_full_clip(instance_id, clip_id, duration, start_time);

        self.add_midi_clip_instance(track_id, instance)?;
        Ok(instance_id)
    }

    /// Remove a MIDI clip instance from a track (for undo/redo support)
    pub fn remove_midi_clip(&mut self, track_id: TrackId, instance_id: MidiClipInstanceId) -> Result<(), &'static str> {
        if let Some(track) = self.get_track_mut(track_id) {
            track.remove_midi_clip_instance(instance_id);
            Ok(())
        } else {
            Err("Track not found")
        }
    }

    /// Remove an audio clip instance from a track (for undo/redo support)
    pub fn remove_audio_clip(&mut self, track_id: TrackId, instance_id: AudioClipInstanceId) -> Result<(), &'static str> {
        if let Some(track) = self.get_track_mut(track_id) {
            track.remove_audio_clip_instance(instance_id);
            Ok(())
        } else {
            Err("Track not found")
        }
    }

    /// Render all root tracks into the output buffer
    pub fn render(
        &mut self,
        output: &mut [f32],
        audio_pool: &AudioClipPool,
        buffer_pool: &mut BufferPool,
        playhead_seconds: f64,
        sample_rate: u32,
        channels: u32,
    ) {
        output.fill(0.0);

        let any_solo = self.any_solo();

        // Create initial render context
        let ctx = RenderContext::new(
            playhead_seconds,
            sample_rate,
            channels,
            output.len(),
        );

        // Render each root track (index-based to avoid clone)
        for i in 0..self.root_tracks.len() {
            let track_id = self.root_tracks[i];
            self.render_track(
                track_id,
                output,
                audio_pool,
                buffer_pool,
                ctx,
                any_solo,
                false, // root tracks are not inside a soloed parent
            );
        }
    }

    /// Recursively render a track (audio or group) into the output buffer
    fn render_track(
        &mut self,
        track_id: TrackId,
        output: &mut [f32],
        audio_pool: &AudioClipPool,
        buffer_pool: &mut BufferPool,
        ctx: RenderContext,
        any_solo: bool,
        parent_is_soloed: bool,
    ) {
        // Check if track should be rendered based on mute/solo
        let should_render = match self.tracks.get(&track_id) {
            Some(TrackNode::Audio(track)) => {
                // If parent is soloed, only check mute state
                // Otherwise, check normal solo logic
                if parent_is_soloed {
                    !track.muted
                } else {
                    track.is_active(any_solo)
                }
            }
            Some(TrackNode::Midi(track)) => {
                // Same logic for MIDI tracks
                if parent_is_soloed {
                    !track.muted
                } else {
                    track.is_active(any_solo)
                }
            }
            Some(TrackNode::Group(group)) => {
                // Same logic for groups
                if parent_is_soloed {
                    !group.muted
                } else {
                    group.is_active(any_solo)
                }
            }
            None => return,
        };

        if !should_render {
            return;
        }

        // Handle audio track vs MIDI track vs group track
        match self.tracks.get_mut(&track_id) {
            Some(TrackNode::Audio(track)) => {
                // Render audio track directly into output
                track.render(output, audio_pool, ctx.playhead_seconds, ctx.sample_rate, ctx.channels);
            }
            Some(TrackNode::Midi(track)) => {
                // Render MIDI track directly into output
                // Access midi_clip_pool from self - safe because we only need immutable access
                track.render(output, &self.midi_clip_pool, ctx.playhead_seconds, ctx.sample_rate, ctx.channels);
            }
            Some(TrackNode::Group(group)) => {
                // Skip rendering if playhead is outside the metatrack's trim window
                if !group.is_active_at_time(ctx.playhead_seconds) {
                    return;
                }

                // Read group properties and transform context (index-based child iteration to avoid clone)
                let num_children = group.children.len();
                let this_group_is_soloed = group.solo;
                let child_ctx = group.transform_context(ctx);

                // Acquire a temporary buffer for the group mix
                let mut group_buffer = buffer_pool.acquire();
                group_buffer.resize(output.len(), 0.0);
                group_buffer.fill(0.0);

                // Recursively render all children into the group buffer
                // If this group is soloed (or parent was soloed), children inherit that state
                let children_parent_soloed = parent_is_soloed || this_group_is_soloed;
                for i in 0..num_children {
                    let child_id = match self.tracks.get(&track_id) {
                        Some(TrackNode::Group(g)) => g.children[i],
                        _ => break,
                    };
                    self.render_track(
                        child_id,
                        &mut group_buffer,
                        audio_pool,
                        buffer_pool,
                        child_ctx,
                        any_solo,
                        children_parent_soloed,
                    );
                }

                // Route children's mix through metatrack's audio graph
                if let Some(TrackNode::Group(group)) = self.tracks.get_mut(&track_id) {
                    // Inject children's mix into audio graph's input node
                    let node_indices: Vec<_> = group.audio_graph.node_indices().collect();
                    for node_idx in node_indices {
                        if let Some(graph_node) = group.audio_graph.get_graph_node_mut(node_idx) {
                            if graph_node.node.node_type() == "AudioInput" {
                                if let Some(input_node) = graph_node.node.as_any_mut()
                                    .downcast_mut::<super::node_graph::nodes::AudioInputNode>()
                                {
                                    input_node.inject_audio(&group_buffer);
                                    break;
                                }
                            }
                        }
                    }

                    // Process through the audio graph into a fresh buffer
                    let mut graph_output = buffer_pool.acquire();
                    graph_output.resize(output.len(), 0.0);
                    graph_output.fill(0.0);
                    group.audio_graph.process(&mut graph_output, &[], ctx.playhead_seconds);

                    // Apply group volume and mix into output
                    for (out_sample, graph_sample) in output.iter_mut().zip(graph_output.iter()) {
                        *out_sample += graph_sample * group.volume;
                    }
                    buffer_pool.release(graph_output);
                }

                // Release children mix buffer back to pool
                buffer_pool.release(group_buffer);
            }
            None => {}
        }
    }

    /// Reset all per-clip read-ahead target frames before a new render cycle.
    pub fn reset_read_ahead_targets(&self) {
        for track in self.tracks.values() {
            if let TrackNode::Audio(audio_track) = track {
                for clip in &audio_track.clips {
                    if let Some(ra) = clip.read_ahead.as_deref() {
                        ra.reset_target_frame();
                    }
                }
            }
        }
    }

    /// Stop all notes on all MIDI tracks
    pub fn stop_all_notes(&mut self) {
        for track in self.tracks.values_mut() {
            if let TrackNode::Midi(midi_track) = track {
                midi_track.stop_all_notes();
            }
        }
    }

    /// Set export (blocking) mode on all clip read-ahead buffers.
    /// When enabled, `render_from_file` blocks until the disk reader
    /// has filled the needed frames instead of returning silence.
    pub fn set_export_mode(&self, export: bool) {
        for track in self.tracks.values() {
            if let TrackNode::Audio(t) = track {
                for clip in &t.clips {
                    if let Some(ref ra) = clip.read_ahead {
                        ra.set_export_mode(export);
                    }
                }
            }
        }
    }

    /// Reset all node graphs (clears effect buffers on seek)
    pub fn reset_all_graphs(&mut self) {
        for track in self.tracks.values_mut() {
            match track {
                TrackNode::Audio(t) => t.effects_graph.reset(),
                TrackNode::Midi(t) => t.instrument_graph.reset(),
                TrackNode::Group(_) => {}
            }
        }
    }

    /// Propagate tempo to all audio graphs (for BeatNode sync)
    pub fn set_tempo(&mut self, bpm: f32, beats_per_bar: u32) {
        for track in self.tracks.values_mut() {
            match track {
                TrackNode::Audio(t) => t.effects_graph.set_tempo(bpm, beats_per_bar),
                TrackNode::Midi(t) => t.instrument_graph.set_tempo(bpm, beats_per_bar),
                TrackNode::Group(g) => g.audio_graph.set_tempo(bpm, beats_per_bar),
            }
        }
    }

    /// Process live MIDI input from all MIDI tracks (called even when not playing)
    pub fn process_live_midi(&mut self, output: &mut [f32], sample_rate: u32, channels: u32) {
        // Process all MIDI tracks to handle queued live input events
        for track in self.tracks.values_mut() {
            if let TrackNode::Midi(midi_track) = track {
                // Process only queued live events, not clips
                midi_track.process_live_input(output, sample_rate, channels);
            }
        }
    }

    /// Send a live MIDI note on event to a track's instrument
    /// Note: With node-based instruments, MIDI events are handled during the process() call
    pub fn send_midi_note_on(&mut self, track_id: TrackId, note: u8, velocity: u8) {
        // Queue the MIDI note-on event to the track's live MIDI queue
        if let Some(TrackNode::Midi(track)) = self.tracks.get_mut(&track_id) {
            let event = MidiEvent::note_on(0.0, 0, note, velocity);
            track.queue_live_midi(event);
        }
    }

    /// Send a live MIDI note off event to a track's instrument
    pub fn send_midi_note_off(&mut self, track_id: TrackId, note: u8) {
        // Queue the MIDI note-off event to the track's live MIDI queue
        if let Some(TrackNode::Midi(track)) = self.tracks.get_mut(&track_id) {
            let event = MidiEvent::note_off(0.0, 0, note, 0);
            track.queue_live_midi(event);
        }
    }

    /// Prepare all tracks for serialization by saving their audio graphs as presets
    pub fn prepare_for_save(&mut self) {
        for track in self.tracks.values_mut() {
            match track {
                TrackNode::Audio(audio_track) => {
                    audio_track.prepare_for_save();
                }
                TrackNode::Midi(midi_track) => {
                    midi_track.prepare_for_save();
                }
                TrackNode::Group(group) => {
                    group.prepare_for_save();
                }
            }
        }
    }

    /// Rebuild all audio graphs from presets after deserialization
    ///
    /// This should be called after deserializing a Project to reconstruct
    /// the AudioGraph instances from their stored presets.
    ///
    /// # Arguments
    /// * `buffer_size` - Buffer size for audio processing (typically 8192)
    pub fn rebuild_audio_graphs(&mut self, buffer_size: usize) -> Result<(), String> {
        for track in self.tracks.values_mut() {
            match track {
                TrackNode::Audio(audio_track) => {
                    audio_track.rebuild_audio_graph(self.sample_rate, buffer_size)?;
                }
                TrackNode::Midi(midi_track) => {
                    midi_track.rebuild_audio_graph(self.sample_rate, buffer_size)?;
                }
                TrackNode::Group(group) => {
                    group.rebuild_audio_graph(self.sample_rate, buffer_size)?;
                }
            }
        }
        Ok(())
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new(48000) // Use 48kHz as default, will be overridden when created with actual sample rate
    }
}
