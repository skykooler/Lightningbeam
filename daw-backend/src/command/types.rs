use crate::audio::{ClipId, MidiClip, MidiClipId, TrackId};

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

    // Group management commands
    /// Create a new group track with a name
    CreateGroup(String),
    /// Add a track to a group (track_id, group_id)
    AddToGroup(TrackId, TrackId),
    /// Remove a track from its parent group
    RemoveFromGroup(TrackId),

    // MIDI commands
    /// Create a new MIDI track with a name
    CreateMidiTrack(String),
    /// Create a new MIDI clip on a track (track_id, start_time, duration)
    CreateMidiClip(TrackId, f64, f64),
    /// Add a MIDI note to a clip (track_id, clip_id, time_offset, note, velocity, duration)
    AddMidiNote(TrackId, MidiClipId, f64, u8, u8, f64),
    /// Add a pre-loaded MIDI clip to a track
    AddLoadedMidiClip(TrackId, MidiClip),
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
    /// A new track was created (track_id, is_group, name)
    TrackCreated(TrackId, bool, String),
}
