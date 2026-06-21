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

use std::sync::Arc;
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
        /// The keyframe's own UUID (the A-canvas key in `GpuBrushEngine`).
        kf_id:    Uuid,
        /// Full canvas dimensions (may differ from workspace dims for floating selections).
        canvas_w: u32,
        canvas_h: u32,
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
    /// Optional R8Unorm selection mask (same pixel dimensions as A/B/C).
    /// `None` means the entire workspace is selected.
    pub mask_texture: Option<Arc<wgpu::Texture>>,
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
    /// Panic-safe bounds check.  Asserts that every GPU canvas exists and has
    /// the dimensions declared by this workspace.  Called by the framework
    /// before `begin()` and before each `update()`.
    pub fn validate(&self, gpu: &crate::gpu_brush::GpuBrushEngine) {
        for (name, id) in [
            ("A", self.a_canvas_id),
            ("B", self.b_canvas_id),
            ("C", self.c_canvas_id),
        ] {
            let canvas = gpu.canvases.get(&id).unwrap_or_else(|| {
                panic!(
                    "RasterWorkspace::validate: buffer '{}' (id={}) not found in GpuBrushEngine",
                    name, id
                )
            });
            assert_eq!(
                canvas.width, self.width,
                "RasterWorkspace::validate: buffer '{}' width {} != workspace width {}",
                name, canvas.width, self.width
            );
            assert_eq!(
                canvas.height, self.height,
                "RasterWorkspace::validate: buffer '{}' height {} != workspace height {}",
                name, canvas.height, self.height
            );
        }
        let expected = (self.width * self.height * 4) as usize;
        assert_eq!(
            self.before_pixels.len(), expected,
            "RasterWorkspace::validate: before_pixels.len()={} != expected {}",
            self.before_pixels.len(), expected
        );
    }

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

    /// Called on **Escape** or tool switch mid-stroke.  The caller restores the
    /// source pixels from `ws.before_pixels` without creating an undo entry; the
    /// tool just cleans up internal state.
    fn cancel(&mut self, ws: &RasterWorkspace);

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

    fn cancel(&mut self, _ws: &RasterWorkspace) {
        self.pending = None;
        self.has_dabs = false;
    }

    fn take_pending_gpu_work(&mut self) -> Option<Box<dyn PendingGpuWork>> {
        self.pending.take().map(|w| w as Box<dyn PendingGpuWork>)
    }
}

// ── EffectBrushTool ───────────────────────────────────────────────────────────

/// Raster tool for effect brushes (Blur, Sharpen, Dodge, Burn, Sponge, Desaturate).
///
/// C accumulates a per-pixel influence weight (R channel, 0–255).
/// The composite pass applies the effect to A, scaled by C.r, writing to B:
///   `B = lerp(A, effect(A), C.r)`
///
/// Using C as an influence map (rather than accumulating modified pixels) prevents
/// overlapping dabs from compounding the effect beyond the C.r cap (255).
///
/// # GPU implementation (TODO)
/// Requires a dedicated `effect_brush_composite.wgsl` shader that reads A and C,
/// applies the blend-mode-specific filter to A, and blends by C.r → B.
pub struct EffectBrushTool {
    brush:      BrushSettings,
    blend_mode: RasterBlendMode,
    has_dabs:   bool,
}

impl EffectBrushTool {
    pub fn new(brush: BrushSettings, blend_mode: RasterBlendMode) -> Self {
        Self { brush, blend_mode, has_dabs: false }
    }
}

impl RasterTool for EffectBrushTool {
    fn begin(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {}
    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.has_dabs = true; // placeholder
    }
    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dabs }
    fn cancel(&mut self, _ws: &RasterWorkspace) { self.has_dabs = false; }
    // GPU shaders not yet implemented; take_pending_gpu_work returns None (default).
}

// ── SmudgeTool ────────────────────────────────────────────────────────────────

/// Raster tool for the smudge brush.
///
/// `begin()`: copy A → C so C starts with the source pixels for color pickup.
/// `update()`: dispatch smudge dabs using `blend_mode=2` (reads C as source,
///   writes smear to C); then composite C over A → B.
///   Because the smudge shader reads from `canvas_src` (C.src) and writes to
///   `canvas_dst` (C.dst), existing dabs are preserved in the smear history.
///
/// # GPU implementation (TODO)
/// Requires an initial A → C copy in `begin()` (via GPU copy command).
/// The smudge dab dispatch then uses `render_dabs(c_id, smudge_dabs, ...)`.
/// The composite pass is `composite_a_c_to_b` (same as BrushRasterTool).
pub struct SmudgeTool {
    brush:    BrushSettings,
    has_dabs: bool,
}

impl SmudgeTool {
    pub fn new(brush: BrushSettings) -> Self {
        Self { brush, has_dabs: false }
    }
}

impl RasterTool for SmudgeTool {
    fn begin(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {}
    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.has_dabs = true; // placeholder
    }
    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dabs }
    fn cancel(&mut self, _ws: &RasterWorkspace) { self.has_dabs = false; }
    // GPU shaders not yet implemented; take_pending_gpu_work returns None (default).
}

// ── GradientRasterTool ────────────────────────────────────────────────────────

use crate::gpu_brush::GpuGradientStop;
use lightningbeam_core::gradient::{GradientExtend, GradientType, ShapeGradient};

fn gradient_stops_to_gpu(gradient: &ShapeGradient) -> Vec<GpuGradientStop> {
    gradient.stops.iter().map(|s| {
        GpuGradientStop::from_srgb_u8(s.position, s.color.r, s.color.g, s.color.b, s.color.a)
    }).collect()
}

fn gradient_extend_to_u32(extend: GradientExtend) -> u32 {
    match extend {
        GradientExtend::Pad     => 0,
        GradientExtend::Reflect => 1,
        GradientExtend::Repeat  => 2,
    }
}

fn gradient_kind_to_u32(kind: GradientType) -> u32 {
    match kind {
        GradientType::Linear => 0,
        GradientType::Radial => 1,
    }
}

struct PendingGradientWork {
    a_id:        Uuid,
    b_id:        Uuid,
    stops:       Vec<GpuGradientStop>,
    start:       (f32, f32),
    end:         (f32, f32),
    opacity:     f32,
    extend_mode: u32,
    kind:        u32,
}

impl PendingGpuWork for PendingGradientWork {
    fn execute(&self, device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut crate::gpu_brush::GpuBrushEngine) {
        gpu.apply_gradient_fill(
            device, queue,
            &self.a_id, &self.b_id,
            &self.stops,
            self.start, self.end,
            self.opacity, self.extend_mode, self.kind,
        );
    }
}

/// Raster tool for gradient fills.
///
/// `begin()` records the canvas-local start position.
/// `update()` recomputes gradient parameters from settings and queues a
/// `PendingGradientWork` that calls `apply_gradient_fill` in `prepare()`.
/// `finish()` returns whether any gradient was dispatched.
pub struct GradientRasterTool {
    start_canvas:   egui::Vec2,
    end_canvas:     egui::Vec2,
    pending:        Option<Box<PendingGradientWork>>,
    has_dispatched: bool,
}

impl GradientRasterTool {
    pub fn new() -> Self {
        Self {
            start_canvas:   egui::Vec2::ZERO,
            end_canvas:     egui::Vec2::ZERO,
            pending:        None,
            has_dispatched: false,
        }
    }
}

impl RasterTool for GradientRasterTool {
    fn begin(&mut self, ws: &RasterWorkspace, pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        let canvas_pos = pos - egui::vec2(ws.x as f32, ws.y as f32);
        self.start_canvas = canvas_pos;
        self.end_canvas   = canvas_pos;
    }

    fn update(&mut self, ws: &RasterWorkspace, pos: egui::Vec2, _dt: f32, settings: &crate::tools::RasterToolSettings) {
        self.end_canvas = pos - egui::vec2(ws.x as f32, ws.y as f32);
        let gradient = &settings.gradient;
        self.pending = Some(Box::new(PendingGradientWork {
            a_id:        ws.a_canvas_id,
            b_id:        ws.b_canvas_id,
            stops:       gradient_stops_to_gpu(gradient),
            start:       (self.start_canvas.x, self.start_canvas.y),
            end:         (self.end_canvas.x,   self.end_canvas.y),
            opacity:     settings.gradient_opacity,
            extend_mode: gradient_extend_to_u32(gradient.extend),
            kind:        gradient_kind_to_u32(gradient.kind),
        }));
        self.has_dispatched = true;
    }

    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dispatched }

    fn cancel(&mut self, _ws: &RasterWorkspace) {
        self.pending        = None;
        self.has_dispatched = false;
    }

    fn take_pending_gpu_work(&mut self) -> Option<Box<dyn PendingGpuWork>> {
        self.pending.take().map(|w| w as Box<dyn PendingGpuWork>)
    }
}

// ── TransformRasterTool ───────────────────────────────────────────────────────

use crate::gpu_brush::RasterTransformGpuParams;

struct PendingTransformWork {
    a_id:   Uuid,
    b_id:   Uuid,
    params: RasterTransformGpuParams,
}

impl PendingGpuWork for PendingTransformWork {
    fn execute(&self, device: &wgpu::Device, queue: &wgpu::Queue, gpu: &mut crate::gpu_brush::GpuBrushEngine) {
        gpu.render_transform(device, queue, &self.a_id, &self.b_id, self.params);
    }
}

/// Raster tool for affine transforms (move, scale, rotate, shear).
///
/// `begin()` stores the initial canvas dimensions and queues an identity
/// transform so B is initialised on the first frame.
/// `update()` recomputes the inverse affine matrix from the current handle
/// positions and queues a new `PendingTransformWork`.
///
/// The inverse matrix maps output pixel coordinates back to source pixel
/// coordinates:  `src = M_inv * dst + b`
/// where `M_inv = [[a00, a01], [a10, a11]]` and `b = [b0, b1]`.
///
/// # GPU implementation
/// Fully wired — uses `GpuBrushEngine::render_transform`.  Handle interaction
/// logic (drag, rotate, scale) is handled by the tool's `update()` caller in
/// `stage.rs` which computes and passes in the `RasterTransformGpuParams`.
pub struct TransformRasterTool {
    pending:        Option<Box<PendingTransformWork>>,
    has_dispatched: bool,
    canvas_w:       u32,
    canvas_h:       u32,
}

impl TransformRasterTool {
    pub fn new() -> Self {
        Self {
            pending:        None,
            has_dispatched: false,
            canvas_w:       0,
            canvas_h:       0,
        }
    }

    /// Queue a transform with the given inverse-affine matrix.
    /// Called by the stage handler after computing handle positions.
    pub fn set_transform(
        &mut self,
        ws:     &RasterWorkspace,
        params: RasterTransformGpuParams,
    ) {
        self.pending = Some(Box::new(PendingTransformWork {
            a_id:   ws.a_canvas_id,
            b_id:   ws.b_canvas_id,
            params,
        }));
        self.has_dispatched = true;
    }
}

impl RasterTool for TransformRasterTool {
    fn begin(&mut self, ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.canvas_w = ws.width;
        self.canvas_h = ws.height;
        // Queue identity transform so B shows the source immediately.
        let identity = RasterTransformGpuParams {
            a00: 1.0, a01: 0.0,
            a10: 0.0, a11: 1.0,
            b0: 0.0, b1: 0.0,
            src_w: ws.width,  src_h: ws.height,
            dst_w: ws.width,  dst_h: ws.height,
            _pad0: 0, _pad1: 0,
        };
        self.set_transform(ws, identity);
    }

    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        // Handle interaction and matrix updates are driven from stage.rs via set_transform().
    }

    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dispatched }

    fn cancel(&mut self, _ws: &RasterWorkspace) {
        self.pending        = None;
        self.has_dispatched = false;
    }

    fn take_pending_gpu_work(&mut self) -> Option<Box<dyn PendingGpuWork>> {
        self.pending.take().map(|w| w as Box<dyn PendingGpuWork>)
    }
}

// ── WarpRasterTool ────────────────────────────────────────────────────────────

/// Raster tool for warp / mesh deformation.
///
/// Uses a displacement buffer (managed by `GpuBrushEngine`) that maps each
/// output pixel to a source offset.  The displacement grid is updated by
/// dragging control points; the warp shader reads anchor pixels + displacement
/// → B each frame.
///
/// # GPU implementation (TODO)
/// Requires: `create_displacement_buf`, `apply_warp` already exist in
/// `GpuBrushEngine`.  Wire brush-drag interaction to update displacement
/// entries and call `apply_warp`.
pub struct WarpRasterTool {
    has_dispatched: bool,
}

impl WarpRasterTool {
    pub fn new() -> Self { Self { has_dispatched: false } }
}

impl RasterTool for WarpRasterTool {
    fn begin(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {}
    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.has_dispatched = true; // placeholder
    }
    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dispatched }
    fn cancel(&mut self, _ws: &RasterWorkspace) { self.has_dispatched = false; }
    // take_pending_gpu_work: default (None) — full GPU wiring is TODO.
}

// ── LiquifyRasterTool ─────────────────────────────────────────────────────────

/// Raster tool for liquify (per-pixel displacement painting).
///
/// Similar to `WarpRasterTool` but uses a full per-pixel displacement map
/// (grid_cols = grid_rows = 0 in `apply_warp`) painted by brush strokes.
/// Each dab accumulates displacement in the push/pull/swirl direction.
///
/// # GPU implementation (TODO)
/// Requires: a dab-to-displacement shader that accumulates per-pixel offsets
/// into the displacement buffer, then `apply_warp` reads it → B.
pub struct LiquifyRasterTool {
    has_dispatched: bool,
}

impl LiquifyRasterTool {
    pub fn new() -> Self { Self { has_dispatched: false } }
}

impl RasterTool for LiquifyRasterTool {
    fn begin(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {}
    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.has_dispatched = true; // placeholder
    }
    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { self.has_dispatched }
    fn cancel(&mut self, _ws: &RasterWorkspace) { self.has_dispatched = false; }
    // take_pending_gpu_work: default (None) — full GPU wiring is TODO.
}

// ── SelectionTool ─────────────────────────────────────────────────────────────

/// Raster selection tool (Magic Wand / Quick Select).
///
/// C (RGBA8) acts as the growing selection; C.r = mask value (0 or 255).
/// Each `update()` frame a flood-fill / region-grow shader extends C.r.
/// The composite pass draws A + a tinted overlay from C.r → B so the user
/// sees the growing selection boundary.
///
/// `finish()` returns false (commit does not write pixels back to the layer;
/// instead the caller extracts C.r into the standalone `R8Unorm` selection
/// texture via `shared.raster_selection`).
///
/// # GPU implementation (TODO)
/// Requires: a flood-fill compute shader seeded by the click position that
/// grows the selection in C.r; and a composite shader that tints selected
/// pixels blue/cyan for preview.
pub struct SelectionTool {
    has_selection: bool,
}

impl SelectionTool {
    pub fn new() -> Self { Self { has_selection: false } }
}

impl RasterTool for SelectionTool {
    fn begin(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {}
    fn update(&mut self, _ws: &RasterWorkspace, _pos: egui::Vec2, _dt: f32, _settings: &crate::tools::RasterToolSettings) {
        self.has_selection = true; // placeholder
    }
    /// Selection tools never trigger a pixel readback/commit on mouseup.
    /// The caller reads C.r directly into the selection mask texture.
    fn finish(&mut self, _ws: &RasterWorkspace) -> bool { false }
    fn cancel(&mut self, _ws: &RasterWorkspace) { self.has_selection = false; }
    // take_pending_gpu_work: default (None) — full GPU wiring is TODO.
}
