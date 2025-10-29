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

/// MIDI clip ID type
pub type MidiClipId = u32;

/// MIDI clip containing a sequence of MIDI events
#[derive(Debug, Clone)]
pub struct MidiClip {
    pub id: MidiClipId,
    pub events: Vec<MidiEvent>,
    pub start_time: f64,  // Position on timeline in seconds
    pub duration: f64,    // Clip duration in seconds
    pub loop_enabled: bool,
}

impl MidiClip {
    /// Create a new MIDI clip
    pub fn new(id: MidiClipId, start_time: f64, duration: f64) -> Self {
        Self {
            id,
            events: Vec::new(),
            start_time,
            duration,
            loop_enabled: false,
        }
    }

    /// Add a MIDI event to the clip
    pub fn add_event(&mut self, event: MidiEvent) {
        self.events.push(event);
        // Keep events sorted by timestamp (using partial_cmp for f64)
        self.events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
    }

    /// Get the end time of the clip
    pub fn end_time(&self) -> f64 {
        self.start_time + self.duration
    }

    /// Get events that should be triggered in a given time range
    ///
    /// Returns events along with their absolute timestamps in samples
    pub fn get_events_in_range(
        &self,
        range_start_seconds: f64,
        range_end_seconds: f64,
        _sample_rate: u32,
    ) -> Vec<MidiEvent> {
        let mut result = Vec::new();

        // Check if clip overlaps with the range
        if range_start_seconds >= self.end_time() || range_end_seconds <= self.start_time {
            return result;
        }

        // Calculate the intersection
        let play_start = range_start_seconds.max(self.start_time);
        let play_end = range_end_seconds.min(self.end_time());

        // Position within the clip
        let clip_position_seconds = play_start - self.start_time;
        let clip_end_seconds = play_end - self.start_time;

        // Find events in this range
        // Note: event.timestamp is now in seconds relative to clip start
        // Use half-open interval [start, end) to avoid triggering events twice
        for event in &self.events {
            if event.timestamp >= clip_position_seconds && event.timestamp < clip_end_seconds {
                result.push(*event);
            }
        }

        result
    }
}
