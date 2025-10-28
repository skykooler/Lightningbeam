use daw_backend::{AudioEvent, AudioSystem, EngineController, EventEmitter, WaveformPeak};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tauri::{Emitter, Manager};

#[derive(serde::Serialize)]
pub struct AudioFileMetadata {
    pub pool_index: usize,
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub waveform: Vec<WaveformPeak>,
}

#[derive(serde::Serialize)]
pub struct MidiNote {
    pub note: u8,           // MIDI note number (0-127)
    pub start_time: f64,    // Start time in seconds
    pub duration: f64,      // Note duration in seconds
    pub velocity: u8,       // Note velocity (0-127)
}

#[derive(serde::Serialize)]
pub struct MidiFileMetadata {
    pub duration: f64,
    pub notes: Vec<MidiNote>,
}

pub struct AudioState {
    controller: Option<EngineController>,
    sample_rate: u32,
    channels: u32,
    next_track_id: u32,
    next_pool_index: usize,
    next_graph_node_id: u32,
    // Track next node ID for each VoiceAllocator template (VoiceAllocator backend ID -> next template node ID)
    template_node_counters: HashMap<u32, u32>,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            controller: None,
            sample_rate: 0,
            channels: 0,
            next_track_id: 0,
            next_pool_index: 0,
            next_graph_node_id: 0,
            template_node_counters: HashMap::new(),
        }
    }
}

/// Implementation of EventEmitter that uses Tauri's event system
struct TauriEventEmitter {
    app_handle: tauri::AppHandle,
}

impl EventEmitter for TauriEventEmitter {
    fn emit(&self, event: AudioEvent) {
        // Serialize the event to the format expected by the frontend
        let serialized_event = match event {
            AudioEvent::PlaybackPosition(time) => {
                SerializedAudioEvent::PlaybackPosition { time }
            }
            AudioEvent::RecordingStarted(track_id, clip_id) => {
                SerializedAudioEvent::RecordingStarted { track_id, clip_id }
            }
            AudioEvent::RecordingProgress(clip_id, duration) => {
                SerializedAudioEvent::RecordingProgress { clip_id, duration }
            }
            AudioEvent::RecordingStopped(clip_id, pool_index, waveform) => {
                SerializedAudioEvent::RecordingStopped { clip_id, pool_index, waveform }
            }
            AudioEvent::RecordingError(message) => {
                SerializedAudioEvent::RecordingError { message }
            }
            AudioEvent::NoteOn(note, velocity) => {
                SerializedAudioEvent::NoteOn { note, velocity }
            }
            AudioEvent::NoteOff(note) => {
                SerializedAudioEvent::NoteOff { note }
            }
            AudioEvent::GraphNodeAdded(track_id, node_id, node_type) => {
                SerializedAudioEvent::GraphNodeAdded { track_id, node_id, node_type }
            }
            AudioEvent::GraphConnectionError(track_id, message) => {
                SerializedAudioEvent::GraphConnectionError { track_id, message }
            }
            AudioEvent::GraphStateChanged(track_id) => {
                SerializedAudioEvent::GraphStateChanged { track_id }
            }
            AudioEvent::GraphPresetLoaded(track_id) => {
                SerializedAudioEvent::GraphPresetLoaded { track_id }
            }
            _ => return, // Ignore other event types for now
        };

        // Emit the event via Tauri
        if let Err(e) = self.app_handle.emit("audio-event", serialized_event) {
            eprintln!("Failed to emit audio event: {}", e);
        }
    }
}

#[tauri::command]
pub async fn audio_init(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    let mut audio_state = state.lock().unwrap();

    // Check if already initialized - if so, reset DAW state (for hot-reload)
    if let Some(controller) = &mut audio_state.controller {
        controller.reset();
        audio_state.next_track_id = 0;
        audio_state.next_pool_index = 0;
        audio_state.next_graph_node_id = 0;
        return Ok(format!(
            "Audio already initialized (DAW state reset): {} Hz, {} ch",
            audio_state.sample_rate, audio_state.channels
        ));
    }

    // Create TauriEventEmitter
    let emitter = Arc::new(TauriEventEmitter { app_handle });

    // AudioSystem handles all cpal initialization internally
    let system = AudioSystem::new(Some(emitter))?;

    let info = format!(
        "Audio initialized: {} Hz, {} ch",
        system.sample_rate, system.channels
    );

    // Leak the stream to keep it alive for the lifetime of the app
    // This is intentional - we want the audio stream to run until app closes
    Box::leak(Box::new(system.stream));

    audio_state.controller = Some(system.controller);
    audio_state.sample_rate = system.sample_rate;
    audio_state.channels = system.channels;
    audio_state.next_track_id = 0;
    audio_state.next_pool_index = 0;
    audio_state.next_graph_node_id = 0;

    Ok(info)
}

#[tauri::command]
pub async fn audio_play(state: tauri::State<'_, Arc<Mutex<AudioState>>>) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.play();
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_stop(state: tauri::State<'_, Arc<Mutex<AudioState>>>) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.stop();
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_test_beep(state: tauri::State<'_, Arc<Mutex<AudioState>>>) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // Create MIDI track
        controller.create_midi_track("Test".to_string());

        // Note: Track ID will be 0 (first track created)
        // Create MIDI clip and add notes for a C major chord arpeggio
        controller.create_midi_clip(0, 0.0, 2.0);
        controller.add_midi_note(0, 0, 0.0, 60, 100, 0.5); // C
        controller.add_midi_note(0, 0, 0.5, 64, 100, 0.5); // E
        controller.add_midi_note(0, 0, 1.0, 67, 100, 0.5); // G

        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_seek(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    seconds: f64,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.seek(seconds);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_set_track_parameter(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    parameter: String,
    value: f32,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        match parameter.as_str() {
            "volume" => controller.set_track_volume(track_id, value),
            "mute" => controller.set_track_mute(track_id, value > 0.5),
            "solo" => controller.set_track_solo(track_id, value > 0.5),
            "pan" => {
                // Pan effect - would need to add this via effects system
                controller.add_pan_effect(track_id, value);
            }
            "gain_db" => {
                controller.add_gain_effect(track_id, value);
            }
            _ => return Err(format!("Unknown parameter: {}", parameter)),
        }
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_get_available_instruments() -> Result<Vec<String>, String> {
    // Return list of available instruments
    // For now, only SimpleSynth is available
    Ok(vec!["SimpleSynth".to_string()])
}

#[tauri::command]
pub async fn audio_create_track(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    name: String,
    track_type: String,
    instrument: Option<String>,
) -> Result<u32, String> {
    let mut audio_state = state.lock().unwrap();

    // Get track ID and increment counter before borrowing controller
    let track_id = audio_state.next_track_id;
    audio_state.next_track_id += 1;

    if let Some(controller) = &mut audio_state.controller {
        match track_type.as_str() {
            "audio" => controller.create_audio_track(name),
            "midi" => {
                // Validate instrument for MIDI tracks
                let inst = instrument.unwrap_or_else(|| "SimpleSynth".to_string());
                if inst != "SimpleSynth" {
                    return Err(format!("Unknown instrument: {}", inst));
                }
                controller.create_midi_track(name)
            },
            _ => return Err(format!("Unknown track type: {}", track_type)),
        }
        Ok(track_id)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_load_file(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    path: String,
) -> Result<AudioFileMetadata, String> {
    // Load the audio file from disk
    let audio_file = daw_backend::io::AudioFile::load(&path)?;

    // Calculate duration
    let duration = audio_file.duration();

    // Generate adaptive waveform peaks based on duration
    // Aim for ~300 peaks per second, with min 1000 and max 20000
    let target_peaks = ((duration * 300.0) as usize).clamp(1000, 20000);
    let waveform = audio_file.generate_waveform_overview(target_peaks);
    let sample_rate = audio_file.sample_rate;
    let channels = audio_file.channels;

    // Get a lock on the audio state and send the loaded data to the audio thread
    let mut audio_state = state.lock().unwrap();

    // Get pool index and increment counter before borrowing controller
    let pool_index = audio_state.next_pool_index;
    audio_state.next_pool_index += 1;

    if let Some(controller) = &mut audio_state.controller {
        controller.add_audio_file(
            path,
            audio_file.data,
            audio_file.channels,
            audio_file.sample_rate,
        );

        Ok(AudioFileMetadata {
            pool_index,
            duration,
            sample_rate,
            channels,
            waveform,
        })
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_add_clip(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    pool_index: usize,
    start_time: f64,
    duration: f64,
    offset: f64,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.add_audio_clip(track_id, pool_index, start_time, duration, offset);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_move_clip(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    clip_id: u32,
    new_start_time: f64,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.move_clip(track_id, clip_id, new_start_time);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_start_recording(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    start_time: f64,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.start_recording(track_id, start_time);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_stop_recording(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.stop_recording();
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_pause_recording(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.pause_recording();
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_resume_recording(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.resume_recording();
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_create_midi_clip(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    start_time: f64,
    duration: f64,
) -> Result<u32, String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.create_midi_clip(track_id, start_time, duration);
        // Return a clip ID (for now, just use 0 as clips are managed internally)
        Ok(0)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_add_midi_note(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    clip_id: u32,
    time_offset: f64,
    note: u8,
    velocity: u8,
    duration: f64,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.add_midi_note(track_id, clip_id, time_offset, note, velocity, duration);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_send_midi_note_on(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    note: u8,
    velocity: u8,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // For now, send to the first MIDI track (track_id 0)
        // TODO: Make this configurable to select which track to send to
        controller.send_midi_note_on(track_id, note, velocity);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_send_midi_note_off(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    note: u8,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.send_midi_note_off(track_id, note);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_load_midi_file(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    path: String,
    start_time: f64,
) -> Result<MidiFileMetadata, String> {
    let mut audio_state = state.lock().unwrap();

    // Extract sample_rate before the mutable borrow
    let sample_rate = audio_state.sample_rate;

    if let Some(controller) = &mut audio_state.controller {
        // Load and parse the MIDI file
        let mut clip = daw_backend::load_midi_file(&path, 0, sample_rate)?;

        // Set the start time
        clip.start_time = start_time;
        let duration = clip.duration;

        // Extract note data from MIDI events
        let mut notes = Vec::new();
        let mut active_notes: std::collections::HashMap<u8, (f64, u8)> = std::collections::HashMap::new();

        for event in &clip.events {
            let time_seconds = event.timestamp as f64 / sample_rate as f64;

            if event.is_note_on() {
                // Store note on event (time and velocity)
                active_notes.insert(event.data1, (time_seconds, event.data2));
            } else if event.is_note_off() {
                // Find matching note on and create a MidiNote
                if let Some((start, velocity)) = active_notes.remove(&event.data1) {
                    notes.push(MidiNote {
                        note: event.data1,
                        start_time: start,
                        duration: time_seconds - start,
                        velocity,
                    });
                }
            }
        }

        // Add the loaded MIDI clip to the track
        controller.add_loaded_midi_clip(track_id, clip);

        Ok(MidiFileMetadata {
            duration,
            notes,
        })
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn audio_update_midi_clip_notes(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    clip_id: u32,
    notes: Vec<(f64, u8, u8, f64)>,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();

    if let Some(controller) = &mut audio_state.controller {
        controller.update_midi_clip_notes(track_id, clip_id, notes);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

// Node graph commands

#[tauri::command]
pub async fn graph_add_node(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_type: String,
    x: f32,
    y: f32,
) -> Result<u32, String> {
    let mut audio_state = state.lock().unwrap();

    // Get the next node ID before adding (nodes are added sequentially)
    let node_id = audio_state.next_graph_node_id;
    audio_state.next_graph_node_id += 1;

    if let Some(controller) = &mut audio_state.controller {
        controller.graph_add_node(track_id, node_type, x, y);
        Ok(node_id)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_add_node_to_template(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    voice_allocator_id: u32,
    node_type: String,
    x: f32,
    y: f32,
) -> Result<u32, String> {
    let mut audio_state = state.lock().unwrap();

    // Get template-local node ID for this VoiceAllocator
    let node_id = audio_state.template_node_counters
        .entry(voice_allocator_id)
        .or_insert(0);
    let template_node_id = *node_id;
    *node_id += 1;

    if let Some(controller) = &mut audio_state.controller {
        controller.graph_add_node_to_template(track_id, voice_allocator_id, node_type, x, y);
        Ok(template_node_id)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_remove_node(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_remove_node(track_id, node_id);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_connect(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    from_node: u32,
    from_port: usize,
    to_node: u32,
    to_port: usize,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_connect(track_id, from_node, from_port, to_node, to_port);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_connect_in_template(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    voice_allocator_id: u32,
    from_node: u32,
    from_port: usize,
    to_node: u32,
    to_port: usize,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_connect_in_template(track_id, voice_allocator_id, from_node, from_port, to_node, to_port);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_disconnect(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    from_node: u32,
    from_port: usize,
    to_node: u32,
    to_port: usize,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_disconnect(track_id, from_node, from_port, to_node, to_port);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_set_parameter(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
    param_id: u32,
    value: f32,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_set_parameter(track_id, node_id, param_id, value);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_set_output_node(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        controller.graph_set_output_node(track_id, node_id);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

// Preset management commands

#[tauri::command]
pub async fn graph_save_preset(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    preset_name: String,
    description: String,
    tags: Vec<String>,
) -> Result<String, String> {
    use std::fs;

    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // Get user presets directory
        let app_data_dir = app_handle.path().app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;
        let presets_dir = app_data_dir.join("presets");

        // Create presets directory if it doesn't exist
        fs::create_dir_all(&presets_dir)
            .map_err(|e| format!("Failed to create presets directory: {}", e))?;

        // Create preset path
        let filename = format!("{}.json", preset_name.replace(" ", "_"));
        let preset_path = presets_dir.join(&filename);
        let preset_path_str = preset_path.to_string_lossy().to_string();

        // Send command to save preset
        controller.graph_save_preset(
            track_id,
            preset_path_str.clone(),
            preset_name,
            description,
            tags
        );

        Ok(preset_path_str)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_load_preset(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    preset_path: String,
) -> Result<(), String> {
    use daw_backend::GraphPreset;

    let mut audio_state = state.lock().unwrap();

    // Load the preset JSON to count nodes
    let json = std::fs::read_to_string(&preset_path)
        .map_err(|e| format!("Failed to read preset file: {}", e))?;
    let preset = GraphPreset::from_json(&json)
        .map_err(|e| format!("Failed to parse preset: {}", e))?;

    // Update the node ID counter to account for nodes in the preset
    let node_count = preset.nodes.len() as u32;
    audio_state.next_graph_node_id = node_count;

    if let Some(controller) = &mut audio_state.controller {
        // Send command to load preset
        controller.graph_load_preset(track_id, preset_path);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[derive(serde::Serialize)]
pub struct PresetInfo {
    pub name: String,
    pub path: String,
    pub description: String,
    pub author: String,
    pub tags: Vec<String>,
    pub is_factory: bool,
}

#[tauri::command]
pub async fn graph_list_presets(
    app_handle: tauri::AppHandle,
) -> Result<Vec<PresetInfo>, String> {
    use daw_backend::GraphPreset;
    use std::fs;

    let mut presets = Vec::new();

    // Recursively scan for JSON files in instruments directory
    fn scan_presets_recursive(dir: &std::path::Path, presets: &mut Vec<PresetInfo>) {
        eprintln!("Scanning directory: {:?}", dir);
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    // Recurse into subdirectories
                    scan_presets_recursive(&path, presets);
                } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    eprintln!("Found JSON file: {:?}", path);
                    // Load JSON preset files
                    match std::fs::read_to_string(&path) {
                        Ok(json) => {
                            match daw_backend::GraphPreset::from_json(&json) {
                                Ok(preset) => {
                                    eprintln!("  ✓ Loaded preset: {}", preset.metadata.name);
                                    presets.push(PresetInfo {
                                        name: preset.metadata.name,
                                        path: path.to_string_lossy().to_string(),
                                        description: preset.metadata.description,
                                        author: preset.metadata.author,
                                        tags: preset.metadata.tags,
                                        is_factory: true,
                                    });
                                }
                                Err(e) => eprintln!("  ✗ Failed to parse preset: {}", e),
                            }
                        }
                        Err(e) => eprintln!("  ✗ Failed to read file: {}", e),
                    }
                }
            }
        }
    }

    // Try multiple locations for instruments
    let mut instruments_found = false;

    // 1. Try bundled resources (production)
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        let instruments_dir = resource_dir.join("assets/instruments");
        eprintln!("Trying bundled path: {:?} (exists: {})", instruments_dir, instruments_dir.exists());
        if instruments_dir.exists() {
            scan_presets_recursive(&instruments_dir, &mut presets);
            instruments_found = true;
        }
    }

    // 2. Fallback to dev location (development mode)
    if !instruments_found {
        // Try relative to current working directory (dev mode)
        if let Ok(cwd) = std::env::current_dir() {
            let dev_instruments = cwd.join("../src/assets/instruments");
            eprintln!("Trying dev path: {:?} (exists: {})", dev_instruments, dev_instruments.exists());
            if dev_instruments.exists() {
                scan_presets_recursive(&dev_instruments, &mut presets);
            }
        }
    }

    eprintln!("Found {} factory presets", presets.len());

    // Load user presets
    if let Ok(app_data_dir) = app_handle.path().app_data_dir() {
        let user_presets_dir = app_data_dir.join("presets");
        if user_presets_dir.exists() {
            if let Ok(entries) = fs::read_dir(user_presets_dir) {
                for entry in entries.flatten() {
                    if let Ok(path) = entry.path().canonicalize() {
                        if path.extension().and_then(|s| s.to_str()) == Some("json") {
                            if let Ok(json) = fs::read_to_string(&path) {
                                if let Ok(preset) = GraphPreset::from_json(&json) {
                                    presets.push(PresetInfo {
                                        name: preset.metadata.name,
                                        path: path.to_string_lossy().to_string(),
                                        description: preset.metadata.description,
                                        author: preset.metadata.author,
                                        tags: preset.metadata.tags,
                                        is_factory: false,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(presets)
}

#[tauri::command]
pub async fn graph_delete_preset(
    app_handle: tauri::AppHandle,
    preset_path: String,
) -> Result<(), String> {
    use std::fs;
    use std::path::Path;

    let preset_path = Path::new(&preset_path);

    // Check if preset is in the app's resource directory (factory content - cannot delete)
    if let Ok(resource_dir) = app_handle.path().resource_dir() {
        if let Ok(canonical_preset) = preset_path.canonicalize() {
            if let Ok(canonical_resource) = resource_dir.canonicalize() {
                if canonical_preset.starts_with(canonical_resource) {
                    return Err("Cannot delete factory presets or bundled instruments".to_string());
                }
            }
        }
    }

    // If we get here, it's a user preset - safe to delete
    fs::remove_file(&preset_path)
        .map_err(|e| format!("Failed to delete preset: {}", e))?;

    Ok(())
}

#[tauri::command]
pub async fn graph_get_state(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
) -> Result<String, String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // Use synchronous query to get graph state
        controller.query_graph_state(track_id)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn graph_get_template_state(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    voice_allocator_id: u32,
) -> Result<String, String> {
    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // Use synchronous query to get template graph state
        controller.query_template_state(track_id, voice_allocator_id)
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn sampler_load_sample(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
    file_path: String,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();

    if let Some(controller) = &mut audio_state.controller {
        controller.sampler_load_sample(track_id, node_id, file_path);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn multi_sampler_add_layer(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
    file_path: String,
    key_min: u8,
    key_max: u8,
    root_key: u8,
    velocity_min: u8,
    velocity_max: u8,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();

    if let Some(controller) = &mut audio_state.controller {
        controller.multi_sampler_add_layer(
            track_id,
            node_id,
            file_path,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        );
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[derive(serde::Serialize)]
pub struct LayerInfo {
    pub file_path: String,
    pub key_min: u8,
    pub key_max: u8,
    pub root_key: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
}

#[tauri::command]
pub async fn multi_sampler_get_layers(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
) -> Result<Vec<LayerInfo>, String> {
    eprintln!("[multi_sampler_get_layers] FUNCTION CALLED with track_id: {}, node_id: {}", track_id, node_id);
    use daw_backend::GraphPreset;

    let mut audio_state = state.lock().unwrap();
    if let Some(controller) = &mut audio_state.controller {
        // Use preset serialization to get node data including layers
        // Use timestamp to ensure unique temp file for each query to avoid conflicts
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_path = std::env::temp_dir().join(format!("temp_layers_query_{}_{}_{}.json", track_id, node_id, timestamp));
        let temp_path_str = temp_path.to_string_lossy().to_string();
        eprintln!("[multi_sampler_get_layers] Temp path: {}", temp_path_str);

        controller.graph_save_preset(
            track_id,
            temp_path_str.clone(),
            "temp".to_string(),
            "".to_string(),
            vec![]
        );

        // Give the audio thread time to process
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Read the temp file and parse it
        eprintln!("[multi_sampler_get_layers] Reading temp file...");
        match std::fs::read_to_string(&temp_path) {
            Ok(json) => {
                // Clean up temp file
                let _ = std::fs::remove_file(&temp_path);

                // Parse the preset JSON
                let preset: GraphPreset = match serde_json::from_str(&json) {
                    Ok(p) => p,
                    Err(e) => return Err(format!("Failed to parse preset: {}", e)),
                };

                // Find the node with the matching ID
                eprintln!("[multi_sampler_get_layers] Looking for node_id: {}", node_id);
                eprintln!("[multi_sampler_get_layers] Available nodes: {:?}", preset.nodes.iter().map(|n| (n.id, &n.node_type)).collect::<Vec<_>>());

                if let Some(node) = preset.nodes.iter().find(|n| n.id == node_id) {
                    eprintln!("[multi_sampler_get_layers] Found node: {} type: {}", node.id, node.node_type);
                    if let Some(ref sample_data) = node.sample_data {
                        eprintln!("[multi_sampler_get_layers] Node has sample_data");
                        // Check if it's a MultiSampler
                        if let daw_backend::audio::node_graph::preset::SampleData::MultiSampler { layers } = sample_data {
                            eprintln!("[multi_sampler_get_layers] Returning {} layers", layers.len());
                            return Ok(layers.iter().map(|layer| LayerInfo {
                                file_path: layer.file_path.clone().unwrap_or_default(),
                                key_min: layer.key_min,
                                key_max: layer.key_max,
                                root_key: layer.root_key,
                                velocity_min: layer.velocity_min,
                                velocity_max: layer.velocity_max,
                            }).collect());
                        } else {
                            eprintln!("[multi_sampler_get_layers] sample_data is not MultiSampler type");
                        }
                    } else {
                        eprintln!("[multi_sampler_get_layers] Node has no sample_data");
                    }
                } else {
                    eprintln!("[multi_sampler_get_layers] Node not found");
                }

                Ok(Vec::new())
            }
            Err(e) => {
                eprintln!("[multi_sampler_get_layers] Failed to read temp file: {}", e);
                Ok(Vec::new()) // Return empty list if file doesn't exist
            }
        }
    } else {
        eprintln!("[multi_sampler_get_layers] Audio not initialized");
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn multi_sampler_update_layer(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
    layer_index: usize,
    key_min: u8,
    key_max: u8,
    root_key: u8,
    velocity_min: u8,
    velocity_max: u8,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();

    if let Some(controller) = &mut audio_state.controller {
        controller.multi_sampler_update_layer(
            track_id,
            node_id,
            layer_index,
            key_min,
            key_max,
            root_key,
            velocity_min,
            velocity_max,
        );
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[tauri::command]
pub async fn multi_sampler_remove_layer(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    track_id: u32,
    node_id: u32,
    layer_index: usize,
) -> Result<(), String> {
    let mut audio_state = state.lock().unwrap();

    if let Some(controller) = &mut audio_state.controller {
        controller.multi_sampler_remove_layer(track_id, node_id, layer_index);
        Ok(())
    } else {
        Err("Audio not initialized".to_string())
    }
}

#[derive(serde::Serialize, Clone)]
#[serde(tag = "type")]
pub enum SerializedAudioEvent {
    PlaybackPosition { time: f64 },
    RecordingStarted { track_id: u32, clip_id: u32 },
    RecordingProgress { clip_id: u32, duration: f64 },
    RecordingStopped { clip_id: u32, pool_index: usize, waveform: Vec<WaveformPeak> },
    RecordingError { message: String },
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8 },
    GraphNodeAdded { track_id: u32, node_id: u32, node_type: String },
    GraphConnectionError { track_id: u32, message: String },
    GraphStateChanged { track_id: u32 },
    GraphPresetLoaded { track_id: u32 },
}

// audio_get_events command removed - events are now pushed via Tauri event system
