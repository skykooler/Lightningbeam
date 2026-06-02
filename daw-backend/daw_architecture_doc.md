# DAW Backend Architecture & Implementation Roadmap

**Version:** 1.0  
**Date:** October 2025  
**Language:** Rust  
**Audio I/O:** cpal

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Core Components](#core-components)
3. [Metatracks Architecture](#metatracks-architecture)
4. [Implementation Roadmap](#implementation-roadmap)
5. [Technical Specifications](#technical-specifications)
6. [Testing Strategy](#testing-strategy)

---

## Architecture Overview

### High-Level Design

The DAW follows a **multi-threaded, message-passing architecture** that separates real-time audio processing from UI and control logic:

```
┌─────────────┐     Commands      ┌─────────────┐     Commands      ┌─────────────┐
│  UI Thread  │ ←──────────────→  │Control Thread│ ←──────────────→ │ Audio Thread│
└─────────────┘     Events        └─────────────┘    Lock-free     └─────────────┘
                                          │             Queues              │
                                          ↓                                 │
                                   ┌─────────────┐                         │
                                   │Project State│←────────────────────────┘
                                   │(Triple-buf) │      Atomic reads
                                   └─────────────┘
```

### Design Principles

1. **Real-time Safety**: Audio thread is lock-free and allocation-free
2. **Hierarchical Composition**: Tracks can contain other tracks (metatracks)
3. **Message-Based Communication**: All cross-thread communication via lock-free queues
4. **Incremental Complexity**: Architecture supports simple flat tracks initially, scales to nested metatracks
5. **Separation of Concerns**: Audio processing, state management, and UI are decoupled

---

## Core Components

### 1. Audio Engine (Real-time Thread)

**Responsibilities:**
- Process audio graph in response to cpal callbacks
- Execute commands from control thread
- Maintain playback position and transport state
- Send events back to control thread

**Constraints:**
- No memory allocations
- No blocking operations (mutex, I/O)
- No unbounded loops
- Pre-allocated buffers only

**Key Structures:**

```rust
struct AudioEngine {
    tracks: Vec<TrackNode>,
    playhead: u64,              // Sample position
    playing: bool,
    sample_rate: u32,
    
    // Communication
    command_rx: rtrb::Consumer<Command>,
    event_tx: rtrb::Producer<AudioEvent>,
    
    // Pre-allocated resources
    mix_buffer: Vec<f32>,
    buffer_pool: BufferPool,
}

enum Command {
    Play,
    Stop,
    Seek(f64),
    SetTempo(f32),
    UpdateTrackVolume(TrackId, f32),
    UpdateTrackMute(TrackId, bool),
    AddEffect(TrackId, EffectType),
    // ... more commands
}

enum AudioEvent {
    PlaybackPosition(f64),
    PeakLevel(TrackId, f32),
    BufferUnderrun,
    // ... more events
}
```

### 2. Track Hierarchy

**Track Node Types:**

```rust
enum TrackNode {
    Audio(AudioTrack),
    Midi(MidiTrack),
    Metatrack(Metatrack),
    Bus(BusTrack),
}

struct AudioTrack {
    id: TrackId,
    name: String,
    clips: Vec<Clip>,
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
    parent: Option<TrackId>,
}

struct MidiTrack {
    id: TrackId,
    name: String,
    clips: Vec<MidiClip>,
    instrument: Box<dyn Effect>,  // Virtual instrument
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
    parent: Option<TrackId>,
}

struct Metatrack {
    id: TrackId,
    name: String,
    children: Vec<TrackId>,
    effects: Vec<Box<dyn Effect>>,
    
    // Metatrack-specific features
    time_stretch: f32,      // Speed multiplier (0.5 = half speed)
    pitch_shift: f32,       // Semitones
    offset: f64,            // Time offset in seconds
    
    volume: f32,
    pan: f32,
    muted: bool,
    solo: bool,
    parent: Option<TrackId>,
    
    // UI hints
    collapsed: bool,
    color: Color,
}

struct BusTrack {
    id: TrackId,
    name: String,
    inputs: Vec<TrackId>,   // Which tracks send to this bus
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    pan: f32,
}
```

### 3. Clips and Regions

```rust
struct Clip {
    id: ClipId,
    content: ClipContent,
    start_time: f64,        // Position in parent track (seconds)
    duration: f64,          // Clip duration (seconds)
    offset: f64,            // Offset into content (seconds)
    
    // Clip-level processing
    gain: f32,
    fade_in: f64,
    fade_out: f64,
    reversed: bool,
}

enum ClipContent {
    AudioFile {
        pool_index: usize,  // Index into AudioPool
    },
    MidiData {
        events: Vec<MidiEvent>,
    },
    MetatrackReference {
        track_id: TrackId,
    },
}

struct MidiEvent {
    timestamp: u64,         // Sample offset within clip
    status: u8,
    data1: u8,
    data2: u8,
}
```

### 4. Audio Pool

Shared audio file storage:

```rust
struct AudioPool {
    files: Vec<AudioFile>,
    cache: LruCache<FileId, Vec<f32>>,
}

struct AudioFile {
    id: FileId,
    path: PathBuf,
    data: Vec<f32>,         // Interleaved samples
    channels: u32,
    sample_rate: u32,
    frames: u64,
}
```

### 5. Effect System

```rust
trait Effect: Send {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32);
    fn set_parameter(&mut self, id: u32, value: f32);
    fn get_parameter(&self, id: u32) -> f32;
    fn reset(&mut self);
}

// Example implementations
struct GainEffect {
    gain_db: f32,
}

struct SimpleEQ {
    low_gain: f32,
    mid_gain: f32,
    high_gain: f32,
    filters: [BiquadFilter; 3],
}

struct SimpleSynth {
    oscillators: Vec<Oscillator>,
    adsr: AdsrEnvelope,
}
```

### 6. Render Context

Carries time and tempo information through the track hierarchy:

```rust
struct RenderContext {
    global_position: u64,       // Absolute sample position
    local_position: u64,        // Position within current scope
    sample_rate: u32,
    tempo: f32,
    time_signature: (u32, u32),
    time_stretch: f32,          // Accumulated stretch factor
}

impl Metatrack {
    fn transform_context(&self, ctx: RenderContext) -> RenderContext {
        let offset_samples = (self.offset * ctx.sample_rate as f64) as u64;
        let local_pos = ((ctx.local_position.saturating_sub(offset_samples)) as f64 
            / self.time_stretch as f64) as u64;
        
        RenderContext {
            global_position: ctx.global_position,
            local_position: local_pos,
            sample_rate: ctx.sample_rate,
            tempo: ctx.tempo * self.time_stretch,
            time_signature: ctx.time_signature,
            time_stretch: ctx.time_stretch * self.time_stretch,
        }
    }
}
```

### 7. Project State

```rust
struct Project {
    tracks: HashMap<TrackId, TrackNode>,
    root_tracks: Vec<TrackId>,
    audio_pool: AudioPool,
    
    // Global settings
    sample_rate: u32,
    tempo: f32,
    time_signature: (u32, u32),
    
    // Metadata
    name: String,
    created: SystemTime,
    modified: SystemTime,
}

impl Project {
    fn get_processing_order(&self) -> Vec<TrackId> {
        // Depth-first traversal for correct rendering order
        let mut order = Vec::new();
        for root_id in &self.root_tracks {
            self.collect_depth_first(*root_id, &mut order);
        }
        order
    }
    
    fn collect_depth_first(&self, id: TrackId, order: &mut Vec<TrackId>) {
        if let Some(TrackNode::Metatrack(meta)) = self.tracks.get(&id) {
            for child_id in &meta.children {
                self.collect_depth_first(*child_id, order);
            }
        }
        order.push(id);
    }
}
```

### 8. Buffer Management

```rust
struct BufferPool {
    buffers: Vec<Vec<f32>>,
    available: Vec<usize>,
    buffer_size: usize,
}

impl BufferPool {
    fn acquire(&mut self) -> Vec<f32> {
        if let Some(idx) = self.available.pop() {
            let mut buf = std::mem::take(&mut self.buffers[idx]);
            buf.fill(0.0);
            buf
        } else {
            vec![0.0; self.buffer_size]
        }
    }
    
    fn release(&mut self, buffer: Vec<f32>) {
        let idx = self.buffers.len();
        self.buffers.push(buffer);
        self.available.push(idx);
    }
}
```

---

## Metatracks Architecture

### Processing Model

Metatracks use a **pre-mix** model:

1. Mix all children into a temporary buffer
2. Apply metatrack's effects to the mixed buffer
3. Mix result into parent's output

```rust
fn process_metatrack(
    meta: &Metatrack,
    project: &Project,
    output: &mut [f32],
    context: RenderContext,
    buffer_pool: &mut BufferPool,
) {
    // Transform context for children
    let child_context = meta.transform_context(context);
    
    // Acquire scratch buffer
    let mut submix = buffer_pool.acquire();
    submix.resize(output.len(), 0.0);
    
    // Process all children into submix
    for child_id in &meta.children {
        if let Some(child) = project.tracks.get(child_id) {
            process_track_node(
                child,
                project,
                &mut submix,
                child_context,
                buffer_pool
            );
        }
    }
    
    // Apply metatrack's effects
    for effect in &mut meta.effects {
        effect.process(&mut submix, 2, context.sample_rate);
    }
    
    // Mix into output with volume
    for (out, sub) in output.iter_mut().zip(submix.iter()) {
        *out += sub * meta.volume;
    }
    
    // Return buffer to pool
    buffer_pool.release(submix);
}
```

### Time Transformation

Metatracks can manipulate time for all children:

- **Time Stretch**: Speed up or slow down playback
- **Offset**: Shift content in time
- **Pitch Shift**: Transpose content (future feature, requires pitch-preserving time stretch)

### Metatrack Operations

```rust
enum MetatrackOperation {
    // Creation
    CreateFromSelection(Vec<TrackId>),
    CreateEmpty,
    
    // Hierarchy manipulation
    AddToMetatrack(TrackId, Vec<TrackId>),
    RemoveFromMetatrack(TrackId, Vec<TrackId>),
    MoveToMetatrack { track: TrackId, new_parent: TrackId },
    Ungroup(TrackId),
    Flatten(TrackId),
    
    // Transformation
    SetTimeStretch(TrackId, f32),
    SetOffset(TrackId, f64),
    
    // Rendering
    BounceToAudio(TrackId),
    Freeze(TrackId),
    Unfreeze(TrackId),
}
```

### Nesting Limits

- **Recommended maximum depth**: 10 levels
- **Reason**: Performance and UI complexity
- **Implementation**: Check depth during metatrack creation

---

## Implementation Roadmap

### Phase 1: Single Audio File Playback (Week 1-2)

**Goal**: Play one audio file through speakers

**Deliverables:**
- Basic cpal integration
- Load audio file with symphonia
- Simple playback loop
- Press spacebar to play/pause

**Core Implementation:**

```rust
struct SimpleEngine {
    audio_data: Vec<f32>,
    playhead: usize,
    sample_rate: u32,
    playing: Arc<AtomicBool>,
}

// Main audio callback
fn audio_callback(data: &mut [f32], engine: &mut SimpleEngine) {
    if engine.playing.load(Ordering::Relaxed) {
        let end = (engine.playhead + data.len()).min(engine.audio_data.len());
        let available = end - engine.playhead;
        
        data[..available].copy_from_slice(
            &engine.audio_data[engine.playhead..end]
        );
        
        engine.playhead = end;
    } else {
        data.fill(0.0);
    }
}
```

**Dependencies:**
- `cpal = "0.15"`
- `symphonia = "0.5"`

**Success Criteria:**
- Audio plays without clicks or pops
- Can start/stop playback
- No audio thread panics

---

### Phase 2: Transport Control + UI Communication (Week 2-3)

**Goal**: Start/stop/seek from a basic UI

**Deliverables:**
- Lock-free command queue
- Atomic playhead position
- Basic UI (terminal or simple window)
- Play/pause/seek controls

**Core Implementation:**

```rust
enum Command {
    Play,
    Stop,
    Seek(f64),
}

struct Engine {
    audio_data: Vec<f32>,
    playhead: Arc<AtomicU64>,
    command_rx: rtrb::Consumer<Command>,
    playing: bool,
    sample_rate: u32,
}

fn audio_callback(data: &mut [f32], engine: &mut Engine) {
    // Process all pending commands
    while let Ok(cmd) = engine.command_rx.pop() {
        match cmd {
            Command::Play => engine.playing = true,
            Command::Stop => {
                engine.playing = false;
                engine.playhead.store(0, Ordering::Relaxed);
            }
            Command::Seek(seconds) => {
                let samples = (seconds * engine.sample_rate as f64) as u64;
                engine.playhead.store(samples, Ordering::Relaxed);
            }
        }
    }
    
    // Render audio...
}
```

**New Dependencies:**
- `rtrb = "0.3"` (lock-free ringbuffer)

**Success Criteria:**
- Commands execute within 1 buffer period
- No audio glitches during seek
- Playhead position updates smoothly

---

### Phase 3: Multiple Audio Tracks (Week 3-4)

**Goal**: Play multiple audio files simultaneously

**Deliverables:**
- Track data structure
- Per-track volume control
- Mute/solo functionality
- Mix multiple tracks

**Core Implementation:**

```rust
struct Track {
    id: u32,
    audio_data: Vec<f32>,
    volume: f32,
    muted: bool,
    solo: bool,
}

struct Engine {
    tracks: Vec<Track>,
    playhead: Arc<AtomicU64>,
    command_rx: rtrb::Consumer<Command>,
    sample_rate: u32,
    mix_buffer: Vec<f32>,
}

enum Command {
    Play,
    Stop,
    Seek(f64),
    SetTrackVolume(u32, f32),
    SetTrackMute(u32, bool),
    SetTrackSolo(u32, bool),
}

fn audio_callback(data: &mut [f32], engine: &mut Engine) {
    // Process commands...
    
    if engine.playing {
        // Clear mix buffer
        engine.mix_buffer.fill(0.0);
        
        // Check if any track is soloed
        let any_solo = engine.tracks.iter().any(|t| t.solo);
        
        // Mix all active tracks
        for track in &engine.tracks {
            let active = !track.muted && (!any_solo || track.solo);
            if active {
                mix_track(track, &mut engine.mix_buffer, engine.playhead, data.len());
            }
        }
        
        // Copy mix to output
        data.copy_from_slice(&engine.mix_buffer[..data.len()]);
    }
}

fn mix_track(track: &Track, output: &mut [f32], playhead: u64, frames: usize) {
    let start = playhead as usize;
    let end = (start + frames).min(track.audio_data.len());
    
    for (i, sample) in track.audio_data[start..end].iter().enumerate() {
        output[i] += sample * track.volume;
    }
}
```

**Success Criteria:**
- 4+ tracks play simultaneously without distortion
- Volume changes are smooth (no clicks)
- Solo/mute work correctly
- CPU usage remains reasonable

---

### Phase 4: Clips & Timeline (Week 4-5)

**Goal**: Place audio regions at different positions on timeline

**Deliverables:**
- Clip data structure
- Timeline-based playback
- Audio pool for shared audio data
- Multiple clips per track

**Core Implementation:**

```rust
struct Clip {
    id: u32,
    audio_pool_index: usize,
    start_time: f64,        // Seconds
    duration: f64,
    offset: f64,            // Offset into audio file
    gain: f32,
}

struct Track {
    id: u32,
    clips: Vec<Clip>,
    volume: f32,
    muted: bool,
    solo: bool,
}

struct AudioPool {
    files: Vec<Vec<f32>>,
}

fn render_track(
    track: &Track,
    output: &mut [f32],
    pool: &AudioPool,
    playhead_seconds: f64,
    sample_rate: u32,
    frames: usize,
) {
    for clip in &track.clips {
        let clip_start = clip.start_time;
        let clip_end = clip.start_time + clip.duration;
        
        // Check if clip is active in this time range
        if playhead_seconds < clip_end && 
           playhead_seconds + (frames as f64 / sample_rate as f64) > clip_start {
            
            render_clip(clip, output, pool, playhead_seconds, sample_rate, frames);
        }
    }
}

fn render_clip(
    clip: &Clip,
    output: &mut [f32],
    pool: &AudioPool,
    playhead_seconds: f64,
    sample_rate: u32,
    frames: usize,
) {
    let audio = &pool.files[clip.audio_pool_index];
    
    // Calculate position within clip
    let clip_position = playhead_seconds - clip.start_time + clip.offset;
    let start_sample = (clip_position * sample_rate as f64) as usize;
    
    // Calculate how many samples to copy
    let samples_available = audio.len().saturating_sub(start_sample);
    let samples_to_copy = samples_available.min(output.len());
    
    // Mix into output
    for i in 0..samples_to_copy {
        output[i] += audio[start_sample + i] * clip.gain * clip.volume;
    }
}
```

**Success Criteria:**
- Clips play at correct timeline positions
- Multiple clips per track work correctly
- Clips can overlap
- Audio pool prevents duplication

**UI Requirements:**
- Basic timeline view
- Drag clips to position them
- Visual representation of waveforms

---

### Phase 5: Effect Processing (Week 5-6)

**Goal**: Add gain/pan/simple effects to tracks

**Deliverables:**
- Effect trait
- Basic effects (gain, pan, simple EQ)
- Per-track effect chain
- Effect parameter control

**Core Implementation:**

```rust
trait Effect: Send {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32);
    fn set_parameter(&mut self, id: u32, value: f32);
    fn get_parameter(&self, id: u32) -> f32;
    fn reset(&mut self);
}

struct GainEffect {
    gain_linear: f32,
}

impl Effect for GainEffect {
    fn process(&mut self, buffer: &mut [f32], _channels: usize, _sample_rate: u32) {
        for sample in buffer.iter_mut() {
            *sample *= self.gain_linear;
        }
    }
    
    fn set_parameter(&mut self, id: u32, value: f32) {
        if id == 0 {  // Gain in dB
            self.gain_linear = 10.0_f32.powf(value / 20.0);
        }
    }
    
    fn get_parameter(&self, id: u32) -> f32 {
        if id == 0 {
            20.0 * self.gain_linear.log10()
        } else {
            0.0
        }
    }
    
    fn reset(&mut self) {}
}

struct Track {
    id: u32,
    clips: Vec<Clip>,
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    muted: bool,
}

fn render_track(
    track: &mut Track,
    output: &mut [f32],
    // ... other params
) {
    // Render all clips...
    
    // Apply effect chain
    for effect in &mut track.effects {
        effect.process(output, 2, sample_rate);
    }
    
    // Apply track volume
    for sample in output.iter_mut() {
        *sample *= track.volume;
    }
}
```

**Additional Effects to Implement:**

```rust
struct PanEffect {
    pan: f32,  // -1.0 (left) to 1.0 (right)
}

struct SimpleEQ {
    low_gain: f32,
    mid_gain: f32,
    high_gain: f32,
    low_filter: BiquadFilter,
    high_filter: BiquadFilter,
}

struct BiquadFilter {
    b0: f32, b1: f32, b2: f32,
    a1: f32, a2: f32,
    x1: f32, x2: f32,
    y1: f32, y2: f32,
}
```

**Success Criteria:**
- Effects process without distortion
- Multiple effects can chain
- Parameter changes are smooth
- No performance degradation

**Begin DSP Library:**
- Basic filters (lowpass, highpass, bandpass)
- Utilities (db to linear, frequency to coefficients)

---

### Phase 6: Hierarchical Tracks - Foundation (Week 6-7)

**Goal**: Introduce track hierarchy (groups) without full metatracks

**Deliverables:**
- TrackNode enum
- Group tracks
- Recursive rendering
- Parent-child relationships

**Core Implementation:**

```rust
enum TrackNode {
    Audio(AudioTrack),
    Group(GroupTrack),
}

struct AudioTrack {
    id: u32,
    clips: Vec<Clip>,
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    parent: Option<u32>,
}

struct GroupTrack {
    id: u32,
    children: Vec<u32>,
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
}

struct Project {
    tracks: HashMap<u32, TrackNode>,
    root_tracks: Vec<u32>,
    audio_pool: AudioPool,
}

fn render_track_node(
    node_id: u32,
    project: &Project,
    output: &mut [f32],
    context: &RenderContext,
    buffer_pool: &mut BufferPool,
) {
    match &project.tracks[&node_id] {
        TrackNode::Audio(track) => {
            render_audio_track(track, output, &project.audio_pool, context);
        }
        TrackNode::Group(group) => {
            // Get temp buffer from pool
            let mut group_buffer = buffer_pool.acquire();
            group_buffer.resize(output.len(), 0.0);
            
            // Render all children
            for child_id in &group.children {
                render_track_node(
                    *child_id,
                    project,
                    &mut group_buffer,
                    context,
                    buffer_pool
                );
            }
            
            // Apply group effects
            for effect in &mut group.effects {
                effect.process(&mut group_buffer, 2, context.sample_rate);
            }
            
            // Mix into output
            for (out, group) in output.iter_mut().zip(group_buffer.iter()) {
                *out += group * group.volume;
            }
            
            buffer_pool.release(group_buffer);
        }
    }
}

struct RenderContext {
    playhead_seconds: f64,
    sample_rate: u32,
    tempo: f32,
}
```

**Success Criteria:**
- Can create groups of tracks
- Groups can nest (test 3-4 levels)
- Effects on groups affect all children
- No audio glitches from recursion

**Refactoring Required:**
- Migrate from `Vec<Track>` to `HashMap<TrackId, TrackNode>`
- Update all track access code
- Add parent tracking

---

### Phase 7: MIDI Support (Week 7-8)

**Goal**: Play MIDI through virtual instruments

**Deliverables:**
- MIDI data structures
- MIDI clip rendering
- Simple virtual instrument
- MIDI track type

**Core Implementation:**

```rust
struct MidiEvent {
    timestamp: u64,     // Sample position
    status: u8,
    data1: u8,          // Note/CC number
    data2: u8,          // Velocity/value
}

struct MidiClip {
    id: u32,
    events: Vec<MidiEvent>,
    start_time: f64,
    duration: f64,
}

struct MidiTrack {
    id: u32,
    clips: Vec<MidiClip>,
    instrument: Box<dyn Effect>,  // Synth as effect
    effects: Vec<Box<dyn Effect>>,
    volume: f32,
    parent: Option<u32>,
}

enum TrackNode {
    Audio(AudioTrack),
    Midi(MidiTrack),
    Group(GroupTrack),
}

// Simple sine wave synth for testing
struct SimpleSynth {
    voices: Vec<SynthVoice>,
    sample_rate: f32,
}

struct SynthVoice {
    active: bool,
    note: u8,
    velocity: u8,
    phase: f32,
    frequency: f32,
}

impl Effect for SimpleSynth {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32) {
        // Process active voices
        for voice in &mut self.voices {
            if voice.active {
                for frame in buffer.chunks_mut(channels) {
                    let sample = (voice.phase * 2.0 * PI).sin() 
                        * (voice.velocity as f32 / 127.0) * 0.3;
                    
                    for channel in frame.iter_mut() {
                        *channel += sample;
                    }
                    
                    voice.phase += voice.frequency / sample_rate as f32;
                    if voice.phase >= 1.0 {
                        voice.phase -= 1.0;
                    }
                }
            }
        }
    }
    
    // Handle MIDI events via parameters
    fn set_parameter(&mut self, id: u32, value: f32) {
        match id {
            0 => self.note_on(value as u8, 100),  // Note on
            1 => self.note_off(value as u8),       // Note off
            _ => {}
        }
    }
}

fn render_midi_track(
    track: &mut MidiTrack,
    output: &mut [f32],
    context: &RenderContext,
    frames: usize,
) {
    // Collect MIDI events for this render period
    let mut events_to_process = Vec::new();
    
    for clip in &track.clips {
        collect_events_in_range(
            clip,
            context.playhead_seconds,
            frames,
            context.sample_rate,
            &mut events_to_process
        );
    }
    
    // Sort by timestamp
    events_to_process.sort_by_key(|e| e.timestamp);
    
    // Process events through instrument
    for event in events_to_process {
        handle_midi_event(&mut track.instrument, event);
    }
    
    // Generate audio
    track.instrument.process(output, 2, context.sample_rate);
    
    // Apply effect chain
    for effect in &mut track.effects {
        effect.process(output, 2, context.sample_rate);
    }
}
```

**Success Criteria:**
- Can load and play MIDI files
- Notes trigger at correct times
- Polyphony works (4+ notes)
- Timing is sample-accurate

**Dependencies:**
- `midly = "0.5"` (for MIDI file parsing)

---

### Phase 8: Full Metatracks (Week 8-9)

**Goal**: Add time transformation and metatrack-specific features

**Deliverables:**
- Metatrack type with transformations
- Time stretch functionality
- Offset capability
- Transform context propagation

**Core Implementation:**

```rust
struct Metatrack {
    id: u32,
    children: Vec<u32>,
    effects: Vec<Box<dyn Effect>>,
    
    // Transformation parameters
    time_stretch: f32,      // 0.5 = half speed, 2.0 = double speed
    pitch_shift: f32,       // Semitones (future feature)
    offset: f64,            // Time offset in seconds
    
    volume: f32,
    parent: Option<u32>,
}

enum TrackNode {
    Audio(AudioTrack),
    Midi(MidiTrack),
    Metatrack(Metatrack),
    Group(GroupTrack),
}

struct RenderContext {
    global_position: u64,   // Absolute sample position
    local_position: u64,    // Position within current scope
    sample_rate: u32,
    tempo: f32,
    time_signature: (u32, u32),
    time_stretch: f32,      // Accumulated stretch
}

impl Metatrack {
    fn transform_context(&self, ctx: RenderContext) -> RenderContext {
        let offset_samples = (self.offset * ctx.sample_rate as f64) as u64;
        
        let adjusted_position = ctx.local_position.saturating_sub(offset_samples);
        let stretched_position = (adjusted_position as f64 / self.time_stretch as f64) as u64;
        
        RenderContext {
            global_position: ctx.global_position,
            local_position: stretched_position,
            sample_rate: ctx.sample_rate,
            tempo: ctx.tempo * self.time_stretch,
            time_signature: ctx.time_signature,
            time_stretch: ctx.time_stretch * self.time_stretch,
        }
    }
}

fn render_metatrack(
    meta: &Metatrack,
    project: &Project,
    output: &mut [f32],
    context: RenderContext,
    buffer_pool: &mut BufferPool,
) {
    // Transform context for children
    let child_context = meta.transform_context(context);
    
    // Acquire buffer for submix
    let mut submix = buffer_pool.acquire();
    submix.resize(output.len(), 0.0);
    
    // Render all children with transformed context
    for child_id in &meta.children {
        if let Some(child) = project.tracks.get(child_id) {
            render_track_node(
                *child_id,
                project,
                &mut submix,
                child_context,
                buffer_pool
            );
        }
    }
    
    // Apply metatrack effects
    for effect in &mut meta.effects {
        effect.process(&mut submix, 2, context.sample_rate);
    }
    
    // Mix into output
    for (out, sub) in output.iter_mut().zip(submix.iter()) {
        *out += sub * meta.volume;
    }
    
    buffer_pool.release(submix);
}
```

**Metatrack Operations:**

```rust
impl Project {
    fn create_metatrack_from_selection(&mut self, track_ids: Vec<u32>) -> u32 {
        let metatrack_id = self.next_id();
        
        let metatrack = Metatrack {
            id: metatrack_id,
            children: track_ids.clone(),
            effects: Vec::new(),
            time_stretch: 1.0,
            pitch_shift: 0.0,
            offset: 0.0,
            volume: 1.0,
            parent: None,
        };
        
        // Update parent references
        for track_id in track_ids {
            if let Some(track) = self.tracks.get_mut(&track_id) {
                set_parent(track, Some(metatrack_id));
            }
        }
        
        self.tracks.insert(metatrack_id, TrackNode::Metatrack(metatrack));
        self.root_tracks.push(metatrack_id);
        
        metatrack_id
    }
    
    fn ungroup_metatrack(&mut self, metatrack_id: u32) {
        if let Some(TrackNode::Metatrack(meta)) = self.tracks.get(&metatrack_id) {
            let children = meta.children.clone();
            
            // Remove parent from children
            for child_id in children {
                if let Some(track) = self.tracks.get_mut(&child_id) {
                    set_parent(track, None);
                }
                self.root_tracks.push(child_id);
            }
            
            // Remove metatrack
            self.tracks.remove(&metatrack_id);
            self.root_tracks.retain(|&id| id != metatrack_id);
        }
    }
}
```

**Success Criteria:**
- Can create metatracks from track selection
- Time stretch affects all children
- Offset shifts children in time
- Can nest metatracks 5+ levels deep
- Performance remains acceptable

---

### Phase 9: Polish & Optimization (Week 9-11)

#### 9a. Buffer Pool Optimization (Week 9)

**Goal**: Eliminate allocations in audio thread

```rust
struct BufferPool {
    buffers: Vec<Vec<f32>>,
    available: Vec<usize>,
    buffer_size: usize,
    total_allocations: AtomicUsize,
}

impl BufferPool {
    fn new(count: usize, size: usize) -> Self {
        let mut buffers = Vec::with_capacity(count);
        let mut available = Vec::with_capacity(count);
        
        for i in 0..count {
            buffers.push(vec![0.0; size]);
            available.push(i);
        }
        
        BufferPool {
            buffers,
            available,
            buffer_size: size,
            total_allocations: AtomicUsize::new(0),
        }
    }
    
    fn acquire(&mut self) -> Vec<f32> {
        if let Some(idx) = self.available.pop() {
            let mut buf = std::mem::take(&mut self.buffers[idx]);
            buf.fill(0.0);
            buf
        } else {
            self.total_allocations.fetch_add(1, Ordering::Relaxed);
            vec![0.0; self.buffer_size]
        }
    }
    
    fn release(&mut self, buffer: Vec<f32>) {
        if buffer.len() == self.buffer_size {
            let idx = self.buffers.len();
            self.buffers.push(buffer);
            self.available.push(idx);
        }
    }
}
```

**Success Criteria:**
- Zero allocations during steady-state playback
- Pool size auto-adjusts to actual usage
- Metrics show allocation count

#### 9b. Lock-Free State Updates (Week 10)

**Goal**: Replace Mutex with triple-buffering for project state

```rust
struct TripleBuffer<T> {
    buffers: [T; 3],
    write_idx: AtomicUsize,
    read_idx: AtomicUsize,
}

impl<T: Clone> TripleBuffer<T> {
    fn new(initial: T) -> Self {
        TripleBuffer {
            buffers: [initial.clone(), initial.clone(), initial],
            write_idx: AtomicUsize::new(0),
            read_idx: AtomicUsize::new(0),
        }
    }
    
    // Called from control thread
    fn write(&mut self, value: T) {
        let write_idx = self.write_idx.load(Ordering::Acquire);
        let next_idx = (write_idx + 1) % 3;
        
        self.buffers[next_idx] = value;
        self.write_idx.store(next_idx, Ordering::Release);
    }
    
    // Called from audio thread
    fn read(&self) -> &T {
        let read_idx = self.read_idx.load(Ordering::Acquire);
        let write_idx = self.write_idx.load(Ordering::Acquire);
        
        if read_idx != write_idx {
            self.read_idx.store(write_idx, Ordering::Release);
        }
        
        &self.buffers[read_idx]
    }
}
```

**Success Criteria:**
- No locks in audio thread
- State updates propagate within 1-2 buffers
- No audio glitches during updates

#### 9c. Disk Streaming (Week 11)

**Goal**: Stream large audio files that don't fit in RAM

```rust
struct StreamingFile {
    id: FileId,
    path: PathBuf,
    channels: u32,
    sample_rate: u32,
    total_frames: u64,
    
    // Streaming state
    buffer_rx: rtrb::Consumer<AudioChunk>,
    current_chunk: Option<AudioChunk>,
    chunk_offset: usize,
}

struct AudioChunk {
    start_frame: u64,
    data: Vec<f32>,
}

// Background streaming thread
fn streaming_thread(
    file_path: PathBuf,
    request_rx: Receiver<StreamRequest>,
    chunk_tx: rtrb::Producer<AudioChunk>,
) {
    let mut file = File::open(file_path).unwrap();
    let mut decoder = /* create decoder */;
    
    loop {
        if let Ok(request) = request_rx.try_recv() {
            // Seek to requested position
            decoder.seek(request.frame);
        }
        
        // Read chunk
        let chunk = decoder.read_frames(CHUNK_SIZE);
        
        // Send to audio thread
        let _ = chunk_tx.push(AudioChunk {
            start_frame: current_frame,
            data: chunk,
        });
        
        current_frame += CHUNK_SIZE;
    }
}
```

**Success Criteria:**
- Can play files larger than RAM
- No dropouts during streaming
- Seek works smoothly
- Multiple streaming files simultaneously

---

### Phase 10: Advanced Features (Week 11+)

#### Automation

```rust
struct AutomationLane {
    parameter_id: u32,
    points: Vec<AutomationPoint>,
}

struct AutomationPoint {
    time: f64,
    value: f32,
    curve: CurveType,
}

enum CurveType {
    Linear,
    Exponential,
    SCurve,
}
```

#### Plugin Hosting (VST/CLAP)

```rust
struct PluginHost {
    scanner: PluginScanner,
    instances: HashMap<PluginId, PluginInstance>,
}

struct PluginInstance {
    id: PluginId,
    plugin_type: PluginType,
    handle: *mut c_void,
    parameters: Vec<Parameter>,
}

enum PluginType {
    VST3,
    CLAP,
}
```

#### Project Save/Load

```rust
#[derive(Serialize, Deserialize)]
struct ProjectFile {
    version: String,
    metadata: ProjectMetadata,
    tracks: Vec<SerializedTrack>,
    audio_files: Vec<AudioFileReference>,
    tempo_map: TempoMap,
}
```

#### Undo/Redo System

```rust
trait Command {
    fn execute(&mut self, project: &mut Project);
    fn undo(&mut self, project: &mut Project);
}

struct CommandHistory {
    commands: Vec<Box<dyn Command>>,
    position: usize,
}
```

---

## Technical Specifications

### Performance Targets

- **Latency**: < 10ms (varies by buffer size and sample rate)
- **CPU Usage**: < 50% for 32 tracks with effects at 44.1kHz
- **Track Count**: Support 64+ tracks without performance degradation
- **Nesting Depth**: 10 levels of metatrack nesting
- **Plugin Count**: 8+ plugins per track

### Buffer Sizes

- **Audio Callback**: 128-512 frames (adjustable)
- **Streaming Chunk**: 8192 frames
- **Ring Buffer**: 8192 samples (UI → Audio commands)

### Sample Rates

- **Supported**: 44.1kHz, 48kHz, 88.2kHz, 96kHz
- **Default**: 48kHz
- **Internal Processing**: Always at project sample rate

### Memory Budget

- **Audio Pool Cache**: 1GB default, configurable
- **Buffer Pool**: 50 buffers × 4096 samples × 4 bytes = 800KB
- **Per-Track Overhead**: < 1KB

### Thread Model

1. **UI Thread**: User interaction, visualization
2. **Control Thread**: Project state management, file I/O
3. **Audio Thread**: Real-time processing (cpal callback)
4. **Streaming Thread(s)**: Disk I/O for large files

### Data Format

- **Internal Audio**: 32-bit float, interleaved
- **File Support**: WAV, FLAC, MP3, OGG via symphonia
- **MIDI**: Standard MIDI File Format
- **Project Files**: JSON or MessagePack

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_clip_rendering_at_position() {
        let clip = Clip {
            id: 0,
            audio_pool_index: 0,
            start_time: 1.0,  // Starts at 1 second
            duration: 2.0,
            offset: 0.0,
            gain: 1.0,
        };
        
        let pool = AudioPool {
            files: vec![vec![1.0; 96000]],  // 2 seconds at 48kHz
        };
        
        let mut output = vec![0.0; 4800];  // 0.1 seconds
        
        render_clip(&clip, &mut output, &pool, 1.5, 48000, 4800);
        
        assert!(output.iter().any(|&s| s != 0.0));
    }
    
    #[test]
    fn test_metatrack_time_stretch() {
        let mut context = RenderContext {
            global_position: 48000,
            local_position: 48000,
            sample_rate: 48000,
            tempo: 120.0,
            time_signature: (4, 4),
            time_stretch: 1.0,
        };
        
        let metatrack = Metatrack {
            id: 0,
            children: vec![],
            effects: vec![],
            time_stretch: 0.5,  // Half speed
            pitch_shift: 0.0,
            offset: 0.0,
            volume: 1.0,
            parent: None,
        };
        
        let child_context = metatrack.transform_context(context);
        
        assert_eq!(child_context.time_stretch, 0.5);
        assert_eq!(child_context.tempo, 60.0);
    }
    
    #[test]
    fn test_buffer_pool_no_allocations() {
        let mut pool = BufferPool::new(10, 1024);
        
        let buf1 = pool.acquire();
        let buf2 = pool.acquire();
        
        pool.release(buf1);
        pool.release(buf2);
        
        let buf3 = pool.acquire();
        let buf4 = pool.acquire();
        
        // Should reuse buffers, no new allocations
        assert_eq!(pool.total_allocations.load(Ordering::Relaxed), 0);
    }
}
```

### Integration Tests

```rust
#[test]
fn test_full_playback_pipeline() {
    // Setup
    let (cmd_tx, cmd_rx) = rtrb::RingBuffer::new(256);
    let (evt_tx, evt_rx) = rtrb::RingBuffer::new(256);
    
    let mut engine = AudioEngine::new(48000, cmd_rx, evt_tx);
    engine.load_audio_file("test.wav");
    
    // Start playback
    cmd_tx.push(Command::Play).unwrap();
    
    // Render some audio
    let mut output = vec![0.0; 4800];
    engine.process(&mut output);
    
    // Verify audio was rendered
    assert!(output.iter().any(|&s| s.abs() > 0.001));
    
    // Check events
    if let Ok(AudioEvent::PlaybackPosition(pos)) = evt_rx.pop() {
        assert!(pos > 0.0);
    }
}

#[test]
fn test_nested_metatrack_rendering() {
    let mut project = Project::new(48000);
    
    // Create structure: Metatrack1 -> Metatrack2 -> AudioTrack
    let audio_id = project.add_audio_track();
    let meta2_id = project.create_metatrack_from_selection(vec![audio_id]);
    let meta1_id = project.create_metatrack_from_selection(vec![meta2_id]);
    
    // Apply transformations
    project.set_metatrack_time_stretch(meta1_id, 0.5);
    project.set_metatrack_time_stretch(meta2_id, 2.0);
    
    // Render
    let mut output = vec![0.0; 4800];
    project.render(&mut output, 0.0);
    
    // Effective stretch should be 0.5 * 2.0 = 1.0 (normal speed)
    assert!(output.iter().any(|&s| s != 0.0));
}
```

### Performance Tests

```rust
#[bench]
fn bench_render_32_tracks(b: &mut Bencher) {
    let mut engine = create_engine_with_tracks(32);
    let mut output = vec![0.0; 512 * 2];
    
    b.iter(|| {
        engine.process(&mut output);
    });
}

#[bench]
fn bench_metatrack_nesting_10_levels(b: &mut Bencher) {
    let mut project = create_nested_metatracks(10);
    let mut output = vec![0.0; 512 * 2];
    
    b.iter(|| {
        project.render(&mut output, 0.0);
    });
}
```

### Audio Quality Tests

- **THD+N**: Total Harmonic Distortion + Noise < 0.01%
- **Frequency Response**: Flat ±0.1dB 20Hz-20kHz
- **Click Detection**: No clicks during parameter changes
- **Timing Accuracy**: MIDI events within ±1 sample

### Stress Tests

- **Long Sessions**: 8+ hours continuous playback
- **Many Tracks**: 128 tracks, 8 effects each
- **Deep Nesting**: 20 levels of metatrack nesting
- **Rapid Commands**: 1000 commands/second
- **Large Files**: 1GB+ audio files streaming

---

## Recommended Crates

### Core Audio
- `cpal = "0.15"` - Audio I/O
- `symphonia = "0.5"` - Audio decoding
- `rubato = "0.14"` - Sample rate conversion

### Concurrency
- `rtrb = "0.3"` - Lock-free ring buffers
- `crossbeam = "0.8"` - Additional concurrency tools
- `parking_lot = "0.12"` - Better mutexes (non-realtime)

### DSP
- `realfft = "3.3"` - FFT for spectral processing
- `biquad = "0.4"` - IIR filters

### Serialization
- `serde = { version = "1.0", features = ["derive"] }`
- `serde_json = "1.0"` or `rmp-serde = "1.1"` (MessagePack)

### File I/O
- `midly = "0.5"` - MIDI file parsing
- `hound = "3.5"` - WAV file writing

### Future
- `vst3-sys` or `clack` - Plugin hosting
- `egui = "0.24"` - Immediate mode GUI (if building UI in Rust)

---

## Project Structure

```
daw-backend/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   │
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── engine.rs          # Audio engine, main processing loop
│   │   ├── track.rs           # Track types (Audio, MIDI, Metatrack)
│   │   ├── clip.rs            # Clip management
│   │   ├── pool.rs            # Audio pool
│   │   ├── buffer_pool.rs     # Buffer allocation pool
│   │   └── render.rs          # Rendering functions
│   │
│   ├── project/
│   │   ├── mod.rs
│   │   ├── project.rs         # Project state
│   │   ├── hierarchy.rs       # Track hierarchy management
│   │   ├── operations.rs      # Project operations (add track, etc.)
│   │   └── serialization.rs   # Save/load
│   │
│   ├── effects/
│   │   ├── mod.rs
│   │   ├── trait.rs           # Effect trait
│   │   ├── gain.rs
│   │   ├── pan.rs
│   │   ├── eq.rs
│   │   └── synth.rs           # Simple synth for MIDI
│   │
│   ├── dsp/
│   │   ├── mod.rs
│   │   ├── filters.rs         # Biquad, etc.
│   │   ├── envelope.rs        # ADSR
│   │   ├── oscillator.rs
│   │   └── utils.rs           # DB conversion, etc.
│   │
│   ├── io/
│   │   ├── mod.rs
│   │   ├── audio_file.rs      # Audio file loading
│   │   ├── midi_file.rs       # MIDI file loading
│   │   └── streaming.rs       # Disk streaming
│   │
│   ├── command/
│   │   ├── mod.rs
│   │   ├── types.rs           # Command/Event enums
│   │   └── queue.rs           # Command queue management
│   │
│   └── ui/
│       ├── mod.rs
│       └── bridge.rs          # UI-Audio communication
│
├── tests/
│   ├── integration_tests.rs
│   └── audio_quality_tests.rs
│
└── benches/
    └── performance.rs
```

---

## Conclusion

This architecture provides:

1. **Incremental Development**: Each phase builds on the last without requiring rewrites
2. **Real-time Safety**: Lock-free, allocation-free audio thread from the start
3. **Flexibility**: Hierarchical tracks support simple projects and complex arrangements
4. **Scalability**: Architecture handles 64+ tracks with deep nesting
5. **Extensibility**: Effect trait makes plugin hosting straightforward

The roadmap gets you from "hello audio" to a full-featured DAW in 11 weeks, with each phase delivering working, testable functionality.

**Next Steps:**
1. Set up Rust project with cpal
2. Implement Phase 1 (single file playback)
3. Add comprehensive tests
4. Profile and optimize as needed
5. Continue through phases sequentially

Good luck building your DAW!