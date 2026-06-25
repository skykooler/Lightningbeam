# Lightningbeam .beam File Inspector

A Python command-line tool to inspect and analyze `.beam` project files.

Handles **both** container formats automatically (detected by the file's magic
bytes): the current **SQLite** `.beam` and the **legacy ZIP** `.beam`. See
[BEAM_FILE_FORMAT.md](./BEAM_FILE_FORMAT.md) for the format specification.

## Features

- **Project Information**: Container type, schema version, metadata, dimensions, framerate, duration, background color
- **Clips Analysis**: List all vector, video, and audio clips
- **Layer Hierarchy**: Display the complete layer tree structure
- **Audio Tracks**: Show all audio/MIDI tracks with their settings
- **Audio Pool**: List all audio files and their storage details
- **Media Store** (SQLite): List the `media` table — kinds, storage, codecs, sizes
- **ZIP Structure** (legacy): Examine the internal ZIP archive structure and compression
- **Extraction**: Extract `project.json` or media files

## Installation

The tool requires Python 3.6+ with no external dependencies (uses only the standard
library; `sqlite3` ships with Python).

```bash
chmod +x beam_inspector.py
```

## Usage

### Show All Information
```bash
./beam_inspector.py project.beam
```

### Show Specific Sections
```bash
# Basic project info only
./beam_inspector.py project.beam --info

# Audio tracks only
./beam_inspector.py project.beam --tracks

# Audio pool entries
./beam_inspector.py project.beam --pool

# Layer hierarchy
./beam_inspector.py project.beam --layers

# Clips summary
./beam_inspector.py project.beam --clips

# Media store (SQLite files): the media table — kinds, storage, codecs, sizes
./beam_inspector.py project.beam --media

# ZIP archive structure (legacy ZIP files only)
./beam_inspector.py project.beam --zip
```

When run with no section flags, the tool shows everything and automatically
picks the media-store view (SQLite) or ZIP-structure view (legacy) for the file.

### Extract Files
```bash
# Print project.json to stdout (pretty-printed)
./beam_inspector.py project.beam --extract-json

# Save project.json to a file
./beam_inspector.py project.beam --extract-json output.json

# Extract all media files to a directory
./beam_inspector.py project.beam --extract-media ./extracted_media
```

## Example Output

```
============================================================
PROJECT INFORMATION
============================================================
Container:     SQLite
Schema Ver:    1
Version:       1.0.0
Created:       2025-12-01T12:00:00Z
Modified:      2025-12-01T12:30:00Z

Project Name:  My Animation
ID:            550e8400-e29b-41d4-a716-446655440000
Dimensions:    1920 x 1080
Framerate:     60.0 fps
Duration:      10.00 seconds
Background:    rgba(255, 255, 255, 255)

Sample Rate:   48000 Hz

============================================================
AUDIO TRACKS
============================================================
Total Tracks:  2

  Track 0: Piano [SOLO]
    Type:      Midi
    Volume:    0.80
    Pan:       0.00
    Instrument: Piano
    Notes:     24

  Track 1: Background Music
    Type:      Audio
    Volume:    0.60
    Pan:       0.00
    Clips:     3

Master Track:
  Volume:      1.00

============================================================
AUDIO POOL
============================================================
Total Entries: 2

  [0] C2.mp3
    Media ID:    e7e555a6-85f1-4faa-bfd4-c6e4b790986a
    Path:        N/A
    Storage:     Packed in DB, mp3, 412.0 KiB
    Channels:    2
    Sample Rate: 44100 Hz

  [1] Background.flac
    Media ID:    a18c0e22-1d3b-4c77-9f0a-2b5e6c4d8e10
    Path:        N/A
    Storage:     Packed in DB, flac, 6.4 MiB
    Channels:    2
    Sample Rate: 48000 Hz
```

## Understanding the Output

### Storage Types
- **Packed in DB**: Audio bytes are chunked into the SQLite `media` table (the entry's `media_id` resolves to a packed row). Current default for most audio.
- **External reference**: File is referenced from the filesystem by `relative_path` (e.g. large media or video audio).
- **Embedded (inline base64)**: Bytes stored directly in `project.json` (`embedded_data`) — legacy/fallback.
- **Embedded (in ZIP)**: Legacy ZIP files only — bytes stored inside the archive under `media/audio/`.
- **Unresolved (missing)**: No packed row, external file, or embedded data — reported as a missing file on load.

### Track Types
- **Audio**: Traditional audio track with clips from the audio pool
- **Midi**: MIDI track with note events and virtual instrument

### Layer Types
Based on the `AnyLayer` enum:
- **Group**: Container for other layers
- **Vector**: Vector graphics layer
- **Video**: Video clip layer
- **Audio**: Audio waveform layer
- **Image**: Raster image layer
- **Text**: Text layer

## Advanced Usage

### Combine with Other Tools
```bash
# Pretty-print and page through JSON
./beam_inspector.py project.beam --extract-json | less

# Search for specific content in JSON
./beam_inspector.py project.beam --extract-json | grep "sample_rate"

# Count total audio files
./beam_inspector.py project.beam --pool | grep "^\[" | wc -l

# Extract and process media files
#   SQLite: writes flat <uuid>.<codec> files; ZIP: preserves media/ paths
./beam_inspector.py project.beam --extract-media /tmp/media
ls -lh /tmp/media/
```

### Scripting
```python
from beam_inspector import BeamInspector
from pathlib import Path

inspector = BeamInspector(Path("project.beam"))
if inspector.load():
    # Access parsed data
    version = inspector.project_data['version']
    tracks = inspector.project_data['audio_backend']['project']['tracks']
    print(f"Found {len(tracks)} tracks in version {version}")
```

## Troubleshooting

### "Error loading .beam file"
- Ensure the file is a valid SQLite database (current) or ZIP archive (legacy)
- Check that `project.json` exists (the `project_json` table for SQLite, or a top-level entry for ZIP)
- Verify the JSON is well-formed

### "File not found"
- Provide the full path to the .beam file
- Check file permissions

### Missing media files
- External references may point to files that don't exist
- Use `--extract-media` to see what's actually in the archive
- Check the `relative_path` values in `--pool` output

## See Also

- [BEAM_FILE_FORMAT.md](./BEAM_FILE_FORMAT.md) - Complete .beam file format specification
- [Lightningbeam Documentation](./README.md) - Main project documentation
