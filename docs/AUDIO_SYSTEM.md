# Audio System Architecture

This document describes the architecture of Lightningbeam's audio engine (`daw-backend`), including real-time constraints, lock-free design patterns, and how to extend the system with new effects and features.

## Table of Contents

- [Overview](#overview)
- [Architecture](#architecture)
- [Real-Time Constraints](#real-time-constraints)
- [Lock-Free Communication](#lock-free-communication)
- [Audio Processing Pipeline](#audio-processing-pipeline)
- [Adding Effects](#adding-effects)
- [Adding Synthesizers](#adding-synthesizers)
- [MIDI System](#midi-system)
- [Performance Optimization](#performance-optimization)
- [Debugging Audio Issues](#debugging-audio-issues)

## Overview

The `daw-backend` crate is a standalone real-time audio engine designed for:

- **Multi-track audio playback and recording**
- **Real-time audio effects processing**
- **MIDI input/output and sequencing**
- **Modular audio routing** (node graph system)
- **Audio export** (WAV, MP3, AAC)

### Key Features

- Lock-free design for real-time safety
- Cross-platform audio I/O via cpal
- Audio decoding via symphonia (MP3, FLAC, WAV, Ogg, AAC)
- Node-based audio graph processing
- Comprehensive effects library
- Multiple synthesizer types
- Zero-allocation audio thread

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         UI Thread                           │
│  (lightningbeam-editor or other application)                │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  AudioSystem::new() ─────> Creates audio stream             │
│         │                                                   │
│         ├─> command_sender (rtrb::Producer)                 │
│         └─> state_receiver (rtrb::Consumer)                 │
│                                                             │
│  Commands sent:                                             │
│    - Play / Stop / Seek                                     │
│    - Add / Remove tracks                                    │
│    - Load audio files                                       │
│    - Add / Remove effects                                   │
│    - Update parameters                                      │
│                                                             │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       │ Lock-free queues (rtrb)
                       │
┌──────────────────────▼──────────────────────────────────────┐
│                    Audio Thread (Real-Time)                 │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Engine::process(output_buffer)                             │
│    │                                                        │
│    ├─> Receive commands from queue                          │
│    ├─> Update playhead position                             │
│    ├─> For each track:                                      │
│    │     ├─> Read audio samples at playhead                 │
│    │     ├─> Apply effects chain                            │
│    │     └─> Mix to output                                  │
│    ├─> Apply master effects                                 │
│    └─> Write samples to output_buffer                       │
│                                                             │
│  Send state updates back to UI thread                       │
│    - Playhead position                                      │
│    - Meter levels                                           │
│    - Overrun warnings                                       │
│                                                             │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
                 ┌───────────┐
                 │   cpal    │
                 │  (Audio   │
                 │   I/O)    │
                 └───────────┘
                       │
                       ▼
                ┌──────────────┐
                │ Audio Output │
                │  (Speakers)  │
                └──────────────┘
```

### Core Components

#### AudioSystem (`src/lib.rs`)
- Entry point for the audio engine
- Creates the audio stream
- Sets up lock-free communication channels
- Manages audio device configuration

#### Engine (`src/audio/engine.rs`)
- The main audio callback
- Runs on the real-time audio thread
- Processes commands, mixes tracks, applies effects
- Must complete in ~5ms (at 44.1kHz, 256 frame buffer)

#### Project (`src/audio/project.rs`)
- Top-level audio state
- Contains tracks, tempo, time signature
- Manages global settings

#### Track (`src/audio/track.rs`)
- Individual audio track
- Contains audio clips and effects chain
- Handles track-specific state (volume, pan, mute, solo)

## Real-Time Constraints

### The Golden Rule

**The audio thread must NEVER block.**

Audio callbacks run with strict timing deadlines:
- **Buffer size**: 256 frames (default) = ~5.8ms at 44.1kHz
- **ALSA on Linux**: May provide smaller buffers (64-75 frames = ~1.5ms)
- **Deadline**: Audio callback must complete before next buffer is needed

If the audio callback takes too long:
- **Audio dropout**: Audible glitch/pop in output
- **Buffer underrun**: Missing samples
- **System instability**: Priority inversion, thread starvation

### Forbidden Operations in Audio Thread

❌ **Never do these in the audio callback:**

- **Locking**: `Mutex`, `RwLock`, or any blocking synchronization
- **Allocation**: `Vec::push()`, `Box::new()`, `String` operations
- **I/O**: File operations, network, print statements
- **System calls**: Most OS operations
- **Unbounded loops**: Must have guaranteed completion time

✅ **Safe operations:**

- Reading/writing lock-free queues (rtrb)
- Fixed-size array operations
- Arithmetic and DSP calculations
- Pre-allocated buffer operations

### Optimized Debug Builds

To meet real-time deadlines, audio code is compiled with optimizations even in debug builds:

```toml
# In lightningbeam-ui/Cargo.toml
[profile.dev.package.daw-backend]
opt-level = 2

[profile.dev.package.symphonia]
opt-level = 2
# ... other audio libraries
```

This allows fast iteration while maintaining audio performance.

## Lock-Free Communication

### Command Queue (UI → Audio)

The UI thread sends commands to the audio thread via a lock-free ringbuffer:

```rust
// UI Thread
let command = AudioCommand::Play;
command_sender.push(command).ok();

// Audio Thread (in Engine::process)
while let Ok(command) = command_receiver.pop() {
    match command {
        AudioCommand::Play => self.playing = true,
        AudioCommand::Stop => self.playing = false,
        AudioCommand::Seek(time) => self.playhead = time,
        // ... handle other commands
    }
}
```

### State Updates (Audio → UI)

The audio thread sends state updates back to the UI:

```rust
// Audio Thread
let state = AudioState {
    playhead: self.playhead,
    is_playing: self.playing,
    meter_levels: self.compute_meters(),
};
state_sender.push(state).ok();

// UI Thread
if let Ok(state) = state_receiver.pop() {
    // Update UI with new state
}
```

### Design Pattern: Command-Response

1. **UI initiates action**: Send command to audio thread
2. **Audio thread executes**: In `Engine::process()`, between buffer fills
3. **Audio thread confirms**: Send state update back to UI
4. **UI updates**: Reflect new state in user interface

This pattern ensures:
- No blocking on either side
- UI remains responsive
- Audio thread never waits

## Audio Processing Pipeline

### Per-Buffer Processing

Every audio buffer (typically 256 frames), the `Engine::process()` callback:

```rust
pub fn process(&mut self, output: &mut [f32]) -> Result<(), AudioError> {
    // 1. Process commands from UI thread
    self.process_commands();

    // 2. Update playhead
    if self.playing {
        self.playhead += buffer_duration;
    }

    // 3. Clear output buffer
    output.fill(0.0);

    // 4. Process each track
    for track in &mut self.tracks {
        if track.muted {
            continue;
        }

        // Read audio samples at playhead position
        let samples = track.read_samples(self.playhead, output.len());

        // Apply track effects chain
        let mut processed = samples;
        for effect in &mut track.effects {
            processed = effect.process(processed);
        }

        // Mix to output with volume/pan
        mix_to_output(output, &processed, track.volume, track.pan);
    }

    // 5. Apply master effects
    for effect in &mut self.master_effects {
        effect.process_in_place(output);
    }

    // 6. Send state updates to UI
    self.send_state_update();

    Ok(())
}
```

### Sample Rate and Buffer Size

- **Sample rate**: 44.1kHz (default) or 48kHz
- **Buffer size**: 256 frames (configurable)
- **Channels**: Stereo (2 channels)

Buffer is interleaved: `[L, R, L, R, L, R, ...]`

### Time Representation

- **Playhead position**: Stored as `f64` seconds
- **Sample index**: `(playhead * sample_rate) as usize`
- **Frame index**: `sample_index / channels`

## Node Graph System

### Overview

Tracks use a node graph architecture powered by `dasp_graph` for flexible audio routing. Unlike simple serial effect chains, the node graph allows:

- **Parallel processing**: Multiple effects processing the same input
- **Complex routing**: Effects feeding into each other in arbitrary configurations
- **Modular synthesis**: Build synthesizers from oscillators, filters, and modulators
- **Send/return chains**: Shared effects (reverb, delay) fed by multiple tracks
- **Sidechain processing**: One signal controlling another (compression, vocoding)

### Node Graph Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Track Node Graph                    │
├─────────────────────────────────────────────────────────┤
│                                                          │
│   ┌─────────┐                                           │
│   │ Input   │ (Audio clip or synthesizer)               │
│   └────┬────┘                                           │
│        │                                                 │
│        ├──────┬──────────────┬─────────────┐           │
│        │      │              │             │           │
│        ▼      ▼              ▼             ▼           │
│   ┌────────┐ ┌────────┐ ┌────────┐  ┌─────────┐      │
│   │Filter  │ │Distort │ │  EQ    │  │ Reverb  │      │
│   │(Node 1)│ │(Node 2)│ │(Node 3)│  │(Node 4) │      │
│   └───┬────┘ └───┬────┘ └───┬────┘  └────┬────┘      │
│       │          │          │             │           │
│       └────┬─────┴──────┬───┘             │           │
│            │            │                 │           │
│            ▼            ▼                 │           │
│       ┌─────────┐  ┌─────────┐           │           │
│       │ Mixer   │  │Compress │           │           │
│       │(Node 5) │  │(Node 6) │◄──────────┘           │
│       └────┬────┘  └────┬────┘   (sidechain)         │
│            │            │                             │
│            └─────┬──────┘                             │
│                  │                                     │
│                  ▼                                     │
│            ┌──────────┐                               │
│            │  Output  │                               │
│            └──────────┘                               │
│                                                        │
└────────────────────────────────────────────────────────┘
```

### Node Types

#### Input Nodes
- **Audio Clip Reader**: Reads samples from audio file
- **Oscillator**: Generates waveforms (sine, saw, square, triangle)
- **Noise Generator**: White/pink noise
- **External Input**: Microphone or line-in

#### Processing Nodes
- **Effects**: Any audio effect (see [Adding Effects](#adding-effects))
- **Filters**: Low-pass, high-pass, band-pass, notch
- **Mixers**: Combine multiple inputs with gain control
- **Splitters**: Duplicate signal to multiple outputs

#### Output Nodes
- **Track Output**: Sends to mixer or master bus
- **Send Output**: Feeds auxiliary effects

### Building a Node Graph

```rust
use dasp_graph::{Node, NodeData, Input, BoxedNode};
use petgraph::graph::NodeIndex;

pub struct TrackGraph {
    graph: dasp_graph::Graph,
    input_node: NodeIndex,
    output_node: NodeIndex,
}

impl TrackGraph {
    pub fn new() -> Self {
        let mut graph = dasp_graph::Graph::new();

        // Create input and output nodes
        let input_node = graph.add_node(NodeData::new1(
            Input::default(),
            PassThrough, // Simple input node
        ));

        let output_node = graph.add_node(NodeData::new1(
            Input::default(),
            PassThrough, // Simple output node
        ));

        Self {
            graph,
            input_node,
            output_node,
        }
    }

    pub fn add_effect(&mut self, effect: BoxedNode) -> NodeIndex {
        // Add effect node between input and output
        let effect_node = self.graph.add_node(NodeData::new1(
            Input::default(),
            effect,
        ));

        // Connect: input -> effect -> output
        self.graph.add_edge(self.input_node, effect_node, ());
        self.graph.add_edge(effect_node, self.output_node, ());

        effect_node
    }

    pub fn connect(&mut self, from: NodeIndex, to: NodeIndex) {
        self.graph.add_edge(from, to, ());
    }

    pub fn process(&mut self, input: &[f32], output: &mut [f32]) {
        // Set input samples
        self.graph.set_input(self.input_node, input);

        // Process entire graph
        self.graph.process();

        // Read output samples
        self.graph.get_output(self.output_node, output);
    }
}
```

### Example: Serial Effect Chain

Simple effects chain (the most common case):

```rust
// Input -> Distortion -> EQ -> Reverb -> Output

let mut graph = TrackGraph::new();

let distortion = graph.add_effect(Box::new(Distortion::new(0.5)));
let eq = graph.add_effect(Box::new(EQ::new()));
let reverb = graph.add_effect(Box::new(Reverb::new()));

// Connect in series
graph.connect(graph.input_node, distortion);
graph.connect(distortion, eq);
graph.connect(eq, reverb);
graph.connect(reverb, graph.output_node);
```

### Example: Parallel Processing

Split signal into parallel paths:

```rust
// Input -> Split -> [Distortion + Clean] -> Mix -> Output

let mut graph = TrackGraph::new();

// Create parallel paths
let distortion = graph.add_effect(Box::new(Distortion::new(0.7)));
let clean = graph.add_effect(Box::new(Gain::new(1.0)));
let mixer = graph.add_effect(Box::new(Mixer::new(2))); // 2 inputs

// Connect parallel paths
graph.connect(graph.input_node, distortion);
graph.connect(graph.input_node, clean);
graph.connect(distortion, mixer);
graph.connect(clean, mixer);
graph.connect(mixer, graph.output_node);
```

### Example: Modular Synthesizer

Build a synthesizer from basic components:

```rust
//     ┌─ LFO ────┐ (modulation)
//     │          ▼
// Oscillator -> Filter -> Envelope -> Output

let mut graph = TrackGraph::new();

// Sound source
let oscillator = graph.add_effect(Box::new(Oscillator::new(440.0)));

// Modulation source
let lfo = graph.add_effect(Box::new(LFO::new(5.0))); // 5 Hz

// Filter with LFO modulation
let filter = graph.add_effect(Box::new(Filter::new_modulated()));

// Envelope
let envelope = graph.add_effect(Box::new(ADSREnvelope::new()));

// Connect sound path
graph.connect(oscillator, filter);
graph.connect(filter, envelope);
graph.connect(envelope, graph.output_node);

// Connect modulation path
graph.connect(lfo, filter); // LFO modulates filter cutoff
```

### Example: Sidechain Compression

One signal controls another:

```rust
// Input (bass) ──────────────────┐
//                                ▼
// Kick drum ────> Compressor (sidechain) -> Output

let mut graph = TrackGraph::new();

// Main signal input (bass)
let bass_input = graph.add_effect(Box::new(PassThrough));

// Sidechain signal input (kick drum)
let kick_input = graph.add_effect(Box::new(PassThrough));

// Compressor with sidechain
let compressor = graph.add_effect(Box::new(SidechainCompressor::new()));

// Connect main signal
graph.connect(bass_input, compressor);

// Connect sidechain signal (port 1 = main, port 2 = sidechain)
graph.connect_to_port(kick_input, compressor, 1);

graph.connect(compressor, graph.output_node);
```

### Node Interface

All nodes implement the `dasp_graph::Node` trait:

```rust
pub trait Node {
    /// Process audio for this node
    fn process(&mut self, inputs: &[Input], output: &mut [f32]);

    /// Number of input ports
    fn num_inputs(&self) -> usize;

    /// Number of output ports
    fn num_outputs(&self) -> usize;

    /// Reset internal state
    fn reset(&mut self);
}
```

### Multi-Channel Processing

Nodes can have multiple input/output channels:

```rust
pub struct StereoEffect {
    left_processor: Processor,
    right_processor: Processor,
}

impl Node for StereoEffect {
    fn process(&mut self, inputs: &[Input], output: &mut [f32]) {
        // Split stereo input
        let (left_in, right_in) = inputs[0].as_stereo();

        // Process each channel
        let left_out = self.left_processor.process(left_in);
        let right_out = self.right_processor.process(right_in);

        // Interleave output
        for i in 0..left_out.len() {
            output[i * 2] = left_out[i];
            output[i * 2 + 1] = right_out[i];
        }
    }

    fn num_inputs(&self) -> usize { 1 } // One stereo input
    fn num_outputs(&self) -> usize { 1 } // One stereo output

    fn reset(&mut self) {
        self.left_processor.reset();
        self.right_processor.reset();
    }
}
```

### Parameter Modulation

Nodes can expose parameters for automation or modulation:

```rust
pub struct ModulatableFilter {
    filter: Filter,
    cutoff: f32,
    resonance: f32,
}

impl Node for ModulatableFilter {
    fn process(&mut self, inputs: &[Input], output: &mut [f32]) {
        let audio_in = &inputs[0]; // Port 0: audio input

        // Port 1 (optional): cutoff modulation
        if inputs.len() > 1 {
            let mod_signal = &inputs[1];
            // Modulate cutoff: base + modulation
            self.filter.set_cutoff(self.cutoff + mod_signal[0] * 1000.0);
        }

        // Process audio
        self.filter.process(audio_in, output);
    }

    fn num_inputs(&self) -> usize { 2 } // Audio + modulation
    fn num_outputs(&self) -> usize { 1 }

    fn reset(&mut self) {
        self.filter.reset();
    }
}
```

### Graph Execution Order

`dasp_graph` automatically determines execution order using topological sort:

1. Nodes with no dependencies execute first (inputs, oscillators)
2. Nodes execute when all inputs are ready
3. Cycles are detected and prevented
4. Output nodes execute last

This ensures:
- No node processes before its inputs are ready
- Efficient CPU cache usage
- Deterministic execution

### Performance Considerations

#### Graph Overhead

Node graphs have small overhead:
- **Topological sort**: Done once when graph changes, not per-buffer
- **Buffer copying**: Minimized by reusing buffers
- **Indirection**: Virtual function calls (unavoidable with trait objects)

For simple serial chains, the overhead is negligible (<1% CPU).

#### When to Use Node Graphs vs Simple Chains

**Use node graphs when:**
- Complex routing (parallel, feedback, modulation)
- Building synthesizers from components
- User-configurable effect routing
- Sidechain processing

**Use simple chains when:**
- Just a few effects in series
- Performance is critical
- Graph structure never changes

**Note**: In Lightningbeam, audio layers always use node graphs to provide maximum flexibility for users. This allows any track to have complex routing, modular synthesis, or effect configurations without requiring different track types.

```rust
// Simple chain (no graph overhead)
pub struct SimpleChain {
    effects: Vec<Box<dyn AudioEffect>>,
}

impl SimpleChain {
    fn process(&mut self, buffer: &mut [f32]) {
        for effect in &mut self.effects {
            effect.process_in_place(buffer);
        }
    }
}
```

### Debugging Node Graphs

Enable graph visualization:

```rust
// Print graph structure
println!("{:?}", graph);

// Export to DOT format for visualization
let dot = graph.to_dot();
std::fs::write("graph.dot", dot)?;
// Then: dot -Tpng graph.dot -o graph.png
```

Trace signal flow:

```rust
// Add probe nodes to inspect signals
let probe = graph.add_effect(Box::new(SignalProbe::new("After Filter")));
graph.connect(filter, probe);
graph.connect(probe, output);

// Probe prints min/max/RMS of signal
```

## Adding Effects

### Effect Trait

All effects implement the `AudioEffect` trait:

```rust
pub trait AudioEffect: Send {
    fn process(&mut self, input: &[f32], output: &mut [f32]);
    fn process_in_place(&mut self, buffer: &mut [f32]);
    fn reset(&mut self);
}
```

### Example: Simple Gain Effect

```rust
pub struct Gain {
    gain: f32,
}

impl Gain {
    pub fn new(gain: f32) -> Self {
        Self { gain }
    }
}

impl AudioEffect for Gain {
    fn process(&mut self, input: &[f32], output: &mut [f32]) {
        for (i, &sample) in input.iter().enumerate() {
            output[i] = sample * self.gain;
        }
    }

    fn process_in_place(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample *= self.gain;
        }
    }

    fn reset(&mut self) {
        // No state to reset for gain
    }
}
```

### Example: Delay Effect (with state)

```rust
pub struct Delay {
    buffer: Vec<f32>,
    write_pos: usize,
    delay_samples: usize,
    feedback: f32,
    mix: f32,
}

impl Delay {
    pub fn new(sample_rate: f32, delay_time: f32, feedback: f32, mix: f32) -> Self {
        let delay_samples = (delay_time * sample_rate) as usize;
        let buffer_size = delay_samples.next_power_of_two();

        Self {
            buffer: vec![0.0; buffer_size],
            write_pos: 0,
            delay_samples,
            feedback,
            mix,
        }
    }
}

impl AudioEffect for Delay {
    fn process_in_place(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            // Read delayed sample
            let read_pos = (self.write_pos + self.buffer.len() - self.delay_samples)
                           % self.buffer.len();
            let delayed = self.buffer[read_pos];

            // Write new sample with feedback
            self.buffer[self.write_pos] = *sample + delayed * self.feedback;
            self.write_pos = (self.write_pos + 1) % self.buffer.len();

            // Mix dry and wet signals
            *sample = *sample * (1.0 - self.mix) + delayed * self.mix;
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }
}
```

### Adding Effects to Tracks

```rust
// UI Thread
let command = AudioCommand::AddEffect {
    track_id: track_id,
    effect: Box::new(Delay::new(44100.0, 0.5, 0.3, 0.5)),
};
command_sender.push(command).ok();
```

### Built-In Effects

Located in `daw-backend/src/effects/`:

- **reverb.rs**: Reverb
- **delay.rs**: Delay
- **eq.rs**: Equalizer
- **compressor.rs**: Dynamic range compressor
- **distortion.rs**: Distortion/overdrive
- **chorus.rs**: Chorus
- **flanger.rs**: Flanger
- **phaser.rs**: Phaser
- **limiter.rs**: Brick-wall limiter

## Adding Synthesizers

### Synthesizer Trait

```rust
pub trait Synthesizer: Send {
    fn process(&mut self, output: &mut [f32], sample_rate: f32);
    fn note_on(&mut self, note: u8, velocity: u8);
    fn note_off(&mut self, note: u8);
    fn reset(&mut self);
}
```

### Example: Simple Oscillator

```rust
pub struct Oscillator {
    phase: f32,
    frequency: f32,
    amplitude: f32,
    sample_rate: f32,
}

impl Oscillator {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            phase: 0.0,
            frequency: 440.0,
            amplitude: 0.0,
            sample_rate,
        }
    }
}

impl Synthesizer for Oscillator {
    fn process(&mut self, output: &mut [f32], _sample_rate: f32) {
        for sample in output.iter_mut() {
            // Generate sine wave
            *sample = (self.phase * 2.0 * std::f32::consts::PI).sin() * self.amplitude;

            // Advance phase
            self.phase += self.frequency / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }

    fn note_on(&mut self, note: u8, velocity: u8) {
        // Convert MIDI note to frequency
        self.frequency = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
        self.amplitude = velocity as f32 / 127.0;
    }

    fn note_off(&mut self, _note: u8) {
        self.amplitude = 0.0;
    }

    fn reset(&mut self) {
        self.phase = 0.0;
        self.amplitude = 0.0;
    }
}
```

### Built-In Synthesizers

Located in `daw-backend/src/synth/`:

- **oscillator.rs**: Basic waveform generator (sine, saw, square, triangle)
- **fm_synth.rs**: FM synthesis
- **wavetable.rs**: Wavetable synthesis
- **sampler.rs**: Sample-based synthesis

## MIDI System

### MIDI Input

```rust
// Setup MIDI input (UI thread)
let midi_input = midir::MidiInput::new("Lightningbeam")?;
let port = midi_input.ports()[0];

midi_input.connect(&port, "input", move |_timestamp, message, _| {
    // Parse MIDI message
    match message[0] & 0xF0 {
        0x90 => {
            // Note On
            let note = message[1];
            let velocity = message[2];
            command_sender.push(AudioCommand::NoteOn { note, velocity }).ok();
        }
        0x80 => {
            // Note Off
            let note = message[1];
            command_sender.push(AudioCommand::NoteOff { note }).ok();
        }
        _ => {}
    }
}, ())?;
```

### MIDI File Parsing

```rust
use midly::{Smf, TrackEventKind};

let smf = Smf::parse(&midi_data)?;
for track in smf.tracks {
    for event in track {
        match event.kind {
            TrackEventKind::Midi { channel, message } => {
                // Process MIDI message
            }
            _ => {}
        }
    }
}
```

## Performance Optimization

### Pre-Allocation

Allocate all buffers before audio thread starts:

```rust
// Good: Pre-allocated
pub struct Track {
    buffer: Vec<f32>,  // Allocated once in constructor
    // ...
}

// Bad: Allocates in audio thread
fn process(&mut self) {
    let mut temp = Vec::new();  // ❌ Allocates!
    // ...
}
```

### Memory-Mapped Audio Files

Large audio files use memory-mapped I/O for zero-copy access:

```rust
use memmap2::Mmap;

let file = File::open(path)?;
let mmap = unsafe { Mmap::map(&file)? };
// Audio samples can be read directly from mmap
```

### SIMD Optimization

For portable SIMD operations, use the `fearless_simd` crate:

```rust
use fearless_simd::*;

fn process_simd(samples: &mut [f32], gain: f32) {
    // Automatically uses best available SIMD instructions
    // (SSE, AVX, NEON, etc.) without unsafe code
    for chunk in samples.chunks_exact_mut(f32x8::LEN) {
        let simd_samples = f32x8::from_slice(chunk);
        let simd_gain = f32x8::splat(gain);
        let result = simd_samples * simd_gain;
        result.write_to_slice(chunk);
    }

    // Handle remainder
    let remainder = samples.chunks_exact_mut(f32x8::LEN).into_remainder();
    for sample in remainder {
        *sample *= gain;
    }
}
```

This approach is:
- **Portable**: Works across x86, ARM, and other architectures
- **Safe**: No unsafe code required
- **Automatic**: Uses best available SIMD instructions for the target
- **Fallback**: Gracefully degrades on platforms without SIMD

### Avoid Branching in Inner Loops

```rust
// Bad: Branch in inner loop
for sample in samples.iter_mut() {
    if self.gain > 0.5 {
        *sample *= 2.0;
    }
}

// Good: Branch outside loop
let multiplier = if self.gain > 0.5 { 2.0 } else { 1.0 };
for sample in samples.iter_mut() {
    *sample *= multiplier;
}
```

## Debugging Audio Issues

### Enable Debug Logging

```bash
DAW_AUDIO_DEBUG=1 cargo run
```

Output includes:
```
[AUDIO] Buffer size: 256 frames (5.8ms at 44100 Hz)
[AUDIO] Processing time: avg=0.8ms, worst=2.1ms
[AUDIO] Playhead: 1.234s
[AUDIO] WARNING: Audio overrun detected!
```

### Common Issues

#### Audio Dropouts

**Symptoms**: Clicks, pops, glitches in audio output

**Causes**:
- Audio callback taking too long
- Blocking operation in audio thread
- Insufficient CPU resources

**Solutions**:
- Increase buffer size (reduces CPU pressure, increases latency)
- Optimize audio processing code
- Remove debug prints from audio thread
- Check `DAW_AUDIO_DEBUG=1` output for timing info

#### Crackling/Distortion

**Symptoms**: Harsh, noisy audio

**Causes**:
- Samples exceeding [-1.0, 1.0] range (clipping)
- Incorrect sample rate conversion
- Denormal numbers in filters

**Solutions**:
- Add limiter to master output
- Use hard clipping: `sample.clamp(-1.0, 1.0)`
- Enable flush-to-zero for denormals

#### No Audio Output

**Symptoms**: Silence, but no errors

**Causes**:
- Audio device not found
- Wrong device selected
- All tracks muted
- Volume set to zero

**Solutions**:
- Check `cpal` device enumeration
- Verify track volumes and mute states
- Check master volume
- Test with simple sine wave

### Profiling Audio Performance

```bash
# Use perf on Linux
perf record --call-graph dwarf cargo run --release
perf report

# Look for hot spots in Engine::process()
```

## Related Documentation

- [ARCHITECTURE.md](../ARCHITECTURE.md) - Overall system architecture
- [docs/UI_SYSTEM.md](UI_SYSTEM.md) - UI integration with audio system
- [docs/BUILDING.md](BUILDING.md) - Build troubleshooting
