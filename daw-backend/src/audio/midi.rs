use crate::time::Beats;

/// MIDI event representing a single MIDI message
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MidiEvent {
    /// Time position in beats (quarter-note beats)
    pub timestamp: Beats,
    /// MIDI status byte (includes channel)
    pub status: u8,
    /// First data byte (note number, CC number, etc.)
    pub data1: u8,
    /// Second data byte (velocity, CC value, etc.)
    pub data2: u8,
}

impl MidiEvent {
    pub fn new(timestamp: Beats, status: u8, data1: u8, data2: u8) -> Self {
        Self { timestamp, status, data1, data2 }
    }

    pub fn note_on(timestamp: Beats, channel: u8, note: u8, velocity: u8) -> Self {
        Self { timestamp, status: 0x90 | (channel & 0x0F), data1: note, data2: velocity }
    }

    pub fn note_off(timestamp: Beats, channel: u8, note: u8, velocity: u8) -> Self {
        Self { timestamp, status: 0x80 | (channel & 0x0F), data1: note, data2: velocity }
    }

    pub fn is_note_on(&self) -> bool {
        (self.status & 0xF0) == 0x90 && self.data2 > 0
    }

    pub fn is_note_off(&self) -> bool {
        (self.status & 0xF0) == 0x80 || ((self.status & 0xF0) == 0x90 && self.data2 == 0)
    }

    pub fn channel(&self) -> u8 { self.status & 0x0F }
    pub fn message_type(&self) -> u8 { self.status & 0xF0 }
}

/// MIDI clip ID type (for clips stored in the pool)
pub type MidiClipId = u32;

/// MIDI clip instance ID type (for instances placed on tracks)
pub type MidiClipInstanceId = u32;

/// MIDI clip content — stores the actual MIDI events.
/// `duration` is in beats.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiClip {
    pub id: MidiClipId,
    pub events: Vec<MidiEvent>,
    /// Total content duration in beats
    pub duration: Beats,
    pub name: String,
}

impl MidiClip {
    pub fn new(id: MidiClipId, events: Vec<MidiEvent>, duration: Beats, name: String) -> Self {
        let mut clip = Self { id, events, duration, name };
        clip.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
        clip
    }

    pub fn empty(id: MidiClipId, duration: Beats, name: String) -> Self {
        Self { id, events: Vec::new(), duration, name }
    }

    pub fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
        self.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
    }

    /// Get events within a beat range (relative to clip start)
    pub fn get_events_in_range(&self, start: Beats, end: Beats) -> Vec<MidiEvent> {
        self.events.iter()
            .filter(|e| e.timestamp >= start && e.timestamp < end)
            .copied()
            .collect()
    }
}

/// MIDI clip instance — a reference to MidiClip content with timeline positioning.
///
/// All timing fields are in beats.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiClipInstance {
    pub id: MidiClipInstanceId,
    pub clip_id: MidiClipId,

    /// Start of the trimmed region within the clip content (beats)
    pub internal_start: Beats,
    /// End of the trimmed region within the clip content (beats)
    pub internal_end: Beats,

    /// Start position on the timeline (beats)
    pub external_start: Beats,
    /// Duration on the timeline (beats); > internal duration = looping
    pub external_duration: Beats,
}

impl MidiClipInstance {
    pub fn new(
        id: MidiClipInstanceId,
        clip_id: MidiClipId,
        internal_start: Beats,
        internal_end: Beats,
        external_start: Beats,
        external_duration: Beats,
    ) -> Self {
        Self { id, clip_id, internal_start, internal_end, external_start, external_duration }
    }

    /// Create an instance covering the full clip with no trim
    pub fn from_full_clip(
        id: MidiClipInstanceId,
        clip_id: MidiClipId,
        clip_duration: Beats,
        external_start: Beats,
    ) -> Self {
        Self {
            id,
            clip_id,
            internal_start: Beats::ZERO,
            internal_end: clip_duration,
            external_start,
            external_duration: clip_duration,
        }
    }

    pub fn internal_duration(&self) -> Beats { self.internal_end - self.internal_start }
    pub fn external_end(&self) -> Beats { self.external_start + self.external_duration }
    pub fn is_looping(&self) -> bool { self.external_duration > self.internal_duration() }

    /// Check if this instance overlaps with a beat range
    pub fn overlaps_range(&self, range_start: Beats, range_end: Beats) -> bool {
        self.external_start < range_end && self.external_end() > range_start
    }

    /// Get events that should fire in a given beat range on the timeline.
    /// Returns events with `timestamp` set to their global timeline beat position.
    pub fn get_events_in_range(
        &self,
        clip: &MidiClip,
        range_start: Beats,
        range_end: Beats,
    ) -> Vec<MidiEvent> {
        let mut result = Vec::new();

        if !self.overlaps_range(range_start, range_end) {
            return result;
        }

        let internal_duration = self.internal_duration();
        if internal_duration <= Beats::ZERO {
            return result;
        }

        let num_loops = if self.external_duration > internal_duration {
            (self.external_duration / internal_duration).ceil() as usize
        } else {
            1
        };

        let external_end = self.external_end();

        for loop_idx in 0..num_loops {
            let loop_offset = internal_duration * loop_idx as f64;

            for event in &clip.events {
                if event.timestamp < self.internal_start || event.timestamp > self.internal_end {
                    continue;
                }

                let relative_content_time = event.timestamp - self.internal_start;
                let timeline_time = self.external_start + loop_offset + relative_content_time;

                if timeline_time >= range_start
                    && timeline_time < range_end
                    && timeline_time <= external_end
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
