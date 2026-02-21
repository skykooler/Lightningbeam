#!/usr/bin/env python3
"""Build non-orchestral factory instrument presets.

Sources:
  - Acoustic Guitar: University of Iowa MIS (unrestricted license)
  - Bass Guitar: Karoryfer Growlybass CC0 (public domain)
  - Drum Kit: Salamander Drumkit (public domain)

Usage:
    python3 scripts/build_non_orchestral.py
    # or with anaconda (needed for aubio):
    ~/anaconda3/bin/python3 scripts/build_non_orchestral.py
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

# Try to import aubio (needed for guitar splitting)
try:
    import aubio
    HAS_AUBIO = True
except ImportError:
    HAS_AUBIO = False
    print("WARNING: aubio not installed — guitar splitting will be skipped")
    print("  Install with: pip install aubio")

SAMPLES_DIR = Path.home() / "Downloads" / "non-orchestral-samples"
INSTRUMENTS_DIR = Path(__file__).parent.parent / "src" / "assets" / "instruments"

NOTE_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
FLAT_TO_SHARP = {'Db': 'C#', 'Eb': 'D#', 'Gb': 'F#', 'Ab': 'G#', 'Bb': 'A#'}
NOTE_MAP = {
    'c': 0, 'c#': 1, 'db': 1, 'd': 2, 'd#': 3, 'eb': 3,
    'e': 4, 'f': 5, 'f#': 6, 'gb': 6, 'g': 7, 'g#': 8, 'ab': 8,
    'a': 9, 'a#': 10, 'bb': 10, 'b': 11,
}


def note_to_midi(note_name: str, octave: int) -> int:
    return (octave + 1) * 12 + NOTE_MAP[note_name.lower()]


def midi_to_name(midi: int) -> str:
    return f"{NOTE_NAMES[midi % 12]}{midi // 12 - 1}"


def parse_note_str(n: str) -> int:
    """Parse 'E2', 'Bb5', 'C#3' etc to MIDI number."""
    if len(n) >= 3 and n[1] in 'b#':
        name, oct = n[:2], int(n[2:])
        name = FLAT_TO_SHARP.get(name, name)
    else:
        name, oct = n[0], int(n[1:])
    return (oct + 1) * 12 + NOTE_NAMES.index(name)


def convert_to_mp3(input_path: Path, mp3_path: Path, bitrate: str = '192k'):
    """Convert any audio to normalized MP3."""
    mp3_path.parent.mkdir(parents=True, exist_ok=True)
    if mp3_path.exists():
        return
    subprocess.run([
        'ffmpeg', '-i', str(input_path),
        '-af', 'loudnorm=I=-16:TP=-1:LRA=11',
        '-ar', '44100', '-ab', bitrate,
        '-y', '-loglevel', 'error',
        str(mp3_path)
    ], check=True)


def extract_segment(input_path: Path, output_path: Path, start: float, end: float,
                    bitrate: str = '192k'):
    """Extract a time segment from audio and convert to normalized MP3."""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if output_path.exists():
        return
    duration = end - start
    subprocess.run([
        'ffmpeg', '-ss', str(start), '-i', str(input_path),
        '-t', str(duration),
        '-af', 'loudnorm=I=-16:TP=-1:LRA=11',
        '-ar', '44100', '-ab', bitrate,
        '-y', '-loglevel', 'error',
        str(output_path)
    ], check=True)


def compute_key_ranges(layers: list[dict]) -> list[dict]:
    if not layers:
        return layers
    layers.sort(key=lambda l: l['root_key'])
    for i, layer in enumerate(layers):
        if i == 0:
            layer['key_min'] = 0
        else:
            midpoint = (layers[i-1]['root_key'] + layer['root_key']) // 2 + 1
            layer['key_min'] = midpoint
            layers[i-1]['key_max'] = midpoint - 1
        if i == len(layers) - 1:
            layer['key_max'] = 127
    return layers


def make_preset(name: str, description: str, author: str, tags: list[str],
                layers: list[dict], attack: float = 0.01, release: float = 0.3) -> dict:
    return {
        "metadata": {
            "name": name,
            "description": description,
            "author": author,
            "version": 1,
            "tags": tags
        },
        "midi_targets": [0],
        "output_node": 2,
        "nodes": [
            {
                "id": 0, "node_type": "MidiInput", "name": "MIDI In",
                "parameters": {}, "position": [100.0, 100.0]
            },
            {
                "id": 1, "node_type": "MultiSampler", "name": f"{name} Sampler",
                "parameters": {"0": 1.0, "1": attack, "2": release, "3": 0.0},
                "sample_data": {"type": "multi_sampler", "layers": layers},
                "position": [350.0, 0.0]
            },
            {
                "id": 2, "node_type": "AudioOutput", "name": "Out",
                "parameters": {}, "position": [700.0, 100.0]
            }
        ],
        "connections": [
            {"from_node": 0, "from_port": 0, "to_node": 1, "to_port": 0},
            {"from_node": 1, "from_port": 0, "to_node": 2, "to_port": 0}
        ]
    }


# ============================================================
# ACOUSTIC GUITAR (University of Iowa MIS)
# ============================================================

def detect_onsets(fpath: str, threshold: float = 0.3, minioi: float = 2.0,
                  method: str = "default") -> list[float]:
    """Detect note onsets in an audio file using aubio."""
    src = aubio.source(fpath, 44100, 512)
    onset_det = aubio.onset(method, 1024, 512, 44100)
    onset_det.set_threshold(threshold)
    onset_det.set_minioi_s(minioi)
    onsets = []
    while True:
        samples, read = src()
        if onset_det(samples):
            onsets.append(onset_det.get_last_s())
        if read < 512:
            break
    if not onsets or onsets[0] > 1.0:
        onsets.insert(0, 0.0)
    return onsets


def get_file_duration(fpath: str) -> float:
    """Get audio file duration in seconds."""
    result = subprocess.run(
        ['ffprobe', '-v', 'error', '-show_entries', 'format=duration',
         '-of', 'default=noprint_wrappers=1:nokey=1', fpath],
        capture_output=True, text=True)
    return float(result.stdout.strip())


# Preferred string for each MIDI note range (avoids duplicates across strings)
GUITAR_STRING_RANGES = {
    'sulE':  (40, 49),  # E2-C#3
    'sulA':  (50, 54),  # D3-F#3
    'sulD':  (55, 58),  # G3-A#3
    'sulG':  (59, 63),  # B3-D#4
    'sulB':  (64, 68),  # E4-G#4
    'sul_E': (69, 83),  # A4-B5
}


def build_guitar():
    """Split Iowa MIS guitar chromatic scales into individual notes and build preset."""
    if not HAS_AUBIO:
        print("  SKIPPED (aubio required)")
        return

    guitar_dir = SAMPLES_DIR / "iowa-guitar" / "extracted" / "1644stereo"
    if not guitar_dir.exists():
        print(f"  ERROR: Guitar samples not found at {guitar_dir}")
        return

    out_dir = INSTRUMENTS_DIR / "guitar" / "acoustic-guitar"
    samples_dir = out_dir / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)

    # Process each dynamic level
    DYNAMICS = {'pp': (0, 42), 'mf': (43, 95), 'ff': (96, 127)}
    all_layers = []

    for dyn, (vel_min, vel_max) in DYNAMICS.items():
        print(f"  Processing {dyn} dynamics...")
        # Use lower threshold for pp to catch quiet onsets
        threshold = 0.2 if dyn == 'pp' else 0.3

        for fname in sorted(os.listdir(guitar_dir)):
            if not fname.endswith('.aif'):
                continue
            parts = fname.replace('.aif', '').split('.')
            if parts[1] != dyn:
                continue

            string = parts[2]
            note_range_str = parts[3]

            # Parse note range
            m = re.match(r'([A-G][b#]?\d)([A-G][b#]?\d)', note_range_str)
            if m:
                file_lo = parse_note_str(m.group(1))
                file_hi = parse_note_str(m.group(2))
            else:
                file_lo = file_hi = parse_note_str(note_range_str)

            # Check overlap with preferred range for this string
            pref_lo, pref_hi = GUITAR_STRING_RANGES.get(string, (0, 0))
            overlap_lo = max(file_lo, pref_lo)
            overlap_hi = min(file_hi, pref_hi)
            if overlap_lo > overlap_hi:
                continue  # No notes needed from this file

            fpath = str(guitar_dir / fname)
            total_notes = file_hi - file_lo + 1

            if total_notes == 1:
                # Single note file
                mp3_name = f"{midi_to_name(file_lo)}_{dyn}.mp3"
                print(f"    {fname} -> {mp3_name}")
                convert_to_mp3(Path(fpath), samples_dir / mp3_name)
                all_layers.append({
                    "file_path": f"samples/{mp3_name}",
                    "root_key": file_lo,
                    "velocity_min": vel_min,
                    "velocity_max": vel_max,
                })
                continue

            # Multi-note file: detect onsets and split
            onsets = detect_onsets(fpath, threshold=threshold)
            duration = get_file_duration(fpath)

            if len(onsets) != total_notes:
                # Try progressively different thresholds and methods
                found = False
                for method in ["default", "specflux"]:
                    for t in [0.1, 0.15, 0.2, 0.5, 0.8, 1.0]:
                        onsets = detect_onsets(fpath, threshold=t, method=method)
                        if len(onsets) == total_notes:
                            found = True
                            break
                    if found:
                        break
                if not found:
                    print(f"    SKIPPING {fname} (no threshold/method gives {total_notes} onsets)")
                    continue

            # Extract each needed note
            for note_idx in range(total_notes):
                midi = file_lo + note_idx
                if midi < overlap_lo or midi > overlap_hi:
                    continue  # Not in our preferred range

                start = onsets[note_idx]
                end = onsets[note_idx + 1] if note_idx + 1 < len(onsets) else duration
                # Trim to max 8 seconds per note (plenty for guitar decay)
                end = min(end, start + 8.0)

                mp3_name = f"{midi_to_name(midi)}_{dyn}.mp3"
                print(f"    {fname} [{note_idx}] -> {mp3_name} ({start:.2f}s-{end:.2f}s)")
                extract_segment(Path(fpath), samples_dir / mp3_name, start, end)

                all_layers.append({
                    "file_path": f"samples/{mp3_name}",
                    "root_key": midi,
                    "velocity_min": vel_min,
                    "velocity_max": vel_max,
                })

    # Compute key ranges per velocity group
    vel_groups = {}
    for layer in all_layers:
        vel_key = (layer["velocity_min"], layer["velocity_max"])
        vel_groups.setdefault(vel_key, []).append(layer)

    final_layers = []
    for vel_key, group in vel_groups.items():
        group = compute_key_ranges(group)
        for layer in group:
            layer["velocity_min"] = vel_key[0]
            layer["velocity_max"] = vel_key[1]
        final_layers.extend(group)

    preset = make_preset(
        "Acoustic Guitar",
        "Nylon-string classical guitar (Raimundo 118) with three velocity layers",
        "University of Iowa MIS",
        ["guitar", "acoustic", "nylon", "classical"],
        final_layers,
        attack=0.001, release=0.8,
    )
    preset_path = out_dir / "acoustic-guitar.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    print(f"  -> Wrote {preset_path} ({len(final_layers)} layers)")


# ============================================================
# BASS GUITAR (Karoryfer Growlybass)
# ============================================================

def parse_growlybass_filename(filename: str) -> dict | None:
    """Parse Growlybass naming: note_dyn_rr.wav (e.g. a2_ff_rr1.wav, db2_pp_rr3.wav)"""
    m = re.match(r'([a-g][b#]?)(\d+)_(pp|p|f|ff)_rr(\d+)\.wav', filename.lower())
    if not m:
        return None
    note, octave = m.group(1), int(m.group(2))
    dynamic = m.group(3)
    rr = int(m.group(4))
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note.upper()}{octave}", 'dynamic': dynamic, 'rr': rr}


def build_bass_guitar():
    """Build bass guitar instrument from Karoryfer Growlybass samples."""
    source_dir = SAMPLES_DIR / "growlybass" / "extracted" / "Growlybass" / "sustain"
    if not source_dir.exists():
        print(f"  ERROR: Growlybass samples not found at {source_dir}")
        return

    out_dir = INSTRUMENTS_DIR / "guitar" / "bass-guitar"
    samples_dir = out_dir / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)

    # Growlybass has 4 dynamics (pp, p, f, ff) and 4 round robins each.
    # We'll use round robin 1 only (our MultiSampler doesn't support round robin yet)
    # and map all 4 dynamics to velocity layers.
    DYNAMICS_ORDER = ['pp', 'p', 'f', 'ff']
    dynamics_groups: dict[str, list[dict]] = {}

    for wav in sorted(source_dir.glob("*.wav")):
        parsed = parse_growlybass_filename(wav.name)
        if not parsed:
            print(f"  WARNING: Could not parse {wav.name}")
            continue
        if parsed['rr'] != 1:
            continue  # Only use round robin 1

        dyn = parsed['dynamic']
        mp3_name = f"{parsed['note']}_{dyn}.mp3"
        mp3_path = samples_dir / mp3_name
        print(f"  Converting {wav.name} -> {mp3_name} (MIDI {parsed['midi']})")
        convert_to_mp3(wav, mp3_path)

        layer = {
            "file_path": f"samples/{mp3_name}",
            "root_key": parsed['midi'],
        }
        dynamics_groups.setdefault(dyn, []).append(layer)

    # Assign velocity ranges
    num_levels = len(dynamics_groups)
    vel_step = 128 // num_levels
    dyn_keys = sorted(dynamics_groups.keys(),
                      key=lambda d: DYNAMICS_ORDER.index(d))
    for i, dyn in enumerate(dyn_keys):
        vel_min = i * vel_step
        vel_max = (i + 1) * vel_step - 1 if i < num_levels - 1 else 127
        for layer in dynamics_groups[dyn]:
            layer["velocity_min"] = vel_min
            layer["velocity_max"] = vel_max

    # Compute key ranges per velocity group
    all_layers = []
    for dyn, group in dynamics_groups.items():
        group = compute_key_ranges(group)
        all_layers.extend(group)

    preset = make_preset(
        "Bass Guitar",
        "Electric bass guitar (Squier Jazz) with four velocity layers",
        "Karoryfer Samples (CC0)",
        ["guitar", "bass", "electric"],
        all_layers,
        attack=0.001, release=0.5,
    )
    preset_path = out_dir / "bass-guitar.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    dyn_summary = ", ".join(f"{k}: {len(v)}" for k, v in dynamics_groups.items())
    print(f"  -> Wrote {preset_path} ({len(all_layers)} layers: {dyn_summary})")


# ============================================================
# DRUM KIT (Salamander Drumkit)
# ============================================================

# Salamander uses GM-like drum mapping.
# Files: kick_OH_F_1.wav, snare_OH_FF_1.wav, hihatClosed_OH_P_1.wav, etc.
# OH = overhead mic, F/FF/P/PP/MP/Ghost = dynamics, number = round robin

# GM drum map — maps Salamander drum names to MIDI notes
GM_DRUMS = {
    'kick':           36,  # C2  - Bass Drum 1
    'snare':          38,  # D2  - Acoustic Snare
    'snareOFF':       40,  # E2  - Electric Snare (snares off)
    'snareStick':     37,  # C#2 - Side Stick
    'hihatClosed':    42,  # F#2 - Closed Hi-Hat
    'hihatOpen':      46,  # A#2 - Open Hi-Hat
    'hihatFoot':      44,  # G#2 - Pedal Hi-Hat
    'hiTom':          50,  # D3  - High Tom
    'loTom':          45,  # A2  - Low Tom
    'crash1':         49,  # C#3 - Crash Cymbal 1
    'crash2':         57,  # A3  - Crash Cymbal 2
    'ride1':          51,  # D#3 - Ride Cymbal 1
    'ride1Bell':      53,  # F3  - Ride Bell
    'cowbell':        56,  # G#3 - Cowbell
    'splash1':        55,  # G3  - Splash Cymbal
}


def parse_salamander_filename(filename: str) -> dict | None:
    """Parse Salamander naming: drum_OH_dyn_rr.wav or drum_dyn_rr.wav"""
    # Try with OH mic prefix first
    m = re.match(r'(\w+?)_OH_([A-Za-z]+)_(\d+)\.wav', filename)
    if not m:
        # Some drums (cowbell, bellchime) don't have _OH_
        m = re.match(r'(\w+?)_([A-Z][A-Za-z]*)_(\d+)\.wav', filename)
    if not m:
        return None
    drum, dynamic, rr = m.group(1), m.group(2).lower(), int(m.group(3))
    midi = GM_DRUMS.get(drum)
    if midi is None:
        return None
    return {'midi': midi, 'drum': drum, 'dynamic': dynamic, 'rr': rr}


def build_drum_kit():
    """Build drum kit instrument from Salamander Drumkit samples."""
    # Find the OH (overhead mic) sample directory
    sal_base = SAMPLES_DIR / "salamander-drums"
    source_dir = None
    for candidate in [sal_base / "OH",
                      sal_base / "salamanderDrumkit" / "OH"]:
        if candidate.exists():
            source_dir = candidate
            break

    if source_dir is None:
        for p in sal_base.rglob("OH"):
            if p.is_dir():
                source_dir = p
                break

    if source_dir is None:
        print(f"  ERROR: Salamander OH samples not found under {sal_base}")
        return

    print(f"  Using samples from: {source_dir}")
    out_dir = INSTRUMENTS_DIR / "drums" / "drum-kit"
    samples_dir = out_dir / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)

    # Group by drum type and dynamics
    # We'll use OH (overhead) mic for a natural stereo image
    # Only use round robin 1 to keep size down
    drum_groups: dict[str, dict[str, list]] = {}  # drum -> {dyn: [layers]}

    for wav in sorted(source_dir.glob("*.wav")):
        parsed = parse_salamander_filename(wav.name)
        if not parsed:
            continue
        if parsed['rr'] != 1:
            continue

        drum = parsed['drum']
        dyn = parsed['dynamic']
        mp3_name = f"{drum}_{dyn}.mp3"
        mp3_path = samples_dir / mp3_name
        print(f"  Converting {wav.name} -> {mp3_name} (MIDI {parsed['midi']})")
        convert_to_mp3(wav, mp3_path)

        drum_groups.setdefault(drum, {}).setdefault(dyn, []).append({
            "file_path": f"samples/{mp3_name}",
            "root_key": parsed['midi'],
        })

    # Build layers: each drum piece gets its own MIDI note
    # Dynamics map to velocity layers
    DYNAMICS_ORDER = ['ghost', 'pp', 'p', 'mp', 'mf', 'f', 'ff']
    all_layers = []

    for drum, dyn_map in drum_groups.items():
        dyn_keys = sorted(dyn_map.keys(),
                          key=lambda d: DYNAMICS_ORDER.index(d) if d in DYNAMICS_ORDER else 3)
        num_levels = len(dyn_keys)

        if num_levels == 1:
            for layer in list(dyn_map.values())[0]:
                layer["velocity_min"] = 0
                layer["velocity_max"] = 127
                layer["key_min"] = layer["root_key"]
                layer["key_max"] = layer["root_key"]
                all_layers.append(layer)
        else:
            vel_step = 128 // num_levels
            for i, dyn in enumerate(dyn_keys):
                vel_min = i * vel_step
                vel_max = (i + 1) * vel_step - 1 if i < num_levels - 1 else 127
                for layer in dyn_map[dyn]:
                    layer["velocity_min"] = vel_min
                    layer["velocity_max"] = vel_max
                    layer["key_min"] = layer["root_key"]
                    layer["key_max"] = layer["root_key"]
                    all_layers.append(layer)

    preset = make_preset(
        "Drum Kit",
        "Acoustic drum kit (Salamander) — GM-compatible MIDI mapping",
        "Salamander Drumkit (Public Domain)",
        ["drums", "percussion", "kit", "acoustic"],
        all_layers,
        attack=0.001, release=0.5,
    )
    preset_path = out_dir / "drum-kit.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    print(f"  -> Wrote {preset_path} ({len(all_layers)} layers, {len(drum_groups)} drums)")


# ============================================================
# MAIN
# ============================================================

def main():
    print("=== Building Non-Orchestral Factory Instruments ===\n")

    print("\n[1/3] Acoustic Guitar (University of Iowa MIS)")
    build_guitar()

    print("\n[2/3] Bass Guitar (Karoryfer Growlybass)")
    build_bass_guitar()

    print("\n[3/3] Drum Kit (Salamander Drumkit)")
    build_drum_kit()

    print("\n=== Done! ===")


if __name__ == '__main__':
    main()
