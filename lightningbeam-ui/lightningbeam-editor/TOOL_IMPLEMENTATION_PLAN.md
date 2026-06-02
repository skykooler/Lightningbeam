# Tool Integration Implementation Plan
*Updated with correct architecture patterns from JS codebase*

## Architecture Overview

**Type-Safe Action System**: Document mutations only through `Action` trait
- Read: Public via `ActionExecutor::document()`
- Write: Only via `pub(crate)` methods in action implementations
- Enforcement: Rust's module privacy system

**Key Corrections**:
- ✅ GraphicsObject nesting (recursive hit testing)
- ✅ Shape tools create `Shape` + `Object`, add to active `VectorLayer`
- ✅ Tools only work on `VectorLayer` (check `active_layer.type`)
- ✅ Path fitting uses JS algorithms (RDP or Schneider)
- ✅ Paint bucket uses vector flood fill with quadtree

---

## Phase 1: Action System Foundation

### 1.1 Create Action System Core
**File: `lightningbeam-core/src/action.rs`**

```rust
pub trait Action: Send {
    fn execute(&mut self, document: &mut Document);
    fn rollback(&mut self, document: &mut Document);
    fn description(&self) -> String;
}

pub struct ActionExecutor {
    document: Document,
    undo_stack: Vec<Box<dyn Action>>,
    redo_stack: Vec<Box<dyn Action>>,
}
```

Methods:
- `document(&self) -> &Document` - Read-only access
- `execute(&mut self, Box<dyn Action>)` - Execute + push to undo
- `undo(&mut self) -> bool` - Pop and rollback
- `redo(&mut self) -> bool` - Re-execute from redo stack

### 1.2 Update Document for Controlled Access
**File: `lightningbeam-core/src/document.rs`**

Add `pub(crate)` mutation methods:
- `root_mut() -> &mut GraphicsObject`
- `get_layer_mut(&self, id: &Uuid) -> Option<&mut AnyLayer>`
- Keep all fields private
- Keep existing public read methods

### 1.3 Update Layer for Shape Operations
**File: `lightningbeam-core/src/layer.rs`**

Add `pub(crate)` methods to `VectorLayer`:
- `add_shape_internal(&mut self, shape: Shape) -> Uuid`
- `add_object_internal(&mut self, object: Object) -> Uuid`
- `remove_shape_internal(&mut self, id: &Uuid) -> Option<Shape>`
- `remove_object_internal(&mut self, id: &Uuid) -> Option<Object>`
- `modify_object_internal(&mut self, id: &Uuid, f: impl FnOnce(&mut Object))`

### 1.4 Integrate ActionExecutor into EditorApp
**File: `lightningbeam-editor/src/main.rs`**

- Replace `document: Document` with `action_executor: ActionExecutor`
- Add `active_layer_id: Option<Uuid>` to track current layer
- Update `SharedPaneState` to pass `document: &Document` (read-only)
- Add `execute_action(&mut self, action: Box<dyn Action>)` method
- Wire Ctrl+Z / Ctrl+Shift+Z to undo/redo

---

## Phase 2: Selection System

### 2.1 Create Selection State
**File: `lightningbeam-core/src/selection.rs`**

```rust
pub struct Selection {
    selected_objects: Vec<Uuid>,
    selected_shapes: Vec<Uuid>,
}
```

Methods: `add`, `remove`, `clear`, `contains`, `is_empty`, `objects()`, `shapes()`

### 2.2 Add to Editor State
Add to `EditorApp`:
- `selection: Selection`
- Pass through `SharedPaneState` (read-only for rendering, mutable for tools)

---

## Phase 3: Hit Testing Infrastructure

### 3.1 Hit Test Module
**File: `lightningbeam-core/src/hit_test.rs`**

**Recursive Hit Testing through GraphicsObject hierarchy:**

```rust
pub fn hit_test_layer(
    layer: &VectorLayer,
    point: Point,
    tolerance: f64,
    parent_transform: Affine,
) -> Option<Uuid> {
    // Hit test objects in this layer
    for object in layer.objects.iter().rev() { // Back to front
        let shape = layer.get_shape(&object.shape_id)?;

        // Combine parent transform with object transform
        let combined_transform = parent_transform * object.to_affine();

        if hit_test_shape(shape, point, tolerance, combined_transform) {
            return Some(object.id);
        }
    }
    None
}

fn hit_test_shape(
    shape: &Shape,
    point: Point,
    tolerance: f64,
    transform: Affine,
) -> bool {
    // Transform point to shape's local space
    let inverse_transform = transform.inverse();
    let local_point = inverse_transform * point;

    // Check if point is inside path (kurbo's contains())
    if shape.path.contains(local_point) {
        return true;
    }

    // Check stroke bounds if has stroke
    if shape.stroke_style.is_some() {
        let stroke_tolerance = shape.stroke_style.unwrap().width / 2.0 + tolerance;
        // Check distance to path
        // Use kurbo path methods for nearest point
    }

    false
}
```

**Rectangle Hit Testing:**
```rust
pub fn hit_test_objects_in_rect(
    layer: &VectorLayer,
    rect: Rect,
    parent_transform: Affine,
) -> Vec<Uuid> {
    let mut hits = Vec::new();

    for object in &layer.objects {
        let shape = layer.get_shape(&object.shape_id).unwrap();
        let combined_transform = parent_transform * object.to_affine();
        let bbox = shape.path.bounding_box();
        let transformed_bbox = combined_transform.transform_rect_bbox(bbox);

        if rect.intersect(transformed_bbox).area() > 0.0 {
            hits.push(object.id);
        }
    }

    hits
}
```

### 3.2 Bounding Box Calculation
Add to `lightningbeam-core/src/object.rs`:

```rust
impl Object {
    pub fn bounding_box(&self, shape: &Shape) -> Rect {
        let path_bbox = shape.path.bounding_box();
        self.to_affine().transform_rect_bbox(path_bbox)
    }
}
```

---

## Phase 4: Tool State Management

### 4.1 Tool State Enum
**File: `lightningbeam-core/src/tool.rs`**

```rust
pub enum ToolState {
    Idle,

    DrawingPath {
        points: Vec<Point>,
        simplify_mode: SimplifyMode, // "corners" | "smooth" | "verbatim"
    },

    DraggingSelection {
        start_pos: Point,
        start_mouse: Point,
        original_transforms: HashMap<Uuid, Transform>,
    },

    MarqueeSelecting {
        start: Point,
        current: Point,
    },

    CreatingRectangle {
        start_corner: Point,
        current_corner: Point,
    },

    CreatingEllipse {
        center: Point,
        current_point: Point,
    },

    Transforming {
        mode: TransformMode,
        original_transforms: HashMap<Uuid, Transform>,
        pivot: Point,
    },
}

pub enum SimplifyMode {
    Corners,  // Ramer-Douglas-Peucker
    Smooth,   // Schneider curve fitting
    Verbatim, // No simplification
}
```

Add to `EditorApp`: `tool_state: ToolState`

---

## Phase 5: Select Tool

### 5.1 Active Layer Validation
**All tools check:**
```rust
// In Stage.handle_tool_input()
let Some(active_layer_id) = shared.active_layer_id else {
    return None; // No active layer
};

let active_layer = shared.document.get_layer(active_layer_id)?;

// Only work on VectorLayer
let AnyLayer::Vector(vector_layer) = active_layer else {
    return None; // Not a vector layer
};
```

### 5.2 Click Selection
**Mouse Down:**
- Hit test at click position using recursive `hit_test_layer()`
- If object found:
  - If Shift: toggle in selection
  - Else: replace selection with clicked object
  - If already selected: enter `DraggingSelection` state
- If nothing found: enter `MarqueeSelecting` state

**Mouse Drag (when dragging selection):**
- Calculate delta from start_mouse
- Update object positions (temporary, for preview)
- Re-render with updated positions

**Mouse Up:**
- If was dragging: create `MoveObjectsAction`
- If was marquee: select objects in rectangle

### 5.3 Move Objects Action
**File: `lightningbeam-core/src/actions/move_objects.rs`**

```rust
pub struct MoveObjectsAction {
    layer_id: Uuid,
    object_transforms: HashMap<Uuid, (Transform, Transform)>, // (old, new)
}

impl Action for MoveObjectsAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        if let AnyLayer::Vector(vector_layer) = layer {
            for (object_id, (_old, new)) in &self.object_transforms {
                vector_layer.modify_object_internal(object_id, |obj| {
                    obj.transform = new.clone();
                });
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        if let AnyLayer::Vector(vector_layer) = layer {
            for (object_id, (old, _new)) in &self.object_transforms {
                vector_layer.modify_object_internal(object_id, |obj| {
                    obj.transform = old.clone();
                });
            }
        }
    }
}
```

### 5.4 Selection Rendering
In `VelloCallback::prepare()`:
- After rendering document
- For each selected object ID:
  - Get object and its shape from active layer
  - Calculate bounding box (with transform)
  - Draw selection outline (blue, 2px stroke)

---

## Phase 6: Rectangle & Ellipse Tools

### 6.1 Add Shape Action
**File: `lightningbeam-core/src/actions/add_shape.rs`**

```rust
pub struct AddShapeAction {
    layer_id: Uuid,
    shape: Shape,
    object: Object,
    created_shape_id: Option<Uuid>,
    created_object_id: Option<Uuid>,
}

impl Action for AddShapeAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        if let AnyLayer::Vector(vector_layer) = layer {
            let shape_id = vector_layer.add_shape_internal(self.shape.clone());
            let object_id = vector_layer.add_object_internal(self.object.clone());
            self.created_shape_id = Some(shape_id);
            self.created_object_id = Some(object_id);
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if let (Some(shape_id), Some(object_id)) = (self.created_shape_id, self.created_object_id) {
            let layer = document.get_layer_mut(&self.layer_id).unwrap();
            if let AnyLayer::Vector(vector_layer) = layer {
                vector_layer.remove_object_internal(&object_id);
                vector_layer.remove_shape_internal(&shape_id);
            }
        }
    }
}
```

### 6.2 Rectangle Tool
**Mouse Down:** Enter `CreatingRectangle { start_corner, current_corner }`

**Mouse Drag:**
- Update current_corner
- If Shift: constrain to square (equal width/height)
- Create preview path: `Rect::from_points(start, current).to_path()`
- Render preview with dashed stroke

**Mouse Up:**
- Create `Shape` with rectangle path
- Create `Object` at (0, 0) with shape_id
- Return `AddShapeAction { layer_id, shape, object }`

### 6.3 Ellipse Tool
**Mouse Down:** Enter `CreatingEllipse { center, current_point }`

**Mouse Drag:**
- Calculate radii from center to current_point
- If Shift: constrain to circle (equal radii)
- Create preview: `Circle::new(center, radius).to_path()`
- Render preview

**Mouse Up:**
- Create `Shape` with ellipse path
- Create `Object` with shape_id
- Return `AddShapeAction`

---

## Phase 7: Draw/Pen Tool

### 7.1 Path Fitting Module
**File: `lightningbeam-core/src/path_fitting.rs`**

**Implement two algorithms from JS:**

#### A. Ramer-Douglas-Peucker Simplification
```rust
pub fn simplify_rdp(points: &[Point], tolerance: f64) -> Vec<Point> {
    // Port from /src/simplify.js
    // 1. Radial distance filter first
    // 2. Then Douglas-Peucker recursive simplification
    // Tolerance: 10 (squared internally)
}
```

#### B. Schneider Curve Fitting
```rust
pub fn fit_bezier_curves(points: &[Point], max_error: f64) -> BezPath {
    // Port from /src/fit-curve.js
    // Based on Graphics Gems algorithm
    // 1. Chord-length parameterization
    // 2. Least-squares fit for control points
    // 3. Newton-Raphson refinement (max 20 iterations)
    // 4. Recursive split at max error point if needed
    // max_error: 30
}
```

### 7.2 Draw Tool Implementation
**Mouse Down:** Enter `DrawingPath { points: vec![start], simplify_mode }`

**Mouse Drag:**
- Add point if distance from last point > threshold (2-5 pixels)
- Build preview path from points
- Render preview

**Mouse Up:**
- Based on `simplify_mode`:
  - **Corners**: Apply RDP simplification (tolerance=10), then create mid-point Beziers
  - **Smooth**: Apply Schneider curve fitting (error=30)
  - **Verbatim**: Use points as-is
- Create `Shape` with fitted path
- Create `Object` with shape_id
- Return `AddShapeAction`

**Simplify Mode Setting:**
Add to `EditorApp`: `pen_simplify_mode: SimplifyMode`
Show in info panel / toolbar

---

## Phase 8: Transform Tool

### 8.1 Transform Handles
In `VelloCallback::prepare()` when `Tool::Transform` and selection non-empty:

Calculate selection bbox (union of all selected object bboxes):
```rust
let mut bbox = Rect::ZERO;
for object_id in selection.objects() {
    let object = get_object(object_id);
    let shape = get_shape(object.shape_id);
    bbox = bbox.union(object.bounding_box(shape));
}
```

Render 8 handles + rotation handle:
- 4 corners (8x8 squares) → scale from opposite corner
- 4 edge midpoints → scale along axis
- 1 rotation handle (circle, 20px above top edge)
- Bounding box outline

### 8.2 Handle Hit Testing
```rust
fn hit_test_transform_handle(
    point: Point,
    bbox: Rect,
    tolerance: f64,
) -> Option<TransformMode> {
    // Check rotation handle first
    let rotation_handle = Point::new(bbox.center().x, bbox.min_y() - 20.0);
    if point.distance(rotation_handle) < tolerance {
        return Some(TransformMode::Rotate { center: bbox.center() });
    }

    // Check corner handles
    let corners = [bbox.origin(), /* ... */];
    for (i, corner) in corners.iter().enumerate() {
        if point.distance(*corner) < tolerance {
            let opposite = corners[(i + 2) % 4];
            return Some(TransformMode::ScaleCorner { origin: opposite });
        }
    }

    // Check edge handles
    // ...
}
```

### 8.3 Transform Interaction
**Mouse Down on handle:**
- Enter `Transforming { mode, original_transforms, pivot }`

**Mouse Drag:**
- Calculate new transform based on mode:
  - **ScaleCorner**: Compute scale from opposite corner
  - **ScaleEdge**: Scale along one axis
  - **Rotate**: Compute angle from pivot to cursor
- Apply to all selected objects (preview)

**Mouse Up:**
- Create `TransformObjectsAction`
- Return for execution

### 8.4 Transform Action
**File: `lightningbeam-core/src/actions/transform.rs`**

```rust
pub struct TransformObjectsAction {
    layer_id: Uuid,
    object_transforms: HashMap<Uuid, (Transform, Transform)>, // (old, new)
}
```

Similar to MoveObjectsAction but updates full Transform struct.

---

## Phase 9: Paint Bucket Tool

### 9.1 Quadtree for Curve Indexing
**File: `lightningbeam-core/src/quadtree.rs`**

Port from JS (`/src/utils.js`):
```rust
pub struct Quadtree {
    bounds: Rect,
    capacity: usize,
    curves: Vec<(BezPath, usize)>, // (curve, index)
    subdivided: bool,
    // children: [Box<Quadtree>; 4]
}

impl Quadtree {
    pub fn insert(&mut self, curve: BezPath, index: usize);
    pub fn query(&self, range: Rect) -> Vec<usize>; // Return curve indices
}
```

### 9.2 Vector Flood Fill
**File: `lightningbeam-core/src/flood_fill.rs`**

Port from JS (`/src/utils.js` lines 173-307):

```rust
pub struct FloodFillRegion {
    start_point: Point,
    epsilon: f64,           // Gap closing tolerance (default: 5)
    canvas_bounds: Rect,
}

impl FloodFillRegion {
    pub fn fill(
        &self,
        shapes: &[Shape],      // All visible shapes on layer
    ) -> Result<Vec<Point>, String> {
        // 1. Build quadtree for all curves in all shapes
        // 2. Stack-based flood fill
        // 3. For each point:
        //    - Check if near any curve (using quadtree query + projection)
        //    - If near curve (within epsilon): save projection point, stop expanding
        //    - If not near: expand to 4 neighbors
        // 4. Return boundary points (projections on curves)
        // 5. If < 10 points found, retry with epsilon=1
    }

    fn is_near_curve(
        &self,
        point: Point,
        shape: &Shape,
        quadtree: &Quadtree,
    ) -> Option<Point> {
        let query_bbox = Rect::new(
            point.x - self.epsilon/2.0,
            point.y - self.epsilon/2.0,
            point.x + self.epsilon/2.0,
            point.y + self.epsilon/2.0,
        );

        for curve_idx in quadtree.query(query_bbox) {
            let curve = &shape.curves[curve_idx];
            let projection = curve.nearest(point, 0.1); // kurbo's nearest point
            if projection.distance_sq < self.epsilon * self.epsilon {
                return Some(projection.point);
            }
        }
        None
    }
}
```

### 9.3 Point Sorting
```rust
fn sort_points_by_proximity(points: Vec<Point>) -> Vec<Point> {
    // Port from JS lines 276-307
    // Greedy nearest-neighbor sort to create coherent path
}
```

### 9.4 Paint Bucket Action
**File: `lightningbeam-core/src/actions/paint_bucket.rs`**

```rust
pub struct PaintBucketAction {
    layer_id: Uuid,
    click_point: Point,
    epsilon: f64,
    created_shape_id: Option<Uuid>,
    created_object_id: Option<Uuid>,
}

impl Action for PaintBucketAction {
    fn execute(&mut self, document: &mut Document) {
        let layer = document.get_layer(&self.layer_id).unwrap();
        let AnyLayer::Vector(vector_layer) = layer else { return };

        // Get all shapes in layer
        let shapes: Vec<_> = vector_layer.shapes.clone();

        // Perform flood fill
        let fill_region = FloodFillRegion {
            start_point: self.click_point,
            epsilon: self.epsilon,
            canvas_bounds: Rect::new(0.0, 0.0, document.width, document.height),
        };

        let boundary_points = fill_region.fill(&shapes)?;

        // Sort points by proximity
        let sorted_points = sort_points_by_proximity(boundary_points);

        // Fit curve with very low error (1.0) for precision
        let path = fit_bezier_curves(&sorted_points, 1.0);

        // Create filled shape
        let shape = Shape::new(path)
            .with_fill(/* current fill color */)
            .without_stroke();

        // Create object
        let object = Object::new(shape.id);

        // Add to layer
        let layer = document.get_layer_mut(&self.layer_id).unwrap();
        if let AnyLayer::Vector(vector_layer) = layer {
            self.created_shape_id = Some(vector_layer.add_shape_internal(shape));
            self.created_object_id = Some(vector_layer.add_object_internal(object));
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        // Remove created shape and object
    }
}
```

### 9.5 Paint Bucket Tool Handler
In `handle_tool_input()` when `Tool::PaintBucket`:

**Mouse Click:**
- Get click position
- Create `PaintBucketAction { click_point, epsilon: 5.0 }`
- Return action for execution
- Tool stays active for multiple fills

---

## Phase 10: Eyedropper Tool

### 10.1 Color Sampling
In `handle_tool_input()` when `Tool::Eyedropper`:

**Mouse Click:**
- Hit test at cursor position
- If object found:
  - Get object's shape
  - Read shape's fill_color
  - Update `fill_color` in EditorApp
  - Show toast/feedback with sampled color
- Tool stays active

**Visual Feedback:**
- Custom cursor showing crosshair
- Color preview circle at cursor
- Display hex value

---

## Implementation Order

### Sprint 1: Foundation (3-4 days)
- [ ] Action system (ActionExecutor, Action trait)
- [ ] Document controlled access (pub(crate) methods)
- [ ] Integrate ActionExecutor into EditorApp
- [ ] Undo/redo shortcuts (Ctrl+Z, Ctrl+Shift+Z)

### Sprint 2: Selection (3-4 days)
- [ ] Selection state struct
- [ ] Recursive hit testing (through GraphicsObject hierarchy)
- [ ] Active layer tracking
- [ ] Selection rendering
- [ ] Click selection

### Sprint 3: Select Tool (4-5 days)
- [ ] Tool state management
- [ ] Stage input handling refactor
- [ ] Layer type validation
- [ ] Drag-to-move (MoveObjectsAction)
- [ ] Marquee selection

### Sprint 4: Shape Tools (4-5 days)
- [ ] AddShapeAction
- [ ] Rectangle tool (with Shift constraint)
- [ ] Ellipse tool (with Shift constraint)
- [ ] Preview rendering
- [ ] Integration with active layer

### Sprint 5: Draw Tool (5-6 days)
- [ ] RDP simplification algorithm
- [ ] Schneider curve fitting algorithm
- [ ] Path fitting module
- [ ] Draw tool with mode selection
- [ ] Preview rendering

### Sprint 6: Transform Tool (5-6 days)
- [ ] Transform handle rendering
- [ ] Handle hit testing
- [ ] Scale operations
- [ ] Rotate operation
- [ ] TransformObjectsAction

### Sprint 7: Paint Bucket (6-7 days)
- [ ] Quadtree implementation
- [ ] Vector flood fill algorithm
- [ ] Point sorting
- [ ] Curve fitting integration
- [ ] PaintBucketAction

### Sprint 8: Polish (2-3 days)
- [ ] Eyedropper tool
- [ ] Tool cursors
- [ ] Edge cases and bugs

**Total: ~6-7 weeks**

---

## Key Architectural Corrections

✅ **GraphicsObject Nesting**: Hit testing uses recursive transform multiplication through parent hierarchy

✅ **Shape Creation**: Tools create `Shape` instances, then `Object` instances pointing to them, add both to `VectorLayer`

✅ **Layer Type Validation**: Check `active_layer` is `VectorLayer` before tool operations

✅ **Path Fitting**: Port exact JS algorithms (RDP tolerance=10, Schneider error=30)

✅ **Paint Bucket**: Vector-based flood fill with quadtree-accelerated curve projection

✅ **Type Safety**: Compile-time enforcement that document mutations only through actions
