use crate::audio::{
    AutomationLaneId, ClipId, CurveType, MidiClip, MidiClipId, ParameterId,
    TrackId,
};
use crate::audio::buffer_pool::BufferPoolStats;
use crate::io::WaveformPeak;

/// Commands sent from UI/control thread to audio thread
#[derive(Debug, Clone)]
pub enum Command {
    // Transport commands
    /// Start playback
    Play,
    /// Stop playback and reset to beginning
    Stop,
    /// Pause playback (maintains position)
    Pause,
    /// Seek to a specific position in seconds
    Seek(f64),

    // Track management commands
    /// Set track volume (0.0 = silence, 1.0 = unity gain)
    SetTrackVolume(TrackId, f32),
    /// Set track mute state
    SetTrackMute(TrackId, bool),
    /// Set track solo state
    SetTrackSolo(TrackId, bool),

    // Clip management commands
    /// Move a clip to a new timeline position
    MoveClip(TrackId, ClipId, f64),

    // Effect management commands
    /// Add or update gain effect on track (gain in dB)
    AddGainEffect(TrackId, f32),
    /// Add or update pan effect on track (-1.0 = left, 0.0 = center, 1.0 = right)
    AddPanEffect(TrackId, f32),
    /// Add or update EQ effect on track (low_db, mid_db, high_db)
    AddEQEffect(TrackId, f32, f32, f32),
    /// Clear all effects from a track
    ClearEffects(TrackId),

    // Metatrack management commands
    /// Create a new metatrack with a name
    CreateMetatrack(String),
    /// Add a track to a metatrack (track_id, metatrack_id)
    AddToMetatrack(TrackId, TrackId),
    /// Remove a track from its parent metatrack
    RemoveFromMetatrack(TrackId),

    // Metatrack transformation commands
    /// Set metatrack time stretch factor (track_id, stretch_factor)
    /// 0.5 = half speed, 1.0 = normal, 2.0 = double speed
    SetTimeStretch(TrackId, f32),
    /// Set metatrack time offset in seconds (track_id, offset)
    /// Positive = shift content later, negative = shift earlier
    SetOffset(TrackId, f64),
    /// Set metatrack pitch shift in semitones (track_id, semitones) - for future use
    SetPitchShift(TrackId, f32),

    // Audio track commands
    /// Create a new audio track with a name
    CreateAudioTrack(String),
    /// Add an audio file to the pool (path, data, channels, sample_rate)
    /// Returns the pool index via an AudioEvent
    AddAudioFile(String, Vec<f32>, u32, u32),
    /// Add a clip to an audio track (track_id, pool_index, start_time, duration, offset)
    AddAudioClip(TrackId, usize, f64, f64, f64),

    // MIDI commands
    /// Create a new MIDI track with a name
    CreateMidiTrack(String),
    /// Create a new MIDI clip on a track (track_id, start_time, duration)
    CreateMidiClip(TrackId, f64, f64),
    /// Add a MIDI note to a clip (track_id, clip_id, time_offset, note, velocity, duration)
    AddMidiNote(TrackId, MidiClipId, f64, u8, u8, f64),
    /// Add a pre-loaded MIDI clip to a track
    AddLoadedMidiClip(TrackId, MidiClip),
    /// Update MIDI clip notes (track_id, clip_id, notes: Vec<(start_time, note, velocity, duration)>)
    /// NOTE: May need to switch to individual note operations if this becomes slow on clips with many notes
    UpdateMidiClipNotes(TrackId, MidiClipId, Vec<(f64, u8, u8, f64)>),

    // Diagnostics commands
    /// Request buffer pool statistics
    RequestBufferPoolStats,

    // Automation commands
    /// Create a new automation lane on a track (track_id, parameter_id)
    CreateAutomationLane(TrackId, ParameterId),
    /// Add an automation point to a lane (track_id, lane_id, time, value, curve)
    AddAutomationPoint(TrackId, AutomationLaneId, f64, f32, CurveType),
    /// Remove an automation point at a specific time (track_id, lane_id, time, tolerance)
    RemoveAutomationPoint(TrackId, AutomationLaneId, f64, f64),
    /// Clear all automation points from a lane (track_id, lane_id)
    ClearAutomationLane(TrackId, AutomationLaneId),
    /// Remove an automation lane (track_id, lane_id)
    RemoveAutomationLane(TrackId, AutomationLaneId),
    /// Enable/disable an automation lane (track_id, lane_id, enabled)
    SetAutomationLaneEnabled(TrackId, AutomationLaneId, bool),

    // Recording commands
    /// Start recording on a track (track_id, start_time)
    StartRecording(TrackId, f64),
    /// Stop the current recording
    StopRecording,
    /// Pause the current recording
    PauseRecording,
    /// Resume the current recording
    ResumeRecording,

    // Project commands
    /// Reset the entire project (remove all tracks, clear audio pool, reset state)
    Reset,

    // Live MIDI input commands
    /// Send a live MIDI note on event to a track's instrument (track_id, note, velocity)
    SendMidiNoteOn(TrackId, u8, u8),
    /// Send a live MIDI note off event to a track's instrument (track_id, note)
    SendMidiNoteOff(TrackId, u8),
}

/// Events sent from audio thread back to UI/control thread
#[derive(Debug, Clone)]
pub enum AudioEvent {
    /// Current playback position in seconds
    PlaybackPosition(f64),
    /// Playback has stopped (reached end of audio)
    PlaybackStopped,
    /// Audio buffer underrun detected
    BufferUnderrun,
    /// A new track was created (track_id, is_metatrack, name)
    TrackCreated(TrackId, bool, String),
    /// An audio file was added to the pool (pool_index, path)
    AudioFileAdded(usize, String),
    /// A clip was added to a track (track_id, clip_id)
    ClipAdded(TrackId, ClipId),
    /// Buffer pool statistics response
    BufferPoolStats(BufferPoolStats),
    /// Automation lane created (track_id, lane_id, parameter_id)
    AutomationLaneCreated(TrackId, AutomationLaneId, ParameterId),
    /// Recording started (track_id, clip_id)
    RecordingStarted(TrackId, ClipId),
    /// Recording progress update (clip_id, current_duration)
    RecordingProgress(ClipId, f64),
    /// Recording stopped (clip_id, pool_index, waveform)
    RecordingStopped(ClipId, usize, Vec<WaveformPeak>),
    /// Recording error (error_message)
    RecordingError(String),
    /// Project has been reset
    ProjectReset,
    /// MIDI note started playing (note, velocity)
    NoteOn(u8, u8),
    /// MIDI note stopped playing (note)
    NoteOff(u8),
}
