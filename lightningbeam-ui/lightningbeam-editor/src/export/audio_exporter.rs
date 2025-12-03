//! Audio export functionality
//!
//! Exports audio from the timeline to various formats:
//! - WAV and FLAC: Use existing DAW backend export
//! - MP3 and AAC: Use FFmpeg encoding with rendered samples

use lightningbeam_core::export::{AudioExportSettings, AudioFormat};
use daw_backend::audio::{
    export::{ExportFormat, ExportSettings as DawExportSettings, render_to_memory},
    midi_pool::MidiClipPool,
    pool::AudioPool,
    project::Project,
};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Export audio to a file
///
/// This function routes to the appropriate export method based on the format:
/// - WAV/FLAC: Use DAW backend export
/// - MP3/AAC: Use FFmpeg encoding (TODO)
pub fn export_audio<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
    cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    // Validate settings
    settings.validate()?;

    // Check for cancellation before starting
    if cancel_flag.load(Ordering::Relaxed) {
        return Err("Export cancelled by user".to_string());
    }

    match settings.format {
        AudioFormat::Wav | AudioFormat::Flac => {
            export_audio_daw_backend(project, pool, midi_pool, settings, output_path)
        }
        AudioFormat::Mp3 => {
            export_audio_ffmpeg_mp3(project, pool, midi_pool, settings, output_path, cancel_flag)
        }
        AudioFormat::Aac => {
            export_audio_ffmpeg_aac(project, pool, midi_pool, settings, output_path, cancel_flag)
        }
    }
}

/// Export audio using the DAW backend (WAV/FLAC)
fn export_audio_daw_backend<P: AsRef<Path>>(
    project: &mut Project,
    pool: &AudioPool,
    midi_pool: &MidiClipPool,
    settings: &AudioExportSettings,
    output_path: P,
) -> Result<(), String> {
    // Convert our export settings to DAW backend format
    let daw_settings = DawExportSettings {
        format: match settings.format {
            AudioFormat::Wav => ExportFormat::Wav,
            AudioFormat::Flac => ExportFormat::Flac,
            _ => unreachable!(), // This function only handles WAV/FLAC
        },
        sample_rate: settings.sample_rate,
        channels: settings.channels,
        bit_depth: settings.bit_depth,
        mp3_bitrate: 320, // Not used for WAV/FLAC
        start_time: settings.start_time,
        end_time: settings.end_time,
    };

    // Use the existing DAW backend export function
    // No progress reporting for this direct export path
    daw_backend::audio::export::export_audio(
        project,
        pool,
        midi_pool,
        &daw_settings,
        output_path,
        None,
    )
}

/// Export audio as MP3 using FFmpeg
fn export_audio_ffmpeg_mp3<P: AsRef<Path>>(
    _project: &mut Project,
    _pool: &AudioPool,
    _midi_pool: &MidiClipPool,
    _settings: &AudioExportSettings,
    _output_path: P,
    _cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    // TODO: Implement MP3 export using FFmpeg
    // The FFmpeg encoder API is complex and needs more investigation
    // For now, users can export as WAV or FLAC (both fully working)
    Err("MP3 export is not yet implemented. Please use WAV or FLAC format for now, or export as WAV and convert using an external tool.".to_string())
}

/// Export audio as AAC using FFmpeg
fn export_audio_ffmpeg_aac<P: AsRef<Path>>(
    _project: &mut Project,
    _pool: &AudioPool,
    _midi_pool: &MidiClipPool,
    _settings: &AudioExportSettings,
    _output_path: P,
    _cancel_flag: &Arc<AtomicBool>,
) -> Result<(), String> {
    // TODO: Implement AAC export using FFmpeg
    // The FFmpeg encoder API is complex and needs more investigation
    // For now, users can export as WAV or FLAC (both fully working)
    Err("AAC export is not yet implemented. Please use WAV or FLAC format for now, or export as WAV and convert using an external tool.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_audio_validation() {
        let mut settings = AudioExportSettings::default();
        settings.sample_rate = 0; // Invalid

        let project = Project::new();
        let pool = AudioPool::new();
        let midi_pool = MidiClipPool::new();
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let result = export_audio(
            &mut project.clone(),
            &pool,
            &midi_pool,
            &settings,
            "/tmp/test.wav",
            &cancel_flag,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Sample rate"));
    }

    #[test]
    fn test_export_audio_cancellation() {
        let settings = AudioExportSettings::default();
        let mut project = Project::new();
        let pool = AudioPool::new();
        let midi_pool = MidiClipPool::new();
        let cancel_flag = Arc::new(AtomicBool::new(true)); // Pre-cancelled

        let result = export_audio(
            &mut project,
            &pool,
            &midi_pool,
            &settings,
            "/tmp/test.wav",
            &cancel_flag,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cancelled"));
    }
}
