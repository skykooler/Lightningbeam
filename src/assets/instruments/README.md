# Lightningbeam Factory Instruments

This directory contains bundled factory instruments for Lightningbeam DAW.

## Directory Structure

Instruments are organized by category:

```
instruments/
  keyboards/          # Sampled keyboard instruments
    piano/
      samples/        # MP3 audio samples
        A1.mp3
        C1.mp3
        ...
      piano.json      # MultiSampler configuration
    epiano/
      ...
  synthesizers/       # Synthesizer presets (node graph-based)
    bass.json
    lead.json
    pad.json
    ...
  drums/              # Drum kits and percussion
    acoustic_kit/
      ...
```

## Instrument Definition Format

Each instrument is a JSON file that defines a MultiSampler node configuration:

```json
{
  "name": "Instrument Name",
  "description": "Brief description",
  "version": "1.0",
  "node_type": "MultiSampler",
  "parameters": {
    "gain": 1.0,
    "attack": 0.001,
    "release": 0.5,
    "transpose": 0
  },
  "layers": [
    {
      "sample_path": "./samples/C4.mp3",
      "root_note": 60,           // MIDI note number (C4 = 60)
      "key_range": [58, 62],     // MIDI notes [min, max]
      "velocity_range": [0, 127] // Velocity [min, max]
    }
  ]
}
```

## MIDI Note Numbers

- C1 = 24, A1 = 33
- C2 = 36, A2 = 45
- C3 = 48, A3 = 57
- C4 = 60 (middle C), A4 = 69
- C5 = 72, A5 = 81
- C6 = 84, A6 = 93
- C7 = 96, A7 = 105

## Sample Format Guidelines

**Factory Samples (bundled):**
- Format: MP3 (for size efficiency)
- Sample rate: 44.1 kHz recommended
- Bit depth: 16-bit minimum
- Total size: Keep under 50MB per instrument

**User Samples (external):**
- Format: WAV, FLAC, MP3, OGG
- Any sample rate/bit depth supported
- No size restrictions

## Adding New Instruments

1. Create a new directory: `instruments/my-instrument/`
2. Add samples to: `instruments/my-instrument/samples/`
3. Create configuration: `instruments/my-instrument/my-instrument.json`
4. Reference samples with relative paths: `./samples/filename.mp3`

## Loading Instruments

Instruments are loaded via the frontend and can be:
- Dragged into the node graph editor
- Selected from the instrument browser
- Loaded programmatically via the MultiSampler API
