//! File I/O for .beam project files
//!
//! This module handles saving and loading Lightningbeam projects in the .beam format,
//! which is a ZIP archive containing:
//! - project.json (compressed) - Project metadata and structure
//! - media/ directory (uncompressed) - Embedded media files (FLAC for audio)

use crate::document::Document;
use daw_backend::audio::pool::AudioPoolEntry;
use daw_backend::audio::project::Project as AudioProject;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};
use flacenc::error::Verify;

/// File format version
pub const BEAM_VERSION: &str = "1.0.0";

/// Default buffer size for audio processing (512 samples = ~10.7ms at 48kHz)
pub const DEFAULT_BUFFER_SIZE: usize = 512;

/// Complete .beam project structure for serialization
#[derive(Serialize, Deserialize)]
pub struct BeamProject {
    /// File format version
    pub version: String,

    /// Project creation timestamp (ISO 8601)
    pub created: String,

    /// Last modified timestamp (ISO 8601)
    pub modified: String,

    /// UI state (Document from lightningbeam-core)
    pub ui_state: Document,

    /// Audio backend state
    pub audio_backend: SerializedAudioBackend,
}

/// Serialized audio backend state
#[derive(Serialize, Deserialize)]
pub struct SerializedAudioBackend {
    /// Sample rate for audio processing
    pub sample_rate: u32,

    /// Audio project (tracks, MIDI clips, etc.)
    pub project: AudioProject,

    /// Audio pool entries (metadata and paths for audio files)
    /// Note: embedded_data field from daw-backend is ignored; embedded files
    /// are stored as FLAC in the ZIP's media/audio/ directory instead
    pub audio_pool_entries: Vec<AudioPoolEntry>,
}

/// Settings for saving a project
#[derive(Debug, Clone)]
pub struct SaveSettings {
    /// Automatically embed files smaller than this size (in bytes)
    pub auto_embed_threshold_bytes: u64,

    /// Force embedding all media files
    pub force_embed_all: bool,

    /// Force linking all media files (don't embed any)
    pub force_link_all: bool,
}

impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            auto_embed_threshold_bytes: 10_000_000, // 10 MB
            force_embed_all: false,
            force_link_all: false,
        }
    }
}

/// Result of loading a project
pub struct LoadedProject {
    /// Deserialized document
    pub document: Document,

    /// Deserialized audio project
    pub audio_project: AudioProject,

    /// Loaded audio pool entries
    pub audio_pool_entries: Vec<AudioPoolEntry>,

    /// List of files that couldn't be found
    pub missing_files: Vec<MissingFileInfo>,
}

/// Information about a missing file
#[derive(Debug, Clone)]
pub struct MissingFileInfo {
    /// Index in the audio pool
    pub pool_index: usize,

    /// Original file path
    pub original_path: PathBuf,

    /// Type of media file
    pub file_type: MediaFileType,
}

/// Type of media file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFileType {
    Audio,
    Video,
    Image,
}

/// Save a project to a .beam file
///
/// This function:
/// 1. Prepares audio project for save (saves AudioGraph presets)
/// 2. Serializes project data to JSON
/// 3. Creates ZIP archive with compressed project.json
/// 4. Embeds media files as FLAC (for audio) in media/ directory
///
/// # Arguments
/// * `path` - Path to save the .beam file
/// * `document` - UI document state
/// * `audio_project` - Audio backend project
/// * `audio_pool_entries` - Serialized audio pool entries
/// * `settings` - Save settings (embedding preferences)
///
/// # Returns
/// Ok(()) on success, or error message
pub fn save_beam(
    path: &Path,
    document: &Document,
    audio_project: &mut AudioProject,
    audio_pool_entries: Vec<AudioPoolEntry>,
    _settings: &SaveSettings,
) -> Result<(), String> {
    // 1. Create backup if file exists
    if path.exists() {
        let backup_path = path.with_extension("beam.backup");
        std::fs::copy(path, &backup_path)
            .map_err(|e| format!("Failed to create backup: {}", e))?;
    }

    // 2. Prepare audio project for serialization (save AudioGraph presets)
    audio_project.prepare_for_save();

    // 3. Create ZIP writer
    let file = File::create(path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    let mut zip = ZipWriter::new(file);

    // 4. Process audio pool entries and write embedded audio files to ZIP
    // Smart compression: lossy formats (mp3, ogg) stored as-is, lossless data as FLAC
    let mut modified_entries = Vec::new();
    for entry in &audio_pool_entries {
        let mut modified_entry = entry.clone();

        if let Some(ref embedded_data) = entry.embedded_data {
            // Decode base64 audio data
            let audio_bytes = base64::decode(&embedded_data.data_base64)
                .map_err(|e| format!("Failed to decode base64 audio data for pool index {}: {}", entry.pool_index, e))?;

            let format_lower = embedded_data.format.to_lowercase();
            let is_lossy = format_lower == "mp3" || format_lower == "ogg"
                        || format_lower == "aac" || format_lower == "m4a"
                        || format_lower == "opus";

            let zip_filename = if is_lossy {
                // Store lossy formats directly (no transcoding)
                format!("media/audio/{}.{}", entry.pool_index, embedded_data.format)
            } else {
                // Store lossless data as FLAC
                format!("media/audio/{}.flac", entry.pool_index)
            };

            // Write to ZIP (uncompressed - audio is already compressed)
            let file_options = FileOptions::default()
                .compression_method(CompressionMethod::Stored);

            zip.start_file(&zip_filename, file_options)
                .map_err(|e| format!("Failed to create {} in ZIP: {}", zip_filename, e))?;

            if is_lossy {
                // Write lossy file directly
                zip.write_all(&audio_bytes)
                    .map_err(|e| format!("Failed to write {}: {}", zip_filename, e))?;
            } else {
                // Decode PCM samples and encode to FLAC
                // The audio_bytes are raw PCM samples (interleaved f32 little-endian)
                let samples: Vec<f32> = audio_bytes
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();

                // Convert f32 samples to i32 for FLAC encoding (FLAC doesn't support f32)
                // FLAC supports up to 24-bit samples: range [-8388608, 8388607]
                let samples_i32: Vec<i32> = samples
                    .iter()
                    .map(|&s| {
                        // Clamp to [-1.0, 1.0] first, then scale to 24-bit range
                        let clamped = s.clamp(-1.0, 1.0);
                        (clamped * 8388607.0) as i32
                    })
                    .collect();

                // Configure FLAC encoder
                let config = flacenc::config::Encoder::default()
                    .into_verified()
                    .map_err(|(_, e)| format!("FLAC encoder config error: {:?}", e))?;

                let source = flacenc::source::MemSource::from_samples(
                    &samples_i32,
                    entry.channels as usize,
                    24, // bits per sample (FLAC max is 24-bit)
                    entry.sample_rate as usize,
                );

                // Encode to FLAC
                let flac_stream = flacenc::encode_with_fixed_block_size(
                    &config,
                    source,
                    config.block_size,
                ).map_err(|e| format!("FLAC encoding failed: {:?}", e))?;

                // Convert stream to bytes
                use flacenc::component::BitRepr;
                let mut sink = flacenc::bitsink::ByteSink::new();
                flac_stream.write(&mut sink)
                    .map_err(|e| format!("Failed to write FLAC stream: {:?}", e))?;
                let flac_bytes = sink.as_slice();

                zip.write_all(flac_bytes)
                    .map_err(|e| format!("Failed to write {}: {}", zip_filename, e))?;
            }

            // Update entry to point to ZIP file instead of embedding data
            modified_entry.embedded_data = None;
            modified_entry.relative_path = Some(zip_filename);
        }

        modified_entries.push(modified_entry);
    }

    // 5. Build BeamProject structure with modified entries
    let now = chrono::Utc::now().to_rfc3339();
    let beam_project = BeamProject {
        version: BEAM_VERSION.to_string(),
        created: now.clone(),
        modified: now,
        ui_state: document.clone(),
        audio_backend: SerializedAudioBackend {
            sample_rate: 48000, // TODO: Get from audio engine
            project: audio_project.clone(),
            audio_pool_entries: modified_entries,
        },
    };

    // 6. Write project.json (compressed with DEFLATE)
    let json_options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    zip.start_file("project.json", json_options)
        .map_err(|e| format!("Failed to create project.json in ZIP: {}", e))?;

    let json = serde_json::to_string_pretty(&beam_project)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;

    zip.write_all(json.as_bytes())
        .map_err(|e| format!("Failed to write project.json: {}", e))?;

    // 7. Finalize ZIP
    zip.finish()
        .map_err(|e| format!("Failed to finalize ZIP: {}", e))?;

    Ok(())
}

/// Load a project from a .beam file
///
/// This function:
/// 1. Opens ZIP archive and reads project.json
/// 2. Deserializes project data
/// 3. Loads embedded media files from archive
/// 4. Attempts to load external media files
/// 5. Rebuilds AudioGraphs from presets with correct sample_rate
///
/// # Arguments
/// * `path` - Path to the .beam file
///
/// # Returns
/// LoadedProject on success (with missing_files list), or error message
pub fn load_beam(path: &Path) -> Result<LoadedProject, String> {
    // 1. Open ZIP archive
    let file = File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    let mut zip = ZipArchive::new(file)
        .map_err(|e| format!("Failed to open ZIP archive: {}", e))?;

    // 2. Read project.json
    let mut project_file = zip.by_name("project.json")
        .map_err(|e| format!("Failed to find project.json in archive: {}", e))?;

    let mut json_data = String::new();
    project_file.read_to_string(&mut json_data)
        .map_err(|e| format!("Failed to read project.json: {}", e))?;

    // 3. Deserialize BeamProject
    let beam_project: BeamProject = serde_json::from_str(&json_data)
        .map_err(|e| format!("Failed to deserialize project.json: {}", e))?;

    // 4. Check version compatibility
    if beam_project.version != BEAM_VERSION {
        return Err(format!(
            "Unsupported file version: {} (expected {})",
            beam_project.version, BEAM_VERSION
        ));
    }

    // 5. Extract document and audio backend state
    let document = beam_project.ui_state;
    let mut audio_project = beam_project.audio_backend.project;
    let audio_pool_entries = beam_project.audio_backend.audio_pool_entries;

    // 6. Rebuild AudioGraphs from presets
    audio_project.rebuild_audio_graphs(DEFAULT_BUFFER_SIZE)
        .map_err(|e| format!("Failed to rebuild audio graphs: {}", e))?;

    // 7. Extract embedded audio files from ZIP and restore to entries
    drop(project_file); // Close project.json file handle
    let mut restored_entries = Vec::new();

    for entry in &audio_pool_entries {
        let mut restored_entry = entry.clone();

        // Check if this entry has a file in the ZIP (relative_path starts with "media/audio/")
        if let Some(ref rel_path) = entry.relative_path {
            if rel_path.starts_with("media/audio/") {
                // Extract file from ZIP
                match zip.by_name(rel_path) {
                    Ok(mut audio_file) => {
                        let mut audio_bytes = Vec::new();
                        audio_file.read_to_end(&mut audio_bytes)
                            .map_err(|e| format!("Failed to read {} from ZIP: {}", rel_path, e))?;

                        // Determine format from filename
                        let format = rel_path.split('.').last()
                            .unwrap_or("flac")
                            .to_string();

                        // For lossless formats, decode back to PCM f32 samples
                        // For lossy formats, store the original bytes
                        let embedded_data = if format == "flac" {
                            // Decode FLAC to PCM f32 samples
                            let cursor = std::io::Cursor::new(&audio_bytes);
                            let mut reader = claxon::FlacReader::new(cursor)
                                .map_err(|e| format!("Failed to create FLAC reader: {:?}", e))?;

                            let stream_info = reader.streaminfo();
                            let bits_per_sample = stream_info.bits_per_sample;
                            let max_value = (1i64 << (bits_per_sample - 1)) as f32;

                            // Read all samples and convert to f32
                            let mut samples_f32 = Vec::new();
                            for sample_result in reader.samples() {
                                let sample = sample_result
                                    .map_err(|e| format!("Failed to read FLAC sample: {:?}", e))?;
                                samples_f32.push(sample as f32 / max_value);
                            }

                            // Convert f32 samples to bytes (little-endian)
                            let mut pcm_bytes = Vec::new();
                            for sample in samples_f32 {
                                pcm_bytes.extend_from_slice(&sample.to_le_bytes());
                            }

                            Some(daw_backend::audio::pool::EmbeddedAudioData {
                                data_base64: base64::encode(&pcm_bytes),
                                format: "wav".to_string(), // Mark as WAV since it's now PCM
                            })
                        } else {
                            // Lossy format - store as-is
                            Some(daw_backend::audio::pool::EmbeddedAudioData {
                                data_base64: base64::encode(&audio_bytes),
                                format: format.clone(),
                            })
                        };

                        restored_entry.embedded_data = embedded_data;
                        restored_entry.relative_path = None; // Clear ZIP path
                    }
                    Err(_) => {
                        // File not found in ZIP, treat as external reference
                    }
                }
            }
        }

        restored_entries.push(restored_entry);
    }

    // 8. Check for missing external files
    // An entry is missing if it has a relative_path (external reference)
    // but no embedded_data and the file doesn't exist
    let project_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let missing_files: Vec<MissingFileInfo> = restored_entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            // Check if this entry references an external file that doesn't exist
            if entry.embedded_data.is_none() {
                if let Some(ref rel_path) = entry.relative_path {
                    let full_path = project_dir.join(rel_path);
                    if !full_path.exists() {
                        return Some(MissingFileInfo {
                            pool_index: idx,
                            original_path: full_path,
                            file_type: MediaFileType::Audio,
                        });
                    }
                }
            }
            None
        })
        .collect();

    Ok(LoadedProject {
        document,
        audio_project,
        audio_pool_entries: restored_entries,
        missing_files,
    })
}
