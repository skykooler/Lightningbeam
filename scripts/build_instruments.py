#!/usr/bin/env python3
"""Build factory instrument presets from Virtual Playing Orchestra 3 samples.

Usage:
    python3 scripts/build_instruments.py

Converts WAV samples to MP3 and generates MultiSampler JSON presets.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

VPO = Path.home() / "Downloads" / "Virtual-Playing-Orchestra3" / "libs"
INSTRUMENTS_DIR = Path(__file__).parent.parent / "src" / "assets" / "instruments"

# Note name to semitone offset (within octave)
NOTE_MAP = {
    'c': 0, 'c#': 1, 'db': 1, 'd': 2, 'd#': 3, 'eb': 3,
    'e': 4, 'f': 5, 'f#': 6, 'gb': 6, 'g': 7, 'g#': 8, 'ab': 8,
    'a': 9, 'a#': 10, 'bb': 10, 'b': 11,
}


def note_to_midi(note_name: str, octave: int) -> int:
    """Convert note name + octave to MIDI number. C4 = 60."""
    semitone = NOTE_MAP[note_name.lower()]
    return (octave + 1) * 12 + semitone


def parse_sso_filename(filename: str) -> dict | None:
    """Parse SSO-style: instrument-sus-note-PB-loop.wav (e.g. 1st-violins-sus-a#3.wav)
    Also handles flats: oboe-a#3, basses-sus-d#2, etc.
    """
    m = re.search(r'([a-g][#b]?)(\d+)', filename.lower())
    if not m:
        return None
    note, octave = m.group(1), int(m.group(2))
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note.upper()}{octave}"}


def parse_nbo_filename(filename: str) -> dict | None:
    """Parse NBO-style: octave_note.wav (e.g. 3_Bb-PB-loop.wav)"""
    m = re.match(r'(\d+)_([A-Ga-g][b#]?)', filename)
    if not m:
        return None
    octave, note = int(m.group(1)), m.group(2)
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note}{octave}"}


def parse_nbo_with_dynamics(filename: str) -> dict | None:
    """Parse NBO2-style with dynamics: octave_note_p.wav or octave_note.wav"""
    m = re.match(r'(\d+)_([A-Ga-g][b#]?)(?:_(p|f|mf|ff))?', filename)
    if not m:
        return None
    octave, note = int(m.group(1)), m.group(2)
    dynamic = m.group(3)
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note}{octave}", 'dynamic': dynamic}


def parse_mw_viola_filename(filename: str) -> dict | None:
    """Parse MW-style: Violas_note.wav (e.g. Violas_c4.wav, Violas_d#3.wav)"""
    m = re.search(r'_([a-g][#b]?)(\d+)\.wav', filename.lower())
    if not m:
        return None
    note, octave = m.group(1), int(m.group(2))
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note.upper()}{octave}"}


def parse_mw_horn_filename(filename: str) -> dict | None:
    """Parse MW horn: horns-sus-ff-note-PB-loop.wav or horns-sus-mp-note-PB-loop.wav"""
    # Extract dynamics marker (ff, mp) from filename
    dyn_match = re.search(r'-(ff|mp|mf|p|pp)-', filename.lower())
    dynamic = dyn_match.group(1) if dyn_match else None
    # Extract note
    m = re.search(r'([a-g][#b]?)(\d+)', filename.lower())
    if not m:
        return None
    note, octave = m.group(1), int(m.group(2))
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note.upper()}{octave}", 'dynamic': dynamic}


def parse_vsco_harp_filename(filename: str) -> dict | None:
    """Parse VSCO harp: KSHarp_Note_dyn.wav (e.g. KSHarp_A4_mf.wav)"""
    m = re.search(r'KSHarp_([A-G][b#]?)(\d+)', filename)
    if not m:
        return None
    note, octave = m.group(1), int(m.group(2))
    midi = note_to_midi(note, octave)
    return {'midi': midi, 'note': f"{note}{octave}"}


def convert_wav_to_mp3(wav_path: Path, mp3_path: Path, bitrate: str = '192k'):
    """Convert WAV to MP3 using ffmpeg with peak normalization."""
    mp3_path.parent.mkdir(parents=True, exist_ok=True)
    if mp3_path.exists():
        return  # Skip if already converted
    # Peak-normalize to -1dBFS so all samples have consistent max level.
    # Using dynaudnorm with very gentle settings to avoid changing the
    # character of the sound — just brings everything to the same peak level.
    subprocess.run([
        'ffmpeg', '-i', str(wav_path),
        '-af', 'loudnorm=I=-16:TP=-1:LRA=11',
        '-ar', '44100', '-ab', bitrate,
        '-y', '-loglevel', 'error',
        str(mp3_path)
    ], check=True)


def compute_key_ranges(layers: list[dict]) -> list[dict]:
    """Compute key_min/key_max for each layer by splitting at midpoints between adjacent root notes."""
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


def make_preset(name: str, description: str, tags: list[str], layers: list[dict],
                attack: float = 0.01, release: float = 0.3) -> dict:
    """Generate a complete instrument preset JSON."""
    return {
        "metadata": {
            "name": name,
            "description": description,
            "author": "Virtual Playing Orchestra 3",
            "version": 1,
            "tags": tags
        },
        "midi_targets": [0],
        "output_node": 2,
        "nodes": [
            {
                "id": 0,
                "node_type": "MidiInput",
                "name": "MIDI In",
                "parameters": {},
                "position": [100.0, 100.0]
            },
            {
                "id": 1,
                "node_type": "MultiSampler",
                "name": f"{name} Sampler",
                "parameters": {
                    "0": 1.0,      # gain
                    "1": attack,   # attack
                    "2": release,  # release
                    "3": 0.0       # transpose
                },
                "sample_data": {
                    "type": "multi_sampler",
                    "layers": layers
                },
                "position": [350.0, 0.0]
            },
            {
                "id": 2,
                "node_type": "AudioOutput",
                "name": "Out",
                "parameters": {},
                "position": [700.0, 100.0]
            }
        ],
        "connections": [
            {"from_node": 0, "from_port": 0, "to_node": 1, "to_port": 0},
            {"from_node": 1, "from_port": 0, "to_node": 2, "to_port": 0}
        ]
    }


def build_simple_instrument(name: str, description: str, tags: list[str],
                            source_dir: Path, output_subdir: str,
                            filename_filter=None, parser=parse_sso_filename,
                            attack: float = 0.01, release: float = 0.3,
                            loop: bool = False):
    """Build a single-velocity instrument from a directory of WAV files."""
    out_dir = INSTRUMENTS_DIR / output_subdir
    samples_dir = out_dir / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)

    layers = []
    wav_files = sorted(source_dir.glob("*.wav"))
    for wav in wav_files:
        if filename_filter and not filename_filter(wav.name):
            continue
        parsed = parser(wav.name)
        if not parsed:
            print(f"  WARNING: Could not parse {wav.name}, skipping")
            continue
        mp3_name = f"{parsed['note']}.mp3"
        mp3_path = samples_dir / mp3_name
        print(f"  Converting {wav.name} -> {mp3_name} (MIDI {parsed['midi']})")
        convert_wav_to_mp3(wav, mp3_path)
        layer = {
            "file_path": f"samples/{mp3_name}",
            "root_key": parsed['midi'],
            "velocity_min": 0,
            "velocity_max": 127,
        }
        if loop:
            layer["loop_mode"] = "continuous"
        layers.append(layer)

    layers = compute_key_ranges(layers)
    preset = make_preset(name, description, tags, layers, attack, release)
    preset_path = out_dir / f"{output_subdir.split('/')[-1]}.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    print(f"  -> Wrote {preset_path} ({len(layers)} layers)")
    return layers


def build_dynamics_instrument(name: str, description: str, tags: list[str],
                              source_dir: Path, output_subdir: str,
                              filename_filter=None, parser=parse_nbo_with_dynamics,
                              attack: float = 0.01, release: float = 0.3,
                              loop: bool = False):
    """Build an instrument with velocity layers from dynamics markings."""
    out_dir = INSTRUMENTS_DIR / output_subdir
    samples_dir = out_dir / "samples"
    samples_dir.mkdir(parents=True, exist_ok=True)

    # Group samples by dynamics level
    # Map dynamics markings to velocity ranges (soft to loud)
    DYNAMICS_ORDER = ['pp', 'p', 'mp', 'mf', 'f', 'ff']
    dynamics_groups: dict[str | None, list[dict]] = {}

    wav_files = sorted(source_dir.glob("*.wav"))
    for wav in wav_files:
        if filename_filter and not filename_filter(wav.name):
            continue
        parsed = parser(wav.name)
        if not parsed:
            print(f"  WARNING: Could not parse {wav.name}, skipping")
            continue
        dyn = parsed.get('dynamic')
        suffix = f"_{dyn}" if dyn else ""
        mp3_name = f"{parsed['note']}{suffix}.mp3"
        mp3_path = samples_dir / mp3_name
        print(f"  Converting {wav.name} -> {mp3_name} (MIDI {parsed['midi']}, dyn={dyn})")
        convert_wav_to_mp3(wav, mp3_path)

        layer = {
            "file_path": f"samples/{mp3_name}",
            "root_key": parsed['midi'],
        }
        if loop:
            layer["loop_mode"] = "continuous"
        dynamics_groups.setdefault(dyn, []).append(layer)

    # Determine velocity ranges based on how many dynamics levels exist
    # Treat None (unmarked) as forte — it's the "normal" dynamic
    dyn_keys = sorted(dynamics_groups.keys(),
                      key=lambda d: DYNAMICS_ORDER.index(d) if d and d in DYNAMICS_ORDER else
                      (DYNAMICS_ORDER.index('f') if d is None else 3))
    if len(dyn_keys) == 1:
        # Only one dynamics level — full velocity
        for layer in dynamics_groups[dyn_keys[0]]:
            layer["velocity_min"] = 0
            layer["velocity_max"] = 127
    else:
        num_levels = len(dyn_keys)
        vel_step = 128 // num_levels
        for i, dyn in enumerate(dyn_keys):
            vel_min = i * vel_step
            vel_max = (i + 1) * vel_step - 1 if i < num_levels - 1 else 127
            for layer in dynamics_groups[dyn]:
                layer["velocity_min"] = vel_min
                layer["velocity_max"] = vel_max

    # Compute key ranges separately for each velocity group
    all_layers = []
    for dyn, group in dynamics_groups.items():
        group = compute_key_ranges(group)
        all_layers.extend(group)
    preset = make_preset(name, description, tags, all_layers, attack, release)
    preset_path = out_dir / f"{output_subdir.split('/')[-1]}.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    dyn_summary = ", ".join(f"{k or 'default'}: {len(v)}" for k, v in dynamics_groups.items())
    print(f"  -> Wrote {preset_path} ({len(all_layers)} layers: {dyn_summary})")
    return all_layers


def build_combined_instrument(name: str, description: str, tags: list[str],
                              component_dirs: list[str], output_subdir: str,
                              attack: float = 0.01, release: float = 0.3):
    """Build a combined instrument that references samples from component instruments.

    component_dirs: list of output_subdir paths for component instruments, ordered low to high pitch.
    Splits the keyboard range across them.
    """
    out_dir = INSTRUMENTS_DIR / output_subdir
    out_dir.mkdir(parents=True, exist_ok=True)

    # Load each component's preset to get its layers
    all_component_layers = []
    for comp_dir in component_dirs:
        comp_path = INSTRUMENTS_DIR / comp_dir
        json_files = list(comp_path.glob("*.json"))
        if not json_files:
            print(f"  WARNING: No preset found in {comp_dir}")
            continue
        with open(json_files[0]) as f:
            comp_preset = json.load(f)
        comp_layers = comp_preset["nodes"][1]["sample_data"]["layers"]
        # Adjust file paths to be relative from the combined instrument dir
        rel_prefix = os.path.relpath(comp_path, out_dir)
        for layer in comp_layers:
            layer["file_path"] = f"{rel_prefix}/{layer['file_path']}"
        all_component_layers.extend(comp_layers)

    # Re-sort by root key and recompute ranges across all layers
    # Group by velocity range to handle dynamics separately
    vel_groups = {}
    for layer in all_component_layers:
        vel_key = (layer["velocity_min"], layer["velocity_max"])
        vel_groups.setdefault(vel_key, []).append(layer)

    final_layers = []
    for vel_key, group in vel_groups.items():
        group = compute_key_ranges(group)
        # Preserve the original velocity range
        for layer in group:
            layer["velocity_min"] = vel_key[0]
            layer["velocity_max"] = vel_key[1]
        final_layers.extend(group)

    preset = make_preset(name, description, tags, final_layers, attack, release)
    preset_path = out_dir / f"{output_subdir.split('/')[-1]}.json"
    with open(preset_path, 'w') as f:
        json.dump(preset, f, indent=2)
    print(f"  -> Wrote {preset_path} ({len(final_layers)} layers from {len(component_dirs)} components)")


def main():
    print("=== Building Lightningbeam Factory Instruments from VPO3 ===\n")

    if not VPO.exists():
        print(f"ERROR: VPO3 not found at {VPO}")
        sys.exit(1)

    # --- STRINGS ---

    print("\n[1/14] Violin Section (SSO 1st Violins sustain)")
    build_simple_instrument(
        "Violin Section", "Orchestral violin section with sustained bowing",
        ["strings", "violin", "section", "orchestral"],
        VPO / "SSO" / "Samples" / "1st Violins",
        "strings/violin-section",
        filename_filter=lambda f: 'sus' in f.lower(),
        parser=parse_sso_filename,
        attack=0.05, release=0.4,
        loop=True,
    )

    print("\n[2/14] Viola Section (Mattias-Westlund)")
    build_simple_instrument(
        "Viola Section", "Orchestral viola section with sustained bowing",
        ["strings", "viola", "section", "orchestral"],
        VPO / "Mattias-Westlund" / "ViolaSect" / "Samples",
        "strings/viola-section",
        parser=parse_mw_viola_filename,
        attack=0.05, release=0.4,
        loop=True,
    )

    print("\n[3/14] Cello Section (NBO sustain)")
    build_simple_instrument(
        "Cello Section", "Orchestral cello section with sustained bowing",
        ["strings", "cello", "section", "orchestral"],
        VPO / "NoBudgetOrch" / "CelloSect" / "Sustain",
        "strings/cello-section",
        parser=parse_nbo_filename,
        attack=0.05, release=0.4,
        loop=True,
    )

    print("\n[4/14] Bass Section (SSO sustain)")
    build_simple_instrument(
        "Bass Section", "Orchestral double bass section with sustained bowing",
        ["strings", "bass", "contrabass", "section", "orchestral"],
        VPO / "SSO" / "Samples" / "Basses",
        "strings/bass-section",
        filename_filter=lambda f: 'sus' in f.lower(),
        parser=parse_sso_filename,
        attack=0.08, release=0.5,
        loop=True,
    )

    print("\n[5/14] Harp (VSCO2-CE)")
    build_simple_instrument(
        "Harp", "Concert harp",
        ["strings", "harp", "orchestral"],
        VPO / "VSCO2-CE" / "Strings" / "Harp",
        "strings/harp",
        parser=parse_vsco_harp_filename,
        attack=0.001, release=0.8,
    )

    # --- WOODWINDS ---

    print("\n[6/14] Flute Section (NBO)")
    build_simple_instrument(
        "Flute", "Orchestral flute section",
        ["woodwinds", "flute", "section", "orchestral"],
        VPO / "NoBudgetOrch" / "FluteSect",
        "woodwinds/flute",
        parser=parse_nbo_filename,
        attack=0.03, release=0.3,
        loop=True,
    )

    print("\n[7/14] Oboe (SSO solo)")
    build_simple_instrument(
        "Oboe", "Solo oboe",
        ["woodwinds", "oboe", "solo", "orchestral"],
        VPO / "SSO" / "Samples" / "Oboe",
        "woodwinds/oboe",
        filename_filter=lambda f: f.endswith('.wav') and 'readme' not in f.lower(),
        parser=parse_sso_filename,
        attack=0.02, release=0.25,
        loop=True,
    )

    print("\n[8/14] Clarinet Section (NBO)")
    build_simple_instrument(
        "Clarinet", "Orchestral clarinet section",
        ["woodwinds", "clarinet", "section", "orchestral"],
        VPO / "NoBudgetOrch" / "ClarinetSect" / "Sustain",
        "woodwinds/clarinet",
        parser=parse_nbo_filename,
        attack=0.02, release=0.25,
        loop=True,
    )

    print("\n[9/14] Bassoon (SSO)")
    build_simple_instrument(
        "Bassoon", "Solo bassoon",
        ["woodwinds", "bassoon", "solo", "orchestral"],
        VPO / "SSO" / "Samples" / "Bassoon",
        "woodwinds/bassoon",
        parser=parse_sso_filename,
        attack=0.03, release=0.3,
        loop=True,
    )

    # --- BRASS ---

    print("\n[10/14] Horn Section (Mattias-Westlund, ff + mp dynamics)")
    build_dynamics_instrument(
        "Horn Section", "French horn section with forte and mezzo-piano dynamics",
        ["brass", "horn", "french horn", "section", "orchestral"],
        VPO / "Mattias-Westlund" / "Horns" / "Samples",
        "brass/horn-section",
        parser=parse_mw_horn_filename,
        attack=0.04, release=0.4,
        loop=True,
    )

    print("\n[11/14] Trumpet Section (NBO2 with dynamics)")
    build_dynamics_instrument(
        "Trumpet Section", "Orchestral trumpet section with piano and forte dynamics",
        ["brass", "trumpet", "section", "orchestral"],
        VPO / "NoBudgetOrch2" / "Trumpet" / "TrumpetSect" / "Sustain",
        "brass/trumpet-section",
        attack=0.02, release=0.3,
        loop=True,
    )

    print("\n[12/14] Trombone Section (NBO2 with dynamics)")
    build_dynamics_instrument(
        "Trombone Section", "Orchestral trombone section with piano and forte dynamics",
        ["brass", "trombone", "section", "orchestral"],
        VPO / "NoBudgetOrch2" / "Trombone" / "TromboneSect" / "Sustain",
        "brass/trombone-section",
        attack=0.03, release=0.35,
        loop=True,
    )

    print("\n[13/14] Tuba (SSO sustain)")
    build_simple_instrument(
        "Tuba", "Orchestral tuba",
        ["brass", "tuba", "orchestral"],
        VPO / "SSO" / "Samples" / "Tuba",
        "brass/tuba",
        filename_filter=lambda f: 'sus' in f.lower(),
        parser=parse_sso_filename,
        attack=0.04, release=0.4,
        loop=True,
    )

    # --- PERCUSSION ---

    print("\n[14/14] Timpani (NBO)")
    build_simple_instrument(
        "Timpani", "Orchestral timpani",
        ["percussion", "timpani", "orchestral"],
        VPO / "NoBudgetOrch" / "Timpani",
        "orchestral/timpani",
        parser=lambda f: parse_sso_filename(f),  # Note-octave format like A2-PB.wav
        attack=0.001, release=1.5,
    )

    # --- COMBINED INSTRUMENTS ---

    print("\n[Combined] Strings")
    build_combined_instrument(
        "Strings", "Full string section — auto-selects violin, viola, cello, or bass by pitch range",
        ["strings", "section", "orchestral", "combined"],
        [
            "strings/bass-section",
            "strings/cello-section",
            "strings/viola-section",
            "strings/violin-section",
        ],
        "strings/strings-combined",
        attack=0.05, release=0.4,
    )

    print("\n[Combined] Woodwinds")
    build_combined_instrument(
        "Woodwinds", "Full woodwind section — auto-selects bassoon, clarinet, oboe, or flute by pitch range",
        ["woodwinds", "section", "orchestral", "combined"],
        [
            "woodwinds/bassoon",
            "woodwinds/clarinet",
            "woodwinds/oboe",
            "woodwinds/flute",
        ],
        "woodwinds/woodwinds-combined",
        attack=0.03, release=0.3,
    )

    print("\n[Combined] Brass")
    build_combined_instrument(
        "Brass", "Full brass section — auto-selects tuba, trombone, horn, or trumpet by pitch range",
        ["brass", "section", "orchestral", "combined"],
        [
            "brass/tuba",
            "brass/trombone-section",
            "brass/horn-section",
            "brass/trumpet-section",
        ],
        "brass/brass-combined",
        attack=0.03, release=0.35,
    )

    print("\n=== Done! ===")


if __name__ == '__main__':
    main()
