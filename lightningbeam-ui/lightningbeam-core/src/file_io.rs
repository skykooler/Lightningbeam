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
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

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

    /// Mapping from UI layer UUIDs to backend TrackIds
    /// Preserves the connection between UI layers and audio engine tracks across save/load
    #[serde(default)]
    pub layer_to_track_map: std::collections::HashMap<uuid::Uuid, u32>,

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

    /// Mapping from UI layer UUIDs to backend TrackIds (empty for old files)
    pub layer_to_track_map: std::collections::HashMap<uuid::Uuid, u32>,

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
    layer_to_track_map: &std::collections::HashMap<uuid::Uuid, u32>,
    _settings: &SaveSettings,
) -> Result<(), String> {
    let fn_start = std::time::Instant::now();
    eprintln!("📊 [SAVE_BEAM] Starting save_beam()...");

    // 1. Create backup if file exists and open it for reading old audio files
    let step1_start = std::time::Instant::now();
    let mut old_zip = if path.exists() {
        let backup_path = path.with_extension("beam.backup");
        std::fs::copy(path, &backup_path)
            .map_err(|e| format!("Failed to create backup: {}", e))?;

        // Open the backup as a ZIP archive for reading
        match File::open(&backup_path) {
            Ok(file) => match ZipArchive::new(file) {
                Ok(archive) => {
                    eprintln!("📊 [SAVE_BEAM] Step 1: Create backup and open for reading took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);
                    Some(archive)
                }
                Err(e) => {
                    eprintln!("⚠️ [SAVE_BEAM] Failed to open backup as ZIP: {}, will not copy old audio files", e);
                    eprintln!("📊 [SAVE_BEAM] Step 1: Create backup took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);
                    None
                }
            },
            Err(e) => {
                eprintln!("⚠️ [SAVE_BEAM] Failed to open backup: {}, will not copy old audio files", e);
                eprintln!("📊 [SAVE_BEAM] Step 1: Create backup took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);
                None
            }
        }
    } else {
        eprintln!("📊 [SAVE_BEAM] Step 1: No backup needed (new file)");
        None
    };

    // 2. Graph presets are already populated by the engine thread (in GetProject handler)
    // before cloning. Do NOT call prepare_for_save() here — the cloned project has
    // default empty graphs (AudioTrack::clone() doesn't copy the graph), so calling
    // prepare_for_save() would overwrite the good presets with empty ones.
    let step2_start = std::time::Instant::now();
    eprintln!("📊 [SAVE_BEAM] Step 2: (graph presets already prepared) took {:.2}ms", step2_start.elapsed().as_secs_f64() * 1000.0);

    // 3. Create ZIP writer
    let step3_start = std::time::Instant::now();
    let file = File::create(path)
        .map_err(|e| format!("Failed to create file: {}", e))?;
    let mut zip = ZipWriter::new(file);
    eprintln!("📊 [SAVE_BEAM] Step 3: Create ZIP writer took {:.2}ms", step3_start.elapsed().as_secs_f64() * 1000.0);

    // 4. Process audio pool entries and write embedded audio files to ZIP
    // Priority: old ZIP file > external file > encode PCM as FLAC
    let step4_start = std::time::Instant::now();
    let mut modified_entries = Vec::new();
    let mut flac_encode_time = 0.0;
    let mut zip_write_time = 0.0;
    let project_dir = path.parent().unwrap_or_else(|| Path::new("."));

    for entry in &audio_pool_entries {
        let mut modified_entry = entry.clone();

        // Try to get audio data from various sources (in priority order)
        let audio_source: Option<(Vec<u8>, String)> = if let Some(ref rel_path) = entry.relative_path {
            // Priority 1: Check if file is in the old ZIP
            if rel_path.starts_with("media/audio/") {
                if let Some(ref mut old_zip_archive) = old_zip {
                    match old_zip_archive.by_name(rel_path) {
                        Ok(mut file) => {
                            let mut bytes = Vec::new();
                            if file.read_to_end(&mut bytes).is_ok() {
                                let extension = rel_path.split('.').last().unwrap_or("bin").to_string();
                                eprintln!("📊 [SAVE_BEAM] Copying from old ZIP: {}", rel_path);
                                Some((bytes, extension))
                            } else {
                                eprintln!("⚠️ [SAVE_BEAM] Failed to read {} from old ZIP", rel_path);
                                None
                            }
                        }
                        Err(_) => {
                            eprintln!("⚠️ [SAVE_BEAM] File {} not found in old ZIP", rel_path);
                            None
                        }
                    }
                } else {
                    None
                }
            }
            // Priority 2: Check external filesystem
            else {
                let full_path = project_dir.join(rel_path);
                if full_path.exists() {
                    match std::fs::read(&full_path) {
                        Ok(bytes) => {
                            let extension = full_path.extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("bin")
                                .to_string();
                            eprintln!("📊 [SAVE_BEAM] Using external file: {:?}", full_path);
                            Some((bytes, extension))
                        }
                        Err(e) => {
                            eprintln!("⚠️ [SAVE_BEAM] Failed to read {:?}: {}", full_path, e);
                            None
                        }
                    }
                } else {
                    eprintln!("⚠️ [SAVE_BEAM] External file not found: {:?}", full_path);
                    None
                }
            }
        } else {
            None
        };

        if let Some((audio_bytes, extension)) = audio_source {
            // We have the original file - copy it directly
            let zip_filename = format!("media/audio/{}.{}", entry.pool_index, extension);

            let file_options = FileOptions::default()
                .compression_method(CompressionMethod::Stored);

            zip.start_file(&zip_filename, file_options)
                .map_err(|e| format!("Failed to create {} in ZIP: {}", zip_filename, e))?;

            let write_start = std::time::Instant::now();
            zip.write_all(&audio_bytes)
                .map_err(|e| format!("Failed to write {}: {}", zip_filename, e))?;
            zip_write_time += write_start.elapsed().as_secs_f64() * 1000.0;

            // Update entry to point to ZIP file
            modified_entry.embedded_data = None;
            modified_entry.relative_path = Some(zip_filename);

        } else if let Some(ref embedded_data) = entry.embedded_data {
            // Priority 3: No original file - encode PCM as FLAC
            eprintln!("📊 [SAVE_BEAM] Encoding PCM to FLAC for pool {} (no original file)", entry.pool_index);
            // Embedded data is always PCM - encode as FLAC
            let audio_bytes = BASE64_STANDARD.decode(&embedded_data.data_base64)
                .map_err(|e| format!("Failed to decode base64 audio data for pool index {}: {}", entry.pool_index, e))?;

            let zip_filename = format!("media/audio/{}.flac", entry.pool_index);

            let file_options = FileOptions::default()
                .compression_method(CompressionMethod::Stored);

            zip.start_file(&zip_filename, file_options)
                .map_err(|e| format!("Failed to create {} in ZIP: {}", zip_filename, e))?;

            // Encode PCM samples to FLAC
            let flac_start = std::time::Instant::now();

            // The audio_bytes are raw PCM samples (interleaved f32 little-endian)
            let samples: Vec<f32> = audio_bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect();

            // Convert f32 samples to i32 for FLAC encoding
            let samples_i32: Vec<i32> = samples
                .iter()
                .map(|&s| {
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
                24,
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

            flac_encode_time += flac_start.elapsed().as_secs_f64() * 1000.0;

            let write_start = std::time::Instant::now();
            zip.write_all(flac_bytes)
                .map_err(|e| format!("Failed to write {}: {}", zip_filename, e))?;
            zip_write_time += write_start.elapsed().as_secs_f64() * 1000.0;

            // Update entry to point to ZIP file instead of embedding data
            modified_entry.embedded_data = None;
            modified_entry.relative_path = Some(zip_filename);
        }

        modified_entries.push(modified_entry);
    }
    eprintln!("📊 [SAVE_BEAM] Step 4: Process audio pool ({} entries) took {:.2}ms",
              audio_pool_entries.len(), step4_start.elapsed().as_secs_f64() * 1000.0);
    if flac_encode_time > 0.0 {
        eprintln!("📊 [SAVE_BEAM]   - FLAC encoding: {:.2}ms", flac_encode_time);
    }
    if zip_write_time > 0.0 {
        eprintln!("📊 [SAVE_BEAM]   - ZIP writing: {:.2}ms", zip_write_time);
    }

    // 4b. Write raster layer PNG buffers to ZIP (media/raster/<keyframe-uuid>.png)
    let step4b_start = std::time::Instant::now();
    let raster_file_options = FileOptions::default()
        .compression_method(CompressionMethod::Stored); // PNG is already compressed
    let mut raster_count = 0usize;
    for layer in &document.root.children {
        if let crate::layer::AnyLayer::Raster(rl) = layer {
            for kf in &rl.keyframes {
                if !kf.raw_pixels.is_empty() {
                    // Encode raw RGBA to PNG for storage
                    let img = crate::brush_engine::image_from_raw(
                        kf.raw_pixels.clone(), kf.width, kf.height,
                    );
                    match crate::brush_engine::encode_png(&img) {
                        Ok(png_bytes) => {
                            let zip_path = kf.media_path.clone();
                            zip.start_file(&zip_path, raster_file_options)
                                .map_err(|e| format!("Failed to create {} in ZIP: {}", zip_path, e))?;
                            zip.write_all(&png_bytes)
                                .map_err(|e| format!("Failed to write {}: {}", zip_path, e))?;
                            raster_count += 1;
                        }
                        Err(e) => eprintln!("⚠️ [SAVE_BEAM] Failed to encode raster PNG {}: {}", kf.media_path, e),
                    }
                }
            }
        }
    }
    eprintln!("📊 [SAVE_BEAM] Step 4b: Write {} raster PNG buffers took {:.2}ms",
              raster_count, step4b_start.elapsed().as_secs_f64() * 1000.0);

    // 5. Build BeamProject structure with modified entries
    let step5_start = std::time::Instant::now();
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
            layer_to_track_map: layer_to_track_map.clone(),
        },
    };
    eprintln!("📊 [SAVE_BEAM] Step 5: Build BeamProject structure took {:.2}ms", step5_start.elapsed().as_secs_f64() * 1000.0);

    // 6. Write project.json (compressed with DEFLATE)
    let step6_start = std::time::Instant::now();
    let json_options = FileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(6));

    zip.start_file("project.json", json_options)
        .map_err(|e| format!("Failed to create project.json in ZIP: {}", e))?;

    let json = serde_json::to_string_pretty(&beam_project)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;

    zip.write_all(json.as_bytes())
        .map_err(|e| format!("Failed to write project.json: {}", e))?;
    eprintln!("📊 [SAVE_BEAM] Step 6: Write project.json ({} bytes) took {:.2}ms", json.len(), step6_start.elapsed().as_secs_f64() * 1000.0);

    // 7. Finalize ZIP
    let step7_start = std::time::Instant::now();
    zip.finish()
        .map_err(|e| format!("Failed to finalize ZIP: {}", e))?;
    eprintln!("📊 [SAVE_BEAM] Step 7: Finalize ZIP took {:.2}ms", step7_start.elapsed().as_secs_f64() * 1000.0);

    eprintln!("📊 [SAVE_BEAM] ✅ Total save_beam() time: {:.2}ms", fn_start.elapsed().as_secs_f64() * 1000.0);

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
    let fn_start = std::time::Instant::now();
    eprintln!("📊 [LOAD_BEAM] Starting load_beam()...");

    // 1. Open ZIP archive
    let step1_start = std::time::Instant::now();
    let file = File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    let mut zip = ZipArchive::new(file)
        .map_err(|e| format!("Failed to open ZIP archive: {}", e))?;
    eprintln!("📊 [LOAD_BEAM] Step 1: Open ZIP archive took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);

    // 2. Read project.json
    let step2_start = std::time::Instant::now();
    let mut project_file = zip.by_name("project.json")
        .map_err(|e| format!("Failed to find project.json in archive: {}", e))?;

    let mut json_data = String::new();
    project_file.read_to_string(&mut json_data)
        .map_err(|e| format!("Failed to read project.json: {}", e))?;
    eprintln!("📊 [LOAD_BEAM] Step 2: Read project.json ({} bytes) took {:.2}ms", json_data.len(), step2_start.elapsed().as_secs_f64() * 1000.0);

    // 3. Deserialize BeamProject
    let step3_start = std::time::Instant::now();
    let beam_project: BeamProject = serde_json::from_str(&json_data)
        .map_err(|e| format!("Failed to deserialize project.json: {}", e))?;
    eprintln!("📊 [LOAD_BEAM] Step 3: Deserialize BeamProject took {:.2}ms", step3_start.elapsed().as_secs_f64() * 1000.0);

    // 4. Check version compatibility
    if beam_project.version != BEAM_VERSION {
        return Err(format!(
            "Unsupported file version: {} (expected {})",
            beam_project.version, BEAM_VERSION
        ));
    }

    // 5. Extract document and audio backend state
    let step5_start = std::time::Instant::now();
    let mut document = beam_project.ui_state;
    // Rebuild derived seconds cache on all TempoMap entries after deserialization.
    document.tempo_map_mut().rebuild_seconds();
    let mut audio_project = beam_project.audio_backend.project;
    let audio_pool_entries = beam_project.audio_backend.audio_pool_entries;
    let layer_to_track_map = beam_project.audio_backend.layer_to_track_map;
    eprintln!("📊 [LOAD_BEAM] Step 5: Extract document and audio state took {:.2}ms", step5_start.elapsed().as_secs_f64() * 1000.0);

    // 6. Rebuild AudioGraphs from presets
    let step6_start = std::time::Instant::now();
    audio_project.rebuild_audio_graphs(DEFAULT_BUFFER_SIZE)
        .map_err(|e| format!("Failed to rebuild audio graphs: {}", e))?;
    eprintln!("📊 [LOAD_BEAM] Step 6: Rebuild AudioGraphs took {:.2}ms", step6_start.elapsed().as_secs_f64() * 1000.0);

    // 7. Extract embedded audio files from ZIP and restore to entries
    let step7_start = std::time::Instant::now();
    drop(project_file); // Close project.json file handle
    let mut restored_entries = Vec::new();
    let mut flac_decode_time = 0.0;

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
                            let flac_decode_start = std::time::Instant::now();

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

                            // Encode f32 samples as a proper WAV file (with RIFF header)
                            let channels = entry.channels;
                            let sample_rate = entry.sample_rate;
                            let num_samples = samples_f32.len();
                            let bytes_per_sample = 4u32; // 32-bit float
                            let data_size = num_samples * bytes_per_sample as usize;
                            let file_size = 36 + data_size;

                            let mut wav_data = Vec::with_capacity(44 + data_size);
                            wav_data.extend_from_slice(b"RIFF");
                            wav_data.extend_from_slice(&(file_size as u32).to_le_bytes());
                            wav_data.extend_from_slice(b"WAVE");
                            wav_data.extend_from_slice(b"fmt ");
                            wav_data.extend_from_slice(&16u32.to_le_bytes());
                            wav_data.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
                            wav_data.extend_from_slice(&(channels as u16).to_le_bytes());
                            wav_data.extend_from_slice(&sample_rate.to_le_bytes());
                            wav_data.extend_from_slice(&(sample_rate * channels * bytes_per_sample).to_le_bytes());
                            wav_data.extend_from_slice(&((channels * bytes_per_sample) as u16).to_le_bytes());
                            wav_data.extend_from_slice(&32u16.to_le_bytes());
                            wav_data.extend_from_slice(b"data");
                            wav_data.extend_from_slice(&(data_size as u32).to_le_bytes());
                            for &sample in &samples_f32 {
                                wav_data.extend_from_slice(&sample.to_le_bytes());
                            }

                            flac_decode_time += flac_decode_start.elapsed().as_secs_f64() * 1000.0;

                            Some(daw_backend::audio::pool::EmbeddedAudioData {
                                data_base64: BASE64_STANDARD.encode(&wav_data),
                                format: "wav".to_string(),
                            })
                        } else {
                            // Lossy format - store as-is
                            Some(daw_backend::audio::pool::EmbeddedAudioData {
                                data_base64: BASE64_STANDARD.encode(&audio_bytes),
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
    eprintln!("📊 [LOAD_BEAM] Step 7: Extract embedded audio ({} entries) took {:.2}ms",
              audio_pool_entries.len(), step7_start.elapsed().as_secs_f64() * 1000.0);
    if flac_decode_time > 0.0 {
        eprintln!("📊 [LOAD_BEAM]   - FLAC decoding: {:.2}ms", flac_decode_time);
    }

    // 7b. Load raster layer PNG buffers from ZIP
    let step7b_start = std::time::Instant::now();
    let mut raster_load_count = 0usize;
    for layer in document.root.children.iter_mut() {
        if let crate::layer::AnyLayer::Raster(rl) = layer {
            for kf in &mut rl.keyframes {
                if !kf.media_path.is_empty() {
                    match zip.by_name(&kf.media_path) {
                        Ok(mut png_file) => {
                            let mut png_bytes = Vec::new();
                            let _ = png_file.read_to_end(&mut png_bytes);
                            // Decode PNG into raw RGBA pixels for fast in-memory access
                            match crate::brush_engine::decode_png(&png_bytes) {
                                Ok(rgba) => {
                                    kf.raw_pixels = rgba.into_raw();
                                    raster_load_count += 1;
                                }
                                Err(e) => eprintln!("⚠️ [LOAD_BEAM] Failed to decode raster PNG {}: {}", kf.media_path, e),
                            }
                        }
                        Err(_) => {
                            // Keyframe PNG not in ZIP yet (new keyframe); leave raw_pixels empty
                        }
                    }
                }
            }
        }
    }
    eprintln!("📊 [LOAD_BEAM] Step 7b: Load {} raster PNG buffers took {:.2}ms",
              raster_load_count, step7b_start.elapsed().as_secs_f64() * 1000.0);

    // 8. Check for missing external files
    // An entry is missing if it has a relative_path (external reference)
    // but no embedded_data and the file doesn't exist
    let step8_start = std::time::Instant::now();
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
    eprintln!("📊 [LOAD_BEAM] Step 8: Check missing files took {:.2}ms", step8_start.elapsed().as_secs_f64() * 1000.0);

    eprintln!("📊 [LOAD_BEAM] ✅ Total load_beam() time: {:.2}ms", fn_start.elapsed().as_secs_f64() * 1000.0);

    Ok(LoadedProject {
        document,
        audio_project,
        layer_to_track_map,
        audio_pool_entries: restored_entries,
        missing_files,
    })
}
