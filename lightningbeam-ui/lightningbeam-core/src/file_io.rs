//! File I/O for .beam project files
//!
//! The `.beam` format is a single **SQLite database** (see [`crate::beam_archive`]):
//! - `project_json` table — serialized project metadata and structure
//! - `media` / `media_chunk` tables — audio and raster media (packed as chunked
//!   blobs, or referenced by external path)
//!
//! Older `.beam` files are ZIP archives; [`load_beam`] detects and reads those
//! too (via [`load_beam_zip_legacy`]). Saving always writes the SQLite form, so
//! opening a legacy file and saving migrates it.

use crate::beam_archive::{BeamArchive, MediaKind, MediaMeta, LARGE_MEDIA_THRESHOLD};
use crate::document::Document;
use daw_backend::audio::pool::AudioPoolEntry;
use daw_backend::audio::project::Project as AudioProject;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use zip::ZipArchive;
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

    /// Audio pool entries (metadata and media references for audio files)
    pub audio_pool_entries: Vec<AudioPoolEntry>,

    /// Mapping from UI layer UUIDs to backend TrackIds
    /// Preserves the connection between UI layers and audio engine tracks across save/load
    #[serde(default)]
    pub layer_to_track_map: std::collections::HashMap<uuid::Uuid, u32>,

}

/// How to store a media file at or above [`LARGE_MEDIA_THRESHOLD`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LargeMediaMode {
    /// Not yet decided — prompt the user the first time a large file is imported.
    /// Treated as [`LargeMediaMode::Reference`] at save time. Resetting the
    /// preference to `Ask` re-triggers the prompt (useful for testing).
    Ask,
    /// Pack the bytes into the `.beam` container (chunked, streamed from disk).
    Pack,
    /// Keep the file external and store only a path reference.
    Reference,
}

impl Default for LargeMediaMode {
    fn default() -> Self {
        LargeMediaMode::Ask
    }
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

    /// How to store files at/above [`LARGE_MEDIA_THRESHOLD`] (pack vs reference).
    /// `Ask` behaves as `Reference` here (safe default: don't bloat the DB).
    pub large_media_mode: LargeMediaMode,
}

impl Default for SaveSettings {
    fn default() -> Self {
        Self {
            auto_embed_threshold_bytes: 10_000_000, // 10 MB
            force_embed_all: false,
            force_link_all: false,
            large_media_mode: LargeMediaMode::Ask,
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

    /// Persisted video-thumbnail packs by clip id (opaque LBTN blobs; decoded and
    /// inserted into the VideoManager by the editor). Clips present here don't need
    /// their thumbnails regenerated on load.
    pub thumbnail_blobs: std::collections::HashMap<uuid::Uuid, Vec<u8>>,

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

/// Save a project to a `.beam` file (SQLite container).
///
/// Re-saving an existing SQLite `.beam` updates it **in place** inside a single
/// transaction: unchanged (large) media is never rewritten, only changed rows
/// are touched, and the commit is atomic/crash-safe. A brand-new file or a
/// legacy-ZIP migration is written to a temp file and atomically renamed (there
/// is no large existing container to copy in that case).
///
/// Audio and raster media become rows in the `media` table — packed as chunked
/// blobs, or referenced by external path for files at/above
/// [`LARGE_MEDIA_THRESHOLD`]. `project.json` goes in the `project_json` table.
/// Whether a stored media codec is an audio format the disk reader (Symphonia)
/// can stream directly from a packed blob. Video-container audio tracks and any
/// unknown formats fall back to the legacy reconstitution-and-decode path.
fn is_streamable_audio_codec(codec: &str) -> bool {
    matches!(
        codec.to_lowercase().as_str(),
        "mp3" | "flac" | "ogg" | "oga" | "wav" | "wave" | "aiff" | "aif"
            | "aac" | "m4a" | "opus" | "alac" | "caf"
    )
}

/// A `Sync` wrapper over core's `BlobReader` so it satisfies Symphonia's
/// `MediaSource: Send + Sync`. `BlobReader` holds a rusqlite `Connection`
/// (`Send` but `!Sync`); the disk reader uses it single-threaded, so the
/// hot Read/Seek path goes through `Mutex::get_mut` (no runtime locking).
struct SyncBlobReader {
    inner: std::sync::Mutex<crate::beam_archive::BlobReader>,
    len: u64,
}

impl std::io::Read for SyncBlobReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.get_mut().unwrap().read(buf)
    }
}
impl std::io::Seek for SyncBlobReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.inner.get_mut().unwrap().seek(pos)
    }
}
impl daw_backend::audio::MediaByteSource for SyncBlobReader {
    fn byte_len(&self) -> u64 {
        self.len
    }
}

/// The host's packed-media byte-source factory: opens an independent streaming
/// reader over a `.beam` container's packed audio by media id. Installed into the
/// engine on load so container-packed audio streams without a full decode.
#[derive(Debug)]
struct BeamBlobFactory {
    db_path: PathBuf,
}

impl daw_backend::audio::AudioBlobSourceFactory for BeamBlobFactory {
    fn open(
        &self,
        media_id: &str,
    ) -> Result<Box<dyn daw_backend::audio::MediaByteSource>, String> {
        let id = Uuid::parse_str(media_id).map_err(|e| format!("bad media id {}: {}", media_id, e))?;
        let archive = BeamArchive::open(&self.db_path)?;
        let reader = archive.open_blob_reader(&self.db_path, id)?;
        let len = reader.len();
        Ok(Box::new(SyncBlobReader { inner: std::sync::Mutex::new(reader), len }))
    }
}

/// Build a packed-media byte-source factory for a `.beam` file, to install into
/// the engine (`EngineController::set_blob_source_factory`) before loading so its
/// packed audio can be streamed.
pub fn blob_source_factory(
    beam_path: &Path,
) -> std::sync::Arc<dyn daw_backend::audio::AudioBlobSourceFactory> {
    std::sync::Arc::new(BeamBlobFactory { db_path: beam_path.to_path_buf() })
}

/// Deterministic id for the waveform-pyramid media row of audio pool entry
/// `pool_index`, within a single project container. Stable across saves (so an
/// in-place re-save reuses the row instead of orphaning/rewriting it) and
/// independent of how the audio bytes are stored. The top 32 bits are a fixed
/// "LBWF" sentinel so it can't collide with the random v4 ids used elsewhere.
fn waveform_media_id(pool_index: usize) -> Uuid {
    const SENTINEL: u128 = 0x4C42_5746u128 << 96; // "LBWF" in the high 32 bits
    Uuid::from_u128(SENTINEL | (pool_index as u128))
}

/// Deterministic id for the thumbnail-pack media row of a video clip. Derived from
/// the clip id by XOR with a fixed constant — bijective and stable across saves, so
/// it reuses the row in place, and it can't (in practice) collide with the random
/// v4 ids used for other media of a different kind.
fn thumbnail_media_id(clip_id: Uuid) -> Uuid {
    // "LBTN" repeated — moves clip ids into a distinct region of the id space.
    const SENTINEL: u128 = 0x4C42_544E_4C42_544E_4C42_544E_4C42_544E;
    Uuid::from_u128(clip_id.as_u128() ^ SENTINEL)
}

pub fn save_beam(
    path: &Path,
    document: &Document,
    audio_project: &mut AudioProject,
    audio_pool_entries: Vec<AudioPoolEntry>,
    layer_to_track_map: &std::collections::HashMap<uuid::Uuid, u32>,
    thumbnail_blobs: &std::collections::HashMap<uuid::Uuid, Vec<u8>>,
    _settings: &SaveSettings,
) -> Result<(), String> {
    let fn_start = std::time::Instant::now();
    eprintln!("📊 [SAVE_BEAM] Starting save_beam() (SQLite container)...");

    let project_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let in_place = path.exists() && BeamArchive::is_sqlite(path);

    // In-place for an existing SQLite container (don't rewrite unchanged media);
    // temp + atomic rename for new files / legacy-ZIP migration.
    let tmp_path = path.with_extension("beam.tmp");
    let mut archive = if in_place {
        BeamArchive::open(path)?
    } else {
        BeamArchive::create(&tmp_path)?
    };

    let now = chrono::Utc::now().to_rfc3339();
    let created = if in_place {
        archive.get_meta("created").ok().flatten().unwrap_or_else(|| now.clone())
    } else {
        now.clone()
    };

    let txn = archive.transaction()?;

    // --- audio pool entries -> media rows (packed) or external references ---
    let mut modified_entries = Vec::with_capacity(audio_pool_entries.len());
    let mut live_media: HashSet<Uuid> = HashSet::new();

    for entry in &audio_pool_entries {
        let mut e = entry.clone();
        let existing_id = entry.media_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());

        // Already packed in this archive (in-place re-save): leave the bytes
        // untouched, just keep the reference.
        if let Some(id) = existing_id {
            if txn.media_exists(id)? {
                live_media.insert(id);
                e.media_id = Some(id.to_string());
                e.relative_path = None;
                e.embedded_data = None;
                modified_entries.push(e);
                continue;
            }
        }

        // Otherwise resolve the source: external file (Priority 2, streamed from
        // disk so a huge file is never fully loaded), or embedded data (Priority 3).
        let meta = MediaMeta {
            channels: Some(entry.channels),
            sample_rate: Some(entry.sample_rate),
            ..Default::default()
        };
        let mut wrote_packed: Option<Uuid> = None;
        let mut referenced: Option<String> = None;

        if let Some(rel) = entry.relative_path.as_ref() {
            let full = if Path::new(rel).is_absolute() {
                PathBuf::from(rel)
            } else {
                project_dir.join(rel)
            };
            // Require an actual file: an empty/blank `relative_path` resolves to the
            // project directory itself (`join("")` == dir), which `exists()` accepts
            // but can't be read as media. `is_file()` skips dirs + missing paths, so
            // such an entry correctly falls through to embedded data below.
            if full.is_file() {
                let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
                let codec = full
                    .extension()
                    .and_then(|x| x.to_str())
                    .unwrap_or("bin")
                    .to_lowercase();
                // Video-audio entries are always referenced (the video is already
                // referenced by its VideoClip; reloaded by re-probing via FFmpeg).
                // Otherwise large files honor the user's pack-vs-reference choice
                // (`Ask` == reference); smaller files are always packed.
                let reference_it = entry.is_video_audio
                    || (size >= LARGE_MEDIA_THRESHOLD
                        && _settings.large_media_mode != LargeMediaMode::Pack);
                if reference_it {
                    referenced = Some(rel.clone());
                } else {
                    let id = existing_id.unwrap_or_else(Uuid::new_v4);
                    txn.put_media_packed_from_path(id, MediaKind::Audio, &codec, &full, meta)?;
                    wrote_packed = Some(id);
                }
            }
        }

        if wrote_packed.is_none() && referenced.is_none() {
            if let Some(ed) = entry.embedded_data.as_ref() {
                if let Ok(bytes) = BASE64_STANDARD.decode(&ed.data_base64) {
                    let id = existing_id.unwrap_or_else(Uuid::new_v4);
                    txn.put_media_packed(id, MediaKind::Audio, &ed.format.to_lowercase(), &bytes, meta)?;
                    wrote_packed = Some(id);
                }
            }
        }

        if let Some(id) = wrote_packed {
            live_media.insert(id);
            e.media_id = Some(id.to_string());
            e.relative_path = None;
            e.embedded_data = None;
        } else if let Some(rel) = referenced {
            e.media_id = None;
            e.relative_path = Some(rel);
            e.embedded_data = None;
        } // else: nothing available — keep original references (reported missing on load)

        // Persist this entry's waveform pyramid (keyed by pool index, independent
        // of the audio storage above). Reuse the row in place on re-save.
        let wf_id = waveform_media_id(entry.pool_index);
        if let Some(blob) = entry.waveform_blob.as_ref() {
            txn.put_media_packed(wf_id, MediaKind::Waveform, "lbwf", blob, MediaMeta::default())?;
            live_media.insert(wf_id);
        } else if txn.media_exists(wf_id)? {
            // Unchanged this save — keep the stored waveform row.
            live_media.insert(wf_id);
        }

        modified_entries.push(e);
    }

    // --- raster keyframes -> media rows (PNG), keyed by keyframe id ---
    // (Phase 0 writes all resident frames each save; a disk-dirty flag to skip
    // unchanged frames in place is deferred to Phase 3.)
    // Walk ALL layers (incl. nested in groups/clips) so nested raster keyframes
    // are persisted too, and so `live_media` covers them — matching the load path,
    // which arms `needs_fault_in` recursively. Top-level-only projects are unaffected.
    let mut raster_count = 0usize;
    for layer in document.all_layers() {
        if let crate::layer::AnyLayer::Raster(rl) = layer {
            for kf in &rl.keyframes {
                if !kf.raw_pixels.is_empty() {
                    let img =
                        crate::brush_engine::image_from_raw(kf.raw_pixels.clone(), kf.width, kf.height);
                    match crate::brush_engine::encode_png(&img) {
                        Ok(png_bytes) => {
                            txn.put_media_packed(
                                kf.id,
                                MediaKind::Raster,
                                "png",
                                &png_bytes,
                                MediaMeta {
                                    width: Some(kf.width),
                                    height: Some(kf.height),
                                    ..Default::default()
                                },
                            )?;
                            live_media.insert(kf.id);
                            raster_count += 1;
                        }
                        Err(e) => eprintln!("⚠️ [SAVE_BEAM] Failed to encode raster {}: {}", kf.id, e),
                    }
                } else if txn.media_exists(kf.id)? {
                    // Pixels not resident but already stored — keep the row.
                    live_media.insert(kf.id);
                }
            }
        }
    }

    // --- video thumbnail packs -> media rows (opaque LBTN blob), keyed by a
    //     sentinel-derived id from the video clip id ---
    for clip_id in document.video_clips.keys() {
        let tn_id = thumbnail_media_id(*clip_id);
        if let Some(blob) = thumbnail_blobs.get(clip_id) {
            txn.put_media_packed(tn_id, MediaKind::Thumbnail, "lbtn", blob, MediaMeta::default())?;
            live_media.insert(tn_id);
        } else if txn.media_exists(tn_id)? {
            // Not regenerated this session — keep the stored pack.
            live_media.insert(tn_id);
        }
    }

    // --- orphan cleanup: drop media for removed clips/keyframes ---
    let removed = txn.retain_media(&live_media)?;

    // --- project.json + meta ---
    let beam_project = BeamProject {
        version: BEAM_VERSION.to_string(),
        created: created.clone(),
        modified: now.clone(),
        ui_state: document.clone(),
        audio_backend: SerializedAudioBackend {
            sample_rate: 48000, // TODO: Get from audio engine
            project: audio_project.clone(),
            audio_pool_entries: modified_entries,
            layer_to_track_map: layer_to_track_map.clone(),
        },
    };
    let json = serde_json::to_string(&beam_project)
        .map_err(|e| format!("JSON serialization failed: {}", e))?;
    txn.set_project_json(&json)?;
    txn.set_meta("version", BEAM_VERSION)?;
    txn.set_meta("created", &created)?;
    txn.set_meta("modified", &now)?;
    txn.commit()?;

    // Close the connection before renaming (required on Windows; harmless elsewhere).
    drop(archive);
    if !in_place {
        std::fs::rename(&tmp_path, path)
            .map_err(|e| format!("Failed to finalize {:?}: {}", path, e))?;
    }

    eprintln!(
        "📊 [SAVE_BEAM] ✅ Saved {} audio + {} raster media, {} orphans removed, in {:.2}ms",
        audio_pool_entries.len(),
        raster_count,
        removed,
        fn_start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Load a project from a `.beam` file.
///
/// Detects the container format: SQLite (current) or legacy ZIP, and dispatches
/// accordingly. Both produce an identical [`LoadedProject`].
pub fn load_beam(path: &Path) -> Result<LoadedProject, String> {
    if BeamArchive::is_sqlite(path) {
        load_beam_sqlite(path)
    } else {
        load_beam_zip_legacy(path)
    }
}

/// Load a project from a SQLite `.beam` container.
///
/// Phase 0 reconstitutes packed audio into each entry's `embedded_data` so the
/// existing (full-decode) audio pool loader keeps working unchanged; Phase 1b
/// replaces this with streaming reads via `BlobReader`.
fn load_beam_sqlite(path: &Path) -> Result<LoadedProject, String> {
    let fn_start = std::time::Instant::now();
    eprintln!("📊 [LOAD_BEAM] Starting load_beam() (SQLite container)...");

    let archive = BeamArchive::open(path)?;
    let json = archive.get_project_json()?;
    let beam_project: BeamProject = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to deserialize project.json: {}", e))?;

    if beam_project.version != BEAM_VERSION {
        return Err(format!(
            "Unsupported file version: {} (expected {})",
            beam_project.version, BEAM_VERSION
        ));
    }

    let mut document = beam_project.ui_state;
    document.tempo_map_mut().rebuild_seconds();
    let mut audio_project = beam_project.audio_backend.project;
    audio_project
        .rebuild_audio_graphs(DEFAULT_BUFFER_SIZE)
        .map_err(|e| format!("Failed to rebuild audio graphs: {}", e))?;
    let layer_to_track_map = beam_project.audio_backend.layer_to_track_map;

    // For each packed audio item: stream it (leave `embedded_data` empty so the
    // pool builds a Compressed placeholder backed by the blob factory) when it's a
    // recognized audio codec; otherwise fall back to the legacy reconstitution
    // (whole bytes → base64 → decode), which still covers video-container audio
    // tracks symphonia can't stream and any unknown formats.
    let mut restored_entries = Vec::with_capacity(beam_project.audio_backend.audio_pool_entries.len());
    for entry in &beam_project.audio_backend.audio_pool_entries {
        let mut e = entry.clone();
        if let Some(id) = entry.media_id.as_ref().and_then(|s| Uuid::parse_str(s).ok()) {
            match archive.media_info(id) {
                Ok(Some(info)) => {
                    if is_streamable_audio_codec(&info.codec) {
                        // Stream: keep media_id, no embedded bytes. The engine opens
                        // the packed blob via the factory at activation time.
                        e.embedded_data = None;
                        e.relative_path = None;
                    } else {
                        match archive.read_media_full(id) {
                            Ok(bytes) => {
                                e.embedded_data = Some(daw_backend::audio::pool::EmbeddedAudioData {
                                    data_base64: BASE64_STANDARD.encode(&bytes),
                                    format: info.codec,
                                });
                                e.relative_path = None;
                            }
                            Err(err) => eprintln!("⚠️ [LOAD_BEAM] Failed to read audio media {}: {}", id, err),
                        }
                    }
                }
                Ok(None) => eprintln!("⚠️ [LOAD_BEAM] Audio media {} missing from archive", id),
                Err(err) => eprintln!("⚠️ [LOAD_BEAM] media_info({}) failed: {}", id, err),
            }
        }

        // Restore this entry's persisted waveform pyramid, if present — avoids
        // re-decoding the source media just to redraw the overview.
        let wf_id = waveform_media_id(entry.pool_index);
        if let Ok(Some(_)) = archive.media_info(wf_id) {
            match archive.read_media_full(wf_id) {
                Ok(bytes) => e.waveform_blob = Some(bytes),
                Err(err) => eprintln!("⚠️ [LOAD_BEAM] Failed to read waveform {}: {}", wf_id, err),
            }
        }

        restored_entries.push(e);
    }

    // Raster keyframes are NOT eagerly decoded (Phase 3 paging): `raw_pixels` stays
    // empty and is faulted in on demand from the container's `Raster` rows via the
    // editor's `RasterStore` (keyed by `kf.id`). Loading a big paint project is now
    // instant and only the resident window lives in RAM. Mark every keyframe
    // `needs_fault_in` (recursively, incl. nested layers) so the renderer requests a
    // page-in; a freshly-created keyframe stays `false` (blank-resident, nothing to load).
    let mut raster_load_count = 0usize;
    for layer in document.all_layers_mut() {
        if let crate::layer::AnyLayer::Raster(rl) = layer {
            for kf in &mut rl.keyframes {
                kf.needs_fault_in = true;
                raster_load_count += 1;
            }
        }
    }

    // Missing external files (referenced entries whose file no longer exists).
    let project_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let missing_files: Vec<MissingFileInfo> = restored_entries
        .iter()
        .enumerate()
        .filter_map(|(idx, entry)| {
            if entry.embedded_data.is_none() && entry.media_id.is_none() {
                if let Some(rel) = entry.relative_path.as_ref() {
                    let full = if Path::new(rel).is_absolute() {
                        PathBuf::from(rel)
                    } else {
                        project_dir.join(rel)
                    };
                    if !full.exists() {
                        return Some(MissingFileInfo {
                            pool_index: idx,
                            original_path: full,
                            file_type: MediaFileType::Audio,
                        });
                    }
                }
            }
            None
        })
        .collect();

    // Persisted video thumbnail packs (opaque LBTN blobs), keyed by clip id. The
    // editor decodes + inserts them and skips regeneration for these clips.
    let mut thumbnail_blobs = std::collections::HashMap::new();
    for clip_id in document.video_clips.keys() {
        let tn_id = thumbnail_media_id(*clip_id);
        if let Ok(Some(info)) = archive.media_info(tn_id) {
            if info.kind == MediaKind::Thumbnail {
                match archive.read_media_full(tn_id) {
                    Ok(bytes) => { thumbnail_blobs.insert(*clip_id, bytes); }
                    Err(e) => eprintln!("⚠️ [LOAD_BEAM] Failed to read thumbnails for {}: {}", clip_id, e),
                }
            }
        }
    }

    eprintln!(
        "📊 [LOAD_BEAM] ✅ Loaded {} audio entries, {} raster frames, {} thumbnail packs in {:.2}ms",
        restored_entries.len(),
        raster_load_count,
        thumbnail_blobs.len(),
        fn_start.elapsed().as_secs_f64() * 1000.0
    );

    Ok(LoadedProject {
        document,
        audio_project,
        layer_to_track_map,
        audio_pool_entries: restored_entries,
        thumbnail_blobs,
        missing_files,
    })
}

/// Load a project from a legacy ZIP `.beam` archive (pre-SQLite format).
/// Retained for backward compatibility; saving converts to SQLite.
fn load_beam_zip_legacy(path: &Path) -> Result<LoadedProject, String> {
    let fn_start = std::time::Instant::now();
    eprintln!("📊 [LOAD_BEAM] Starting load_beam() (legacy ZIP)...");

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
        thumbnail_blobs: std::collections::HashMap::new(), // legacy ZIP has no thumbnail packs
        missing_files,
    })
}
