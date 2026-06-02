# Lightningbeam Rust UI - Implementation Plan

## Project Overview

**Goal**: Complete migration from JavaScript/Tauri to Rust/egui
- **Scope**: ~10,000+ lines of code migration
- **Source**: `~/Dev/Lightningbeam-2/src/` (JS models, actions, main.js)
- **Target**: Full-featured Rust animation editor with native performance
- **Motivation**: IPC overhead between Rust↔JS too slow for real-time performance

## UI Boundaries

**In Scope (This UI)**:
- Layout system (panes, splits, resize)
- Stage rendering (layer compositing with Vello)
- Timeline (frame scrubbing, keyframes)
- Tools (pen, select, transform)
- Property panels
- User interaction & editing

**Out of Scope (External Systems)**:
- **Video import/export**: Handled by separate video processing module
- **Audio playback/processing**: Handled by `daw-backend`
- **File I/O**: Coordinated with backend systems
- **Plugin architecture**: TBD in separate crate

## Technology Stack

### UI Framework
- **Primary**: `eframe` 0.29 + `egui` (immediate-mode GUI)
- **Theming**: `egui-aesthetix` for professional appearance
- **Windowing**: `winit` 0.30
- **Native Menus**: `muda` for OS-integrated menus (File, Edit, etc.)

### Rendering
- **GPU**: `wgpu` 22 for low-level GPU access
- **2D Graphics**: `Vello` 0.3 for high-performance vector rendering
  - 84% faster than iced in benchmarks
  - Used for: Stage, Timeline, Node Editor, Virtual Piano
- **Architecture**: Layer-based rendering
  - Each layer (2D animation, video, etc.) renders to texture
  - Textures composited together on Stage canvas

### Serialization
- **Format**: JSON (serde_json)
- **Compatibility**: Match existing JS JSON schema for layouts

---

## Implementation Phases

### ✅ Phase 1: Layout System (COMPLETE)

**Status**: Fully implemented and tested

**Features**:
- [x] Workspace structure (lightningbeam-core + lightningbeam-editor)
- [x] JSON layout loading (8 predefined layouts)
- [x] Recursive pane tree rendering
- [x] Layout switching via menu
- [x] Drag-to-resize dividers with visual feedback
- [x] Split operations with live preview
- [x] Join operations (remove splits)
- [x] Context menus on dividers
- [x] ESC/click-outside cancellation
- [x] Pane selection and type switching

**Files**:
- `lightningbeam-core/src/layout.rs` - Core data structures
- `lightningbeam-editor/src/main.rs` - Rendering and interaction
- `assets/layouts.json` - Layout definitions

---

### 🔄 Phase 2: Pane Architecture (CURRENT)

**Goal**: Define proper pane abstraction with header + content sections

**Requirements**:
1. **Pane Trait/Struct**:
   ```rust
   trait Pane {
       fn header(&mut self, ui: &mut egui::Ui) -> Option<Response>;
       fn content(&mut self, ui: &mut egui::Ui, rect: Rect);
       fn name(&self) -> &str;
   }
   ```

2. **Header Section**:
   - Optional controls (used by Timeline pane)
   - Play/pause, zoom, frame counter
   - Collapsible/expandable

3. **Content Section**:
   - Main pane body
   - Custom rendering per pane type
   - Can use egui widgets or custom GPU rendering

4. **Integration**:
   - Update `render_pane()` to use new trait
   - Support dynamic pane instantiation
   - Maintain layout tree structure

**Deliverables**:
- [ ] Pane trait in `lightningbeam-core/src/pane.rs`
- [ ] Example placeholder pane implementation
- [ ] Update main.rs to use pane trait
- [ ] Document pane interface

---

### Phase 3: Native Menu Integration

**Goal**: OS-integrated menu bar using `muda`

**Features**:
- [ ] File menu (New, Open, Save, Export)
- [ ] Edit menu (Undo, Redo, Cut, Copy, Paste)
- [ ] View menu (Layouts, Zoom, Panels)
- [ ] Help menu (Documentation, About)
- [ ] Platform-specific integration (macOS menu bar, Windows menu bar)
- [ ] Keyboard shortcuts

**Dependencies**:
```toml
muda = "*"  # Native menu system (what Tauri uses)
```

---

### Phase 4: Stage Pane Implementation

**Goal**: Main canvas with Vello-based layer compositing

**Architecture**:
```
Stage Pane
├── wgpu Surface
├── Vello Scene
└── Layer Renderer
    ├── Layer 1 → Texture
    ├── Layer 2 → Texture
    ├── Layer 3 → Texture (video from external)
    └── Composite → Final render
```

**Features**:
- [ ] wgpu surface integration with egui
- [ ] Vello scene management
- [ ] Layer texture rendering
- [ ] Compositing pipeline
- [ ] Camera/viewport controls (pan, zoom)
- [ ] Selection visualization
- [ ] Tool integration (later phase)

**Performance Targets**:
- 60 FPS at 1920x1080
- Sub-16ms frame time
- Smooth layer compositing

---

### Phase 5: Timeline Pane Implementation

**Goal**: Frame-based animation timeline with Vello rendering

**Features**:
- [ ] Frame scrubber with Vello
- [ ] Layer tracks
- [ ] Keyframe visualization
- [ ] Playback controls in header
- [ ] Zoom/pan timeline
- [ ] Frame selection
- [ ] Drag keyframes
- [ ] Multi-selection

**Header Controls**:
- Play/pause button
- Current frame indicator
- FPS selector
- Zoom slider

**Audio Integration**:
- Display audio waveforms (data from `daw-backend`)
- Sync playback with audio system
- No direct audio processing in UI

---

### Phase 6: Additional Panes

**Priority Order**:
1. **Toolbar** (simple, egui widgets)
2. **Info Panel** (property editor, egui widgets)
3. **Outliner** (layer hierarchy, egui tree)
4. **Node Editor** (Vello-based graph)
5. **Piano Roll** (MIDI editor, Vello)
6. **Preset Browser** (file list, egui)

---

### Phase 7: Core Class Migration

**Source**: `~/Dev/Lightningbeam-2/src/models/`

#### Models to Migrate

**From `~/Dev/Lightningbeam-2/src/models/`**:

1. **root.js** (34 lines)
   - Document root structure
   - → `lightningbeam-core/src/document.rs`

2. **layer.js**
   - Layer types (2D, video, audio, etc.)
   - Transform properties
   - Visibility, opacity
   - → `lightningbeam-core/src/layer.rs`

3. **shapes.js** (752 lines)
   - Path, Rectangle, Ellipse, Star, Polygon
   - Stroke/fill properties
   - Bezier curves
   - → `lightningbeam-core/src/shape.rs`

4. **animation.js**
   - Keyframe data structures
   - Interpolation curves
   - → `lightningbeam-core/src/animation.rs`

5. **graphics-object.js**
   - Base graphics properties
   - Transform matrices
   - → `lightningbeam-core/src/graphics.rs`

#### Actions to Migrate

**From `~/Dev/Lightningbeam-2/src/actions/`**:

1. **index.js** (2,615 lines)
   - Drawing tools
   - Transform operations
   - Layer operations
   - → `lightningbeam-core/src/actions/`

2. **selection-actions.js** (166 lines)
   - Selection state management
   - → `lightningbeam-core/src/selection.rs`

#### Migration Strategy

1. **Rust-first design**: Leverage Rust's type system, don't just transliterate JS
2. **Serde compatibility**: Ensure classes can serialize/deserialize from existing JSON
3. **Performance**: Use `Arc<RwLock<T>>` for shared mutable state where needed
4. **Memory safety**: Eliminate runtime errors through compile-time checks

---

### Phase 8: Tools & Interaction

**After core classes are migrated**:

- [ ] Pen tool (Bezier curves)
- [ ] Select tool (bounding box, direct selection)
- [ ] Transform tool (rotate, scale, skew)
- [ ] Shape tools (rectangle, ellipse, star, polygon)
- [ ] Text tool
- [ ] Eyedropper
- [ ] Zoom/pan

---

### Phase 9: Feature Parity

**Remaining features from JS version**:

- [ ] Onion skinning
- [ ] Frame export (PNG sequence)
- [ ] Project save/load
- [ ] Undo/redo system
- [ ] Preferences/settings
- [ ] Keyboard shortcuts
- [ ] Help/documentation
- [ ] Clipboard operations (copy/paste)

---

## Architecture Decisions

### Why egui over iced?
- **Vello compatibility**: iced has issues with Vello integration
- **Immediate mode**: Simpler state management for complex UI
- **Maturity**: More stable and well-documented
- **Performance**: Good enough for our needs

### Layer Rendering Strategy
```rust
// Conceptual pipeline
for layer in document.layers {
    let texture = layer.render_to_texture(vello_renderer);
    composite_textures.push(texture);
}
let final_image = compositor.blend(composite_textures);
stage.display(final_image);
```

### State Management
- **Document state**: `Arc<RwLock<Document>>` - shared across panes
- **Selection state**: Event-based updates
- **UI state**: Local to each pane (egui handles this)

### External System Integration

**Video Layers**:
- UI requests frame texture from video processing module
- Module returns GPU texture handle
- UI composites texture with other layers

**Audio Playback**:
- UI sends playback commands to `daw-backend`
- Backend handles audio processing/mixing
- UI displays waveforms (data from backend)

---

## File Structure (Target)

```
lightningbeam-ui/
├── lightningbeam-core/          # Pure Rust library
│   ├── src/
│   │   ├── lib.rs
│   │   ├── layout.rs            # ✅ Layout system
│   │   ├── pane.rs              # ⏳ Pane trait
│   │   ├── document.rs          # ❌ Document root
│   │   ├── layer.rs             # ❌ Layer types
│   │   ├── shape.rs             # ❌ Shape primitives
│   │   ├── animation.rs         # ❌ Keyframes
│   │   ├── graphics.rs          # ❌ Graphics base
│   │   ├── selection.rs         # ❌ Selection state
│   │   └── actions/             # ❌ Action system
│   └── Cargo.toml
│
├── lightningbeam-editor/        # egui application
│   ├── src/
│   │   ├── main.rs              # ✅ App + layout rendering
│   │   ├── menu.rs              # ❌ Native menu integration (muda)
│   │   ├── panes/               # ⏳ Pane implementations
│   │   │   ├── stage.rs         # ❌ Stage pane
│   │   │   ├── timeline.rs      # ❌ Timeline pane
│   │   │   ├── toolbar.rs       # ❌ Toolbar
│   │   │   ├── infopanel.rs     # ❌ Info panel
│   │   │   └── outliner.rs      # ❌ Outliner
│   │   ├── rendering/           # ❌ Vello integration
│   │   └── tools/               # ❌ Drawing tools
│   ├── assets/
│   │   └── layouts.json         # ✅ Layout definitions
│   └── Cargo.toml
│
└── Cargo.toml                   # Workspace
```

Legend: ✅ Complete | ⏳ In Progress | ❌ Not Started

---

## Dependencies Roadmap

### Current (Phase 1)
```toml
eframe = "0.29"           # UI framework
wgpu = "22"               # GPU
vello = "0.3"             # 2D rendering
kurbo = "0.11"            # 2D geometry
peniko = "0.5"            # 2D primitives
serde = "1.0"             # Serialization
serde_json = "1.0"        # JSON
```

### Phase 2-3 Additions
```toml
muda = "*"                # Native OS menus
```

### Phase 4+ Additions
```toml
image = "*"               # Image loading
egui-aesthetix = "*"      # Theming
clipboard = "*"           # Copy/paste
```

---

## Performance Goals

### Rendering
- **Stage**: 60 FPS @ 1080p, 100+ layers
- **Timeline**: Smooth scrolling with 1000+ frames
- **Memory**: < 500MB for typical project

### Benchmarks to Track
- Layer render time
- Composite time
- UI frame time
- Memory usage per layer

---

## Testing Strategy

1. **Unit tests**: Core classes (Layer, Shape, Animation)
2. **Integration tests**: Pane rendering
3. **Visual tests**: Screenshot comparison for rendering
4. **Performance tests**: Benchmark critical paths
5. **Manual testing**: UI interaction, edge cases

---

## Migration Checklist

### Before Deprecating JS Version
- [ ] All panes functional
- [ ] Core classes migrated
- [ ] Project save/load working
- [ ] Performance meets/exceeds JS version
- [ ] No critical bugs
- [ ] User testing complete
- [ ] Native menus integrated
- [ ] External system integration verified (video, audio)

---

## Assets Management

**Current Approach**:
- SVG icons referenced from `~/Dev/Lightningbeam-2/src/assets/`
- Allows pulling in new icons/changes from upstream during rewrite
- Using `egui_extras::RetainedImage` for cached SVG rendering

**TODO (Before Release)**:
- [ ] Move assets to `lightningbeam-editor/assets/icons/`
- [ ] Update asset paths in code
- [ ] Set up asset bundling/embedding

---

## Open Questions

1. **Plugin system**: Support for extensions?
2. **Scripting**: Embed Lua/JavaScript for automation?
3. **Collaborative editing**: Future consideration?

---

## References

- **JS Source**: `~/Dev/Lightningbeam-2/src/`
- **egui docs**: https://docs.rs/egui
- **Vello docs**: https://docs.rs/vello
- **wgpu docs**: https://docs.rs/wgpu
- **muda docs**: https://docs.rs/muda
