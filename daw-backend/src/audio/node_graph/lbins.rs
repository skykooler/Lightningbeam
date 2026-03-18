/// Load and save `.lbins` instrument bundle files.
///
/// A `.lbins` file is a ZIP archive with the following layout:
///
/// ```
/// instrument.lbins  (ZIP)
/// ├── instrument.json       ← GraphPreset JSON (existing schema)
/// ├── samples/
/// │   ├── kick.wav
/// │   └── snare.flac
/// └── models/
///     └── amp.nam
/// ```
///
/// All asset paths in `instrument.json` are ZIP-relative
/// (e.g. `"samples/kick.wav"`, `"models/amp.nam"`).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use crate::audio::node_graph::preset::{GraphPreset, SampleData};

/// Load a `.lbins` file.
///
/// Returns the deserialized `GraphPreset` together with a map of all
/// non-JSON entries keyed by their ZIP-relative path (e.g. `"samples/kick.wav"`).
pub fn load_lbins(path: &Path) -> Result<(GraphPreset, HashMap<String, Vec<u8>>), String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open .lbins file: {}", e))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to read ZIP archive: {}", e))?;

    // Read instrument.json first
    let preset_json = {
        let mut entry = archive
            .by_name("instrument.json")
            .map_err(|_| "Missing instrument.json in .lbins archive".to_string())?;
        let mut buf = String::new();
        entry
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read instrument.json: {}", e))?;
        buf
    };

    let preset = GraphPreset::from_json(&preset_json)
        .map_err(|e| format!("Failed to parse instrument.json: {}", e))?;

    // Read all other entries into memory
    let mut assets: HashMap<String, Vec<u8>> = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry {}: {}", i, e))?;

        let entry_name = entry.name().to_string();
        if entry_name == "instrument.json" || entry.is_dir() {
            continue;
        }

        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Failed to read {}: {}", entry_name, e))?;

        assets.insert(entry_name, bytes);
    }

    Ok((preset, assets))
}

/// Save a preset to a `.lbins` file.
///
/// Asset paths in `preset` are rewritten to ZIP-relative form
/// (`samples/<basename>` or `models/<basename>`).
/// If the path is already ZIP-relative (starts with `samples/` or `models/`)
/// it is used as-is.  Absolute / relative filesystem paths are resolved
/// relative to `asset_base` (typically the directory that contained the
/// original `.json` preset) and then read from disk.
pub fn save_lbins(path: &Path, preset: &GraphPreset, asset_base: Option<&Path>) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Failed to create .lbins file: {}", e))?;

    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // We'll build a rewritten copy of the preset while collecting assets
    let mut rewritten = preset.clone();
    // Map: original path → (zip_path, file_bytes)
    let mut asset_map: HashMap<String, (String, Vec<u8>)> = HashMap::new();

    // Helper: given an original asset path string and a subdirectory ("samples" or "models"),
    // resolve the bytes and return the canonical ZIP-relative path.
    let mut resolve_asset = |orig_path: &str, subdir: &str| -> Result<String, String> {
        // Already a ZIP-relative path — no re-reading needed, caller stored bytes already
        // or the asset will be provided by a prior pass.  Just normalise the subdirectory.
        if orig_path.starts_with(&format!("{}/", subdir)) {
            return Ok(orig_path.to_string());
        }

        let basename = Path::new(orig_path)
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("Cannot determine filename for asset: {}", orig_path))?;

        let zip_path = format!("{}/{}", subdir, basename);

        if !asset_map.contains_key(orig_path) {
            // Resolve to an absolute filesystem path
            let fs_path = if Path::new(orig_path).is_absolute() {
                std::path::PathBuf::from(orig_path)
            } else if let Some(base) = asset_base {
                base.join(orig_path)
            } else {
                std::path::PathBuf::from(orig_path)
            };

            let bytes = std::fs::read(&fs_path)
                .map_err(|e| format!("Failed to read asset {}: {}", fs_path.display(), e))?;

            asset_map.insert(orig_path.to_string(), (zip_path.clone(), bytes));
        }

        Ok(zip_path)
    };

    // Rewrite paths in all nodes
    for node in &mut rewritten.nodes {
        // Sample data paths
        if let Some(ref mut sample_data) = node.sample_data {
            match sample_data {
                SampleData::SimpleSampler { ref mut file_path, .. } => {
                    if let Some(ref orig) = file_path.clone() {
                        if !orig.is_empty() {
                            match resolve_asset(orig, "samples") {
                                Ok(zip_path) => *file_path = Some(zip_path),
                                Err(e) => eprintln!("Warning: {}", e),
                            }
                        }
                    }
                }
                SampleData::MultiSampler { ref mut layers } => {
                    for layer in layers.iter_mut() {
                        if let Some(ref orig) = layer.file_path.clone() {
                            if !orig.is_empty() {
                                match resolve_asset(orig, "samples") {
                                    Ok(zip_path) => layer.file_path = Some(zip_path),
                                    Err(e) => eprintln!("Warning: {}", e),
                                }
                            }
                        }
                    }
                }
            }
        }

        // NAM model path
        if let Some(ref orig) = node.nam_model_path.clone() {
            if !orig.starts_with("bundled:") && !orig.is_empty() {
                match resolve_asset(orig, "models") {
                    Ok(zip_path) => node.nam_model_path = Some(zip_path),
                    Err(e) => eprintln!("Warning: {}", e),
                }
            }
        }
    }

    // Write all collected assets to the ZIP
    for (_, (zip_path, bytes)) in &asset_map {
        zip.start_file(zip_path, options)
            .map_err(|e| format!("Failed to start ZIP entry {}: {}", zip_path, e))?;
        zip.write_all(bytes)
            .map_err(|e| format!("Failed to write {}: {}", zip_path, e))?;
    }

    // Write instrument.json last (after assets so paths are already rewritten)
    let json = rewritten
        .to_json()
        .map_err(|e| format!("Failed to serialize preset: {}", e))?;

    zip.start_file("instrument.json", options)
        .map_err(|e| format!("Failed to start instrument.json entry: {}", e))?;
    zip.write_all(json.as_bytes())
        .map_err(|e| format!("Failed to write instrument.json: {}", e))?;

    zip.finish()
        .map_err(|e| format!("Failed to finalize ZIP: {}", e))?;

    Ok(())
}
