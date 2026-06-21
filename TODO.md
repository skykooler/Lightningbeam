# Lightningbeam TODO

## Animation System Refactoring

### Completed
- ✅ Implement AnimationData curve-based system (Keyframe, AnimationCurve, AnimationData classes)
- ✅ Add GraphicsObject.currentTime property
- ✅ Migrate shape rendering to use AnimationData curves (exists, zOrder)
- ✅ Binary search optimization for keyframe lookups

### In Progress
- Migrating from Frame-based to AnimationData curve-based system throughout codebase

### Pending Features

#### Animation Curve Enhancements
- [ ] Implement extrapolation modes (separate for start vs end):
  - "hold" (default) - hold value at first/last keyframe
  - "extend" - linearly extend the curve beyond keyframes
  - "repeat" - repeat the animation
  - "decay" - exponential decay to a target value
- [ ] Add position, scale, rotation animation curves for shapes
- [ ] Add shape morphing/tweening between keyframes

#### Keyframing Behavior
- [ ] Add user preference for keyframing behavior when editing objects:
  - Auto-keyframe (current default): create/update keyframe at current time
  - Edit previous (Flash-style): update most recent keyframe before current time
  - Ephemeral (Blender-style): changes don't persist without manual keyframe
  - Optional: Add modifier key (e.g. Shift) to toggle between modes

#### Shape Ordering
- [ ] Add "Bring Forward" menu option (swap zOrder with shape in front)
- [ ] Add "Send Backward" menu option (swap zOrder with shape behind)
- [ ] Add "Bring to Front" menu option (set zOrder to max + 1)
- [ ] Add "Send to Back" menu option (set zOrder to min - 1)

#### Code Cleanup
- [ ] Remove all remaining references to Frame-based system
- [ ] Remove legacy Frame class once migration is complete
- [ ] Clean up GraphicsObject.shapes[] array (shapes should only live in Layers)

## Known Issues / Platform Limitations

### Animation: Tweens are broken (Rust codebase) — LOW PRIORITY
- **Issue**: Animation tweening between keyframes (shape/vector interpolation, and the
  `tween_after` behavior on keyframes) does not work correctly in the current Rust app.
  Needs investigation + fix. Not urgent — revisit later.
- (Older JS-codebase animation entries below reference `src/*.js` and are stale.)

### Audio: Oscillator Timbre Drift (Phase Accumulation Error)
- **Issue**: Oscillators exhibit timbre changes over time due to floating-point phase accumulation errors
- **Affected Files**:
  - `daw-backend/src/effects/synth.rs:117-120` (SimpleSynth)
  - `daw-backend/src/audio/node_graph/nodes/oscillator.rs:167-170` (OscillatorNode)
- **Root Cause**: Current phase wrapping uses conditional subtraction (`if phase >= 1.0 { phase -= 1.0 }`), which accumulates f32 rounding errors over time, especially for long-playing notes
- **Current Code**:
  ```rust
  self.phase += frequency / sample_rate;
  if self.phase >= 1.0 {
      self.phase -= 1.0;
  }
  ```
- **Recommended Fix**: Replace with `.fract()` for numerically stable wraparound:
  ```rust
  self.phase += frequency / sample_rate;
  self.phase = self.phase.fract();
  ```
- **Impact**: Medium - affects audio quality for sustained notes, becomes noticeable after several seconds
- **Priority**: Medium - should be addressed before production use

### UI: Node Connections Render Behind VoiceAllocator Child Nodes
- **Issue**: Connection lines (SVG paths) inside expanded VoiceAllocator nodes render behind child nodes due to z-index stacking
- **Affected File**: `src/styles.css:1128`
- **Root Cause**: Child nodes have `z-index: 10` while connection SVG paths have default/lower z-index
- **Current Code**:
  ```css
  .drawflow .drawflow-node.child-node {
    opacity: 0.9;
    border: 1px solid #5a5aaa !important;
    box-shadow: 0 2px 8px rgba(90, 90, 170, 0.3);
    z-index: 10;
  }
  ```
- **Recommended Fix**: Either:
  1. Remove `z-index: 10` from `.child-node` (simplest), or
  2. Add higher z-index to connection SVG paths, or
  3. Use CSS `isolation: isolate` on the VoiceAllocator contents area to create a new stacking context
- **Impact**: Low - visual issue only, connections still function but appear to go "behind" nodes
- **Priority**: Low - cosmetic issue that doesn't affect functionality

### UI: VoiceAllocator Child Nodes Don't Move with Parent
- **Issue**: When a VoiceAllocator node is moved, its child nodes remain in their original positions instead of moving with the parent
- **Affected File**: `src/main.js:6202-6207`
- **Root Cause**: The `nodeMoved` event handler only handles the case where a child node is moved (resizes parent), but doesn't handle when the VoiceAllocator itself is moved
- **Current Code**:
  ```javascript
  editor.on("nodeMoved", (nodeId) => {
    const node = editor.getNodeFromId(nodeId);
    if (node && node.data.parentNodeId) {
      resizeVoiceAllocatorToFit(node.data.parentNodeId);
    }
  });
  ```
- **Recommended Fix**: Add logic to detect when a VoiceAllocator is moved and update all child node positions:
  ```javascript
  editor.on("nodeMoved", (nodeId) => {
    const node = editor.getNodeFromId(nodeId);

    // Case 1: A child node was moved - resize parent
    if (node && node.data.parentNodeId) {
      resizeVoiceAllocatorToFit(node.data.parentNodeId);
    }

    // Case 2: A VoiceAllocator was moved - move all children
    if (node && node.data.nodeType === 'VoiceAllocator') {
      // Calculate delta from previous position (need to track)
      // Update all child node positions by the delta
      // Call editor.updateConnectionNodes() for parent and all children
    }
  });
  ```
- **Impact**: High - child nodes become disconnected from parent visually
- **Priority**: High - breaks expected behavior of grouped nodes

### UI: VoiceAllocator Expansion Doesn't Update Connection Positions
- **Issue**: When expanding/collapsing a VoiceAllocator, connection endpoints don't update to match the new port positions
- **Affected File**: `src/main.js:6496-6555` (handleNodeDoubleClick function)
- **Root Cause**: The expand/collapse logic shows/hides child nodes and resizes the container, but never calls `editor.updateConnectionNodes()` to refresh connection positions
- **Current Code**: In `handleNodeDoubleClick()`, after expanding or collapsing:
  ```javascript
  // Expand
  expandedNodes.add(nodeId);
  nodeElement.classList.add('expanded');
  nodeElement.style.width = '600px';
  nodeElement.style.height = '400px';
  // ... shows child nodes ...
  // Missing: editor.updateConnectionNodes(`node-${nodeId}`)
  ```
- **Recommended Fix**: Call `editor.updateConnectionNodes()` after resizing:
  ```javascript
  // After expanding
  expandedNodes.add(nodeId);
  nodeElement.classList.add('expanded');
  // ... resize and show children ...

  // Update connection positions for VoiceAllocator and all children
  editor.updateConnectionNodes(`node-${nodeId}`);
  for (const [childId, parentId] of nodeParents.entries()) {
    if (parentId === nodeId) {
      editor.updateConnectionNodes(`node-${childId}`);
    }
  }
  ```
- **Impact**: Medium - connections appear in wrong positions until manually moved
- **Priority**: Medium - visual issue that affects usability

### UI: Node Editor Allows Editing Without MIDI Layer Selected
- **Issue**: The node editor pane allows adding/editing instrument nodes even when no MIDI layer is selected, and always uses hardcoded `trackId: 0`
- **Affected File**: `src/main.js:6045-6920` (nodeEditor function)
- **Root Cause**: The node editor never checks if `context.activeObject.activeLayer` exists or is a MIDI track, and all backend commands use hardcoded `trackId: 0`
- **Current Code**: All graph commands hardcode track 0:
  ```javascript
  const commandArgs = parentNodeId
    ? {
        trackId: 0,  // HARDCODED!
        voiceAllocatorId: editor.getNodeFromId(parentNodeId).data.backendId,
        nodeType: nodeType,
        x: x,
        y: y
      }
    : {
        trackId: 0,  // HARDCODED!
        nodeType: nodeType,
        x: x,
        y: y
      };
  ```
- **Recommended Fix**:
  1. Check if activeLayer is a MIDI track before allowing edits:
     ```javascript
     function getSelectedMidiTrack() {
       const activeLayer = context.activeObject?.activeLayer;
       if (!activeLayer || activeLayer.type !== 'midi') {
         return null;
       }
       return activeLayer;
     }
     ```
  2. Show placeholder when no MIDI track selected:
     ```javascript
     function nodeEditor() {
       const container = document.createElement("div");
       const midiTrack = getSelectedMidiTrack();

       if (!midiTrack) {
         container.innerHTML = '<div class="placeholder">Select a MIDI layer to edit instruments</div>';
         return container;
       }
       // ... rest of node editor code ...
     }
     ```
  3. Use actual track ID instead of hardcoded 0:
     ```javascript
     const trackId = midiTrack.audioTrackId || 0;
     const commandArgs = { trackId, nodeType, x, y };
     ```
  4. Add listener to refresh node editor when layer selection changes
- **Impact**: High - allows editing wrong track's instrument graph, data corruption risk
- **Priority**: High - can cause confusion and data loss

### Animation: Wrong Default Interpolation for Shape and Object Keyframes
- **Issue**: Shape index and object transform keyframes default to "linear" interpolation but should default to "hold" (step function), and there's no UI to change interpolation after creation
- **Affected Files**:
  - `src/models/animation.js:124` (Keyframe constructor defaults to "linear")
  - `src/main.js:2161` (shapeIndex keyframes default to "linear")
  - `src/main.js:2198` (object position/rotation/scale keyframes default to "linear")
  - `src/main.js:5910` (Timeline menu - missing tween options)
- **Root Cause**:
  1. The Keyframe constructor defaults interpolation to "linear"
  2. Shape index keyframes preserve existing interpolation or default to "linear"
  3. Object transform keyframes explicitly use "linear"
  4. No menu options exist to change interpolation mode after keyframe creation
- **Current Code**:
  - Keyframe constructor (animation.js:124):
    ```javascript
    constructor(time, value, interpolation = "linear", uuid = undefined) {
    ```
  - Shape index keyframes (main.js:2161):
    ```javascript
    const interpolationType = existingShapeIndexKf ? existingShapeIndexKf.interpolation : 'linear';
    const shapeIndexKeyframe = new Keyframe(currentTime, newShapeIndex, interpolationType);
    ```
  - Object keyframes (main.js:2198):
    ```javascript
    const newKeyframe = new Keyframe(
      currentTime,
      currentValue,
      'linear' // Default to linear interpolation
    );
    ```
- **Expected Behavior**:
  - Shape index keyframes should default to "hold" (shapes shouldn't morph between versions)
  - Object transforms should default to "hold" (objects shouldn't move/rotate/scale between keyframes unless explicitly tweened)
  - Timeline menu should have options to convert between interpolation modes
- **Recommended Fix**:
  1. Change shapeIndex default to "hold" (main.js:2161):
     ```javascript
     const interpolationType = existingShapeIndexKf ? existingShapeIndexKf.interpolation : 'hold';
     ```
  2. Change object keyframe default to "hold" (main.js:2198):
     ```javascript
     const newKeyframe = new Keyframe(currentTime, currentValue, 'hold');
     ```
  3. Add Timeline menu options (main.js:5910, in timelineSubmenu):
     ```javascript
     {
       text: "Add Shape Tween",
       enabled: /* check if shape is selected and has keyframes */,
       action: () => {
         // Find shapeIndex curve for selected shape
         // Change interpolation between keyframes to "linear"
       }
     },
     {
       text: "Add Motion Tween",
       enabled: /* check if object is selected and has transform keyframes */,
       action: () => {
         // Find position/rotation/scale curves for selected object
         // Change interpolation between keyframes to "linear" or "bezier"
       }
     }
     ```
- **Note**: exists and zOrder keyframes already correctly use "hold" (main.js:2139, 2150)
- **Impact**: High - causes unwanted interpolation, shapes morph unexpectedly, objects move when they shouldn't
- **Priority**: High - fundamental animation behavior is incorrect

### Tauri Pinch-Zoom on Linux
- **Issue**: Two-finger pinch gestures zoom the entire Tauri window instead of individual canvases
- **Status**: Known Tauri limitation on Linux/GTK with no cross-platform solution
- **Tracking**: https://github.com/tauri-apps/tauri/discussions/3843
- **Workaround attempts**: Tried `zoomHotkeysEnabled: false`, `touch-action: none`, viewport meta tags - none worked
- **Resolution**: Monitor Tauri releases for official fix

## Notes

### Architecture
- **GraphicsObject** contains Layers and has `currentTime` (continuous time)
- **Layer** contains `shapes[]` array and `animationData` (AnimationData instance)
- **AnimationData** contains curves dictionary, each curve identified by parameter name
  - Shape curves: `shape.{uuid}.exists`, `shape.{uuid}.zOrder`
  - Future: `shape.{uuid}.x`, `shape.{uuid}.y`, `shape.{uuid}.rotation`, etc.
- **Shapes render based on curves**: Layer.draw checks exists > 0, sorts by zOrder, draws in order

### Interpolation Types
- `linear` - Linear interpolation between keyframes
- `bezier` - Cubic Bezier with easing control points
- `step`/`hold` - Step function (jumps to next value)
