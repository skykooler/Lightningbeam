# UI System Architecture

This document describes Lightningbeam's UI architecture, including the pane system, tool system, GPU integration, and patterns for extending the UI with new features.

## Table of Contents

- [Overview](#overview)
- [Pane System](#pane-system)
- [Shared State](#shared-state)
- [Two-Phase Dispatch](#two-phase-dispatch)
- [ID Collision Avoidance](#id-collision-avoidance)
- [Tool System](#tool-system)
- [GPU Integration](#gpu-integration)
- [Adding New Panes](#adding-new-panes)
- [Adding New Tools](#adding-new-tools)
- [Event Handling](#event-handling)
- [Best Practices](#best-practices)

## Overview

Lightningbeam's UI is built with **egui**, an immediate-mode GUI framework. Unlike retained-mode frameworks (Qt, GTK), immediate-mode rebuilds the UI every frame by running code that describes what should be displayed.

### Key Technologies

- **egui 0.33.3**: Immediate-mode GUI framework
- **eframe**: Application framework wrapping egui
- **winit**: Cross-platform windowing
- **Vello**: GPU-accelerated 2D vector rendering
- **wgpu**: Low-level GPU API
- **egui-wgpu**: Integration layer between egui and wgpu

### Immediate Mode Overview

```rust
// Immediate mode: UI is described every frame
fn render(&mut self, ui: &mut egui::Ui) {
    if ui.button("Click me").clicked() {
        self.counter += 1;
    }
    ui.label(format!("Count: {}", self.counter));
}
```

**Benefits**:
- Simple mental model (just describe what you see)
- No manual synchronization between state and UI
- Easy to compose and reuse components

**Considerations**:
- Must avoid expensive operations in render code
- IDs needed for stateful widgets (handled automatically in most cases)

## Pane System

Lightningbeam uses a flexible pane system where the UI is composed of independent, reusable panes (Stage, Timeline, Asset Library, etc.).

### Pane Architecture

```
┌─────────────────────────────────────────────────────────┐
│                   Main Application                       │
│                    (LightningbeamApp)                    │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  ┌────────────────────────────────────────────────┐    │
│  │           Pane Tree (egui_tiles)               │    │
│  │                                                │    │
│  │   ┌──────────┐  ┌──────────┐  ┌──────────┐   │    │
│  │   │  Stage   │  │ Timeline │  │  Asset   │   │    │
│  │   │  Pane    │  │   Pane   │  │ Library  │   │    │
│  │   └──────────┘  └──────────┘  └──────────┘   │    │
│  │                                                │    │
│  │   Each pane:                                   │    │
│  │   - Renders its UI                             │    │
│  │   - Registers actions with SharedPaneState     │    │
│  │   - Accesses shared document state             │    │
│  └────────────────────────────────────────────────┘    │
│                                                          │
│  ┌────────────────────────────────────────────────┐    │
│  │         SharedPaneState                         │    │
│  │  - Document                                     │    │
│  │  - Selected tool                                │    │
│  │  - Pending actions                              │    │
│  │  - Audio system                                 │    │
│  └────────────────────────────────────────────────┘    │
│                                                          │
│  After all panes render:                                │
│  - Execute pending actions                              │
│  - Update undo/redo stacks                              │
│  - Synchronize with audio engine                        │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

### PaneInstance Enum

All panes are variants of the `PaneInstance` enum:

```rust
// In lightningbeam-editor/src/panes/mod.rs
pub enum PaneInstance {
    Stage(Stage),
    Timeline(Timeline),
    AssetLibrary(AssetLibrary),
    InfoPanel(InfoPanel),
    VirtualPiano(VirtualPiano),
    Toolbar(Toolbar),
    NodeEditor(NodeEditor),
    PianoRoll(PianoRoll),
    Outliner(Outliner),
    PresetBrowser(PresetBrowser),
}

impl PaneInstance {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        match self {
            PaneInstance::Stage(stage) => stage.render(ui, shared_state),
            PaneInstance::Timeline(timeline) => timeline.render(ui, shared_state),
            PaneInstance::AssetLibrary(lib) => lib.render(ui, shared_state),
            // ... dispatch to specific pane
        }
    }

    pub fn title(&self) -> &str {
        match self {
            PaneInstance::Stage(_) => "Stage",
            PaneInstance::Timeline(_) => "Timeline",
            // ...
        }
    }
}
```

### Individual Pane Structure

Each pane is a struct with its own state and a `render` method:

```rust
pub struct MyPane {
    // Pane-specific state
    scroll_offset: f32,
    selected_item: Option<usize>,
    // ... other state
}

impl MyPane {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0.0,
            selected_item: None,
        }
    }

    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        // Render pane UI
        ui.heading("My Pane");

        // Access shared state
        let document = &shared_state.document;

        // Create actions
        if ui.button("Do something").clicked() {
            let action = Box::new(MyAction { /* ... */ });
            shared_state.pending_actions.push(action);
        }
    }
}
```

### Key Panes

Located in `lightningbeam-editor/src/panes/`:

- **stage.rs** (214KB): Main canvas for drawing and transform tools
- **timeline.rs** (84KB): Multi-track timeline with clip editing
- **asset_library.rs** (70KB): Asset browser with drag-to-timeline
- **infopanel.rs** (31KB): Context-sensitive property editor
- **virtual_piano.rs** (31KB): On-screen MIDI keyboard
- **toolbar.rs** (9KB): Tool palette

## Shared State

`SharedPaneState` is passed to all panes during rendering to share data and coordinate actions.

### SharedPaneState Structure

```rust
pub struct SharedPaneState {
    // Document state
    pub document: Document,
    pub undo_stack: Vec<Box<dyn Action>>,
    pub redo_stack: Vec<Box<dyn Action>>,

    // Tool state
    pub selected_tool: Tool,
    pub tool_state: ToolState,

    // Actions to execute after rendering
    pub pending_actions: Vec<Box<dyn Action>>,

    // Audio engine
    pub audio_system: AudioSystem,
    pub playhead_position: f64,
    pub is_playing: bool,

    // Selection state
    pub selected_clips: HashSet<Uuid>,
    pub selected_shapes: HashSet<Uuid>,

    // Clipboard
    pub clipboard: Option<ClipboardData>,

    // UI state
    pub show_grid: bool,
    pub snap_to_grid: bool,
    pub grid_size: f32,
}
```

### Accessing Shared State

```rust
impl MyPane {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        // Read from document
        let layer_count = shared_state.document.layers.len();
        ui.label(format!("Layers: {}", layer_count));

        // Check tool state
        if shared_state.selected_tool == Tool::Select {
            // ... render selection-specific UI
        }

        // Check playback state
        if shared_state.is_playing {
            ui.label("▶ Playing");
        }
    }
}
```

## Two-Phase Dispatch

Panes cannot directly mutate shared state during rendering due to Rust's borrowing rules. Instead, they register actions to be executed after all panes have rendered.

### Why Two-Phase?

```rust
// This doesn't work: can't borrow shared_state as mutable twice
pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
    if ui.button("Add layer").clicked() {
        // ❌ Can't mutate document while borrowed by render
        shared_state.document.layers.push(Layer::new());
    }
}
```

### Solution: Pending Actions

```rust
// Phase 1: Register action during render
pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
    if ui.button("Add layer").clicked() {
        let action = Box::new(AddLayerAction::new());
        shared_state.pending_actions.push(action);
    }
}

// Phase 2: Execute after all panes rendered (in main app)
for action in shared_state.pending_actions.drain(..) {
    action.apply(&mut shared_state.document);
    shared_state.undo_stack.push(action);
}
```

### Action Trait

All actions implement the `Action` trait:

```rust
pub trait Action: Send {
    fn apply(&mut self, document: &mut Document);
    fn undo(&mut self, document: &mut Document);
    fn redo(&mut self, document: &mut Document);
}
```

Example action:

```rust
pub struct AddLayerAction {
    layer_id: Uuid,
    layer_type: LayerType,
}

impl Action for AddLayerAction {
    fn apply(&mut self, document: &mut Document) {
        let layer = Layer::new(self.layer_id, self.layer_type);
        document.layers.push(layer);
    }

    fn undo(&mut self, document: &mut Document) {
        document.layers.retain(|l| l.id != self.layer_id);
    }

    fn redo(&mut self, document: &mut Document) {
        self.apply(document);
    }
}
```

## ID Collision Avoidance

egui uses IDs to track widget state across frames (e.g., scroll position, collapse state). When multiple instances of the same pane exist, IDs can collide.

### The Problem

```rust
// If two Timeline panes exist, they'll share the same ID
ui.collapsing("Track 1", |ui| {
    // ... content
}); // ID is derived from label "Track 1"
```

Both timeline instances would have the same "Track 1" ID, causing state conflicts.

### Solution: Salt IDs with Node Path

Each pane has a unique node path (e.g., `"root/0/1/2"`). Salt all IDs with this path:

```rust
pub struct Timeline {
    node_path: String, // Unique path for this pane instance
}

impl Timeline {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        // Salt IDs with node path
        ui.push_id(&self.node_path, |ui| {
            // Now all IDs within this closure are unique to this instance
            ui.collapsing("Track 1", |ui| {
                // ... content
            });
        });
    }
}
```

### Alternative: Per-Widget Salting

For individual widgets:

```rust
ui.collapsing("Track 1", |ui| {
    // ... content
}).id.with(&self.node_path); // Salt this specific ID
```

### Best Practice

**Always salt IDs in new panes** to support multiple instances:

```rust
impl NewPane {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        ui.push_id(&self.node_path, |ui| {
            // All rendering code goes here
        });
    }
}
```

## Tool System

Tools handle user input on the Stage pane (drawing, selection, transforms, etc.).

### Tool Enum

```rust
pub enum Tool {
    Select,
    Draw,
    Rectangle,
    Ellipse,
    Line,
    PaintBucket,
    Transform,
    Eyedropper,
}
```

### Tool State

```rust
pub struct ToolState {
    // Generic tool state
    pub mouse_pos: Pos2,
    pub mouse_down: bool,
    pub drag_start: Option<Pos2>,

    // Tool-specific state
    pub draw_points: Vec<Pos2>,
    pub transform_mode: TransformMode,
    pub paint_bucket_tolerance: f32,
}
```

### Tool Implementation

Tools implement the `ToolBehavior` trait:

```rust
pub trait ToolBehavior {
    fn on_mouse_down(&mut self, pos: Pos2, shared_state: &mut SharedPaneState);
    fn on_mouse_move(&mut self, pos: Pos2, shared_state: &mut SharedPaneState);
    fn on_mouse_up(&mut self, pos: Pos2, shared_state: &mut SharedPaneState);
    fn on_key(&mut self, key: Key, shared_state: &mut SharedPaneState);
    fn render_overlay(&self, painter: &Painter);
}
```

Example: Rectangle tool:

```rust
pub struct RectangleTool {
    start_pos: Option<Pos2>,
}

impl ToolBehavior for RectangleTool {
    fn on_mouse_down(&mut self, pos: Pos2, _shared_state: &mut SharedPaneState) {
        self.start_pos = Some(pos);
    }

    fn on_mouse_move(&mut self, pos: Pos2, _shared_state: &mut SharedPaneState) {
        // Visual feedback handled in render_overlay
    }

    fn on_mouse_up(&mut self, pos: Pos2, shared_state: &mut SharedPaneState) {
        if let Some(start) = self.start_pos.take() {
            // Create rectangle shape
            let rect = Rect::from_two_pos(start, pos);
            let action = Box::new(AddShapeAction::rectangle(rect));
            shared_state.pending_actions.push(action);
        }
    }

    fn render_overlay(&self, painter: &Painter) {
        if let Some(start) = self.start_pos {
            let current = painter.mouse_pos();
            let rect = Rect::from_two_pos(start, current);
            painter.rect_stroke(rect, 0.0, Stroke::new(2.0, Color32::WHITE));
        }
    }
}
```

### Tool Selection

```rust
// In Toolbar pane
if ui.button("✏ Draw").clicked() {
    shared_state.selected_tool = Tool::Draw;
}

// In Stage pane
match shared_state.selected_tool {
    Tool::Draw => self.draw_tool.on_mouse_move(pos, shared_state),
    Tool::Select => self.select_tool.on_mouse_move(pos, shared_state),
    // ...
}
```

## GPU Integration

The Stage pane uses custom wgpu rendering for vector graphics and waveforms.

### egui-wgpu Callbacks

```rust
// In Stage::render()
ui.painter().add(egui_wgpu::Callback::new_paint_callback(
    rect,
    StageCallback {
        document: shared_state.document.clone(),
        vello_renderer: self.vello_renderer.clone(),
        waveform_renderer: self.waveform_renderer.clone(),
    },
));
```

### Callback Implementation

```rust
struct StageCallback {
    document: Document,
    vello_renderer: Arc<Mutex<VelloRenderer>>,
    waveform_renderer: Arc<Mutex<WaveformRenderer>>,
}

impl egui_wgpu::CallbackTrait for StageCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        resources: &egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Prepare GPU resources
        let mut vello = self.vello_renderer.lock().unwrap();
        vello.prepare_scene(&self.document);

        vec![]
    }

    fn paint<'a>(
        &'a self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        resources: &'a egui_wgpu::CallbackResources,
    ) {
        // Render vector graphics
        let vello = self.vello_renderer.lock().unwrap();
        vello.render(render_pass);

        // Render waveforms
        let waveforms = self.waveform_renderer.lock().unwrap();
        waveforms.render(render_pass);
    }
}
```

### Vello Integration

Vello renders 2D vector graphics using GPU compute shaders:

```rust
use vello::{Scene, SceneBuilder, kurbo};

fn build_vello_scene(document: &Document) -> Scene {
    let mut scene = Scene::new();
    let mut builder = SceneBuilder::for_scene(&mut scene);

    for layer in &document.layers {
        if let Layer::VectorLayer { clips, .. } = layer {
            for clip in clips {
                for shape in &clip.shapes {
                    // Convert shape to kurbo path
                    let path = shape.to_kurbo_path();

                    // Add to scene with fill/stroke
                    builder.fill(
                        Fill::NonZero,
                        Affine::IDENTITY,
                        &shape.fill_color,
                        None,
                        &path,
                    );
                }
            }
        }
    }

    scene
}
```

## Adding New Panes

### Step 1: Create Pane Struct

```rust
// In lightningbeam-editor/src/panes/my_pane.rs
pub struct MyPane {
    node_path: String,
    // Pane-specific state
    selected_index: usize,
    scroll_offset: f32,
}

impl MyPane {
    pub fn new(node_path: String) -> Self {
        Self {
            node_path,
            selected_index: 0,
            scroll_offset: 0.0,
        }
    }

    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        // IMPORTANT: Salt IDs with node path
        ui.push_id(&self.node_path, |ui| {
            ui.heading("My Pane");

            // Render pane content
            // ...
        });
    }
}
```

### Step 2: Add to PaneInstance Enum

```rust
// In lightningbeam-editor/src/panes/mod.rs
pub enum PaneInstance {
    // ... existing variants
    MyPane(MyPane),
}

impl PaneInstance {
    pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
        match self {
            // ... existing cases
            PaneInstance::MyPane(pane) => pane.render(ui, shared_state),
        }
    }

    pub fn title(&self) -> &str {
        match self {
            // ... existing cases
            PaneInstance::MyPane(_) => "My Pane",
        }
    }
}
```

### Step 3: Add to Menu

```rust
// In main application
if ui.button("My Pane").clicked() {
    let pane = PaneInstance::MyPane(MyPane::new(generate_node_path()));
    app.add_pane(pane);
}
```

## Adding New Tools

### Step 1: Add to Tool Enum

```rust
pub enum Tool {
    // ... existing tools
    MyTool,
}
```

### Step 2: Implement Tool Behavior

```rust
pub struct MyToolState {
    // Tool-specific state
    start_pos: Option<Pos2>,
}

impl MyToolState {
    pub fn handle_input(
        &mut self,
        response: &Response,
        shared_state: &mut SharedPaneState,
    ) {
        if response.clicked() {
            self.start_pos = response.interact_pointer_pos();
        }

        if response.drag_released() {
            if let Some(start) = self.start_pos.take() {
                // Create action
                let action = Box::new(MyAction { /* ... */ });
                shared_state.pending_actions.push(action);
            }
        }
    }

    pub fn render_overlay(&self, painter: &Painter) {
        // Draw tool-specific overlay
    }
}
```

### Step 3: Add to Toolbar

```rust
// In Toolbar pane
if ui.button("🔧 My Tool").clicked() {
    shared_state.selected_tool = Tool::MyTool;
}
```

### Step 4: Handle in Stage Pane

```rust
// In Stage pane
match shared_state.selected_tool {
    // ... existing tools
    Tool::MyTool => self.my_tool_state.handle_input(&response, shared_state),
}

// Render overlay
match shared_state.selected_tool {
    // ... existing tools
    Tool::MyTool => self.my_tool_state.render_overlay(&painter),
}
```

## Event Handling

### Mouse Events

```rust
let response = ui.allocate_rect(rect, Sense::click_and_drag());

if response.clicked() {
    let pos = response.interact_pointer_pos().unwrap();
    // Handle click at pos
}

if response.dragged() {
    let delta = response.drag_delta();
    // Handle drag by delta
}

if response.drag_released() {
    // Handle drag end
}
```

### Keyboard Events

```rust
ui.input(|i| {
    if i.key_pressed(Key::Delete) {
        // Delete selected items
    }

    if i.modifiers.ctrl && i.key_pressed(Key::Z) {
        // Undo
    }

    if i.modifiers.ctrl && i.key_pressed(Key::Y) {
        // Redo
    }
});
```

### Drag and Drop

```rust
// Source (Asset Library)
let response = ui.label("Audio Clip");
if response.dragged() {
    let payload = DragPayload::AudioClip(clip_id);
    ui.memory_mut(|mem| {
        mem.data.insert_temp(Id::new("drag_payload"), payload);
    });
}

// Target (Timeline)
let response = ui.allocate_rect(rect, Sense::hover());
if response.hovered() {
    if let Some(payload) = ui.memory(|mem| mem.data.get_temp::<DragPayload>(Id::new("drag_payload"))) {
        // Handle drop
        let action = Box::new(AddClipAction { clip_id: payload.clip_id(), position });
        shared_state.pending_actions.push(action);
    }
}
```

## Best Practices

### 1. Always Salt IDs

```rust
// ✅ Good
ui.push_id(&self.node_path, |ui| {
    // All rendering here
});

// ❌ Bad (ID collisions if multiple instances)
ui.collapsing("Settings", |ui| {
    // ...
});
```

### 2. Use Pending Actions

```rust
// ✅ Good
shared_state.pending_actions.push(Box::new(action));

// ❌ Bad (borrowing conflicts)
shared_state.document.layers.push(layer);
```

### 3. Split Borrows with std::mem::take

```rust
// ✅ Good
let mut clips = std::mem::take(&mut self.clips);
for clip in &mut clips {
    self.render_clip(ui, clip); // Can borrow self immutably
}
self.clips = clips;

// ❌ Bad (can't borrow self while iterating clips)
for clip in &mut self.clips {
    self.render_clip(ui, clip); // Error!
}
```

### 4. Avoid Expensive Operations in Render

```rust
// ❌ Bad (heavy computation every frame)
pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
    let thumbnail = self.generate_thumbnail(); // Expensive!
    ui.image(thumbnail);
}

// ✅ Good (cache result)
pub fn render(&mut self, ui: &mut Ui, shared_state: &mut SharedPaneState) {
    if self.thumbnail_cache.is_none() {
        self.thumbnail_cache = Some(self.generate_thumbnail());
    }
    ui.image(self.thumbnail_cache.as_ref().unwrap());
}
```

### 5. Handle Missing State Gracefully

```rust
// ✅ Good
if let Some(layer) = document.layers.get(layer_index) {
    // Render layer
} else {
    ui.label("Layer not found");
}

// ❌ Bad (panics if layer missing)
let layer = &document.layers[layer_index]; // May panic!
```

## Related Documentation

- [ARCHITECTURE.md](../ARCHITECTURE.md) - Overall system architecture
- [docs/AUDIO_SYSTEM.md](AUDIO_SYSTEM.md) - Audio engine integration
- [docs/RENDERING.md](RENDERING.md) - GPU rendering details
- [CONTRIBUTING.md](../CONTRIBUTING.md) - Development workflow
