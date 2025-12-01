use crate::audio::{
    AudioClipInstanceId, AutomationLaneId, ClipId, CurveType, MidiClip, MidiClipId,
    MidiClipInstanceId, ParameterId, TrackId,
};
use crate::audio::buffer_pool::BufferPoolStats;
use crate::audio::node_graph::nodes::LoopMode;
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
    /// Move a clip to a new timeline position (track_id, clip_id, new_external_start)
    MoveClip(TrackId, ClipId, f64),
    /// Trim a clip's internal boundaries (track_id, clip_id, new_internal_start, new_internal_end)
    /// This changes which portion of the source content is used
    TrimClip(TrackId, ClipId, f64, f64),
    /// Extend/shrink a clip's external duration (track_id, clip_id, new_external_duration)
    /// If duration > internal duration, the clip will loop
    ExtendClip(TrackId, ClipId, f64),

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
    /// Add a MIDI clip to the pool without placing it on a track
    AddMidiClipToPool(MidiClip),
    /// Create a new MIDI clip on a track (track_id, start_time, duration)
    CreateMidiClip(TrackId, f64, f64),
    /// Add a MIDI note to a clip (track_id, clip_id, time_offset, note, velocity, duration)
    AddMidiNote(TrackId, MidiClipId, f64, u8, u8, f64),
    /// Add a pre-loaded MIDI clip to a track (track_id, clip, start_time)
    AddLoadedMidiClip(TrackId, MidiClip, f64),
    /// Update MIDI clip notes (track_id, clip_id, notes: Vec<(start_time, note, velocity, duration)>)
    /// NOTE: May need to switch to individual note operations if this becomes slow on clips with many notes
    UpdateMidiClipNotes(TrackId, MidiClipId, Vec<(f64, u8, u8, f64)>),
    /// Remove a MIDI clip instance from a track (track_id, instance_id) - for undo/redo support
    RemoveMidiClip(TrackId, MidiClipInstanceId),
    /// Remove an audio clip instance from a track (track_id, instance_id) - for undo/redo support
    RemoveAudioClip(TrackId, AudioClipInstanceId),

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

    // MIDI Recording commands
    /// Start MIDI recording on a track (track_id, clip_id, start_time)
    StartMidiRecording(TrackId, MidiClipId, f64),
    /// Stop the current MIDI recording
    StopMidiRecording,

    // Project commands
    /// Reset the entire project (remove all tracks, clear audio pool, reset state)
    Reset,

    // Live MIDI input commands
    /// Send a live MIDI note on event to a track's instrument (track_id, note, velocity)
    SendMidiNoteOn(TrackId, u8, u8),
    /// Send a live MIDI note off event to a track's instrument (track_id, note)
    SendMidiNoteOff(TrackId, u8),
    /// Set the active MIDI track for external MIDI input routing (track_id or None)
    SetActiveMidiTrack(Option<TrackId>),

    // Metronome command
    /// Enable or disable the metronome click track
    SetMetronomeEnabled(bool),

    // Node graph commands
    /// Add a node to a track's instrument graph (track_id, node_type, position_x, position_y)
    GraphAddNode(TrackId, String, f32, f32),
    /// Add a node to a VoiceAllocator's template graph (track_id, voice_allocator_node_id, node_type, position_x, position_y)
    GraphAddNodeToTemplate(TrackId, u32, String, f32, f32),
    /// Remove a node from a track's instrument graph (track_id, node_index)
    GraphRemoveNode(TrackId, u32),
    /// Connect two nodes in a track's graph (track_id, from_node, from_port, to_node, to_port)
    GraphConnect(TrackId, u32, usize, u32, usize),
    /// Connect nodes in a VoiceAllocator template (track_id, voice_allocator_node_id, from_node, from_port, to_node, to_port)
    GraphConnectInTemplate(TrackId, u32, u32, usize, u32, usize),
    /// Disconnect two nodes in a track's graph (track_id, from_node, from_port, to_node, to_port)
    GraphDisconnect(TrackId, u32, usize, u32, usize),
    /// Set a parameter on a node (track_id, node_index, param_id, value)
    GraphSetParameter(TrackId, u32, u32, f32),
    /// Set which node receives MIDI events (track_id, node_index, enabled)
    GraphSetMidiTarget(TrackId, u32, bool),
    /// Set which node is the audio output (track_id, node_index)
    GraphSetOutputNode(TrackId, u32),

    /// Save current graph as a preset (track_id, preset_path, preset_name, description, tags)
    GraphSavePreset(TrackId, String, String, String, Vec<String>),
    /// Load a preset into a track's graph (track_id, preset_path)
    GraphLoadPreset(TrackId, String),
    /// Save a VoiceAllocator's template graph as a preset (track_id, voice_allocator_id, preset_path, preset_name)
    GraphSaveTemplatePreset(TrackId, u32, String, String),

    /// Load a sample into a SimpleSampler node (track_id, node_id, file_path)
    SamplerLoadSample(TrackId, u32, String),
    /// Add a sample layer to a MultiSampler node (track_id, node_id, file_path, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode)
    MultiSamplerAddLayer(TrackId, u32, String, u8, u8, u8, u8, u8, Option<usize>, Option<usize>, LoopMode),
    /// Update a MultiSampler layer's configuration (track_id, node_id, layer_index, key_min, key_max, root_key, velocity_min, velocity_max, loop_start, loop_end, loop_mode)
    MultiSamplerUpdateLayer(TrackId, u32, usize, u8, u8, u8, u8, u8, Option<usize>, Option<usize>, LoopMode),
    /// Remove a layer from a MultiSampler node (track_id, node_id, layer_index)
    MultiSamplerRemoveLayer(TrackId, u32, usize),

    // Automation Input Node commands
    /// Add or update a keyframe on an AutomationInput node (track_id, node_id, time, value, interpolation, ease_out, ease_in)
    AutomationAddKeyframe(TrackId, u32, f64, f32, String, (f32, f32), (f32, f32)),
    /// Remove a keyframe from an AutomationInput node (track_id, node_id, time)
    AutomationRemoveKeyframe(TrackId, u32, f64),
    /// Set the display name of an AutomationInput node (track_id, node_id, name)
    AutomationSetName(TrackId, u32, String),
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
    /// MIDI recording stopped (track_id, clip_id, note_count)
    MidiRecordingStopped(TrackId, MidiClipId, usize),
    /// MIDI recording progress (track_id, clip_id, duration, notes)
    /// Notes format: (start_time, note, velocity, duration)
    MidiRecordingProgress(TrackId, MidiClipId, f64, Vec<(f64, u8, u8, f64)>),
    /// Project has been reset
    ProjectReset,
    /// MIDI note started playing (note, velocity)
    NoteOn(u8, u8),
    /// MIDI note stopped playing (note)
    NoteOff(u8),

    // Node graph events
    /// Node added to graph (track_id, node_index, node_type)
    GraphNodeAdded(TrackId, u32, String),
    /// Connection error occurred (track_id, error_message)
    GraphConnectionError(TrackId, String),
    /// Graph state changed (for full UI sync)
    GraphStateChanged(TrackId),
    /// Preset fully loaded (track_id) - emitted after all nodes and samples are loaded
    GraphPresetLoaded(TrackId),
    /// Preset has been saved to file (track_id, preset_path)
    GraphPresetSaved(TrackId, String),
}

/// Synchronous queries sent from UI thread to audio thread
#[derive(Debug)]
pub enum Query {
    /// Get the current graph state as JSON (track_id)
    GetGraphState(TrackId),
    /// Get a voice allocator's template graph state as JSON (track_id, voice_allocator_id)
    GetTemplateState(TrackId, u32),
    /// Get oscilloscope data from a node (track_id, node_id, sample_count)
    GetOscilloscopeData(TrackId, u32, usize),
    /// Get MIDI clip data (track_id, clip_id)
    GetMidiClip(TrackId, MidiClipId),
    /// Get keyframes from an AutomationInput node (track_id, node_id)
    GetAutomationKeyframes(TrackId, u32),
    /// Get the display name of an AutomationInput node (track_id, node_id)
    GetAutomationName(TrackId, u32),
    /// Serialize audio pool for project saving (project_path)
    SerializeAudioPool(std::path::PathBuf),
    /// Load audio pool from serialized entries (entries, project_path)
    LoadAudioPool(Vec<crate::audio::pool::AudioPoolEntry>, std::path::PathBuf),
    /// Resolve a missing audio file (pool_index, new_path)
    ResolveMissingAudioFile(usize, std::path::PathBuf),
    /// Serialize a track's effects/instrument graph (track_id, project_path)
    SerializeTrackGraph(TrackId, std::path::PathBuf),
    /// Load a track's effects/instrument graph (track_id, preset_json, project_path)
    LoadTrackGraph(TrackId, String, std::path::PathBuf),
    /// Create a new audio track (name) - returns track ID synchronously
    CreateAudioTrackSync(String),
    /// Create a new MIDI track (name) - returns track ID synchronously
    CreateMidiTrackSync(String),
    /// Get waveform data from audio pool (pool_index, target_peaks)
    GetPoolWaveform(usize, usize),
    /// Get file info from audio pool (pool_index) - returns (duration, sample_rate, channels)
    GetPoolFileInfo(usize),
    /// Export audio to file (settings, output_path)
    ExportAudio(crate::audio::ExportSettings, std::path::PathBuf),
    /// Add a MIDI clip to a track synchronously (track_id, clip, start_time) - returns instance ID
    AddMidiClipSync(TrackId, crate::audio::midi::MidiClip, f64),
    /// Add a MIDI clip instance to a track synchronously (track_id, instance) - returns instance ID
    /// The clip must already exist in the MidiClipPool
    AddMidiClipInstanceSync(TrackId, crate::audio::midi::MidiClipInstance),
    /// Add an audio clip to a track synchronously (track_id, pool_index, start_time, duration, offset) - returns instance ID
    AddAudioClipSync(TrackId, usize, f64, f64, f64),
    /// Get a clone of the current project for serialization
    GetProject,
    /// Set the project (replaces current project state)
    SetProject(Box<crate::audio::project::Project>),
}

/// Oscilloscope data from a node
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OscilloscopeData {
    /// Audio samples
    pub audio: Vec<f32>,
    /// CV samples (may be empty if no CV input)
    pub cv: Vec<f32>,
}

/// MIDI clip data for serialization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MidiClipData {
    pub duration: f64,
    pub events: Vec<crate::audio::midi::MidiEvent>,
}

/// Automation keyframe data for serialization
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AutomationKeyframeData {
    pub time: f64,
    pub value: f32,
    pub interpolation: String,
    pub ease_out: (f32, f32),
    pub ease_in: (f32, f32),
}

/// Responses to synchronous queries
#[derive(Debug)]
pub enum QueryResponse {
    /// Graph state as JSON string
    GraphState(Result<String, String>),
    /// Oscilloscope data samples
    OscilloscopeData(Result<OscilloscopeData, String>),
    /// MIDI clip data
    MidiClipData(Result<MidiClipData, String>),
    /// Automation keyframes
    AutomationKeyframes(Result<Vec<AutomationKeyframeData>, String>),
    /// Automation node name
    AutomationName(Result<String, String>),
    /// Serialized audio pool entries
    AudioPoolSerialized(Result<Vec<crate::audio::pool::AudioPoolEntry>, String>),
    /// Audio pool loaded (returns list of missing pool indices)
    AudioPoolLoaded(Result<Vec<usize>, String>),
    /// Audio file resolved
    AudioFileResolved(Result<(), String>),
    /// Track graph serialized as JSON
    TrackGraphSerialized(Result<String, String>),
    /// Track graph loaded
    TrackGraphLoaded(Result<(), String>),
    /// Track created (returns track ID)
    TrackCreated(Result<TrackId, String>),
    /// Pool waveform data
    PoolWaveform(Result<Vec<crate::io::WaveformPeak>, String>),
    /// Pool file info (duration, sample_rate, channels)
    PoolFileInfo(Result<(f64, u32, u32), String>),
    /// Audio exported
    AudioExported(Result<(), String>),
    /// MIDI clip instance added (returns instance ID)
    MidiClipInstanceAdded(Result<MidiClipInstanceId, String>),
    /// Audio clip instance added (returns instance ID)
    AudioClipInstanceAdded(Result<AudioClipInstanceId, String>),
    /// Project retrieved
    ProjectRetrieved(Result<Box<crate::audio::project::Project>, String>),
    /// Project set
    ProjectSet(Result<(), String>),
}
