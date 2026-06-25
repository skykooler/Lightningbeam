//! Unified raster tool interface.
//!
//! Every raster tool operates on three GPU textures of identical dimensions:
//!
//! | Buffer | Access | Purpose |
//! |--------|--------|---------|
//! | **A** | Read-only  | Source pixels, uploaded from layer/float at mousedown. |
//! | **B** | Write-only | Output / display. Compositor shows B while the tool is active. |
//! | **C** | Read+Write | Scratch. Dabs accumulate here across the stroke; composite A+C→B each frame. |
//!
//! All three are `Rgba16Float` with the same pixel dimensions.  The framework
//! allocates and validates them in [`begin_raster_workspace`]; tools only
//! dispatch shaders.

use uuid::Uuid;
use eframe::egui;

// ── WorkspaceSource ──────────────────────────────────────────────────────────

/// Describes whether the tool is operating on a raster layer or a floating selection.
#[derive(Clone, Debug)]
pub enum WorkspaceSource {
    /// Operating on the full raster layer.
    Layer {
        layer_id: Uuid,
        time:     f64,
    },
    /// Operating on the floating selection.
    Float,
}

// ── RasterWorkspace ───────────────────────────────────────────────────────────

/// GPU buffer IDs and metadata for a single tool operation.
///
/// Created by [`begin_raster_workspace`] on mousedown.  All three canvas UUIDs
/// index into `GpuBrushEngine::canvases` and are valid for the lifetime of the
/// active tool.  They are queued for removal in `pending_canvas_removals` after
/// commit or cancel.
#[derive(Debug)]
pub struct RasterWorkspace {
    /// A canvas (Rgba16Float) — source pixels, uploaded at mousedown, read-only for tools.
    pub a_canvas_id: Uuid,
    /// B canvas (Rgba16Float) — output / display; compositor shows this while active.
    pub b_canvas_id: Uuid,
    /// C canvas (Rgba16Float) — scratch; tools accumulate dabs here across the stroke.
    pub c_canvas_id: Uuid,
    /// Pixel dimensions.  A, B, C, and mask are all guaranteed to be this size.
    pub width:  u32,
    pub height: u32,
    /// Top-left position in document-pixel space.
    /// `(0, 0)` for a layer workspace; `(float.x, float.y)` for a float workspace.
    pub x: i32,
    pub y: i32,
    /// Where the workspace came from — drives commit behaviour.
    pub source: WorkspaceSource,
    /// CPU snapshot taken at mousedown for undo / cancel.
    /// Length is always `width * height * 4` (sRGB premultiplied RGBA).
    pub before_pixels: Vec<u8>,
}

impl RasterWorkspace {
    /// Returns the three canvas UUIDs as an array (convenient for bulk removal).
    pub fn canvas_ids(&self) -> [Uuid; 3] {
        [self.a_canvas_id, self.b_canvas_id, self.c_canvas_id]
    }
}

// ── WorkspaceInitPacket ───────────────────────────────────────────────────────

/// Data sent to `prepare()` on the first frame to create and upload the A/B/C canvases.
///
/// The canvas UUIDs are pre-allocated in `begin_raster_workspace()` (UI thread).
/// The actual `wgpu::Texture` creation and pixel upload happens in `prepare()`.
pub struct WorkspaceInitPacket {
    /// A canvas UUID (already in `RasterWorkspace::a_canvas_id`).
    pub a_canvas_id: Uuid,
    /// Pixel data to upload to A.  Length must equal `width * height * 4`.
    pub a_pixels: Vec<u8>,
    /// B canvas UUID.
    pub b_canvas_id: Uuid,
    /// C canvas UUID.
    pub c_canvas_id: Uuid,
    pub width:  u32,
    pub height: u32,
}

// ── ActiveToolRender ──────────────────────────────────────────────────────────

/// Passed to `VelloRenderContext` so the compositor can blit the tool's B output
/// in the correct position in the layer stack.
///
/// While an `ActiveToolRender` is set:
/// - If `layer_id == Some(id)`: blit B at that layer's compositor slot.
/// - If `layer_id == None`: blit B at the float's compositor slot.
#[derive(Clone, Debug)]
pub struct ActiveToolRender {
    /// B canvas to blit.
    pub b_canvas_id: Uuid,
    /// Position of the B canvas in document space.
    pub x: i32,
    pub y: i32,
    /// Pixel dimensions of the B canvas.
    pub width:  u32,
    pub height: u32,
    /// `Some(layer_id)` → B replaces this layer's render slot.
    /// `None`           → B replaces the float render slot.
    pub layer_id: Option<Uuid>,
}

// ── PendingGpuWork ────────────────────────────────────────────────────────────

/// GPU work to execute in `VelloCallback::prepare()`.
///
/// Tools compute dab lists and other CPU-side data in `update()` (UI thread),
/// store them as a `Box<dyn PendingGpuWork>`, and return that work through
/// `RasterTool::take_pending_gpu_work()` each frame.  `prepare()` then calls
/// `execute()` with the render-thread `device`/`queue`/`gpu`.
///
/// `execute()` takes `&self` so the work object need not be consumed; it lives
/// in the `VelloRenderContext` (which is immutable in `prepare()`).
pub trait PendingGpuWork: Send + Sync {
    fn execute(
        &self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        gpu:    &mut crate::gpu_brush::GpuBrushEngine,
    );
}

// ── RasterTool trait ──────────────────────────────────────────────────────────

/// Unified interface for all raster tools.
///
/// All methods run on the UI thread.  They update the tool's internal state
/// and store pending GPU op descriptors in fields that `StagePane` forwards
/// to `VelloRenderContext` for execution by `VelloCallback::prepare()`.
pub trait RasterTool: Send + Sync {
    /// Called on **mousedown** after [`begin_raster_workspace`] has allocated and
    /// validated A, B, and C.  The tool should initialise its internal state and
    /// optionally queue an initial GPU dispatch (e.g. identity composite for
    /// transform so the handle frame appears immediately).
    fn begin(
        &mut self,
        ws:       &RasterWorkspace,
        pos:      egui::Vec2,
        dt:       f32,
        settings: &crate::tools::RasterToolSettings,
    );

    /// Called every frame while the pointer is held (including the first drag frame).
    /// The tool should accumulate new work into C and queue a composite A+C→B pass.
    /// `dt` is the elapsed time in seconds since the previous call; used by time-based
    /// brushes (airbrush, etc.) to fire dabs at the correct rate when stationary.
    fn update(
        &mut self,
        ws:       &RasterWorkspace,
        pos:      egui::Vec2,
        dt:       f32,
        settings: &crate::tools::RasterToolSettings,
    );

    /// Called on **pointer release**.  Returns `true` if a GPU readback of B should
    /// be performed and the result committed to the document.  Returns `false` if
    /// the operation was a no-op (e.g. the pointer never moved).
    fn finish(&mut self, ws: &RasterWorkspace) -> bool;

    /// Called once per frame (in the VelloCallback construction, UI thread) to
    /// extract pending GPU work accumulated by `begin()` / `update()`.
    ///
    /// The tool clears its internal pending work and returns it.  `prepare()` on
    /// the render thread then calls `work.execute()`.  Default: no GPU work.
    fn take_pending_gpu_work(&mut self) -> Option<Box<dyn PendingGpuWork>> {
        None
    }
}

// ── BrushRasterTool ───────────────────────────────────────────────────────────

use lightningbeam_core::brush_engine::{BrushEngine, GpuDab, StrokeState};
use lightningbeam_core::brush_settings::BrushSettings;
use lightningbeam_core::raster_layer::{RasterBlendMode, StrokePoint, StrokeRecord};

/// GPU work for one frame of a brush stroke: dispatch dabs into C, then composite A+C→B.
struct PendingBrushWork {
    dabs:     Vec<GpuDab>,
    bbox:     (i32, i32, i32, i32),
    a_id:     Uuid,
    b_id:     Uuid,
    c_id:     Uuid,
    canvas_w: u32,
    canvas_h: u32,
}

impl PendingGpuWork for PendingBrushWork {
    fn execute(
        &self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        gpu:    &mut crate::gpu_brush::GpuBrushEngine,
    ) {
        // 1. Accumulate this frame's dabs into C (if any).
        if !self.dabs.is_empty() {
            gpu.render_dabs(device, queue, self.c_id, &self.dabs, self.bbox, self.canvas_w, self.canvas_h);
        }
        // 2. Always composite A + C → B so B shows A's content even with no dabs this frame.
        //    On begin() with empty C this initialises B = A, avoiding a transparent flash.
        gpu.composite_a_c_to_b(device, queue, self.a_id, self.c_id, self.b_id, self.canvas_w, self.canvas_h);
    }
}

/// Raster tool for paint brushes (Normal blend mode).
///
/// Each `update()` call computes new dabs for that frame and stores them as
/// `PendingBrushWork`.  `take_pending_gpu_work()` hands the work to `prepare()`
/// which dispatches the dab and composite shaders on the render thread.
pub struct BrushRasterTool {
    color:        [f32; 4],
    brush:        BrushSettings,
    blend_mode:   RasterBlendMode,
    stroke_state: StrokeState,
    last_point:   Option<StrokePoint>,
    pending:      Option<Box<PendingBrushWork>>,
    /// True after at least one non-empty frame (so finish() knows a commit is needed).
    has_dabs:     bool,
    /// Offset to convert world coordinates to canvas-local coordinates.
    canvas_offset_x: i32,
    canvas_offset_y: i32,
}

impl BrushRasterTool {
    /// Create a new brush tool.
    ///
    /// `color` — linear premultiplied RGBA, matches the format expected by `GpuDab`.
    pub fn new(
        color:      [f32; 4],
        brush:      BrushSettings,
        blend_mode: RasterBlendMode,
    ) -> Self {
        Self {
            color,
            brush,
            blend_mode,
            stroke_state: StrokeState::new(),
            last_point:   None,
            pending:      None,
            has_dabs:     false,
            canvas_offset_x: 0,
            canvas_offset_y: 0,
        }
    }

    fn make_stroke_point(pos: egui::Vec2, off_x: i32, off_y: i32) -> StrokePoint {
        let pressure = crate::tablet::current_pressure();
        let (tilt_x, tilt_y) = crate::tablet::current_tilt();
        StrokePoint {
            x:         pos.x - off_x as f32,
            y:         pos.y - off_y as f32,
            pressure,
            tilt_x,
            tilt_y,
            timestamp: 0.0,
        }
    }

    fn dispatch_dabs(
        &mut self,
        ws:  &RasterWorkspace,
        pt:  StrokePoint,
        dt:  f32,
    ) {
        // Use a 2-point segment when we have a previous point so the engine
        // interpolates dabs along the path.  First mousedown uses a single point.
        let points = match self.last_point.take() {
            Some(prev) => vec![prev, pt.clone()],
            None       => vec![pt.clone()],
        };
        let record = StrokeRecord {
            brush_settings: self.brush.clone(),
            color:          self.color,
            blend_mode:     self.blend_mode,
            tool_params:    [0.0; 4],
            points,
        };
        let (dabs, bbox) = BrushEngine::compute_dabs(&record, &mut self.stroke_state, dt);
        if !dabs.is_empty() {
            self.has_dabs = true;
            self.pending = Some(Box::new(PendingBrushWork {
                dabs,
                bbox,
                a_id:     ws.a_canvas_id,
                b_id:     ws.b_canvas_id,
                c_id:     ws.c_canvas_id,
                canvas_w: ws.width,
                canvas_h: ws.height,
            }));
        }
        self.last_point = Some(pt);
    }
}

impl RasterTool for BrushRasterTool {
    fn begin(&mut self, ws: &RasterWorkspace, pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.canvas_offset_x = ws.x;
        self.canvas_offset_y = ws.y;
        let pt = Self::make_stroke_point(pos, ws.x, ws.y);
        self.dispatch_dabs(ws, pt, 0.0);
        // Always ensure a composite is queued on begin() so B is initialised from A
        // on the first frame even if no dabs fired (large spacing, etc.).
        if self.pending.is_none() {
            self.pending = Some(Box::new(PendingBrushWork {
                dabs:     vec![],
                bbox:     (0, 0, ws.width as i32, ws.height as i32),
                a_id:     ws.a_canvas_id,
                b_id:     ws.b_canvas_id,
                c_id:     ws.c_canvas_id,
                canvas_w: ws.width,
                canvas_h: ws.height,
            }));
        }
    }

    fn update(&mut self, ws: &RasterWorkspace, pos: egui::Vec2, dt: f32, _settings: &crate::tools::RasterToolSettings) {
        let pt = Self::make_stroke_point(pos, ws.x, ws.y);
        self.dispatch_dabs(ws, pt, dt);
    }

    fn finish(&mut self, _ws: &RasterWorkspace) -> bool {
        self.has_dabs
    }

    fn take_pending_gpu_work(&mut self) -> Option<Box<dyn PendingGpuWork>> {
        self.pending.take().map(|w| w as Box<dyn PendingGpuWork>)
    }
}
