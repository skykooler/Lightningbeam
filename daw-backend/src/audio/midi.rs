/// MIDI event representing a single MIDI message
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MidiEvent {
    /// Time position within the clip in seconds (sample-rate independent)
    pub timestamp: f64,
    /// MIDI status byte (includes channel)
    pub status: u8,
    /// First data byte (note number, CC number, etc.)
    pub data1: u8,
    /// Second data byte (velocity, CC value, etc.)
    pub data2: u8,
}

impl MidiEvent {
    /// Create a new MIDI event
    pub fn new(timestamp: f64, status: u8, data1: u8, data2: u8) -> Self {
        Self {
            timestamp,
            status,
            data1,
            data2,
        }
    }

    /// Create a note on event
    pub fn note_on(timestamp: f64, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            timestamp,
            status: 0x90 | (channel & 0x0F),
            data1: note,
            data2: velocity,
        }
    }

    /// Create a note off event
    pub fn note_off(timestamp: f64, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            timestamp,
            status: 0x80 | (channel & 0x0F),
            data1: note,
            data2: velocity,
        }
    }

    /// Check if this is a note on event (with non-zero velocity)
    pub fn is_note_on(&self) -> bool {
        (self.status & 0xF0) == 0x90 && self.data2 > 0
    }

    /// Check if this is a note off event (or note on with zero velocity)
    pub fn is_note_off(&self) -> bool {
        (self.status & 0xF0) == 0x80 || ((self.status & 0xF0) == 0x90 && self.data2 == 0)
    }

    /// Get the MIDI channel (0-15)
    pub fn channel(&self) -> u8 {
        self.status & 0x0F
    }

    /// Get the message type (upper 4 bits of status)
    pub fn message_type(&self) -> u8 {
        self.status & 0xF0
    }
}

/// MIDI clip ID type (for clips stored in the pool)
pub type MidiClipId = u32;

/// MIDI clip instance ID type (for instances placed on tracks)
pub type MidiClipInstanceId = u32;

/// MIDI clip content - stores the actual MIDI events
///
/// This represents the content data stored in the MidiClipPool.
/// Events have timestamps relative to the start of the clip (0.0 = clip beginning).
#[derive(Debug, Clone)]
pub struct MidiClip {
    pub id: MidiClipId,
    pub events: Vec<MidiEvent>,
    pub duration: f64,    // Total content duration in seconds
    pub name: String,
}

impl MidiClip {
    /// Create a new MIDI clip with content
    pub fn new(id: MidiClipId, events: Vec<MidiEvent>, duration: f64, name: String) -> Self {
        let mut clip = Self {
            id,
            events,
            duration,
            name,
        };
        // Sort events by timestamp
        clip.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
        clip
    }

    /// Create an empty MIDI clip
    pub fn empty(id: MidiClipId, duration: f64, name: String) -> Self {
        Self {
            id,
            events: Vec::new(),
            duration,
            name,
        }
    }

    /// Add a MIDI event to the clip
    pub fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
        // Keep events sorted by timestamp
        self.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
    }

    /// Get events within a time range (relative to clip start)
    /// This is used by MidiClipInstance to fetch events for a given portion
    pub fn get_events_in_range(&self, start: f64, end: f64) -> Vec<MidiEvent> {
        self.events
            .iter()
            .filter(|e| e.timestamp >= start && e.timestamp < end)
            .copied()
            .collect()
    }
}

/// MIDI clip instance - a reference to MidiClip content with timeline positioning
///
/// ## Timing Model
/// - `internal_start` / `internal_end`: Define the region of the source clip to play (trimming)
/// - `external_start` / `external_duration`: Define where the instance appears on the timeline and how long
///
/// ## Looping
/// If `external_duration` is greater than `internal_end - internal_start`,
/// the instance will seamlessly loop back to `internal_start` when it reaches `internal_end`.
#[derive(Debug, Clone)]
pub struct MidiClipInstance {
    pub id: MidiClipInstanceId,
    pub clip_id: MidiClipId,  // Reference to MidiClip in pool

    /// Start position within the clip content (seconds)
    pub internal_start: f64,
    /// End position within the clip content (seconds)
    pub internal_end: f64,

    /// Start position on the timeline (seconds)
    pub external_start: f64,
    /// Duration on the timeline (seconds) - can be longer than internal duration for looping
    pub external_duration: f64,
}

impl MidiClipInstance {
    /// Create a new MIDI clip instance
    pub fn new(
        id: MidiClipInstanceId,
        clip_id: MidiClipId,
        internal_start: f64,
        internal_end: f64,
        external_start: f64,
        external_duration: f64,
    ) -> Self {
        Self {
            id,
            clip_id,
            internal_start,
            internal_end,
            external_start,
            external_duration,
        }
    }

    /// Create an instance that uses the full clip content (no trimming, no looping)
    pub fn from_full_clip(
        id: MidiClipInstanceId,
        clip_id: MidiClipId,
        clip_duration: f64,
        external_start: f64,
    ) -> Self {
        Self {
            id,
            clip_id,
            internal_start: 0.0,
            internal_end: clip_duration,
            external_start,
            external_duration: clip_duration,
        }
    }

    /// Get the internal (content) duration
    pub fn internal_duration(&self) -> f64 {
        self.internal_end - self.internal_start
    }

    /// Get the end time on the timeline
    pub fn external_end(&self) -> f64 {
        self.external_start + self.external_duration
    }

    /// Check if this instance loops
    pub fn is_looping(&self) -> bool {
        self.external_duration > self.internal_duration()
    }

    /// Get the end time on the timeline (for backwards compatibility)
    pub fn end_time(&self) -> f64 {
        self.external_end()
    }

    /// Get the start time on the timeline (for backwards compatibility)
    pub fn start_time(&self) -> f64 {
        self.external_start
    }

    /// Check if this instance overlaps with a time range
    pub fn overlaps_range(&self, range_start: f64, range_end: f64) -> bool {
        self.external_start < range_end && self.external_end() > range_start
    }

    /// Get events that should be triggered in a given timeline range
    ///
    /// This handles:
    /// - Trimming (internal_start/internal_end)
    /// - Looping (when external duration > internal duration)
    /// - Time mapping from timeline to clip content
    ///
    /// Returns events with timestamps adjusted to timeline time (not clip-relative)
    pub fn get_events_in_range(
        &self,
        clip: &MidiClip,
        range_start_seconds: f64,
        range_end_seconds: f64,
    ) -> Vec<MidiEvent> {
        let mut result = Vec::new();

        // Check if instance overlaps with the range
        if !self.overlaps_range(range_start_seconds, range_end_seconds) {
            return result;
        }

        let internal_duration = self.internal_duration();
        if internal_duration <= 0.0 {
            return result;
        }

        // Calculate how many complete loops fit in the external duration
        let num_loops = if self.external_duration > internal_duration {
            (self.external_duration / internal_duration).ceil() as usize
        } else {
            1
        };

        let external_end = self.external_end();

        for loop_idx in 0..num_loops {
            let loop_offset = loop_idx as f64 * internal_duration;

            // Get events from the clip that fall within the internal range
            for event in &clip.events {
                // Skip events outside the trimmed region
                if event.timestamp < self.internal_start || event.timestamp >= self.internal_end {
                    continue;
                }

                // Convert to timeline time
                let relative_content_time = event.timestamp - self.internal_start;
                let timeline_time = self.external_start + loop_offset + relative_content_time;

                // Check if within current buffer range and instance bounds
                if timeline_time >= range_start_seconds
                    && timeline_time < range_end_seconds
                    && timeline_time < external_end
                {
                    let mut adjusted_event = *event;
                    adjusted_event.timestamp = timeline_time;
                    result.push(adjusted_event);
                }
            }
        }

        result
    }
}
