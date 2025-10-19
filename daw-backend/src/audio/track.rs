use super::clip::Clip;
use super::midi::MidiClip;
use super::pool::AudioPool;
use crate::effects::{Effect, SimpleSynth};

/// Track ID type
pub type TrackId = u32;

/// Type alias for backwards compatibility
pub type Track = AudioTrack;

/// Node in the track hierarchy - can be an audio track, MIDI track, or a group
pub enum TrackNode {
    Audio(AudioTrack),
    Midi(MidiTrack),
    Group(GroupTrack),
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

/// Group track that contains other tracks (audio or groups)
pub struct GroupTrack {
    pub id: TrackId,
    pub name: String,
    pub children: Vec<TrackId>,
    pub effects: Vec<Box<dyn Effect>>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

impl GroupTrack {
    /// Create a new group track
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            name,
            children: Vec::new(),
            effects: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
        }
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

    /// Add an effect to the group's effect chain
    pub fn add_effect(&mut self, effect: Box<dyn Effect>) {
        self.effects.push(effect);
    }

    /// Clear all effects from the group
    pub fn clear_effects(&mut self) {
        self.effects.clear();
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
}

/// MIDI track with MIDI clips and a virtual instrument
pub struct MidiTrack {
    pub id: TrackId,
    pub name: String,
    pub clips: Vec<MidiClip>,
    pub instrument: SimpleSynth,
    pub effects: Vec<Box<dyn Effect>>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

impl MidiTrack {
    /// Create a new MIDI track with default settings
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            name,
            clips: Vec::new(),
            instrument: SimpleSynth::new(),
            effects: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
        }
    }

    /// Add an effect to the track's effect chain
    pub fn add_effect(&mut self, effect: Box<dyn Effect>) {
        self.effects.push(effect);
    }

    /// Clear all effects from the track
    pub fn clear_effects(&mut self) {
        self.effects.clear();
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
        for clip in &self.clips {
            let events = clip.get_events_in_range(
                playhead_seconds,
                buffer_end_seconds,
                sample_rate,
            );

            // Queue events in the instrument
            for (_timestamp, event) in events {
                self.instrument.queue_event(event);
            }
        }

        // Generate audio from the instrument
        self.instrument.process(output, channels as usize, sample_rate);

        // Apply effect chain
        for effect in &mut self.effects {
            effect.process(output, channels as usize, sample_rate);
        }

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= self.volume;
        }
    }
}

/// Audio track with clips and effect chain
pub struct AudioTrack {
    pub id: TrackId,
    pub name: String,
    pub clips: Vec<Clip>,
    pub effects: Vec<Box<dyn Effect>>,
    pub volume: f32,
    pub muted: bool,
    pub solo: bool,
}

impl AudioTrack {
    /// Create a new audio track with default settings
    pub fn new(id: TrackId, name: String) -> Self {
        Self {
            id,
            name,
            clips: Vec::new(),
            effects: Vec::new(),
            volume: 1.0,
            muted: false,
            solo: false,
        }
    }

    /// Add an effect to the track's effect chain
    pub fn add_effect(&mut self, effect: Box<dyn Effect>) {
        self.effects.push(effect);
    }

    /// Remove an effect from the chain by index
    pub fn remove_effect(&mut self, index: usize) -> Option<Box<dyn Effect>> {
        if index < self.effects.len() {
            Some(self.effects.remove(index))
        } else {
            None
        }
    }

    /// Clear all effects from the track
    pub fn clear_effects(&mut self) {
        self.effects.clear();
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

        let mut rendered = 0;

        // Render all active clips
        for clip in &self.clips {
            // Check if clip overlaps with current buffer time range
            if clip.start_time < buffer_end_seconds && clip.end_time() > playhead_seconds {
                rendered += self.render_clip(
                    clip,
                    output,
                    pool,
                    playhead_seconds,
                    sample_rate,
                    channels,
                );
            }
        }

        // Apply effect chain
        for effect in &mut self.effects {
            effect.process(output, channels as usize, sample_rate);
        }

        // Apply track volume
        for sample in output.iter_mut() {
            *sample *= self.volume;
        }

        rendered
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
