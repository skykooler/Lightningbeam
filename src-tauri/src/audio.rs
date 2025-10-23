use daw_backend::{AudioEvent, AudioSystem, EngineController, EventEmitter, WaveformPeak};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

#[derive(serde::Serialize)]
pub struct AudioFileMetadata {
    pub pool_index: usize,
    pub duration: f64,
    pub sample_rate: u32,
    pub channels: u32,
    pub waveform: Vec<WaveformPeak>,
}

pub struct AudioState {
    controller: Option<EngineController>,
    sample_rate: u32,
    channels: u32,
    next_track_id: u32,
    next_pool_index: usize,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            controller: None,
            sample_rate: 0,
            channels: 0,
            next_track_id: 0,
            next_pool_index: 0,
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
pub async fn audio_create_track(
    state: tauri::State<'_, Arc<Mutex<AudioState>>>,
    name: String,
    track_type: String,
) -> Result<u32, String> {
    let mut audio_state = state.lock().unwrap();

    // Get track ID and increment counter before borrowing controller
    let track_id = audio_state.next_track_id;
    audio_state.next_track_id += 1;

    if let Some(controller) = &mut audio_state.controller {
        match track_type.as_str() {
            "audio" => controller.create_audio_track(name),
            "midi" => controller.create_midi_track(name),
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

#[derive(serde::Serialize, Clone)]
#[serde(tag = "type")]
pub enum SerializedAudioEvent {
    RecordingStarted { track_id: u32, clip_id: u32 },
    RecordingProgress { clip_id: u32, duration: f64 },
    RecordingStopped { clip_id: u32, pool_index: usize, waveform: Vec<WaveformPeak> },
    RecordingError { message: String },
}

// audio_get_events command removed - events are now pushed via Tauri event system
