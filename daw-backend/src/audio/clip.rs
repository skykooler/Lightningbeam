use std::sync::Arc;
use serde::{Serialize, Deserialize};
use crate::time::{Beats, Seconds};
use crate::tempo_map::TempoMap;

/// Audio clip instance ID type
pub type AudioClipInstanceId = u32;

/// Type alias for backwards compatibility
pub type ClipId = AudioClipInstanceId;

/// Audio clip instance that references content in the AudioClipPool
///
/// ## Timing Model
/// - `internal_start` / `internal_end`: Region of the source audio to play (seconds — audio file seek positions)
/// - `external_start` / `external_duration`: Where the clip appears on the timeline (**beats**)
///
/// ## Looping
/// If `external_duration_secs(bpm)` > `internal_end - internal_start`, the clip loops.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioClipInstance {
    pub id: AudioClipInstanceId,
    pub audio_pool_index: usize,

    /// Start position within the audio content
    pub internal_start: Seconds,
    /// End position within the audio content
    pub internal_end: Seconds,

    /// Start position on the timeline
    pub external_start: Beats,
    /// Duration on the timeline
    pub external_duration: Beats,

    /// Clip-level gain
    pub gain: f32,

    /// Per-instance read-ahead buffer for compressed audio streaming.
    #[serde(skip)]
    pub read_ahead: Option<Arc<super::disk_reader::ReadAheadBuffer>>,
}

/// Type alias for backwards compatibility
pub type Clip = AudioClipInstance;

impl AudioClipInstance {
    pub fn new(
        id: AudioClipInstanceId,
        audio_pool_index: usize,
        internal_start: Seconds,
        internal_end: Seconds,
        external_start: Beats,
        external_duration: Beats,
    ) -> Self {
        Self {
            id,
            audio_pool_index,
            internal_start,
            internal_end,
            external_start,
            external_duration,
            gain: 1.0,
            read_ahead: None,
        }
    }

    pub fn external_end(&self) -> Beats {
        self.external_start + self.external_duration
    }

    pub fn external_start_secs(&self, tempo_map: &TempoMap) -> Seconds {
        tempo_map.beats_to_seconds(self.external_start)
    }

    pub fn external_end_secs(&self, tempo_map: &TempoMap) -> Seconds {
        tempo_map.beats_to_seconds(self.external_end())
    }

    pub fn external_duration_secs(&self, tempo_map: &TempoMap) -> Seconds {
        tempo_map.beats_to_seconds(self.external_end()) - tempo_map.beats_to_seconds(self.external_start)
    }

    pub fn is_active_at(&self, time: Seconds, tempo_map: &TempoMap) -> bool {
        time >= self.external_start_secs(tempo_map) && time < self.external_end_secs(tempo_map)
    }

    pub fn internal_duration(&self) -> Seconds {
        self.internal_end - self.internal_start
    }

    pub fn is_looping(&self, tempo_map: &TempoMap) -> bool {
        self.external_duration_secs(tempo_map) > self.internal_duration()
    }

    /// Get the audio content position for a given timeline position. Handles looping.
    pub fn get_content_position(&self, timeline_pos: Seconds, tempo_map: &TempoMap) -> Option<Seconds> {
        let start_secs = self.external_start_secs(tempo_map);
        let end_secs = self.external_end_secs(tempo_map);
        if timeline_pos < start_secs || timeline_pos >= end_secs {
            return None;
        }
        let relative_pos = timeline_pos - start_secs;
        let internal_duration = self.internal_duration();
        if internal_duration.0 <= 0.0 {
            return None;
        }
        let content_offset = relative_pos % internal_duration;
        Some(self.internal_start + content_offset)
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.max(0.0);
    }
}
