//! Sample filename parsing and folder scanning for MultiSampler import.
//!
//! Handles various naming conventions found in sample libraries:
//! - Note-octave: `a#2`, `C4`, `Gb3` (Horns, Philharmonia)
//! - Octave_note: `2_A`, `3_Gb`, `4_Bb` (NoBudgetOrch)
//! - Dynamic velocity markers: `ff`, `mp`, `p`, `f`
//! - Round-robin variants: `rr1`, `rr2`, or `_2` suffix
//! - Loop hints from filename (`-loop`, `sus`) and folder path (`Sustain/`, `Pizzicato/`)

use std::path::{Path, PathBuf};
use std::collections::HashMap;
use daw_backend::audio::node_graph::nodes::LoopMode;

// ─── Audio file extensions ───────────────────────────────────────────────────

const AUDIO_EXTENSIONS: &[&str] = &["wav", "aif", "aiff", "flac", "mp3", "ogg"];

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

// ─── Note name ↔ MIDI conversion ─────────────────────────────────────────────

/// Parse a note letter + optional accidental into a semitone offset (0=C, 11=B).
/// Returns (semitone, chars_consumed).
fn parse_note_letter(s: &str) -> Option<(u8, usize)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let letter = bytes[0].to_ascii_uppercase();
    let base = match letter {
        b'C' => 0,
        b'D' => 2,
        b'E' => 4,
        b'F' => 5,
        b'G' => 7,
        b'A' => 9,
        b'B' => 11,
        _ => return None,
    };
    if bytes.len() > 1 && bytes[1] == b'#' {
        Some(((base + 1) % 12, 2))
    } else if bytes.len() > 1 && bytes[1] == b'b' {
        Some(((base + 11) % 12, 2))
    } else {
        Some((base, 1))
    }
}

/// Format a MIDI note number as a note name (e.g., 60 → "C4").
pub fn midi_to_note_name(midi: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let octave = (midi as i32 / 12) - 1;
    let name = NAMES[midi as usize % 12];
    format!("{}{}", name, octave)
}

// ─── Filename parsing ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum LoopHint {
    Auto,
    OneShot,
    Loop,
}

#[derive(Debug, Clone)]
pub struct ParsedSample {
    pub path: PathBuf,
    pub filename: String,
    pub detected_note: Option<u8>,
    pub velocity_marker: Option<String>,
    pub rr_index: Option<u8>,
    pub is_percussion: bool,
    pub loop_hint: LoopHint,
}

/// Try to find a note-octave pattern like "a#2", "C4", "Gb3" in a token.
/// Returns (midi_note, token_is_consumed) if found.
fn try_note_octave(token: &str) -> Option<u8> {
    let bytes = token.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // Must start with a note letter
    let first = bytes[0].to_ascii_uppercase();
    if !matches!(first, b'A'..=b'G') {
        return None;
    }
    let (semitone, consumed) = parse_note_letter(token)?;
    let rest = &token[consumed..];
    // Next must be an octave digit (optionally preceded by -)
    let octave_str = rest;
    let octave: i8 = octave_str.parse().ok()?;
    if (-1..=9).contains(&octave) {
        let midi = (octave as i32 + 1) * 12 + semitone as i32;
        if (0..=127).contains(&midi) {
            return Some(midi as u8);
        }
    }
    None
}

/// Try to find an octave_note pattern like "2_A", "3_Gb" across two adjacent tokens.
/// token1 is the octave number, token2 is the note name.
fn try_octave_note(octave_token: &str, note_token: &str) -> Option<u8> {
    let octave: i8 = octave_token.parse().ok()?;
    if !(-1..=9).contains(&octave) {
        return None;
    }
    // note_token should be just a note letter + optional accidental, no octave digit
    let (semitone, consumed) = parse_note_letter(note_token)?;
    // Remaining after note should be empty (pure note token)
    if consumed != note_token.len() {
        return None;
    }
    let midi = (octave as i32 + 1) * 12 + semitone as i32;
    if (0..=127).contains(&midi) {
        Some(midi as u8)
    } else {
        None
    }
}

/// Dynamic markings sorted by loudness.
const DYNAMICS: &[&str] = &["ppp", "pp", "p", "mp", "mf", "f", "ff", "fff"];

/// Check if a token is a dynamic marking (exact match, case-insensitive).
fn is_dynamic_marker(token: &str) -> bool {
    let lower = token.to_lowercase();
    DYNAMICS.contains(&lower.as_str())
}

/// Get the sort order for a velocity marker (lower = softer).
pub fn velocity_marker_order(marker: &str) -> u8 {
    let lower = marker.to_lowercase();
    match lower.as_str() {
        "ppp" => 0,
        "pp" => 1,
        "p" => 2,
        "mp" => 3,
        "mf" => 4,
        "f" => 5,
        "ff" => 6,
        "fff" => 7,
        _ => {
            // Numeric markers: v1, v2, v3...
            if let Some(rest) = lower.strip_prefix('v') {
                if let Ok(n) = rest.parse::<u8>() {
                    return n.saturating_add(10); // offset to separate from dynamics
                }
            }
            128 // unknown, sort last
        }
    }
}

/// Check if a token is a round-robin marker like "rr1", "rr2".
fn parse_rr_marker(token: &str) -> Option<u8> {
    let lower = token.to_lowercase();
    lower.strip_prefix("rr")?.parse().ok()
}

/// Detect loop hints from filename tokens and folder path.
fn detect_loop_hint(tokens: &[&str], full_path: &Path) -> LoopHint {
    // Check filename tokens
    for token in tokens {
        let lower = token.to_lowercase();
        if lower == "loop" {
            return LoopHint::Loop;
        }
        if matches!(lower.as_str(), "sus" | "sustain") {
            return LoopHint::Loop;
        }
        if matches!(lower.as_str(), "stac" | "stc" | "staccato" | "piz" | "pizz" | "pizzicato") {
            return LoopHint::OneShot;
        }
    }
    // Check folder path components
    for component in full_path.components() {
        if let std::path::Component::Normal(name) = component {
            let name_lower = name.to_string_lossy().to_lowercase();
            if matches!(name_lower.as_str(), "sustain" | "vibrato" | "tremolo") {
                return LoopHint::Loop;
            }
            if matches!(name_lower.as_str(), "pizzicato" | "staccato") {
                return LoopHint::OneShot;
            }
        }
    }
    LoopHint::Auto
}

/// Tokenize a filename stem on common delimiters.
fn tokenize(stem: &str) -> Vec<&str> {
    stem.split(|c: char| c == '-' || c == '_' || c == '.' || c == ' ')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a sample filename to extract note, velocity, round-robin, and loop hint info.
pub fn parse_sample_filename(path: &Path) -> ParsedSample {
    let filename = path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    // Strip extension to get stem
    let stem = path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| filename.clone());

    let tokens = tokenize(&stem);
    let loop_hint = detect_loop_hint(&tokens, path);

    let mut detected_note: Option<u8> = None;
    let mut velocity_marker: Option<String> = None;
    let mut rr_index: Option<u8> = None;
    let mut note_token_indices: Vec<usize> = Vec::new();

    // Pass 1: Find notes using note-octave format (e.g., "a#2", "C4")
    // Use last match as it's most reliable
    for (i, token) in tokens.iter().enumerate() {
        if let Some(midi) = try_note_octave(token) {
            detected_note = Some(midi);
            note_token_indices.clear();
            note_token_indices.push(i);
        }
    }

    // Pass 2: If no note-octave found, try octave_note format (e.g., "2" + "A", "3" + "Gb")
    if detected_note.is_none() && tokens.len() >= 2 {
        for i in 0..tokens.len() - 1 {
            if let Some(midi) = try_octave_note(tokens[i], tokens[i + 1]) {
                detected_note = Some(midi);
                note_token_indices.clear();
                note_token_indices.push(i);
                note_token_indices.push(i + 1);
            }
        }
    }

    // Pass 3: Find velocity markers and round-robin
    for (i, token) in tokens.iter().enumerate() {
        if note_token_indices.contains(&i) {
            continue;
        }

        // Round-robin: "rr1", "rr2"
        if let Some(rr) = parse_rr_marker(token) {
            rr_index = Some(rr);
            continue;
        }

        // Dynamic markers: "ff", "mp", "p", "f" etc. (must be exact token match)
        if is_dynamic_marker(token) {
            velocity_marker = Some(token.to_lowercase());
            continue;
        }

        // Numeric velocity: "v1", "v2"
        let lower = token.to_lowercase();
        if lower.starts_with('v') && lower[1..].parse::<u8>().is_ok() {
            velocity_marker = Some(lower);
            continue;
        }
    }

    // Pass 4: For octave_note format, check if trailing bare number after note is RR variant
    // e.g., "5_C_2" → tokens ["5", "C", "2"] — "2" is RR, not a note
    if detected_note.is_some() && rr_index.is_none() && note_token_indices.len() == 2 {
        let after_note = note_token_indices[1] + 1;
        if after_note < tokens.len() {
            let candidate = tokens[after_note];
            // If it's a bare small number and NOT a dynamic marker, treat as RR
            if let Ok(n) = candidate.parse::<u8>() {
                if n >= 1 && n <= 20 && !is_dynamic_marker(candidate) {
                    rr_index = Some(n);
                }
            }
        }
    }

    // Pass 5: Check for suffix velocity in octave_note format
    // e.g., "3_A_f.wav" → the "f" after note could be velocity
    // Only apply if we used octave_note format and velocity wasn't already found
    if velocity_marker.is_none() && note_token_indices.len() == 2 {
        let after_note = note_token_indices[1] + 1;
        if after_note < tokens.len() {
            let candidate = tokens[after_note];
            if is_dynamic_marker(candidate) && rr_index.as_ref().map_or(true, |&rr| {
                // If rr was parsed from this position, don't also treat it as velocity
                after_note < tokens.len() - 1 || rr == 0
            }) {
                velocity_marker = Some(candidate.to_lowercase());
            }
        }
    }

    ParsedSample {
        path: path.to_path_buf(),
        filename,
        detected_note,
        velocity_marker,
        rr_index,
        is_percussion: false, // set later in percussion pass
        loop_hint,
    }
}

// ─── GM Drum Map ─────────────────────────────────────────────────────────────

/// GM drum note assignments for common percussion instrument names.
const GM_DRUM_MAP: &[(&[&str], u8)] = &[
    (&["kick", "bass_drum", "bassdrum", "bdrum"], 36),
    (&["rimshot", "rim"], 37),
    (&["snare"], 38),
    (&["clap", "handclap"], 39),
    (&["hihat", "hi_hat", "hh"], 42),
    (&["tom"], 45),
    (&["crash"], 49),
    (&["ride"], 51),
    (&["cymbal"], 52),
    (&["tamtam", "tam_tam", "gong"], 52),
    (&["tambourine", "tamb"], 54),
    (&["cowbell"], 56),
    (&["bongo"], 60),
    (&["conga"], 63),
    (&["shaker"], 70),
    (&["woodblock"], 76),
    (&["triangle"], 81),
    (&["bar_chimes", "chime", "chimes"], 84),
    (&["castanets"], 85),
];

/// Try to match a filename/path against GM drum instrument names.
fn gm_drum_note(filename: &str, relative_path: &str) -> Option<u8> {
    let search = format!("{}/{}", relative_path, filename).to_lowercase();
    for (names, midi) in GM_DRUM_MAP {
        for name in *names {
            if search.contains(name) {
                return Some(*midi);
            }
        }
    }
    None
}

// ─── Folder scanning ─────────────────────────────────────────────────────────

/// Recursively collect audio files from a folder.
fn collect_audio_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_audio_files(&path, files)?;
        } else if is_audio_file(&path) {
            files.push(path);
        }
    }
    Ok(())
}

/// Scan a folder for audio samples, parse filenames, and apply percussion detection.
pub fn scan_folder(folder_path: &Path) -> std::io::Result<Vec<ParsedSample>> {
    let mut files = Vec::new();
    collect_audio_files(folder_path, &mut files)?;

    let mut samples: Vec<ParsedSample> = files.iter()
        .map(|path| parse_sample_filename(path))
        .collect();

    // Percussion pass: for samples with no detected note, try GM drum mapping
    let mut used_drum_notes: Vec<u8> = Vec::new();
    for sample in &mut samples {
        if sample.detected_note.is_some() {
            continue;
        }
        let relative = sample.path.strip_prefix(folder_path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Some(drum_note) = gm_drum_note(&sample.filename, &relative) {
            // Avoid duplicate drum note assignments — if already taken, offset
            let mut note = drum_note;
            while used_drum_notes.contains(&note) && note < 127 {
                note += 1;
            }
            sample.detected_note = Some(note);
            sample.is_percussion = true;
            used_drum_notes.push(note);
        }
    }

    // For remaining unmapped percussion: assign sequential notes from 36
    let mut next_drum = 36u8;
    for sample in &mut samples {
        if sample.detected_note.is_some() {
            continue;
        }
        // Skip notes already used
        while used_drum_notes.contains(&next_drum) && next_drum < 127 {
            next_drum += 1;
        }
        if next_drum <= 127 {
            sample.detected_note = Some(next_drum);
            sample.is_percussion = true;
            used_drum_notes.push(next_drum);
            next_drum += 1;
        }
    }

    // Sort by note, then velocity, then RR index
    samples.sort_by(|a, b| {
        a.detected_note.cmp(&b.detected_note)
            .then_with(|| {
                let va = a.velocity_marker.as_deref().map(velocity_marker_order).unwrap_or(128);
                let vb = b.velocity_marker.as_deref().map(velocity_marker_order).unwrap_or(128);
                va.cmp(&vb)
            })
            .then_with(|| a.rr_index.cmp(&b.rr_index))
    });

    Ok(samples)
}

// ─── Import layer building ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ImportLayer {
    pub path: PathBuf,
    pub filename: String,
    pub root_key: u8,
    pub key_min: u8,
    pub key_max: u8,
    pub velocity_min: u8,
    pub velocity_max: u8,
    pub enabled: bool,
    pub is_percussion: bool,
}

pub struct FolderScanResult {
    pub layers: Vec<ImportLayer>,
    pub unmapped: Vec<ParsedSample>,
    pub loop_mode: LoopMode,
    pub velocity_markers: Vec<String>,
    pub velocity_ranges: Vec<(String, u8, u8)>,
}

/// Compute auto key ranges for a sorted list of unique MIDI notes.
/// Each note gets the range from midpoint-to-previous to midpoint-to-next.
fn auto_key_ranges(notes: &[u8]) -> Vec<(u8, u8)> {
    if notes.is_empty() {
        return Vec::new();
    }
    if notes.len() == 1 {
        return vec![(0, 127)];
    }
    let mut ranges = Vec::with_capacity(notes.len());
    for i in 0..notes.len() {
        let min = if i == 0 {
            0
        } else {
            // One past the previous note's midpoint boundary, so adjacent ranges
            // don't both claim the midpoint key. The previous note's max is
            // floor((notes[i-1] + notes[i]) / 2); start here at that + 1.
            (((notes[i - 1] as u16 + notes[i] as u16) / 2) + 1) as u8
        };
        let max = if i == notes.len() - 1 {
            127
        } else {
            ((notes[i] as u16 + notes[i + 1] as u16) / 2) as u8
        };
        ranges.push((min, max));
    }
    ranges
}

/// Compute velocity ranges by evenly splitting 0-127 among sorted markers.
fn auto_velocity_ranges(markers: &[String]) -> Vec<(String, u8, u8)> {
    if markers.is_empty() {
        return Vec::new();
    }
    if markers.len() == 1 {
        return vec![(markers[0].clone(), 0, 127)];
    }
    let n = markers.len();
    let step = 128.0 / n as f32;
    markers.iter().enumerate().map(|(i, m)| {
        let min = (i as f32 * step).round() as u8;
        let max = if i == n - 1 { 127 } else { ((i + 1) as f32 * step).round() as u8 - 1 };
        (m.clone(), min, max)
    }).collect()
}

/// Detect global loop mode from all parsed samples' loop hints.
fn detect_global_loop_mode(samples: &[ParsedSample]) -> LoopMode {
    let mut loop_count = 0;
    let mut oneshot_count = 0;
    for s in samples {
        match s.loop_hint {
            LoopHint::Loop => loop_count += 1,
            LoopHint::OneShot => oneshot_count += 1,
            LoopHint::Auto => {}
        }
    }
    if loop_count > oneshot_count {
        LoopMode::Continuous
    } else if oneshot_count > 0 {
        LoopMode::OneShot
    } else {
        LoopMode::OneShot // default when no hints
    }
}

/// Build import layers from parsed samples with auto key ranges and velocity mapping.
pub fn build_import_layers(samples: Vec<ParsedSample>) -> FolderScanResult {
    let loop_mode = detect_global_loop_mode(&samples);
    // Separate mapped vs unmapped
    let mut mapped: Vec<ParsedSample> = Vec::new();
    let mut unmapped: Vec<ParsedSample> = Vec::new();
    for s in samples {
        if s.detected_note.is_some() {
            mapped.push(s);
        } else {
            unmapped.push(s);
        }
    }

    // Collect unique velocity markers (sorted by loudness)
    let mut velocity_markers: Vec<String> = mapped.iter()
        .filter_map(|s| s.velocity_marker.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    velocity_markers.sort_by_key(|m| velocity_marker_order(m));

    let velocity_ranges = auto_velocity_ranges(&velocity_markers);

    // Build velocity lookup: marker → (min, max)
    let vel_map: HashMap<String, (u8, u8)> = velocity_ranges.iter()
        .map(|(m, min, max)| (m.clone(), (*min, *max)))
        .collect();

    // Collect unique notes for auto key range computation
    let mut unique_notes: Vec<u8> = mapped.iter()
        .filter_map(|s| s.detected_note)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    unique_notes.sort();

    let key_ranges = auto_key_ranges(&unique_notes);
    let note_to_range: HashMap<u8, (u8, u8)> = unique_notes.iter()
        .zip(key_ranges.iter())
        .map(|(&note, &range)| (note, range))
        .collect();

    // Build layers
    let layers: Vec<ImportLayer> = mapped.iter().map(|s| {
        let root_key = s.detected_note.unwrap();
        let (key_min, key_max) = note_to_range.get(&root_key).copied().unwrap_or((0, 127));
        let (vel_min, vel_max) = s.velocity_marker.as_ref()
            .and_then(|m| vel_map.get(m))
            .copied()
            .unwrap_or((0, 127));

        ImportLayer {
            path: s.path.clone(),
            filename: s.filename.clone(),
            root_key,
            key_min,
            key_max,
            velocity_min: vel_min,
            velocity_max: vel_max,
            enabled: true,
            is_percussion: s.is_percussion,
        }
    }).collect();

    FolderScanResult {
        layers,
        unmapped,
        loop_mode,
        velocity_markers,
        velocity_ranges,
    }
}

/// Recompute key ranges for layers based on their current root_key values.
/// Only affects enabled, non-percussion layers.
pub fn recalc_key_ranges(layers: &mut [ImportLayer]) {
    let mut unique_notes: Vec<u8> = layers.iter()
        .filter(|l| l.enabled && !l.is_percussion)
        .map(|l| l.root_key)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    unique_notes.sort();

    let ranges = auto_key_ranges(&unique_notes);
    let note_to_range: HashMap<u8, (u8, u8)> = unique_notes.iter()
        .zip(ranges.iter())
        .map(|(&note, &range)| (note, range))
        .collect();

    for layer in layers.iter_mut() {
        if !layer.enabled || layer.is_percussion {
            continue;
        }
        if let Some(&(min, max)) = note_to_range.get(&layer.root_key) {
            layer.key_min = min;
            layer.key_max = max;
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_note_octave() {
        assert_eq!(try_note_octave("C4"), Some(60));
        assert_eq!(try_note_octave("A4"), Some(69));
        assert_eq!(try_note_octave("A#3"), Some(58));
        assert_eq!(try_note_octave("Bb2"), Some(46));
        assert_eq!(try_note_octave("C-1"), Some(0));
        assert_eq!(try_note_octave("G9"), Some(127));
    }

    #[test]
    fn test_note_octave_format() {
        // Horns: horns-sus-ff-a#2-PB-loop.wav
        let p = parse_sample_filename(
            Path::new("/samples/horns-sus-ff-a#2-PB-loop.wav"),
        );
        assert_eq!(p.detected_note, Some(46)); // A#2
        assert_eq!(p.velocity_marker, Some("ff".to_string()));
        assert_eq!(p.loop_hint, LoopHint::Loop);

        // Philharmonia: viola_A#3-staccato-rr1-PB.wav
        let p = parse_sample_filename(
            Path::new("/samples/viola_A#3-staccato-rr1-PB.wav"),
        );
        assert_eq!(p.detected_note, Some(58)); // A#3
        assert_eq!(p.rr_index, Some(1));
        assert_eq!(p.loop_hint, LoopHint::OneShot);

        // Bare note: A1.mp3
        let p = parse_sample_filename(
            Path::new("/samples/A1.mp3"),
        );
        assert_eq!(p.detected_note, Some(33)); // A1
    }

    #[test]
    fn test_octave_note_format() {
        // NoBudgetOrch: 2_A-PB.wav
        let p = parse_sample_filename(
            Path::new("/samples/2_A-PB.wav"),
        );
        assert_eq!(p.detected_note, Some(45)); // A2

        // 3_Gb-PB.wav
        let p = parse_sample_filename(
            Path::new("/samples/3_Gb-PB.wav"),
        );
        assert_eq!(p.detected_note, Some(54)); // Gb3

        // 1_Bb.wav
        let p = parse_sample_filename(
            Path::new("/samples/1_Bb.wav"),
        );
        assert_eq!(p.detected_note, Some(34)); // Bb1
    }

    #[test]
    fn test_velocity_suffix() {
        // NoBudgetOrch TubularBells: 3_A_f.wav
        let p = parse_sample_filename(
            Path::new("/samples/3_A_f.wav"),
        );
        assert_eq!(p.detected_note, Some(57)); // A3
        assert_eq!(p.velocity_marker, Some("f".to_string()));

        // 3_C_p.wav
        let p = parse_sample_filename(
            Path::new("/samples/3_C_p.wav"),
        );
        assert_eq!(p.detected_note, Some(48)); // C3
        assert_eq!(p.velocity_marker, Some("p".to_string()));
    }

    #[test]
    fn test_rr_detection() {
        // NoBudgetOrch: 5_C_2-PB.wav → C5, rr2
        let p = parse_sample_filename(
            Path::new("/samples/5_C_2-PB.wav"),
        );
        assert_eq!(p.detected_note, Some(72)); // C5
        assert_eq!(p.rr_index, Some(2));

        // rr marker: viola_A#3-staccato-rr1-PB.wav
        let p = parse_sample_filename(
            Path::new("/samples/viola_A#3-staccato-rr1-PB.wav"),
        );
        assert_eq!(p.rr_index, Some(1));
    }

    #[test]
    fn test_loop_hints_from_folder() {
        let p = parse_sample_filename(
            Path::new("/libs/Cello/Sustain/2_A.wav"),
        );
        assert_eq!(p.loop_hint, LoopHint::Loop);

        let p = parse_sample_filename(
            Path::new("/libs/Cello/Pizzicato/2_A-PB.wav"),
        );
        assert_eq!(p.loop_hint, LoopHint::OneShot);
    }

    #[test]
    fn test_gm_drum_mapping() {
        assert_eq!(gm_drum_note("snare-lh-ff-PB.wav", "Percussion"), Some(38));
        assert_eq!(gm_drum_note("bass_drum-f-PB.wav", "Percussion"), Some(36));
        assert_eq!(gm_drum_note("castanets_mf1-PB.wav", "Percussion"), Some(85));
    }

    #[test]
    fn test_auto_key_ranges() {
        let notes = vec![36, 48, 60, 72];
        let ranges = auto_key_ranges(&notes);
        assert_eq!(ranges[0], (0, 42));   // 36: 0 to (36+48)/2=42
        assert_eq!(ranges[1], (43, 54));  // 48: 43 to (48+60)/2=54
        assert_eq!(ranges[2], (55, 66));  // 60: 55 to (60+72)/2=66
        assert_eq!(ranges[3], (67, 127)); // 72: 67 to 127
    }

    #[test]
    fn test_auto_velocity_ranges() {
        let markers = vec!["p".to_string(), "f".to_string()];
        let ranges = auto_velocity_ranges(&markers);
        assert_eq!(ranges[0], ("p".to_string(), 0, 63));
        assert_eq!(ranges[1], ("f".to_string(), 64, 127));
    }

    #[test]
    fn test_velocity_marker_order() {
        assert!(velocity_marker_order("p") < velocity_marker_order("f"));
        assert!(velocity_marker_order("pp") < velocity_marker_order("mp"));
        assert!(velocity_marker_order("mf") < velocity_marker_order("ff"));
        assert!(velocity_marker_order("v1") < velocity_marker_order("v2"));
    }
}
