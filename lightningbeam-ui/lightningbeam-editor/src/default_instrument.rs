/// Default MIDI instrument loader
///
/// This module provides a default instrument (bass synthesizer) for MIDI tracks
/// until the user implements the node editor to load custom instruments.

use std::path::PathBuf;

/// Embedded default MIDI instrument preset (bass synthesizer)
const DEFAULT_MIDI_INSTRUMENT: &str = include_str!("../../../src/assets/instruments/synthesizers/bass.json");

/// Load the default MIDI instrument into a daw-backend MIDI track
///
/// This function:
/// 1. Parses the embedded bass.json preset
/// 2. Writes it to a temporary file (required by daw-backend API)
/// 3. Loads the preset into the track's instrument graph
/// 4. Asynchronously cleans up the temp file after a delay
///
/// # Arguments
/// * `audio_controller` - Mutable reference to the daw-backend EngineController
/// * `track_id` - The MIDI track ID to load the instrument into
///
/// # Returns
/// * `Ok(())` if successful
/// * `Err(String)` with error message if parsing or file I/O fails
pub fn load_default_instrument(
    audio_controller: &mut daw_backend::EngineController,
    track_id: daw_backend::TrackId,
) -> Result<(), String> {
    // Verify the embedded JSON is valid by attempting to parse it
    let _preset: serde_json::Value = serde_json::from_str(DEFAULT_MIDI_INSTRUMENT)
        .map_err(|e| format!("Failed to parse embedded default instrument: {}", e))?;

    // Create temp directory path
    let temp_dir = std::env::temp_dir();
    let temp_filename = format!("lightningbeam_default_instrument_{}.json", track_id);
    let temp_path = temp_dir.join(&temp_filename);

    // Write preset to temporary file
    std::fs::write(&temp_path, DEFAULT_MIDI_INSTRUMENT)
        .map_err(|e| format!("Failed to write temp preset file: {}", e))?;

    // Load preset into track's instrument graph via daw-backend API
    let temp_path_str = temp_path.to_string_lossy().to_string();
    audio_controller.graph_load_preset(track_id, temp_path_str);

    // Schedule async cleanup of temp file (give backend time to load it first)
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let _ = std::fs::remove_file(temp_path);
    });

    Ok(())
}

/// Get the name of the default instrument for display purposes
pub fn default_instrument_name() -> &'static str {
    "Deep Bass (Default)"
}
