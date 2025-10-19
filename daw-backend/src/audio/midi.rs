/// MIDI event representing a single MIDI message
#[derive(Debug, Clone, Copy)]
pub struct MidiEvent {
    /// Sample position within the clip
    pub timestamp: u64,
    /// MIDI status byte (includes channel)
    pub status: u8,
    /// First data byte (note number, CC number, etc.)
    pub data1: u8,
    /// Second data byte (velocity, CC value, etc.)
    pub data2: u8,
}

impl MidiEvent {
    /// Create a new MIDI event
    pub fn new(timestamp: u64, status: u8, data1: u8, data2: u8) -> Self {
        Self {
            timestamp,
            status,
            data1,
            data2,
        }
    }

    /// Create a note on event
    pub fn note_on(timestamp: u64, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            timestamp,
            status: 0x90 | (channel & 0x0F),
            data1: note,
            data2: velocity,
        }
    }

    /// Create a note off event
    pub fn note_off(timestamp: u64, channel: u8, note: u8, velocity: u8) -> Self {
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
        // Keep events sorted by timestamp
        self.events.sort_by_key(|e| e.timestamp);
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
        sample_rate: u32,
    ) -> Vec<(u64, MidiEvent)> {
        let mut result = Vec::new();

        // Check if clip overlaps with the range
        if range_start_seconds >= self.end_time() || range_end_seconds <= self.start_time {
            return result;
        }

        // Calculate the intersection
        let play_start = range_start_seconds.max(self.start_time);
        let play_end = range_end_seconds.min(self.end_time());

        // Convert to samples
        let range_start_samples = (range_start_seconds * sample_rate as f64) as u64;

        // Position within the clip
        let clip_position_seconds = play_start - self.start_time;
        let clip_position_samples = (clip_position_seconds * sample_rate as f64) as u64;
        let clip_end_samples = ((play_end - self.start_time) * sample_rate as f64) as u64;

        // Find events in this range
        // Note: Using <= for the end boundary to include events exactly at the clip end
        for event in &self.events {
            if event.timestamp >= clip_position_samples && event.timestamp <= clip_end_samples {
                // Calculate absolute timestamp in the output buffer
                let absolute_timestamp = range_start_samples + (event.timestamp - clip_position_samples);
                result.push((absolute_timestamp, *event));
            }
        }

        result
    }
}
