# Lightningbeam Architecture

This document provides a comprehensive overview of Lightningbeam's architecture, design decisions, and component interactions.

## Table of Contents

- [System Overview](#system-overview)
- [Technology Stack](#technology-stack)
- [Component Architecture](#component-architecture)
- [Data Flow](#data-flow)
- [Rendering Pipeline](#rendering-pipeline)
- [Audio Architecture](#audio-architecture)
- [Key Design Decisions](#key-design-decisions)
- [Directory Structure](#directory-structure)

## System Overview

Lightningbeam is a 2D multimedia editor combining vector animation, audio production, and video editing. The application is built as a pure Rust desktop application using immediate-mode GUI (egui) with GPU-accelerated vector rendering (Vello).

### High-Level Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    Lightningbeam Editor                    │
│                         (egui UI)                          │
├────────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐    │
│  │  Stage   │  │ Timeline │  │  Asset   │  │   Info   │    │
│  │   Pane   │  │   Pane   │  │ Library  │  │  Panel   │    │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘    │
│                                                            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           Lightningbeam Core (Data Model)            │  │
│  │    Document, Layers, Clips, Actions, Undo/Redo       │  │
│  └──────────────────────────────────────────────────────┘  │
├────────────────────────────────────────────────────────────┤
│                  Rendering & Audio                         │
│  ┌──────────────────┐        ┌──────────────────┐          │
│  │  Vello + wgpu    │        │   daw-backend    │          │
│  │ (GPU Rendering)  │        │  (Audio Engine)  │          │
│  └──────────────────┘        └──────────────────┘          │
└────────────────────────────────────────────────────────────┘
         ↓                              ↓
    ┌─────────┐                   ┌─────────┐
    │  GPU    │                   │  cpal   │
    │ (Vulkan │                   │ (Audio  │
    │ /Metal) │                   │   I/O)  │
    └─────────┘                   └─────────┘
```

### Migration from Tauri/JavaScript

Lightningbeam is undergoing a rewrite from a Tauri/JavaScript prototype to pure Rust. The original architecture hit IPC bandwidth limitations when streaming decoded video frames. The new Rust UI eliminates this bottleneck by handling all rendering natively.

**Current Status**: Active development on the `rust-ui` branch. Core UI, tools, and undo system are implemented. Audio integration in progress.

## Technology Stack

### UI Framework
- **egui 0.33.3**: Immediate-mode GUI framework
- **eframe 0.33.3**: Application framework wrapping egui
- **winit 0.30**: Cross-platform windowing

### GPU Rendering
- **Vello (git main)**: GPU-accelerated 2D vector graphics using compute shaders
- **wgpu 27**: Low-level GPU API (Vulkan/Metal backend)
- **kurbo 0.12**: 2D curve and shape primitives
- **peniko 0.5**: Color and brush definitions

### Audio Engine
- **daw-backend**: Custom real-time audio engine
- **cpal 0.15**: Cross-platform audio I/O
- **symphonia 0.5**: Audio decoding (MP3, FLAC, WAV, Ogg, etc.)
- **rtrb 0.3**: Lock-free ringbuffers for audio thread communication
- **dasp**: Audio graph processing

### Video
- **FFmpeg**: Video encoding/decoding (via ffmpeg-next)

### Serialization
- **serde**: Document serialization
- **serde_json**: JSON format

## Component Architecture

### 1. Lightningbeam Core (`lightningbeam-core/`)

The core crate contains the data model and business logic, independent of UI framework.

**Key Types:**

```rust
Document {
    canvas_size: (u32, u32),
    layers: Vec<Layer>,
    undo_stack: Vec<Box<dyn Action>>,
    redo_stack: Vec<Box<dyn Action>>,
}

Layer (enum) {
    VectorLayer { clips: Vec<VectorClip>, ... },
    AudioLayer { clips: Vec<AudioClip>, ... },
    VideoLayer { clips: Vec<VideoClip>, ... },
}

ClipInstance {
    clip_id: Uuid,          // Reference to clip definition
    start_time: f64,        // Timeline position
    duration: f64,
    trim_start: f64,
    trim_end: f64,
}
```

**Responsibilities:**
- Document structure and state
- Clip and layer management
- Action system (undo/redo)
- Tool definitions
- Animation data and keyframes

### 2. Lightningbeam Editor (`lightningbeam-editor/`)

The editor application implements the UI and user interactions.

**Main Entry Point:** `src/main.rs`
- Initializes eframe application
- Sets up window, GPU context, and audio system
- Runs main event loop

**Panes** (`src/panes/`):
Each pane is a self-contained UI component:

- `stage.rs` (214KB): Main canvas for drawing, transform tools, GPU rendering
- `timeline.rs` (84KB): Multi-track timeline with clip editing
- `asset_library.rs` (70KB): Asset browser with drag-and-drop
- `infopanel.rs` (31KB): Context-sensitive property editor
- `virtual_piano.rs` (31KB): MIDI keyboard input
- `toolbar.rs` (9KB): Tool palette

**Pane System:**
```rust
pub enum PaneInstance {
    Stage(Stage),
    Timeline(Timeline),
    AssetLibrary(AssetLibrary),
    // ... other panes
}

impl PaneInstance {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        match self {
            PaneInstance::Stage(stage) => stage.render(ui, shared_state),
            // ... dispatch to specific pane
        }
    }
}
```

**SharedPaneState:**
Facilitates communication between panes:
```rust
pub struct SharedPaneState {
    pub document: Document,
    pub selected_tool: Tool,
    pub pending_actions: Vec<Box<dyn Action>>,
    pub audio_system: AudioSystem,
    // ... other shared state
}
```

### 3. DAW Backend (`daw-backend/`)

Standalone audio engine crate with real-time audio processing.

**Architecture:**
```
UI Thread                    Audio Thread (real-time)
    │                               │
    │  Commands (rtrb queue)        │
    ├──────────────────────────────>│
    │                               │
    │       State Updates           │
    │<──────────────────────────────┤
    │                               │
                                    ↓
                            ┌───────────────┐
                            │ Audio Engine  │
                            │   process()   │
                            └───────────────┘
                                    ↓
                            ┌───────────────┐
                            │  Track Mix    │
                            └───────────────┘
                                    ↓
                            ┌───────────────┐
                            │  cpal Output  │
                            └───────────────┘
```

**Key Components:**

- **Engine** (`audio/engine.rs`): Main audio callback, runs on real-time thread
- **Project** (`audio/project.rs`): Top-level audio state
- **Track** (`audio/track.rs`): Individual audio tracks with effects chains
- **Effects**: Reverb, delay, EQ, compressor, distortion, etc.
- **Synthesizers**: Oscillator, FM synth, wavetable, sampler

**Lock-Free Design:**
The audio thread never blocks. UI sends commands via lock-free ringbuffers (rtrb), audio thread processes them between buffer callbacks.

## Data Flow

### Document Editing Flow

```
User Input (mouse/keyboard)
    ↓
egui Event Handlers (in pane.render())
    ↓
Create Action (implements Action trait)
    ↓
Add to SharedPaneState.pending_actions
    ↓
After all panes render: execute actions
    ↓
Action.apply(&mut document)
    ↓
Push to undo_stack
    ↓
UI re-renders with updated document
```

### Audio Playback Flow

```
UI: User clicks Play
    ↓
Send PlayCommand to audio engine (via rtrb queue)
    ↓
Audio thread: Receive command
    ↓
Audio thread: Start playback, increment playhead
    ↓
Audio callback (every ~5ms): Engine::process()
    ↓
Mix tracks, apply effects, output samples
    ↓
Send playhead position back to UI
    ↓
UI: Update timeline playhead position
```

### GPU Rendering Flow

```
egui layout phase
    ↓
Stage pane requests wgpu callback
    ↓
Vello renders vector shapes to GPU texture
    ↓
Custom wgpu integration composites:
  - Vello output (vector graphics)
  - Waveform textures (GPU-rendered audio)
  - egui UI overlay
    ↓
Present to screen
```

## Rendering Pipeline

### Stage Rendering

The Stage pane uses a custom wgpu callback to render directly to GPU:

```rust
ui.painter().add(egui_wgpu::Callback::new_paint_callback(
    rect,
    StageCallback { /* render data */ }
));
```

**Vello Integration:**
1. Create Vello `Scene` from document shapes
2. Render scene to GPU texture using compute shaders
3. Composite with UI elements

**Waveform Rendering:**
- Audio waveforms rendered on GPU using custom WGSL shaders
- Mipmaps generated via compute shader for level-of-detail
- Uniform buffers store view parameters (zoom, offset, tint color)

**WGSL Alignment Requirements:**
WGSL has strict alignment rules. `vec4<f32>` requires 16-byte alignment:

```rust
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveformParams {
    view_matrix: [f32; 16],      // 64 bytes
    viewport_size: [f32; 2],     // 8 bytes
    zoom: f32,                   // 4 bytes
    _pad1: f32,                  // 4 bytes padding
    tint_color: [f32; 4],        // 16 bytes (requires 16-byte alignment)
}
// Total: 96 bytes
```

## Audio Architecture

### Real-Time Constraints

Audio callbacks run on a dedicated real-time thread with strict timing requirements:
- Buffer size: 256 frames default (~5.8ms at 44.1kHz)
- ALSA may provide smaller buffers (64-75 frames, ~1.5ms)
- **No blocking operations allowed**: No locks, no allocations, no syscalls

### Lock-Free Communication

UI and audio thread communicate via lock-free ringbuffers (rtrb):

```rust
// UI Thread
command_sender.push(AudioCommand::Play).ok();

// Audio Thread (in process callback)
while let Ok(command) = command_receiver.pop() {
    match command {
        AudioCommand::Play => self.playing = true,
        // ... handle other commands
    }
}
```

### Audio Processing Pipeline

```
Audio Callback Invoked (every ~5ms)
    ↓
Process queued commands
    ↓
For each track:
  - Read audio samples at playhead position
  - Apply effects chain
  - Mix to master output
    ↓
Write samples to output buffer
    ↓
Return from callback (must complete in <5ms)
```

### Optimized Debug Builds

Audio code is optimized even in debug builds to meet real-time deadlines:

```toml
[profile.dev.package.daw-backend]
opt-level = 2

[profile.dev.package.symphonia]
opt-level = 2
# ... other audio libraries
```

## Key Design Decisions

### Layer & Clip System

**Type-Specific Layers:**
Each layer type supports only its matching clip type:
- `VectorLayer` → `VectorClip`
- `AudioLayer` → `AudioClip`
- `VideoLayer` → `VideoClip`

**Recursive Nesting:**
Vector clips can contain internal layers of any type, enabling complex nested compositions.

**Clip vs ClipInstance:**
- **Clip**: Template/definition in asset library (the "master")
- **ClipInstance**: Placed on timeline with instance-specific properties (position, duration, trim points)
- Multiple instances can reference the same clip
- "Make Unique" operation duplicates the underlying clip

### Undo/Redo System

**Action Trait:**
```rust
pub trait Action: Send {
    fn apply(&mut self, document: &mut Document);
    fn undo(&mut self, document: &mut Document);
    fn redo(&mut self, document: &mut Document);
}
```

All operations (drawing, editing, clip manipulation) implement this trait.

**Continuous Operations:**
Dragging sliders or scrubbing creates only one undo action when complete, not one per frame.

### Two-Phase Dispatch Pattern

Panes cannot directly mutate shared state during rendering (borrowing rules). Instead:

1. **Phase 1 (Render)**: Panes register actions
   ```rust
   shared_state.register_action(Box::new(MyAction { ... }));
   ```

2. **Phase 2 (Execute)**: After all panes rendered, execute actions
   ```rust
   for action in shared_state.pending_actions.drain(..) {
       action.apply(&mut document);
       undo_stack.push(action);
   }
   ```

### Pane ID Salting

egui uses IDs to track widget state. Multiple instances of the same pane would collide without unique IDs.

**Solution**: Salt all IDs with the pane's node path:
```rust
ui.horizontal(|ui| {
    ui.label("My Widget");
}).id.with(&node_path);
```

### Selection & Clipboard

- **Selection scope**: Limited to current clip/layer
- **Type-aware paste**: Content must match target type
- **Clip instance copying**: Creates reference to same underlying clip
- **Make unique**: Duplicates underlying clip for independent editing

## Directory Structure

```
lightningbeam-2/
├── lightningbeam-ui/              # Rust UI workspace
│   ├── Cargo.toml                 # Workspace manifest
│   ├── lightningbeam-editor/      # Main application crate
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── main.rs            # Entry point, event loop
│   │       ├── app.rs             # Application state
│   │       ├── panes/
│   │       │   ├── mod.rs         # Pane system dispatch
│   │       │   ├── stage.rs       # Main canvas
│   │       │   ├── timeline.rs    # Timeline editor
│   │       │   ├── asset_library.rs
│   │       │   └── ...
│   │       ├── tools/             # Drawing and editing tools
│   │       ├── rendering/
│   │       │   ├── vello_integration.rs
│   │       │   ├── waveform_gpu.rs
│   │       │   └── shaders/
│   │       │       ├── waveform.wgsl
│   │       │       └── waveform_mipgen.wgsl
│   │       └── export/            # Export functionality
│   └── lightningbeam-core/        # Core data model crate
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── document.rs        # Document structure
│           ├── layer.rs           # Layer types
│           ├── clip.rs            # Clip types and instances
│           ├── shape.rs           # Shape definitions
│           ├── action.rs          # Action trait and undo/redo
│           ├── animation.rs       # Keyframe animation
│           └── tools.rs           # Tool definitions
│
├── daw-backend/                   # Audio engine (standalone)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                 # Audio system initialization
│       ├── audio/
│       │   ├── engine.rs          # Main audio callback
│       │   ├── track.rs           # Track management
│       │   ├── project.rs         # Project state
│       │   └── buffer.rs          # Audio buffer utilities
│       ├── effects/               # Audio effects
│       │   ├── reverb.rs
│       │   ├── delay.rs
│       │   └── ...
│       ├── synth/                 # Synthesizers
│       └── midi/                  # MIDI handling
│
├── src-tauri/                     # Legacy Tauri backend
├── src/                           # Legacy JavaScript frontend
├── CONTRIBUTING.md                # Contributor guide
├── ARCHITECTURE.md                # This file
├── README.md                      # Project overview
└── docs/                          # Additional documentation
    ├── AUDIO_SYSTEM.md
    ├── UI_SYSTEM.md
    └── ...
```

## Performance Considerations

### GPU Rendering
- Vello uses compute shaders for efficient 2D rendering
- Waveforms pre-rendered on GPU with mipmaps for smooth zooming
- Custom wgpu integration minimizes CPU↔GPU data transfer

### Audio Processing
- Lock-free design: No blocking in audio thread
- Optimized even in debug builds (`opt-level = 2`)
- Memory-mapped file I/O for large audio files
- Zero-copy audio buffers where possible

### Memory Management
- Audio buffers pre-allocated, no allocations in audio thread
- Vello manages GPU memory automatically
- Document structure uses `Rc`/`Arc` for shared clip references

## Future Considerations

### Video Integration
Video decoding has been ported from the legacy Tauri backend. Video soundtracks become audio tracks in daw-backend, enabling full effects processing.

### File Format
The .beam file format is not yet finalized. Considerations:
- Single JSON file vs container format (e.g., ZIP)
- Embedded media vs external references
- Forward/backward compatibility strategy

### Node Editor
Primary use: Audio effects chains and modular synthesizers. Future expansion to visual effects and procedural generation is possible.

## Related Documentation

- [CONTRIBUTING.md](CONTRIBUTING.md) - Development setup and workflow
- [docs/AUDIO_SYSTEM.md](docs/AUDIO_SYSTEM.md) - Detailed audio engine documentation
- [docs/UI_SYSTEM.md](docs/UI_SYSTEM.md) - UI pane system details
- [docs/RENDERING.md](docs/RENDERING.md) - GPU rendering pipeline
- [Claude.md](Claude.md) - Comprehensive architectural reference for AI assistants
