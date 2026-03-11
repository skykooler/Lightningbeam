/// Stage pane - main animation canvas with Vello rendering
///
/// Renders composited layers using Vello GPU renderer via egui callbacks.
/// Supports HDR compositing pipeline with per-layer buffers and effects.

use eframe::egui;
use lightningbeam_core::action::Action;
use lightningbeam_core::clip::ClipInstance;
use lightningbeam_core::gpu::{BufferPool, BufferFormat, BufferSpec, Compositor, EffectProcessor, SrgbToLinearConverter};
use lightningbeam_core::layer::{AnyLayer, AudioLayer};
use lightningbeam_core::renderer::RenderedLayerType;
use super::{DragClipType, NodePath, PaneRenderer, SharedPaneState};
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

/// When set to `true` (via `--cpu-renderer`), forces Vello to use its CPU
/// rendering path regardless of GPU capability.
pub static FORCE_CPU_RENDERER: AtomicBool = AtomicBool::new(false);

/// Enable HDR compositing pipeline (per-layer rendering with proper opacity)
/// Set to true to use the new pipeline, false for legacy single-scene rendering
const USE_HDR_COMPOSITING: bool = true; // Enabled for testing

/// Shared Vello resources (created once, reused by all Stage panes)
struct SharedVelloResources {
    renderer: Arc<Mutex<vello::Renderer>>,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    /// HDR to sRGB blit pipeline (linear→sRGB conversion for display)
    hdr_blit_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Shared image cache for avoiding re-decoding images every frame
    image_cache: Mutex<lightningbeam_core::renderer::ImageCache>,
    /// Video manager for video decoding and frame caching
    video_manager: std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>,
    /// Buffer pool for HDR compositing pipeline
    buffer_pool: Mutex<BufferPool>,
    /// Compositor for layer blending
    compositor: Compositor,
    /// Effect processor for GPU shader effects
    effect_processor: Mutex<EffectProcessor>,
    /// sRGB to linear color converter (for Vello output)
    srgb_to_linear: SrgbToLinearConverter,
    /// GPU raster brush engine (compute pipeline + canvas texture cache)
    gpu_brush: Mutex<crate::gpu_brush::GpuBrushEngine>,
    /// Canvas blit pipeline (renders GPU canvas to layer sRGB buffer)
    canvas_blit: crate::gpu_brush::CanvasBlitPipeline,
}

/// Per-instance Vello resources (created for each Stage pane)
struct InstanceVelloResources {
    /// Output texture (Rgba8Unorm for legacy, used for final blit)
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    blit_bind_group: Option<wgpu::BindGroup>,
    /// HDR composite texture (Rgba16Float for internal compositing)
    hdr_texture: Option<wgpu::Texture>,
    hdr_texture_view: Option<wgpu::TextureView>,
    /// Bind group for HDR to sRGB conversion
    hdr_blit_bind_group: Option<wgpu::BindGroup>,
}

/// Container for all Vello instances, stored in egui's CallbackResources
pub struct VelloResourcesMap {
    shared: Option<Arc<SharedVelloResources>>,
    instances: std::collections::HashMap<u64, InstanceVelloResources>,
}

impl SharedVelloResources {
    pub fn new(device: &wgpu::Device, video_manager: std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>, target_format: wgpu::TextureFormat) -> Result<Self, String> {
        let use_cpu = FORCE_CPU_RENDERER.load(Ordering::Relaxed);

        // wgpu panics (rather than returning Err) when shader validation fails, so we
        // catch panics here and fall back to Vello's CPU renderer.  This commonly
        // happens on old GPUs lacking SHADER_FLOAT16_IN_FLOAT32 (required by Vello's
        // flatten shader via unpack2x16float).  The CPU path uses pre-compiled Rust
        // implementations of the same compute shaders, so no GPU shader compilation
        // occurs and the capability check is bypassed entirely.
        let gpu_result = if use_cpu {
            // Skip GPU attempt entirely when forced via --cpu-renderer.
            Err(Box::new("cpu-renderer flag set") as Box<dyn std::any::Any + Send>)
        } else {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                vello::Renderer::new(
                    device,
                    vello::RendererOptions {
                        use_cpu: false,
                        antialiasing_support: vello::AaSupport::all(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                    },
                )
            }))
        };
        let renderer = match gpu_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Err(format!("Failed to create Vello renderer: {e}")),
            Err(_) => {
                if !use_cpu {
                    eprintln!(
                        "WARNING: GPU Vello renderer failed to initialise (missing shader \
                         capability). Falling back to CPU renderer — performance may be reduced."
                    );
                }
                vello::Renderer::new(
                    device,
                    vello::RendererOptions {
                        use_cpu: true,
                        antialiasing_support: vello::AaSupport::all(),
                        num_init_threads: std::num::NonZeroUsize::new(1),
                        pipeline_cache: None,
                    },
                ).map_err(|e| format!("CPU fallback renderer also failed: {e}"))?
            }
        };

        // Create blit shader for rendering texture to screen
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("vello_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        // Create bind group layout for texture + sampler
        let blit_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vello_blit_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("vello_blit_pipeline_layout"),
            bind_group_layouts: &[&blit_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create render pipeline for blitting
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("vello_blit_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format, // Use egui's actual target format
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create HDR blit pipeline (linear→sRGB conversion for display output)
        // Uses linear_to_srgb.wgsl which reads from Rgba16Float HDR texture
        let hdr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hdr_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/linear_to_srgb.wgsl").into()),
        });

        let hdr_blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hdr_blit_pipeline"),
            layout: Some(&pipeline_layout), // Reuse same layout (texture + sampler)
            vertex: wgpu::VertexState {
                module: &hdr_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &hdr_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm, // Intermediate texture format (not swapchain)
                    blend: None, // No blending - direct replacement
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vello_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Initialize buffer pool for HDR compositing
        let buffer_pool = BufferPool::new();

        // Initialize compositor for layer blending
        // Use HDR format for internal compositing
        let compositor = Compositor::new(device, lightningbeam_core::gpu::HDR_FORMAT);

        // Initialize effect processor for GPU shader effects
        let effect_processor = EffectProcessor::new(device, lightningbeam_core::gpu::HDR_FORMAT);

        // Initialize sRGB to linear converter for Vello output
        let srgb_to_linear = SrgbToLinearConverter::new(device);

        // Initialize GPU raster brush engine
        let gpu_brush = crate::gpu_brush::GpuBrushEngine::new(device);
        let canvas_blit = crate::gpu_brush::CanvasBlitPipeline::new(device);

        println!("✅ Vello shared resources initialized (renderer, shaders, HDR compositor, effect processor, color converter, and GPU brush engine)");

        Ok(Self {
            renderer: Arc::new(Mutex::new(renderer)),
            blit_pipeline,
            blit_bind_group_layout,
            hdr_blit_pipeline,
            sampler,
            image_cache: Mutex::new(lightningbeam_core::renderer::ImageCache::new()),
            video_manager,
            buffer_pool: Mutex::new(buffer_pool),
            compositor,
            effect_processor: Mutex::new(effect_processor),
            srgb_to_linear,
            gpu_brush: Mutex::new(gpu_brush),
            canvas_blit,
        })
    }
}

impl InstanceVelloResources {
    pub fn new() -> Self {
        Self {
            texture: None,
            texture_view: None,
            blit_bind_group: None,
            hdr_texture: None,
            hdr_texture_view: None,
            hdr_blit_bind_group: None,
        }
    }

    fn ensure_texture(&mut self, device: &wgpu::Device, shared: &SharedVelloResources, width: u32, height: u32) {
        // Clamp to GPU limits (most GPUs support up to 8192)
        let max_texture_size = 8192;
        let width = width.min(max_texture_size);
        let height = height.min(max_texture_size);

        // Only recreate if size changed
        if let Some(tex) = &self.texture {
            if tex.width() == width && tex.height() == height {
                return;
            }
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("vello_output"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            // RENDER_ATTACHMENT needed for HDR blit, STORAGE_BINDING for Vello
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create bind group for blit pipeline (using shared layout and sampler)
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("vello_blit_bind_group"),
            layout: &shared.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shared.sampler),
                },
            ],
        });

        self.texture = Some(texture);
        self.texture_view = Some(texture_view);
        self.blit_bind_group = Some(bind_group);
    }

    /// Ensure HDR texture exists for compositing pipeline
    fn ensure_hdr_texture(&mut self, device: &wgpu::Device, shared: &SharedVelloResources, width: u32, height: u32) {
        // Clamp to GPU limits
        let max_texture_size = 8192;
        let width = width.min(max_texture_size);
        let height = height.min(max_texture_size);

        // Only recreate if size changed
        if let Some(tex) = &self.hdr_texture {
            if tex.width() == width && tex.height() == height {
                return;
            }
        }

        // Create HDR texture (Rgba16Float for internal compositing)
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_composite_output"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: lightningbeam_core::gpu::HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let hdr_texture_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create bind group for HDR to sRGB conversion (uses same layout as blit)
        let hdr_blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("hdr_blit_bind_group"),
            layout: &shared.blit_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shared.sampler),
                },
            ],
        });

        self.hdr_texture = Some(hdr_texture);
        self.hdr_texture_view = Some(hdr_texture_view);
        self.hdr_blit_bind_group = Some(hdr_blit_bind_group);
    }
}

/// Context for Vello rendering — groups all state needed for the paint callback
struct VelloRenderContext {
    /// Viewport rectangle in screen coordinates
    rect: egui::Rect,
    /// Camera pan offset
    pan_offset: egui::Vec2,
    /// Camera zoom level
    zoom: f32,
    /// Unique instance ID for GPU resource caching
    instance_id: u64,
    /// Document snapshot
    document: std::sync::Arc<lightningbeam_core::document::Document>,
    /// Current tool interaction state
    tool_state: lightningbeam_core::tool::ToolState,
    /// Active layer for tool operations
    active_layer_id: Option<uuid::Uuid>,
    /// Delta for drag preview (world space)
    drag_delta: Option<vello::kurbo::Vec2>,
    /// Current selection state
    selection: lightningbeam_core::selection::Selection,
    /// Current fill color for shape previews
    fill_color: egui::Color32,
    /// Current stroke color for shape previews
    stroke_color: egui::Color32,
    /// Current stroke width for shape previews
    stroke_width: f64,
    /// Current tool (for rendering mode-specific UI)
    selected_tool: lightningbeam_core::tool::Tool,
    /// Whether fill is enabled for shape creation previews
    fill_enabled: bool,
    /// Pending eyedropper sample request
    eyedropper_request: Option<(egui::Pos2, super::ColorMode)>,
    /// Current playback time for animation evaluation
    playback_time: f64,
    /// Video frame manager
    video_manager: std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>,
    /// Surface format for blit pipelines
    target_format: wgpu::TextureFormat,
    /// Which VectorClip is being edited (None = document root)
    #[allow(dead_code)]
    editing_clip_id: Option<uuid::Uuid>,
    /// The clip instance ID being edited (for skip + re-render)
    editing_instance_id: Option<uuid::Uuid>,
    /// The parent layer ID containing the clip instance being edited
    editing_parent_layer_id: Option<uuid::Uuid>,
    /// Active region selection state (for rendering boundary overlay)
    region_selection: Option<lightningbeam_core::selection::RegionSelection>,
    /// Mouse position in document-local (clip-local) world coordinates, for hover hit testing
    mouse_world_pos: Option<vello::kurbo::Point>,
    /// Latest webcam frame for live preview (if any camera is active)
    webcam_frame: Option<lightningbeam_core::webcam::CaptureFrame>,
    /// GPU brush dabs to dispatch in this frame's prepare() call.
    pending_raster_dabs: Option<PendingRasterDabs>,
    /// GPU affine-resample dispatch for the raster transform tool.
    pending_transform_dispatch: Option<PendingTransformDispatch>,
    /// When Some, override the float canvas blit with the display canvas during transform.
    transform_display: Option<TransformDisplayInfo>,
    /// GPU ops for Warp/Liquify tools to dispatch in prepare().
    pending_warp_ops: Vec<PendingWarpOp>,
    /// When Some, override the layer's raster blit with the warp display canvas.
    warp_display: Option<(uuid::Uuid, uuid::Uuid)>,  // (layer_id, display_canvas_id)
    /// Pending GPU gradient fill dispatch for next prepare() frame.
    pending_gradient_op: Option<PendingGradientOp>,
    /// Instance ID (for storing readback results in the global map).
    instance_id_for_readback: u64,
    /// The (layer_id, keyframe_id) of the raster layer with a live GPU canvas.
    /// Present for the entire stroke duration, not just frames with new dabs.
    painting_canvas: Option<(uuid::Uuid, uuid::Uuid)>,
    /// GPU canvas keyframe to remove at the top of this prepare() call.
    pending_canvas_removal: Option<uuid::Uuid>,
    /// True while the current stroke targets the float buffer (B) rather than
    /// the layer canvas (A).  Used in prepare() to route the GPU canvas blit.
    painting_float: bool,
    /// Shared pixel buffer for brush preview thumbnails.
    /// `prepare()` renders all presets here on the first frame;
    /// the infopanel converts the pixel data to egui TextureHandles.
    /// Each entry is `(width, height, sRGB-premultiplied RGBA bytes)`.
    brush_preview_pixels: std::sync::Arc<std::sync::Mutex<Vec<(u32, u32, Vec<u8>)>>>,

    // ── New unified raster tool rendering ─────────────────────────────────────

    /// When `Some`, the compositor blits B (the tool output canvas) at the layer
    /// or float slot described here, instead of the Vello scene / idle raster texture.
    active_tool_render: Option<crate::raster_tool::ActiveToolRender>,
    /// Canvas UUIDs to remove from `GpuBrushEngine` at the top of the next `prepare()`.
    /// Replaced the single `pending_canvas_removal` field.
    pending_canvas_removals: Vec<uuid::Uuid>,
    /// First-frame canvas initialization for the active raster tool workspace.
    /// `prepare()` creates A/B/C canvases and uploads source pixels on the same frame
    /// the tool starts (mousedown).  Cleared after one consume.
    pending_workspace_init: Option<crate::raster_tool::WorkspaceInitPacket>,
    /// GPU work extracted from the active `RasterTool` this frame via
    /// `take_pending_gpu_work()`.  Executed in `prepare()` before compositing.
    pending_tool_gpu_work: Option<Box<dyn crate::raster_tool::PendingGpuWork>>,
    /// Raster layer keyframe UUIDs whose `raster_layer_cache` entry should be
    /// removed at the top of `prepare()` so the fresh `raw_pixels` are re-uploaded.
    /// Populated by the pre-callback dirty-keyframe scan (for undo/redo) and by
    /// stroke/fill/warp commit handlers.
    pending_layer_cache_removals: Vec<uuid::Uuid>,
    /// When `Some`, readback this B-canvas into `RASTER_READBACK_RESULTS` after
    /// dispatching GPU tool work.  Set on mouseup by the unified raster tool commit path.
    pending_tool_readback_b: Option<uuid::Uuid>,
}

/// Callback for Vello rendering within egui
struct VelloCallback {
    ctx: VelloRenderContext,
}

impl egui_wgpu::CallbackTrait for VelloCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Get or create the resources map
        if !resources.contains::<VelloResourcesMap>() {
            resources.insert(VelloResourcesMap {
                shared: None,
                instances: std::collections::HashMap::new(),
            });
        }

        let map: &mut VelloResourcesMap = resources.get_mut().unwrap();

        // Initialize shared resources if not yet created (only happens once for first Stage pane)
        if map.shared.is_none() {
            map.shared = Some(Arc::new(
                SharedVelloResources::new(device, self.ctx.video_manager.clone(), self.ctx.target_format)
                    .unwrap_or_else(|e| panic!("{}", e))
            ));
        }

        let shared = map.shared.as_ref().unwrap().clone();

        // Get or create per-instance resources
        let instance_resources = map.instances.entry(self.ctx.instance_id).or_insert_with(|| {
            println!("✅ Creating instance resources for Stage pane #{}", self.ctx.instance_id);
            InstanceVelloResources::new()
        });

        // Ensure texture is the right size
        let width = self.ctx.rect.width() as u32;
        let height = self.ctx.rect.height() as u32;

        if width == 0 || height == 0 {
            return Vec::new();
        }

        instance_resources.ensure_texture(device, &shared, width, height);

        // Build camera transform: translate for pan, scale for zoom
        use vello::kurbo::Affine;
        let camera_transform = Affine::translate((self.ctx.pan_offset.x as f64, self.ctx.pan_offset.y as f64))
            * Affine::scale(self.ctx.zoom as f64);

        // Overlay transform: camera + clip instance transform (for rendering overlays in clip-local space)
        let overlay_transform = if let (Some(parent_layer_id), Some(instance_id)) = (self.ctx.editing_parent_layer_id, self.ctx.editing_instance_id) {
            let clip_affine = self.ctx.document.get_layer(&parent_layer_id)
                .and_then(|layer| {
                    if let lightningbeam_core::layer::AnyLayer::Vector(vl) = layer {
                        vl.clip_instances.iter().find(|ci| ci.id == instance_id)
                    } else {
                        None
                    }
                })
                .map(|ci| ci.transform.to_affine())
                .unwrap_or(Affine::IDENTITY);
            camera_transform * clip_affine
        } else {
            camera_transform
        };

        // Timing instrumentation: track where frame budget is spent.
        // Prints to stderr when any section exceeds 2 ms, or total > 8 ms.
        let _t_prepare_start = std::time::Instant::now();

        // Choose rendering path based on HDR compositing flag
        let mut scene = if USE_HDR_COMPOSITING {
            // HDR Compositing Pipeline: render each layer separately for proper opacity
            // Uses incremental compositing: render layer → composite onto accumulator → release buffer
            // This means we only need 1 layer buffer at a time (plus the HDR accumulator)
            instance_resources.ensure_hdr_texture(device, &shared, width, height);

            // --- Deferred GPU canvas removal ---
            // The previous frame's render_content consumed a readback result and updated
            // raw_pixels.  Now that the Vello scene is current we can safely drop the
            // GPU canvas; painting_canvas was already cleared so the compositor will use
            // the Vello scene from here on.
            if let Some(kf_id) = self.ctx.pending_canvas_removal {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    gpu_brush.remove_canvas(&kf_id);
                }
            }
            // Process the bulk-removal list (A/B/C canvases from finished tool ops).
            // The Vec was moved into this callback by StagePane via std::mem::take,
            // so it is already gone from StagePane; no drain needed.
            if !self.ctx.pending_canvas_removals.is_empty() {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    for id in &self.ctx.pending_canvas_removals {
                        gpu_brush.remove_canvas(id);
                    }
                }
            }
            // Invalidate raster_layer_cache entries whose raw_pixels changed (undo/redo,
            // stroke commit, fill commit, etc.).  Removing the entry here causes the
            // raster-cache section below to re-upload the fresh pixels on the same frame.
            if !self.ctx.pending_layer_cache_removals.is_empty() {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    for id in &self.ctx.pending_layer_cache_removals {
                        gpu_brush.remove_layer_texture(id);
                    }
                }
            }
            let _t_after_removals = std::time::Instant::now();

            // First-frame canvas initialization for the unified raster tool workspace.
            // Creates A (source), B (output) and C (scratch) canvases; uploads pixels to A.
            // B and C start zero-initialized (transparent).
            if let Some(ref init) = self.ctx.pending_workspace_init {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    // A canvas: source pixels.
                    gpu_brush.ensure_canvas(device, init.a_canvas_id, init.width, init.height);
                    if let Some(canvas) = gpu_brush.canvases.get(&init.a_canvas_id) {
                        canvas.upload(queue, &init.a_pixels);
                    }
                    // B canvas: output (zero-initialized by GPU allocation).
                    gpu_brush.ensure_canvas(device, init.b_canvas_id, init.width, init.height);
                    // C canvas: scratch (zero-initialized by GPU allocation).
                    gpu_brush.ensure_canvas(device, init.c_canvas_id, init.width, init.height);
                }
            }

            // Unified raster tool GPU dispatch (dab shaders, composite pass, etc.).
            if let Some(ref work) = self.ctx.pending_tool_gpu_work {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    work.execute(device, queue, &mut *gpu_brush);
                }
            }

            // Unified tool B-canvas readback on mouseup (commit path).
            // Triggered when the active RasterTool's finish() returns true.
            if let Some(b_id) = self.ctx.pending_tool_readback_b {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    let dims = gpu_brush.canvases.get(&b_id).map(|c| (c.width, c.height));
                    if let Some((w, h)) = dims {
                        if let Some(pixels) = gpu_brush.readback_canvas(device, queue, b_id) {
                            let results = RASTER_READBACK_RESULTS.get_or_init(|| {
                                Arc::new(Mutex::new(std::collections::HashMap::new()))
                            });
                            if let Ok(mut map) = results.lock() {
                                map.insert(self.ctx.instance_id_for_readback, RasterReadbackResult {
                                    layer_id: uuid::Uuid::nil(), // unused; routing via pending_undo_before
                                    time: 0.0,
                                    canvas_width: w,
                                    canvas_height: h,
                                    pixels,
                                });
                            }
                        }
                    }
                }
            }

            // Lazy float GPU canvas initialization.
            // If a float exists but its GPU canvas hasn't been created yet, upload float.pixels now.
            if let Some(ref float_sel) = self.ctx.selection.raster_floating {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    if !gpu_brush.canvases.contains_key(&float_sel.canvas_id) {
                        gpu_brush.ensure_canvas(device, float_sel.canvas_id, float_sel.width, float_sel.height);
                        if let Some(canvas) = gpu_brush.canvases.get(&float_sel.canvas_id) {
                            let pixels = if float_sel.pixels.is_empty() {
                                vec![0u8; (float_sel.width * float_sel.height * 4) as usize]
                            } else {
                                (*float_sel.pixels).clone()
                            };
                            canvas.upload(queue, &pixels);
                        }
                    }
                }
            }

            // --- GPU brush dispatch ---
            // Dispatch the compute shader for any pending raster dabs from this frame's
            // input event.  Must happen before compositing so the updated canvas texture
            // is sampled correctly when the layer is blitted.
            if let Some(ref pending) = self.ctx.pending_raster_dabs {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    // Ensure the canvas pair exists (creates it if missing or wrong size)
                    gpu_brush.ensure_canvas(
                        device,
                        pending.keyframe_id,
                        pending.canvas_width,
                        pending.canvas_height,
                    );
                    // On stroke start, upload the pre-stroke pixel data to both textures
                    if let Some(ref pixels) = pending.initial_pixels {
                        if let Some(canvas) = gpu_brush.canvases.get(&pending.keyframe_id) {
                            canvas.upload(queue, pixels);
                        }
                    }
                    // Dispatch the compute shader for this frame's dabs
                    if !pending.dabs.is_empty() {
                        gpu_brush.render_dabs(
                            device,
                            queue,
                            pending.keyframe_id,
                            &pending.dabs,
                            pending.dab_bbox,
                            pending.canvas_width,
                            pending.canvas_height,
                        );
                    }
                    // On stroke end, read back the finished canvas and store it so
                    // the next ui() call can create the undo action.
                    if pending.wants_final_readback {
                        if let Some(pixels) = gpu_brush.readback_canvas(
                            device,
                            queue,
                            pending.keyframe_id,
                        ) {
                            let results = RASTER_READBACK_RESULTS.get_or_init(|| {
                                Arc::new(Mutex::new(std::collections::HashMap::new()))
                            });
                            if let Ok(mut map) = results.lock() {
                                map.insert(self.ctx.instance_id_for_readback, RasterReadbackResult {
                                    layer_id:      pending.layer_id,
                                    time:          pending.time,
                                    canvas_width:  pending.canvas_width,
                                    canvas_height: pending.canvas_height,
                                    pixels,
                                });
                            }
                            // Canvas is kept alive: the compositor will still blit it
                            // this frame (painting_canvas is still Some).  render_content
                            // will clear painting_canvas and set pending_canvas_removal,
                            // so the texture is freed at the top of the next prepare().
                        }
                    }
                }
            }

            // --- Raster transform dispatch ---
            // Runs after dab dispatch; uploads anchor pixels and runs the affine-resample
            // shader from anchor → display canvas.
            if let Some(ref dispatch) = self.ctx.pending_transform_dispatch {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    // Ensure anchor canvas at original dimensions.
                    gpu_brush.ensure_canvas(device, dispatch.anchor_canvas_id, dispatch.anchor_w, dispatch.anchor_h);
                    if let Some(canvas) = gpu_brush.canvases.get(&dispatch.anchor_canvas_id) {
                        canvas.upload(queue, &dispatch.anchor_pixels);
                    }
                    // Ensure display canvas at new (transformed) dimensions.
                    gpu_brush.ensure_canvas(device, dispatch.display_canvas_id, dispatch.new_w, dispatch.new_h);
                    // Dispatch the affine-resample shader.
                    let params = crate::gpu_brush::RasterTransformGpuParams {
                        a00: dispatch.a00, a01: dispatch.a01,
                        a10: dispatch.a10, a11: dispatch.a11,
                        b0:  dispatch.b0,  b1:  dispatch.b1,
                        src_w: dispatch.anchor_w, src_h: dispatch.anchor_h,
                        dst_w: dispatch.new_w,    dst_h: dispatch.new_h,
                        _pad0: 0, _pad1: 0,
                    };
                    gpu_brush.render_transform(device, queue, &dispatch.anchor_canvas_id, &dispatch.display_canvas_id, params);

                    // Final commit: readback the display canvas so render_content() can swap it in as the new float.
                    if dispatch.is_final_commit {
                        if let Some(pixels) = gpu_brush.readback_canvas(device, queue, dispatch.display_canvas_id) {
                            let results = TRANSFORM_READBACK_RESULTS.get_or_init(|| {
                                Arc::new(Mutex::new(std::collections::HashMap::new()))
                            });
                            if let Ok(mut map) = results.lock() {
                                map.insert(self.ctx.instance_id_for_readback, TransformReadbackResult {
                                    pixels,
                                    width:  dispatch.new_w,
                                    height: dispatch.new_h,
                                    x: dispatch.new_x,
                                    y: dispatch.new_y,
                                    display_canvas_id: dispatch.display_canvas_id,
                                });
                            }
                        }
                    }
                }
            }

            // --- Gradient fill GPU dispatch ---
            if let Some(ref op) = self.ctx.pending_gradient_op {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    // Ensure both canvases exist.
                    gpu_brush.ensure_canvas(device, op.anchor_canvas_id, op.w, op.h);
                    gpu_brush.ensure_canvas(device, op.display_canvas_id, op.w, op.h);
                    // Upload anchor pixels on the first frame (drag start).
                    if let Some(ref pixels) = op.anchor_pixels {
                        if let Some(canvas) = gpu_brush.canvases.get(&op.anchor_canvas_id) {
                            canvas.upload(queue, pixels);
                        }
                    }
                    // Dispatch gradient fill shader.
                    gpu_brush.apply_gradient_fill(
                        device, queue,
                        &op.anchor_canvas_id,
                        &op.display_canvas_id,
                        &op.stops,
                        (op.start_x, op.start_y),
                        (op.end_x,   op.end_y),
                        op.opacity,
                        op.extend_mode,
                        op.kind,
                    );
                }
            }

            // --- Warp / Liquify GPU dispatch ---
            if !self.ctx.pending_warp_ops.is_empty() {
                if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                    let mut final_commit_result: Option<WarpReadbackResult> = None;

                    for op in self.ctx.pending_warp_ops.iter() {
                        match op {
                            PendingWarpOp::Init { anchor_canvas_id, display_canvas_id, disp_buf_id, w, h, anchor_pixels, is_liquify } => {
                                let (w, h) = (*w, *h);
                                // Always upload anchor_pixels: the GPU canvas may be stale
                                // (e.g. merge-down updated kf.raw_pixels but left GPU canvas with old content).
                                gpu_brush.ensure_canvas(device, *anchor_canvas_id, w, h);
                                if let Some(canvas) = gpu_brush.canvases.get(anchor_canvas_id) {
                                    canvas.upload(queue, anchor_pixels);
                                }
                                gpu_brush.ensure_canvas(device, *display_canvas_id, w, h);
                                // Initialise displacement buffer and populate display canvas = anchor.
                                if !gpu_brush.displacement_bufs.contains_key(disp_buf_id) {
                                    if *is_liquify {
                                        // Liquify needs a full per-pixel buffer.
                                        gpu_brush.create_displacement_buf(device, *disp_buf_id, w, h);
                                    } else {
                                        // Warp uses a 1×1 grid buffer (zero = identity).
                                        gpu_brush.create_displacement_buf(device, *disp_buf_id, 1, 1);
                                    }
                                    gpu_brush.clear_displacement_buf(queue, disp_buf_id);
                                }
                                // Apply identity warp so display canvas immediately shows the anchor.
                                let (gc, gr) = if *is_liquify { (0, 0) } else { (1, 1) };
                                gpu_brush.apply_warp(device, queue, anchor_canvas_id, disp_buf_id, display_canvas_id, None, gc, gr);
                            }
                            PendingWarpOp::WarpApply { anchor_canvas_id, disp_buf_id, display_canvas_id, disp_data, grid_cols, grid_rows, final_commit, layer_id, time, is_float_warp, .. } => {
                                // Resize displacement buffer if grid dimensions changed.
                                let needs_resize = gpu_brush.displacement_bufs.get(disp_buf_id)
                                    .map_or(true, |db| db.width != *grid_cols || db.height != *grid_rows);
                                if needs_resize {
                                    gpu_brush.remove_displacement_buf(disp_buf_id);
                                    gpu_brush.create_displacement_buf(device, *disp_buf_id, *grid_cols, *grid_rows);
                                }
                                gpu_brush.apply_warp(device, queue, anchor_canvas_id, disp_buf_id, display_canvas_id, disp_data.as_deref(), *grid_cols, *grid_rows);
                                if *final_commit {
                                    let after_pixels  = gpu_brush.readback_canvas(device, queue, *display_canvas_id);
                                    let before_pixels = gpu_brush.readback_canvas(device, queue, *anchor_canvas_id);
                                    if let (Some(after), Some(before)) = (after_pixels, before_pixels) {
                                        let canvas = gpu_brush.canvases.get(display_canvas_id);
                                        let (fw, fh) = canvas.map(|c| (c.width, c.height)).unwrap_or((0, 0));
                                        final_commit_result = Some(WarpReadbackResult { layer_id: *layer_id, time: *time, before_pixels: before, after_pixels: after, width: fw, height: fh, display_canvas_id: *display_canvas_id, anchor_canvas_id: *anchor_canvas_id, is_float_warp: *is_float_warp });
                                    }
                                }
                            }
                            PendingWarpOp::LiquifyBrushStep { disp_buf_id, params } => {
                                gpu_brush.liquify_brush_step(device, queue, disp_buf_id, *params);
                            }
                            PendingWarpOp::LiquifyApply { anchor_canvas_id, disp_buf_id, display_canvas_id, final_commit, layer_id, time, is_float_warp, .. } => {
                                // Per-pixel mode: grid_cols = 0.
                                gpu_brush.apply_warp(device, queue, anchor_canvas_id, disp_buf_id, display_canvas_id, None, 0, 0);
                                if *final_commit {
                                    let after_pixels  = gpu_brush.readback_canvas(device, queue, *display_canvas_id);
                                    let before_pixels = gpu_brush.readback_canvas(device, queue, *anchor_canvas_id);
                                    if let (Some(after), Some(before)) = (after_pixels, before_pixels) {
                                        let canvas = gpu_brush.canvases.get(display_canvas_id);
                                        let (fw, fh) = canvas.map(|c| (c.width, c.height)).unwrap_or((0, 0));
                                        final_commit_result = Some(WarpReadbackResult { layer_id: *layer_id, time: *time, before_pixels: before, after_pixels: after, width: fw, height: fh, display_canvas_id: *display_canvas_id, anchor_canvas_id: *anchor_canvas_id, is_float_warp: *is_float_warp });
                                    }
                                }
                            }
                        }
                    }

                    if let Some(result) = final_commit_result {
                        let results = WARP_READBACK_RESULTS.get_or_init(|| {
                            Arc::new(Mutex::new(std::collections::HashMap::new()))
                        });
                        if let Ok(mut map) = results.lock() {
                            map.insert(self.ctx.instance_id_for_readback, result);
                        }
                    }
                }
            }

            // Generate brush preview thumbnails on first use (one-time, blocking readback).
            if let Ok(mut previews) = self.ctx.brush_preview_pixels.try_lock() {
                if previews.is_empty() {
                    if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                        use lightningbeam_core::brush_engine::{BrushEngine, StrokeState};
                        use lightningbeam_core::raster_layer::{StrokeRecord, StrokePoint, RasterBlendMode};
                        use lightningbeam_core::brush_settings::bundled_brushes;

                        const PW: u32 = 120;
                        const PH: u32 = 56;

                        for preset in bundled_brushes() {
                            let preview_radius = (PH as f32 * 0.22).max(2.5);
                            let mut scaled = preset.settings.clone();
                            scaled.radius_log = preview_radius.ln();
                            scaled.slow_tracking = 0.0;
                            scaled.slow_tracking_per_dab = 0.0;

                            let y_lo  = PH as f32 * 0.72;
                            let y_hi  = PH as f32 * 0.28;
                            let x0    = PW as f32 * 0.10;
                            let x1    = PW as f32 * 0.90;
                            let mid_x = (x0 + x1) * 0.5;
                            let mid_y = (y_lo + y_hi) * 0.5;
                            let stroke = StrokeRecord {
                                brush_settings: scaled,
                                color: [0.85f32, 0.88, 1.0, 1.0],
                                blend_mode: RasterBlendMode::Normal,
                                tool_params: [0.0; 4],
                                points: vec![
                                    StrokePoint { x: x0,    y: y_lo,  pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0 },
                                    StrokePoint { x: mid_x, y: mid_y, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0 },
                                    StrokePoint { x: x1,    y: y_hi,  pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0 },
                                ],
                            };
                            let mut state = StrokeState::new();
                            let (dabs, _) = BrushEngine::compute_dabs(&stroke, &mut state, 0.0);
                            let pixels = gpu_brush.render_to_image(device, queue, &dabs, PW, PH);
                            previews.push((PW, PH, pixels));
                        }
                    }
                }
            }

            let _t_after_gpu_dispatches = std::time::Instant::now();

            let mut image_cache = shared.image_cache.lock().unwrap();

            let composite_result = lightningbeam_core::renderer::render_document_for_compositing(
                &self.ctx.document,
                camera_transform,
                &mut image_cache,
                &shared.video_manager,
                self.ctx.webcam_frame.as_ref(),
                self.ctx.selection.raster_floating.as_ref(),
                true, // Draw checkerboard for transparent backgrounds in the UI
            );
            drop(image_cache);
            let _t_after_scene_build = std::time::Instant::now();

            // Get buffer pool for layer rendering
            let mut buffer_pool = shared.buffer_pool.lock().unwrap();

            // Buffer spec for layer rendering (Vello outputs Rgba8)
            let layer_spec = lightningbeam_core::gpu::BufferSpec::new(
                width,
                height,
                lightningbeam_core::gpu::BufferFormat::Rgba8Srgb,
            );

            // Render parameters for Vello (transparent background for layers)
            let layer_render_params = vello::RenderParams {
                base_color: vello::peniko::Color::TRANSPARENT,
                width,
                height,
                antialiasing_method: vello::AaConfig::Msaa16,
            };

            // HDR buffer spec for linear buffers
            let hdr_spec = BufferSpec::new(width, height, BufferFormat::Rgba16Float);

            // First, render background and composite it
            // The background scene contains only a rectangle at document bounds,
            // so we use TRANSPARENT base_color to not fill the whole viewport
            let bg_srgb_handle = buffer_pool.acquire(device, layer_spec);
            let bg_hdr_handle = buffer_pool.acquire(device, hdr_spec);
            if let (Some(bg_srgb_view), Some(bg_hdr_view), Some(hdr_view)) = (
                buffer_pool.get_view(bg_srgb_handle),
                buffer_pool.get_view(bg_hdr_handle),
                &instance_resources.hdr_texture_view,
            ) {
                // Render background scene with transparent base (scene has the bg rect)
                let bg_render_params = vello::RenderParams {
                    base_color: vello::peniko::Color::TRANSPARENT,
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Msaa16,
                };

                if let Ok(mut renderer) = shared.renderer.lock() {
                    renderer.render_to_texture(device, queue, &composite_result.background, bg_srgb_view, &bg_render_params).ok();
                }

                // Convert sRGB to linear HDR
                let mut convert_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bg_srgb_to_linear_encoder"),
                });
                shared.srgb_to_linear.convert(device, &mut convert_encoder, bg_srgb_view, bg_hdr_view);
                queue.submit(Some(convert_encoder.finish()));

                // Composite background onto HDR texture (first layer, clears to dark gray for stage area)
                let bg_compositor_layer = lightningbeam_core::gpu::CompositorLayer::normal(bg_hdr_handle, 1.0);
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("bg_composite_encoder"),
                });
                // Clear to dark gray (stage background outside document bounds)
                // Note: stage_bg values are already in linear space for HDR compositing
                let stage_bg = [45.0 / 255.0, 45.0 / 255.0, 48.0 / 255.0, 1.0];
                shared.compositor.composite(
                    device,
                    queue,
                    &mut encoder,
                    &[bg_compositor_layer],
                    &buffer_pool,
                    hdr_view,
                    Some(stage_bg),
                );
                queue.submit(Some(encoder.finish()));
            }
            buffer_pool.release(bg_srgb_handle);
            buffer_pool.release(bg_hdr_handle);

            // Build a float-local R8 selection mask for the float canvas blit.
            // Computed every frame from raster_selection so it is always correct
            // (during strokes and during idle move/drag).
            let float_mask_texture: Option<wgpu::Texture> =
                if let Some(ref float_sel) = self.ctx.selection.raster_floating {
                    if let Some(ref sel) = self.ctx.selection.raster_selection {
                        let fw = float_sel.width;
                        let fh = float_sel.height;
                        let fx = float_sel.x;
                        let fy = float_sel.y;
                        let mut pixels = vec![0u8; (fw * fh) as usize];
                        let (x0, y0, x1, y1) = sel.bounding_rect();
                        let bx0 = (x0 - fx).max(0) as u32;
                        let by0 = (y0 - fy).max(0) as u32;
                        let bx1 = ((x1 - fx) as u32).min(fw);
                        let by1 = ((y1 - fy) as u32).min(fh);
                        for py in by0..by1 {
                            for px in bx0..bx1 {
                                if sel.contains_pixel(fx + px as i32, fy + py as i32) {
                                    pixels[(py * fw + px) as usize] = 255;
                                }
                            }
                        }
                        let tex = device.create_texture(&wgpu::TextureDescriptor {
                            label: Some("float_mask_tex"),
                            size: wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu::TextureFormat::R8Unorm,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                            view_formats: &[],
                        });
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &tex,
                                mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            &pixels,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(fw),
                                rows_per_image: Some(fh),
                            },
                            wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
                        );
                        Some(tex)
                    } else {
                        None
                    }
                } else {
                    None
                };
            let float_mask_view: Option<wgpu::TextureView> =
                float_mask_texture.as_ref().map(|t| t.create_view(&Default::default()));

            // Lock effect processor
            let mut effect_processor = shared.effect_processor.lock().unwrap();

            // Now render and composite each layer incrementally
            for rendered_layer in &composite_result.layers {
                // Determine which GPU canvas (if any) to blit for this layer.
                //
                // Priority order:
                // 1. Active tool B canvas (new unified tool render).
                // 2. Legacy painting_canvas (old per-tool render path, kept during migration).
                // 3. Warp/Liquify display canvas.
                // 4. Raster layer texture cache (idle raster layers — bypasses Vello).
                // 5. None → fall through to Vello scene rendering.
                //
                // When painting_float is true, the active tool is working on the float,
                // so the layer itself should still render normally (via Vello or cache).
                let gpu_canvas_kf: Option<uuid::Uuid> = {
                    // 1. New unified tool render: B canvas replaces this layer.
                    let from_tool = self.ctx.active_tool_render.as_ref()
                        .filter(|tr| tr.layer_id == Some(rendered_layer.layer_id))
                        .map(|tr| tr.b_canvas_id);

                    // 2. Legacy painting_canvas (old stroke path).
                    let from_legacy = if self.ctx.painting_float {
                        None
                    } else {
                        self.ctx.painting_canvas
                            .filter(|(layer_id, _)| *layer_id == rendered_layer.layer_id)
                            .map(|(_, kf_id)| kf_id)
                    };

                    // 3. Warp/Liquify display canvas.
                    let from_warp = self.ctx.warp_display
                        .filter(|(layer_id, _)| *layer_id == rendered_layer.layer_id)
                        .map(|(_, display_id)| display_id);

                    from_tool.or(from_legacy).or(from_warp)
                };

                // 4. Raster layer texture cache: for idle raster layers (no active tool canvas).
                // Upload raw_pixels to the cache if texture_dirty; then use the cache entry.
                let raster_cache_kf: Option<uuid::Uuid> = if gpu_canvas_kf.is_none() {
                    // Find the active keyframe for this raster layer.
                    let doc = &self.ctx.document;
                    let raster_kf_id = doc.get_layer(&rendered_layer.layer_id)
                        .and_then(|l| match l {
                            lightningbeam_core::layer::AnyLayer::Raster(rl) => {
                                rl.keyframe_at(self.ctx.playback_time)
                            }
                            _ => None,
                        })
                        .map(|kf| kf.id);

                    if let Some(kf_id) = raster_kf_id {
                        if let Ok(mut gpu_brush) = shared.gpu_brush.lock() {
                            // Check if we have pixels to upload.
                            let kf_data = doc.get_layer(&rendered_layer.layer_id)
                                .and_then(|l| match l {
                                    lightningbeam_core::layer::AnyLayer::Raster(rl) => {
                                        rl.keyframe_at(self.ctx.playback_time)
                                    }
                                    _ => None,
                                });
                            if let Some(kf) = kf_data {
                                if !kf.raw_pixels.is_empty() {
                                    // Pass dirty=false: the cache entry was already removed
                                    // above via pending_layer_cache_removals when raw_pixels
                                    // changed (undo/redo, stroke commit, etc.).  A cache miss
                                    // triggers upload; a cache hit skips the expensive sRGB
                                    // conversion + GPU write that was firing every frame.
                                    gpu_brush.ensure_layer_texture(
                                        device, queue, kf_id,
                                        &kf.raw_pixels,
                                        kf.width, kf.height,
                                        false,
                                    );
                                    Some(kf_id)
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                if !rendered_layer.has_content && gpu_canvas_kf.is_none() && raster_cache_kf.is_none() {
                    continue;
                }

                match &rendered_layer.layer_type {
                    RenderedLayerType::Vector => {
                        // Vector/group layer — render Vello scene → sRGB → linear → composite.
                        let srgb_handle = buffer_pool.acquire(device, layer_spec);
                        let hdr_layer_handle = buffer_pool.acquire(device, hdr_spec);

                        if let (Some(srgb_view), Some(hdr_layer_view), Some(hdr_view)) = (
                            buffer_pool.get_view(srgb_handle),
                            buffer_pool.get_view(hdr_layer_handle),
                            &instance_resources.hdr_texture_view,
                        ) {
                            if let Ok(mut renderer) = shared.renderer.lock() {
                                renderer.render_to_texture(device, queue, &rendered_layer.scene, srgb_view, &layer_render_params).ok();
                            }
                            let mut convert_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("layer_srgb_to_linear_encoder"),
                            });
                            shared.srgb_to_linear.convert(device, &mut convert_encoder, srgb_view, hdr_layer_view);
                            queue.submit(Some(convert_encoder.finish()));

                            let compositor_layer = lightningbeam_core::gpu::CompositorLayer::new(
                                hdr_layer_handle,
                                rendered_layer.opacity,
                                rendered_layer.blend_mode,
                            );
                            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("layer_composite_encoder"),
                            });
                            shared.compositor.composite(
                                device, queue, &mut encoder, &[compositor_layer], &buffer_pool, hdr_view, None,
                            );
                            queue.submit(Some(encoder.finish()));
                        }

                        buffer_pool.release(srgb_handle);
                        buffer_pool.release(hdr_layer_handle);
                    }
                    RenderedLayerType::Raster { transform: layer_transform, .. } => {
                        // Raster layer — GPU canvas blit directly to HDR (bypasses Vello).
                        // Tool override canvas (gpu_canvas_kf) takes priority over cached texture.
                        if let Some(use_kf_id) = gpu_canvas_kf.or(raster_cache_kf) {
                            let hdr_layer_handle = buffer_pool.acquire(device, hdr_spec);
                            if let (Some(hdr_layer_view), Some(hdr_view)) = (
                                buffer_pool.get_view(hdr_layer_handle),
                                &instance_resources.hdr_texture_view,
                            ) {
                                if let Ok(gpu_brush) = shared.gpu_brush.lock() {
                                    let canvas = gpu_brush.canvases.get(&use_kf_id)
                                        .or_else(|| gpu_brush.raster_layer_cache.get(&use_kf_id));
                                    if let Some(canvas) = canvas {
                                        let bt = crate::gpu_brush::BlitTransform::new(
                                            *layer_transform,
                                            canvas.width, canvas.height,
                                            width, height,
                                        );
                                        shared.canvas_blit.blit(
                                            device, queue, canvas.src_view(), hdr_layer_view, &bt, None,
                                        );
                                    }
                                }
                                let compositor_layer = lightningbeam_core::gpu::CompositorLayer::new(
                                    hdr_layer_handle,
                                    rendered_layer.opacity,
                                    rendered_layer.blend_mode,
                                );
                                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("raster_composite_encoder"),
                                });
                                shared.compositor.composite(
                                    device, queue, &mut encoder, &[compositor_layer], &buffer_pool, hdr_view, None,
                                );
                                queue.submit(Some(encoder.finish()));
                            }
                            buffer_pool.release(hdr_layer_handle);
                        }
                    }
                    RenderedLayerType::Video { instances } => {
                        // Video layer — per-instance: upload decoded frame → blit → composite.
                        for inst in instances {
                            if inst.rgba_data.is_empty() { continue; }
                            let hdr_layer_handle = buffer_pool.acquire(device, hdr_spec);
                            if let (Some(hdr_layer_view), Some(hdr_view)) = (
                                buffer_pool.get_view(hdr_layer_handle),
                                &instance_resources.hdr_texture_view,
                            ) {
                                // Convert sRGB straight-alpha → linear premultiplied.
                                let linear: Vec<u8> = inst.rgba_data.chunks_exact(4).flat_map(|p| {
                                    let a = p[3] as f32 / 255.0;
                                    let lin = |c: u8| -> f32 {
                                        let f = c as f32 / 255.0;
                                        if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) }
                                    };
                                    let r = (lin(p[0]) * a * 255.0 + 0.5) as u8;
                                    let g = (lin(p[1]) * a * 255.0 + 0.5) as u8;
                                    let b = (lin(p[2]) * a * 255.0 + 0.5) as u8;
                                    [r, g, b, p[3]]
                                }).collect();

                                let tex = device.create_texture(&wgpu::TextureDescriptor {
                                    label: Some("video_frame_tex"),
                                    size: wgpu::Extent3d { width: inst.width, height: inst.height, depth_or_array_layers: 1 },
                                    mip_level_count: 1, sample_count: 1,
                                    dimension: wgpu::TextureDimension::D2,
                                    format: wgpu::TextureFormat::Rgba8Unorm,
                                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                                    view_formats: &[],
                                });
                                queue.write_texture(
                                    wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                                    &linear,
                                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(inst.width * 4), rows_per_image: Some(inst.height) },
                                    wgpu::Extent3d { width: inst.width, height: inst.height, depth_or_array_layers: 1 },
                                );
                                let tex_view = tex.create_view(&wgpu::TextureViewDescriptor::default());

                                let bt = crate::gpu_brush::BlitTransform::new(
                                    inst.transform, inst.width, inst.height, width, height,
                                );
                                shared.canvas_blit.blit(device, queue, &tex_view, hdr_layer_view, &bt, None);

                                let compositor_layer = lightningbeam_core::gpu::CompositorLayer::new(
                                    hdr_layer_handle,
                                    inst.opacity,
                                    lightningbeam_core::gpu::BlendMode::Normal,
                                );
                                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("video_composite_encoder"),
                                });
                                shared.compositor.composite(
                                    device, queue, &mut encoder, &[compositor_layer], &buffer_pool, hdr_view, None,
                                );
                                queue.submit(Some(encoder.finish()));
                            }
                            buffer_pool.release(hdr_layer_handle);
                        }
                    }
                    RenderedLayerType::Float { canvas_id: float_canvas_id, x: float_x, y: float_y, width: fw, height: fh, transform: layer_transform, pixels: _ } => {
                        // Floating raster selection — now composited at the correct z-position
                        // (immediately above its parent layer) rather than on top of everything.
                        //
                        // Override priority:
                        //   1. transform_display: transform tool is active on the float.
                        //   2. active_tool_render (layer_id=None): unified tool on the float.
                        //   3. float_canvas_id from this entry: normal float display.
                        let blit_params: Option<(uuid::Uuid, i32, i32, u32, u32)> =
                            if let Some(ref td) = self.ctx.transform_display {
                                Some((td.display_canvas_id, td.x, td.y, td.w, td.h))
                            } else if let Some(ref tr) = self.ctx.active_tool_render.as_ref().filter(|tr| tr.layer_id.is_none()) {
                                Some((tr.b_canvas_id, tr.x, tr.y, tr.width, tr.height))
                            } else {
                                Some((*float_canvas_id, *float_x, *float_y, *fw, *fh))
                            };

                        if let Some((blit_canvas_id, blit_x, blit_y, blit_w, blit_h)) = blit_params {
                            if let Ok(gpu_brush) = shared.gpu_brush.lock() {
                                if let Some(canvas) = gpu_brush.canvases.get(&blit_canvas_id) {
                                    let float_hdr_handle = buffer_pool.acquire(device, hdr_spec);
                                    if let (Some(fhdr_view), Some(hdr_view)) = (
                                        buffer_pool.get_view(float_hdr_handle),
                                        &instance_resources.hdr_texture_view,
                                    ) {
                                        // float_canvas_px → viewport_px:
                                        //   layer_transform maps doc_px → viewport_px
                                        //   translate(blit_x, blit_y) maps float_canvas_px → doc_px
                                        let float_to_vp = *layer_transform
                                            * Affine::translate((blit_x as f64, blit_y as f64));
                                        let bt = crate::gpu_brush::BlitTransform::new(
                                            float_to_vp, blit_w, blit_h, width, height,
                                        );
                                        shared.canvas_blit.blit(
                                            device, queue, canvas.src_view(), fhdr_view, &bt,
                                            float_mask_view.as_ref(),
                                        );
                                        let float_layer = lightningbeam_core::gpu::CompositorLayer::normal(float_hdr_handle, 1.0);
                                        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                            label: Some("float_canvas_composite"),
                                        });
                                        shared.compositor.composite(device, queue, &mut enc, &[float_layer], &buffer_pool, hdr_view, None);
                                        queue.submit(Some(enc.finish()));
                                    }
                                    buffer_pool.release(float_hdr_handle);
                                }
                            }
                        }
                    }
                    RenderedLayerType::Effect { effect_instances } => {
                        // Effect layer - apply effects to the current HDR accumulator
                        let current_time = self.ctx.document.current_time;

                        for effect_instance in effect_instances {
                            // Get effect definition from document
                            let Some(effect_def) = self.ctx.document.get_effect_definition(&effect_instance.clip_id) else {
                                println!("Effect definition not found for clip_id: {:?}", effect_instance.clip_id);
                                continue;
                            };

                            // Compile effect if needed
                            if !effect_processor.is_compiled(&effect_def.id) {
                                let success = effect_processor.compile_effect(device, effect_def);
                                if !success {
                                    eprintln!("Failed to compile effect: {}", effect_def.name);
                                    continue;
                                }
                                println!("Compiled effect: {}", effect_def.name);
                            }

                            // Create EffectInstance from ClipInstance for the processor
                            // For now, create a simple effect instance with default parameters
                            let effect_inst = lightningbeam_core::effect::EffectInstance::new(
                                effect_def,
                                effect_instance.timeline_start,
                                effect_instance.timeline_start + effect_instance.effective_duration(lightningbeam_core::effect::EFFECT_DURATION),
                            );

                            // Acquire temp buffer for effect output (HDR format)
                            let effect_output_handle = buffer_pool.acquire(device, hdr_spec);

                            if let (Some(hdr_view), Some(effect_output_view)) = (
                                &instance_resources.hdr_texture_view,
                                buffer_pool.get_view(effect_output_handle),
                            ) {
                                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("effect_encoder"),
                                });

                                // Apply effect: HDR accumulator → effect output buffer
                                let applied = effect_processor.apply_effect(
                                    device,
                                    queue,
                                    &mut encoder,
                                    effect_def,
                                    &effect_inst,
                                    hdr_view,
                                    effect_output_view,
                                    width,
                                    height,
                                    current_time,
                                );

                                if applied {
                                    queue.submit(Some(encoder.finish()));

                                    // Copy effect output back to HDR accumulator
                                    // We need to blit the effect result back to the HDR texture
                                    let mut copy_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                        label: Some("effect_copy_encoder"),
                                    });

                                    // Use compositor to copy (with opacity 1.0, replacing content)
                                    let effect_layer = lightningbeam_core::gpu::CompositorLayer::normal(
                                        effect_output_handle,
                                        rendered_layer.opacity, // Apply effect layer opacity
                                    );
                                    shared.compositor.composite(
                                        device,
                                        queue,
                                        &mut copy_encoder,
                                        &[effect_layer],
                                        &buffer_pool,
                                        hdr_view,
                                        Some([0.0, 0.0, 0.0, 0.0]), // Clear with transparent (we're replacing)
                                    );
                                    queue.submit(Some(copy_encoder.finish()));
                                } else {
                                    eprintln!("Effect {} failed to apply", effect_def.name);
                                }
                            }

                            buffer_pool.release(effect_output_handle);
                        }
                    }
                }
            }

            drop(effect_processor);

            // When editing inside a clip: dim overlay + re-render the clip at full opacity
            if let (Some(parent_layer_id), Some(instance_id)) = (self.ctx.editing_parent_layer_id, self.ctx.editing_instance_id) {
                // 1. Render dim overlay scene
                let mut dim_scene = vello::Scene::new();
                let doc_rect = vello::kurbo::Rect::new(0.0, 0.0, self.ctx.document.width, self.ctx.document.height);
                dim_scene.fill(
                    vello::peniko::Fill::NonZero,
                    camera_transform,
                    vello::peniko::Color::new([0.0, 0.0, 0.0, 0.5]),
                    None,
                    &doc_rect,
                );

                // Composite dim overlay onto HDR texture
                let dim_srgb_handle = buffer_pool.acquire(device, lightningbeam_core::gpu::BufferSpec::new(width, height, lightningbeam_core::gpu::BufferFormat::Rgba8Srgb));
                let dim_hdr_handle = buffer_pool.acquire(device, lightningbeam_core::gpu::BufferSpec::new(width, height, BufferFormat::Rgba16Float));
                if let (Some(dim_srgb_view), Some(dim_hdr_view), Some(hdr_view)) = (
                    buffer_pool.get_view(dim_srgb_handle),
                    buffer_pool.get_view(dim_hdr_handle),
                    &instance_resources.hdr_texture_view,
                ) {
                    let dim_params = vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width, height,
                        antialiasing_method: vello::AaConfig::Msaa16,
                    };
                    if let Ok(mut renderer) = shared.renderer.lock() {
                        renderer.render_to_texture(device, queue, &dim_scene, dim_srgb_view, &dim_params).ok();
                    }
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dim_srgb_to_linear") });
                    shared.srgb_to_linear.convert(device, &mut enc, dim_srgb_view, dim_hdr_view);
                    queue.submit(Some(enc.finish()));

                    let dim_layer = lightningbeam_core::gpu::CompositorLayer::normal(dim_hdr_handle, 1.0);
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("dim_composite") });
                    shared.compositor.composite(device, queue, &mut enc, &[dim_layer], &buffer_pool, hdr_view, None);
                    queue.submit(Some(enc.finish()));
                }
                buffer_pool.release(dim_srgb_handle);
                buffer_pool.release(dim_hdr_handle);

                // 2. Re-render the clip instance at full opacity
                let mut clip_scene = vello::Scene::new();
                let mut image_cache = shared.image_cache.lock().unwrap();
                lightningbeam_core::renderer::render_single_clip_instance(
                    &self.ctx.document,
                    &mut clip_scene,
                    camera_transform,
                    &parent_layer_id,
                    &instance_id,
                    &mut image_cache,
                    &shared.video_manager,
                );
                drop(image_cache);

                let clip_srgb_handle = buffer_pool.acquire(device, lightningbeam_core::gpu::BufferSpec::new(width, height, lightningbeam_core::gpu::BufferFormat::Rgba8Srgb));
                let clip_hdr_handle = buffer_pool.acquire(device, lightningbeam_core::gpu::BufferSpec::new(width, height, BufferFormat::Rgba16Float));
                if let (Some(clip_srgb_view), Some(clip_hdr_view), Some(hdr_view)) = (
                    buffer_pool.get_view(clip_srgb_handle),
                    buffer_pool.get_view(clip_hdr_handle),
                    &instance_resources.hdr_texture_view,
                ) {
                    let clip_params = vello::RenderParams {
                        base_color: vello::peniko::Color::TRANSPARENT,
                        width, height,
                        antialiasing_method: vello::AaConfig::Msaa16,
                    };
                    if let Ok(mut renderer) = shared.renderer.lock() {
                        renderer.render_to_texture(device, queue, &clip_scene, clip_srgb_view, &clip_params).ok();
                    }
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("clip_srgb_to_linear") });
                    shared.srgb_to_linear.convert(device, &mut enc, clip_srgb_view, clip_hdr_view);
                    queue.submit(Some(enc.finish()));

                    let clip_layer = lightningbeam_core::gpu::CompositorLayer::normal(clip_hdr_handle, 1.0);
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("clip_composite") });
                    shared.compositor.composite(device, queue, &mut enc, &[clip_layer], &buffer_pool, hdr_view, None);
                    queue.submit(Some(enc.finish()));
                }
                buffer_pool.release(clip_srgb_handle);
                buffer_pool.release(clip_hdr_handle);
            }

            // Advance frame counter for buffer cleanup
            buffer_pool.next_frame();
            drop(buffer_pool);

            // --- Frame timing report ---
            let _t_end = std::time::Instant::now();
            let total_ms = (_t_end - _t_prepare_start).as_secs_f64() * 1000.0;
            let removals_ms = (_t_after_removals - _t_prepare_start).as_secs_f64() * 1000.0;
            let gpu_dispatches_ms = (_t_after_gpu_dispatches - _t_after_removals).as_secs_f64() * 1000.0;
            let scene_build_ms = (_t_after_scene_build - _t_after_gpu_dispatches).as_secs_f64() * 1000.0;
            let composite_ms = (_t_end - _t_after_scene_build).as_secs_f64() * 1000.0;
            crate::debug_overlay::update_prepare_timing(
                total_ms, removals_ms, gpu_dispatches_ms, scene_build_ms, composite_ms,
            );

            // For drag preview and other overlays, we still need a scene
            // Create an empty scene - the composited result is already in hdr_texture
            vello::Scene::new()
        } else {
            // Legacy single-scene rendering
            let mut scene = vello::Scene::new();
            let mut image_cache = shared.image_cache.lock().unwrap();

            lightningbeam_core::renderer::render_document_with_transform(
                &self.ctx.document,
                &mut scene,
                camera_transform,
                &mut image_cache,
                &shared.video_manager,
            );

            // When editing inside a clip: dim overlay + re-render the clip at full opacity
            if let (Some(parent_layer_id), Some(instance_id)) = (self.ctx.editing_parent_layer_id, self.ctx.editing_instance_id) {
                // Semi-transparent dim overlay
                let doc_rect = vello::kurbo::Rect::new(0.0, 0.0, self.ctx.document.width, self.ctx.document.height);
                scene.fill(
                    vello::peniko::Fill::NonZero,
                    camera_transform,
                    vello::peniko::Color::new([0.0, 0.0, 0.0, 0.5]),
                    None,
                    &doc_rect,
                );
                // Re-render the clip instance on top
                lightningbeam_core::renderer::render_single_clip_instance(
                    &self.ctx.document,
                    &mut scene,
                    camera_transform,
                    &parent_layer_id,
                    &instance_id,
                    &mut image_cache,
                    &shared.video_manager,
                );
            }

            // Render selected DCEL from active region selection (with transform)
            if let Some(ref region_sel) = self.ctx.region_selection {
                let sel_transform = overlay_transform * region_sel.transform;
                lightningbeam_core::renderer::render_dcel(
                    &region_sel.selected_dcel,
                    &mut scene,
                    sel_transform,
                    1.0,
                    &self.ctx.document,
                    &mut image_cache,
                );
            }

            drop(image_cache);
            scene
        };

        // Render region selection fill into the overlay scene.
        // In HDR mode the main scene-building block returns an empty scene (only layer content
        // goes through the HDR pipeline), so we must add the selected-DCEL fill here so it
        // appears underneath the stipple overlay. In legacy mode the render_dcel call inside
        // the block already handled this, but running it again is harmless since `scene` would
        // be a fresh empty scene only in HDR mode.
        if USE_HDR_COMPOSITING {
            if let Some(ref region_sel) = self.ctx.region_selection {
                let sel_transform = overlay_transform * region_sel.transform;
                let mut image_cache = shared.image_cache.lock().unwrap();
                lightningbeam_core::renderer::render_dcel(
                    &region_sel.selected_dcel,
                    &mut scene,
                    sel_transform,
                    1.0,
                    &self.ctx.document,
                    &mut image_cache,
                );
            }
        }

        // Render drag preview objects with transparency
        if let (Some(delta), Some(active_layer_id)) = (self.ctx.drag_delta, self.ctx.active_layer_id) {
            if let Some(layer) = self.ctx.document.get_layer(&active_layer_id) {
                if let lightningbeam_core::layer::AnyLayer::Vector(vector_layer) = layer {
                    if let lightningbeam_core::tool::ToolState::DraggingSelection { ref original_positions, .. } = self.ctx.tool_state {
                        use vello::peniko::Color;

                        // Render each object at its preview position (original + delta)
                        for (object_id, original_pos) in original_positions {
                            // TODO: DCEL - shape drag preview disabled during migration
                            // (was: get_shape_in_keyframe for drag preview rendering)

                            // Try clip instance
                            if let Some(clip_inst) = vector_layer.clip_instances.iter().find(|ci| ci.id == *object_id) {
                                // Render clip at preview position
                                // For now, just render the bounding box outline in semi-transparent blue
                                let new_x = original_pos.x + delta.x;
                                let new_y = original_pos.y + delta.y;

                                use vello::kurbo::Stroke;
                                let clip_transform = Affine::translate((new_x, new_y));
                                let combined_transform = overlay_transform * clip_transform;

                                // Calculate clip bounds for preview
                                let clip_time = ((self.ctx.playback_time - clip_inst.timeline_start) * clip_inst.playback_speed) + clip_inst.trim_start;
                                let content_bounds = if let Some(vector_clip) = self.ctx.document.get_vector_clip(&clip_inst.clip_id) {
                                    vector_clip.calculate_content_bounds(&self.ctx.document, clip_time)
                                } else if let Some(video_clip) = self.ctx.document.get_video_clip(&clip_inst.clip_id) {
                                    use vello::kurbo::Rect as KurboRect;
                                    KurboRect::new(0.0, 0.0, video_clip.width, video_clip.height)
                                } else {
                                    continue;
                                };

                                // Draw preview outline
                                let alpha_color = Color::from_rgba8(255, 150, 100, 150); // Orange, semi-transparent
                                let stroke_width = 2.0 / self.ctx.zoom.max(0.5) as f64;
                                scene.stroke(
                                    &Stroke::new(stroke_width),
                                    combined_transform,
                                    alpha_color,
                                    None,
                                    &content_bounds,
                                );
                            }
                        }
                    }
                }
            }
        }

        // Render selection overlays (outlines, handles, marquee)
        if let Some(active_layer_id) = self.ctx.active_layer_id {
            if let Some(layer) = self.ctx.document.get_layer(&active_layer_id) {
                if let lightningbeam_core::layer::AnyLayer::Vector(vector_layer) = layer {
                    use vello::peniko::{Color, Fill};
                    use vello::kurbo::{Circle, Rect as KurboRect, Stroke};

                    let selection_color = Color::from_rgb8(0, 120, 255); // Blue
                    let stroke_width = 2.0 / self.ctx.zoom.max(0.5) as f64;

                    // 1. Draw selection stipple overlay on selected DCEL elements + clip outlines
                    // NOTE: Skip this if Transform tool is active (it has its own handles)
                    if !self.ctx.selection.is_empty() && !matches!(self.ctx.selected_tool, Tool::Transform) {
                        // Draw Flash-style stipple pattern on selected edges and faces
                        if self.ctx.selection.has_dcel_selection() {
                            if let Some(dcel) = vector_layer.dcel_at_time(self.ctx.playback_time) {
                                let stipple_brush = selection_stipple_brush();
                                // brush_transform scales the stipple so 1 pattern pixel = 1 screen pixel.
                                // The shape is in document space, transformed to screen by overlay_transform
                                // (which includes zoom). The brush tiles in document space by default,
                                // so we scale it by 1/zoom to make each 2x2 tile = 2x2 screen pixels.
                                let inv_zoom = 1.0 / self.ctx.zoom as f64;
                                let brush_xform = Some(Affine::scale(inv_zoom));

                                // Stipple selected faces
                                for &face_id in self.ctx.selection.selected_faces() {
                                    let face = dcel.face(face_id);
                                    if face.deleted || face_id.0 == 0 { continue; }
                                    let path = dcel.face_to_bezpath_with_holes(face_id);
                                    scene.fill(
                                        Fill::NonZero,
                                        overlay_transform,
                                        stipple_brush,
                                        brush_xform,
                                        &path,
                                    );
                                }

                                // Stipple selected edges
                                for &edge_id in self.ctx.selection.selected_edges() {
                                    let edge = dcel.edge(edge_id);
                                    if edge.deleted { continue; }
                                    let width = edge.stroke_style.as_ref()
                                        .map(|s| s.width)
                                        .unwrap_or(2.0);
                                    let mut path = vello::kurbo::BezPath::new();
                                    path.move_to(edge.curve.p0);
                                    path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
                                    scene.stroke(
                                        &Stroke::new(width),
                                        overlay_transform,
                                        stipple_brush,
                                        brush_xform,
                                        &path,
                                    );
                                }
                            }
                        }

                        // Also draw selection outlines for clip instances
                        for &clip_id in self.ctx.selection.clip_instances() {
                            if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == clip_id) {
                                // Skip clip instances not active at current time
                                let clip_dur = self.ctx.document.get_clip_duration(&clip_instance.clip_id).unwrap_or(0.0);
                                let instance_end = clip_instance.timeline_start + clip_instance.effective_duration(clip_dur);
                                if self.ctx.playback_time < clip_instance.timeline_start || self.ctx.playback_time >= instance_end {
                                    continue;
                                }

                                // Calculate clip-local time
                                let clip_time = ((self.ctx.playback_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

                                // Get dynamic clip bounds from content at current time
                                let bbox = if let Some(vector_clip) = self.ctx.document.get_vector_clip(&clip_instance.clip_id) {
                                    vector_clip.calculate_content_bounds(&self.ctx.document, clip_time)
                                } else if let Some(video_clip) = self.ctx.document.get_video_clip(&clip_instance.clip_id) {
                                    KurboRect::new(0.0, 0.0, video_clip.width, video_clip.height)
                                } else {
                                    continue; // Clip not found or is audio
                                };


                                // Apply clip instance transform and camera transform
                                let clip_transform = clip_instance.transform.to_affine();
                                let combined_transform = overlay_transform * clip_transform;

                                // Draw selection outline with different color for clip instances
                                let clip_selection_color = Color::from_rgb8(255, 120, 0); // Orange
                                scene.stroke(
                                    &Stroke::new(stroke_width),
                                    combined_transform,
                                    clip_selection_color,
                                    None,
                                    &bbox,
                                );

                                // Draw corner handles (4 circles at corners)
                                let handle_radius = (6.0 / self.ctx.zoom.max(0.5) as f64).max(4.0);
                                let corners = [
                                    (bbox.x0, bbox.y0),
                                    (bbox.x1, bbox.y0),
                                    (bbox.x1, bbox.y1),
                                    (bbox.x0, bbox.y1),
                                ];

                                for (x, y) in corners {
                                    let corner_circle = Circle::new((x, y), handle_radius);
                                    // Fill with orange
                                    scene.fill(
                                        Fill::NonZero,
                                        combined_transform,
                                        clip_selection_color,
                                        None,
                                        &corner_circle,
                                    );
                                    // White outline
                                    scene.stroke(
                                        &Stroke::new(1.0),
                                        combined_transform,
                                        Color::from_rgb8(255, 255, 255),
                                        None,
                                        &corner_circle,
                                    );
                                }
                            }
                        }
                    }

                    // 1a. Draw stipple overlay on region-selected DCEL
                    if let Some(ref region_sel) = self.ctx.region_selection {
                        use lightningbeam_core::dcel::FaceId as DcelFaceId;
                        let sel_dcel = &region_sel.selected_dcel;
                        let sel_transform = overlay_transform * region_sel.transform;
                        let stipple_brush = selection_stipple_brush();
                        let inv_zoom = 1.0 / self.ctx.zoom as f64;
                        let brush_xform = Some(Affine::scale(inv_zoom));

                        // Stipple faces with visible fill
                        for (i, face) in sel_dcel.faces.iter().enumerate() {
                            if face.deleted || i == 0 { continue; }
                            if face.fill_color.is_none() && face.image_fill.is_none() && face.gradient_fill.is_none() { continue; }
                            let face_id = DcelFaceId(i as u32);
                            let path = sel_dcel.face_to_bezpath_with_holes(face_id);
                            scene.fill(
                                vello::peniko::Fill::NonZero,
                                sel_transform,
                                stipple_brush,
                                brush_xform,
                                &path,
                            );
                        }

                        // Stipple edges with visible stroke
                        for edge in &sel_dcel.edges {
                            if edge.deleted { continue; }
                            if edge.stroke_style.is_none() && edge.stroke_color.is_none() { continue; }
                            let width = edge.stroke_style.as_ref()
                                .map(|s| s.width)
                                .unwrap_or(2.0);
                            let mut path = vello::kurbo::BezPath::new();
                            path.move_to(edge.curve.p0);
                            path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
                            scene.stroke(
                                &vello::kurbo::Stroke::new(width),
                                sel_transform,
                                stipple_brush,
                                brush_xform,
                                &path,
                            );
                        }
                    }

                    // 1b. Draw stipple hover highlight on the curve under the mouse
                    // During active curve editing, lock highlight to the edited curve
                    if matches!(self.ctx.selected_tool, Tool::Select | Tool::BezierEdit) {
                        use lightningbeam_core::tool::ToolState;

                        // Determine which edge to highlight: active edit takes priority over hover
                        let highlight_edge = match &self.ctx.tool_state {
                            ToolState::EditingCurve { edge_id, .. }
                            | ToolState::PendingCurveInteraction { edge_id, .. } => {
                                Some(*edge_id)
                            }
                            _ => {
                                // Fall back to hover hit test
                                self.ctx.mouse_world_pos.and_then(|mouse_pos| {
                                    use lightningbeam_core::hit_test::{hit_test_vector_editing, EditingHitTolerance, VectorEditHit};
                                    let is_bezier = matches!(self.ctx.selected_tool, Tool::BezierEdit);
                                    let tolerance = EditingHitTolerance::scaled_by_zoom(self.ctx.zoom as f64);
                                    let hit = hit_test_vector_editing(
                                        vector_layer,
                                        self.ctx.playback_time,
                                        mouse_pos,
                                        &tolerance,
                                        Affine::IDENTITY,
                                        is_bezier,
                                    );
                                    match hit {
                                        Some(VectorEditHit::Curve { edge_id, .. }) => Some(edge_id),
                                        _ => None,
                                    }
                                })
                            }
                        };

                        if let Some(edge_id) = highlight_edge {
                            if let Some(dcel) = vector_layer.dcel_at_time(self.ctx.playback_time) {
                                let edge = dcel.edge(edge_id);
                                if !edge.deleted {
                                    let stipple_brush = selection_stipple_brush();
                                    let inv_zoom = 1.0 / self.ctx.zoom as f64;
                                    let brush_xform = Some(Affine::scale(inv_zoom));
                                    let width = edge.stroke_style.as_ref()
                                        .map(|s| s.width + 4.0)
                                        .unwrap_or(3.0)
                                        .max(3.0);
                                    let mut path = vello::kurbo::BezPath::new();
                                    path.move_to(edge.curve.p0);
                                    path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
                                    scene.stroke(
                                        &Stroke::new(width),
                                        overlay_transform,
                                        stipple_brush,
                                        brush_xform,
                                        &path,
                                    );
                                }
                            }
                        }
                    }

                    // 2. Draw marquee selection rectangle
                    if let lightningbeam_core::tool::ToolState::MarqueeSelecting { ref start, ref current } = self.ctx.tool_state {
                        let marquee_rect = KurboRect::new(
                            start.x.min(current.x),
                            start.y.min(current.y),
                            start.x.max(current.x),
                            start.y.max(current.y),
                        );

                        // Semi-transparent fill
                        let marquee_fill = Color::from_rgba8(0, 120, 255, 100);
                        scene.fill(
                            Fill::NonZero,
                            overlay_transform,
                            marquee_fill,
                            None,
                            &marquee_rect,
                        );

                        // Border stroke
                        scene.stroke(
                            &Stroke::new(1.0),
                            overlay_transform,
                            selection_color,
                            None,
                            &marquee_rect,
                        );
                    }

                    // 2b. Draw region selection overlay (rect or lasso)
                    match &self.ctx.tool_state {
                        lightningbeam_core::tool::ToolState::RegionSelectingRect { start, current } => {
                            let region_rect = KurboRect::new(
                                start.x.min(current.x),
                                start.y.min(current.y),
                                start.x.max(current.x),
                                start.y.max(current.y),
                            );
                            // Semi-transparent orange fill
                            let region_fill = Color::from_rgba8(255, 150, 0, 60);
                            scene.fill(
                                Fill::NonZero,
                                overlay_transform,
                                region_fill,
                                None,
                                &region_rect,
                            );
                            // Dashed-like border (solid for now)
                            let region_stroke_color = Color::from_rgba8(255, 150, 0, 200);
                            scene.stroke(
                                &Stroke::new(1.5),
                                overlay_transform,
                                region_stroke_color,
                                None,
                                &region_rect,
                            );
                        }
                        lightningbeam_core::tool::ToolState::RegionSelectingLasso { points } => {
                            if points.len() >= 2 {
                                // Build polyline path
                                let mut lasso_path = vello::kurbo::BezPath::new();
                                lasso_path.move_to(points[0]);
                                for &p in &points[1..] {
                                    lasso_path.line_to(p);
                                }
                                // Close back to start
                                lasso_path.close_path();

                                // Semi-transparent orange fill
                                let region_fill = Color::from_rgba8(255, 150, 0, 60);
                                scene.fill(
                                    Fill::NonZero,
                                    overlay_transform,
                                    region_fill,
                                    None,
                                    &lasso_path,
                                );
                                // Border
                                let region_stroke_color = Color::from_rgba8(255, 150, 0, 200);
                                scene.stroke(
                                    &Stroke::new(1.5),
                                    overlay_transform,
                                    region_stroke_color,
                                    None,
                                    &lasso_path,
                                );
                            }
                        }
                        _ => {}
                    }


                    // 8. Vector editing preview: DCEL edits are applied live to the document,
                    // so the normal DCEL render path draws the current state. No separate
                    // preview rendering is needed.

                    // 6. Draw transform tool handles (when Transform tool is active)
                    use lightningbeam_core::tool::Tool;
                    let should_draw_transform_handles = matches!(self.ctx.selected_tool, Tool::Transform) && !self.ctx.selection.is_empty();
                    if should_draw_transform_handles {
                        // For single object: use object-aligned (rotated) bounding box
                        // For multiple objects: use axis-aligned bounding box (simpler for now)

                        let total_selected = self.ctx.selection.clip_instances().len();
                        if self.ctx.selection.has_dcel_selection() {
                            // DCEL selection: compute bbox from selected vertices
                            if let Some(dcel) = vector_layer.dcel_at_time(self.ctx.playback_time) {
                                let mut min_x = f64::INFINITY;
                                let mut min_y = f64::INFINITY;
                                let mut max_x = f64::NEG_INFINITY;
                                let mut max_y = f64::NEG_INFINITY;
                                let mut found_any = false;

                                for &vid in self.ctx.selection.selected_vertices() {
                                    let v = dcel.vertex(vid);
                                    if v.deleted { continue; }
                                    min_x = min_x.min(v.position.x);
                                    min_y = min_y.min(v.position.y);
                                    max_x = max_x.max(v.position.x);
                                    max_y = max_y.max(v.position.y);
                                    found_any = true;
                                }

                                if found_any {
                                    let bbox = KurboRect::new(min_x, min_y, max_x, max_y);
                                    let handle_size = (8.0 / self.ctx.zoom.max(0.5) as f64).max(6.0);
                                    let handle_color = Color::from_rgb8(0, 120, 255);
                                    let rotation_handle_offset = 20.0 / self.ctx.zoom.max(0.5) as f64;

                                    scene.stroke(&Stroke::new(stroke_width), overlay_transform, handle_color, None, &bbox);

                                    let corners = [
                                        vello::kurbo::Point::new(bbox.x0, bbox.y0),
                                        vello::kurbo::Point::new(bbox.x1, bbox.y0),
                                        vello::kurbo::Point::new(bbox.x1, bbox.y1),
                                        vello::kurbo::Point::new(bbox.x0, bbox.y1),
                                    ];

                                    for corner in &corners {
                                        let handle_rect = KurboRect::new(
                                            corner.x - handle_size / 2.0, corner.y - handle_size / 2.0,
                                            corner.x + handle_size / 2.0, corner.y + handle_size / 2.0,
                                        );
                                        scene.fill(Fill::NonZero, overlay_transform, handle_color, None, &handle_rect);
                                        scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &handle_rect);
                                    }

                                    let edges = [
                                        vello::kurbo::Point::new(bbox.center().x, bbox.y0),
                                        vello::kurbo::Point::new(bbox.x1, bbox.center().y),
                                        vello::kurbo::Point::new(bbox.center().x, bbox.y1),
                                        vello::kurbo::Point::new(bbox.x0, bbox.center().y),
                                    ];

                                    for edge in &edges {
                                        let edge_circle = Circle::new(*edge, handle_size / 2.0);
                                        scene.fill(Fill::NonZero, overlay_transform, handle_color, None, &edge_circle);
                                        scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &edge_circle);
                                    }

                                    let rotation_handle_pos = vello::kurbo::Point::new(bbox.center().x, bbox.y0 - rotation_handle_offset);
                                    let rotation_circle = Circle::new(rotation_handle_pos, handle_size / 2.0);
                                    scene.fill(Fill::NonZero, overlay_transform, Color::from_rgb8(50, 200, 50), None, &rotation_circle);
                                    scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &rotation_circle);

                                    let line_path = {
                                        let mut path = vello::kurbo::BezPath::new();
                                        path.move_to(rotation_handle_pos);
                                        path.line_to(vello::kurbo::Point::new(bbox.center().x, bbox.y0));
                                        path
                                    };
                                    scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(50, 200, 50), None, &line_path);
                                }
                            }
                        } else if total_selected == 1 {
                            // Single clip instance - draw rotated bounding box
                            let object_id = *self.ctx.selection.clip_instances().iter().next().unwrap();

                            // TODO: DCEL - single-object transform handles disabled during migration
                            // (was: get_shape_in_keyframe for rotated bbox + handle drawing)
                            let _ = object_id;
                        } else {
                            // Multiple objects - use axis-aligned bbox (existing code)
                            let combined_bbox: Option<KurboRect> = None;

                            // TODO: DCEL - multi-object shape bbox calculation disabled during migration
                            // (was: iterate shape_instances, get_shape_in_keyframe, compute combined bbox)

                            if let Some(bbox) = combined_bbox {
                                let handle_size = (8.0 / self.ctx.zoom.max(0.5) as f64).max(6.0);
                                let handle_color = Color::from_rgb8(0, 120, 255);
                                let rotation_handle_offset = 20.0 / self.ctx.zoom.max(0.5) as f64;

                                scene.stroke(&Stroke::new(stroke_width), overlay_transform, handle_color, None, &bbox);

                                let corners = [
                                    vello::kurbo::Point::new(bbox.x0, bbox.y0),
                                    vello::kurbo::Point::new(bbox.x1, bbox.y0),
                                    vello::kurbo::Point::new(bbox.x1, bbox.y1),
                                    vello::kurbo::Point::new(bbox.x0, bbox.y1),
                                ];

                                for corner in &corners {
                                    let handle_rect = KurboRect::new(
                                        corner.x - handle_size / 2.0, corner.y - handle_size / 2.0,
                                        corner.x + handle_size / 2.0, corner.y + handle_size / 2.0,
                                    );
                                    scene.fill(Fill::NonZero, overlay_transform, handle_color, None, &handle_rect);
                                    scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &handle_rect);
                                }

                                let edges = [
                                    vello::kurbo::Point::new(bbox.center().x, bbox.y0),
                                    vello::kurbo::Point::new(bbox.x1, bbox.center().y),
                                    vello::kurbo::Point::new(bbox.center().x, bbox.y1),
                                    vello::kurbo::Point::new(bbox.x0, bbox.center().y),
                                ];

                                for edge in &edges {
                                    let edge_circle = Circle::new(*edge, handle_size / 2.0);
                                    scene.fill(Fill::NonZero, overlay_transform, handle_color, None, &edge_circle);
                                    scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &edge_circle);
                                }

                                let rotation_handle_pos = vello::kurbo::Point::new(bbox.center().x, bbox.y0 - rotation_handle_offset);
                                let rotation_circle = Circle::new(rotation_handle_pos, handle_size / 2.0);
                                scene.fill(Fill::NonZero, overlay_transform, Color::from_rgb8(50, 200, 50), None, &rotation_circle);
                                scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(255, 255, 255), None, &rotation_circle);

                                let line_path = {
                                    let mut path = vello::kurbo::BezPath::new();
                                    path.move_to(rotation_handle_pos);
                                    path.line_to(vello::kurbo::Point::new(bbox.center().x, bbox.y0));
                                    path
                                };
                                scene.stroke(&Stroke::new(1.0), overlay_transform, Color::from_rgb8(50, 200, 50), None, &line_path);
                            }
                        }
                    }
                } else if let lightningbeam_core::layer::AnyLayer::Video(video_layer) = layer {
                    // Draw transform handles for video layers when Transform tool is active
                    use lightningbeam_core::tool::Tool;
                    if matches!(self.ctx.selected_tool, Tool::Transform) {
                        use vello::peniko::{Color, Fill};
                        use vello::kurbo::{Circle, Rect as KurboRect, Stroke};

                        let stroke_width = 2.0 / self.ctx.zoom.max(0.5) as f64;

                        // Find visible clip instance at current playback time
                        let playback_time = self.ctx.playback_time;

                        // Find clip instance visible at playback time
                        let visible_clip = video_layer.clip_instances.iter().find(|inst| {
                            let clip_duration = self.ctx.document.get_clip_duration(&inst.clip_id).unwrap_or(0.0);
                            let effective_duration = inst.effective_duration(clip_duration);
                            playback_time >= inst.timeline_start && playback_time < inst.timeline_start + effective_duration
                        });

                    if let Some(clip_inst) = visible_clip {
                        // Get video clip dimensions
                        if let Some(video_clip) = self.ctx.document.get_video_clip(&clip_inst.clip_id) {
                            let handle_size = (8.0 / self.ctx.zoom.max(0.5) as f64).max(6.0);
                            let handle_color = Color::from_rgb8(0, 120, 255); // Blue
                            let rotation_handle_offset = 20.0 / self.ctx.zoom.max(0.5) as f64;

                            // Video clip local bounding box (0,0 to width,height)
                            let local_bbox = KurboRect::new(0.0, 0.0, video_clip.width, video_clip.height);

                            // Calculate the 4 corners in local space
                            let local_corners = [
                                vello::kurbo::Point::new(local_bbox.x0, local_bbox.y0), // Top-left
                                vello::kurbo::Point::new(local_bbox.x1, local_bbox.y0), // Top-right
                                vello::kurbo::Point::new(local_bbox.x1, local_bbox.y1), // Bottom-right
                                vello::kurbo::Point::new(local_bbox.x0, local_bbox.y1), // Bottom-left
                            ];

                            // Build skew transforms around center
                            let center_x = (local_bbox.x0 + local_bbox.x1) / 2.0;
                            let center_y = (local_bbox.y0 + local_bbox.y1) / 2.0;

                            let skew_transform = if clip_inst.transform.skew_x != 0.0 || clip_inst.transform.skew_y != 0.0 {
                                let skew_x_affine = if clip_inst.transform.skew_x != 0.0 {
                                    let tan_skew = clip_inst.transform.skew_x.to_radians().tan();
                                    Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
                                } else {
                                    Affine::IDENTITY
                                };

                                let skew_y_affine = if clip_inst.transform.skew_y != 0.0 {
                                    let tan_skew = clip_inst.transform.skew_y.to_radians().tan();
                                    Affine::new([1.0, tan_skew, 0.0, 1.0, 0.0, 0.0])
                                } else {
                                    Affine::IDENTITY
                                };

                                Affine::translate((center_x, center_y))
                                    * skew_x_affine
                                    * skew_y_affine
                                    * Affine::translate((-center_x, -center_y))
                            } else {
                                Affine::IDENTITY
                            };

                            // Transform to world space
                            let obj_transform = Affine::translate((clip_inst.transform.x, clip_inst.transform.y))
                                * Affine::rotate(clip_inst.transform.rotation.to_radians())
                                * Affine::scale_non_uniform(clip_inst.transform.scale_x, clip_inst.transform.scale_y)
                                * skew_transform;

                            let world_corners: Vec<vello::kurbo::Point> = local_corners
                                .iter()
                                .map(|&p| obj_transform * p)
                                .collect();

                            // Draw rotated bounding box outline
                            let bbox_path = {
                                let mut path = vello::kurbo::BezPath::new();
                                path.move_to(world_corners[0]);
                                path.line_to(world_corners[1]);
                                path.line_to(world_corners[2]);
                                path.line_to(world_corners[3]);
                                path.close_path();
                                path
                            };

                            scene.stroke(
                                &Stroke::new(stroke_width),
                                overlay_transform,
                                handle_color,
                                None,
                                &bbox_path,
                            );

                            // Draw 4 corner handles (squares)
                            for corner in &world_corners {
                                let handle_rect = KurboRect::new(
                                    corner.x - handle_size / 2.0,
                                    corner.y - handle_size / 2.0,
                                    corner.x + handle_size / 2.0,
                                    corner.y + handle_size / 2.0,
                                );

                                // Fill
                                scene.fill(
                                    Fill::NonZero,
                                    overlay_transform,
                                    handle_color,
                                    None,
                                    &handle_rect,
                                );

                                // White outline
                                scene.stroke(
                                    &Stroke::new(1.0),
                                    overlay_transform,
                                    Color::from_rgb8(255, 255, 255),
                                    None,
                                    &handle_rect,
                                );
                            }

                            // Draw 4 edge handles (circles at midpoints)
                            let edge_midpoints = [
                                vello::kurbo::Point::new((world_corners[0].x + world_corners[1].x) / 2.0, (world_corners[0].y + world_corners[1].y) / 2.0), // Top
                                vello::kurbo::Point::new((world_corners[1].x + world_corners[2].x) / 2.0, (world_corners[1].y + world_corners[2].y) / 2.0), // Right
                                vello::kurbo::Point::new((world_corners[2].x + world_corners[3].x) / 2.0, (world_corners[2].y + world_corners[3].y) / 2.0), // Bottom
                                vello::kurbo::Point::new((world_corners[3].x + world_corners[0].x) / 2.0, (world_corners[3].y + world_corners[0].y) / 2.0), // Left
                            ];

                            for edge in &edge_midpoints {
                                let edge_circle = Circle::new(*edge, handle_size / 2.0);

                                // Fill
                                scene.fill(
                                    Fill::NonZero,
                                    overlay_transform,
                                    handle_color,
                                    None,
                                    &edge_circle,
                                );

                                // White outline
                                scene.stroke(
                                    &Stroke::new(1.0),
                                    overlay_transform,
                                    Color::from_rgb8(255, 255, 255),
                                    None,
                                    &edge_circle,
                                );
                            }

                            // Draw rotation handle (circle above top edge center)
                            let top_center = edge_midpoints[0];
                            let rotation_rad = clip_inst.transform.rotation.to_radians();
                            let cos_r = rotation_rad.cos();
                            let sin_r = rotation_rad.sin();
                            let offset_x = -(-rotation_handle_offset) * sin_r;
                            let offset_y = -rotation_handle_offset * cos_r;
                            let rotation_handle_pos = vello::kurbo::Point::new(
                                top_center.x + offset_x,
                                top_center.y + offset_y,
                            );
                            let rotation_circle = Circle::new(rotation_handle_pos, handle_size / 2.0);

                            // Fill with different color (green)
                            scene.fill(
                                Fill::NonZero,
                                overlay_transform,
                                Color::from_rgb8(50, 200, 50),
                                None,
                                &rotation_circle,
                            );

                            // White outline
                            scene.stroke(
                                &Stroke::new(1.0),
                                overlay_transform,
                                Color::from_rgb8(255, 255, 255),
                                None,
                                &rotation_circle,
                            );

                            // Draw line connecting rotation handle to bbox
                            let line_path = {
                                let mut path = vello::kurbo::BezPath::new();
                                path.move_to(rotation_handle_pos);
                                path.line_to(top_center);
                                path
                            };

                            scene.stroke(
                                &Stroke::new(1.0),
                                overlay_transform,
                                Color::from_rgb8(50, 200, 50),
                                None,
                                &line_path,
                            );
                        }
                    }
                    }
                }
            }
        }

        // Shape / path creation previews — drawn regardless of layer type so raster layers
        // also see the live outline during drag.
        {
            use vello::peniko::{Color, Fill};
            use vello::kurbo::{Rect as KurboRect, Stroke};

            // Rectangle preview
            if let lightningbeam_core::tool::ToolState::CreatingRectangle { ref start_point, ref current_point, centered, constrain_square, .. } = self.ctx.tool_state {
                use vello::kurbo::Point;
                let (width, height, position) = if centered {
                    let dx = current_point.x - start_point.x;
                    let dy = current_point.y - start_point.y;
                    let (w, h) = if constrain_square {
                        let size = dx.abs().max(dy.abs()) * 2.0;
                        (size, size)
                    } else {
                        (dx.abs() * 2.0, dy.abs() * 2.0)
                    };
                    let pos = Point::new(start_point.x - w / 2.0, start_point.y - h / 2.0);
                    (w, h, pos)
                } else {
                    let mut min_x = start_point.x.min(current_point.x);
                    let mut min_y = start_point.y.min(current_point.y);
                    let mut max_x = start_point.x.max(current_point.x);
                    let mut max_y = start_point.y.max(current_point.y);
                    if constrain_square {
                        let size = (max_x - min_x).max(max_y - min_y);
                        if current_point.x > start_point.x { max_x = min_x + size; } else { min_x = max_x - size; }
                        if current_point.y > start_point.y { max_y = min_y + size; } else { min_y = max_y - size; }
                    }
                    (max_x - min_x, max_y - min_y, Point::new(min_x, min_y))
                };
                if width > 0.0 && height > 0.0 {
                    let rect = KurboRect::new(0.0, 0.0, width, height);
                    let preview_transform = overlay_transform * Affine::translate((position.x, position.y));
                    if self.ctx.fill_enabled {
                        let fc = self.ctx.fill_color;
                        scene.fill(Fill::NonZero, preview_transform, Color::from_rgba8(fc.r(), fc.g(), fc.b(), fc.a()), None, &rect);
                    }
                    let sc = self.ctx.stroke_color;
                    scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, Color::from_rgba8(sc.r(), sc.g(), sc.b(), sc.a()), None, &rect);
                }
            }

            // Ellipse preview
            if let lightningbeam_core::tool::ToolState::CreatingEllipse { ref start_point, ref current_point, corner_mode, constrain_circle, .. } = self.ctx.tool_state {
                use vello::kurbo::{Point, Circle as KurboCircle, Ellipse};
                let (rx, ry, position) = if corner_mode {
                    let min_x = start_point.x.min(current_point.x);
                    let min_y = start_point.y.min(current_point.y);
                    let max_x = start_point.x.max(current_point.x);
                    let max_y = start_point.y.max(current_point.y);
                    let (rx, ry) = if constrain_circle {
                        let r = (max_x - min_x).max(max_y - min_y) / 2.0;
                        (r, r)
                    } else { ((max_x - min_x) / 2.0, (max_y - min_y) / 2.0) };
                    (rx, ry, Point::new(min_x + rx, min_y + ry))
                } else {
                    let dx = (current_point.x - start_point.x).abs();
                    let dy = (current_point.y - start_point.y).abs();
                    let (rx, ry) = if constrain_circle {
                        let r = (dx * dx + dy * dy).sqrt(); (r, r)
                    } else { (dx, dy) };
                    (rx, ry, *start_point)
                };
                if rx > 0.0 && ry > 0.0 {
                    let preview_transform = overlay_transform * Affine::translate((position.x, position.y));
                    let fc = self.ctx.fill_color;
                    let fill_color = Color::from_rgba8(fc.r(), fc.g(), fc.b(), fc.a());
                    let sc = self.ctx.stroke_color;
                    let stroke_color = Color::from_rgba8(sc.r(), sc.g(), sc.b(), sc.a());
                    if rx == ry {
                        let circle = KurboCircle::new((0.0, 0.0), rx);
                        if self.ctx.fill_enabled { scene.fill(Fill::NonZero, preview_transform, fill_color, None, &circle); }
                        scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, stroke_color, None, &circle);
                    } else {
                        let ellipse = Ellipse::new((0.0, 0.0), (rx, ry), 0.0);
                        if self.ctx.fill_enabled { scene.fill(Fill::NonZero, preview_transform, fill_color, None, &ellipse); }
                        scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, stroke_color, None, &ellipse);
                    }
                }
            }

            // Line preview
            if let lightningbeam_core::tool::ToolState::CreatingLine { ref start_point, ref current_point, .. } = self.ctx.tool_state {
                use vello::kurbo::Line;
                let dx = current_point.x - start_point.x;
                let dy = current_point.y - start_point.y;
                if (dx * dx + dy * dy).sqrt() > 0.0 {
                    let sc = self.ctx.stroke_color;
                    let line = Line::new(*start_point, *current_point);
                    scene.stroke(&Stroke::new(2.0), overlay_transform, Color::from_rgba8(sc.r(), sc.g(), sc.b(), sc.a()), None, &line);
                }
            }

            // Polygon preview
            if let lightningbeam_core::tool::ToolState::CreatingPolygon { ref center, ref current_point, num_sides, .. } = self.ctx.tool_state {
                use vello::kurbo::{BezPath, Point};
                use std::f64::consts::PI;
                let dx = current_point.x - center.x;
                let dy = current_point.y - center.y;
                let radius = (dx * dx + dy * dy).sqrt();
                if radius > 5.0 && num_sides >= 3 {
                    let preview_transform = overlay_transform * Affine::translate((center.x, center.y));
                    let angle_step = 2.0 * PI / num_sides as f64;
                    let start_angle = -PI / 2.0;
                    let mut path = BezPath::new();
                    path.move_to(Point::new(radius * start_angle.cos(), radius * start_angle.sin()));
                    for i in 1..num_sides {
                        let angle = start_angle + angle_step * i as f64;
                        path.line_to(Point::new(radius * angle.cos(), radius * angle.sin()));
                    }
                    path.close_path();
                    if self.ctx.fill_enabled {
                        let fc = self.ctx.fill_color;
                        scene.fill(Fill::NonZero, preview_transform, Color::from_rgba8(fc.r(), fc.g(), fc.b(), fc.a()), None, &path);
                    }
                    let sc = self.ctx.stroke_color;
                    scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, Color::from_rgba8(sc.r(), sc.g(), sc.b(), sc.a()), None, &path);
                }
            }

            // Freehand path preview
            if let lightningbeam_core::tool::ToolState::DrawingPath { ref points, .. } = self.ctx.tool_state {
                use vello::kurbo::BezPath;
                if points.len() >= 2 {
                    let mut preview_path = BezPath::new();
                    preview_path.move_to(points[0]);
                    for point in &points[1..] { preview_path.line_to(*point); }
                    if self.ctx.fill_enabled {
                        let fc = self.ctx.fill_color;
                        scene.fill(Fill::NonZero, overlay_transform, Color::from_rgba8(fc.r(), fc.g(), fc.b(), fc.a()), None, &preview_path);
                    }
                    let sc = self.ctx.stroke_color;
                    scene.stroke(&Stroke::new(self.ctx.stroke_width), overlay_transform, Color::from_rgba8(sc.r(), sc.g(), sc.b(), sc.a()), None, &preview_path);
                }
            }
        }

        // Render scene to texture using shared renderer
        if let Some(texture_view) = &instance_resources.texture_view {
            if USE_HDR_COMPOSITING {
                // HDR mode: First render overlays to HDR texture, then blit to output

                // Step 1: Render overlay scene (selection handles, drag previews, etc.) to HDR texture
                // The overlay scene was built above with all the UI elements
                if let Some(hdr_view) = &instance_resources.hdr_texture_view {
                    let mut buffer_pool = shared.buffer_pool.lock().unwrap();
                    let overlay_srgb_spec = lightningbeam_core::gpu::BufferSpec::new(
                        width,
                        height,
                        lightningbeam_core::gpu::BufferFormat::Rgba8Srgb,
                    );
                    let overlay_hdr_spec = lightningbeam_core::gpu::BufferSpec::new(
                        width,
                        height,
                        lightningbeam_core::gpu::BufferFormat::Rgba16Float,
                    );
                    let overlay_srgb_handle = buffer_pool.acquire(device, overlay_srgb_spec);
                    let overlay_hdr_handle = buffer_pool.acquire(device, overlay_hdr_spec);

                    if let (Some(overlay_srgb_view), Some(overlay_hdr_view)) = (
                        buffer_pool.get_view(overlay_srgb_handle),
                        buffer_pool.get_view(overlay_hdr_handle),
                    ) {
                        // Render overlay scene to sRGB buffer
                        let overlay_params = vello::RenderParams {
                            base_color: vello::peniko::Color::TRANSPARENT,
                            width,
                            height,
                            antialiasing_method: vello::AaConfig::Msaa16,
                        };

                        if let Ok(mut renderer) = shared.renderer.lock() {
                            renderer.render_to_texture(device, queue, &scene, overlay_srgb_view, &overlay_params).ok();
                        }

                        // Convert sRGB to linear HDR (same as main document layers)
                        let mut convert_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("overlay_srgb_to_linear_encoder"),
                        });
                        shared.srgb_to_linear.convert(device, &mut convert_encoder, overlay_srgb_view, overlay_hdr_view);
                        queue.submit(Some(convert_encoder.finish()));

                        // Composite overlay onto HDR texture
                        let overlay_layer = lightningbeam_core::gpu::CompositorLayer::normal(overlay_hdr_handle, 1.0);
                        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("overlay_composite_encoder"),
                        });
                        shared.compositor.composite(
                            device,
                            queue,
                            &mut encoder,
                            &[overlay_layer],
                            &buffer_pool,
                            hdr_view,
                            None, // Don't clear - blend onto existing content
                        );
                        queue.submit(Some(encoder.finish()));
                    }

                    buffer_pool.release(overlay_srgb_handle);
                    buffer_pool.release(overlay_hdr_handle);
                    drop(buffer_pool);
                }

                // Step 2: Blit HDR texture to output with linear→sRGB conversion
                if let Some(hdr_bind_group) = &instance_resources.hdr_blit_bind_group {
                    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("hdr_to_srgb_encoder"),
                    });

                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("hdr_to_srgb_pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: texture_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                    store: wgpu::StoreOp::Store,
                                },
                                depth_slice: None,
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                        });

                        render_pass.set_pipeline(&shared.hdr_blit_pipeline);
                        render_pass.set_bind_group(0, hdr_bind_group, &[]);
                        render_pass.draw(0..3, 0..1); // Full-screen triangle (3 vertices)
                    }

                    queue.submit(Some(encoder.finish()));
                }
            } else {
                // Legacy mode: Direct single-scene rendering
                let render_params = vello::RenderParams {
                    base_color: vello::peniko::Color::from_rgb8(45, 45, 48), // Dark background
                    width,
                    height,
                    antialiasing_method: vello::AaConfig::Msaa16,
                };

                if let Ok(mut renderer) = shared.renderer.lock() {
                    renderer
                        .render_to_texture(device, queue, &scene, texture_view, &render_params)
                        .ok();
                }
            }
        }

        // Handle eyedropper pixel sampling if requested
        if let Some((screen_pos, color_mode)) = self.ctx.eyedropper_request {
            if let Some(texture) = &instance_resources.texture {
                // Convert screen position to texture coordinates
                let tex_x = ((screen_pos.x - self.ctx.rect.min.x).max(0.0).min(self.ctx.rect.width())) as u32;
                let tex_y = ((screen_pos.y - self.ctx.rect.min.y).max(0.0).min(self.ctx.rect.height())) as u32;

                // Clamp to texture bounds
                if tex_x < width && tex_y < height {
                    // Create a staging buffer to read back the pixel
                    let _bytes_per_pixel = 4; // RGBA8
                    // Align bytes_per_row to 256 (wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    let bytes_per_row_alignment = 256u32;
                    let bytes_per_row = bytes_per_row_alignment; // Single pixel, use minimum alignment
                    let buffer_size = bytes_per_row as u64;

                    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("eyedropper_staging_buffer"),
                        size: buffer_size,
                        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });

                    // Create a command encoder for the copy operation
                    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("eyedropper_copy_encoder"),
                    });

                    // Copy the pixel from texture to staging buffer
                    encoder.copy_texture_to_buffer(
                        wgpu::TexelCopyTextureInfo {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x: tex_x, y: tex_y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyBufferInfo {
                            buffer: &staging_buffer,
                            layout: wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(bytes_per_row),
                                rows_per_image: Some(1),
                            },
                        },
                        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    );

                    // Submit the copy command
                    queue.submit(Some(encoder.finish()));

                    // Map the buffer and read the pixel (synchronous for simplicity)
                    let buffer_slice = staging_buffer.slice(..);
                    let (sender, receiver) = std::sync::mpsc::channel();
                    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
                        sender.send(result).ok();
                    });

                    // Poll the device to complete the mapping
                    let _ = device.poll(wgpu::PollType::wait_indefinitely());

                    // Read the pixel data
                    if receiver.recv().is_ok() {
                        let data = buffer_slice.get_mapped_range();
                        if data.len() >= 4 {
                            let r = data[0];
                            let g = data[1];
                            let b = data[2];
                            let a = data[3];

                            let sampled_color = egui::Color32::from_rgba_unmultiplied(r, g, b, a);

                            // Store the result in the global eyedropper results
                            if let Ok(mut results) = EYEDROPPER_RESULTS
                                .get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
                                .lock() {
                                results.insert(self.ctx.instance_id, (sampled_color, color_mode));
                            }
                        }
                    }

                    // Unmap the buffer
                    let _ = buffer_slice;
                    staging_buffer.unmap();
                }
            }
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        // Get Vello resources map
        let map: &VelloResourcesMap = match resources.get() {
            Some(m) => m,
            None => return, // Resources not initialized yet
        };

        // Get shared resources
        let shared = match &map.shared {
            Some(s) => s,
            None => return, // Shared resources not initialized yet
        };

        // Get instance resources
        let instance_resources = match map.instances.get(&self.ctx.instance_id) {
            Some(r) => r,
            None => return, // Instance not initialized yet
        };

        // Check if we have a bind group (texture ready)
        let bind_group = match &instance_resources.blit_bind_group {
            Some(bg) => bg,
            None => return, // Texture not ready yet
        };

        // Render fullscreen quad with our texture (using shared pipeline)
        render_pass.set_pipeline(&shared.blit_pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..4, 0..1); // Triangle strip: 4 vertices
    }
}

pub struct StagePane {
    // Camera state
    pan_offset: egui::Vec2,
    zoom: f32,
    // Whether the initial view has been centered (on first render with a known rect)
    needs_initial_center: bool,
    // Interaction state
    is_panning: bool,
    last_pan_pos: Option<egui::Pos2>,
    // Unique ID for this stage instance (for Vello resources)
    instance_id: u64,
    // Eyedropper state
    pending_eyedropper_sample: Option<(egui::Pos2, super::ColorMode)>,
    // Last known viewport rect (for zoom-to-fit calculation)
    last_viewport_rect: Option<egui::Rect>,
    // Vector editing cache
    dcel_editing_cache: Option<DcelEditingCache>,
    // Current snap result (for visual feedback rendering)
    current_snap: Option<lightningbeam_core::snap::SnapResult>,
    // Raster stroke in progress: (layer_id, time, brush_state, buffer_before)
    raster_stroke_state: Option<(uuid::Uuid, f64, lightningbeam_core::brush_engine::StrokeState, Vec<u8>)>,
    // Last raster stroke point (for incremental segment painting)
    raster_last_point: Option<lightningbeam_core::raster_layer::StrokePoint>,
    /// GPU dabs computed during this frame's drag event — consumed by prepare().
    pending_raster_dabs: Option<PendingRasterDabs>,
    /// Undo snapshot info captured at mouse-down; claimed when readback completes.
    /// (layer_id, time, canvas_w, canvas_h, buffer_before)
    pending_undo_before: Option<(uuid::Uuid, f64, u32, u32, Vec<u8>)>,
    /// The (layer_id, keyframe_id) of the raster layer whose GPU canvas is live.
    /// Set on mouse-down, cleared when the readback result is consumed.
    /// Used every frame to blit the GPU canvas instead of the stale Vello scene.
    painting_canvas: Option<(uuid::Uuid, uuid::Uuid)>,
    /// Keyframe UUID whose GPU canvas should be removed at the start of the next
    /// prepare() call.  Set by render_content after consuming the readback result
    /// and updating raw_pixels, so the canvas lives one extra composite frame to
    /// avoid a flash of the stale Vello scene.
    pending_canvas_removal: Option<uuid::Uuid>,
    /// Selection outline saved at stroke mouse-down for post-readback pixel masking.
    /// Pixels outside the selection are restored from `buffer_before` so strokes
    /// only affect the area inside the selection outline.
    stroke_clip_selection: Option<lightningbeam_core::selection::RasterSelection>,
    /// True while the current stroke is being painted onto the float buffer (B)
    /// rather than the layer canvas (A).
    painting_float: bool,
    /// Timestamp (ui time in seconds) of the last `compute_dabs` call for this stroke.
    /// Used to compute `dt` for the unified distance+time dab accumulator.
    raster_last_compute_time: f64,
    /// Clone stamp: (source_world - drag_start_world) computed at stroke start.
    /// Constant for the entire stroke; cleared when the stroke ends.
    clone_stroke_offset: Option<(f32, f32)>,
    /// Live state for the raster transform tool (scale/rotate/move float).
    raster_transform_state: Option<RasterTransformState>,
    /// GPU transform work to dispatch in prepare().
    pending_transform_dispatch: Option<PendingTransformDispatch>,
    /// Accumulated state for the quick-select brush tool.
    quick_select_state: Option<QuickSelectState>,
    /// Live state for the Warp tool.
    warp_state: Option<WarpState>,
    /// Live state for the Liquify tool.
    liquify_state: Option<LiquifyState>,
    /// Live state for the Gradient fill tool (raster layers).
    gradient_state: Option<GradientState>,
    /// Live state for the Gradient fill tool (vector layers).
    vector_gradient_state: Option<VectorGradientState>,
    /// GPU gradient fill dispatch to run next prepare() frame.
    pending_gradient_op: Option<PendingGradientOp>,
    /// GPU ops for Warp/Liquify to dispatch in prepare().
    pending_warp_ops: Vec<PendingWarpOp>,

    // ── New unified raster tool state ─────────────────────────────────────────
    /// The active `RasterTool` implementation plus its GPU workspace.
    /// Set on mousedown; cleared (and workspace queued for removal) on commit/cancel.
    active_raster_tool: Option<(Box<dyn crate::raster_tool::RasterTool>, crate::raster_tool::RasterWorkspace)>,
    /// Canvas UUIDs to remove from `GpuBrushEngine` at the top of the next `prepare()`.
    /// Drains into `VelloRenderContext::pending_canvas_removals` each frame.
    pending_canvas_removals: Vec<uuid::Uuid>,
    /// First-frame canvas init packet for the active raster tool.  Forwarded to
    /// `VelloRenderContext` on the mousedown frame; cleared after one forwarding.
    pending_workspace_init: Option<crate::raster_tool::WorkspaceInitPacket>,
    /// Keyframe UUIDs whose `raster_layer_cache` entry must be removed so fresh
    /// `raw_pixels` are re-uploaded.  Drained into `VelloRenderContext` each frame.
    pending_layer_cache_removals: Vec<uuid::Uuid>,
    /// True when the unified raster tool has finished (mouseup) and is waiting for
    /// the GPU readback result.  Cleared in render_content() after the result arrives.
    active_tool_awaiting_readback: bool,
    /// B-canvas UUID to readback into RASTER_READBACK_RESULTS on the next prepare().
    /// Set on mouseup when `tool.finish()` returns true; forwarded to VelloRenderContext.
    pending_tool_readback_b: Option<uuid::Uuid>,

    /// Synthetic drag/click override for test mode replay (debug builds only)
    #[cfg(debug_assertions)]
    replay_override: Option<ReplayDragState>,
}

/// Synthetic drag/click state injected during test mode replay
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
pub struct ReplayDragState {
    pub drag_started: bool,
    pub dragged: bool,
    pub drag_stopped: bool,
}

/// Accumulated state for the Quick Select brush-based selection tool.
struct QuickSelectState {
    /// Per-pixel OR'd selection mask (width × height).
    mask: Vec<bool>,
    /// RGBA snapshot of the canvas at drag start (read-only for all fills).
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    /// Last canvas-pixel position where a fill was run (for debouncing).
    last_pos: (i32, i32),
}

/// Live state for an ongoing raster Warp operation.
struct WarpState {
    layer_id: uuid::Uuid,
    time: f64,
    /// Anchor canvas: existing keyframe GPU canvas (kf.id), read-only during warp.
    anchor_canvas_id: uuid::Uuid,
    /// Display canvas: warp-shader output shown in place of the layer.
    display_canvas_id: uuid::Uuid,
    /// Displacement map buffer (zero = no deformation).
    disp_buf_id: uuid::Uuid,
    anchor_w: u32,
    anchor_h: u32,
    grid_cols: u32,
    grid_rows: u32,
    /// Per-control-point state: [home_x, home_y, displaced_x, displaced_y].
    /// Coordinates are in world space (canvas pixels, offset by float_offset if float warp).
    control_points: Vec<[f32; 4]>,
    /// Index of the control point being dragged (if any).
    active_point: Option<usize>,
    /// Index of the control point the cursor is currently over.
    hovered_point: Option<usize>,
    /// True when control points changed and a GPU re-apply is needed.
    dirty: bool,
    /// True once the first warp dispatch has been sent (display canvas has content).
    warp_applied: bool,
    /// True after Enter: waiting for final readback.
    wants_commit: bool,
    /// When warping a floating selection: its world-space top-left offset.
    /// None = warping the full layer canvas.
    float_offset: Option<(i32, i32)>,
}

/// Live state for an ongoing raster Liquify operation.
struct LiquifyState {
    layer_id: uuid::Uuid,
    time: f64,
    /// Anchor canvas: existing keyframe GPU canvas (kf.id), read-only during liquify.
    anchor_canvas_id: uuid::Uuid,
    display_canvas_id: uuid::Uuid,
    disp_buf_id: uuid::Uuid,
    anchor_w: u32,
    anchor_h: u32,
    /// Last brush position (canvas pixels) for debouncing.
    last_brush_pos: Option<(f32, f32)>,
    /// True once the first brush step has been applied.
    liquify_applied: bool,
    /// True after Enter: waiting for final readback.
    wants_commit: bool,
    /// When liquifying a floating selection: its world-space top-left offset. None = full layer.
    float_offset: Option<(i32, i32)>,
}

/// Live state for an ongoing raster Gradient fill drag.
struct GradientState {
    layer_id: uuid::Uuid,
    time: f64,
    start: egui::Vec2,
    end: egui::Vec2,
    /// Snapshot of canvas pixels at drag start (used for CPU commit path).
    before_pixels: Vec<u8>,
    canvas_w: u32,
    canvas_h: u32,
    /// Anchor canvas: holds before_pixels (read-only by gradient shader each frame).
    anchor_canvas_id: uuid::Uuid,
    /// Display canvas: gradient shader writes here each frame; shown via painting_canvas or float path.
    display_canvas_id: uuid::Uuid,
    /// True when painting onto a floating selection instead of the layer canvas.
    is_float: bool,
    /// World-space top-left of the float in canvas pixels (None for non-float).
    float_offset: Option<(f32, f32)>,
}

/// Live state for an ongoing vector-layer Gradient fill drag.
struct VectorGradientState {
    layer_id: uuid::Uuid,
    time: f64,
    face_ids: Vec<lightningbeam_core::dcel2::FaceId>,
    start: egui::Vec2,  // World-space drag start
    end:   egui::Vec2,  // World-space drag end
}

/// GPU ops queued by the Warp/Liquify handlers for `prepare()`.
enum PendingWarpOp {
    /// Upload control-point grid displacements and run warp-apply shader.
    /// disp_data: one vec2 per control point (grid_cols * grid_rows entries).
    /// None = reuse existing buffer (e.g. for final-commit re-apply).
    WarpApply {
        anchor_canvas_id: uuid::Uuid,
        disp_buf_id: uuid::Uuid,
        display_canvas_id: uuid::Uuid,
        disp_data: Option<Vec<[f32; 2]>>,
        grid_cols: u32,
        grid_rows: u32,
        w: u32, h: u32,
        final_commit: bool,
        layer_id: uuid::Uuid,
        time: f64,
        /// True when warping a floating selection.
        is_float_warp: bool,
    },
    /// Update the displacement map from one brush step (Liquify tool).
    LiquifyBrushStep {
        disp_buf_id: uuid::Uuid,
        params: crate::gpu_brush::LiquifyBrushParams,
    },
    /// Run warp-apply shader (Liquify tool — displacement already updated).
    LiquifyApply {
        anchor_canvas_id: uuid::Uuid,
        disp_buf_id: uuid::Uuid,
        display_canvas_id: uuid::Uuid,
        w: u32, h: u32,
        final_commit: bool,
        layer_id: uuid::Uuid,
        time: f64,
        /// True when liquifying a floating selection.
        is_float_warp: bool,
    },
    /// Initialise GPU resources for a new warp/liquify operation.
    /// anchor_canvas_id = kf.id (reuses existing GPU canvas; ensure_canvas is a no-op if present).
    /// anchor_pixels: uploaded to anchor canvas only if it was missing (e.g. after stroke commit).
    /// is_liquify: if true, displacement buffer is full w×h (per-pixel); otherwise 1×1 (grid mode init).
    Init {
        anchor_canvas_id: uuid::Uuid,
        display_canvas_id: uuid::Uuid,
        disp_buf_id: uuid::Uuid,
        w: u32, h: u32,
        anchor_pixels: Vec<u8>,
        is_liquify: bool,
    },
}

/// Result stored by `prepare()` after a warp/liquify commit readback.
struct WarpReadbackResult {
    layer_id: uuid::Uuid,
    time: f64,
    before_pixels: Vec<u8>,
    after_pixels: Vec<u8>,
    width: u32,
    height: u32,
    display_canvas_id: uuid::Uuid,
    anchor_canvas_id: uuid::Uuid,
    /// True when warping a floating selection (don't write to kf.raw_pixels).
    is_float_warp: bool,
}

static WARP_READBACK_RESULTS: OnceLock<Arc<Mutex<std::collections::HashMap<u64, WarpReadbackResult>>>> = OnceLock::new();

/// Cached DCEL snapshot for undo when editing vertices, curves, or control points
#[derive(Clone)]
struct DcelEditingCache {
    /// The layer ID containing the DCEL being edited
    layer_id: uuid::Uuid,
    /// The time of the keyframe being edited
    time: f64,
    /// Snapshot of the DCEL at edit start (for undo)
    dcel_before: lightningbeam_core::dcel::Dcel,
}

// Global counter for generating unique instance IDs
static INSTANCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// Global storage for eyedropper results (instance_id -> (color, color_mode))
static EYEDROPPER_RESULTS: OnceLock<Arc<Mutex<std::collections::HashMap<u64, (egui::Color32, super::ColorMode)>>>> = OnceLock::new();

/// Pending GPU dabs for a single drag event.
///
/// Created by the event handler (`handle_raster_stroke_tool`) and consumed once
/// by `VelloCallback::prepare()`.
struct PendingRasterDabs {
    /// Keyframe UUID — indexes the canvas texture pair in `GpuBrushEngine`.
    keyframe_id: uuid::Uuid,
    /// Layer UUID — used for the undo readback result.
    layer_id: uuid::Uuid,
    /// Playback time of the keyframe.
    time: f64,
    /// Canvas dimensions (pixels).
    canvas_width: u32,
    canvas_height: u32,
    /// Raw RGBA pixel data to upload to the canvas texture on the very first dab of
    /// a stroke (i.e., when the stroke starts).  `None` on subsequent drag events.
    initial_pixels: Option<Vec<u8>>,
    /// Dab list computed by `BrushEngine::compute_dabs()`.
    dabs: Vec<lightningbeam_core::brush_engine::GpuDab>,
    /// Union bounding box of `dabs` (x0, y0, x1, y1) in canvas pixel coords.
    dab_bbox: (i32, i32, i32, i32),
    /// When `true`, perform a full canvas readback after dispatching and store
    /// the result in `RASTER_READBACK_RESULTS` so the next frame can create
    /// the undo action.
    wants_final_readback: bool,
}

/// Which transform handle the user is interacting with.
#[derive(Clone, Copy, PartialEq)]
enum RasterTransformHandle {
    Move,
    Corner { right: bool, bottom: bool },
    EdgeH  { bottom: bool },
    EdgeV  { right: bool  },
    Rotate,
    Origin,  // the pivot point, draggable
}

/// Live state for an ongoing raster transform operation.
struct RasterTransformState {
    /// canvas_id of the float when this state was created. If different → stale, reinit.
    float_canvas_id: uuid::Uuid,
    /// Anchor: original pixels, never written during transform.
    anchor_canvas_id: uuid::Uuid,
    /// sRGB-encoded pixel data for the anchor canvas (re-uploaded each dispatch).
    anchor_pixels: Vec<u8>,
    anchor_w: u32,
    anchor_h: u32,
    /// Display canvas: compute shader output, shown in place of float during transform.
    display_canvas_id: uuid::Uuid,
    /// Center of the transformed bounding box in canvas (world) coords.
    cx: f32, cy: f32,
    scale_x: f32, scale_y: f32,
    /// Rotation in radians.
    angle: f32,
    /// Pivot point for rotate/scale (defaults to center).
    origin_x: f32, origin_y: f32,
    /// Which handle is being dragged, if any.
    active_handle: Option<RasterTransformHandle>,
    /// Which handle the cursor is currently over (for visual feedback).
    hovered_handle: Option<RasterTransformHandle>,
    /// World position where the current drag started.
    drag_start_world: egui::Vec2,
    /// Snapped values captured at drag start.
    snap_cx: f32, snap_cy: f32,
    snap_sx: f32, snap_sy: f32,
    snap_angle: f32,
    snap_origin_x: f32, snap_origin_y: f32,
    /// True once at least one GPU transform dispatch has been queued.
    transform_applied: bool,
    /// True after Enter: waiting for the final readback before clearing state.
    wants_apply: bool,
}

/// GPU work queued by `handle_raster_transform_tool` for `prepare()`.
struct PendingTransformDispatch {
    anchor_canvas_id: uuid::Uuid,
    /// Anchor pixels — re-uploaded each dispatch to keep the anchor immutable.
    anchor_pixels: Vec<u8>,
    anchor_w: u32,
    anchor_h: u32,
    /// Display canvas: compute shader output (was float_canvas_id).
    display_canvas_id: uuid::Uuid,
    /// AABB of the transformed output (for readback result positioning).
    new_x: i32, new_y: i32,
    /// Output canvas dimensions (may differ from anchor if scaled/rotated).
    new_w: u32,
    new_h: u32,
    /// Inverse affine coefficients: src_pixel = A * out_pixel + b.
    a00: f32, a01: f32,
    a10: f32, a11: f32,
    b0: f32, b1: f32,
    /// If true, readback the display canvas after dispatch and store in TRANSFORM_READBACK_RESULTS.
    is_final_commit: bool,
}

/// Pending GPU dispatch for the gradient fill tool.
struct PendingGradientOp {
    anchor_canvas_id:  uuid::Uuid,
    display_canvas_id: uuid::Uuid,
    w: u32,
    h: u32,
    /// If Some: upload these sRGB-premultiplied pixels to the anchor canvas first.
    anchor_pixels: Option<Vec<u8>>,
    start_x: f32,
    start_y: f32,
    end_x: f32,
    end_y: f32,
    opacity:     f32,
    extend_mode: u32,
    kind:        u32,  // 0 = Linear, 1 = Radial
    stops: Vec<crate::gpu_brush::GpuGradientStop>,
}

/// Pixels read back from the transformed display canvas, stored per-instance.
struct TransformReadbackResult {
    pixels: Vec<u8>,
    width:  u32,
    height: u32,
    x: i32,
    y: i32,
    display_canvas_id: uuid::Uuid,
}

/// Sent from StagePane to VelloCallback to override float blit with display canvas.
struct TransformDisplayInfo {
    display_canvas_id: uuid::Uuid,
    x: i32, y: i32,
    w: u32, h: u32,
}

static TRANSFORM_READBACK_RESULTS: OnceLock<Arc<Mutex<std::collections::HashMap<u64, TransformReadbackResult>>>> = OnceLock::new();

/// Result stored by `prepare()` after a stroke-end readback.
struct RasterReadbackResult {
    layer_id: uuid::Uuid,
    time: f64,
    canvas_width: u32,
    canvas_height: u32,
    /// Raw RGBA pixels from the completed stroke.
    pixels: Vec<u8>,
}

// Global storage for raster readback results (instance_id -> result)
static RASTER_READBACK_RESULTS: OnceLock<Arc<Mutex<std::collections::HashMap<u64, RasterReadbackResult>>>> = OnceLock::new();

/// Cached 2x2 stipple image brush for selection overlay.
/// Pattern: [[black, transparent], [transparent, white]]
/// Tiled with nearest-neighbor sampling so each pixel stays crisp.
static SELECTION_STIPPLE: OnceLock<vello::peniko::ImageBrush> = OnceLock::new();

fn selection_stipple_brush() -> &'static vello::peniko::ImageBrush {
    SELECTION_STIPPLE.get_or_init(|| {
        use vello::peniko::{Blob, Extend, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
        // 2x2 RGBA pixels: row-major order
        // [0,0] = black opaque,  [1,0] = transparent
        // [0,1] = transparent,   [1,1] = white opaque
        let pixels: Vec<u8> = vec![
            0,   0,   0,   255, // (0,0) black
            0,   0,   0,   0,   // (1,0) transparent
            0,   0,   0,   0,   // (0,1) transparent
            255, 255, 255, 255, // (1,1) white
        ];
        let image_data = ImageData {
            data: Blob::from(pixels),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: 2,
            height: 2,
        };
        ImageBrush::new(image_data)
            .with_extend(Extend::Repeat)
            .with_quality(ImageQuality::Low)
    })
}

impl StagePane {
    pub fn new() -> Self {
        let instance_id = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            pan_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            needs_initial_center: true,
            is_panning: false,
            last_pan_pos: None,
            instance_id,
            pending_eyedropper_sample: None,
            last_viewport_rect: None,
            dcel_editing_cache: None,
            current_snap: None,
            raster_stroke_state: None,
            raster_last_point: None,
            pending_raster_dabs: None,
            pending_undo_before: None,
            painting_canvas: None,
            pending_canvas_removal: None,
            stroke_clip_selection: None,
            painting_float: false,
            raster_last_compute_time: 0.0,
            clone_stroke_offset: None,
            raster_transform_state: None,
            pending_transform_dispatch: None,
            quick_select_state: None,
            warp_state: None,
            liquify_state: None,
            gradient_state: None,
            vector_gradient_state: None,
            pending_gradient_op: None,
            pending_warp_ops: Vec::new(),
            active_raster_tool: None,
            pending_canvas_removals: Vec::new(),
            pending_workspace_init: None,
            pending_layer_cache_removals: Vec::new(),
            active_tool_awaiting_readback: false,
            pending_tool_readback_b: None,
            #[cfg(debug_assertions)]
            replay_override: None,
        }
    }

    /// Check if a drag started, respecting replay override
    fn rsp_drag_started(&self, response: &egui::Response) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.drag_started; }
        response.drag_started()
    }

    /// Check if dragging, respecting replay override
    fn rsp_dragged(&self, response: &egui::Response) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.dragged; }
        response.dragged()
    }

    /// Check if drag stopped, respecting replay override
    fn rsp_drag_stopped(&self, response: &egui::Response) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.drag_stopped; }
        response.drag_stopped()
    }

    /// Check if clicked (a click is a drag_started + drag_stopped in the same spot),
    /// respecting replay override
    fn rsp_clicked(&self, response: &egui::Response) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.drag_started; }
        response.clicked()
    }

    /// Check if primary mouse button was just pressed this frame,
    /// respecting replay override
    fn rsp_primary_pressed(&self, ui: &egui::Ui) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.drag_started; }
        ui.input(|i| i.pointer.primary_pressed())
    }

    /// Check if any pointer button was released this frame,
    /// respecting replay override (returns the synthetic drag_stopped during replay)
    fn rsp_any_released(&self, ui: &egui::Ui) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.drag_stopped; }
        ui.input(|i| i.pointer.any_released())
    }

    /// Check if primary pointer button is currently held down,
    /// respecting replay override
    fn rsp_primary_down(&self, ui: &egui::Ui) -> bool {
        #[cfg(debug_assertions)]
        if let Some(ref o) = self.replay_override { return o.dragged || o.drag_started; }
        ui.input(|i| i.pointer.primary_down())
    }

    /// Convert a document-space position to clip-local coordinates when editing inside a clip.
    /// Returns the position unchanged when at root level.
    fn doc_to_clip_local(&self, doc_pos: egui::Vec2, shared: &SharedPaneState) -> egui::Vec2 {
        if let (Some(parent_layer_id), Some(instance_id)) = (shared.editing_parent_layer_id, shared.editing_instance_id) {
            let document = shared.action_executor.document();
            let clip_affine = document.get_layer(&parent_layer_id)
                .and_then(|layer| {
                    if let lightningbeam_core::layer::AnyLayer::Vector(vl) = layer {
                        vl.clip_instances.iter().find(|ci| ci.id == instance_id)
                    } else {
                        None
                    }
                })
                .map(|ci| ci.transform.to_affine())
                .unwrap_or(vello::kurbo::Affine::IDENTITY);
            let inv = clip_affine.inverse();
            let p = inv * vello::kurbo::Point::new(doc_pos.x as f64, doc_pos.y as f64);
            egui::vec2(p.x as f32, p.y as f32)
        } else {
            doc_pos
        }
    }

    /// Convert a clip-local position back to document-space coordinates.
    /// Returns the position unchanged when at root level.
    fn clip_local_to_doc(&self, local_pos: vello::kurbo::Point, shared: &SharedPaneState) -> vello::kurbo::Point {
        if let (Some(parent_layer_id), Some(instance_id)) = (shared.editing_parent_layer_id, shared.editing_instance_id) {
            let document = shared.action_executor.document();
            let clip_affine = document.get_layer(&parent_layer_id)
                .and_then(|layer| {
                    if let lightningbeam_core::layer::AnyLayer::Vector(vl) = layer {
                        vl.clip_instances.iter().find(|ci| ci.id == instance_id)
                    } else {
                        None
                    }
                })
                .map(|ci| ci.transform.to_affine())
                .unwrap_or(vello::kurbo::Affine::IDENTITY);
            clip_affine * local_pos
        } else {
            local_pos
        }
    }

    /// Execute a view action with the given parameters
    /// Called from main.rs after determining this is the best handler
    pub fn execute_view_action(&mut self, action: &crate::menu::MenuAction, zoom_center: egui::Vec2) {
        use crate::menu::MenuAction;
        match action {
            MenuAction::ZoomIn => self.zoom_in(zoom_center),
            MenuAction::ZoomOut => self.zoom_out(zoom_center),
            MenuAction::ActualSize => self.actual_size(),
            MenuAction::RecenterView => self.recenter(),
            _ => {} // Not a view action we handle
        }
    }

    /// Zoom in by a fixed increment (to center of viewport)
    pub fn zoom_in(&mut self, center: egui::Vec2) {
        self.apply_zoom_at_point(0.2, center);
    }

    /// Zoom out by a fixed increment (to center of viewport)
    pub fn zoom_out(&mut self, center: egui::Vec2) {
        self.apply_zoom_at_point(-0.2, center);
    }

    /// Reset zoom to 100% (1.0)
    pub fn actual_size(&mut self) {
        self.zoom = 1.0;
    }

    /// Reset pan to center (0,0) and zoom to 100%
    pub fn recenter(&mut self) {
        self.pan_offset = egui::Vec2::ZERO;
        self.zoom = 1.0;
    }

    /// Zoom to fit the canvas (document dimensions) in the available viewport
    pub fn zoom_to_fit(&mut self, shared: &SharedPaneState) {
        let document = shared.action_executor.document();

        // Get document dimensions
        let doc_width = document.width as f32;
        let doc_height = document.height as f32;

        // Get viewport size from last known rect
        let viewport_size = if let Some(rect) = self.last_viewport_rect {
            rect.size()
        } else {
            // Fallback if we don't have a rect yet
            egui::vec2(800.0, 600.0)
        };

        // Calculate zoom to fit both width and height (no padding - use entire space)
        let zoom_x = viewport_size.x / doc_width;
        let zoom_y = viewport_size.y / doc_height;

        // Use the smaller zoom to ensure both dimensions fit
        self.zoom = zoom_x.min(zoom_y).clamp(0.1, 10.0);

        // Center the document in the viewport
        let canvas_center = egui::vec2(doc_width / 2.0, doc_height / 2.0) * self.zoom;
        let viewport_center = viewport_size / 2.0;
        self.pan_offset = viewport_center - canvas_center;
    }

    /// Apply zoom while keeping the point under the mouse cursor stationary
    fn apply_zoom_at_point(&mut self, zoom_delta: f32, mouse_canvas_pos: egui::Vec2) {
        let old_zoom = self.zoom;

        // Calculate world position under mouse before zoom
        let world_pos = (mouse_canvas_pos - self.pan_offset) / old_zoom;

        // Apply zoom
        let new_zoom = (old_zoom * (1.0 + zoom_delta)).clamp(0.1, 10.0);
        self.zoom = new_zoom;

        // Adjust pan so the same world point stays under the mouse
        self.pan_offset = mouse_canvas_pos - (world_pos * new_zoom);
    }

    fn handle_select_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shift_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::hit_test::{self, hit_test_vector_editing, EditingHitTolerance, VectorEditHit};
        use vello::kurbo::{Point, Rect as KurboRect, Affine};

        // Check if we have an active vector layer
        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return, // No active layer
        };

        // Revert any active region selection on mouse press before borrowing the document
        // immutably, so the two selection modes don't coexist.
        if self.rsp_primary_pressed(ui) {
            Self::revert_region_selection_static(shared);
        }

        let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        let vector_layer = match active_layer {
            AnyLayer::Vector(vl) => vl,
            _ => return, // Not a vector layer
        };

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Double-click: enter/exit movie clip editing
        if response.double_clicked() {
            // Hit test clip instances at the click position
            let document = shared.action_executor.document();
            let clip_hit = hit_test::hit_test_clip_instances(
                &vector_layer.clip_instances,
                document,
                point,
                Affine::IDENTITY,
                *shared.playback_time,
            );

            if let Some(instance_id) = clip_hit {
                // Find the clip instance to get its clip_id
                if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == instance_id) {
                    // Check if this is a movie clip (not a group)
                    if let Some(vector_clip) = document.get_vector_clip(&clip_instance.clip_id) {
                        if !vector_clip.is_group {
                            // Enter the movie clip
                            *shared.pending_enter_clip = Some((
                                clip_instance.clip_id,
                                instance_id,
                                active_layer_id,
                            ));
                            return;
                        }
                    }
                }
            } else if shared.editing_clip_id.is_some() {
                // Double-click on empty space while inside a clip: exit
                *shared.pending_exit_clip = true;
                return;
            }
        }

        // Mouse down: start interaction (check on initial press, not after drag starts)
        // Scope this section to drop vector_layer borrow before drag handling
        let mouse_pressed = self.rsp_primary_pressed(ui);
        if mouse_pressed {
            // VECTOR EDITING: Check for vertex/curve editing first (higher priority than selection)
            let tolerance = EditingHitTolerance::scaled_by_zoom(self.zoom as f64);
            let vector_hit = hit_test_vector_editing(
                vector_layer,
                *shared.playback_time,
                point,
                &tolerance,
                Affine::IDENTITY,
                false, // Select tool doesn't show control points
            );
            // Priority 1: Vector editing (vertices immediately, curves deferred)
            if let Some(hit) = vector_hit {
                match hit {
                    VectorEditHit::Vertex { vertex_id } => {
                        self.start_vertex_editing(vertex_id, point, active_layer_id, shared);
                        return;
                    }
                    VectorEditHit::Curve { edge_id, parameter_t } => {
                        // Defer: drag → curve editing, click → edge selection
                        *shared.tool_state = ToolState::PendingCurveInteraction {
                            edge_id,
                            parameter_t,
                            start_mouse: point,
                        };
                        return;
                    }
                    _ => {
                        // Fill hit - fall through to normal selection
                    }
                }
            }

            // Priority 2: Normal selection/dragging (no vector element hit)
            // Hit test at click position
            // Test clip instances first (they're on top of shapes)
            let document = shared.action_executor.document();
            let clip_hit = hit_test::hit_test_clip_instances(
                &vector_layer.clip_instances,
                document,
                point,
                Affine::IDENTITY,
                *shared.playback_time,
            );

            let hit_result = if let Some(clip_id) = clip_hit {
                Some(hit_test::HitResult::ClipInstance(clip_id))
            } else {
                // No clip hit, test DCEL edges and faces
                hit_test::hit_test_layer(vector_layer, *shared.playback_time, point, 5.0, Affine::IDENTITY)
                    .map(|dcel_hit| match dcel_hit {
                        hit_test::DcelHitResult::Edge(eid) => hit_test::HitResult::Edge(eid),
                        hit_test::DcelHitResult::Face(fid) => hit_test::HitResult::Face(fid),
                    })
            };

            if let Some(hit) = hit_result {
                match hit {
                    hit_test::HitResult::Edge(edge_id) => {
                        // DCEL edge was hit
                        if let Some(dcel) = vector_layer.dcel_at_time(*shared.playback_time) {
                            if shift_held {
                                shared.selection.toggle_edge(edge_id, dcel);
                            } else {
                                shared.selection.clear_dcel_selection();
                                shared.selection.select_edge(edge_id, dcel);
                            }
                        }
                        if let Some(layer_id) = *shared.active_layer_id {
                            *shared.focus = lightningbeam_core::selection::FocusSelection::Geometry { layer_id, time: *shared.playback_time };
                        }
                        // DCEL element dragging deferred to Phase 3
                    }
                    hit_test::HitResult::Face(face_id) => {
                        // DCEL face was hit
                        if let Some(dcel) = vector_layer.dcel_at_time(*shared.playback_time) {
                            if shift_held {
                                shared.selection.toggle_face(face_id, dcel);
                            } else {
                                shared.selection.clear_dcel_selection();
                                shared.selection.select_face(face_id, dcel);
                            }
                        }
                        if let Some(layer_id) = *shared.active_layer_id {
                            *shared.focus = lightningbeam_core::selection::FocusSelection::Geometry { layer_id, time: *shared.playback_time };
                        }
                        // DCEL element dragging deferred to Phase 3
                    }
                    hit_test::HitResult::ClipInstance(clip_id) => {
                        // Clip instance was hit
                        if shift_held {
                            // Shift: toggle selection
                            shared.selection.toggle_clip_instance(clip_id);
                        } else {
                            // No shift: replace selection
                            if !shared.selection.contains_clip_instance(&clip_id) {
                                shared.selection.select_only_clip_instance(clip_id);
                            }
                        }
                        *shared.focus = lightningbeam_core::selection::FocusSelection::ClipInstances(shared.selection.clip_instances().to_vec());

                        // If clip instance is now selected, prepare for dragging
                        if shared.selection.contains_clip_instance(&clip_id) {
                            // Store original positions of all selected clip instances
                            let mut original_positions = std::collections::HashMap::new();
                            for &clip_inst_id in shared.selection.clip_instances() {
                                // Find the clip instance in the layer
                                if let Some(clip_inst) = vector_layer.clip_instances.iter()
                                    .find(|ci| ci.id == clip_inst_id) {
                                    original_positions.insert(
                                        clip_inst_id,
                                        Point::new(clip_inst.transform.x, clip_inst.transform.y),
                                    );
                                }
                            }

                            *shared.tool_state = ToolState::DraggingSelection {
                                start_pos: point,
                                start_mouse: point,
                                original_positions,
                            };
                        }
                    }
                }
            } else {
                // Nothing hit - start marquee selection
                if !shift_held {
                    shared.selection.clear();
                    *shared.focus = lightningbeam_core::selection::FocusSelection::None;
                }

                *shared.tool_state = ToolState::MarqueeSelecting {
                    start: point,
                    current: point,
                };
            }
        }

        // Mouse drag: update tool state
        if self.rsp_dragged(response) {
            match shared.tool_state {
                ToolState::PendingCurveInteraction { edge_id, parameter_t, start_mouse } => {
                    // Drag detected — transition to curve editing
                    let edge_id = *edge_id;
                    let parameter_t = *parameter_t;
                    let start_mouse = *start_mouse;
                    self.start_curve_editing(edge_id, parameter_t, start_mouse, active_layer_id, shared);
                    self.update_vector_editing(point, shared);
                }
                ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } => {
                    // Vector editing - update happens in helper method
                    self.update_vector_editing(point, shared);
                }
                ToolState::DraggingSelection { .. } => {
                    // Update current position (visual feedback only)
                    // Actual move happens on mouse up
                }
                ToolState::MarqueeSelecting { start, .. } => {
                    // Update marquee rectangle
                    *shared.tool_state = ToolState::MarqueeSelecting {
                        start: *start,
                        current: point,
                    };
                }
                _ => {}
            }
        }

        // Mouse up: finish interaction
        let drag_stopped = self.rsp_drag_stopped(response);
        let pointer_released = self.rsp_any_released(ui);
        let is_pending_curve = matches!(shared.tool_state, ToolState::PendingCurveInteraction { .. });
        let is_drag_or_marquee = matches!(shared.tool_state, ToolState::DraggingSelection { .. } | ToolState::MarqueeSelecting { .. });
        let is_vector_editing = matches!(shared.tool_state, ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } | ToolState::EditingControlPoint { .. });

        if drag_stopped || (pointer_released && (is_drag_or_marquee || is_vector_editing || is_pending_curve)) {
            match shared.tool_state.clone() {
                ToolState::PendingCurveInteraction { edge_id, .. } => {
                    // Mouse released without drag — select the edge
                    let shift_held = ui.input(|i| i.modifiers.shift);
                    let document = shared.action_executor.document();
                    if let Some(layer) = document.get_layer(&active_layer_id) {
                        if let AnyLayer::Vector(vl) = layer {
                            if let Some(dcel) = vl.dcel_at_time(*shared.playback_time) {
                                if !shift_held {
                                    shared.selection.clear_dcel_selection();
                                }
                                shared.selection.select_edge(edge_id, dcel);
                            }
                        }
                    }
                    if let Some(layer_id) = *shared.active_layer_id {
                        *shared.focus = lightningbeam_core::selection::FocusSelection::Geometry { layer_id, time: *shared.playback_time };
                    }
                    *shared.tool_state = ToolState::Idle;
                }
                ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } | ToolState::EditingControlPoint { .. } => {
                    // Finish vector editing - create action
                    self.finish_vector_editing(active_layer_id, shared);
                }
                ToolState::DraggingSelection { start_mouse, original_positions, .. } => {
                    // Calculate total delta
                    let delta = point - start_mouse;

                    if delta.x.abs() > 0.01 || delta.y.abs() > 0.01 {
                        // Create move actions with new positions
                        use std::collections::HashMap;

                        // Get vector layer again (to avoid holding borrow from earlier)
                        let document = shared.action_executor.document();
                        let layer = match document.get_layer(&active_layer_id) {
                            Some(l) => l,
                            None => return,
                        };
                        let vector_layer = match layer {
                            AnyLayer::Vector(vl) => vl,
                            _ => return,
                        };

                        // Process clip instance drags
                        let mut clip_instance_transforms = HashMap::new();

                        for (id, original_pos) in original_positions {
                            let new_pos = Point::new(
                                original_pos.x + delta.x,
                                original_pos.y + delta.y,
                            );

                            if shared.selection.contains_clip_instance(&id) {
                                if let Some(clip_inst) = vector_layer.clip_instances.iter()
                                    .find(|ci| ci.id == id) {
                                    let mut old_transform = clip_inst.transform.clone();
                                    old_transform.x = original_pos.x;
                                    old_transform.y = original_pos.y;

                                    let mut new_transform = clip_inst.transform.clone();
                                    new_transform.x = new_pos.x;
                                    new_transform.y = new_pos.y;

                                    clip_instance_transforms.insert(id, (old_transform, new_transform));
                                }
                            }
                        }

                        // Create and submit transform action for clip instances
                        if !clip_instance_transforms.is_empty() {
                            use lightningbeam_core::actions::TransformClipInstancesAction;
                            let action = TransformClipInstancesAction::new(active_layer_id, *shared.playback_time, clip_instance_transforms);
                            shared.pending_actions.push(Box::new(action));
                        }
                    }

                    // Reset tool state
                    *shared.tool_state = ToolState::Idle;
                }
                ToolState::MarqueeSelecting { start, current } => {
                    // Create selection rectangle
                    let min_x = start.x.min(current.x);
                    let min_y = start.y.min(current.y);
                    let max_x = start.x.max(current.x);
                    let max_y = start.y.max(current.y);

                    let selection_rect = KurboRect::new(min_x, min_y, max_x, max_y);

                    // Get vector layer again (to avoid holding borrow from earlier)
                    let document = shared.action_executor.document();
                    let layer = match document.get_layer(&active_layer_id) {
                        Some(l) => l,
                        None => return,
                    };
                    let vector_layer = match layer {
                        AnyLayer::Vector(vl) => vl,
                        _ => return,
                    };

                    // Hit test clip instances in rectangle
                    let clip_hits = hit_test::hit_test_clip_instances_in_rect(
                        &vector_layer.clip_instances,
                        document,
                        selection_rect,
                        Affine::IDENTITY,
                        *shared.playback_time,
                    );

                    // Hit test DCEL elements in rectangle
                    let dcel_hits = hit_test::hit_test_dcel_in_rect(
                        vector_layer,
                        *shared.playback_time,
                        selection_rect,
                        Affine::IDENTITY,
                    );

                    // Add clip instances to selection
                    for clip_id in clip_hits {
                        shared.selection.add_clip_instance(clip_id);
                    }

                    // Add DCEL elements to selection
                    if let Some(dcel) = vector_layer.dcel_at_time(*shared.playback_time) {
                        for edge_id in dcel_hits.edges {
                            shared.selection.select_edge(edge_id, dcel);
                        }
                        for face_id in dcel_hits.faces {
                            shared.selection.select_face(face_id, dcel);
                        }
                    }

                    // Update focus based on what was selected
                    if shared.selection.has_dcel_selection() {
                        if let Some(layer_id) = *shared.active_layer_id {
                            *shared.focus = lightningbeam_core::selection::FocusSelection::Geometry { layer_id, time: *shared.playback_time };
                        }
                    } else if !shared.selection.clip_instances().is_empty() {
                        *shared.focus = lightningbeam_core::selection::FocusSelection::ClipInstances(shared.selection.clip_instances().to_vec());
                    }

                    // Reset tool state
                    *shared.tool_state = ToolState::Idle;
                }
                _ => {}
            }
        }
    }

    /// Start editing a vertex - called when user clicks on a vertex
    fn start_vertex_editing(
        &mut self,
        vertex_id: lightningbeam_core::dcel::VertexId,
        _mouse_pos: vello::kurbo::Point,
        active_layer_id: uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::tool::ToolState;

        let time = *shared.playback_time;
        let document = shared.action_executor.document();
        let layer = match document.get_layer(&active_layer_id) {
            Some(AnyLayer::Vector(vl)) => vl,
            _ => return,
        };
        let dcel = match layer.dcel_at_time(time) {
            Some(d) => d,
            None => return,
        };

        // Snapshot DCEL for undo
        self.dcel_editing_cache = Some(DcelEditingCache {
            layer_id: active_layer_id,
            time,
            dcel_before: dcel.clone(),
        });

        // Find connected edges: iterate outgoing half-edges, collect unique edge IDs
        let outgoing = dcel.vertex_outgoing(vertex_id);
        let mut connected_edges = Vec::new();
        for he_id in &outgoing {
            let edge_id = dcel.half_edge(*he_id).edge;
            if !connected_edges.contains(&edge_id) {
                connected_edges.push(edge_id);
            }
        }

        *shared.tool_state = ToolState::EditingVertex {
            vertex_id,
            connected_edges,
        };
    }

    /// Start editing a curve - called when user clicks on a curve
    fn start_curve_editing(
        &mut self,
        edge_id: lightningbeam_core::dcel::EdgeId,
        parameter_t: f64,
        mouse_pos: vello::kurbo::Point,
        active_layer_id: uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::tool::ToolState;

        let time = *shared.playback_time;
        let document = shared.action_executor.document();
        let layer = match document.get_layer(&active_layer_id) {
            Some(AnyLayer::Vector(vl)) => vl,
            _ => return,
        };
        let dcel = match layer.dcel_at_time(time) {
            Some(d) => d,
            None => return,
        };

        let original_curve = dcel.edge(edge_id).curve;

        // Snapshot DCEL for undo
        self.dcel_editing_cache = Some(DcelEditingCache {
            layer_id: active_layer_id,
            time,
            dcel_before: dcel.clone(),
        });

        *shared.tool_state = ToolState::EditingCurve {
            edge_id,
            original_curve,
            start_mouse: mouse_pos,
            parameter_t,
        };
    }

    /// Update vector editing during drag — mutates DCEL directly for live preview
    fn update_vector_editing(
        &mut self,
        mouse_pos: vello::kurbo::Point,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::bezpath_editing::mold_curve;
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::snap::{self, SnapConfig, SnapExclusion, SNAP_SCREEN_RADIUS};
        use lightningbeam_core::tool::ToolState;
        use vello::kurbo::Vec2;

        let cache = match &self.dcel_editing_cache {
            Some(c) => c,
            None => return,
        };
        let layer_id = cache.layer_id;
        let time = cache.time;

        // Clone tool state to avoid borrow conflict
        let tool_state = shared.tool_state.clone();
        let snap_enabled = *shared.snap_enabled;

        // Phase 1: Compute snap target with immutable DCEL borrow.
        // Don't snap during curve molding — the mouse is a relative guide for
        // adjusting control points, not an absolute target.
        let skip_snap = matches!(tool_state, ToolState::EditingCurve { .. });
        let snap_result = if snap_enabled && !skip_snap {
            let document = shared.action_executor.document();
            let dcel = match document.get_layer(&layer_id) {
                Some(AnyLayer::Vector(vl)) => vl.dcel_at_time(time),
                _ => None,
            };
            dcel.and_then(|dcel| {
                let config = SnapConfig::from_screen_radius(SNAP_SCREEN_RADIUS, self.zoom as f64);
                let exclusion = match &tool_state {
                    ToolState::EditingVertex { vertex_id, connected_edges } => SnapExclusion {
                        vertices: vec![*vertex_id],
                        edges: connected_edges.clone(),
                    },
                    ToolState::EditingControlPoint { edge_id, .. } => SnapExclusion {
                        edges: vec![*edge_id],
                        ..Default::default()
                    },
                    _ => SnapExclusion::default(),
                };
                snap::find_snap_target(dcel, mouse_pos, &config, &exclusion)
            })
        } else {
            None
        };

        self.current_snap = snap_result;
        let effective_pos = snap_result.map(|r| r.position).unwrap_or(mouse_pos);

        // Phase 2: Mutate DCEL with the (possibly snapped) position
        let document = shared.action_executor.document_mut();
        let dcel = match document.get_layer_mut(&layer_id) {
            Some(AnyLayer::Vector(vl)) => match vl.dcel_at_time_mut(time) {
                Some(d) => d,
                None => return,
            },
            _ => return,
        };

        match tool_state {
            ToolState::EditingVertex { vertex_id, connected_edges } => {
                let old_pos = dcel.vertex(vertex_id).position;
                let delta = Vec2::new(effective_pos.x - old_pos.x, effective_pos.y - old_pos.y);
                dcel.vertex_mut(vertex_id).position = effective_pos;

                // Update connected edges: shift the adjacent control point by the same delta
                for &edge_id in &connected_edges {
                    let edge = dcel.edge(edge_id);
                    let [he_fwd, _he_bwd] = edge.half_edges;
                    let fwd_origin = dcel.half_edge(he_fwd).origin;
                    let mut curve = dcel.edge(edge_id).curve;

                    if fwd_origin == vertex_id {
                        curve.p0 = effective_pos;
                        curve.p1 = curve.p1 + delta;
                    } else {
                        curve.p3 = effective_pos;
                        curve.p2 = curve.p2 + delta;
                    }
                    dcel.edge_mut(edge_id).curve = curve;
                }
            }
            ToolState::EditingCurve { edge_id, original_curve, start_mouse, .. } => {
                let molded_curve = mold_curve(&original_curve, &effective_pos, &start_mouse);
                dcel.edge_mut(edge_id).curve = molded_curve;
            }
            ToolState::EditingControlPoint { edge_id, point_index, .. } => {
                let curve = &mut dcel.edge_mut(edge_id).curve;
                match point_index {
                    1 => curve.p1 = effective_pos,
                    2 => curve.p2 = effective_pos,
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Finish vector editing and create action for undo/redo
    fn finish_vector_editing(
        &mut self,
        active_layer_id: uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::actions::ModifyDcelAction;
        use lightningbeam_core::layer::AnyLayer;

        // Consume the cache
        let cache = match self.dcel_editing_cache.take() {
            Some(c) => c,
            None => {
                *shared.tool_state = lightningbeam_core::tool::ToolState::Idle;
                return;
            }
        };

        // After editing vertices/curves/control points, rebuild CCW fan ordering
        // at affected vertices and recompute edge intersections before snapshotting.
        // Without this, stale fan ordering causes topology corruption on subsequent
        // stroke insertions (e.g. face/cycle mismatches).
        let editing_info = match &*shared.tool_state {
            lightningbeam_core::tool::ToolState::EditingCurve { edge_id, .. } => {
                Some((vec![*edge_id], vec![]))
            }
            lightningbeam_core::tool::ToolState::EditingVertex { vertex_id, connected_edges } => {
                Some((connected_edges.clone(), vec![*vertex_id]))
            }
            lightningbeam_core::tool::ToolState::EditingControlPoint { edge_id, .. } => {
                Some((vec![*edge_id], vec![]))
            }
            _ => None,
        };

        if let Some((edge_ids, vertex_ids)) = editing_info {
            let document = shared.action_executor.document_mut();
            if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(&active_layer_id) {
                if let Some(dcel) = vl.dcel_at_time_mut(cache.time) {
                    // Rebuild fans at the directly edited vertices
                    for &vid in &vertex_ids {
                        dcel.rebuild_vertex_fan(vid);
                    }
                    // Also rebuild fans at endpoints of connected edges
                    // (their edge angles changed due to the edit)
                    for &eid in &edge_ids {
                        let [fwd, bwd] = dcel.edge(eid).half_edges;
                        let v1 = dcel.half_edge(fwd).origin;
                        let v2 = dcel.half_edge(bwd).origin;
                        if !vertex_ids.contains(&v1) {
                            dcel.rebuild_vertex_fan(v1);
                        }
                        if !vertex_ids.contains(&v2) {
                            dcel.rebuild_vertex_fan(v2);
                        }
                    }
                    // Repair face cycles at all affected vertices
                    // (rebuild_vertex_fan may have split cycles without updating faces)
                    let mut repaired: Vec<lightningbeam_core::dcel2::VertexId> = Vec::new();
                    for &vid in &vertex_ids {
                        if !repaired.contains(&vid) {
                            dcel.repair_face_cycles_at_vertex(vid);
                            repaired.push(vid);
                        }
                    }
                    for &eid in &edge_ids {
                        let [fwd, bwd] = dcel.edge(eid).half_edges;
                        let v1 = dcel.half_edge(fwd).origin;
                        let v2 = dcel.half_edge(bwd).origin;
                        if !repaired.contains(&v1) {
                            dcel.repair_face_cycles_at_vertex(v1);
                            repaired.push(v1);
                        }
                        if !repaired.contains(&v2) {
                            dcel.repair_face_cycles_at_vertex(v2);
                            repaired.push(v2);
                        }
                    }
                    // Recompute intersections for all moved edges
                    for &eid in &edge_ids {
                        dcel.recompute_edge_intersections(eid);
                    }
                }
            }
        }

        // Get current DCEL state (after edits + intersection splits) as dcel_after
        let dcel_after = {
            let document = shared.action_executor.document();
            match document.get_layer(&active_layer_id) {
                Some(AnyLayer::Vector(vl)) => match vl.dcel_at_time(cache.time) {
                    Some(d) => d.clone(),
                    None => {
                        *shared.tool_state = lightningbeam_core::tool::ToolState::Idle;
                        return;
                    }
                },
                _ => {
                    *shared.tool_state = lightningbeam_core::tool::ToolState::Idle;
                    return;
                }
            }
        };

        // Create the undo action
        let action = ModifyDcelAction::new(
            cache.layer_id,
            cache.time,
            cache.dcel_before,
            dcel_after,
            "Edit vector path",
        );

        // Execute via action system (this replaces the DCEL with dcel_after,
        // which is the same as current state, so it's a no-op — but it registers
        // the action in the undo stack with dcel_before for rollback)
        let _ = shared.action_executor.execute(Box::new(action));

        // Reset tool state and clear snap indicator
        *shared.tool_state = lightningbeam_core::tool::ToolState::Idle;
        self.current_snap = None;
    }

    /// Handle BezierEdit tool - similar to Select but with control point editing
    fn handle_bezier_edit_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        _shift_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::hit_test::{hit_test_vector_editing, EditingHitTolerance, VectorEditHit};
        use vello::kurbo::{Point, Affine};

        // Check if we have an active vector layer
        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        let vector_layer = match active_layer {
            AnyLayer::Vector(vl) => vl,
            _ => return,
        };

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // VECTOR EDITING: Check for control points, vertices, and curves (higher priority than selection)
        let tolerance = EditingHitTolerance::scaled_by_zoom(self.zoom as f64);
        let vector_hit = hit_test_vector_editing(
            vector_layer,
            *shared.playback_time,
            point,
            &tolerance,
            Affine::IDENTITY,
            true, // BezierEdit tool shows control points
        );

        // Mouse down: start interaction (check on initial press, not after drag starts)
        let mouse_pressed = self.rsp_primary_pressed(ui);
        if mouse_pressed {
            // Priority 1: Vector editing (control points, vertices, and curves)
            if let Some(hit) = vector_hit {
                match hit {
                    VectorEditHit::ControlPoint { edge_id, point_index } => {
                        self.start_control_point_editing(edge_id, point_index, point, active_layer_id, shared);
                        return;
                    }
                    VectorEditHit::Vertex { vertex_id } => {
                        self.start_vertex_editing(vertex_id, point, active_layer_id, shared);
                        return;
                    }
                    VectorEditHit::Curve { edge_id, parameter_t } => {
                        self.start_curve_editing(edge_id, parameter_t, point, active_layer_id, shared);
                        return;
                    }
                    _ => {
                        // Fill hit - no selection in BezierEdit mode, just ignore
                    }
                }
            }
        }

        // Mouse drag: update tool state
        if self.rsp_dragged(response) {
            match shared.tool_state {
                ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } | ToolState::EditingControlPoint { .. } => {
                    // Vector editing - update happens in helper method
                    self.update_vector_editing(point, shared);
                }
                _ => {}
            }
        }

        // Mouse up: finish interaction
        let drag_stopped = self.rsp_drag_stopped(response);
        let pointer_released = self.rsp_any_released(ui);
        let is_vector_editing = matches!(shared.tool_state, ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } | ToolState::EditingControlPoint { .. });

        if drag_stopped || (pointer_released && is_vector_editing) {
            match shared.tool_state.clone() {
                ToolState::EditingVertex { .. } | ToolState::EditingCurve { .. } | ToolState::EditingControlPoint { .. } => {
                    self.finish_vector_editing(active_layer_id, shared);
                }
                _ => {}
            }
        }
    }

    /// Start editing a control point - called when user clicks on a control point
    fn start_control_point_editing(
        &mut self,
        edge_id: lightningbeam_core::dcel::EdgeId,
        point_index: u8,
        _mouse_pos: vello::kurbo::Point,
        active_layer_id: uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::tool::ToolState;

        let time = *shared.playback_time;
        let document = shared.action_executor.document();
        let layer = match document.get_layer(&active_layer_id) {
            Some(AnyLayer::Vector(vl)) => vl,
            _ => return,
        };
        let dcel = match layer.dcel_at_time(time) {
            Some(d) => d,
            None => return,
        };

        let original_curve = dcel.edge(edge_id).curve;
        let start_pos = match point_index {
            1 => original_curve.p1,
            2 => original_curve.p2,
            _ => return,
        };

        // Snapshot DCEL for undo
        self.dcel_editing_cache = Some(DcelEditingCache {
            layer_id: active_layer_id,
            time,
            dcel_before: dcel.clone(),
        });

        *shared.tool_state = ToolState::EditingControlPoint {
            edge_id,
            point_index,
            original_curve,
            start_pos,
        };
    }

    /// Compute snap for shape/draw tools (no exclusions).
    /// Derives active layer and time from `shared`. Updates `self.current_snap`
    /// and returns the (possibly snapped) position.
    fn snap_point(
        &mut self,
        point: vello::kurbo::Point,
        shared: &SharedPaneState,
    ) -> vello::kurbo::Point {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::snap::{self, SnapConfig, SnapExclusion, SNAP_SCREEN_RADIUS};

        if !*shared.snap_enabled {
            self.current_snap = None;
            return point;
        }

        let layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => { self.current_snap = None; return point; }
        };
        let time = *shared.playback_time;

        let dcel = match shared.action_executor.document().get_layer(&layer_id) {
            Some(AnyLayer::Vector(vl)) => vl.dcel_at_time(time),
            _ => None,
        };

        let result = dcel.and_then(|dcel| {
            let config = SnapConfig::from_screen_radius(SNAP_SCREEN_RADIUS, self.zoom as f64);
            snap::find_snap_target(dcel, point, &config, &SnapExclusion::default())
        });

        self.current_snap = result;
        result.map(|r| r.position).unwrap_or(point)
    }

    fn handle_rectangle_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shift_held: bool,
        ctrl_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        let is_vector = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Vector(_)));
        if !is_raster && !is_vector { return; }

        let point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

        // Mouse down: start creating rectangle (clears any previous preview)
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            *shared.tool_state = ToolState::CreatingRectangle {
                start_point: point,
                current_point: point,
                centered: ctrl_held,
                constrain_square: shift_held,
            };
        }

        // Mouse drag: update rectangle
        if self.rsp_dragged(response) {
            if let ToolState::CreatingRectangle { start_point, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingRectangle {
                    start_point: *start_point,
                    current_point: point,
                    centered: ctrl_held,
                    constrain_square: shift_held,
                };
            }
        }

        // Mouse up: create the rectangle shape
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::CreatingRectangle { .. })) {
            if let ToolState::CreatingRectangle { start_point, current_point, centered, constrain_square } = shared.tool_state.clone() {
                // Calculate rectangle bounds in world space
                let (min_x, min_y, max_x, max_y) = if centered {
                    // Centered mode: start_point is center
                    let dx = current_point.x - start_point.x;
                    let dy = current_point.y - start_point.y;

                    let (half_w, half_h) = if constrain_square {
                        let half = dx.abs().max(dy.abs());
                        (half, half)
                    } else {
                        (dx.abs(), dy.abs())
                    };

                    (start_point.x - half_w, start_point.y - half_h,
                     start_point.x + half_w, start_point.y + half_h)
                } else {
                    // Corner mode: start_point is corner
                    let mut mn_x = start_point.x.min(current_point.x);
                    let mut mn_y = start_point.y.min(current_point.y);
                    let mut mx_x = start_point.x.max(current_point.x);
                    let mut mx_y = start_point.y.max(current_point.y);

                    if constrain_square {
                        let w = mx_x - mn_x;
                        let h = mx_y - mn_y;
                        let size = w.max(h);

                        if current_point.x > start_point.x {
                            mx_x = mn_x + size;
                        } else {
                            mn_x = mx_x - size;
                        }

                        if current_point.y > start_point.y {
                            mx_y = mn_y + size;
                        } else {
                            mn_y = mx_y - size;
                        }
                    }

                    (mn_x, mn_y, mx_x, mx_y)
                };

                let width = max_x - min_x;
                let height = max_y - min_y;

                // Only create shape if rectangle has non-zero size
                if width > 1.0 && height > 1.0 {
                    if is_raster {
                        let sc = *shared.stroke_color;
                        let fc = *shared.fill_color;
                        let fill_en = *shared.fill_enabled;
                        let thickness = *shared.stroke_width as f32;
                        // Subtract 0.5 to align with Vello's pixel-center convention
                        // (ImageBrush displays pixel (px,py) centered at world (px+0.5, py+0.5))
                        let (x0, y0, x1, y1) = (min_x as f32 - 0.5, min_y as f32 - 0.5, max_x as f32 - 0.5, max_y as f32 - 0.5);
                        let stroke_rgba = [sc.r(), sc.g(), sc.b(), sc.a()];
                        let fill_rgba = fill_en.then(|| [fc.r(), fc.g(), fc.b(), fc.a()]);
                        Self::apply_raster_pixel_edit(shared, active_layer_id, "Draw rectangle", |pixels, w, h| {
                            lightningbeam_core::raster_draw::draw_rect(
                                pixels, w, h, x0, y0, x1, y1,
                                Some(stroke_rgba), fill_rgba, thickness,
                            );
                        });
                    } else {
                        use lightningbeam_core::shape::{ShapeColor, StrokeStyle};
                        use lightningbeam_core::actions::AddShapeAction;

                        let path = Self::create_rectangle_path(min_x, min_y, max_x, max_y);
                        let fill_color = if *shared.fill_enabled {
                            Some(ShapeColor::from_egui(*shared.fill_color))
                        } else {
                            None
                        };
                        let action = AddShapeAction::new(
                            active_layer_id,
                            *shared.playback_time,
                            path,
                            Some(StrokeStyle { width: *shared.stroke_width, ..Default::default() }),
                            Some(ShapeColor::from_egui(*shared.stroke_color)),
                            fill_color,
                            true, // closed
                        ).with_description("Add rectangle");
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                    // Clear tool state to stop preview rendering
                    *shared.tool_state = ToolState::Idle;
                }
            }
        }
    }

    fn handle_ellipse_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shift_held: bool,
        ctrl_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        let is_vector = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Vector(_)));
        if !is_raster && !is_vector { return; }

        let point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

        // Mouse down: start creating ellipse (clears any previous preview)
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            *shared.tool_state = ToolState::CreatingEllipse {
                start_point: point,
                current_point: point,
                corner_mode: !ctrl_held,  // Inverted: Ctrl = centered (like rectangle)
                constrain_circle: shift_held,
            };
        }

        // Mouse drag: update ellipse
        if self.rsp_dragged(response) {
            if let ToolState::CreatingEllipse { start_point, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingEllipse {
                    start_point: *start_point,
                    current_point: point,
                    corner_mode: !ctrl_held,  // Inverted: Ctrl = centered (like rectangle)
                    constrain_circle: shift_held,
                };
            }
        }

        // Mouse up: create the ellipse shape
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::CreatingEllipse { .. })) {
            if let ToolState::CreatingEllipse { start_point, current_point, corner_mode, constrain_circle } = shared.tool_state.clone() {
                // Calculate ellipse parameters based on mode
                // Note: corner_mode is true when Ctrl is NOT held (inverted for consistency with rectangle)
                let (rx, ry, position) = if corner_mode {
                    // Corner mode (default): start_point is corner of bounding box
                    let min_x = start_point.x.min(current_point.x);
                    let min_y = start_point.y.min(current_point.y);
                    let max_x = start_point.x.max(current_point.x);
                    let max_y = start_point.y.max(current_point.y);

                    let width = max_x - min_x;
                    let height = max_y - min_y;

                    let (rx, ry) = if constrain_circle {
                        let radius = width.max(height) / 2.0;
                        (radius, radius)
                    } else {
                        (width / 2.0, height / 2.0)
                    };

                    let position = Point::new(min_x + rx, min_y + ry);

                    (rx, ry, position)
                } else {
                    // Center mode (Ctrl held): start_point is center
                    let dx = (current_point.x - start_point.x).abs();
                    let dy = (current_point.y - start_point.y).abs();

                    let (rx, ry) = if constrain_circle {
                        let radius = (dx * dx + dy * dy).sqrt();
                        (radius, radius)
                    } else {
                        (dx, dy)
                    };

                    (rx, ry, start_point)
                };

                // Only create shape if ellipse has non-zero size
                if rx > 1.0 && ry > 1.0 {
                    if is_raster {
                        let sc = *shared.stroke_color;
                        let fc = *shared.fill_color;
                        let fill_en = *shared.fill_enabled;
                        let thickness = *shared.stroke_width as f32;
                        let (cx, cy) = (position.x as f32 - 0.5, position.y as f32 - 0.5);
                        let (erx, ery) = (rx as f32, ry as f32);
                        let stroke_rgba = [sc.r(), sc.g(), sc.b(), sc.a()];
                        let fill_rgba = fill_en.then(|| [fc.r(), fc.g(), fc.b(), fc.a()]);
                        Self::apply_raster_pixel_edit(shared, active_layer_id, "Draw ellipse", |pixels, w, h| {
                            lightningbeam_core::raster_draw::draw_ellipse(
                                pixels, w, h, cx, cy, erx, ery,
                                Some(stroke_rgba), fill_rgba, thickness,
                            );
                        });
                    } else {
                        use lightningbeam_core::shape::{ShapeColor, StrokeStyle};
                        use lightningbeam_core::actions::AddShapeAction;

                        let path = Self::create_ellipse_path(position.x, position.y, rx, ry);
                        let fill_color = if *shared.fill_enabled {
                            Some(ShapeColor::from_egui(*shared.fill_color))
                        } else {
                            None
                        };
                        let action = AddShapeAction::new(
                            active_layer_id,
                            *shared.playback_time,
                            path,
                            Some(StrokeStyle { width: *shared.stroke_width, ..Default::default() }),
                            Some(ShapeColor::from_egui(*shared.stroke_color)),
                            fill_color,
                            true, // closed
                        ).with_description("Add ellipse");
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                    // Clear tool state to stop preview rendering
                    *shared.tool_state = ToolState::Idle;
                }
            }
        }
    }

    fn handle_line_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shift_held: bool,
        _ctrl_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        let is_vector = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Vector(_)));
        if !is_raster && !is_vector { return; }

        let mut point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

        // Shift: snap to 45° angle increments (raster; also applied to vector for consistency).
        if shift_held {
            if let ToolState::CreatingLine { start_point, .. } = shared.tool_state {
                let dx = point.x - start_point.x;
                let dy = point.y - start_point.y;
                let len = (dx * dx + dy * dy).sqrt();
                let angle = (dy as f32).atan2(dx as f32);
                let snapped = (angle / (std::f32::consts::PI / 4.0)).round()
                    * (std::f32::consts::PI / 4.0);
                point = Point::new(
                    start_point.x + len * snapped.cos() as f64,
                    start_point.y + len * snapped.sin() as f64,
                );
            }
        }

        // Mouse down: start creating line
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            *shared.tool_state = ToolState::CreatingLine {
                start_point: point,
                current_point: point,
            };
        }

        // Mouse drag: update line
        if self.rsp_dragged(response) {
            if let ToolState::CreatingLine { start_point, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingLine {
                    start_point: *start_point,
                    current_point: point,
                };
            }
        }

        // Mouse up: create the line shape
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::CreatingLine { .. })) {
            if let ToolState::CreatingLine { start_point, current_point } = shared.tool_state.clone() {
                let dx = current_point.x - start_point.x;
                let dy = current_point.y - start_point.y;
                let length = (dx * dx + dy * dy).sqrt();

                if length > 1.0 {
                    if is_raster {
                        let sc = *shared.stroke_color;
                        let thickness = *shared.stroke_width as f32;
                        let (ax, ay) = (start_point.x as f32 - 0.5, start_point.y as f32 - 0.5);
                        let (bx, by) = (current_point.x as f32 - 0.5, current_point.y as f32 - 0.5);
                        let stroke_rgba = [sc.r(), sc.g(), sc.b(), sc.a()];
                        Self::apply_raster_pixel_edit(shared, active_layer_id, "Draw line", |pixels, w, h| {
                            lightningbeam_core::raster_draw::draw_line(
                                pixels, w, h, ax, ay, bx, by, stroke_rgba, thickness,
                            );
                        });
                    } else {
                        use lightningbeam_core::shape::{ShapeColor, StrokeStyle};
                        use lightningbeam_core::actions::AddShapeAction;

                        let path = Self::create_line_path(start_point, current_point);
                        let action = AddShapeAction::new(
                            active_layer_id,
                            *shared.playback_time,
                            path,
                            Some(StrokeStyle { width: *shared.stroke_width, ..Default::default() }),
                            Some(ShapeColor::from_egui(*shared.stroke_color)),
                            None, // no fill for lines
                            false, // not closed
                        ).with_description("Add line");
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                    *shared.tool_state = ToolState::Idle;
                }
            }
        }
    }

    fn handle_polygon_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        _shift_held: bool,
        _ctrl_held: bool,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        let is_raster = matches!(active_layer, AnyLayer::Raster(_));
        let is_vector = matches!(active_layer, AnyLayer::Vector(_));
        if !is_raster && !is_vector {
            return;
        }

        let num_sides = *shared.polygon_sides;
        let point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

        // Mouse down: start creating polygon (center point)
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            *shared.tool_state = ToolState::CreatingPolygon {
                center: point,
                current_point: point,
                num_sides,
            };
        }

        // Mouse drag: update polygon radius
        if self.rsp_dragged(response) {
            if let ToolState::CreatingPolygon { center, num_sides, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingPolygon {
                    center: *center,
                    current_point: point,
                    num_sides: *num_sides,
                };
            }
        }

        // Mouse up: create the polygon shape
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::CreatingPolygon { .. })) {
            if let ToolState::CreatingPolygon { center, current_point, num_sides } = shared.tool_state.clone() {
                // Calculate radius
                let dx = current_point.x - center.x;
                let dy = current_point.y - center.y;
                let radius = (dx * dx + dy * dy).sqrt();

                // Only create shape if polygon has reasonable size
                if radius > 5.0 {
                    if is_raster {
                        use lightningbeam_core::raster_draw;
                        use std::f64::consts::TAU;

                        let cx = center.x as f32 - 0.5;
                        let cy = center.y as f32 - 0.5;
                        let r = radius as f32;
                        let n = num_sides as usize;
                        let vertices: Vec<(f32, f32)> = (0..n).map(|i| {
                            let angle = (i as f64 / n as f64) * TAU - std::f64::consts::FRAC_PI_2;
                            (cx + r * angle.cos() as f32, cy + r * angle.sin() as f32)
                        }).collect();

                        let stroke_color = shared.stroke_color.to_array();
                        let stroke_rgba = [stroke_color[0], stroke_color[1], stroke_color[2], stroke_color[3]];
                        let fill_rgba = if *shared.fill_enabled {
                            let fc = shared.fill_color.to_array();
                            Some([fc[0], fc[1], fc[2], fc[3]])
                        } else {
                            None
                        };
                        let thickness = *shared.stroke_width as f32;

                        let _ = Self::apply_raster_pixel_edit(shared, active_layer_id, "Add polygon", |pixels, w, h| {
                            raster_draw::draw_polygon(pixels, w, h, &vertices, Some(stroke_rgba), fill_rgba, thickness);
                        });
                    } else {
                        use lightningbeam_core::shape::{ShapeColor, StrokeStyle};
                        use lightningbeam_core::actions::AddShapeAction;

                        let path = Self::create_polygon_path(center, num_sides, radius);

                        let fill_color = if *shared.fill_enabled {
                            Some(ShapeColor::from_egui(*shared.fill_color))
                        } else {
                            None
                        };

                        let action = AddShapeAction::new(
                            active_layer_id,
                            *shared.playback_time,
                            path,
                            Some(StrokeStyle { width: *shared.stroke_width, ..Default::default() }),
                            Some(ShapeColor::from_egui(*shared.stroke_color)),
                            fill_color,
                            true, // closed
                        ).with_description("Add polygon");
                        let _ = shared.action_executor.execute(Box::new(action));
                    }

                    // Clear tool state to stop preview rendering
                    *shared.tool_state = ToolState::Idle;
                }
            }
        }
    }

    fn handle_eyedropper_tool(
        &mut self,
        _ui: &mut egui::Ui,
        response: &egui::Response,
        screen_pos: egui::Pos2,
        shared: &mut SharedPaneState,
    ) {
        // On click, store the screen position and color mode for sampling
        if self.rsp_clicked(response) {
            self.pending_eyedropper_sample = Some((screen_pos, *shared.active_color_mode));
        }
    }

    fn handle_region_select_tool(
        &mut self,
        _ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::{ToolState, RegionSelectMode};
        use lightningbeam_core::region_select;
        use vello::kurbo::{Point, Rect as KurboRect};

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Mouse down: start region selection
        if self.rsp_drag_started(response) {
            // Revert any existing uncommitted region selection, and clear the
            // regular selection so both selection modes don't coexist.
            Self::revert_region_selection_static(shared);
            shared.selection.clear();

            match *shared.region_select_mode {
                RegionSelectMode::Rectangle => {
                    *shared.tool_state = ToolState::RegionSelectingRect {
                        start: point,
                        current: point,
                    };
                }
                RegionSelectMode::Lasso => {
                    *shared.tool_state = ToolState::RegionSelectingLasso {
                        points: vec![point],
                    };
                }
            }
        }

        // Mouse drag: update region
        if self.rsp_dragged(response) {
            match shared.tool_state {
                ToolState::RegionSelectingRect { ref start, .. } => {
                    let start = *start;
                    *shared.tool_state = ToolState::RegionSelectingRect {
                        start,
                        current: point,
                    };
                }
                ToolState::RegionSelectingLasso { ref mut points } => {
                    if let Some(last) = points.last() {
                        if (point.x - last.x).hypot(point.y - last.y) > 3.0 {
                            points.push(point);
                        }
                    }
                }
                _ => {}
            }
        }

        // Mouse up: execute region selection
        if self.rsp_drag_stopped(response) {
            let region_path = match &*shared.tool_state {
                ToolState::RegionSelectingRect { start, current } => {
                    let min_x = start.x.min(current.x);
                    let min_y = start.y.min(current.y);
                    let max_x = start.x.max(current.x);
                    let max_y = start.y.max(current.y);
                    // Ignore tiny drags
                    if (max_x - min_x) < 2.0 || (max_y - min_y) < 2.0 {
                        *shared.tool_state = ToolState::Idle;
                        return;
                    }
                    Some(region_select::rect_to_path(KurboRect::new(min_x, min_y, max_x, max_y)))
                }
                ToolState::RegionSelectingLasso { points } => {
                    if points.len() >= 3 {
                        Some(region_select::lasso_to_path(points))
                    } else {
                        None
                    }
                }
                _ => None,
            };

            *shared.tool_state = ToolState::Idle;

            if let Some(region_path) = region_path {
                Self::execute_region_select(shared, region_path, active_layer_id);
            }
        }
    }

    /// Execute region selection: snapshot DCEL, insert region boundary, extract inside geometry
    fn execute_region_select(
        shared: &mut SharedPaneState,
        region_path: vello::kurbo::BezPath,
        layer_id: uuid::Uuid,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::region_select::line_to_cubic;
        use vello::kurbo::Line;

        let time = *shared.playback_time;

        // Get mutable DCEL and snapshot it before insertion
        let document = shared.action_executor.document_mut();
        let dcel = match document.get_layer_mut(&layer_id) {
            Some(AnyLayer::Vector(vl)) => match vl.dcel_at_time_mut(time) {
                Some(d) => d,
                None => return,
            },
            _ => return,
        };

        let snapshot = dcel.clone();

        // Convert region path line segments to CubicBez for insert_stroke
        let segments: Vec<_> = {
            let mut segs = Vec::new();
            let mut current = vello::kurbo::Point::ZERO;
            let mut subpath_start = vello::kurbo::Point::ZERO;
            for el in region_path.elements() {
                match *el {
                    vello::kurbo::PathEl::MoveTo(p) => {
                        current = p;
                        subpath_start = p;
                    }
                    vello::kurbo::PathEl::LineTo(p) => {
                        segs.push(line_to_cubic(&Line::new(current, p)));
                        current = p;
                    }
                    vello::kurbo::PathEl::ClosePath => {
                        if current.distance(subpath_start) > 1e-10 {
                            segs.push(line_to_cubic(&Line::new(current, subpath_start)));
                        }
                        current = subpath_start;
                    }
                    vello::kurbo::PathEl::CurveTo(p1, p2, p3) => {
                        segs.push(vello::kurbo::CubicBez::new(current, p1, p2, p3));
                        current = p3;
                    }
                    vello::kurbo::PathEl::QuadTo(_p1, p2) => {
                        segs.push(line_to_cubic(&Line::new(current, p2)));
                        current = p2;
                    }
                }
            }
            segs
        };

        if segments.is_empty() {
            return;
        }

        // Capture DCEL snapshot + region path for crash diagnosis (debug builds only)
        #[cfg(debug_assertions)]
        {
            use vello::kurbo::PathEl;
            let path_elems: Vec<serde_json::Value> = region_path.elements().iter().map(|el| match el {
                PathEl::MoveTo(p) => serde_json::json!({"type": "M", "x": p.x, "y": p.y}),
                PathEl::LineTo(p) => serde_json::json!({"type": "L", "x": p.x, "y": p.y}),
                PathEl::QuadTo(p1, p2) => serde_json::json!({"type": "Q", "x1": p1.x, "y1": p1.y, "x2": p2.x, "y2": p2.y}),
                PathEl::CurveTo(p1, p2, p3) => serde_json::json!({"type": "C", "x1": p1.x, "y1": p1.y, "x2": p2.x, "y2": p2.y, "x3": p3.x, "y3": p3.y}),
                PathEl::ClosePath => serde_json::json!({"type": "Z"}),
            }).collect();
            let geom = serde_json::json!({
                "region_path": path_elems,
                "dcel_snapshot": serde_json::to_value(&snapshot).unwrap_or(serde_json::Value::Null),
            });
            shared.test_mode.set_pending_geometry(geom);
        }

        // Insert region boundary as invisible edges (no stroke style/color)
        let stroke_result = dcel.insert_stroke(&segments, None, None, 1.0);
        let boundary_verts: Vec<_> = stroke_result.new_vertices.clone();
        let region_edge_ids: Vec<_> = stroke_result.new_edges.clone();

        // Extract the inside portion; self (dcel) keeps the outside + boundary.
        let mut selected_dcel = dcel.extract_region(&region_path, &boundary_verts);

        // Propagate fills ONLY on the extracted DCEL. The remainder (dcel) already
        // has correct fills from the original data — its filled faces (e.g., the
        // L-shaped remainder) keep their fill, and merged faces from edge removal
        // correctly have no fill. Running propagate_fills on the remainder would
        // incorrectly add fill to merged faces that span filled and unfilled areas.
        selected_dcel.propagate_fills(&snapshot, &region_path, &boundary_verts);

        // Check if the extracted DCEL has any visible content
        let has_visible = selected_dcel.edges.iter().any(|e| !e.deleted && (e.stroke_style.is_some() || e.stroke_color.is_some()))
            || selected_dcel.faces.iter().enumerate().any(|(i, f)| !f.deleted && i > 0 && (f.fill_color.is_some() || f.image_fill.is_some()));

        if !has_visible {
            // Nothing visible inside — restore snapshot and bail
            *dcel = snapshot;
            #[cfg(debug_assertions)]
            shared.test_mode.clear_pending_geometry();
            return;
        }

        // Compute inside_vertices: non-deleted verts in selected_dcel that aren't boundary.
        let inside_vertices: Vec<_> = selected_dcel
            .vertices
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                if v.deleted { return None; }
                let vid = lightningbeam_core::dcel::VertexId(i as u32);
                if !boundary_verts.contains(&vid) { Some(vid) } else { None }
            })
            .collect();

        let action_epoch = shared.action_executor.epoch();

        shared.selection.clear();

        // Populate global selection with the faces from the extracted DCEL so
        // property panels and other tools can see what is selected. We add face
        // IDs only (no boundary edges/vertices) because the boundary geometry
        // lives in selected_dcel, not in the live DCEL.
        for (i, face) in selected_dcel.faces.iter().enumerate() {
            if face.deleted || i == 0 { continue; }
            if face.fill_color.is_some() || face.image_fill.is_some() {
                shared.selection.select_face_id_only(lightningbeam_core::dcel::FaceId(i as u32));
            }
        }

        // Store the extracted DCEL as the clipboard-ready vector subgraph.
        // This allows clipboard_copy_selection to serialize it without needing
        // to re-extract geometry from the live DCEL.
        shared.selection.vector_subgraph = Some(selected_dcel.clone());

        // Store region selection state with extracted DCEL
        *shared.region_selection = Some(lightningbeam_core::selection::RegionSelection {
            region_path,
            layer_id,
            time,
            dcel_snapshot: snapshot,
            selected_dcel,
            transform: vello::kurbo::Affine::IDENTITY,
            committed: false,
            inside_vertices,
            boundary_vertices: boundary_verts,
            region_edge_ids,
            action_epoch_at_selection: action_epoch,
        });

        #[cfg(debug_assertions)]
        shared.test_mode.clear_pending_geometry();
    }

    /// Revert an uncommitted region selection, restoring the DCEL from snapshot
    fn revert_region_selection_static(shared: &mut SharedPaneState) {
        use lightningbeam_core::layer::AnyLayer;

        let region_sel = match shared.region_selection.take() {
            Some(rs) => rs,
            None => return,
        };

        if region_sel.committed {
            // Already committed via action system, nothing to revert
            return;
        }

        let no_actions_taken =
            shared.action_executor.epoch() == region_sel.action_epoch_at_selection;

        let doc = shared.action_executor.document_mut();
        if let Some(AnyLayer::Vector(vl)) = doc.get_layer_mut(&region_sel.layer_id) {
            if let Some(dcel) = vl.dcel_at_time_mut(region_sel.time) {
                if no_actions_taken {
                    // Nothing changed: restore snapshot cleanly (undo boundary insertion)
                    *dcel = region_sel.dcel_snapshot;
                } else {
                    // Actions were applied to the selection: merge selected_dcel back
                    let mut merged = region_sel.dcel_snapshot;
                    merged.merge_back_from_selected(
                        &region_sel.selected_dcel,
                        &region_sel.inside_vertices,
                        &region_sel.boundary_vertices,
                        &region_sel.region_edge_ids,
                    );
                    *dcel = merged;
                }
            }
        }

        shared.selection.clear_dcel_selection();
    }

    /// Create a rectangle path centered at origin (easier for curve editing later)
    fn create_rectangle_path(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};

        let mut path = BezPath::new();
        path.move_to(Point::new(min_x, min_y));
        path.line_to(Point::new(max_x, min_y));
        path.line_to(Point::new(max_x, max_y));
        path.line_to(Point::new(min_x, max_y));
        path.close_path();
        path
    }

    /// Create an ellipse path in world space from bezier curves.
    fn create_ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};

        const KAPPA: f64 = 0.5522847498;

        let kx = rx * KAPPA;
        let ky = ry * KAPPA;

        let mut path = BezPath::new();

        // Start at right point
        path.move_to(Point::new(cx + rx, cy));

        // Top-right quadrant (to top point)
        path.curve_to(
            Point::new(cx + rx, cy - ky),
            Point::new(cx + kx, cy - ry),
            Point::new(cx, cy - ry),
        );

        // Top-left quadrant (to left point)
        path.curve_to(
            Point::new(cx - kx, cy - ry),
            Point::new(cx - rx, cy - ky),
            Point::new(cx - rx, cy),
        );

        // Bottom-left quadrant (to bottom point)
        path.curve_to(
            Point::new(cx - rx, cy + ky),
            Point::new(cx - kx, cy + ry),
            Point::new(cx, cy + ry),
        );

        // Bottom-right quadrant (back to right point)
        path.curve_to(
            Point::new(cx + kx, cy + ry),
            Point::new(cx + rx, cy + ky),
            Point::new(cx + rx, cy),
        );

        path.close_path();
        path
    }

    /// Create a line path in world space from start to end.
    fn create_line_path(start: vello::kurbo::Point, end: vello::kurbo::Point) -> vello::kurbo::BezPath {
        use vello::kurbo::BezPath;

        let mut path = BezPath::new();
        path.move_to(start);
        path.line_to(end);
        path
    }

    /// Create a regular polygon path in world space.
    fn create_polygon_path(center: vello::kurbo::Point, num_sides: u32, radius: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};
        use std::f64::consts::PI;

        let mut path = BezPath::new();

        if num_sides < 3 {
            return path;
        }

        let angle_step = 2.0 * PI / num_sides as f64;
        let start_angle = -PI / 2.0;

        let first_x = center.x + radius * start_angle.cos();
        let first_y = center.y + radius * start_angle.sin();
        path.move_to(Point::new(first_x, first_y));

        for i in 1..num_sides {
            let angle = start_angle + angle_step * i as f64;
            let x = center.x + radius * angle.cos();
            let y = center.y + radius * angle.sin();
            path.line_to(Point::new(x, y));
        }

        path.close_path();
        path
    }

    fn handle_draw_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        // Check if we have an active vector layer
        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start drawing path (snap the first point)
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            let snapped_start = self.snap_point(point, shared);
            *shared.tool_state = ToolState::DrawingPath {
                points: vec![snapped_start],
                simplify_mode: *shared.draw_simplify_mode,
            };
        }

        // Mouse drag: add points to path (no snapping for intermediate freehand points)
        if self.rsp_dragged(response) {
            self.current_snap = None;
            if let ToolState::DrawingPath { points, simplify_mode: _ } = &mut *shared.tool_state {
                // Only add point if it's far enough from the last point (reduce noise)
                const MIN_POINT_DISTANCE: f64 = 2.0;

                if let Some(last_point) = points.last() {
                    let dist_sq = (point.x - last_point.x).powi(2) + (point.y - last_point.y).powi(2);
                    if dist_sq > MIN_POINT_DISTANCE * MIN_POINT_DISTANCE {
                        points.push(point);
                    }
                } else {
                    points.push(point);
                }
            }
        }

        // Mouse up: snap the last point, then complete the path and create shape
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::DrawingPath { .. })) {
            // Snap the final point (extract last point first to avoid borrow conflict)
            let last_point = if let ToolState::DrawingPath { points, .. } = &*shared.tool_state {
                if points.len() >= 2 { Some(*points.last().unwrap()) } else { None }
            } else {
                None
            };
            if let Some(last) = last_point {
                let snapped_end = self.snap_point(last, shared);
                if let ToolState::DrawingPath { points, .. } = &mut *shared.tool_state {
                    *points.last_mut().unwrap() = snapped_end;
                }
            }
            self.current_snap = None;
            if let ToolState::DrawingPath { points, simplify_mode } = shared.tool_state.clone() {
                // Only create shape if we have enough points
                if points.len() >= 2 {
                    use lightningbeam_core::path_fitting::{
                        simplify_rdp, fit_bezier_curves, RdpConfig, SchneiderConfig,
                    };
                    use lightningbeam_core::shape::ShapeColor;
                    use lightningbeam_core::actions::AddShapeAction;

                    // Convert points to the appropriate path based on simplify mode
                    let path = match simplify_mode {
                        lightningbeam_core::tool::SimplifyMode::Corners => {
                            // RDP simplification first, then convert to bezier
                            let config = RdpConfig {
                                tolerance: *shared.rdp_tolerance,
                                highest_quality: false,
                            };
                            let simplified = simplify_rdp(&points, config);

                            // Convert simplified points to smooth bezier with mid-point curves
                            fit_bezier_curves(&simplified, SchneiderConfig {
                                max_error: *shared.schneider_max_error
                            })
                        }
                        lightningbeam_core::tool::SimplifyMode::Smooth => {
                            // Direct Schneider curve fitting for smooth curves
                            let config = SchneiderConfig {
                                max_error: *shared.schneider_max_error,
                            };
                            fit_bezier_curves(&points, config)
                        }
                        lightningbeam_core::tool::SimplifyMode::Verbatim => {
                            // Use raw points as line segments
                            let mut path = vello::kurbo::BezPath::new();
                            if let Some(first) = points.first() {
                                path.move_to(*first);
                                for point in &points[1..] {
                                    path.line_to(*point);
                                }
                            }
                            path
                        }
                    };

                    // Only create shape if path is not empty
                    if !path.is_empty() {
                        use lightningbeam_core::shape::StrokeStyle;
                        // Path is already in world space from mouse coordinates

                        let fill_color = if *shared.fill_enabled {
                            Some(ShapeColor::from_egui(*shared.fill_color))
                        } else {
                            None
                        };

                        let action = AddShapeAction::new(
                            active_layer_id,
                            *shared.playback_time,
                            path,
                            Some(StrokeStyle { width: *shared.stroke_width, ..Default::default() }),
                            Some(ShapeColor::from_egui(*shared.stroke_color)),
                            fill_color,
                            false, // drawn paths are open strokes
                        ).with_description("Draw path");
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                }

                // Clear tool state to stop preview rendering
                *shared.tool_state = ToolState::Idle;
            }
        }
    }

    /// Handle raster stroke tool input (Draw/Erase/Smudge on a raster layer).
    ///
    /// Computes GPU dab lists for each drag event and stores them in
    /// Commit any live floating raster selection into `raw_pixels` right now,
    /// synchronously.  Must be called before capturing `buffer_before` for a
    /// new brush stroke or before starting a new marquee/lasso drag, so the
    /// GPU canvas and undo snapshots are based on the fully-composited canvas.
    ///
    /// Unlike the async `commit_raster_floating_if_any` flag (used for tool
    /// switches detected in main.rs), this path is needed for in-canvas
    /// interactions where the commit must happen *before* other per-frame work.
    fn commit_raster_floating_now(shared: &mut SharedPaneState) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterStrokeAction;
        use lightningbeam_core::selection::RasterFloatingSelection;

        let Some(float): Option<RasterFloatingSelection> =
            shared.selection.raster_floating.take()
        else {
            return;
        };
        let sel = shared.selection.raster_selection.take();

        let document = shared.action_executor.document_mut();
        let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&float.layer_id) else {
            return;
        };
        let Some(kf) = rl.keyframe_at_mut(float.time) else { return };

        // Ensure the canvas buffer is allocated (empty Vec = blank transparent canvas).
        let expected = (kf.width * kf.height * 4) as usize;
        if kf.raw_pixels.len() != expected {
            kf.raw_pixels.resize(expected, 0);
        }

        // Porter-Duff "src over dst" for sRGB-encoded premultiplied pixels,
        // masked by the selection C when present.
        for row in 0..float.height {
            let dy = float.y + row as i32;
            if dy < 0 || dy >= kf.height as i32 { continue; }
            for col in 0..float.width {
                let dx = float.x + col as i32;
                if dx < 0 || dx >= kf.width as i32 { continue; }
                // Apply selection mask C (if selection exists, only composite where inside)
                if let Some(ref s) = sel {
                    if !s.contains_pixel(dx, dy) { continue; }
                }
                let si = ((row * float.width + col) * 4) as usize;
                let di = ((dy as u32 * kf.width + dx as u32) * 4) as usize;
                let sa = float.pixels[si + 3] as u32;
                if sa == 0 { continue; }
                let da = kf.raw_pixels[di + 3] as u32;
                let out_a = sa + da * (255 - sa) / 255;
                kf.raw_pixels[di + 3] = out_a as u8;
                if out_a > 0 {
                    for c in 0..3 {
                        let v = float.pixels[si + c] as u32 * 255
                            + kf.raw_pixels[di + c] as u32 * (255 - sa);
                        kf.raw_pixels[di + c] = (v / 255).min(255) as u8;
                    }
                }
            }
        }

        let canvas_after = kf.raw_pixels.clone();
        let (w, h) = (kf.width, kf.height);
        let action = RasterStrokeAction::new(
            float.layer_id, float.time,
            std::sync::Arc::try_unwrap(float.canvas_before).unwrap_or_else(|a| (*a).clone()),
            canvas_after,
            w, h,
        );
        if let Err(e) = shared.action_executor.execute(Box::new(action)) {
            eprintln!("commit_raster_floating_now: {e}");
        }
    }

    /// Lift the pixels enclosed by the current `raster_selection` into a
    /// `RasterFloatingSelection`, punching a transparent hole in `raw_pixels`.
    ///
    /// Call this immediately after a marquee / lasso selection is finalized so
    /// that all downstream operations (drag-move, copy, cut, stroke-masking)
    /// see a consistent `raster_floating` whenever a selection is active.
    /// Build an R8 mask buffer (0 = outside, 255 = inside) from a selection.
    fn build_selection_mask(
        sel: &lightningbeam_core::selection::RasterSelection,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut mask = vec![0u8; (width * height) as usize];
        let (x0, y0, x1, y1) = sel.bounding_rect();
        let bx0 = x0.max(0) as u32;
        let by0 = y0.max(0) as u32;
        let bx1 = (x1 as u32).min(width);
        let by1 = (y1 as u32).min(height);
        for y in by0..by1 {
            for x in bx0..bx1 {
                if sel.contains_pixel(x as i32, y as i32) {
                    mask[(y * width + x) as usize] = 255;
                }
            }
        }
        mask
    }

    /// Build an R8 mask buffer for the float canvas (0 = outside selection, 255 = inside).
    /// Coordinates are in float-local space: pixel (fx, fy) corresponds to document pixel
    /// (float_x+fx, float_y+fy).
    fn build_float_mask(
        sel: &lightningbeam_core::selection::RasterSelection,
        float_x: i32, float_y: i32,
        float_w: u32, float_h: u32,
    ) -> Vec<u8> {
        let mut mask = vec![0u8; (float_w * float_h) as usize];
        let (x0, y0, x1, y1) = sel.bounding_rect();
        let bx0 = (x0 - float_x).max(0) as u32;
        let by0 = (y0 - float_y).max(0) as u32;
        let bx1 = ((x1 - float_x) as u32).min(float_w);
        let by1 = ((y1 - float_y) as u32).min(float_h);
        for fy in by0..by1 {
            for fx in bx0..bx1 {
                if sel.contains_pixel(float_x + fx as i32, float_y + fy as i32) {
                    mask[(fy * float_w + fx) as usize] = 255;
                }
            }
        }
        mask
    }

    /// Allocate the three A/B/C GPU canvases and build a [`crate::raster_tool::RasterWorkspace`]
    /// for a new raster tool operation.
    ///
    /// Called on **mousedown** before any tool-specific code runs.  The returned
    /// [`crate::raster_tool::WorkspaceInitPacket`] must be stored in `self.pending_workspace_init`
    /// so that [`VelloCallback::prepare`] can create the GPU textures on the first frame.
    ///
    /// - If a floating selection is active, the workspace targets it (Float path).
    /// - Otherwise, any lingering float is committed first, then the active raster
    ///   layer's keyframe becomes the workspace source (Layer path).
    ///
    /// Returns `None` when there is no raster target (no active layer, or the active
    /// layer is not a raster layer).
    fn begin_raster_workspace(
        shared: &mut SharedPaneState,
    ) -> Option<(crate::raster_tool::RasterWorkspace, crate::raster_tool::WorkspaceInitPacket)> {
        use crate::raster_tool::{WorkspaceInitPacket, WorkspaceSource, RasterWorkspace};
        use lightningbeam_core::layer::AnyLayer;

        if let Some(ref float) = shared.selection.raster_floating {
            // ── Float-active path ─────────────────────────────────────────
            // Paint onto the floating selection's existing GPU canvas (A).
            // Do NOT commit the float; it remains active.
            let pixels = if float.pixels.is_empty() {
                vec![0u8; (float.width * float.height * 4) as usize]
            } else {
                (*float.pixels).clone()
            };
            let (w, h, x, y) = (float.width, float.height, float.x, float.y);

            let a_id = uuid::Uuid::new_v4();
            let b_id = uuid::Uuid::new_v4();
            let c_id = uuid::Uuid::new_v4();

            let ws = RasterWorkspace {
                a_canvas_id: a_id,
                b_canvas_id: b_id,
                c_canvas_id: c_id,
                mask_texture: None,
                width: w,
                height: h,
                x,
                y,
                source: WorkspaceSource::Float,
                before_pixels: pixels.clone(),
            };
            let init = WorkspaceInitPacket {
                a_canvas_id: a_id,
                a_pixels: pixels,
                b_canvas_id: b_id,
                c_canvas_id: c_id,
                width: w,
                height: h,
            };
            Some((ws, init))
        } else {
            // ── Layer-active path ─────────────────────────────────────────
            // Commit any lingering float so buffer_before reflects the fully-composited canvas.
            Self::commit_raster_floating_now(shared);

            let layer_id = (*shared.active_layer_id)?;
            let time = *shared.playback_time;

            let (doc_w, doc_h) = {
                let doc = shared.action_executor.document();
                (doc.width as u32, doc.height as u32)
            };

            // Ensure the keyframe exists before reading its ID.
            {
                let doc = shared.action_executor.document_mut();
                if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                    rl.ensure_keyframe_at(time, doc_w, doc_h);
                } else {
                    return None; // not a raster layer
                }
            }

            // Read keyframe id and pixels.
            let (kf_id, w, h, pixels) = {
                let doc = shared.action_executor.document();
                let AnyLayer::Raster(rl) = doc.get_layer(&layer_id)? else { return None };
                let kf = rl.keyframe_at(time)?;
                let pixels = if kf.raw_pixels.is_empty() {
                    vec![0u8; (kf.width * kf.height * 4) as usize]
                } else {
                    kf.raw_pixels.clone()
                };
                (kf.id, kf.width, kf.height, pixels)
            };

            let a_id = uuid::Uuid::new_v4();
            let b_id = uuid::Uuid::new_v4();
            let c_id = uuid::Uuid::new_v4();

            let ws = RasterWorkspace {
                a_canvas_id: a_id,
                b_canvas_id: b_id,
                c_canvas_id: c_id,
                mask_texture: None,
                width: w,
                height: h,
                x: 0,
                y: 0,
                source: WorkspaceSource::Layer {
                    layer_id,
                    time,
                    kf_id,
                    canvas_w: doc_w,
                    canvas_h: doc_h,
                },
                before_pixels: pixels.clone(),
            };
            let init = WorkspaceInitPacket {
                a_canvas_id: a_id,
                a_pixels: pixels,
                b_canvas_id: b_id,
                c_canvas_id: c_id,
                width: w,
                height: h,
            };
            Some((ws, init))
        }
    }

    /// Unified raster stroke handler using the [`crate::raster_tool::RasterTool`] trait.
    ///
    /// Handles all paint-style brush tools (Paint, Pencil, Airbrush, Eraser, etc.).
    /// - **mousedown**: calls `begin_raster_workspace()` + instantiates `BrushRasterTool`.
    /// - **drag**: calls `tool.update()` each frame.
    /// - **mouseup**: calls `tool.finish()`, schedules GPU B-canvas readback if committed.
    fn handle_unified_raster_stroke_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        def: &'static dyn crate::tools::RasterToolDef,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::raster_layer::RasterBlendMode;
        use crate::raster_tool::{BrushRasterTool, RasterTool, WorkspaceSource};

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Only operate on raster layers
        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));
        if !is_raster { return; }

        let blend_mode = def.blend_mode();

        // ----------------------------------------------------------------
        // Mouse down: initialise the workspace and start the tool
        // ----------------------------------------------------------------
        let stroke_start = (self.rsp_primary_pressed(ui) && response.hovered()
                            && self.active_raster_tool.is_none())
                        || (self.rsp_clicked(response) && self.active_raster_tool.is_none());
        if stroke_start {
            // Build brush settings from the tool definition.
            let bp = def.brush_params(shared.raster_settings);
            let (mut b, radius, opacity, hardness, spacing) =
                (bp.base_settings, bp.radius, bp.opacity, bp.hardness, bp.spacing);
            b.radius_log      = radius.ln() - b.pressure_radius_gain * 0.5;
            b.hardness        = hardness;
            b.opaque          = opacity;
            b.dabs_per_radius = spacing;
            if matches!(blend_mode, RasterBlendMode::Smudge) {
                b.dabs_per_actual_radius = 0.0;
                b.smudge_radius_log = shared.raster_settings.smudge_strength;
            }
            if matches!(blend_mode, RasterBlendMode::BlurSharpen) {
                b.dabs_per_actual_radius = 0.0;
            }
            let color = if matches!(blend_mode, RasterBlendMode::Erase) {
                [1.0f32, 1.0, 1.0, 1.0]
            } else {
                let c = if shared.raster_settings.brush_use_fg {
                    *shared.stroke_color
                } else {
                    *shared.fill_color
                };
                let s2l = |v: u8| -> f32 {
                    let f = v as f32 / 255.0;
                    if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) }
                };
                [s2l(c.r()), s2l(c.g()), s2l(c.b()), c.a() as f32 / 255.0]
            };

            if let Some((ws, init)) = Self::begin_raster_workspace(shared) {
                let mut tool = Box::new(BrushRasterTool::new(color, b, blend_mode));
                self.raster_last_compute_time = ui.input(|i| i.time);
                tool.begin(&ws, world_pos, 0.0, shared.raster_settings);
                self.pending_workspace_init = Some(init);
                *shared.tool_state = ToolState::DrawingRasterStroke { points: vec![] };
                self.active_raster_tool = Some((tool, ws));
            }
        }

        // ----------------------------------------------------------------
        // Per-frame update: fires every frame while stroke is active so
        // time-based brushes (airbrush) accumulate dabs even when stationary.
        // ----------------------------------------------------------------
        if self.active_raster_tool.is_some()
            && matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. })
            && !stroke_start
        {
            let current_time = ui.input(|i| i.time);
            let dt = (current_time - self.raster_last_compute_time).clamp(0.0, 0.1) as f32;
            self.raster_last_compute_time = current_time;
            if let Some((ref mut tool, ref ws)) = self.active_raster_tool {
                tool.update(ws, world_pos, dt, shared.raster_settings);
            }
        }

        // Keep egui repainting while a stroke is in progress.
        if matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. }) {
            ui.ctx().request_repaint();
        }

        // ----------------------------------------------------------------
        // Mouse up: finish the tool, trigger readback if needed
        // ----------------------------------------------------------------
        let stroke_end = self.rsp_drag_stopped(response)
            || (self.rsp_any_released(ui)
                && self.active_raster_tool.is_some()
                && matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. }));
        if stroke_end {
            *shared.tool_state = ToolState::Idle;
            if self.active_raster_tool.is_some() {
                let needs_commit = {
                    let (ref mut tool, ref ws) = self.active_raster_tool.as_mut().unwrap();
                    tool.finish(ws)
                };
                if needs_commit {
                    let ws = &self.active_raster_tool.as_ref().unwrap().1;
                    self.painting_float = matches!(ws.source, WorkspaceSource::Float);
                    let (undo_layer_id, undo_time) = match &ws.source {
                        WorkspaceSource::Layer { layer_id, time, .. } => (*layer_id, *time),
                        WorkspaceSource::Float => (uuid::Uuid::nil(), 0.0),
                    };
                    self.pending_undo_before = Some((
                        undo_layer_id, undo_time, ws.width, ws.height,
                        ws.before_pixels.clone(),
                    ));
                    self.pending_tool_readback_b = Some(ws.b_canvas_id);
                    self.active_tool_awaiting_readback = true;
                    // Keep active_raster_tool alive until render_content() consumes the result.
                } else {
                    // No commit (no dabs were placed); discard immediately.
                    if let Some((_, ws)) = self.active_raster_tool.take() {
                        self.pending_canvas_removals.extend(ws.canvas_ids());
                    }
                }
            }
        }
    }

    fn lift_selection_to_float(shared: &mut SharedPaneState) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::selection::RasterFloatingSelection;

        // Clone the selection before any mutable borrows.
        let Some(sel) = shared.selection.raster_selection.clone() else { return };
        let Some(layer_id) = *shared.active_layer_id else { return };
        let time = *shared.playback_time;

        // Commit any existing float first (clears raster_selection — re-set below).
        Self::commit_raster_floating_now(shared);

        let doc = shared.action_executor.document_mut();
        let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) else { return };
        let Some(kf) = rl.keyframe_at_mut(time) else { return };

        let canvas_before = kf.raw_pixels.clone();
        let (x0, y0, x1, y1) = sel.bounding_rect();
        let w = (x1 - x0).max(0) as u32;
        let h = (y1 - y0).max(0) as u32;
        if w == 0 || h == 0 { return; }

        let mut float_pixels = vec![0u8; (w * h * 4) as usize];
        for row in 0..h {
            let sy = y0 + row as i32;
            if sy < 0 || sy >= kf.height as i32 { continue; }
            for col in 0..w {
                let sx = x0 + col as i32;
                if sx < 0 || sx >= kf.width as i32 { continue; }
                if !sel.contains_pixel(sx, sy) { continue; }
                let si = ((sy as u32 * kf.width + sx as u32) * 4) as usize;
                let di = ((row * w + col) * 4) as usize;
                float_pixels[di..di + 4].copy_from_slice(&kf.raw_pixels[si..si + 4]);
                kf.raw_pixels[si..si + 4].fill(0);
            }
        }

        // Re-set selection (commit_raster_floating_now cleared it) and create float.
        shared.selection.raster_selection = Some(sel);
        shared.selection.raster_floating = Some(RasterFloatingSelection {
            pixels: std::sync::Arc::new(float_pixels),
            width: w,
            height: h,
            x: x0,
            y: y0,
            layer_id,
            time,
            canvas_before: std::sync::Arc::new(canvas_before),
            canvas_id: uuid::Uuid::new_v4(),
        });
    }

    /// `self.pending_raster_dabs` for dispatch by `VelloCallback::prepare()`.
    ///
    /// The actual pixel rendering happens on the GPU (compute shader).  The CPU
    /// Build the `tool_params: [f32; 4]` for a StrokeRecord.
    /// For clone/healing: [offset_x, offset_y, 0, 0] (computed from clone_stroke_offset).
    /// For all other tools: delegates to def.tool_params().
    fn make_tool_params(
        &self,
        def: &dyn crate::tools::RasterToolDef,
        shared: &SharedPaneState,
    ) -> [f32; 4] {
        use lightningbeam_core::raster_layer::RasterBlendMode;
        match def.blend_mode() {
            RasterBlendMode::CloneStamp | RasterBlendMode::Healing => {
                if let Some((ox, oy)) = self.clone_stroke_offset {
                    [ox, oy, 0.0, 0.0]
                } else {
                    [0.0; 4]
                }
            }
            _ => def.tool_params(shared.raster_settings),
        }
    }

    /// only does dab placement arithmetic (cheap).  On stroke end a readback is
    /// requested so the undo system can capture the final pixel state.
    fn handle_raster_stroke_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        def: &'static dyn crate::tools::RasterToolDef,
        shared: &mut SharedPaneState,
    ) {
        let blend_mode = def.blend_mode();
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::raster_layer::StrokePoint;
        use lightningbeam_core::brush_engine::{BrushEngine, StrokeState};
        use lightningbeam_core::raster_layer::StrokeRecord;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Only operate on raster layers
        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        if !is_raster { return; }

        let brush = {
            // Delegate brush parameter extraction to the tool definition.
            let bp = def.brush_params(shared.raster_settings);
            let (base_settings, radius, opacity, hardness, spacing) =
                (bp.base_settings, bp.radius, bp.opacity, bp.hardness, bp.spacing);
            let mut b = base_settings;
            // Compensate for pressure_radius_gain so that the UI-chosen radius is the
            // actual rendered radius at our fixed mouse pressure of 1.0.
            // radius_at_pressure(1.0) = exp(radius_log + gain × 0.5)
            // → radius_log = ln(radius) - gain × 0.5
            b.radius_log      = radius.ln() - b.pressure_radius_gain * 0.5;
            b.hardness        = hardness;
            b.opaque          = opacity;
            b.dabs_per_radius = spacing;
            if matches!(blend_mode, lightningbeam_core::raster_layer::RasterBlendMode::Smudge) {
                // Zero dabs_per_actual_radius so the spacing slider is the sole density control.
                b.dabs_per_actual_radius = 0.0;
                // strength controls how far behind the stroke to sample (smudge_dist multiplier).
                // smudge_dist = radius * exp(smudge_radius_log), so log(strength) gives the ratio.
                b.smudge_radius_log = shared.raster_settings.smudge_strength; // linear [0,1] strength
            }
            if matches!(blend_mode, lightningbeam_core::raster_layer::RasterBlendMode::BlurSharpen) {
                // Zero dabs_per_actual_radius so the spacing slider is the sole density control.
                b.dabs_per_actual_radius = 0.0;
            }
            b
        };

        let color = if matches!(blend_mode, lightningbeam_core::raster_layer::RasterBlendMode::Erase) {
            [1.0f32, 1.0, 1.0, 1.0]
        } else {
            let c = if shared.raster_settings.brush_use_fg { *shared.stroke_color } else { *shared.fill_color };
            let s2l = |v: u8| -> f32 {
                let f = v as f32 / 255.0;
                if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) }
            };
            [s2l(c.r()), s2l(c.g()), s2l(c.b()), c.a() as f32 / 255.0]
        };

        // ----------------------------------------------------------------
        // Mouse down: capture buffer_before, start stroke, compute first dab
        // ----------------------------------------------------------------
        // Use primary_pressed (fires immediately on mouse-down) so the first dab
        // appears before any drag movement.  Guard against re-triggering if a stroke
        // is already in progress.
        // rsp_clicked fires on the release frame of a quick click; the first condition
        // already handles the press frame with is_none() guard.  The clicked guard is
        // only needed when no stroke is active (avoids re-starting mid-stroke).
        let stroke_start = (self.rsp_primary_pressed(ui) && response.hovered()
                            && self.raster_stroke_state.is_none())
                        || (self.rsp_clicked(response) && self.raster_stroke_state.is_none());
        if stroke_start {
            // Clone stamp / healing brush: compute and store the source offset (source - drag_start).
            // This is constant for the entire stroke and used in every StrokeRecord below.
            if matches!(blend_mode, lightningbeam_core::raster_layer::RasterBlendMode::CloneStamp
                                  | lightningbeam_core::raster_layer::RasterBlendMode::Healing) {
                self.clone_stroke_offset = shared.raster_settings.clone_source.map(|s| (
                    s.x - world_pos.x, s.y - world_pos.y,
                ));
            } else {
                self.clone_stroke_offset = None;
            }

            // Determine if we are painting into the float (B) or the layer (A).
            let painting_float = shared.selection.raster_floating.is_some();
            self.painting_float = painting_float;
            self.stroke_clip_selection = shared.selection.raster_selection.clone();

            if painting_float {
                // ---- Paint onto float buffer B ----
                // Do NOT commit the float. Use the float's own GPU canvas.
                let (canvas_id, float_x, float_y, canvas_width, canvas_height,
                     buffer_before, layer_id, time) = {
                    let float = shared.selection.raster_floating.as_ref().unwrap();
                    let buf = (*float.pixels).clone();
                    (float.canvas_id, float.x, float.y, float.width, float.height,
                     buf, float.layer_id, float.time)
                };

                // Compute first dab (same arithmetic as the layer case).
                let mut stroke_state = StrokeState::new();
                // Convert to float-local space: dabs must be in canvas pixel coords.
                let first_pt = StrokePoint {
                    x: world_pos.x - float_x as f32,
                    y: world_pos.y - float_y as f32,
                    pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0,
                };
                let single = StrokeRecord {
                    brush_settings: brush.clone(),
                    color,
                    blend_mode,
                    tool_params: self.make_tool_params(def, shared),
                    points: vec![first_pt.clone()],
                };
                let (dabs, dab_bbox) = BrushEngine::compute_dabs(&single, &mut stroke_state, 0.0);
                self.raster_last_compute_time = ui.input(|i| i.time);

                self.painting_canvas = Some((layer_id, canvas_id));
                self.pending_undo_before = Some((
                    layer_id,
                    time,
                    canvas_width,
                    canvas_height,
                    buffer_before,
                ));
                self.pending_raster_dabs = Some(PendingRasterDabs {
                    keyframe_id: canvas_id,
                    layer_id,
                    time,
                    canvas_width,
                    canvas_height,
                    initial_pixels: None,  // canvas already initialized via lazy GPU init
                    dabs,
                    dab_bbox,
                    wants_final_readback: false,
                });
                self.raster_stroke_state = Some((
                    layer_id,
                    time,
                    stroke_state,
                    Vec::new(),
                ));
                self.raster_last_point = Some(first_pt);
                *shared.tool_state = ToolState::DrawingRasterStroke { points: vec![] };

            } else {
                // ---- Paint onto layer canvas A (existing behavior) ----
                // Commit any floating selection synchronously so buffer_before and
                // the GPU canvas initial upload see the fully-composited canvas.
                Self::commit_raster_floating_now(shared);

                let (doc_width, doc_height) = {
                    let doc = shared.action_executor.document();
                    (doc.width as u32, doc.height as u32)
                };

                // Ensure the keyframe exists BEFORE reading its ID, so we always get
                // the real UUID.  Previously we read the ID first and fell back to a
                // randomly-generated UUID when no keyframe existed; that fake UUID was
                // stored in painting_canvas but subsequent drag frames used the real UUID
                // from keyframe_at(), causing the GPU canvas to be a different object from
                // the one being composited.
                {
                    let doc = shared.action_executor.document_mut();
                    if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&active_layer_id) {
                        rl.ensure_keyframe_at(*shared.playback_time, doc_width, doc_height);
                    }
                }

                // Now read the guaranteed-to-exist keyframe to get the real UUID.
                let (keyframe_id, canvas_width, canvas_height, buffer_before, initial_pixels) = {
                    let doc = shared.action_executor.document();
                    if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&active_layer_id) {
                        if let Some(kf) = rl.keyframe_at(*shared.playback_time) {
                            let raw = kf.raw_pixels.clone();
                            let init = if raw.is_empty() {
                                vec![0u8; (kf.width * kf.height * 4) as usize]
                            } else {
                                raw.clone()
                            };
                            (kf.id, kf.width, kf.height, raw, init)
                        } else {
                            return; // shouldn't happen after ensure_keyframe_at
                        }
                    } else {
                        return;
                    }
                };

                // Compute the first dab (single-point tap)
                let mut stroke_state = StrokeState::new();

                let first_pt = StrokePoint {
                    x: world_pos.x, y: world_pos.y,
                    pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0,
                };
                let single = StrokeRecord {
                    brush_settings: brush.clone(),
                    color,
                    blend_mode,
                    tool_params: self.make_tool_params(def, shared),
                    points: vec![first_pt.clone()],
                };
                let (dabs, dab_bbox) = BrushEngine::compute_dabs(&single, &mut stroke_state, 0.0);
                self.raster_last_compute_time = ui.input(|i| i.time);

                // Layer strokes apply selection masking at readback time via stroke_clip_selection.

                self.painting_canvas = Some((active_layer_id, keyframe_id));
                self.pending_undo_before = Some((
                    active_layer_id,
                    *shared.playback_time,
                    canvas_width,
                    canvas_height,
                    buffer_before,
                ));
                self.pending_raster_dabs = Some(PendingRasterDabs {
                    keyframe_id,
                    layer_id: active_layer_id,
                    time: *shared.playback_time,
                    canvas_width,
                    canvas_height,
                    initial_pixels: Some(initial_pixels),
                    dabs,
                    dab_bbox,
                    wants_final_readback: false,
                });
                self.raster_stroke_state = Some((
                    active_layer_id,
                    *shared.playback_time,
                    stroke_state,
                    Vec::new(), // buffer_before now lives in pending_undo_before
                ));
                self.raster_last_point = Some(first_pt);
                *shared.tool_state = ToolState::DrawingRasterStroke { points: vec![] };
            }
        }

        // ----------------------------------------------------------------
        // Mouse drag: compute dabs for this segment
        // ----------------------------------------------------------------
        if self.rsp_dragged(response) {
            let tool_params = self.make_tool_params(def, shared);
            if let Some((layer_id, time, ref mut stroke_state, _)) = self.raster_stroke_state {
                if let Some(prev_pt) = self.raster_last_point.take() {
                    // Get canvas info and float offset now (used for both distance check
                    // and dab dispatch).  prev_pt is already in canvas-local space.
                    let canvas_info = if self.painting_float {
                        shared.selection.raster_floating.as_ref().map(|f| {
                            (f.canvas_id, f.width, f.height, f.x as f32, f.y as f32)
                        })
                    } else {
                        let doc = shared.action_executor.document();
                        if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&layer_id) {
                            if let Some(kf) = rl.keyframe_at(time) {
                                Some((kf.id, kf.width, kf.height, 0.0f32, 0.0f32))
                            } else { None }
                        } else { None }
                    };

                    let Some((canvas_id, cw, ch, cx, cy)) = canvas_info else {
                        self.raster_last_point = Some(prev_pt);
                        return;
                    };

                    // Convert current world position to canvas-local space.
                    let curr_local = StrokePoint {
                        x: world_pos.x - cx, y: world_pos.y - cy,
                        pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0,
                    };

                    const MIN_DIST_SQ: f32 = 1.5 * 1.5;
                    let dx = curr_local.x - prev_pt.x;
                    let dy = curr_local.y - prev_pt.y;
                    let moved_pt = if dx * dx + dy * dy >= MIN_DIST_SQ {
                        curr_local.clone()
                    } else {
                        prev_pt.clone()
                    };

                    if dx * dx + dy * dy >= MIN_DIST_SQ {
                        let seg = StrokeRecord {
                            brush_settings: brush.clone(),
                            color,
                            blend_mode,
                            tool_params,
                            points: vec![prev_pt, curr_local],
                        };
                        let current_time = ui.input(|i| i.time);
                        let dt = (current_time - self.raster_last_compute_time).clamp(0.0, 0.1) as f32;
                        self.raster_last_compute_time = current_time;
                        let (dabs, dab_bbox) = BrushEngine::compute_dabs(&seg, stroke_state, dt);
                        self.pending_raster_dabs = Some(PendingRasterDabs {
                            keyframe_id: canvas_id,
                            layer_id,
                            time,
                            canvas_width: cw,
                            canvas_height: ch,
                            initial_pixels: None,
                            dabs,
                            dab_bbox,
                            wants_final_readback: false,
                        });
                    }

                    self.raster_last_point = Some(moved_pt);
                }
            }
        }

        // ----------------------------------------------------------------
        // Stationary time-based dabs: when the mouse hasn't moved this frame,
        // still pass dt to the engine so time-based brushes (airbrush, etc.)
        // can accumulate and fire at the cursor position.
        // ----------------------------------------------------------------
        if self.pending_raster_dabs.is_none()
            && matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. })
        {
            let current_time = ui.input(|i| i.time);
            if self.raster_last_compute_time > 0.0 {
                let dt = (current_time - self.raster_last_compute_time).clamp(0.0, 0.1) as f32;
                self.raster_last_compute_time = current_time;
                let tool_params = self.make_tool_params(def, shared);

                if let Some((layer_id, time, ref mut stroke_state, _)) = self.raster_stroke_state {
                    let canvas_info = if self.painting_float {
                        shared.selection.raster_floating.as_ref().map(|f| {
                            (f.canvas_id, f.width, f.height, f.x as f32, f.y as f32)
                        })
                    } else {
                        let doc = shared.action_executor.document();
                        if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&layer_id) {
                            if let Some(kf) = rl.keyframe_at(time) {
                                Some((kf.id, kf.width, kf.height, 0.0f32, 0.0f32))
                            } else { None }
                        } else { None }
                    };

                    if let Some((canvas_id, cw, ch, cx, cy)) = canvas_info {
                        let pt = StrokePoint {
                            x: world_pos.x - cx,
                            y: world_pos.y - cy,
                            pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0,
                        };
                        let single = StrokeRecord {
                            brush_settings: brush.clone(),
                            color,
                            blend_mode,
                            tool_params,
                            points: vec![pt],
                        };
                        let (dabs, dab_bbox) = BrushEngine::compute_dabs(&single, stroke_state, dt);
                        if !dabs.is_empty() {
                            self.pending_raster_dabs = Some(PendingRasterDabs {
                                keyframe_id: canvas_id,
                                layer_id,
                                time,
                                canvas_width: cw,
                                canvas_height: ch,
                                initial_pixels: None,
                                dabs,
                                dab_bbox,
                                wants_final_readback: false,
                            });
                        }
                    }
                }
            }
        }

        // Reset compute-time tracker when stroke ends so next stroke starts fresh.
        if !matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. }) {
            self.raster_last_compute_time = 0.0;
        }

        // Keep egui repainting while a stroke is active so that:
        //   1. Time-based dabs (dabs_per_second) fire at the correct rate even when the
        //      mouse is held stationary (no move events → no automatic egui repaint).
        //   2. The post-stroke Vello update (consuming the readback result) happens on
        //      the very next frame rather than waiting for the next user input event.
        if matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. }) {
            ui.ctx().request_repaint();
        }

        // ----------------------------------------------------------------
        // Mouse up: request a full-canvas readback for the undo snapshot
        // ----------------------------------------------------------------
        if self.rsp_drag_stopped(response)
            || (self.rsp_any_released(ui) && matches!(*shared.tool_state, ToolState::DrawingRasterStroke { .. }))
        {
            self.raster_stroke_state = None;
            self.raster_last_point = None;
            *shared.tool_state = ToolState::Idle;

            // Mark the pending dabs (if any this frame) for final readback.
            // If there are no pending dabs this frame, create a "readback only" entry.
            if let Some(ref mut pending) = self.pending_raster_dabs {
                pending.wants_final_readback = true;
            } else if let Some((ub_layer, ub_time, ub_cw, ub_ch, _)) =
                    self.pending_undo_before.as_ref()
            {
                let (ub_layer, ub_time, ub_cw, ub_ch) = (*ub_layer, *ub_time, *ub_cw, *ub_ch);
                // Get canvas_id for the canvas texture lookup.
                // When painting into the float, use float.canvas_id; otherwise the keyframe id.
                let kf_id = if self.painting_float {
                    self.painting_canvas.map(|(_, cid)| cid)
                } else {
                    shared.action_executor.document()
                        .get_layer(&ub_layer)
                        .and_then(|l| if let AnyLayer::Raster(rl) = l {
                            rl.keyframe_at(ub_time).map(|kf| kf.id)
                        } else { None })
                };
                if let Some(kf_id) = kf_id {
                    self.pending_raster_dabs = Some(PendingRasterDabs {
                        keyframe_id: kf_id,
                        layer_id: ub_layer,
                        time: ub_time,
                        canvas_width: ub_cw,
                        canvas_height: ub_ch,
                        initial_pixels: None,
                        dabs: Vec::new(),
                        dab_bbox: (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                        wants_final_readback: true,
                    });
                }
            }
        }
    }

    /// Rectangular marquee selection tool for raster layers.
    /// Snapshot the active raster keyframe pixels, pass them to `draw_fn` to
    /// modify the buffer, then apply the result as an undoable `RasterFillAction`.
    ///
    /// Returns `false` if the layer or keyframe is not available.
    fn apply_raster_pixel_edit<F>(
        shared: &mut SharedPaneState,
        layer_id: uuid::Uuid,
        description: &'static str,
        draw_fn: F,
    ) -> bool
    where
        F: FnOnce(&mut [u8], u32, u32),
    {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterFillAction;

        let time = *shared.playback_time;
        // Canvas dimensions (to create keyframe if needed).
        let (doc_w, doc_h) = {
            let doc = shared.action_executor.document();
            (doc.width as u32, doc.height as u32)
        };
        // Ensure a keyframe exists at the current time.
        {
            let doc = shared.action_executor.document_mut();
            if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                rl.ensure_keyframe_at(time, doc_w, doc_h);
            }
        }
        // Snapshot the pixel buffer before drawing.
        let (buffer_before, w, h) = {
            let doc = shared.action_executor.document();
            match doc.get_layer(&layer_id) {
                Some(AnyLayer::Raster(rl)) => match rl.keyframe_at(time) {
                    Some(kf) => {
                        let expected = (kf.width * kf.height * 4) as usize;
                        let buf = if kf.raw_pixels.len() == expected {
                            kf.raw_pixels.clone()
                        } else {
                            vec![0u8; expected]
                        };
                        (buf, kf.width, kf.height)
                    }
                    None => return false,
                },
                _ => return false,
            }
        };
        let mut buffer_after = buffer_before.clone();
        draw_fn(&mut buffer_after, w, h);
        let action = RasterFillAction::new(layer_id, time, buffer_before, buffer_after, w, h)
            .with_description(description);
        let _ = shared.action_executor.execute(Box::new(action));
        true
    }

    /// Build a per-pixel boolean mask for an ellipse inscribed in the given
    /// axis-aligned bounding box. Used by the elliptical marquee mode.
    fn make_ellipse_mask(x0: i32, y0: i32, x1: i32, y1: i32) -> lightningbeam_core::selection::RasterSelection {
        use lightningbeam_core::selection::RasterSelection;
        let w = (x1 - x0) as u32;
        let h = (y1 - y0) as u32;
        if w == 0 || h == 0 {
            return RasterSelection::Mask { data: vec![], width: 0, height: 0, origin_x: x0, origin_y: y0 };
        }
        // Center in local pixel space. Add 0.5 to radii so the ellipse
        // touches every edge pixel without cutting them off.
        let cx = (w as f32 - 1.0) / 2.0;
        let cy = (h as f32 - 1.0) / 2.0;
        let rx = cx + 0.5;
        let ry = cy + 0.5;
        let mut data = vec![false; (w * h) as usize];
        for row in 0..h {
            for col in 0..w {
                let dx = (col as f32 - cx) / rx;
                let dy = (row as f32 - cy) / ry;
                data[(row * w + col) as usize] = dx * dx + dy * dy <= 1.0;
            }
        }
        RasterSelection::Mask { data, width: w, height: h, origin_x: x0, origin_y: y0 }
    }

    fn handle_raster_select_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::selection::RasterSelection;
        use lightningbeam_core::tool::ToolState;
        use crate::tools::SelectionShape;

        let Some(layer_id) = *shared.active_layer_id else { return };
        let doc = shared.action_executor.document();
        let Some(kf) = doc.get_layer(&layer_id).and_then(|l| {
            if let AnyLayer::Raster(rl) = l { rl.keyframe_at(*shared.playback_time) } else { None }
        }) else { return };
        let (canvas_w, canvas_h) = (kf.width as i32, kf.height as i32);
        let ellipse = shared.raster_settings.select_shape == SelectionShape::Ellipse;

        if self.rsp_drag_started(response) {
            let (px, py) = (world_pos.x as i32, world_pos.y as i32);
            let inside = shared.selection.raster_selection
                .as_ref()
                .map_or(false, |sel| sel.contains_pixel(px, py));

            if inside {
                // Drag inside the selection — move it (and any floating pixels).
                // As a safety net, lift the selection if no float exists yet.
                if shared.selection.raster_floating.is_none() {
                    Self::lift_selection_to_float(shared);
                }
                *shared.tool_state = ToolState::MovingRasterSelection { last: (px, py) };
            } else {
                // Drag outside — start a new marquee (commit any floating first).
                Self::commit_raster_floating_now(shared);
                *shared.tool_state = ToolState::DrawingRasterMarquee {
                    start: (px, py),
                    current: (px, py),
                };
            }
        }

        if self.rsp_dragged(response) {
            let (px, py) = (world_pos.x as i32, world_pos.y as i32);
            match *shared.tool_state {
                ToolState::DrawingRasterMarquee { start, ref mut current } => {
                    *current = (px, py);
                    let (x0, x1) = (start.0.min(px).max(0), start.0.max(px).min(canvas_w));
                    let (y0, y1) = (start.1.min(py).max(0), start.1.max(py).min(canvas_h));
                    shared.selection.raster_selection = Some(if ellipse {
                        Self::make_ellipse_mask(x0, y0, x1, y1)
                    } else {
                        RasterSelection::Rect(x0, y0, x1, y1)
                    });
                }
                ToolState::MovingRasterSelection { ref mut last } => {
                    let (dx, dy) = (px - last.0, py - last.1);
                    *last = (px, py);
                    // Shift the marquee.
                    if let Some(ref mut sel) = shared.selection.raster_selection {
                        *sel = match sel {
                            RasterSelection::Rect(x0, y0, x1, y1) =>
                                RasterSelection::Rect(*x0 + dx, *y0 + dy, *x1 + dx, *y1 + dy),
                            RasterSelection::Lasso(pts) =>
                                RasterSelection::Lasso(pts.iter().map(|(x, y)| (x + dx, y + dy)).collect()),
                            RasterSelection::Mask { data, width, height, origin_x, origin_y } =>
                                RasterSelection::Mask {
                                    data: std::mem::take(data),
                                    width: *width, height: *height,
                                    origin_x: *origin_x + dx,
                                    origin_y: *origin_y + dy,
                                },
                        };
                    }
                    // Shift floating pixels if any.
                    if let Some(ref mut float) = shared.selection.raster_floating {
                        float.x += dx;
                        float.y += dy;
                    }
                }
                _ => {}
            }
        }

        if self.rsp_drag_stopped(response) {
            match *shared.tool_state {
                ToolState::DrawingRasterMarquee { start, current } => {
                    let (x0, x1) = (start.0.min(current.0).max(0), start.0.max(current.0).min(canvas_w));
                    let (y0, y1) = (start.1.min(current.1).max(0), start.1.max(current.1).min(canvas_h));
                    if x1 > x0 && y1 > y0 {
                        shared.selection.raster_selection = Some(if ellipse {
                            Self::make_ellipse_mask(x0, y0, x1, y1)
                        } else {
                            RasterSelection::Rect(x0, y0, x1, y1)
                        });
                        Self::lift_selection_to_float(shared);
                    } else {
                        shared.selection.raster_selection = None;
                    }
                }
                ToolState::MovingRasterSelection { .. } => {}
                _ => {}
            }
            *shared.tool_state = ToolState::Idle;
        }

        if self.rsp_clicked(response) {
            // A click with no drag: if outside the selection, commit any float and
            // clear; if inside, do nothing (preserves the selection).
            let (px, py) = (world_pos.x as i32, world_pos.y as i32);
            let inside = shared.selection.raster_selection
                .as_ref()
                .map_or(false, |sel| sel.contains_pixel(px, py));
            if !inside {
                Self::commit_raster_floating_now(shared);
                shared.selection.raster_selection = None;
            }
            *shared.tool_state = ToolState::Idle;
        }

        let _ = (ui, canvas_h);
    }

    /// Freehand lasso selection tool for raster layers.
    fn handle_raster_lasso_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::selection::RasterSelection;
        use lightningbeam_core::tool::ToolState;

        let Some(layer_id) = *shared.active_layer_id else { return };
        if !shared.action_executor.document()
            .get_layer(&layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)))
        { return; }

        if self.rsp_drag_started(response) {
            Self::commit_raster_floating_now(shared);
            let pt = (world_pos.x as i32, world_pos.y as i32);
            *shared.tool_state = ToolState::DrawingRasterLasso { points: vec![pt] };
        }

        if self.rsp_dragged(response) {
            if let ToolState::DrawingRasterLasso { ref mut points } = *shared.tool_state {
                let pt = (world_pos.x as i32, world_pos.y as i32);
                if let Some(&last) = points.last() {
                    let (dx, dy) = (pt.0 - last.0, pt.1 - last.1);
                    if dx * dx + dy * dy >= 9 {
                        points.push(pt);
                    }
                }
                if points.len() >= 2 {
                    shared.selection.raster_selection = Some(RasterSelection::Lasso(points.clone()));
                }
            }
        }

        if self.rsp_drag_stopped(response) {
            if let ToolState::DrawingRasterLasso { ref points } = *shared.tool_state {
                if points.len() >= 3 {
                    shared.selection.raster_selection = Some(RasterSelection::Lasso(points.clone()));
                    Self::lift_selection_to_float(shared);
                } else {
                    shared.selection.raster_selection = None;
                }
            }
            *shared.tool_state = ToolState::Idle;
        }

        if self.rsp_clicked(response) {
            shared.selection.raster_selection = None;
            *shared.tool_state = ToolState::Idle;
        }

        let _ = ui;
    }

    /// Animated "marching ants" dashed outline along a closed screen-space polygon.
    /// `phase` advances over time to animate the dashes.
    fn draw_marching_ants(painter: &egui::Painter, pts: &[egui::Pos2], phase: f32) {
        if pts.len() < 2 { return; }
        let n = pts.len();
        let mut d = phase.rem_euclid(8.0); // 4px on, 4px off
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let seg = a.distance(b);
            if seg < 0.5 { continue; }
            let dir = (b - a) / seg;
            let mut t = 0.0f32;
            while t < seg {
                let rem = if d < 4.0 { 4.0 - d } else { 8.0 - d };
                let dl = rem.min(seg - t);
                if d < 4.0 {
                    let p0 = a + dir * t;
                    let p1 = a + dir * (t + dl);
                    painter.line_segment([p0, p1], egui::Stroke::new(2.5, egui::Color32::WHITE));
                    painter.line_segment([p0, p1], egui::Stroke::new(1.5, egui::Color32::BLACK));
                }
                d = (d + dl).rem_euclid(8.0);
                t += dl;
            }
        }
    }

    /// Draw marching ants around a canvas-space rect converted to screen space.
    fn draw_marching_ants_rect(
        painter: &egui::Painter,
        rect_min: egui::Pos2,
        x0: i32, y0: i32, x1: i32, y1: i32,
        zoom: f32, pan: egui::Vec2, phase: f32,
    ) {
        let s = |cx: i32, cy: i32| egui::pos2(
            rect_min.x + pan.x + cx as f32 * zoom,
            rect_min.y + pan.y + cy as f32 * zoom,
        );
        Self::draw_marching_ants(painter, &[s(x0,y0), s(x1,y0), s(x1,y1), s(x0,y1)], phase);
    }

    /// Draw marching ants around a canvas-space lasso polygon.
    fn draw_marching_ants_lasso(
        painter: &egui::Painter,
        rect_min: egui::Pos2,
        pts: &[(i32, i32)],
        zoom: f32, pan: egui::Vec2, phase: f32,
    ) {
        let screen: Vec<egui::Pos2> = pts.iter().map(|&(cx, cy)| egui::pos2(
            rect_min.x + pan.x + cx as f32 * zoom,
            rect_min.y + pan.y + cy as f32 * zoom,
        )).collect();
        Self::draw_marching_ants(painter, &screen, phase);
    }

    fn handle_paint_bucket_tool(
        &mut self,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;

        let active_layer_id = match shared.active_layer_id {
            Some(id) => *id,
            None => return,
        };

        if !self.rsp_clicked(response) { return; }

        let is_raster = shared.action_executor.document()
            .get_layer(&active_layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));

        if is_raster {
            self.handle_raster_paint_bucket(world_pos, active_layer_id, shared);
        } else {
            use lightningbeam_core::shape::ShapeColor;
            use lightningbeam_core::actions::PaintBucketAction;
            use vello::kurbo::Point;
            let click_point = Point::new(world_pos.x as f64, world_pos.y as f64);
            let fill_color = ShapeColor::from_egui(*shared.fill_color);
            let action = PaintBucketAction::new(
                active_layer_id,
                *shared.playback_time,
                click_point,
                fill_color,
            );
            let _ = shared.action_executor.execute(Box::new(action));
        }
    }

    fn handle_raster_paint_bucket(
        &mut self,
        world_pos: egui::Vec2,
        layer_id: uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterFillAction;
        use lightningbeam_core::flood_fill::{raster_flood_fill, FillThresholdMode};
        use crate::tools::FillThresholdMode as EditorMode;

        let time = *shared.playback_time;

        // Ensure a keyframe exists at the current time.
        let (doc_w, doc_h) = {
            let doc = shared.action_executor.document();
            (doc.width as u32, doc.height as u32)
        };
        {
            let doc = shared.action_executor.document_mut();
            if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                rl.ensure_keyframe_at(time, doc_w, doc_h);
            }
        }

        // Snapshot current pixels.
        let (buffer_before, width, height) = {
            let doc = shared.action_executor.document();
            if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&layer_id) {
                if let Some(kf) = rl.keyframe_at(time) {
                    let expected = (kf.width * kf.height * 4) as usize;
                    let buf = if kf.raw_pixels.len() == expected {
                        kf.raw_pixels.clone()
                    } else {
                        vec![0u8; expected]
                    };
                    (buf, kf.width, kf.height)
                } else { return; }
            } else { return; }
        };

        let seed_x = world_pos.x as i32;
        let seed_y = world_pos.y as i32;
        if seed_x < 0 || seed_y < 0 || seed_x >= width as i32 || seed_y >= height as i32 {
            return;
        }

        let fill_egui = *shared.fill_color;
        let fill_color = [fill_egui.r(), fill_egui.g(), fill_egui.b(), fill_egui.a()];
        let threshold  = shared.raster_settings.fill_threshold;
        let softness   = shared.raster_settings.fill_softness;
        let core_mode  = match shared.raster_settings.fill_threshold_mode {
            EditorMode::Absolute => FillThresholdMode::Absolute,
            EditorMode::Relative => FillThresholdMode::Relative,
        };

        let mut buffer_after = buffer_before.clone();
        raster_flood_fill(
            &mut buffer_after,
            width, height,
            seed_x, seed_y,
            fill_color,
            threshold, softness,
            core_mode,
            true, // paint bucket always fills contiguous region
            shared.selection.raster_selection.as_ref(),
        );

        let action = RasterFillAction::new(layer_id, time, buffer_before, buffer_after, width, height);
        let _ = shared.action_executor.execute(Box::new(action));
    }

    fn handle_magic_wand_tool(
        &mut self,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::flood_fill::{raster_fill_mask, FillThresholdMode};
        use lightningbeam_core::selection::RasterSelection;
        use crate::tools::FillThresholdMode as EditorMode;

        if !self.rsp_clicked(response) { return; }

        let Some(layer_id) = *shared.active_layer_id else { return };

        let is_raster = shared.action_executor.document()
            .get_layer(&layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        if !is_raster { return; }

        let time = *shared.playback_time;

        // Ensure keyframe exists.
        let (doc_w, doc_h) = {
            let doc = shared.action_executor.document();
            (doc.width as u32, doc.height as u32)
        };
        {
            let doc = shared.action_executor.document_mut();
            if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                rl.ensure_keyframe_at(time, doc_w, doc_h);
            }
        }

        let (pixels, width, height) = {
            let doc = shared.action_executor.document();
            if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&layer_id) {
                if let Some(kf) = rl.keyframe_at(time) {
                    let expected = (kf.width * kf.height * 4) as usize;
                    let buf = if kf.raw_pixels.len() == expected {
                        kf.raw_pixels.clone()
                    } else {
                        vec![0u8; expected]
                    };
                    (buf, kf.width, kf.height)
                } else { return; }
            } else { return; }
        };

        let seed_x = world_pos.x as i32;
        let seed_y = world_pos.y as i32;
        if seed_x < 0 || seed_y < 0 || seed_x >= width as i32 || seed_y >= height as i32 {
            return;
        }

        let threshold  = shared.raster_settings.wand_threshold;
        let contiguous = shared.raster_settings.wand_contiguous;
        let core_mode  = match shared.raster_settings.wand_mode {
            EditorMode::Absolute => FillThresholdMode::Absolute,
            EditorMode::Relative => FillThresholdMode::Relative,
        };

        // Use existing raster_selection as clip if present (so the wand only
        // selects inside the current selection — Shift/Intersect not yet supported).
        let dist_map = raster_fill_mask(
            &pixels, width, height,
            seed_x, seed_y,
            threshold, core_mode, contiguous,
            None, // ignore existing selection for wand — it defines a new one
        );

        let data: Vec<bool> = dist_map.iter().map(|d| d.is_some()).collect();

        shared.selection.raster_selection = Some(RasterSelection::Mask {
            data,
            width,
            height,
            origin_x: 0,
            origin_y: 0,
        });
        Self::lift_selection_to_float(shared);
    }

    fn handle_quick_select_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::selection::RasterSelection;

        let Some(layer_id) = *shared.active_layer_id else { return };

        let is_raster = shared.action_executor.document()
            .get_layer(&layer_id)
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
        if !is_raster { return; }

        let time = *shared.playback_time;
        let radius = shared.raster_settings.quick_select_radius;
        let threshold = shared.raster_settings.wand_threshold;

        if self.rsp_drag_started(response) {
            // Commit any existing float selection before starting a new one.
            Self::commit_raster_floating_now(shared);

            // Ensure the keyframe exists.
            let (doc_w, doc_h) = {
                let doc = shared.action_executor.document();
                (doc.width as u32, doc.height as u32)
            };
            {
                let doc = shared.action_executor.document_mut();
                if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                    rl.ensure_keyframe_at(time, doc_w, doc_h);
                }
            }

            // Snapshot canvas pixels.
            let (pixels, width, height) = {
                let doc = shared.action_executor.document();
                if let Some(AnyLayer::Raster(rl)) = doc.get_layer(&layer_id) {
                    if let Some(kf) = rl.keyframe_at(time) {
                        let expected = (kf.width * kf.height * 4) as usize;
                        let buf = if kf.raw_pixels.len() == expected {
                            kf.raw_pixels.clone()
                        } else {
                            vec![0u8; expected]
                        };
                        (buf, kf.width, kf.height)
                    } else { return; }
                } else { return; }
            };

            let seed_x = world_pos.x as i32;
            let seed_y = world_pos.y as i32;
            let mask = vec![false; (width * height) as usize];

            let mut qs = QuickSelectState {
                mask,
                pixels,
                width,
                height,
                last_pos: (seed_x - (radius as i32 * 2), seed_y), // force first fill
            };

            // Run the initial fill at the starting position.
            let mode = match shared.raster_settings.wand_mode {
                crate::tools::FillThresholdMode::Absolute =>
                    lightningbeam_core::flood_fill::FillThresholdMode::Absolute,
                crate::tools::FillThresholdMode::Relative =>
                    lightningbeam_core::flood_fill::FillThresholdMode::Relative,
            };
            Self::quick_select_fill_point(&mut qs, seed_x, seed_y, threshold, mode, radius);

            shared.selection.raster_selection = Some(RasterSelection::Mask {
                data: qs.mask.clone(),
                width: qs.width,
                height: qs.height,
                origin_x: 0,
                origin_y: 0,
            });

            self.quick_select_state = Some(qs);
        }

        if self.rsp_dragged(response) {
            let mode = match shared.raster_settings.wand_mode {
                crate::tools::FillThresholdMode::Absolute =>
                    lightningbeam_core::flood_fill::FillThresholdMode::Absolute,
                crate::tools::FillThresholdMode::Relative =>
                    lightningbeam_core::flood_fill::FillThresholdMode::Relative,
            };

            if let Some(ref mut qs) = self.quick_select_state {
                let sx = world_pos.x as i32;
                let sy = world_pos.y as i32;
                let dx = sx - qs.last_pos.0;
                let dy = sy - qs.last_pos.1;
                let min_move = (radius / 2.0).max(1.0) as i32;
                if dx * dx + dy * dy >= min_move * min_move {
                    Self::quick_select_fill_point(qs, sx, sy, threshold, mode, radius);
                }
                // Always sync raster_selection from the current mask so the
                // marching ants update every frame (same pattern as marquee select).
                shared.selection.raster_selection = Some(RasterSelection::Mask {
                    data: qs.mask.clone(),
                    width: qs.width,
                    height: qs.height,
                    origin_x: 0,
                    origin_y: 0,
                });
            }
        }

        if self.rsp_drag_stopped(response) {
            if self.quick_select_state.is_some() {
                Self::lift_selection_to_float(shared);
                self.quick_select_state = None;
            }
        }
    }

    /// Run a single flood-fill from `(seed_x, seed_y)` clipped to a local region
    /// and OR the result into `qs.mask`.
    fn quick_select_fill_point(
        qs: &mut QuickSelectState,
        seed_x: i32, seed_y: i32,
        threshold: f32,
        mode: lightningbeam_core::flood_fill::FillThresholdMode,
        radius: f32,
    ) {
        use lightningbeam_core::flood_fill::raster_fill_mask;
        use lightningbeam_core::selection::RasterSelection;

        if seed_x < 0 || seed_y < 0
            || seed_x >= qs.width as i32
            || seed_y >= qs.height as i32
        {
            return;
        }

        let expand = (radius * 3.0) as i32;
        let clip_x0 = (seed_x - expand).max(0);
        let clip_y0 = (seed_y - expand).max(0);
        let clip_x1 = (seed_x + expand).min(qs.width as i32);
        let clip_y1 = (seed_y + expand).min(qs.height as i32);
        let clip = RasterSelection::Rect(clip_x0, clip_y0, clip_x1, clip_y1);

        let dist_map = raster_fill_mask(
            &qs.pixels, qs.width, qs.height,
            seed_x, seed_y,
            threshold, mode, true, // contiguous = true
            Some(&clip),
        );

        for (i, d) in dist_map.iter().enumerate() {
            if d.is_some() {
                qs.mask[i] = true;
            }
        }
        qs.last_pos = (seed_x, seed_y);
    }

    /// Draw marching ants for a pixel mask selection.
    ///
    /// Animates horizontal edges leftward and vertical edges downward (position-based),
    /// producing a coherent clockwise-like marching effect without contour tracing.
    fn draw_marching_ants_mask(
        painter: &egui::Painter,
        rect_min: egui::Pos2,
        data: &[bool],
        width: u32, height: u32,
        origin_x: i32, origin_y: i32,
        zoom: f32, pan: egui::Vec2,
        phase: f32,
    ) {
        let w = width as i32;
        let h = height as i32;

        // Phase in screen pixels: 4px on, 4px off cycling every 8 screen pixels.
        // One canvas pixel = zoom screen pixels; scale phase accordingly.
        let screen_phase = phase; // already in screen pixels (matches draw_marching_ants)
        let cycle_canvas = 8.0 / zoom.max(0.01); // canvas-pixel length of a full 8-screen-px cycle
        let half_cycle_canvas = cycle_canvas / 2.0;

        let to_screen = |cx: i32, cy: i32| egui::pos2(
            rect_min.x + pan.x + cx as f32 * zoom,
            rect_min.y + pan.y + cy as f32 * zoom,
        );

        // Pre-scan: compute tight bounding box of set pixels so we don't iterate
        // the full canvas every frame (critical perf for large canvases with small masks).
        let mut min_row = h;
        let mut max_row = -1i32;
        let mut min_col = w;
        let mut max_col = -1i32;
        for row in 0..h {
            for col in 0..w {
                if data[(row * w + col) as usize] {
                    if row < min_row { min_row = row; }
                    if row > max_row { max_row = row; }
                    if col < min_col { min_col = col; }
                    if col > max_col { max_col = col; }
                }
            }
        }
        if max_row < 0 { return; } // Empty mask — nothing to draw.
        let r0 = (min_row - 1).max(0);
        let r1 = (max_row + 1).min(h - 1);
        let c0 = (min_col - 1).max(0);
        let c1 = (max_col + 1).min(w - 1);

        // Horizontal edges: between (row-1) and (row). Animate along x axis.
        // Use screen-space phase so the dash pattern looks correct at any zoom.
        for row in r0..=(r1 + 1) {
            for col in c0..=c1 {
                let above = row > 0   && data[((row-1) * w + col) as usize];
                let below = row < h   && data[(row     * w + col) as usize];
                if above == below { continue; }
                let cx = origin_x + col;
                let cy = origin_y + row;
                // canvas-pixel position along the edge, converted to screen pixels for phase
                let cx_screen = cx as f32 * zoom;
                let on = (cx_screen - screen_phase).rem_euclid(8.0) < 4.0;
                // Also check next pixel to handle partial overlap of the 4-px window
                let _ = half_cycle_canvas; // suppress unused warning
                if on {
                    let p0 = to_screen(cx, cy);
                    let p1 = to_screen(cx + 1, cy);
                    painter.line_segment([p0, p1], egui::Stroke::new(2.5, egui::Color32::WHITE));
                    painter.line_segment([p0, p1], egui::Stroke::new(1.5, egui::Color32::BLACK));
                }
            }
        }

        // Vertical edges: between (col-1) and (col). Animate along y axis.
        for col in c0..=(c1 + 1) {
            for row in r0..=r1 {
                let left  = col > 0 && data[(row * w + col - 1) as usize];
                let right = col < w && data[(row * w + col    ) as usize];
                if left == right { continue; }
                let cx = origin_x + col;
                let cy = origin_y + row;
                let cy_screen = cy as f32 * zoom;
                let on = (cy_screen - screen_phase).rem_euclid(8.0) < 4.0;
                if on {
                    let p0 = to_screen(cx, cy);
                    let p1 = to_screen(cx, cy + 1);
                    painter.line_segment([p0, p1], egui::Stroke::new(2.5, egui::Color32::WHITE));
                    painter.line_segment([p0, p1], egui::Stroke::new(1.5, egui::Color32::BLACK));
                }
            }
        }
    }

    /// Apply transform preview to objects based on current mouse position
    fn apply_transform_preview(
        vector_layer: &mut lightningbeam_core::layer::VectorLayer,
        mode: &lightningbeam_core::tool::TransformMode,
        original_transforms: &std::collections::HashMap<uuid::Uuid, lightningbeam_core::object::Transform>,
        _pivot: vello::kurbo::Point,
        start_mouse: vello::kurbo::Point,
        current_mouse: vello::kurbo::Point,
        original_bbox: vello::kurbo::Rect,
        _time: f64,
    ) {
        use lightningbeam_core::tool::{TransformMode, Axis};

        match mode {
            TransformMode::ScaleCorner { origin } => {
                println!("--- SCALE CORNER ---");
                println!("Origin: ({:.1}, {:.1})", origin.x, origin.y);
                println!("Start mouse: ({:.1}, {:.1})", start_mouse.x, start_mouse.y);
                println!("Current mouse: ({:.1}, {:.1})", current_mouse.x, current_mouse.y);

                // Calculate world-space scale from opposite corner
                let start_vec = start_mouse - *origin;
                let current_vec = current_mouse - *origin;

                println!("Start vec: ({:.1}, {:.1})", start_vec.x, start_vec.y);
                println!("Current vec: ({:.1}, {:.1})", current_vec.x, current_vec.y);

                let scale_x_world = if start_vec.x.abs() > 0.001 {
                    current_vec.x / start_vec.x
                } else {
                    1.0
                };

                let scale_y_world = if start_vec.y.abs() > 0.001 {
                    current_vec.y / start_vec.y
                } else {
                    1.0
                };

                println!("Scale world: ({:.3}, {:.3})", scale_x_world, scale_y_world);

                // Apply scale to all selected objects (both shape instances and clip instances)
                for (object_id, original_transform) in original_transforms {
                    println!("\nObject {:?}:", object_id);
                    println!("  Original pos: ({:.1}, {:.1})", original_transform.x, original_transform.y);
                    println!("  Original rotation: {:.1}°", original_transform.rotation);
                    println!("  Original scale: ({:.3}, {:.3})", original_transform.scale_x, original_transform.scale_y);

                    // Try to apply to shape instance
                    vector_layer.modify_object_internal(object_id, |obj| {
                        // Get object's rotation in radians
                        let rotation_rad = original_transform.rotation.to_radians();
                        let cos_r = rotation_rad.cos();
                        let sin_r = rotation_rad.sin();

                        // Transform scale from world space to object's local space
                        let cos_r_sq = cos_r * cos_r;
                        let sin_r_sq = sin_r * sin_r;
                        let sx_abs = scale_x_world.abs();
                        let sy_abs = scale_y_world.abs();

                        // Compute how much the object grows along its local axes
                        let local_scale_x = (cos_r_sq * sx_abs * sx_abs + sin_r_sq * sy_abs * sy_abs).sqrt();
                        let local_scale_y = (sin_r_sq * sx_abs * sx_abs + cos_r_sq * sy_abs * sy_abs).sqrt();

                        println!("  Local scale factors: ({:.3}, {:.3})", local_scale_x, local_scale_y);

                        // Scale the object's position relative to the origin point in world space
                        let rel_x = original_transform.x - origin.x;
                        let rel_y = original_transform.y - origin.y;

                        println!("  Relative pos from origin: ({:.1}, {:.1})", rel_x, rel_y);

                        obj.transform.x = origin.x + rel_x * scale_x_world;
                        obj.transform.y = origin.y + rel_y * scale_y_world;

                        println!("  New pos: ({:.1}, {:.1})", obj.transform.x, obj.transform.y);

                        // Apply local-space scale
                        obj.transform.scale_x = original_transform.scale_x * local_scale_x;
                        obj.transform.scale_y = original_transform.scale_y * local_scale_y;

                        println!("  New scale: ({:.3}, {:.3})", obj.transform.scale_x, obj.transform.scale_y);

                        // Keep rotation unchanged
                        obj.transform.rotation = original_transform.rotation;
                    });

                    // Also try to apply to clip instance
                    if let Some(clip_instance) = vector_layer.clip_instances.iter_mut().find(|ci| ci.id == *object_id) {
                        let rotation_rad = original_transform.rotation.to_radians();
                        let cos_r = rotation_rad.cos();
                        let sin_r = rotation_rad.sin();
                        let cos_r_sq = cos_r * cos_r;
                        let sin_r_sq = sin_r * sin_r;
                        let sx_abs = scale_x_world.abs();
                        let sy_abs = scale_y_world.abs();
                        let local_scale_x = (cos_r_sq * sx_abs * sx_abs + sin_r_sq * sy_abs * sy_abs).sqrt();
                        let local_scale_y = (sin_r_sq * sx_abs * sx_abs + cos_r_sq * sy_abs * sy_abs).sqrt();
                        let rel_x = original_transform.x - origin.x;
                        let rel_y = original_transform.y - origin.y;

                        clip_instance.transform.x = origin.x + rel_x * scale_x_world;
                        clip_instance.transform.y = origin.y + rel_y * scale_y_world;
                        clip_instance.transform.scale_x = original_transform.scale_x * local_scale_x;
                        clip_instance.transform.scale_y = original_transform.scale_y * local_scale_y;
                        clip_instance.transform.rotation = original_transform.rotation;
                    }
                }
            }

            TransformMode::ScaleEdge { axis, origin } => {
                // UNIFIED MATRIX APPROACH: Calculate bounding box transform, then apply to each object

                // Step 1: Calculate the bounding box transform (world-space scale from origin)
                // Preserve sign to allow flipping when dragging past the origin
                let (scale_x_world, scale_y_world) = match axis {
                    Axis::Horizontal => {
                        let start_dist = start_mouse.x - origin.x;
                        let current_dist = current_mouse.x - origin.x;
                        let scale = if start_dist.abs() > 0.001 {
                            current_dist / start_dist
                        } else {
                            1.0
                        };
                        (scale, 1.0)
                    }
                    Axis::Vertical => {
                        let start_dist = start_mouse.y - origin.y;
                        let current_dist = current_mouse.y - origin.y;
                        let scale = if start_dist.abs() > 0.001 {
                            current_dist / start_dist
                        } else {
                            1.0
                        };
                        (1.0, scale)
                    }
                };

                // Build the bounding box transform: translate to origin, scale, translate back
                use kurbo::Affine;
                let bbox_transform = Affine::translate((origin.x, origin.y))
                    * Affine::scale_non_uniform(scale_x_world, scale_y_world)
                    * Affine::translate((-origin.x, -origin.y));

                // Step 2: Apply to each object using matrix composition
                for (object_id, original_transform) in original_transforms {
                    // TODO: DCEL - opacity lookup disabled during migration
                    let original_opacity = 1.0_f64;

                    // New position: transform the object's position through bbox_transform
                    let new_pos = bbox_transform * kurbo::Point::new(original_transform.x, original_transform.y);

                    // Transform bbox operation to object's local space
                    // local_transform = R^(-1) * bbox_transform * R
                    let rotation = Affine::rotate(original_transform.rotation.to_radians());
                    let rotation_inv = Affine::rotate(-original_transform.rotation.to_radians());

                    // Extract just the linear part of bbox_transform (no translation)
                    let bbox_linear = Affine::scale_non_uniform(scale_x_world, scale_y_world);

                    // Transform to local space
                    let local_transform = rotation_inv * bbox_linear * rotation;

                    // Extract scale and skew directly from the 2x2 matrix
                    // Matrix form: [[a, c], [b, d]] = [[sx, sx*tan(ky)], [sy*tan(kx), sy]]
                    let coeffs = local_transform.as_coeffs();
                    let a = coeffs[0];
                    let b = coeffs[1];
                    let c = coeffs[2];
                    let d = coeffs[3];

                    // Direct extraction (no rotation assumed in local space)
                    let local_sx = a;
                    let local_sy = d;
                    let local_skew_x = if d.abs() > 0.001 { (b / d).atan().to_degrees() } else { 0.0 };
                    let local_skew_y = if a.abs() > 0.001 { (c / a).atan().to_degrees() } else { 0.0 };

                    // Apply to object
                    vector_layer.modify_object_internal(object_id, |obj| {
                        obj.transform.x = new_pos.x;
                        obj.transform.y = new_pos.y;
                        obj.transform.rotation = original_transform.rotation; // Preserve rotation
                        obj.transform.scale_x = original_transform.scale_x * local_sx;
                        obj.transform.scale_y = original_transform.scale_y * local_sy;
                        obj.transform.skew_x = original_transform.skew_x + local_skew_x;
                        obj.transform.skew_y = original_transform.skew_y + local_skew_y;
                        obj.opacity = original_opacity; // Preserve opacity (now separate from transform)
                    });
                }
            }

            TransformMode::Rotate { center } => {
                // Calculate rotation angle
                let start_vec = start_mouse - *center;
                let current_vec = current_mouse - *center;

                let start_angle = start_vec.y.atan2(start_vec.x);
                let current_angle = current_vec.y.atan2(current_vec.x);
                let delta_angle = (current_angle - start_angle).to_degrees();

                // Apply rotation to all selected objects
                for (object_id, original_transform) in original_transforms {
                    vector_layer.modify_object_internal(object_id, |obj| {
                        // Rotate position around center
                        let rel_x = original_transform.x - center.x;
                        let rel_y = original_transform.y - center.y;

                        let angle_rad = delta_angle.to_radians();
                        let cos_a = angle_rad.cos();
                        let sin_a = angle_rad.sin();

                        obj.transform.x = center.x + rel_x * cos_a - rel_y * sin_a;
                        obj.transform.y = center.y + rel_x * sin_a + rel_y * cos_a;
                        obj.transform.rotation = original_transform.rotation + delta_angle;

                        // Keep scale unchanged
                        obj.transform.scale_x = original_transform.scale_x;
                        obj.transform.scale_y = original_transform.scale_y;
                    });
                }
            }

            TransformMode::Skew { axis, origin } => {
                // Calculate skew angle for center-relative skewing (center stays fixed)
                let center = original_bbox.center();
                let skew_radians = match axis {
                    Axis::Horizontal => {
                        // Determine which horizontal edge we're dragging
                        let edge_y = if (origin.y - original_bbox.y0).abs() < 0.1 {
                            original_bbox.y1 // Origin is top edge, so dragging bottom
                        } else {
                            original_bbox.y0 // Origin is bottom edge, so dragging top
                        };
                        let distance = edge_y - center.y;  // Distance from center to edge
                        if distance.abs() > 0.1 {
                            // tan(skew) = mouse_movement / distance_from_center
                            let tan_skew = (current_mouse.x - start_mouse.x) / distance;
                            tan_skew.atan()
                        } else {
                            0.0
                        }
                    }
                    Axis::Vertical => {
                        // Determine which vertical edge we're dragging
                        let edge_x = if (origin.x - original_bbox.x0).abs() < 0.1 {
                            original_bbox.x1 // Origin is left edge, so dragging right
                        } else {
                            original_bbox.x0 // Origin is right edge, so dragging left
                        };
                        let distance = edge_x - center.x;  // Distance from center to edge
                        if distance.abs() > 0.1 {
                            // tan(skew) = mouse_movement / distance_from_center
                            let tan_skew = (current_mouse.y - start_mouse.y) / distance;
                            tan_skew.atan()
                        } else {
                            0.0
                        }
                    }
                };
                let skew_degrees = skew_radians.to_degrees();

                // Calculate selection center for group skew - this stays fixed
                let selection_center = match axis {
                    Axis::Horizontal => original_bbox.center().y,
                    Axis::Vertical => original_bbox.center().x,
                };

                // Apply skew to all selected objects
                // Note: skew_radians = atan(tan_skew), so tan(skew_radians) = tan_skew
                let tan_skew = skew_radians.tan();
                for (object_id, original_transform) in original_transforms {
                    // Calculate the world-space center where the renderer applies skew
                    // This is the shape's bounding box center transformed to world space
                    // TODO: DCEL - shape center lookup disabled during migration
                    let shape_center_world = (original_transform.x, original_transform.y);

                    vector_layer.modify_object_internal(object_id, |obj| {
                        // Distance from selection center using the object's actual skew center
                        let distance_from_center = match axis {
                            Axis::Horizontal => shape_center_world.1 - selection_center,
                            Axis::Vertical => shape_center_world.0 - selection_center,
                        };

                        // Calculate translation to make group skew around center
                        let (offset_x, offset_y) = match axis {
                            Axis::Horizontal => {
                                // Horizontal skew: objects above/below center move horizontally
                                (distance_from_center * tan_skew, 0.0)
                            }
                            Axis::Vertical => {
                                // Vertical skew: objects left/right of center move vertically
                                (0.0, distance_from_center * tan_skew)
                            }
                        };

                        // Apply skew to individual object
                        match axis {
                            Axis::Horizontal => {
                                obj.transform.skew_x = original_transform.skew_x + skew_degrees;
                            }
                            Axis::Vertical => {
                                obj.transform.skew_y = original_transform.skew_y + skew_degrees;
                            }
                        }

                        // Translate object for group-relative skew
                        obj.transform.x = original_transform.x + offset_x;
                        obj.transform.y = original_transform.y + offset_y;

                        // Keep other transform properties unchanged
                        obj.transform.rotation = original_transform.rotation;
                        obj.transform.scale_x = original_transform.scale_x;
                        obj.transform.scale_y = original_transform.scale_y;
                    });
                }
            }
        }
    }

    /// Hit test transform handles and return which handle was clicked
    fn hit_test_transform_handle(
        point: vello::kurbo::Point,
        bbox: vello::kurbo::Rect,
        tolerance: f64,
    ) -> Option<lightningbeam_core::tool::TransformMode> {
        use lightningbeam_core::tool::{TransformMode, Axis};
        use vello::kurbo::Point;

        // Check rotation handle first (20px above top edge)
        let rotation_handle = Point::new(bbox.center().x, bbox.y0 - 20.0);
        if point.distance(rotation_handle) < tolerance {
            return Some(TransformMode::Rotate {
                center: bbox.center(),
            });
        }

        // Check corner handles (8x8 squares)
        let corners = [
            (Point::new(bbox.x0, bbox.y0), 0), // Top-left
            (Point::new(bbox.x1, bbox.y0), 1), // Top-right
            (Point::new(bbox.x1, bbox.y1), 2), // Bottom-right
            (Point::new(bbox.x0, bbox.y1), 3), // Bottom-left
        ];

        for (corner, idx) in &corners {
            if point.distance(*corner) < tolerance {
                // Opposite corner is 2 positions away (diagonal)
                let opposite = corners[(idx + 2) % 4].0;
                return Some(TransformMode::ScaleCorner { origin: opposite });
            }
        }

        // Check edge handles (circles at midpoints)
        let edges = [
            (Point::new(bbox.center().x, bbox.y0), Axis::Vertical, bbox.y1),   // Top
            (Point::new(bbox.x1, bbox.center().y), Axis::Horizontal, bbox.x0), // Right
            (Point::new(bbox.center().x, bbox.y1), Axis::Vertical, bbox.y0),   // Bottom
            (Point::new(bbox.x0, bbox.center().y), Axis::Horizontal, bbox.x1), // Left
        ];

        for (edge, axis, origin_coord) in &edges {
            if point.distance(*edge) < tolerance {
                let origin = match axis {
                    Axis::Horizontal => Point::new(*origin_coord, edge.y),
                    Axis::Vertical => Point::new(edge.x, *origin_coord),
                };
                return Some(TransformMode::ScaleEdge {
                    axis: *axis,
                    origin,
                });
            }
        }

        // Check for skew (hovering over edge but not near a handle)
        // Define edge segments
        let edge_segments = [
            // Top edge
            (Point::new(bbox.x0, bbox.y0), Point::new(bbox.x1, bbox.y0), Axis::Horizontal, bbox.y1),
            // Right edge
            (Point::new(bbox.x1, bbox.y0), Point::new(bbox.x1, bbox.y1), Axis::Vertical, bbox.x0),
            // Bottom edge
            (Point::new(bbox.x1, bbox.y1), Point::new(bbox.x0, bbox.y1), Axis::Horizontal, bbox.y0),
            // Left edge
            (Point::new(bbox.x0, bbox.y1), Point::new(bbox.x0, bbox.y0), Axis::Vertical, bbox.x1),
        ];

        let skew_tolerance = tolerance * 1.5; // Slightly larger tolerance for edge detection
        for (start, end, axis, origin_coord) in &edge_segments {
            // Calculate distance from point to line segment
            let edge_vec = *end - *start;
            let point_vec = point - *start;
            let edge_length = edge_vec.hypot();

            if edge_length > 0.0 {
                // Project point onto line segment
                let t = (point_vec.x * edge_vec.x + point_vec.y * edge_vec.y) / (edge_length * edge_length);

                // Check if projection is within segment bounds (not at ends where handles are)
                let handle_exclusion = tolerance / edge_length; // Exclude regions near handles

                if t > handle_exclusion && t < (1.0 - handle_exclusion) {
                    // Calculate perpendicular distance to edge
                    let closest_point = *start + edge_vec * t;
                    let distance = point.distance(closest_point);

                    if distance < skew_tolerance {
                        let origin = match axis {
                            Axis::Horizontal => Point::new(point.x, *origin_coord),
                            Axis::Vertical => Point::new(*origin_coord, point.y),
                        };
                        return Some(TransformMode::Skew {
                            axis: *axis,
                            origin,
                        });
                    }
                }
            }
        }

        None
    }

    /// Handle transform tool for DCEL elements (vertices/edges).
    /// Uses snapshot-based undo via ModifyDcelAction.
    fn handle_transform_dcel(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        point: vello::kurbo::Point,
        active_layer_id: &uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;

        let time = *shared.playback_time;

        // Calculate bounding box of selected DCEL vertices
        let selected_verts: Vec<lightningbeam_core::dcel::VertexId> =
            shared.selection.selected_vertices().iter().copied().collect();

        if selected_verts.is_empty() {
            return;
        }

        let bbox = {
            let document = shared.action_executor.document();
            if let Some(AnyLayer::Vector(vl)) = document.get_layer(active_layer_id) {
                if let Some(dcel) = vl.dcel_at_time(time) {
                    let mut min_x = f64::MAX;
                    let mut min_y = f64::MAX;
                    let mut max_x = f64::MIN;
                    let mut max_y = f64::MIN;
                    for &vid in &selected_verts {
                        let v = dcel.vertex(vid);
                        if v.deleted { continue; }
                        min_x = min_x.min(v.position.x);
                        min_y = min_y.min(v.position.y);
                        max_x = max_x.max(v.position.x);
                        max_y = max_y.max(v.position.y);
                    }
                    if min_x > max_x { return; }
                    vello::kurbo::Rect::new(min_x, min_y, max_x, max_y)
                } else {
                    return;
                }
            } else {
                return;
            }
        };

        // If already transforming, handle drag and release
        match shared.tool_state.clone() {
            ToolState::Transforming { mode, start_mouse, original_bbox, .. } => {
                // Drag: apply transform preview to DCEL
                if self.rsp_dragged(response) {
                    *shared.tool_state = ToolState::Transforming {
                        mode: mode.clone(),
                        original_transforms: std::collections::HashMap::new(),
                        pivot: original_bbox.center(),
                        start_mouse,
                        current_mouse: point,
                        original_bbox,
                    };

                    if let Some(ref cache) = self.dcel_editing_cache {
                        let original_dcel = cache.dcel_before.clone();
                        let selected_verts_set: std::collections::HashSet<lightningbeam_core::dcel::VertexId> =
                            selected_verts.iter().copied().collect();
                        let selected_edges: std::collections::HashSet<lightningbeam_core::dcel::EdgeId> =
                            shared.selection.selected_edges().iter().copied().collect();

                        let affine = Self::compute_transform_affine(
                            &mode, start_mouse, point, &original_bbox,
                        );

                        let document = shared.action_executor.document_mut();
                        if let Some(AnyLayer::Vector(vl)) = document.get_layer_mut(active_layer_id) {
                            if let Some(dcel) = vl.dcel_at_time_mut(time) {
                                Self::apply_dcel_transform(
                                    dcel, &original_dcel, &selected_verts_set, &selected_edges, affine,
                                );
                            }
                        }
                    }
                }

                // Release: finalize
                if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(*shared.tool_state, ToolState::Transforming { .. })) {
                    if let Some(cache) = self.dcel_editing_cache.take() {
                        let dcel_after = {
                            let document = shared.action_executor.document();
                            match document.get_layer(active_layer_id) {
                                Some(AnyLayer::Vector(vl)) => vl.dcel_at_time(time).cloned(),
                                _ => None,
                            }
                        };
                        if let Some(dcel_after) = dcel_after {
                            use lightningbeam_core::actions::ModifyDcelAction;
                            let action = ModifyDcelAction::new(
                                cache.layer_id, cache.time, cache.dcel_before, dcel_after, "Transform",
                            );
                            shared.pending_actions.push(Box::new(action));
                        }
                    }
                    *shared.tool_state = ToolState::Idle;
                }

                return;
            }
            _ => {}
        }

        // Idle: check for handle clicks to start a transform
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            let tolerance = 10.0;
            if let Some(mode) = Self::hit_test_transform_handle(point, bbox, tolerance) {
                // Snapshot DCEL for undo
                let document = shared.action_executor.document();
                if let Some(AnyLayer::Vector(vl)) = document.get_layer(active_layer_id) {
                    if let Some(dcel) = vl.dcel_at_time(time) {
                        self.dcel_editing_cache = Some(DcelEditingCache {
                            layer_id: *active_layer_id,
                            time,
                            dcel_before: dcel.clone(),
                        });
                    }
                }

                *shared.tool_state = ToolState::Transforming {
                    mode,
                    original_transforms: std::collections::HashMap::new(),
                    pivot: bbox.center(),
                    start_mouse: point,
                    current_mouse: point,
                    original_bbox: bbox,
                };
            }
        }
    }

    /// Compute an Affine transform from a TransformMode, start mouse, and current mouse position.
    fn compute_transform_affine(
        mode: &lightningbeam_core::tool::TransformMode,
        start_mouse: vello::kurbo::Point,
        current_mouse: vello::kurbo::Point,
        original_bbox: &vello::kurbo::Rect,
    ) -> vello::kurbo::Affine {
        use lightningbeam_core::tool::{TransformMode, Axis};
        use vello::kurbo::Affine;

        match mode {
            TransformMode::ScaleCorner { origin } => {
                let start_vec = start_mouse - *origin;
                let current_vec = current_mouse - *origin;
                let sx = if start_vec.x.abs() > 0.001 { current_vec.x / start_vec.x } else { 1.0 };
                let sy = if start_vec.y.abs() > 0.001 { current_vec.y / start_vec.y } else { 1.0 };
                Affine::translate((origin.x, origin.y))
                    * Affine::scale_non_uniform(sx, sy)
                    * Affine::translate((-origin.x, -origin.y))
            }
            TransformMode::ScaleEdge { axis, origin } => {
                let (sx, sy) = match axis {
                    Axis::Horizontal => {
                        let sd = start_mouse.x - origin.x;
                        let cd = current_mouse.x - origin.x;
                        (if sd.abs() > 0.001 { cd / sd } else { 1.0 }, 1.0)
                    }
                    Axis::Vertical => {
                        let sd = start_mouse.y - origin.y;
                        let cd = current_mouse.y - origin.y;
                        (1.0, if sd.abs() > 0.001 { cd / sd } else { 1.0 })
                    }
                };
                Affine::translate((origin.x, origin.y))
                    * Affine::scale_non_uniform(sx, sy)
                    * Affine::translate((-origin.x, -origin.y))
            }
            TransformMode::Rotate { center } => {
                let start_angle = (start_mouse.y - center.y).atan2(start_mouse.x - center.x);
                let current_angle = (current_mouse.y - center.y).atan2(current_mouse.x - center.x);
                let delta = current_angle - start_angle;
                Affine::translate((center.x, center.y))
                    * Affine::rotate(delta)
                    * Affine::translate((-center.x, -center.y))
            }
            TransformMode::Skew { axis, origin } => {
                let center = original_bbox.center();
                let skew_radians = match axis {
                    Axis::Horizontal => {
                        let edge_y = if (origin.y - original_bbox.y0).abs() < 0.1 {
                            original_bbox.y1
                        } else {
                            original_bbox.y0
                        };
                        let distance = edge_y - center.y;
                        if distance.abs() > 0.1 {
                            ((current_mouse.x - start_mouse.x) / distance).atan()
                        } else {
                            0.0
                        }
                    }
                    Axis::Vertical => {
                        let edge_x = if (origin.x - original_bbox.x0).abs() < 0.1 {
                            original_bbox.x1
                        } else {
                            original_bbox.x0
                        };
                        let distance = edge_x - center.x;
                        if distance.abs() > 0.1 {
                            ((current_mouse.y - start_mouse.y) / distance).atan()
                        } else {
                            0.0
                        }
                    }
                };
                let tan_s = skew_radians.tan();
                let (kx, ky) = match axis {
                    Axis::Horizontal => (tan_s, 0.0),
                    Axis::Vertical => (0.0, tan_s),
                };
                // Skew around center: translate to center, skew, translate back
                let skew = Affine::new([1.0, ky, kx, 1.0, 0.0, 0.0]);
                Affine::translate((center.x, center.y))
                    * skew
                    * Affine::translate((-center.x, -center.y))
            }
        }
    }

    // -------------------------------------------------------------------------
    // Raster transform tool
    // -------------------------------------------------------------------------

    /// CPU computation for raster transform: output AABB and inverse affine matrix.
    ///
    /// Returns `(new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1)` where
    /// the inverse affine maps output pixel (ox, oy) → source pixel (sx, sy):
    ///   sx = a00*ox + a01*oy + b0
    ///   sy = a10*ox + a11*oy + b1
    fn compute_transform_params(
        orig_w: u32, orig_h: u32,
        cx: f32, cy: f32,
        scale_x: f32, scale_y: f32,
        angle: f32,
    ) -> (u32, u32, i32, i32, f32, f32, f32, f32, f32, f32) {
        let hw = scale_x * orig_w as f32 / 2.0;
        let hh = scale_y * orig_h as f32 / 2.0;
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        // Rotate corners of scaled rect around (cx, cy)
        let local = [(-hw, -hh), (hw, -hh), (-hw, hh), (hw, hh)];
        let rotated: [(f32, f32); 4] = local.map(|(lx, ly)| {
            (cx + lx * cos_a - ly * sin_a, cy + lx * sin_a + ly * cos_a)
        });

        // AABB of rotated corners
        let min_x = rotated.iter().map(|p| p.0).fold(f32::INFINITY,     f32::min).floor();
        let min_y = rotated.iter().map(|p| p.1).fold(f32::INFINITY,     f32::min).floor();
        let max_x = rotated.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max).ceil();
        let max_y = rotated.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max).ceil();
        let new_x = min_x as i32;
        let new_y = min_y as i32;
        let new_w = ((max_x - min_x).max(1.0)) as u32;
        let new_h = ((max_y - min_y).max(1.0)) as u32;

        // Inverse affine: R^-1 * S^-1
        // Forward: dst = cx + R * S * (src_center_offset)
        // Inverse: src_pixel = (src_w/2, src_h/2) + S^-1 * R^-1 * (out_pixel - cx, out_pixel_y - cy)
        // with out_pixel center accounted for by baking +0.5 into b (CPU side).
        let a00 =  cos_a / scale_x;
        let a01 =  sin_a / scale_x;
        let a10 = -sin_a / scale_y;
        let a11 =  cos_a / scale_y;

        // b accounts for the center offset and the new AABB origin.
        // For output pixel (ox, oy) at its center (ox + 0.5, oy + 0.5) in output canvas coords,
        // the source pixel is:
        //   (src_w/2, src_h/2) + A^-1 * ((new_x + ox + 0.5) - cx, (new_y + oy + 0.5) - cy)
        // We bake (new_x + 0.5 - cx, new_y + 0.5 - cy) into b so the shader just uses ox/oy directly.
        let off_x = new_x as f32 + 0.5 - cx;
        let off_y = new_y as f32 + 0.5 - cy;
        let b0 = orig_w as f32 / 2.0 + a00 * off_x + a01 * off_y;
        let b1 = orig_h as f32 / 2.0 + a10 * off_x + a11 * off_y;

        (new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1)
    }

    fn handle_raster_transform_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        // If float was cleared, clear transform state.
        if shared.selection.raster_floating.is_none() {
            self.raster_transform_state = None;
            return;
        }
        let float_canvas_id = shared.selection.raster_floating.as_ref().unwrap().canvas_id;

        // If the float changed (new selection made), clear and reinit state.
        if let Some(ref ts) = self.raster_transform_state {
            if ts.float_canvas_id != float_canvas_id {
                self.raster_transform_state = None;
            }
        }

        // --- Lazy init ---
        if self.raster_transform_state.is_none() {
            let float = shared.selection.raster_floating.as_ref().unwrap();
            let expected_len = (float.width * float.height * 4) as usize;
            let anchor_pixels = if float.pixels.len() == expected_len {
                (*float.pixels).clone()
            } else {
                vec![0u8; expected_len]
            };
            let cx = float.x as f32 + float.width  as f32 / 2.0;
            let cy = float.y as f32 + float.height as f32 / 2.0;
            self.raster_transform_state = Some(RasterTransformState {
                float_canvas_id:   float.canvas_id,
                anchor_canvas_id:  uuid::Uuid::new_v4(),
                anchor_pixels,
                anchor_w: float.width,
                anchor_h: float.height,
                display_canvas_id: uuid::Uuid::new_v4(),
                cx, cy,
                scale_x: 1.0, scale_y: 1.0, angle: 0.0,
                origin_x: cx, origin_y: cy,
                active_handle: None, hovered_handle: None,
                drag_start_world: world_pos,
                snap_cx: cx, snap_cy: cy,
                snap_sx: 1.0, snap_sy: 1.0, snap_angle: 0.0,
                snap_origin_x: cx, snap_origin_y: cy,
                transform_applied: true,
                wants_apply: false,
            });
            // Queue an identity dispatch immediately so the display canvas is populated
            // from frame 1. Without this, Move-only drags don't update the image because
            // transform_applied would stay false (no scale/rotate → no needs_dispatch).
            let init_dispatch = {
                let ts = self.raster_transform_state.as_ref().unwrap();
                let (new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1) =
                    Self::compute_transform_params(ts.anchor_w, ts.anchor_h, ts.cx, ts.cy, 1.0, 1.0, 0.0);
                PendingTransformDispatch {
                    anchor_canvas_id:  ts.anchor_canvas_id,
                    anchor_pixels:     ts.anchor_pixels.clone(),
                    anchor_w:          ts.anchor_w,
                    anchor_h:          ts.anchor_h,
                    display_canvas_id: ts.display_canvas_id,
                    new_x, new_y, new_w, new_h,
                    a00, a01, a10, a11, b0, b1,
                    is_final_commit: false,
                }
            };
            self.pending_transform_dispatch = Some(init_dispatch);
        }

        // Early return while waiting for a final readback (wants_apply set, readback pending).
        if self.raster_transform_state.as_ref().map_or(false, |ts| ts.wants_apply) {
            return;
        }

        // --- Keyboard shortcuts ---
        // Enter: queue final dispatch + readback, keep state alive until readback completes.
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            let dispatch = self.raster_transform_state.as_ref().and_then(|ts| {
                if ts.transform_applied {
                    let (new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1) =
                        Self::compute_transform_params(ts.anchor_w, ts.anchor_h, ts.cx, ts.cy, ts.scale_x, ts.scale_y, ts.angle);
                    Some(PendingTransformDispatch {
                        anchor_canvas_id: ts.anchor_canvas_id,
                        anchor_pixels:    ts.anchor_pixels.clone(),
                        anchor_w: ts.anchor_w, anchor_h: ts.anchor_h,
                        display_canvas_id: ts.display_canvas_id,
                        new_x, new_y, new_w, new_h,
                        a00, a01, a10, a11, b0, b1,
                        is_final_commit: true,
                    })
                } else {
                    None
                }
            });
            if let Some(d) = dispatch {
                self.pending_transform_dispatch = Some(d);
                // Keep state alive (wants_apply = true) until readback completes.
                self.raster_transform_state.as_mut().unwrap().wants_apply = true;
            } else {
                // No transform was applied — just clear state.
                self.raster_transform_state = None;
            }
            return;
        }

        // Escape: float canvas is unchanged — just clear state.
        // The anchor/display canvases are orphaned; they'll be freed when the GPU engine
        // is next queried (the canvases are small and short-lived).
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.raster_transform_state = None;
            return;
        }

        // Read drag states before the mutable borrow of raster_transform_state.
        let drag_started = self.rsp_drag_started(response);
        let dragged      = self.rsp_dragged(response);
        let drag_stopped = self.rsp_drag_stopped(response);
        let shift        = ui.input(|i| i.modifiers.shift);

        // Collect pending dispatch from the inner block to assign after the borrow ends.
        let pending_dispatch;
        {
            let ts = self.raster_transform_state.as_mut().unwrap();

            // --- Compute handle positions in world space ---
            let hw = ts.scale_x * ts.anchor_w as f32 / 2.0;
            let hh = ts.scale_y * ts.anchor_h as f32 / 2.0;
            let cos_a = ts.angle.cos();
            let sin_a = ts.angle.sin();
            let zoom  = self.zoom;

            // Local offset → world position
            let to_world = |lx: f32, ly: f32| -> egui::Vec2 {
                egui::vec2(ts.cx + lx * cos_a - ly * sin_a, ts.cy + lx * sin_a + ly * cos_a)
            };

            // Rotate handle: above top-center, 24 screen-pixels outside bbox.
            let rotate_offset = 24.0 / zoom;
            let rotate_handle = to_world(0.0, -hh - rotate_offset);

            let handles: [(RasterTransformHandle, egui::Vec2); 10] = [
                (RasterTransformHandle::Corner { right: false, bottom: false }, to_world(-hw, -hh)),
                (RasterTransformHandle::Corner { right: true,  bottom: false }, to_world( hw, -hh)),
                (RasterTransformHandle::Corner { right: false, bottom: true  }, to_world(-hw,  hh)),
                (RasterTransformHandle::Corner { right: true,  bottom: true  }, to_world( hw,  hh)),
                (RasterTransformHandle::EdgeH  { bottom: false }, to_world(0.0, -hh)),
                (RasterTransformHandle::EdgeH  { bottom: true  }, to_world(0.0,  hh)),
                (RasterTransformHandle::EdgeV  { right: false  }, to_world(-hw, 0.0)),
                (RasterTransformHandle::EdgeV  { right: true   }, to_world( hw, 0.0)),
                (RasterTransformHandle::Rotate,                   rotate_handle),
                (RasterTransformHandle::Origin,                   egui::vec2(ts.origin_x, ts.origin_y)),
            ];

            let hit_r_world = 8.0 / zoom;
            let hovered = handles.iter()
                .find(|(_, wp)| (world_pos - *wp).length() <= hit_r_world)
                .map(|(h, _)| *h);

            // Inside bbox → Move handle (if no specific handle hit)
            let in_bbox = {
                let dx = world_pos.x - ts.cx;
                let dy = world_pos.y - ts.cy;
                let local_x =  dx * cos_a + dy * sin_a;
                let local_y = -dx * sin_a + dy * cos_a;
                local_x.abs() <= hw && local_y.abs() <= hh
            };
            let hovered = hovered.or_else(|| if in_bbox { Some(RasterTransformHandle::Move) } else { None });

            // Store hovered handle for visual feedback in the draw function.
            ts.hovered_handle = if ts.active_handle.is_none() { hovered } else { ts.active_handle };

            // Set cursor icon based on hovered/active handle.
            if let Some(h) = ts.active_handle.or(ts.hovered_handle) {
                let cursor = match h {
                    RasterTransformHandle::Move   => egui::CursorIcon::Grab,
                    RasterTransformHandle::Origin => egui::CursorIcon::Crosshair,
                    RasterTransformHandle::Corner { right, bottom } => {
                        if right == bottom { egui::CursorIcon::ResizeNwSe }
                        else               { egui::CursorIcon::ResizeNeSw }
                    }
                    RasterTransformHandle::EdgeH { .. } => egui::CursorIcon::ResizeVertical,
                    RasterTransformHandle::EdgeV { .. } => egui::CursorIcon::ResizeHorizontal,
                    RasterTransformHandle::Rotate       => egui::CursorIcon::AllScroll,
                };
                ui.ctx().set_cursor_icon(cursor);
            }

            // --- Drag start: use press_origin for hit testing (drag fires after threshold) ---
            if drag_started {
                let click_world = ui.input(|i| i.pointer.press_origin())
                    .map(|sp| {
                        let canvas_pos = sp - response.rect.min.to_vec2();
                        egui::vec2(
                            (canvas_pos.x - self.pan_offset.x) / self.zoom,
                            (canvas_pos.y - self.pan_offset.y) / self.zoom,
                        )
                    })
                    .unwrap_or(world_pos);

                // Recompute hovered handle at click_world position.
                let click_hovered = handles.iter()
                    .find(|(_, wp)| (click_world - *wp).length() <= hit_r_world)
                    .map(|(h, _)| *h);
                let click_in_bbox = {
                    let dx = click_world.x - ts.cx;
                    let dy = click_world.y - ts.cy;
                    let local_x =  dx * cos_a + dy * sin_a;
                    let local_y = -dx * sin_a + dy * cos_a;
                    local_x.abs() <= hw && local_y.abs() <= hh
                };
                let click_handle = click_hovered.or_else(|| if click_in_bbox { Some(RasterTransformHandle::Move) } else { None });

                ts.active_handle     = click_handle;
                ts.drag_start_world  = click_world;
                ts.snap_cx           = ts.cx;
                ts.snap_cy           = ts.cy;
                ts.snap_sx           = ts.scale_x;
                ts.snap_sy           = ts.scale_y;
                ts.snap_angle        = ts.angle;
                ts.snap_origin_x     = ts.origin_x;
                ts.snap_origin_y     = ts.origin_y;
            }

            // --- Drag ---
            let mut needs_dispatch = false;
            if dragged {
                if let Some(handle) = ts.active_handle {
                    let delta      = world_pos - ts.drag_start_world;
                    let snap_hw    = ts.snap_sx * ts.anchor_w as f32 / 2.0;
                    let snap_hh    = ts.snap_sy * ts.anchor_h as f32 / 2.0;
                    let local_dx   =  delta.x * cos_a + delta.y * sin_a;
                    let local_dy   = -delta.x * sin_a + delta.y * cos_a;

                    match handle {
                        RasterTransformHandle::Move => {
                            ts.cx = ts.snap_cx + delta.x;
                            ts.cy = ts.snap_cy + delta.y;
                            ts.origin_x = ts.snap_origin_x + delta.x;
                            ts.origin_y = ts.snap_origin_y + delta.y;
                            // Pure move: display canvas keeps same pixels, position updated via compute_transform_params.
                        }
                        RasterTransformHandle::Origin => {
                            ts.origin_x = ts.snap_origin_x + delta.x;
                            ts.origin_y = ts.snap_origin_y + delta.y;
                            // No GPU dispatch needed for origin move alone.
                        }
                        RasterTransformHandle::Corner { right, bottom } => {
                            let sign_x = if right  { 1.0_f32 } else { -1.0 };
                            let sign_y = if bottom { 1.0_f32 } else { -1.0 };
                            // Divide by 2: dragged corner = new_cx ± new_hw, and
                            // new_cx = wfx ± new_hw, so corner = wfx ± 2*new_hw.
                            // To make the corner move 1:1 with mouse, new_hw grows by delta/2.
                            // Signed clamp: allow negative scale (flip) but prevent exactly 0
                            // which would make the inverse affine matrix singular.
                            let raw_hw = snap_hw + sign_x * local_dx / 2.0;
                            let new_hw = if raw_hw.abs() < 0.001 { if raw_hw <= 0.0 { -0.001 } else { 0.001 } } else { raw_hw };
                            let new_hh = if shift {
                                // Preserve aspect ratio; sign follows new_hw.
                                new_hw * (ts.anchor_h as f32 / ts.anchor_w as f32).max(0.001)
                            } else {
                                let raw_hh = snap_hh + sign_y * local_dy / 2.0;
                                if raw_hh.abs() < 0.001 { if raw_hh <= 0.0 { -0.001 } else { 0.001 } } else { raw_hh }
                            };
                            ts.scale_x = new_hw / (ts.anchor_w as f32 / 2.0).max(0.001);
                            ts.scale_y = new_hh / (ts.anchor_h as f32 / 2.0).max(0.001);
                            // Fixed corner world pos (opposite corner, from snap state).
                            let wfx = ts.snap_cx - sign_x * snap_hw * cos_a + sign_y * snap_hh * sin_a;
                            let wfy = ts.snap_cy - sign_x * snap_hw * sin_a - sign_y * snap_hh * cos_a;
                            // New center: fixed corner + rotated new half-extents.
                            ts.cx = wfx + sign_x * new_hw * cos_a - sign_y * new_hh * sin_a;
                            ts.cy = wfy + sign_x * new_hw * sin_a + sign_y * new_hh * cos_a;
                            // Maintain origin's relative position within the scaled bbox.
                            let o_dx = ts.snap_origin_x - ts.snap_cx;
                            let o_dy = ts.snap_origin_y - ts.snap_cy;
                            let o_local_x =  o_dx * cos_a + o_dy * sin_a;
                            let o_local_y = -o_dx * sin_a + o_dy * cos_a;
                            let o_norm_x = if snap_hw > 0.0 { o_local_x / snap_hw } else { 0.0 };
                            let o_norm_y = if snap_hh > 0.0 { o_local_y / snap_hh } else { 0.0 };
                            let no_x = o_norm_x * new_hw;
                            let no_y = o_norm_y * new_hh;
                            ts.origin_x = ts.cx + no_x * cos_a - no_y * sin_a;
                            ts.origin_y = ts.cy + no_x * sin_a + no_y * cos_a;
                            needs_dispatch = true;
                        }
                        RasterTransformHandle::EdgeH { bottom } => {
                            let sign_y = if bottom { 1.0_f32 } else { -1.0 };
                            let raw_hh = snap_hh + sign_y * local_dy / 2.0;
                            let new_hh = if raw_hh.abs() < 0.001 { if raw_hh <= 0.0 { -0.001 } else { 0.001 } } else { raw_hh };
                            ts.scale_y = new_hh / (ts.anchor_h as f32 / 2.0).max(0.001);
                            // Fixed edge world position (opposite edge center).
                            let wfx = ts.snap_cx + sign_y * snap_hh * sin_a;
                            let wfy = ts.snap_cy - sign_y * snap_hh * cos_a;
                            ts.cx = wfx - sign_y * new_hh * sin_a;
                            ts.cy = wfy + sign_y * new_hh * cos_a;
                            // Maintain origin's relative Y position within the scaled bbox.
                            let o_dx = ts.snap_origin_x - ts.snap_cx;
                            let o_dy = ts.snap_origin_y - ts.snap_cy;
                            let o_local_x =  o_dx * cos_a + o_dy * sin_a;
                            let o_local_y = -o_dx * sin_a + o_dy * cos_a;
                            let o_norm_y = if snap_hh > 0.0 { o_local_y / snap_hh } else { 0.0 };
                            let no_x = o_local_x; // X local coord unchanged by EdgeH
                            let no_y = o_norm_y * new_hh;
                            ts.origin_x = ts.cx + no_x * cos_a - no_y * sin_a;
                            ts.origin_y = ts.cy + no_x * sin_a + no_y * cos_a;
                            needs_dispatch = true;
                        }
                        RasterTransformHandle::EdgeV { right } => {
                            let sign_x = if right { 1.0_f32 } else { -1.0 };
                            let raw_hw = snap_hw + sign_x * local_dx / 2.0;
                            let new_hw = if raw_hw.abs() < 0.001 { if raw_hw <= 0.0 { -0.001 } else { 0.001 } } else { raw_hw };
                            ts.scale_x = new_hw / (ts.anchor_w as f32 / 2.0).max(0.001);
                            // Fixed edge world position (opposite edge center).
                            let wfx = ts.snap_cx - sign_x * snap_hw * cos_a;
                            let wfy = ts.snap_cy - sign_x * snap_hw * sin_a;
                            ts.cx = wfx + sign_x * new_hw * cos_a;
                            ts.cy = wfy + sign_x * new_hw * sin_a;
                            // Maintain origin's relative X position within the scaled bbox.
                            let o_dx = ts.snap_origin_x - ts.snap_cx;
                            let o_dy = ts.snap_origin_y - ts.snap_cy;
                            let o_local_x =  o_dx * cos_a + o_dy * sin_a;
                            let o_local_y = -o_dx * sin_a + o_dy * cos_a;
                            let o_norm_x = if snap_hw > 0.0 { o_local_x / snap_hw } else { 0.0 };
                            let no_x = o_norm_x * new_hw;
                            let no_y = o_local_y; // Y local coord unchanged by EdgeV
                            ts.origin_x = ts.cx + no_x * cos_a - no_y * sin_a;
                            ts.origin_y = ts.cy + no_x * sin_a + no_y * cos_a;
                            needs_dispatch = true;
                        }
                        RasterTransformHandle::Rotate => {
                            // Rotate around origin (not center).
                            let v_start = ts.drag_start_world - egui::vec2(ts.origin_x, ts.origin_y);
                            let v_now   = world_pos           - egui::vec2(ts.origin_x, ts.origin_y);
                            let a_start = v_start.y.atan2(v_start.x);
                            let a_now   = v_now.y.atan2(v_now.x);
                            let d_angle = a_now - a_start;
                            ts.angle = ts.snap_angle + d_angle;
                            // Also rotate cx/cy around the origin.
                            let ox  = ts.snap_origin_x;
                            let oy  = ts.snap_origin_y;
                            let dcx = ts.snap_cx - ox;
                            let dcy = ts.snap_cy - oy;
                            let (cos_d, sin_d) = (d_angle.cos(), d_angle.sin());
                            ts.cx = ox + dcx * cos_d - dcy * sin_d;
                            ts.cy = oy + dcx * sin_d + dcy * cos_d;
                            needs_dispatch = true;
                        }
                    }
                }
            }

            // Build pending dispatch before the borrow ends (avoid partial move issues).
            if needs_dispatch && dragged {
                let (new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1) =
                    Self::compute_transform_params(ts.anchor_w, ts.anchor_h, ts.cx, ts.cy, ts.scale_x, ts.scale_y, ts.angle);
                ts.transform_applied = true;
                let anchor_canvas_id  = ts.anchor_canvas_id;
                let anchor_pixels     = ts.anchor_pixels.clone();
                let anchor_w          = ts.anchor_w;
                let anchor_h          = ts.anchor_h;
                let display_canvas_id = ts.display_canvas_id;
                pending_dispatch = Some(PendingTransformDispatch {
                    anchor_canvas_id, anchor_pixels, anchor_w, anchor_h,
                    display_canvas_id, new_x, new_y, new_w, new_h,
                    a00, a01, a10, a11, b0, b1,
                    is_final_commit: false,
                });
            } else {
                pending_dispatch = None;
            }

            // --- Drag stop ---
            if drag_stopped {
                ts.active_handle = None;
            }
        }

        if let Some(p) = pending_dispatch {
            self.pending_transform_dispatch = Some(p);
        }

        // Handle drawing is deferred to render_raster_transform_overlays(), called
        // from render_content() AFTER the VelloCallback is registered, so the handles
        // appear on top of the Vello scene rather than underneath it.
    }

    fn draw_raster_transform_handles_static(
        ui: &mut egui::Ui,
        rect: egui::Rect,
        ts: &RasterTransformState,
        zoom: f32,
        pan: egui::Vec2,
    ) {
        let painter = ui.painter_at(rect);

        // World → screen
        let w2s = |wx: f32, wy: f32| -> egui::Pos2 {
            egui::pos2(
                wx * zoom + pan.x + rect.min.x,
                wy * zoom + pan.y + rect.min.y,
            )
        };

        let hw = ts.scale_x * ts.anchor_w as f32 / 2.0;
        let hh = ts.scale_y * ts.anchor_h as f32 / 2.0;
        let cos_a = ts.angle.cos();
        let sin_a = ts.angle.sin();
        let to_world = |lx: f32, ly: f32| -> (f32, f32) {
            (ts.cx + lx * cos_a - ly * sin_a, ts.cy + lx * sin_a + ly * cos_a)
        };

        // Draw bounding box outline (4 edges between corners)
        let corners_local = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)];
        let corners_screen: Vec<egui::Pos2> = corners_local.iter()
            .map(|&(lx, ly)| { let (wx, wy) = to_world(lx, ly); w2s(wx, wy) })
            .collect();
        let outline_color = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200);
        let shadow_color  = egui::Color32::from_rgba_unmultiplied(0,   0,   0,   120);
        for i in 0..4 {
            let a = corners_screen[i];
            let b = corners_screen[(i + 1) % 4];
            painter.line_segment([a, b], egui::Stroke::new(2.0, shadow_color));
            painter.line_segment([a, b], egui::Stroke::new(1.0, outline_color));
        }

        // Colors
        let handle_normal  = egui::Color32::WHITE;
        let handle_hovered = egui::Color32::from_rgb(100, 180, 255); // light blue
        let handle_active  = egui::Color32::from_rgb(30,  120, 255); // bright blue

        let handle_color = |h: RasterTransformHandle| -> egui::Color32 {
            if ts.active_handle == Some(h) { handle_active }
            else if ts.hovered_handle == Some(h) { handle_hovered }
            else { handle_normal }
        };

        // Draw corner + edge handles (paired with their handle enum variant)
        let handle_pairs: [(RasterTransformHandle, (f32, f32)); 8] = [
            (RasterTransformHandle::Corner { right: false, bottom: false }, to_world(-hw, -hh)),
            (RasterTransformHandle::Corner { right: true,  bottom: false }, to_world( hw, -hh)),
            (RasterTransformHandle::Corner { right: false, bottom: true  }, to_world(-hw,  hh)),
            (RasterTransformHandle::Corner { right: true,  bottom: true  }, to_world( hw,  hh)),
            (RasterTransformHandle::EdgeH  { bottom: false }, to_world(0.0, -hh)),
            (RasterTransformHandle::EdgeH  { bottom: true  }, to_world(0.0,  hh)),
            (RasterTransformHandle::EdgeV  { right: false  }, to_world(-hw, 0.0)),
            (RasterTransformHandle::EdgeV  { right: true   }, to_world( hw, 0.0)),
        ];
        for (handle, (wx, wy)) in handle_pairs {
            let sp = w2s(wx, wy);
            let is_hover = ts.hovered_handle == Some(handle) || ts.active_handle == Some(handle);
            let size = if is_hover { 10.0 } else { 8.0 };
            let inner = if is_hover {  8.0 } else { 6.0 };
            painter.rect_filled(
                egui::Rect::from_center_size(sp, egui::vec2(size, size)),
                0.0,
                shadow_color,
            );
            painter.rect_filled(
                egui::Rect::from_center_size(sp, egui::vec2(inner, inner)),
                0.0,
                handle_color(handle),
            );
        }

        // Draw rotate handle (circle above top-center)
        let rotate_offset = 24.0 / zoom;
        let (rwx, rwy) = to_world(0.0, -hh - rotate_offset);
        let rsp = w2s(rwx, rwy);
        // Line from top-center to rotate handle
        let (tcx, tcy) = to_world(0.0, -hh);
        painter.line_segment([w2s(tcx, tcy), rsp], egui::Stroke::new(1.0, outline_color));
        let rot_hov = ts.hovered_handle == Some(RasterTransformHandle::Rotate)
            || ts.active_handle  == Some(RasterTransformHandle::Rotate);
        let rot_color = if ts.active_handle  == Some(RasterTransformHandle::Rotate) { handle_active }
            else if ts.hovered_handle == Some(RasterTransformHandle::Rotate) { handle_hovered }
            else { handle_normal };
        let rot_r = if rot_hov { 7.0 } else { 5.0 };
        painter.circle_filled(rsp, rot_r, rot_color);
        painter.circle_stroke(rsp, rot_r, egui::Stroke::new(1.5, shadow_color));

        // Draw origin handle (pivot point for rotate/scale) — a small crosshair circle.
        // origin_x/origin_y are already in world coords, use w2s directly.
        let origin_sp = w2s(ts.origin_x, ts.origin_y);
        let orig_color = if ts.hovered_handle == Some(RasterTransformHandle::Origin)
            || ts.active_handle == Some(RasterTransformHandle::Origin) { handle_hovered } else { handle_normal };
        painter.circle_filled(origin_sp, 5.0, orig_color);
        painter.circle_stroke(origin_sp, 5.0, egui::Stroke::new(1.5, shadow_color));
        let arm = 6.0;
        painter.line_segment([origin_sp - egui::vec2(arm, 0.0), origin_sp + egui::vec2(arm, 0.0)],
            egui::Stroke::new(1.0, shadow_color));
        painter.line_segment([origin_sp - egui::vec2(0.0, arm), origin_sp + egui::vec2(0.0, arm)],
            egui::Stroke::new(1.0, shadow_color));
    }

    /// Apply an affine transform to selected DCEL vertices and their connected edge control points.
    /// Reads original positions from `original_dcel` and writes transformed positions to `dcel`.
    fn apply_dcel_transform(
        dcel: &mut lightningbeam_core::dcel::Dcel,
        original_dcel: &lightningbeam_core::dcel::Dcel,
        selected_verts: &std::collections::HashSet<lightningbeam_core::dcel::VertexId>,
        selected_edges: &std::collections::HashSet<lightningbeam_core::dcel::EdgeId>,
        affine: vello::kurbo::Affine,
    ) {
        // Transform selected vertex positions
        for &vid in selected_verts {
            let original_pos = original_dcel.vertex(vid).position;
            dcel.vertex_mut(vid).position = affine * original_pos;
        }

        // Transform edge curves for selected edges
        for &eid in selected_edges {
            let original_curve = original_dcel.edge(eid).curve;
            let edge = dcel.edge_mut(eid);
            edge.curve.p0 = affine * original_curve.p0;
            edge.curve.p1 = affine * original_curve.p1;
            edge.curve.p2 = affine * original_curve.p2;
            edge.curve.p3 = affine * original_curve.p3;
        }
    }

    // -----------------------------------------------------------------------
    // Warp tool
    // -----------------------------------------------------------------------

    fn handle_raster_warp_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::Tool;
        use uuid::Uuid;

        // Ensure we're on a raster layer.
        let Some(layer_id) = *shared.active_layer_id else { return; };
        let is_raster = shared.action_executor.document().get_layer(&layer_id)
            .map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));
        if !is_raster { return; }

        let grid_cols = shared.raster_settings.warp_grid_cols.max(2);
        let grid_rows = shared.raster_settings.warp_grid_rows.max(2);

        // ---- Keyboard: Enter = commit, Escape = cancel ----
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if escape {
            if let Some(ws) = self.warp_state.take() {
                // Schedule cleanup of display canvas.
                self.pending_canvas_removal = Some(ws.display_canvas_id);
                self.painting_canvas = None;
                let _ = (ws.anchor_canvas_id, ws.disp_buf_id);
            }
            return;
        }

        if enter {
            if let Some(ref mut ws) = self.warp_state {
                if !ws.wants_commit {
                    ws.wants_commit = true;
                    let disp_data = Self::extract_grid_disps(&ws.control_points);
                    self.pending_warp_ops.push(PendingWarpOp::WarpApply {
                        anchor_canvas_id:  ws.anchor_canvas_id,
                        disp_buf_id:       ws.disp_buf_id,
                        display_canvas_id: ws.display_canvas_id,
                        disp_data:         Some(disp_data),
                        grid_cols:         ws.grid_cols,
                        grid_rows:         ws.grid_rows,
                        w: ws.anchor_w, h: ws.anchor_h,
                        final_commit: true,
                        layer_id:     ws.layer_id,
                        time:         ws.time,
                        is_float_warp: ws.float_offset.is_some(),
                    });
                }
            }
            return;
        }

        // ---- Lazy init (first time Warp tool is active on this layer) ----
        let time = *shared.playback_time;
        let needs_init = self.warp_state.as_ref()
            .map_or(true, |ws| ws.layer_id != layer_id);

        if needs_init {
            // Clean up old state if switching layers.
            if let Some(old) = self.warp_state.take() {
                self.pending_canvas_removal = Some(old.display_canvas_id);
                self.painting_canvas = None;
            }

            // Determine anchor source: floating selection on this layer, or the keyframe.
            let float_offset: Option<(i32, i32)>;
            let anchor_canvas_id: uuid::Uuid;
            let anchor_pixels: Vec<u8>;
            let w: u32;
            let h: u32;

            if let Some(float_sel) = shared.selection.raster_floating.as_ref()
                .filter(|f| f.layer_id == layer_id)
            {
                // Warp the floating selection.
                float_offset    = Some((float_sel.x, float_sel.y));
                anchor_canvas_id = float_sel.canvas_id;
                w               = float_sel.width;
                h               = float_sel.height;
                anchor_pixels   = if float_sel.pixels.is_empty() {
                    vec![0u8; (w * h * 4) as usize]
                } else {
                    (*float_sel.pixels).clone()
                };
            } else {
                // Warp the full keyframe canvas.
                float_offset = None;
                let doc = shared.action_executor.document();
                let (kf_id, kw, kh, raw_pix) = doc.get_layer(&layer_id)
                    .and_then(|l| if let lightningbeam_core::layer::AnyLayer::Raster(rl) = l {
                        rl.keyframe_at(time).map(|kf| {
                            let expected = (kf.width * kf.height * 4) as usize;
                            let mut pix = kf.raw_pixels.clone();
                            if pix.len() != expected { pix.resize(expected, 0); }
                            (kf.id, kf.width, kf.height, pix)
                        })
                    } else { None })
                    .unwrap_or_else(|| {
                        let dw = 1920u32; let dh = 1080u32;
                        (Uuid::new_v4(), dw, dh, vec![0u8; (dw * dh * 4) as usize])
                    });
                anchor_canvas_id = kf_id;
                w               = kw;
                h               = kh;
                anchor_pixels   = raw_pix;
            }

            let display_canvas_id = Uuid::new_v4();
            let disp_buf_id       = Uuid::new_v4();

            // Build evenly-spaced control point grid in world space.
            // For a float, control points are offset by float_offset so they align with the float.
            let (ox, oy) = float_offset
                .map(|(x, y)| (x as f32, y as f32))
                .unwrap_or((0.0, 0.0));
            let num_pts = (grid_cols * grid_rows) as usize;
            let mut control_points = Vec::with_capacity(num_pts);
            for row in 0..grid_rows {
                for col in 0..grid_cols {
                    let hx = ox + col as f32 / (grid_cols - 1) as f32 * w as f32;
                    let hy = oy + row as f32 / (grid_rows - 1) as f32 * h as f32;
                    control_points.push([hx, hy, hx, hy]);
                }
            }

            // Queue GPU init.
            self.pending_warp_ops.push(PendingWarpOp::Init {
                anchor_canvas_id,
                display_canvas_id,
                disp_buf_id,
                w, h,
                anchor_pixels,
                is_liquify: false,
            });

            self.warp_state = Some(WarpState {
                layer_id,
                time,
                anchor_canvas_id,
                display_canvas_id,
                disp_buf_id,
                anchor_w: w,
                anchor_h: h,
                grid_cols,
                grid_rows,
                float_offset,
                control_points,
                active_point: None,
                hovered_point: None,
                dirty: false,
                warp_applied: false,
                wants_commit: false,
            });
        }

        // Pre-check drag states before taking the warp_state borrow.
        let drag_started = self.rsp_drag_started(response);
        let dragged      = self.rsp_dragged(response);
        let drag_stopped = self.rsp_drag_stopped(response);
        let drag_delta   = response.drag_delta() / self.zoom;

        let ws = match self.warp_state.as_mut() {
            Some(ws) => ws,
            None => return,
        };

        // Update painting_canvas each frame (in case it was cleared).
        // NOTE: Can't write to self.painting_canvas here while ws borrows self.warp_state.
        // Set painting_canvas after the ws block via a flag.

        // ---- Draw grid overlay ----
        // Use Order::Foreground so the grid renders on top of the GPU canvas paint callback.
        let rect = response.rect;
        let mut painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("warp_grid_overlay"),
        ));
        painter.set_clip_rect(rect);
        let to_screen = |cx: f32, cy: f32| -> egui::Pos2 {
            egui::pos2(
                rect.min.x + self.pan_offset.x + cx * self.zoom,
                rect.min.y + self.pan_offset.y + cy * self.zoom,
            )
        };
        // Horizontal lines
        for row in 0..ws.grid_rows {
            for col in 0..ws.grid_cols - 1 {
                let a = &ws.control_points[(row * ws.grid_cols + col) as usize];
                let b = &ws.control_points[(row * ws.grid_cols + col + 1) as usize];
                painter.line_segment([to_screen(a[2], a[3]), to_screen(b[2], b[3])],
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(180, 180, 180, 180)));
            }
        }
        // Vertical lines
        for row in 0..ws.grid_rows - 1 {
            for col in 0..ws.grid_cols {
                let a = &ws.control_points[(row * ws.grid_cols + col) as usize];
                let b = &ws.control_points[((row + 1) * ws.grid_cols + col) as usize];
                painter.line_segment([to_screen(a[2], a[3]), to_screen(b[2], b[3])],
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(180, 180, 180, 180)));
            }
        }

        // ---- Hit-test control points (hover uses current pos; drag-start uses press_origin) ----
        let hover_r = 10.0_f32;
        let mouse_screen = egui::pos2(
            rect.min.x + self.pan_offset.x + world_pos.x * self.zoom,
            rect.min.y + self.pan_offset.y + world_pos.y * self.zoom,
        );
        let mut new_hover: Option<usize> = None;
        for (i, pt) in ws.control_points.iter().enumerate() {
            let screen_pt = to_screen(pt[2], pt[3]);
            if screen_pt.distance(mouse_screen) < hover_r {
                new_hover = Some(i);
                break;
            }
        }
        ws.hovered_point = new_hover;

        // Draw control points
        for (i, pt) in ws.control_points.iter().enumerate() {
            let screen_pt = to_screen(pt[2], pt[3]);
            let (size, color) = if ws.active_point == Some(i) {
                (5.0, egui::Color32::WHITE)
            } else if ws.hovered_point == Some(i) {
                (4.0, egui::Color32::from_rgb(255, 220, 80))
            } else {
                (3.0, egui::Color32::from_rgba_unmultiplied(220, 220, 220, 200))
            };
            painter.rect_filled(egui::Rect::from_center_size(screen_pt, egui::Vec2::splat(size * 2.0)), 0.0, color);
            painter.rect_stroke(egui::Rect::from_center_size(screen_pt, egui::Vec2::splat(size * 2.0)),
                0.0, egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(60, 60, 60, 200)), egui::StrokeKind::Inside);
        }

        // ---- Drag handling ----
        if drag_started {
            // Use press_origin for hit-testing — drag_started fires after the threshold,
            // so world_pos is already offset from where the user actually clicked.
            let click_screen = ui.input(|i| i.pointer.press_origin())
                .unwrap_or_else(|| egui::pos2(mouse_screen.x, mouse_screen.y));
            ws.active_point = ws.control_points.iter().enumerate()
                .find(|(_, pt)| to_screen(pt[2], pt[3]).distance(click_screen) < hover_r)
                .map(|(i, _)| i);
        }
        if dragged {
            if let Some(idx) = ws.active_point {
                ws.control_points[idx][2] += drag_delta.x;
                ws.control_points[idx][3] += drag_delta.y;
                ws.dirty = true;
            }
        }
        if drag_stopped {
            ws.active_point = None;
        }

        // ---- Collect pending warp op data before releasing ws borrow ----
        let pending_op = if ws.dirty && !ws.wants_commit {
            ws.dirty = false;
            ws.warp_applied = true;
            let disp_data = Self::extract_grid_disps(&ws.control_points);
            Some(PendingWarpOp::WarpApply {
                anchor_canvas_id:  ws.anchor_canvas_id,
                disp_buf_id:       ws.disp_buf_id,
                display_canvas_id: ws.display_canvas_id,
                disp_data:         Some(disp_data),
                grid_cols:         ws.grid_cols,
                grid_rows:         ws.grid_rows,
                w: ws.anchor_w, h: ws.anchor_h,
                final_commit: false,
                layer_id:     ws.layer_id,
                time:         ws.time,
                is_float_warp: ws.float_offset.is_some(),
            })
        } else {
            None
        };
        let (ws_layer_id, ws_display_id, ws_float_offset) = (ws.layer_id, ws.display_canvas_id, ws.float_offset);
        drop(ws);  // release borrow of warp_state

        // Display canvas is initialised by Init (zero-displacement apply), so it always
        // has valid content. For full-layer warp, override the layer blit unconditionally.
        // For float warp the override is done via transform_display in render_content().
        if ws_float_offset.is_none() {
            self.painting_canvas = Some((ws_layer_id, ws_display_id));
        }
        if let Some(op) = pending_op {
            self.pending_warp_ops.push(op);
            ui.ctx().request_repaint();
        }
    }

    /// Compute a per-pixel displacement map from a warp control-point grid.
    ///
    /// For each pixel (x, y) we find its fractional grid position, then bilinearly
    /// interpolate the displacements of the surrounding 4 grid points.
    /// Extract per-control-point displacements (displaced - home) from the control point array.
    /// Returns a tiny vec (grid_cols * grid_rows entries) uploaded to the GPU displacement buffer.
    /// The shader does bilinear interpolation per pixel, so no per-pixel CPU work is needed.
    fn extract_grid_disps(control_points: &[[f32; 4]]) -> Vec<[f32; 2]> {
        // The warp shader is an inverse warp: output pixel (x,y) samples source at (x+d.x, y+d.y).
        // So to make content follow the handle (forward warp), negate: d = home - displaced.
        control_points.iter()
            .map(|p| [p[0] - p[2], p[1] - p[3]])
            .collect()
    }

    // -----------------------------------------------------------------------
    // Liquify tool
    // -----------------------------------------------------------------------

    fn handle_raster_liquify_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use uuid::Uuid;

        // Ensure we're on a raster layer.
        let Some(layer_id) = *shared.active_layer_id else { return; };
        let is_raster = shared.action_executor.document().get_layer(&layer_id)
            .map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));
        if !is_raster { return; }

        let radius   = shared.raster_settings.liquify_radius;
        let strength = shared.raster_settings.liquify_strength;
        let mode     = shared.raster_settings.liquify_mode.as_u32();

        // ---- Keyboard: Enter = commit, Escape = cancel ----
        let enter  = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if escape {
            if let Some(ls) = self.liquify_state.take() {
                self.pending_canvas_removal = Some(ls.display_canvas_id);
                self.painting_canvas = None;
                let _ = (ls.anchor_canvas_id, ls.disp_buf_id);
            }
            return;
        }

        if enter {
            if let Some(ref mut ls) = self.liquify_state {
                if !ls.wants_commit {
                    ls.wants_commit = true;
                    self.pending_warp_ops.push(PendingWarpOp::LiquifyApply {
                        anchor_canvas_id:  ls.anchor_canvas_id,
                        disp_buf_id:       ls.disp_buf_id,
                        display_canvas_id: ls.display_canvas_id,
                        w: ls.anchor_w, h: ls.anchor_h,
                        final_commit: true,
                        layer_id:     ls.layer_id,
                        time:         ls.time,
                        is_float_warp: ls.float_offset.is_some(),
                    });
                }
            }
            return;
        }

        // ---- Draw brush cursor ----
        let liq_rect  = response.rect;
        let screen_cx = liq_rect.min.x + self.pan_offset.x + world_pos.x * self.zoom;
        let screen_cy = liq_rect.min.y + self.pan_offset.y + world_pos.y * self.zoom;
        let screen_r  = radius * self.zoom;
        let painter   = ui.painter_at(liq_rect);
        let time      = ui.input(|i| i.time) as f32;
        let phase     = (time * 8.0).rem_euclid(8.0);

        let pts: Vec<egui::Pos2> = (0..64).map(|i| {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            egui::pos2(screen_cx + a.cos() * screen_r,
                       screen_cy + a.sin() * screen_r)
        }).collect();
        Self::draw_marching_ants(&painter, &pts, phase);

        // ---- Lazy init ----
        let playhead_time = *shared.playback_time;
        let needs_init = self.liquify_state.as_ref()
            .map_or(true, |ls| ls.layer_id != layer_id);

        if needs_init {
            if let Some(old) = self.liquify_state.take() {
                self.pending_canvas_removal = Some(old.display_canvas_id);
                self.painting_canvas = None;
            }

            // Determine anchor: floating selection on this layer, or the keyframe.
            let float_offset: Option<(i32, i32)>;
            let anchor_canvas_id: uuid::Uuid;
            let anchor_pixels: Vec<u8>;
            let w: u32;
            let h: u32;

            if let Some(float_sel) = shared.selection.raster_floating.as_ref()
                .filter(|f| f.layer_id == layer_id)
            {
                float_offset     = Some((float_sel.x, float_sel.y));
                anchor_canvas_id = float_sel.canvas_id;
                w                = float_sel.width;
                h                = float_sel.height;
                anchor_pixels    = if float_sel.pixels.is_empty() {
                    vec![0u8; (w * h * 4) as usize]
                } else {
                    (*float_sel.pixels).clone()
                };
            } else {
                float_offset = None;
                let doc = shared.action_executor.document();
                let (kf_id, kw, kh, raw_pix) = doc.get_layer(&layer_id)
                    .and_then(|l| if let lightningbeam_core::layer::AnyLayer::Raster(rl) = l {
                        rl.keyframe_at(playhead_time).map(|kf| {
                            let expected = (kf.width * kf.height * 4) as usize;
                            let mut pix = kf.raw_pixels.clone();
                            if pix.len() != expected { pix.resize(expected, 0); }
                            (kf.id, kf.width, kf.height, pix)
                        })
                    } else { None })
                    .unwrap_or_else(|| {
                        let dw = 1920u32; let dh = 1080u32;
                        (Uuid::new_v4(), dw, dh, vec![0u8; (dw * dh * 4) as usize])
                    });
                anchor_canvas_id = kf_id;
                w                = kw;
                h                = kh;
                anchor_pixels    = raw_pix;
            }

            let display_canvas_id = Uuid::new_v4();
            let disp_buf_id       = Uuid::new_v4();

            self.pending_warp_ops.push(PendingWarpOp::Init {
                anchor_canvas_id,
                display_canvas_id,
                disp_buf_id,
                w, h,
                anchor_pixels,
                is_liquify: true,
            });

            self.liquify_state = Some(LiquifyState {
                layer_id,
                time: playhead_time,
                anchor_canvas_id,
                display_canvas_id,
                disp_buf_id,
                anchor_w: w,
                anchor_h: h,
                last_brush_pos: None,
                liquify_applied: false,
                wants_commit: false,
                float_offset,
            });
        }

        // Pre-check drag states before taking the liquify_state borrow.
        let drag_started_l = self.rsp_drag_started(response);
        let dragged_l      = self.rsp_dragged(response);
        let drag_stopped_l = self.rsp_drag_stopped(response);

        // Extract what we need from liquify_state and update it, then release borrow.
        // Returns (layer_id, display_id, brush_op) where brush_op is Some if we should
        // push GPU ops this frame.
        let brush_op = {
            let ls = match self.liquify_state.as_mut() {
                Some(ls) => ls,
                None => return,
            };

            let mut op: Option<(uuid::Uuid, uuid::Uuid, uuid::Uuid, u32, u32, f64, f32, f32, f32, f32)> = None;

            if drag_started_l {
                ls.last_brush_pos = Some((world_pos.x, world_pos.y));
                ls.liquify_applied = true;
                op = Some((ls.anchor_canvas_id, ls.disp_buf_id, ls.display_canvas_id,
                           ls.anchor_w, ls.anchor_h, ls.time,
                           world_pos.x, world_pos.y, 0.0, 0.0));
            } else if dragged_l {
                if let Some((lx, ly)) = ls.last_brush_pos {
                    let dx = world_pos.x - lx;
                    let dy = world_pos.y - ly;
                    let dist2 = dx * dx + dy * dy;
                    let min_step = (radius / 4.0).max(1.0);
                    if dist2 >= min_step * min_step {
                        let len = dist2.sqrt().max(0.001);
                        ls.last_brush_pos = Some((world_pos.x, world_pos.y));
                        op = Some((ls.anchor_canvas_id, ls.disp_buf_id, ls.display_canvas_id,
                                   ls.anchor_w, ls.anchor_h, ls.time,
                                   world_pos.x, world_pos.y, dx / len, dy / len));
                    }
                }
            }
            if drag_stopped_l {
                ls.last_brush_pos = None;
            }
            let is_float = ls.float_offset.is_some();
            op.map(|o| (ls.layer_id, is_float, o))
        };

        // For full-layer liquify: override layer blit with display canvas.
        // For float liquify: override the float blit via transform_display in render_content().
        if let Some(ls) = self.liquify_state.as_ref() {
            if ls.float_offset.is_none() {
                self.painting_canvas = Some((ls.layer_id, ls.display_canvas_id));
            }
        }

        if let Some((ls_layer_id, is_float_warp, (anchor_id, disp_buf, display_id, w, h, time, cx, cy, dx, dy))) = brush_op {
            self.pending_warp_ops.push(PendingWarpOp::LiquifyBrushStep {
                disp_buf_id: disp_buf,
                params: crate::gpu_brush::LiquifyBrushParams {
                    cx, cy, radius, strength,
                    dx, dy, mode,
                    map_w: w, map_h: h,
                    _pad0: 0, _pad1: 0, _pad2: 0,
                },
            });
            self.pending_warp_ops.push(PendingWarpOp::LiquifyApply {
                anchor_canvas_id:  anchor_id,
                disp_buf_id:       disp_buf,
                display_canvas_id: display_id,
                w, h,
                final_commit: false,
                layer_id: ls_layer_id,
                time,
                is_float_warp,
            });
            ui.ctx().request_repaint();
        }
    }

    fn handle_raster_gradient_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::actions::RasterFillAction;
        use lightningbeam_core::layer::AnyLayer;

        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Delegate to the vector handler when the active layer is a vector layer.
        if let Some(AnyLayer::Vector(_)) = shared.action_executor.document().get_layer(&active_layer_id) {
            return self.handle_vector_gradient_tool(ui, response, world_pos, shared, response.rect);
        }

        let drag_started = response.drag_started();
        let dragged      = response.dragged();
        let drag_stopped = response.drag_stopped();

        // ── Drag started: snapshot pixels, create GPU canvases ───────────────
        if drag_started {
            // Determine whether we're painting on the floating selection or the layer.
            // Float: gradient writes into float.canvas_id (shown by the float path).
            // Layer: gradient writes into a new display canvas shown via painting_canvas.
            let float_info = shared.selection.raster_floating.as_ref().map(|f| {
                let pixels = if f.pixels.is_empty() {
                    vec![0u8; (f.width * f.height * 4) as usize]
                } else {
                    (*f.pixels).clone()
                };
                (pixels, f.width, f.height, f.time, f.canvas_id, f.x as f32, f.y as f32, f.layer_id)
            });

            let layer_result = if float_info.is_none() {
                let doc = shared.action_executor.document();
                let r = if let Some(layer) = doc.get_layer(&active_layer_id) {
                    if let AnyLayer::Raster(rl) = layer {
                        let time = *shared.playback_time;
                        if let Some(kf) = rl.keyframe_at(time) {
                            let w = doc.width as u32;
                            let h = doc.height as u32;
                            let pixels = if kf.raw_pixels.is_empty() {
                                vec![0u8; (w * h * 4) as usize]
                            } else { kf.raw_pixels.clone() };
                            Some((pixels, w, h, kf.time))
                        } else { None }
                    } else { None }
                } else { None };
                drop(doc);
                r
            } else { None };

            // Unpack into a common set of fields.
            let setup = if let Some((pixels, w, h, time, fid, fx, fy, flid)) = float_info {
                Some((pixels, w, h, time, flid, true, Some(fid), Some((fx, fy))))
            } else if let Some((pixels, w, h, time)) = layer_result {
                Some((pixels, w, h, time, active_layer_id, false, None, None))
            } else { None };

            if let Some((before_pixels, canvas_w, canvas_h, kf_time,
                         target_layer_id, is_float,
                         existing_display_id, float_offset)) = setup
            {
                let anchor_canvas_id  = uuid::Uuid::new_v4();
                let display_canvas_id = existing_display_id.unwrap_or_else(uuid::Uuid::new_v4);

                // Convert world drag-start to canvas-local coords.
                let (sx, sy) = if let Some((fx, fy)) = float_offset {
                    (world_pos.x - fx, world_pos.y - fy)
                } else {
                    (world_pos.x, world_pos.y)
                };

                let gpu_stops = Self::gradient_to_gpu_stops(&shared.raster_settings.gradient);
                let gradient  = &shared.raster_settings.gradient;

                self.gradient_state = Some(GradientState {
                    layer_id: target_layer_id,
                    time: kf_time,
                    start: world_pos,
                    end:   world_pos,
                    before_pixels: before_pixels.clone(),
                    canvas_w,
                    canvas_h,
                    anchor_canvas_id,
                    display_canvas_id,
                    is_float,
                    float_offset,
                });

                self.pending_gradient_op = Some(PendingGradientOp {
                    anchor_canvas_id,
                    display_canvas_id,
                    w: canvas_w,
                    h: canvas_h,
                    anchor_pixels: Some(before_pixels),
                    start_x: sx, start_y: sy,
                    end_x:   sx, end_y:   sy,
                    opacity:     shared.raster_settings.gradient_opacity,
                    extend_mode: Self::gradient_extend_to_u32(gradient.extend),
                    kind:        Self::gradient_kind_to_u32(gradient.kind),
                    stops:       gpu_stops,
                });

                // For layer gradient show a separate display canvas via painting_canvas.
                // For float gradient the float's own canvas_id IS display_canvas_id
                // and is already shown by the float rendering path.
                if !is_float {
                    self.painting_canvas = Some((target_layer_id, display_canvas_id));
                }
                ui.ctx().request_repaint();
            }
        }

        // ── Dragged: update end point, queue GPU dispatch ─────────────────────
        // Skip on the same frame as drag_started — that block already queued the initial
        // GPU op with anchor_pixels = Some(...).  Overwriting it here would lose the upload.
        if dragged && !drag_started {
            if let Some(ref mut gs) = self.gradient_state {
                gs.end = world_pos;
            }
            if let Some(ref gs) = self.gradient_state {
                let gradient = &shared.raster_settings.gradient;
                // Convert world coords to canvas-local (subtract float offset if needed).
                let to_local = |v: egui::Vec2| -> (f32, f32) {
                    if let Some((fx, fy)) = gs.float_offset {
                        (v.x - fx, v.y - fy)
                    } else {
                        (v.x, v.y)
                    }
                };
                let (sx, sy) = to_local(gs.start);
                let (ex, ey) = to_local(gs.end);
                self.pending_gradient_op = Some(PendingGradientOp {
                    anchor_canvas_id:  gs.anchor_canvas_id,
                    display_canvas_id: gs.display_canvas_id,
                    w: gs.canvas_w,
                    h: gs.canvas_h,
                    anchor_pixels: None,  // already on GPU
                    start_x: sx, start_y: sy,
                    end_x:   ex, end_y:   ey,
                    opacity:     shared.raster_settings.gradient_opacity,
                    extend_mode: Self::gradient_extend_to_u32(gradient.extend),
                    kind:        Self::gradient_kind_to_u32(gradient.kind),
                    stops:       Self::gradient_to_gpu_stops(gradient),
                });
                ui.ctx().request_repaint();
            }
        }

        // ── Drag stopped: commit ──────────────────────────────────────────────
        if drag_stopped {
            if let Some(ref mut gs) = self.gradient_state {
                gs.end = world_pos;
            }
            if let Some(ref gs) = self.gradient_state {
                let after_pixels = Self::compute_gradient_pixels(gs, shared);
                if gs.is_float {
                    // Update the float's pixel buffer in place.
                    // The float's GPU canvas (display_canvas_id) already shows the result.
                    if let Some(ref mut float) = shared.selection.raster_floating {
                        float.pixels = std::sync::Arc::new(after_pixels);
                    }
                } else {
                    let action = RasterFillAction::new(
                        gs.layer_id, gs.time,
                        gs.before_pixels.clone(), after_pixels,
                        gs.canvas_w, gs.canvas_h,
                    ).with_description("Gradient Fill");
                    let _ = shared.action_executor.execute(Box::new(action));
                }
            }
            if let Some(gs) = self.gradient_state.take() {
                // Always remove the anchor canvas (temporary scratch).
                // For layer gradient, also remove the display canvas.
                // For float gradient, display_canvas_id IS the float's canvas — keep it.
                if gs.is_float {
                    self.pending_canvas_removal = Some(gs.anchor_canvas_id);
                } else {
                    self.pending_canvas_removal = Some(gs.display_canvas_id);
                    // Anchor leaks here (pre-existing behaviour); acceptable for now.
                }
            }
            self.painting_canvas = None;
        }

        // Keep painting_canvas pointing at the display canvas each frame (layer gradient only).
        if let Some(ref gs) = self.gradient_state {
            if !gs.is_float {
                self.painting_canvas = Some((gs.layer_id, gs.display_canvas_id));
            }
        }

        // Draw direction line overlay.
        if let Some(ref gs) = self.gradient_state {
            let zoom = self.zoom;
            let pan  = self.pan_offset;
            let world_to_screen = |v: egui::Vec2| egui::pos2(v.x * zoom + pan.x, v.y * zoom + pan.y);
            let p0 = world_to_screen(gs.start);
            let p1 = world_to_screen(gs.end);
            let painter = ui.painter();
            painter.line_segment(
                [p0, p1],
                egui::Stroke::new(1.5, egui::Color32::WHITE),
            );
            painter.circle_filled(p0, 5.0, egui::Color32::WHITE);
            painter.circle_filled(p1, 5.0, egui::Color32::WHITE);
            painter.circle_stroke(p0, 5.0, egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));
            painter.circle_stroke(p1, 5.0, egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));
        }
    }

    fn gradient_extend_to_u32(extend: lightningbeam_core::gradient::GradientExtend) -> u32 {
        use lightningbeam_core::gradient::GradientExtend;
        match extend {
            GradientExtend::Pad     => 0,
            GradientExtend::Reflect => 1,
            GradientExtend::Repeat  => 2,
        }
    }

    fn gradient_kind_to_u32(kind: lightningbeam_core::gradient::GradientType) -> u32 {
        use lightningbeam_core::gradient::GradientType;
        match kind {
            GradientType::Linear => 0,
            GradientType::Radial => 1,
        }
    }

    /// Convert gradient stops to GPU-ready form (sRGB u8 → linear f32).
    fn gradient_to_gpu_stops(gradient: &lightningbeam_core::gradient::ShapeGradient) -> Vec<crate::gpu_brush::GpuGradientStop> {
        gradient.stops.iter().map(|s| {
            crate::gpu_brush::GpuGradientStop::from_srgb_u8(
                s.position, s.color.r, s.color.g, s.color.b, s.color.a,
            )
        }).collect()
    }

    /// Compute gradient-filled pixel buffer (CPU), respecting active selection.
    ///
    /// All blending is done in linear premultiplied space to match the GPU shader.
    fn compute_gradient_pixels(gs: &GradientState, shared: &SharedPaneState) -> Vec<u8> {
        let w = gs.canvas_w;
        let h = gs.canvas_h;
        let gradient = &shared.raster_settings.gradient;
        let opacity  = shared.raster_settings.gradient_opacity;

        // Selection confinement (not applicable to float — the float IS the selection).
        let sel = if gs.is_float { None } else { shared.selection.raster_selection.as_ref() };

        // Convert world start/end to canvas-local coords (subtract float offset if any).
        let (start_x, start_y) = if let Some((fx, fy)) = gs.float_offset {
            (gs.start.x - fx, gs.start.y - fy)
        } else {
            (gs.start.x, gs.start.y)
        };
        let (end_x, end_y) = if let Some((fx, fy)) = gs.float_offset {
            (gs.end.x - fx, gs.end.y - fy)
        } else {
            (gs.end.x, gs.end.y)
        };

        let dx = end_x - start_x;
        let dy = end_y - start_y;
        let len2 = dx * dx + dy * dy;
        let is_radial = gradient.kind == lightningbeam_core::gradient::GradientType::Radial;

        // sRGB ↔ linear helpers (match gpu_brush.rs).
        let srgb_to_linear = |c: f32| -> f32 {
            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        let linear_to_srgb = |c: f32| -> f32 {
            let c = c.clamp(0.0, 1.0);
            if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
        };

        let mut out = gs.before_pixels.clone();

        for py in 0..h {
            for px in 0..w {
                let idx = ((py * w + px) * 4) as usize;

                let cx_f = px as f32 + 0.5;
                let cy_f = py as f32 + 0.5;
                let t_raw = if is_radial {
                    // Radial: center at start point, radius = |end-start|.
                    let radius = len2.sqrt();
                    if radius < 0.5 { 0.0f32 } else {
                        let pdx = cx_f - start_x;
                        let pdy = cy_f - start_y;
                        (pdx * pdx + pdy * pdy).sqrt() / radius
                    }
                } else {
                    // Linear: project pixel centre onto gradient axis.
                    if len2 < 1.0 { 0.0f32 } else {
                        let fx = cx_f - start_x;
                        let fy = cy_f - start_y;
                        (fx * dx + fy * dy) / len2
                    }
                };

                let t = gradient.apply_extend(t_raw);
                let [gr, gg, gb, ga] = gradient.eval(t);

                // Selection confinement.
                if let Some(s) = sel {
                    if !s.contains_pixel(px as i32, py as i32) {
                        continue;
                    }
                }

                // Effective alpha: gradient alpha × tool opacity (straight-alpha [0,1]).
                let a = ga as f32 / 255.0 * opacity;

                // Convert gradient RGB from sRGB straight-alpha to linear straight-alpha.
                let gr_lin = srgb_to_linear(gr as f32 / 255.0);
                let gg_lin = srgb_to_linear(gg as f32 / 255.0);
                let gb_lin = srgb_to_linear(gb as f32 / 255.0);

                // Source pixel: sRGB premultiplied bytes → linear premultiplied floats.
                // (upload() does the same conversion for the GPU anchor canvas.)
                let src_r_lin = srgb_to_linear(out[idx]     as f32 / 255.0);
                let src_g_lin = srgb_to_linear(out[idx + 1] as f32 / 255.0);
                let src_b_lin = srgb_to_linear(out[idx + 2] as f32 / 255.0);
                let src_a     = out[idx + 3] as f32 / 255.0;

                // Alpha-over in linear premultiplied space (matches GPU shader exactly).
                let out_a       = a + src_a * (1.0 - a);
                let out_r_lin   = gr_lin * a + src_r_lin * (1.0 - a);
                let out_g_lin   = gg_lin * a + src_g_lin * (1.0 - a);
                let out_b_lin   = gb_lin * a + src_b_lin * (1.0 - a);

                // Convert linear premultiplied → sRGB premultiplied bytes.
                out[idx]     = (linear_to_srgb(out_r_lin) * 255.0 + 0.5) as u8;
                out[idx + 1] = (linear_to_srgb(out_g_lin) * 255.0 + 0.5) as u8;
                out[idx + 2] = (linear_to_srgb(out_b_lin) * 255.0 + 0.5) as u8;
                out[idx + 3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
            }
        }

        out
    }

    /// Handle the Gradient tool when the active layer is a vector layer.
    ///
    /// Drag start→end across a face to set its gradient angle. On release the
    /// current gradient settings (stops, kind, extend) are applied via
    /// `SetFillPaintAction`, which records an undo entry.
    fn handle_vector_gradient_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
        rect: egui::Rect,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::dcel2::FaceId;

        let Some(layer_id) = *shared.active_layer_id else { return };

        // ── Drag started: pick the face under the click origin ───────────────
        if response.drag_started() {
            let click_world = ui
                .input(|i| i.pointer.press_origin())
                .map(|p| {
                    let rel = p - rect.min - self.pan_offset;
                    egui::Vec2::new(rel.x / self.zoom, rel.y / self.zoom)
                })
                .unwrap_or(world_pos);

            let doc = shared.action_executor.document();
            let Some(AnyLayer::Vector(vl)) = doc.get_layer(&layer_id) else { return };
            let Some(kf) = vl.keyframe_at(*shared.playback_time) else { return };

            let point = vello::kurbo::Point::new(click_world.x as f64, click_world.y as f64);
            let face_id = kf.dcel.find_face_containing_point(point);

            // Face 0 is the unbounded background face — nothing to fill.
            if face_id == FaceId(0) || kf.dcel.face(face_id).deleted { return; }

            // If the clicked face is already selected, apply to all selected faces;
            // otherwise apply only to the clicked face.
            let face_ids: Vec<FaceId> = if shared.selection.selected_faces().contains(&face_id) {
                shared.selection.selected_faces().iter().cloned().collect()
            } else {
                vec![face_id]
            };

            self.vector_gradient_state = Some(VectorGradientState {
                layer_id,
                time: *shared.playback_time,
                face_ids,
                start: click_world,
                end:   click_world,
            });
        }

        // ── Dragged: update end point ─────────────────────────────────────────
        if let Some(ref mut gs) = self.vector_gradient_state {
            if response.dragged() {
                gs.end = world_pos;
            }
        }

        // ── Drag stopped: commit gradient ─────────────────────────────────────
        if response.drag_stopped() {
            if let Some(gs) = self.vector_gradient_state.take() {
                let dx = gs.end.x - gs.start.x;
                let dy = gs.end.y - gs.start.y;
                // Tiny / no drag → keep the angle stored in the current gradient settings.
                let angle = if dx.abs() < 0.5 && dy.abs() < 0.5 {
                    shared.raster_settings.gradient.angle
                } else {
                    dy.atan2(dx).to_degrees()
                };

                let gradient = lightningbeam_core::gradient::ShapeGradient {
                    kind:   shared.raster_settings.gradient.kind,
                    stops:  shared.raster_settings.gradient.stops.clone(),
                    angle,
                    extend: shared.raster_settings.gradient.extend,
                    start_world: Some((gs.start.x as f64, gs.start.y as f64)),
                    end_world:   Some((gs.end.x as f64, gs.end.y as f64)),
                };

                use lightningbeam_core::actions::SetFillPaintAction;
                let action = SetFillPaintAction::gradient(
                    gs.layer_id, gs.time, gs.face_ids, Some(gradient),
                );
                if let Err(e) = shared.action_executor.execute(Box::new(action)) {
                    eprintln!("Vector gradient fill: {e}");
                }
            }
        }
    }

    fn handle_transform_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Point;

        // Raster floating selection on a raster layer → raster transform path.
        if let Some(active_id) = *shared.active_layer_id {
            let is_raster = shared.action_executor.document().get_layer(&active_id)
                .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));
            if is_raster && shared.selection.raster_floating.is_some() {
                return self.handle_raster_transform_tool(ui, response, world_pos, shared);
            }
        }

        // Check if we have an active layer
        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Check layer type - support VectorLayer (with selection) and VideoLayer (visible clip at playback time)
        let is_vector_layer;
        let is_video_layer;
        {
            let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
                Some(layer) => layer,
                None => return,
            };

            is_vector_layer = matches!(active_layer, AnyLayer::Vector(_));
            is_video_layer = matches!(active_layer, AnyLayer::Video(_));
        }

        // For vector layers, need a selection to transform
        // For video layers, transform the visible clip at playback time
        if is_vector_layer && shared.selection.is_empty() {
            return;
        } else if !is_vector_layer && !is_video_layer {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // For video layers, transform the visible clip at playback time (no selection needed)
        if is_video_layer {
            self.handle_transform_video_clip(ui, response, point, &active_layer_id, shared);
            return;
        }

        // For vector layers with DCEL selection, use DCEL-specific transform path
        if shared.selection.has_dcel_selection() {
            self.handle_transform_dcel(ui, response, point, &active_layer_id, shared);
            return;
        }

        // For vector layers: single object uses rotated bbox, multiple objects use axis-aligned bbox
        let total_selected = shared.selection.clip_instances().len();
        if total_selected == 1 {
            // Single object - rotated bounding box
            self.handle_transform_single_object(ui, response, point, &active_layer_id, shared);
        } else {
            // Multiple objects - axis-aligned bounding box
            // Calculate combined bounding box for handle hit testing
            let mut combined_bbox: Option<vello::kurbo::Rect> = None;

            // Get immutable reference just for bbox calculation
            if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                // TODO: DCEL - shape instance bbox calculation disabled during migration
                // (was: get_shape_in_keyframe to compute combined bbox for shape instances)

                // Calculate bounding box for clip instances
                for &clip_id in shared.selection.clip_instances() {
                    if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == clip_id) {
                        // Calculate clip-local time
                        let clip_time = ((*shared.playback_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

                        // Get dynamic clip bounds from content at current time
                        use vello::kurbo::Rect as KurboRect;
                        let clip_bbox = if let Some(vector_clip) = shared.action_executor.document().get_vector_clip(&clip_instance.clip_id) {
                            vector_clip.calculate_content_bounds(shared.action_executor.document(), clip_time)
                        } else if let Some(video_clip) = shared.action_executor.document().get_video_clip(&clip_instance.clip_id) {
                            KurboRect::new(0.0, 0.0, video_clip.width, video_clip.height)
                        } else {
                            continue; // Clip not found or is audio
                        };

                        println!("Multi-object clip bbox: clip_id={}, bbox=({:.1}, {:.1}, {:.1}, {:.1}), size={:.1}x{:.1}",
                                 clip_instance.clip_id, clip_bbox.x0, clip_bbox.y0, clip_bbox.x1, clip_bbox.y1,
                                 clip_bbox.width(), clip_bbox.height());

                        // Apply clip instance transform
                        let clip_transform = clip_instance.transform.to_affine();

                        println!("  Transform: x={}, y={}, scale_x={}, scale_y={}, rotation={}",
                                 clip_instance.transform.x, clip_instance.transform.y,
                                 clip_instance.transform.scale_x, clip_instance.transform.scale_y,
                                 clip_instance.transform.rotation);
                        let transformed_bbox = clip_transform.transform_rect_bbox(clip_bbox);

                        combined_bbox = Some(match combined_bbox {
                            None => transformed_bbox,
                            Some(existing) => existing.union(transformed_bbox),
                        });
                    }
                }
            }

            let bbox = match combined_bbox {
                Some(b) => b,
                None => return,
            };

            // Set cursor based on hovering over handles
            let tolerance = 10.0;
            if let Some(mode) = Self::hit_test_transform_handle(point, bbox, tolerance) {
                use lightningbeam_core::tool::TransformMode;
                let cursor = match mode {
                    TransformMode::ScaleCorner { origin } => {
                        // Determine which corner based on origin
                        if (origin.x - bbox.x0).abs() < 0.1 && (origin.y - bbox.y0).abs() < 0.1 {
                            egui::CursorIcon::ResizeNwSe // Top-left
                        } else if (origin.x - bbox.x1).abs() < 0.1 && (origin.y - bbox.y0).abs() < 0.1 {
                            egui::CursorIcon::ResizeNeSw // Top-right
                        } else if (origin.x - bbox.x1).abs() < 0.1 && (origin.y - bbox.y1).abs() < 0.1 {
                            egui::CursorIcon::ResizeNwSe // Bottom-right
                        } else {
                            egui::CursorIcon::ResizeNeSw // Bottom-left
                        }
                    }
                    TransformMode::ScaleEdge { axis, .. } => {
                        use lightningbeam_core::tool::Axis;
                        match axis {
                            Axis::Horizontal => egui::CursorIcon::ResizeHorizontal,
                            Axis::Vertical => egui::CursorIcon::ResizeVertical,
                        }
                    }
                    TransformMode::Rotate { .. } => egui::CursorIcon::AllScroll,
                    TransformMode::Skew { axis, .. } => {
                        use lightningbeam_core::tool::Axis;
                        // Use Move cursor to indicate skew
                        match axis {
                            Axis::Horizontal => egui::CursorIcon::ResizeHorizontal,
                            Axis::Vertical => egui::CursorIcon::ResizeVertical,
                        }
                    }
                };
                ui.ctx().set_cursor_icon(cursor);
            }

            // Mouse down: check if clicking on a handle
            if self.rsp_drag_started(response) || self.rsp_clicked(response) {
                let tolerance = 10.0; // Click tolerance in world space

                if let Some(mode) = Self::hit_test_transform_handle(point, bbox, tolerance) {
                // Store original transforms of all selected objects (shape instances and clip instances)
                use std::collections::HashMap;
                let mut original_transforms = HashMap::new();

                if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                    // TODO: DCEL - shape instance transform storage disabled during migration
                    // (was: get_shape_in_keyframe for each selected shape instance)

                    // Store clip instance transforms
                    for &clip_id in shared.selection.clip_instances() {
                        if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == clip_id) {
                            original_transforms.insert(clip_id, clip_instance.transform.clone());
                        }
                    }
                }

                println!("=== TRANSFORM START ===");
                println!("Mode: {:?}", mode);
                println!("Bbox: x0={:.1}, y0={:.1}, x1={:.1}, y1={:.1}", bbox.x0, bbox.y0, bbox.x1, bbox.y1);
                println!("Start mouse: ({:.1}, {:.1})", point.x, point.y);

                *shared.tool_state = ToolState::Transforming {
                    mode,
                    original_transforms,
                    pivot: bbox.center(),
                    start_mouse: point,
                    current_mouse: point,
                    original_bbox: bbox,  // Store the bbox at start of transform
                };
            }
        }

            // Mouse drag: update current mouse position and apply transforms
            if self.rsp_dragged(response) {
                if let ToolState::Transforming { mode, original_transforms, pivot, start_mouse, original_bbox, .. } = shared.tool_state.clone() {
                    // Update current mouse position
                    *shared.tool_state = ToolState::Transforming {
                        mode,
                        original_transforms: original_transforms.clone(),
                        pivot,
                        start_mouse,
                        current_mouse: point,
                        original_bbox,
                    };

                    // Get mutable access to layer to apply transform preview
                    if let Some(layer) = shared.action_executor.document_mut().get_layer_mut(&active_layer_id) {
                        if let AnyLayer::Vector(vector_layer) = layer {
                            Self::apply_transform_preview(
                                vector_layer,
                                &mode,
                                &original_transforms,
                                pivot,
                                start_mouse,
                                point,
                                original_bbox,
                                *shared.playback_time,
                            );
                        }
                    }
                }
            }

            // Mouse up: finalize transform
            if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::Transforming { .. })) {
                if let ToolState::Transforming { original_transforms, .. } = shared.tool_state.clone() {
                    use std::collections::HashMap;
                    use lightningbeam_core::actions::TransformClipInstancesAction;

                    let mut clip_instance_transforms = HashMap::new();

                    // Get current transforms and pair with originals
                    if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                        for (object_id, original) in original_transforms {
                            if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == object_id) {
                                let new_transform = clip_instance.transform.clone();
                                clip_instance_transforms.insert(object_id, (original, new_transform));
                            }
                        }
                    }

                    // Create action for clip instances
                    if !clip_instance_transforms.is_empty() {
                        let action = TransformClipInstancesAction::new(active_layer_id, *shared.playback_time, clip_instance_transforms);
                        shared.pending_actions.push(Box::new(action));
                    }

                    *shared.tool_state = ToolState::Idle;
                }
            }
        } // End of multi-object else block
    }

    /// Handle transform tool for a single object with rotated bounding box
    fn handle_transform_single_object(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        point: vello::kurbo::Point,
        active_layer_id: &uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::tool::ToolState;
        use lightningbeam_core::layer::AnyLayer;
        use vello::kurbo::Affine;

        // Get the single selected object (either shape instance or clip instance)
        let object_id = if let Some(&id) = shared.selection.clip_instances().iter().next() {
            id
        } else {
            return; // No selection, shouldn't happen
        };

        // Calculate rotated bounding box corners
        let (local_bbox, world_corners, obj_transform, transform) = {
            if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                // TODO: DCEL - shape instance bbox for single-object transform disabled during migration
                // Try clip instance
                if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == object_id) {
                    // Calculate clip-local time
                    let clip_time = ((*shared.playback_time - clip_instance.timeline_start) * clip_instance.playback_speed) + clip_instance.trim_start;

                    // Get dynamic clip bounds from content at current time
                    let local_bbox = if let Some(vector_clip) = shared.action_executor.document().get_vector_clip(&clip_instance.clip_id) {
                        vector_clip.calculate_content_bounds(shared.action_executor.document(), clip_time)
                    } else if let Some(video_clip) = shared.action_executor.document().get_video_clip(&clip_instance.clip_id) {
                        vello::kurbo::Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
                    } else {
                        return; // Clip not found or is audio
                    };

                    println!("Single-object clip bbox: clip_id={}, bbox=({:.1}, {:.1}, {:.1}, {:.1}), size={:.1}x{:.1}",
                             clip_instance.clip_id, local_bbox.x0, local_bbox.y0, local_bbox.x1, local_bbox.y1,
                             local_bbox.width(), local_bbox.height());

                    let local_corners = [
                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y0),
                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y0),
                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y1),
                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y1),
                    ];

                    // Clip instances don't have skew, so transform is simpler
                    let obj_transform = Affine::translate((clip_instance.transform.x, clip_instance.transform.y))
                        * Affine::rotate(clip_instance.transform.rotation.to_radians())
                        * Affine::scale_non_uniform(clip_instance.transform.scale_x, clip_instance.transform.scale_y);

                    let world_corners: Vec<vello::kurbo::Point> = local_corners
                        .iter()
                        .map(|&p| obj_transform * p)
                        .collect();

                    (local_bbox, world_corners, obj_transform, clip_instance.transform.clone())
                } else {
                    return;
                }
            } else if let Some(AnyLayer::Video(video_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                // Handle Video layer clip instance
                if let Some(clip_instance) = video_layer.clip_instances.iter().find(|ci| ci.id == object_id) {
                    // Get video clip dimensions for bounding box
                    let local_bbox = if let Some(video_clip) = shared.action_executor.document().get_video_clip(&clip_instance.clip_id) {
                        vello::kurbo::Rect::new(0.0, 0.0, video_clip.width, video_clip.height)
                    } else {
                        return; // Video clip not found
                    };

                    let local_corners = [
                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y0),
                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y0),
                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y1),
                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y1),
                    ];

                    // Video clip instances use the same transform as vector clip instances
                    let obj_transform = Affine::translate((clip_instance.transform.x, clip_instance.transform.y))
                        * Affine::rotate(clip_instance.transform.rotation.to_radians())
                        * Affine::scale_non_uniform(clip_instance.transform.scale_x, clip_instance.transform.scale_y);

                    let world_corners: Vec<vello::kurbo::Point> = local_corners
                        .iter()
                        .map(|&p| obj_transform * p)
                        .collect();

                    (local_bbox, world_corners, obj_transform, clip_instance.transform.clone())
                } else {
                    return;
                }
            } else {
                return;
            }
        };

        // === Calculate ALL handle positions once (shared by cursor and click logic) ===
        let tolerance = 15.0;

        // Edge midpoints
        let edge_midpoints = [
            vello::kurbo::Point::new((world_corners[0].x + world_corners[1].x) / 2.0, (world_corners[0].y + world_corners[1].y) / 2.0),
            vello::kurbo::Point::new((world_corners[1].x + world_corners[2].x) / 2.0, (world_corners[1].y + world_corners[2].y) / 2.0),
            vello::kurbo::Point::new((world_corners[2].x + world_corners[3].x) / 2.0, (world_corners[2].y + world_corners[3].y) / 2.0),
            vello::kurbo::Point::new((world_corners[3].x + world_corners[0].x) / 2.0, (world_corners[3].y + world_corners[0].y) / 2.0),
        ];

        // Rotation handle position
        let rotation_rad = transform.rotation.to_radians();
        let cos_r = rotation_rad.cos();
        let sin_r = rotation_rad.sin();
        let rotation_handle_offset = 20.0;
        let top_center = edge_midpoints[0];
        let offset_x = -(-rotation_handle_offset) * sin_r;
        let offset_y = -rotation_handle_offset * cos_r;
        let rotation_handle_pos = vello::kurbo::Point::new(top_center.x + offset_x, top_center.y + offset_y);

        // === Set cursor based on hover (using the same handle positions) ===
        if point.distance(rotation_handle_pos) < tolerance {
            ui.ctx().set_cursor_icon(egui::CursorIcon::AllScroll); // 4-way arrows for rotation
        } else {
            let mut hovering_handle = false;

            // Check corner handles with correct diagonal cursors
            for (idx, corner) in world_corners.iter().enumerate() {
                if point.distance(*corner) < tolerance {
                    // Top-left (0) and bottom-right (2): NW-SE diagonal (\)
                    // Top-right (1) and bottom-left (3): NE-SW diagonal (/)
                    let cursor = match idx {
                        0 | 2 => egui::CursorIcon::ResizeNwSe, // Top-left, Bottom-right
                        1 | 3 => egui::CursorIcon::ResizeNeSw, // Top-right, Bottom-left
                        _ => egui::CursorIcon::Default,
                    };
                    ui.ctx().set_cursor_icon(cursor);
                    hovering_handle = true;
                    break;
                }
            }

            // Check edge handles
            if !hovering_handle {
                for (idx, edge_pos) in edge_midpoints.iter().enumerate() {
                    if point.distance(*edge_pos) < tolerance {
                        let cursor = match idx {
                            0 | 2 => egui::CursorIcon::ResizeVertical,   // Top/Bottom
                            1 | 3 => egui::CursorIcon::ResizeHorizontal, // Right/Left
                            _ => egui::CursorIcon::Default,
                        };
                        ui.ctx().set_cursor_icon(cursor);
                        hovering_handle = true;
                        break;
                    }
                }
            }

            // Check for skew (hovering over edge but not near handles)
            if !hovering_handle {
                let skew_tolerance = tolerance * 1.5;

                // Check each edge
                for i in 0..4 {
                    let start = world_corners[i];
                    let end = world_corners[(i + 1) % 4];
                    let edge_midpoint = edge_midpoints[i];

                    // Calculate distance from point to line segment
                    let edge_vec = end - start;
                    let point_vec = point - start;
                    let edge_length = edge_vec.hypot();

                    if edge_length > 0.0 {
                        // Project point onto line segment
                        let t = (point_vec.x * edge_vec.x + point_vec.y * edge_vec.y) / (edge_length * edge_length);

                        // Check if projection is within segment bounds
                        if t > 0.0 && t < 1.0 {
                            let closest_point = start + edge_vec * t;
                            let distance = point.distance(closest_point);

                            // Check if close to edge but not near corner or midpoint handles
                            if distance < skew_tolerance {
                                let near_corner = point.distance(start) < tolerance || point.distance(end) < tolerance;
                                let near_midpoint = point.distance(edge_midpoint) < tolerance;

                                if !near_corner && !near_midpoint {
                                    // Show skew cursor
                                    let cursor = match i {
                                        0 | 2 => egui::CursorIcon::ResizeHorizontal, // Top/Bottom edges
                                        1 | 3 => egui::CursorIcon::ResizeVertical,   // Right/Left edges
                                        _ => egui::CursorIcon::Default,
                                    };
                                    ui.ctx().set_cursor_icon(cursor);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // === Mouse down: hit test handles (using the same handle positions and order as cursor logic) ===
        let should_start_transform = (self.rsp_drag_started(response) || self.rsp_clicked(response))
            || (matches!(*shared.tool_state, ToolState::Idle) && self.rsp_primary_down(ui) && response.hovered());

        if should_start_transform && matches!(*shared.tool_state, ToolState::Idle) {
            // Check rotation handle (same as cursor logic)
            if point.distance(rotation_handle_pos) < tolerance {

                // Start rotation around the visual center of the shape
                // Calculate local center
                let local_center = vello::kurbo::Point::new(
                    (local_bbox.x0 + local_bbox.x1) / 2.0,
                    (local_bbox.y0 + local_bbox.y1) / 2.0,
                );

                // Transform to world space to get the visual center
                let visual_center = obj_transform * local_center;

                use std::collections::HashMap;
                let mut original_transforms = HashMap::new();
                original_transforms.insert(object_id, transform.clone());

                *shared.tool_state = ToolState::Transforming {
                    mode: lightningbeam_core::tool::TransformMode::Rotate { center: visual_center },
                    original_transforms,
                    pivot: visual_center,
                    start_mouse: point,
                    current_mouse: point,
                    original_bbox: vello::kurbo::Rect::new(local_bbox.x0, local_bbox.y0, local_bbox.x1, local_bbox.y1),
                };
                return;
            }

            // Check corner handles
            for (idx, corner) in world_corners.iter().enumerate() {
                if point.distance(*corner) < tolerance {
                    // Get opposite corner in local space
                    let opposite_idx = (idx + 2) % 4;

                    use std::collections::HashMap;
                    let mut original_transforms = HashMap::new();
                    original_transforms.insert(object_id, transform.clone());

                    *shared.tool_state = ToolState::Transforming {
                        mode: lightningbeam_core::tool::TransformMode::ScaleCorner {
                            origin: world_corners[opposite_idx],
                        },
                        original_transforms,
                        pivot: world_corners[opposite_idx],
                        start_mouse: point,
                        current_mouse: point,
                        original_bbox: vello::kurbo::Rect::new(local_bbox.x0, local_bbox.y0, local_bbox.x1, local_bbox.y1),
                    };
                    return;
                }
            }

            // Check edge handles
            for (idx, edge_pos) in edge_midpoints.iter().enumerate() {
                if point.distance(*edge_pos) < tolerance {
                    use std::collections::HashMap;
                    use lightningbeam_core::tool::Axis;

                    let mut original_transforms = HashMap::new();
                    original_transforms.insert(object_id, transform.clone());

                    // Determine axis and opposite edge
                    let (axis, opposite_edge) = match idx {
                        0 => (Axis::Vertical, edge_midpoints[2]),   // Top -> opposite is Bottom
                        1 => (Axis::Horizontal, edge_midpoints[3]), // Right -> opposite is Left
                        2 => (Axis::Vertical, edge_midpoints[0]),   // Bottom -> opposite is Top
                        3 => (Axis::Horizontal, edge_midpoints[1]), // Left -> opposite is Right
                        _ => unreachable!(),
                    };

                    *shared.tool_state = ToolState::Transforming {
                        mode: lightningbeam_core::tool::TransformMode::ScaleEdge {
                            axis,
                            origin: opposite_edge,
                        },
                        original_transforms,
                        pivot: opposite_edge,
                        start_mouse: point,
                        current_mouse: point,
                        original_bbox: vello::kurbo::Rect::new(local_bbox.x0, local_bbox.y0, local_bbox.x1, local_bbox.y1),
                    };
                    return;
                }
            }

            // Check for skew (same logic as cursor hover)
            let skew_tolerance = tolerance * 1.5;
            for i in 0..4 {
                let start = world_corners[i];
                let end = world_corners[(i + 1) % 4];
                let edge_midpoint = edge_midpoints[i];

                let edge_vec = end - start;
                let point_vec = point - start;
                let edge_length = edge_vec.hypot();

                if edge_length > 0.0 {
                    let t = (point_vec.x * edge_vec.x + point_vec.y * edge_vec.y) / (edge_length * edge_length);

                    if t > 0.0 && t < 1.0 {
                        let closest_point = start + edge_vec * t;
                        let distance = point.distance(closest_point);

                        if distance < skew_tolerance {
                            let near_corner = point.distance(start) < tolerance || point.distance(end) < tolerance;
                            let near_midpoint = point.distance(edge_midpoint) < tolerance;

                            if !near_corner && !near_midpoint {
                                use std::collections::HashMap;
                                use lightningbeam_core::tool::Axis;

                                let mut original_transforms = HashMap::new();
                                original_transforms.insert(object_id, transform.clone());

                                // Determine skew axis and origin
                                let (axis, opposite_edge) = match i {
                                    0 => (Axis::Horizontal, edge_midpoints[2]), // Top edge
                                    1 => (Axis::Vertical, edge_midpoints[3]),   // Right edge
                                    2 => (Axis::Horizontal, edge_midpoints[0]), // Bottom edge
                                    3 => (Axis::Vertical, edge_midpoints[1]),   // Left edge
                                    _ => unreachable!(),
                                };

                                *shared.tool_state = ToolState::Transforming {
                                    mode: lightningbeam_core::tool::TransformMode::Skew {
                                        axis,
                                        origin: opposite_edge,
                                    },
                                    original_transforms,
                                    pivot: opposite_edge,
                                    start_mouse: point,
                                    current_mouse: point,
                                    original_bbox: vello::kurbo::Rect::new(local_bbox.x0, local_bbox.y0, local_bbox.x1, local_bbox.y1),
                                };
                                return;
                            }
                        }
                    }
                }
            }
        }

        // Mouse drag: apply transform in local space
        if self.rsp_dragged(response) {
            if let ToolState::Transforming { mode, original_transforms, start_mouse, current_mouse: _, .. } = shared.tool_state.clone() {
                // Update current mouse
                if let ToolState::Transforming { mode, original_transforms, pivot, start_mouse, original_bbox, current_mouse: _ } = shared.tool_state.clone() {
                    *shared.tool_state = ToolState::Transforming {
                        mode,
                        original_transforms: original_transforms.clone(),
                        pivot,
                        start_mouse,
                        current_mouse: point,
                        original_bbox,
                    };
                }

                // Apply transform in LOCAL space (much simpler!)
                if let Some(layer) = shared.action_executor.document_mut().get_layer_mut(active_layer_id) {
                    if let AnyLayer::Vector(vector_layer) = layer {
                        if let Some(original) = original_transforms.get(&object_id) {
                            match mode {
                                lightningbeam_core::tool::TransformMode::ScaleCorner { origin } => {
                                    // Use ORIGINAL transform to avoid numerical issues when scale is small
                                    let original_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let inv_original_transform = original_transform.inverse();

                                    // Transform mouse positions to local space using original transform
                                    let local_start = inv_original_transform * start_mouse;
                                    let local_current = inv_original_transform * point;
                                    let local_origin = inv_original_transform * origin;

                                    // Calculate scale in local space
                                    let start_dx = local_start.x - local_origin.x;
                                    let start_dy = local_start.y - local_origin.y;
                                    let current_dx = local_current.x - local_origin.x;
                                    let current_dy = local_current.y - local_origin.y;

                                    let scale_x = if start_dx.abs() > 0.001 {
                                        current_dx / start_dx
                                    } else {
                                        1.0
                                    };

                                    let scale_y = if start_dy.abs() > 0.001 {
                                        current_dy / start_dy
                                    } else {
                                        1.0
                                    };

                                    // Calculate new scale values
                                    let new_scale_x = original.scale_x * scale_x;
                                    let new_scale_y = original.scale_y * scale_y;

                                    // Clamp to minimum absolute value while preserving sign (for flipping)
                                    const MIN_SCALE: f64 = 0.01;
                                    let new_scale_x = if new_scale_x.abs() < MIN_SCALE {
                                        MIN_SCALE * new_scale_x.signum()
                                    } else {
                                        new_scale_x
                                    };
                                    let new_scale_y = if new_scale_y.abs() < MIN_SCALE {
                                        MIN_SCALE * new_scale_y.signum()
                                    } else {
                                        new_scale_y
                                    };

                                    // To keep the opposite corner fixed, we need to adjust position
                                    // Transform the origin point with OLD transform
                                    let old_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let world_origin_before = old_transform * local_origin;

                                    // Transform the origin point with NEW transform (new scale)
                                    let new_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(new_scale_x, new_scale_y);
                                    let world_origin_after = new_transform * local_origin;

                                    // Adjust position to keep origin fixed
                                    let pos_offset_x = world_origin_before.x - world_origin_after.x;
                                    let pos_offset_y = world_origin_before.y - world_origin_after.y;

                                    // Apply scale and position adjustment
                                    vector_layer.modify_object_internal(&object_id, |obj| {
                                        obj.transform.scale_x = new_scale_x;
                                        obj.transform.scale_y = new_scale_y;
                                        obj.transform.x = original.x + pos_offset_x;
                                        obj.transform.y = original.y + pos_offset_y;
                                        obj.transform.rotation = original.rotation;
                                    });
                                }
                                lightningbeam_core::tool::TransformMode::Rotate { center } => {
                                    // Calculate rotation angle change
                                    let start_vec = start_mouse - center;
                                    let current_vec = point - center;

                                    let start_angle = start_vec.y.atan2(start_vec.x);
                                    let current_angle = current_vec.y.atan2(current_vec.x);
                                    let delta_angle = (current_angle - start_angle).to_degrees();

                                    // Calculate the visual center of the shape in world space (before rotation)
                                    let local_center = vello::kurbo::Point::new(
                                        (local_bbox.x0 + local_bbox.x1) / 2.0,
                                        (local_bbox.y0 + local_bbox.y1) / 2.0,
                                    );

                                    // Transform local center to world space with ORIGINAL transform
                                    let original_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let world_center_before = original_transform * local_center;

                                    // Now with NEW rotation
                                    let new_rotation = original.rotation + delta_angle;
                                    let new_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(new_rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let world_center_after = new_transform * local_center;

                                    // Adjust position to keep the center fixed
                                    let pos_offset_x = world_center_before.x - world_center_after.x;
                                    let pos_offset_y = world_center_before.y - world_center_after.y;

                                    vector_layer.modify_object_internal(&object_id, |obj| {
                                        obj.transform.rotation = new_rotation;
                                        obj.transform.x = original.x + pos_offset_x;
                                        obj.transform.y = original.y + pos_offset_y;
                                        obj.transform.scale_x = original.scale_x;
                                        obj.transform.scale_y = original.scale_y;
                                    });
                                }
                                lightningbeam_core::tool::TransformMode::ScaleEdge { axis, origin } => {
                                    // Similar to corner scaling, but only scale along one axis
                                    let original_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let inv_original_transform = original_transform.inverse();

                                    let local_start = inv_original_transform * start_mouse;
                                    let local_current = inv_original_transform * point;
                                    let local_origin = inv_original_transform * origin;

                                    use lightningbeam_core::tool::Axis;
                                    let (new_scale_x, new_scale_y) = match axis {
                                        Axis::Horizontal => {
                                            // Scale along X axis only
                                            let start_dx = local_start.x - local_origin.x;
                                            let current_dx = local_current.x - local_origin.x;
                                            let scale_x = if start_dx.abs() > 0.001 {
                                                current_dx / start_dx
                                            } else {
                                                1.0
                                            };
                                            let new_scale_x = original.scale_x * scale_x;
                                            const MIN_SCALE: f64 = 0.01;
                                            let new_scale_x = if new_scale_x.abs() < MIN_SCALE {
                                                MIN_SCALE * new_scale_x.signum()
                                            } else {
                                                new_scale_x
                                            };
                                            (new_scale_x, original.scale_y)
                                        }
                                        Axis::Vertical => {
                                            // Scale along Y axis only
                                            let start_dy = local_start.y - local_origin.y;
                                            let current_dy = local_current.y - local_origin.y;
                                            let scale_y = if start_dy.abs() > 0.001 {
                                                current_dy / start_dy
                                            } else {
                                                1.0
                                            };
                                            let new_scale_y = original.scale_y * scale_y;
                                            const MIN_SCALE: f64 = 0.01;
                                            let new_scale_y = if new_scale_y.abs() < MIN_SCALE {
                                                MIN_SCALE * new_scale_y.signum()
                                            } else {
                                                new_scale_y
                                            };
                                            (original.scale_x, new_scale_y)
                                        }
                                    };

                                    // Keep opposite edge fixed
                                    let old_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                    let world_origin_before = old_transform * local_origin;

                                    let new_transform = Affine::translate((original.x, original.y))
                                        * Affine::rotate(original.rotation.to_radians())
                                        * Affine::scale_non_uniform(new_scale_x, new_scale_y);
                                    let world_origin_after = new_transform * local_origin;

                                    let pos_offset_x = world_origin_before.x - world_origin_after.x;
                                    let pos_offset_y = world_origin_before.y - world_origin_after.y;

                                    vector_layer.modify_object_internal(&object_id, |obj| {
                                        obj.transform.scale_x = new_scale_x;
                                        obj.transform.scale_y = new_scale_y;
                                        obj.transform.x = original.x + pos_offset_x;
                                        obj.transform.y = original.y + pos_offset_y;
                                        obj.transform.rotation = original.rotation;
                                    });
                                }
                                lightningbeam_core::tool::TransformMode::Skew { axis, origin } => {
                                    // TODO: DCEL - skew transform for shape instances disabled during migration
                                    // (was: get_shape_in_keyframe to get bbox, compute skew angle, modify_object_internal)
                                    let _ = (axis, origin);
                                }
                            }
                        }
                    } else if let AnyLayer::Video(video_layer) = layer {
                        // Handle Video layer clip instances
                        if let Some(clip_instance) = video_layer.clip_instances.iter_mut().find(|ci| ci.id == object_id) {
                            if let Some(original) = original_transforms.get(&object_id) {
                                match mode {
                                    lightningbeam_core::tool::TransformMode::ScaleCorner { origin } => {
                                        let original_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let inv_original_transform = original_transform.inverse();

                                        let local_start = inv_original_transform * start_mouse;
                                        let local_current = inv_original_transform * point;
                                        let local_origin = inv_original_transform * origin;

                                        let start_dx = local_start.x - local_origin.x;
                                        let start_dy = local_start.y - local_origin.y;
                                        let current_dx = local_current.x - local_origin.x;
                                        let current_dy = local_current.y - local_origin.y;

                                        let scale_x = if start_dx.abs() > 0.001 { current_dx / start_dx } else { 1.0 };
                                        let scale_y = if start_dy.abs() > 0.001 { current_dy / start_dy } else { 1.0 };

                                        let new_scale_x = original.scale_x * scale_x;
                                        let new_scale_y = original.scale_y * scale_y;

                                        const MIN_SCALE: f64 = 0.01;
                                        let new_scale_x = if new_scale_x.abs() < MIN_SCALE { MIN_SCALE * new_scale_x.signum() } else { new_scale_x };
                                        let new_scale_y = if new_scale_y.abs() < MIN_SCALE { MIN_SCALE * new_scale_y.signum() } else { new_scale_y };

                                        let old_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let world_origin_before = old_transform * local_origin;

                                        let new_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(new_scale_x, new_scale_y);
                                        let world_origin_after = new_transform * local_origin;

                                        let pos_offset_x = world_origin_before.x - world_origin_after.x;
                                        let pos_offset_y = world_origin_before.y - world_origin_after.y;

                                        clip_instance.transform.scale_x = new_scale_x;
                                        clip_instance.transform.scale_y = new_scale_y;
                                        clip_instance.transform.x = original.x + pos_offset_x;
                                        clip_instance.transform.y = original.y + pos_offset_y;
                                        clip_instance.transform.rotation = original.rotation;
                                    }
                                    lightningbeam_core::tool::TransformMode::Rotate { center } => {
                                        let start_vec = start_mouse - center;
                                        let current_vec = point - center;
                                        let start_angle = start_vec.y.atan2(start_vec.x);
                                        let current_angle = current_vec.y.atan2(current_vec.x);
                                        let delta_angle = (current_angle - start_angle).to_degrees();

                                        let local_center = vello::kurbo::Point::new(
                                            (local_bbox.x0 + local_bbox.x1) / 2.0,
                                            (local_bbox.y0 + local_bbox.y1) / 2.0,
                                        );

                                        let original_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let world_center_before = original_transform * local_center;

                                        let new_rotation = original.rotation + delta_angle;
                                        let new_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(new_rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let world_center_after = new_transform * local_center;

                                        let pos_offset_x = world_center_before.x - world_center_after.x;
                                        let pos_offset_y = world_center_before.y - world_center_after.y;

                                        clip_instance.transform.rotation = new_rotation;
                                        clip_instance.transform.x = original.x + pos_offset_x;
                                        clip_instance.transform.y = original.y + pos_offset_y;
                                        clip_instance.transform.scale_x = original.scale_x;
                                        clip_instance.transform.scale_y = original.scale_y;
                                    }
                                    lightningbeam_core::tool::TransformMode::ScaleEdge { axis, origin } => {
                                        let original_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let inv_original_transform = original_transform.inverse();

                                        let local_start = inv_original_transform * start_mouse;
                                        let local_current = inv_original_transform * point;
                                        let local_origin = inv_original_transform * origin;

                                        use lightningbeam_core::tool::Axis;
                                        let (new_scale_x, new_scale_y) = match axis {
                                            Axis::Horizontal => {
                                                let start_dx = local_start.x - local_origin.x;
                                                let current_dx = local_current.x - local_origin.x;
                                                let scale_x = if start_dx.abs() > 0.001 { current_dx / start_dx } else { 1.0 };
                                                let new_scale_x = original.scale_x * scale_x;
                                                const MIN_SCALE: f64 = 0.01;
                                                let new_scale_x = if new_scale_x.abs() < MIN_SCALE { MIN_SCALE * new_scale_x.signum() } else { new_scale_x };
                                                (new_scale_x, original.scale_y)
                                            }
                                            Axis::Vertical => {
                                                let start_dy = local_start.y - local_origin.y;
                                                let current_dy = local_current.y - local_origin.y;
                                                let scale_y = if start_dy.abs() > 0.001 { current_dy / start_dy } else { 1.0 };
                                                let new_scale_y = original.scale_y * scale_y;
                                                const MIN_SCALE: f64 = 0.01;
                                                let new_scale_y = if new_scale_y.abs() < MIN_SCALE { MIN_SCALE * new_scale_y.signum() } else { new_scale_y };
                                                (original.scale_x, new_scale_y)
                                            }
                                        };

                                        let old_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let world_origin_before = old_transform * local_origin;

                                        let new_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(new_scale_x, new_scale_y);
                                        let world_origin_after = new_transform * local_origin;

                                        let pos_offset_x = world_origin_before.x - world_origin_after.x;
                                        let pos_offset_y = world_origin_before.y - world_origin_after.y;

                                        clip_instance.transform.scale_x = new_scale_x;
                                        clip_instance.transform.scale_y = new_scale_y;
                                        clip_instance.transform.x = original.x + pos_offset_x;
                                        clip_instance.transform.y = original.y + pos_offset_y;
                                        clip_instance.transform.rotation = original.rotation;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // Mouse up: finalize
        if self.rsp_drag_stopped(response) || (self.rsp_any_released(ui) && matches!(shared.tool_state, ToolState::Transforming { .. })) {
            if let ToolState::Transforming { original_transforms, .. } = shared.tool_state.clone() {
                use std::collections::HashMap;
                use lightningbeam_core::actions::TransformClipInstancesAction;

                let mut clip_instance_transforms = HashMap::new();

                if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                    for (obj_id, original) in original_transforms {
                        if let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| ci.id == obj_id) {
                            clip_instance_transforms.insert(obj_id, (original, clip_instance.transform.clone()));
                        }
                    }
                } else if let Some(AnyLayer::Video(video_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                    for (obj_id, original) in original_transforms {
                        if let Some(clip_instance) = video_layer.clip_instances.iter().find(|ci| ci.id == obj_id) {
                            clip_instance_transforms.insert(obj_id, (original, clip_instance.transform.clone()));
                        }
                    }
                }

                // Create action for clip instances
                if !clip_instance_transforms.is_empty() {
                    let action = TransformClipInstancesAction::new(*active_layer_id, *shared.playback_time, clip_instance_transforms);
                    shared.pending_actions.push(Box::new(action));
                }

                *shared.tool_state = ToolState::Idle;
            }
        }
    }

    fn handle_transform_video_clip(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        point: vello::kurbo::Point,
        layer_id: &uuid::Uuid,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;

        // Find the visible clip instance at playback time
        let playback_time = *shared.playback_time;

        let visible_clip_id = {
            let document = shared.action_executor.document();
            if let Some(AnyLayer::Video(video_layer)) = document.get_layer(layer_id) {
                video_layer.clip_instances.iter().find(|inst| {
                    let clip_duration = document.get_clip_duration(&inst.clip_id).unwrap_or(0.0);
                    let effective_duration = inst.effective_duration(clip_duration);
                    playback_time >= inst.timeline_start && playback_time < inst.timeline_start + effective_duration
                }).map(|inst| inst.id)
            } else {
                None
            }
        };

        // If we found a visible clip, ensure it's selected and handle transform
        if let Some(clip_id) = visible_clip_id {
            // Keep the visible clip selected for video layers
            // (unlike vector layers where user manually selects)
            if !shared.selection.contains_clip_instance(&clip_id) {
                shared.selection.clear();
                shared.selection.add_clip_instance(clip_id);
            }

            // Handle transform with the selected clip
            self.handle_transform_single_object(ui, response, point, layer_id, shared);
        } else {
            // No visible clip at playback time, clear selection
            shared.selection.clear();
        }
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, rect: egui::Rect, shared: &mut SharedPaneState) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Check for mouse release to complete drag operations (even if mouse is offscreen)
        use lightningbeam_core::tool::ToolState;
        use vello::kurbo::Point;

        // When replaying, skip ALL real mouse/scroll input — only synthetic events drive state
        #[cfg(debug_assertions)]
        let is_replaying = matches!(shared.test_mode.mode, crate::test_mode::TestModeOp::Playing(_));
        #[cfg(not(debug_assertions))]
        let is_replaying = false;

        // Store current input as a pending event for panic capture.
        // If processing panics, the panic hook appends this to the saved test case.
        #[cfg(debug_assertions)]
        if !is_replaying {
            if let Some(mouse_pos) = ui.input(|i| i.pointer.latest_pos()) {
                use lightningbeam_core::test_mode::{SerPoint, TestEventKind};
                let mouse_canvas_pos = mouse_pos - rect.min;
                let world_pos_doc = (mouse_canvas_pos - self.pan_offset) / self.zoom;
                let wp = self.doc_to_clip_local(world_pos_doc, shared);
                let pos = SerPoint { x: wp.x as f64, y: wp.y as f64 };
                let kind = if ui.input(|i| i.pointer.any_released()) {
                    TestEventKind::MouseUp { pos }
                } else if ui.input(|i| i.pointer.primary_pressed()) && response.hovered() {
                    TestEventKind::MouseDown { pos }
                } else if response.dragged() || response.drag_started() {
                    TestEventKind::MouseDrag { pos }
                } else {
                    TestEventKind::MouseMove { pos }
                };
                shared.test_mode.set_pending_event(kind);
            }
        }

        if !is_replaying && ui.input(|i| i.pointer.any_released()) {
            match shared.tool_state.clone() {
                ToolState::DraggingSelection { start_mouse, original_positions, .. } => {
                    // Get last known mouse position (will be at edge if offscreen)
                    if let Some(mouse_pos) = ui.input(|i| i.pointer.latest_pos()) {
                        let mouse_canvas_pos = mouse_pos - rect.min;
                        let world_pos_doc = (mouse_canvas_pos - self.pan_offset) / self.zoom;
                        let world_pos = self.doc_to_clip_local(world_pos_doc, shared);
                        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

                        let delta = point - start_mouse;

                        if delta.x.abs() > 0.01 || delta.y.abs() > 0.01 {
                            if let Some(active_layer_id) = shared.active_layer_id {
                                use std::collections::HashMap;

                                let mut clip_instance_transforms = HashMap::new();

                                // Process clip instances from drag
                                for (object_id, original_pos) in original_positions {
                                    let new_pos = Point::new(
                                        original_pos.x + delta.x,
                                        original_pos.y + delta.y,
                                    );

                                    if shared.selection.contains_clip_instance(&object_id) {
                                        // For clip instances, get the full transform
                                        if let Some(layer) = shared.action_executor.document().get_layer(active_layer_id) {
                                            if let lightningbeam_core::layer::AnyLayer::Vector(vector_layer) = layer {
                                                if let Some(clip_inst) = vector_layer.clip_instances.iter().find(|ci| ci.id == object_id) {
                                                    let mut old_transform = clip_inst.transform.clone();
                                                    old_transform.x = original_pos.x;
                                                    old_transform.y = original_pos.y;

                                                    let mut new_transform = clip_inst.transform.clone();
                                                    new_transform.x = new_pos.x;
                                                    new_transform.y = new_pos.y;

                                                    clip_instance_transforms.insert(object_id, (old_transform, new_transform));
                                                }
                                            }
                                        }
                                    }
                                }

                                // Create action for clip instances
                                if !clip_instance_transforms.is_empty() {
                                    use lightningbeam_core::actions::TransformClipInstancesAction;
                                    let action = TransformClipInstancesAction::new(*active_layer_id, *shared.playback_time, clip_instance_transforms);
                                    shared.pending_actions.push(Box::new(action));
                                }
                            }
                        }
                    }
                    *shared.tool_state = ToolState::Idle;
                }
                ToolState::MarqueeSelecting { start, current } => {
                    // Complete marquee selection (even if mouse is offscreen)
                    // Get active layer
                    if let Some(active_layer_id) = shared.active_layer_id {
                        use lightningbeam_core::layer::AnyLayer;
                        use lightningbeam_core::hit_test;
                        use vello::kurbo::{Rect as KurboRect, Affine};

                        if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(&active_layer_id) {
                            // Create selection rectangle
                            let min_x = start.x.min(current.x);
                            let min_y = start.y.min(current.y);
                            let max_x = start.x.max(current.x);
                            let max_y = start.y.max(current.y);

                            let selection_rect = KurboRect::new(min_x, min_y, max_x, max_y);

                            // Hit test clip instances in rectangle
                            let document = shared.action_executor.document();
                            let clip_hits = hit_test::hit_test_clip_instances_in_rect(
                                &vector_layer.clip_instances,
                                document,
                                selection_rect,
                                Affine::IDENTITY,
                                *shared.playback_time,
                            );

                            // Hit test DCEL elements in rectangle
                            let dcel_hits = hit_test::hit_test_dcel_in_rect(
                                vector_layer,
                                *shared.playback_time,
                                selection_rect,
                                Affine::IDENTITY,
                            );

                            // Add clip instances to selection
                            for clip_id in clip_hits {
                                shared.selection.add_clip_instance(clip_id);
                            }

                            // Add DCEL elements to selection
                            if let Some(dcel) = vector_layer.dcel_at_time(*shared.playback_time) {
                                for edge_id in dcel_hits.edges {
                                    shared.selection.select_edge(edge_id, dcel);
                                }
                                for face_id in dcel_hits.faces {
                                    shared.selection.select_face(face_id, dcel);
                                }
                            }
                        }
                    }

                    // Update focus based on what was selected
                    if shared.selection.has_dcel_selection() {
                        if let Some(layer_id) = *shared.active_layer_id {
                            *shared.focus = lightningbeam_core::selection::FocusSelection::Geometry { layer_id, time: *shared.playback_time };
                        }
                    } else if !shared.selection.clip_instances().is_empty() {
                        *shared.focus = lightningbeam_core::selection::FocusSelection::ClipInstances(shared.selection.clip_instances().to_vec());
                    }

                    *shared.tool_state = ToolState::Idle;
                }
                _ => {}
            }
        }

        // Check for synthetic input from test mode replay (debug builds only)
        #[cfg(debug_assertions)]
        let synthetic_input = shared.synthetic_input.take();

        // Only process input if mouse is over the stage pane (or synthetic input is active)
        #[cfg(debug_assertions)]
        let has_synthetic = synthetic_input.is_some();
        #[cfg(not(debug_assertions))]
        let has_synthetic = false;

        if !response.hovered() && !has_synthetic {
            self.is_panning = false;
            self.last_pan_pos = None;
            return;
        }

        // During replay with no synthetic event this frame, skip all input processing
        #[cfg(debug_assertions)]
        if is_replaying && !has_synthetic {
            return;
        }

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);

        // Source input from synthetic (replay) or real UI
        #[cfg(debug_assertions)]
        let (world_pos, alt_held, ctrl_held, shift_held, drag_started, dragged, drag_stopped) = if let Some(syn) = &synthetic_input {
            let wp = egui::Vec2::new(syn.world_pos.x as f32, syn.world_pos.y as f32);
            (wp, syn.alt, syn.ctrl, syn.shift, syn.drag_started, syn.dragged, syn.drag_stopped)
        } else {
            let alt_held = ui.input(|i| i.modifiers.alt);
            let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
            let shift_held = ui.input(|i| i.modifiers.shift);
            let mouse_pos = response.hover_pos().unwrap_or(rect.center());
            let mouse_canvas_pos = mouse_pos - rect.min;
            let world_pos_doc = (mouse_canvas_pos - self.pan_offset) / self.zoom;
            let wp = self.doc_to_clip_local(world_pos_doc, shared);
            (wp, alt_held, ctrl_held, shift_held, response.drag_started(), response.dragged(), response.drag_stopped())
        };

        #[cfg(not(debug_assertions))]
        let (world_pos, alt_held, ctrl_held, shift_held, _drag_started, _dragged, _drag_stopped) = {
            let alt_held = ui.input(|i| i.modifiers.alt);
            let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
            let shift_held = ui.input(|i| i.modifiers.shift);
            let mouse_pos = response.hover_pos().unwrap_or(rect.center());
            let mouse_canvas_pos = mouse_pos - rect.min;
            let world_pos_doc = (mouse_canvas_pos - self.pan_offset) / self.zoom;
            let wp = self.doc_to_clip_local(world_pos_doc, shared);
            (wp, alt_held, ctrl_held, shift_held, response.drag_started(), response.dragged(), response.drag_stopped())
        };

        // Record mouse events for test mode (debug builds only) — skip during replay
        //
        // IMPORTANT: We use `primary_pressed` (fires immediately on button down) for MouseDown
        // instead of `drag_started` (fires after egui's drag threshold, ~6-10px of movement).
        // The select tool hit-tests on `primary_pressed`, so we must record the position at
        // that moment. The `drag_started` frame is recorded as MouseDrag since the press
        // was already captured.
        #[cfg(debug_assertions)]
        if !is_replaying {
            use lightningbeam_core::test_mode::{SerPoint, TestEventKind};
            let pos = SerPoint { x: world_pos.x as f64, y: world_pos.y as f64 };
            let primary_just_pressed = response.hovered() && ui.input(|i| i.pointer.primary_pressed());
            if primary_just_pressed {
                shared.test_mode.record_event(TestEventKind::MouseDown { pos });
            } else if drag_stopped {
                // Emit a final MouseDrag at the release position to close the gap
                // between the last drag frame and the release (the mouse moves between frames)
                shared.test_mode.record_event(TestEventKind::MouseDrag { pos });
                shared.test_mode.record_event(TestEventKind::MouseUp { pos });
            } else if drag_started || dragged {
                // drag_started after primary_pressed is just the first drag motion
                shared.test_mode.record_event(TestEventKind::MouseDrag { pos });
            } else if response.hovered() {
                shared.test_mode.record_event(TestEventKind::MouseMove { pos });
            }
        }

        // Get mouse position for zoom-to-cursor (needed for pan/zoom handling below)
        let mouse_pos = response.hover_pos().unwrap_or(rect.center());
        let mouse_canvas_pos = mouse_pos - rect.min;

        // Set replay override so wrapper methods return synthetic drag state
        #[cfg(debug_assertions)]
        if synthetic_input.is_some() {
            self.replay_override = Some(ReplayDragState {
                drag_started,
                dragged,
                drag_stopped,
            });
        }

        // Alt+click: set source point for clone/healing tools.
        {
            use lightningbeam_core::tool::Tool;
            let tool_uses_alt = crate::tools::raster_tool_def(shared.selected_tool)
                .map_or(false, |d| d.uses_alt_click());
            if tool_uses_alt
                && alt_held
                && self.rsp_primary_pressed(ui)
                && response.hovered()
            {
                eprintln!("[clone/healing] set clone source to ({:.1}, {:.1})", world_pos.x, world_pos.y);
                shared.raster_settings.clone_source = Some(world_pos);
            }
        }

        // Handle tool input (only if not using Alt modifier for panning)
        if !alt_held {
            use lightningbeam_core::tool::Tool;

            match *shared.selected_tool {
                Tool::Select => {
                    let is_raster = shared.active_layer_id.and_then(|id| {
                        shared.action_executor.document().get_layer(&id)
                    }).map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));
                    if is_raster {
                        self.handle_raster_select_tool(ui, &response, world_pos, shared);
                    } else {
                        self.handle_select_tool(ui, &response, world_pos, shift_held, shared);
                    }
                }
                Tool::BezierEdit => {
                    self.handle_bezier_edit_tool(ui, &response, world_pos, shift_held, shared);
                }
                Tool::Rectangle => {
                    self.handle_rectangle_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Ellipse => {
                    self.handle_ellipse_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Draw => {
                    // Dispatch to raster or vector draw handler based on active layer type
                    let is_raster = shared.active_layer_id.and_then(|id| {
                        shared.action_executor.document().get_layer(&id)
                    }).map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));
                    if is_raster {
                        self.handle_unified_raster_stroke_tool(ui, &response, world_pos, &crate::tools::paint::PAINT, shared);
                    } else {
                        self.handle_draw_tool(ui, &response, world_pos, shared);
                    }
                }
                tool if crate::tools::raster_tool_def(&tool).is_some() => {
                    let def = crate::tools::raster_tool_def(&tool).unwrap();
                    self.handle_raster_stroke_tool(ui, &response, world_pos, def, shared);
                }
                Tool::SelectLasso => {
                    self.handle_raster_lasso_tool(ui, &response, world_pos, shared);
                }
                Tool::MagicWand => {
                    self.handle_magic_wand_tool(&response, world_pos, shared);
                }
                Tool::QuickSelect => {
                    self.handle_quick_select_tool(ui, &response, world_pos, shared);
                }
                Tool::Transform => {
                    self.handle_transform_tool(ui, &response, world_pos, shared);
                }
                Tool::PaintBucket => {
                    self.handle_paint_bucket_tool(&response, world_pos, shared);
                }
                Tool::Line => {
                    self.handle_line_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Polygon => {
                    self.handle_polygon_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Eyedropper => {
                    self.handle_eyedropper_tool(ui, &response, mouse_pos, shared);
                }
                Tool::RegionSelect => {
                    self.handle_region_select_tool(ui, &response, world_pos, shared);
                }
                Tool::Warp => {
                    self.handle_raster_warp_tool(ui, &response, world_pos, shared);
                }
                Tool::Liquify => {
                    self.handle_raster_liquify_tool(ui, &response, world_pos, shared);
                }
                Tool::Gradient => {
                    self.handle_raster_gradient_tool(ui, &response, world_pos, shared);
                }
                _ => {
                    // Other tools not implemented yet
                }
            }
        }

        // Clear replay override after tool dispatch
        #[cfg(debug_assertions)]
        { self.replay_override = None; }

        // Delete/Backspace: remove selected DCEL elements
        if ui.input(|i| shared.keymap.action_pressed_with_backspace(crate::keymap::AppAction::StageDelete, i)) {
            if shared.selection.has_dcel_selection() {
                if let Some(active_layer_id) = *shared.active_layer_id {
                    let time = *shared.playback_time;

                    // Collect selected edge IDs before mutating
                    let selected_edges: Vec<lightningbeam_core::dcel::EdgeId> =
                        shared.selection.selected_edges().iter().copied().collect();

                    if !selected_edges.is_empty() {
                        // Snapshot before
                        let dcel_before = {
                            let document = shared.action_executor.document();
                            match document.get_layer(&active_layer_id) {
                                Some(lightningbeam_core::layer::AnyLayer::Vector(vl)) => {
                                    vl.dcel_at_time(time).cloned()
                                }
                                _ => None,
                            }
                        };

                        if let Some(dcel_before) = dcel_before {
                            // Remove selected edges
                            {
                                let document = shared.action_executor.document_mut();
                                if let Some(lightningbeam_core::layer::AnyLayer::Vector(vl)) = document.get_layer_mut(&active_layer_id) {
                                    if let Some(dcel) = vl.dcel_at_time_mut(time) {
                                        for eid in &selected_edges {
                                            dcel.remove_edge(*eid);
                                        }
                                    }
                                }
                            }

                            // Snapshot after
                            let dcel_after = {
                                let document = shared.action_executor.document();
                                match document.get_layer(&active_layer_id) {
                                    Some(lightningbeam_core::layer::AnyLayer::Vector(vl)) => {
                                        vl.dcel_at_time(time).cloned()
                                    }
                                    _ => None,
                                }
                            };

                            if let Some(dcel_after) = dcel_after {
                                use lightningbeam_core::actions::ModifyDcelAction;
                                let action = ModifyDcelAction::new(
                                    active_layer_id,
                                    time,
                                    dcel_before,
                                    dcel_after,
                                    "Delete",
                                );
                                shared.pending_actions.push(Box::new(action));
                            }

                            shared.selection.clear_dcel_selection();
                        }
                    }
                }
            }
        }

        // Skip real scroll/zoom/pan input during replay
        if !is_replaying {
            // Distinguish between mouse wheel (discrete) and trackpad (smooth)
            let mut handled = false;
            ui.input(|i| {
                for event in &i.raw.events {
                    if let egui::Event::MouseWheel { unit, delta, modifiers, .. } = event {
                        match unit {
                            egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                                // Real mouse wheel (discrete clicks) -> always zoom
                                let zoom_delta = if ctrl_held || modifiers.ctrl {
                                    delta.y * 0.01 // Ctrl+wheel: faster zoom
                                } else {
                                    delta.y * 0.005 // Normal zoom
                                };
                                self.apply_zoom_at_point(zoom_delta, mouse_canvas_pos);
                                handled = true;
                            }
                            egui::MouseWheelUnit::Point => {
                                // Trackpad (smooth scrolling) -> only zoom if Ctrl held
                                if ctrl_held || modifiers.ctrl {
                                    let zoom_delta = delta.y * 0.005;
                                    self.apply_zoom_at_point(zoom_delta, mouse_canvas_pos);
                                    handled = true;
                                }
                                // Otherwise let scroll_delta handle panning
                            }
                        }
                    }
                }
            });

            // Handle scroll_delta for trackpad panning (when Ctrl not held)
            if !handled && (scroll_delta.x.abs() > 0.0 || scroll_delta.y.abs() > 0.0) {
                self.pan_offset.x += scroll_delta.x;
                self.pan_offset.y += scroll_delta.y;
            }

            // Handle panning with Alt+Drag
            if alt_held && response.dragged() {
                // Alt+Click+Drag panning
                if let Some(last_pos) = self.last_pan_pos {
                    if let Some(current_pos) = response.interact_pointer_pos() {
                        let delta = current_pos - last_pos;
                        self.pan_offset += delta;
                    }
                }
                self.last_pan_pos = response.interact_pointer_pos();
                self.is_panning = true;
            } else {
                if !response.dragged() {
                    self.is_panning = false;
                    self.last_pan_pos = None;
                }
            }
        }
    }

    /// Render vector editing overlays (vertices, control points, handles)
    fn render_vector_editing_overlays(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &SharedPaneState,
    ) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::tool::Tool;
        use lightningbeam_core::hit_test::{hit_test_vector_editing, EditingHitTolerance, VectorEditHit};
        use vello::kurbo::{Affine, Point};

        // Only show overlays for Select and BezierEdit tools
        let is_bezier_edit_mode = matches!(*shared.selected_tool, Tool::BezierEdit);
        let show_overlays = matches!(*shared.selected_tool, Tool::Select | Tool::BezierEdit);

        if !show_overlays {
            return;
        }

        // Get active layer
        let active_layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(AnyLayer::Vector(layer)) => layer,
            _ => return,
        };

        // Get mouse position in world coordinates (clip-local when inside a clip)
        let mouse_screen_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
        let mouse_canvas_pos = mouse_screen_pos - rect.min;
        let mouse_doc_pos = egui::vec2(
            (mouse_canvas_pos.x - self.pan_offset.x) / self.zoom,
            (mouse_canvas_pos.y - self.pan_offset.y) / self.zoom,
        );
        let mouse_local = self.doc_to_clip_local(mouse_doc_pos, shared);
        let mouse_world_pos = Point::new(mouse_local.x as f64, mouse_local.y as f64);

        // Helper to convert world coordinates (clip-local) to screen coordinates
        let world_to_screen = |world_pos: Point| -> egui::Pos2 {
            // When inside a clip, first transform from clip-local to document space
            let doc_pos = self.clip_local_to_doc(world_pos, shared);
            let screen_x = (doc_pos.x as f32 * self.zoom) + self.pan_offset.x + rect.min.x;
            let screen_y = (doc_pos.y as f32 * self.zoom) + self.pan_offset.y + rect.min.y;
            egui::pos2(screen_x, screen_y)
        };

        let painter = ui.painter_at(rect);

        // Perform hit testing to find what's under the mouse
        let tolerance = EditingHitTolerance::scaled_by_zoom(self.zoom as f64);
        let hit = hit_test_vector_editing(
            layer,
            *shared.playback_time,
            mouse_world_pos,
            &tolerance,
            Affine::IDENTITY,
            is_bezier_edit_mode,
        );

        // Get the DCEL for drawing overlays
        let dcel = match layer.dcel_at_time(*shared.playback_time) {
            Some(d) => d,
            None => return,
        };

        // Visual constants
        let vertex_radius = 4.0_f32;
        let vertex_hover_radius = 6.0_f32;
        let cp_radius = 3.0_f32;
        let cp_hover_radius = 5.0_f32;
        let vertex_color = egui::Color32::WHITE;
        let vertex_stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(40, 100, 220));
        let vertex_hover_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(60, 140, 255));
        let cp_color = egui::Color32::from_rgba_premultiplied(180, 180, 255, 200);
        let cp_hover_color = egui::Color32::from_rgb(100, 160, 255);
        let cp_line_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(120, 120, 200, 150));

        // Determine what's hovered (suppress during active editing to avoid flicker)
        let is_editing = matches!(
            *shared.tool_state,
            lightningbeam_core::tool::ToolState::EditingCurve { .. }
            | lightningbeam_core::tool::ToolState::EditingVertex { .. }
            | lightningbeam_core::tool::ToolState::EditingControlPoint { .. }
            | lightningbeam_core::tool::ToolState::PendingCurveInteraction { .. }
        );
        let hover_vertex = if is_editing { None } else {
            match hit {
                Some(VectorEditHit::Vertex { vertex_id }) => Some(vertex_id),
                _ => None,
            }
        };
        let hover_cp = if is_editing { None } else {
            match hit {
                Some(VectorEditHit::ControlPoint { edge_id, point_index }) => Some((edge_id, point_index)),
                _ => None,
            }
        };

        if is_bezier_edit_mode {
            // BezierEdit mode: Draw all vertices, control points, and tangent lines

            // Draw control point tangent lines and control points for all edges
            for (i, edge) in dcel.edges.iter().enumerate() {
                if edge.deleted { continue; }
                let edge_id = lightningbeam_core::dcel::EdgeId(i as u32);
                let curve = &edge.curve;

                // Tangent lines from endpoints to control points
                let p0_screen = world_to_screen(curve.p0);
                let p1_screen = world_to_screen(curve.p1);
                let p2_screen = world_to_screen(curve.p2);
                let p3_screen = world_to_screen(curve.p3);

                painter.line_segment([p0_screen, p1_screen], cp_line_stroke);
                painter.line_segment([p3_screen, p2_screen], cp_line_stroke);

                // Draw control point p1
                let is_hover_p1 = hover_cp == Some((edge_id, 1));
                if is_hover_p1 {
                    painter.circle_filled(p1_screen, cp_hover_radius, cp_hover_color);
                } else {
                    painter.circle_filled(p1_screen, cp_radius, cp_color);
                }

                // Draw control point p2
                let is_hover_p2 = hover_cp == Some((edge_id, 2));
                if is_hover_p2 {
                    painter.circle_filled(p2_screen, cp_hover_radius, cp_hover_color);
                } else {
                    painter.circle_filled(p2_screen, cp_radius, cp_color);
                }
            }

            // Draw vertices on top of everything
            for (i, vertex) in dcel.vertices.iter().enumerate() {
                if vertex.deleted { continue; }
                let vid = lightningbeam_core::dcel::VertexId(i as u32);
                let screen_pos = world_to_screen(vertex.position);
                let is_hovered = hover_vertex == Some(vid);
                if is_hovered {
                    painter.circle(screen_pos, vertex_hover_radius, vertex_color, vertex_hover_stroke);
                } else {
                    painter.circle(screen_pos, vertex_radius, vertex_color, vertex_stroke);
                }
            }
        } else {
            // Select mode: Only show hover highlight for the element under the mouse
            if let Some(vid) = hover_vertex {
                let pos = dcel.vertex(vid).position;
                let screen_pos = world_to_screen(pos);
                painter.circle(screen_pos, vertex_hover_radius, vertex_color, vertex_hover_stroke);
            }

            // Note: curve hover highlight is now rendered via Vello stipple in the scene

            if let Some((eid, pidx)) = hover_cp {
                let curve = &dcel.edge(eid).curve;
                let cp_pos = if pidx == 1 { curve.p1 } else { curve.p2 };
                let screen_pos = world_to_screen(cp_pos);
                painter.circle_filled(screen_pos, cp_hover_radius, cp_hover_color);
            }
        }
    }

    /// Render raster selection overlays:
    ///   - Animated "marching ants" around the active raster selection (marquee or lasso)
    ///   - (Float pixels are rendered through the Vello HDR pipeline in prepare(), not here)
    fn render_raster_selection_overlays(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &mut SharedPaneState,
    ) {
        use lightningbeam_core::selection::RasterSelection;

        // Don't show marching ants during raster transform — the handles show the bbox outline.
        if self.raster_transform_state.is_some() { return; }

        let has_sel = shared.selection.raster_selection.is_some();
        if !has_sel { return; }

        let time = ui.input(|i| i.time) as f32;
        // 8px/s scroll rate → repeating every 1 s
        let phase = (time * 8.0).rem_euclid(8.0);
        let painter = ui.painter_at(rect);
        let pan = self.pan_offset;
        let zoom = self.zoom;

        // ── Marching ants ─────────────────────────────────────────────────────
        if let Some(sel) = &shared.selection.raster_selection {
            match sel {
                RasterSelection::Rect(x0, y0, x1, y1) => {
                    Self::draw_marching_ants_rect(
                        &painter, rect.min,
                        *x0, *y0, *x1, *y1,
                        zoom, pan, phase,
                    );
                }
                RasterSelection::Lasso(pts) => {
                    Self::draw_marching_ants_lasso(&painter, rect.min, pts, zoom, pan, phase);
                }
                RasterSelection::Mask { data, width, height, origin_x, origin_y } => {
                    Self::draw_marching_ants_mask(
                        &painter, rect.min,
                        data, *width, *height, *origin_x, *origin_y,
                        zoom, pan, phase,
                    );
                }
            }
        }

        // Keep animating while a selection is visible
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(80));
    }

    /// Render snap indicator when snap is active (works for all vector-editing tools).
    /// Also computes hover snap when idle (no active drag snap) so the user can
    /// preview snap targets before clicking.
    fn render_snap_indicator(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        shared: &SharedPaneState,
    ) {
        use lightningbeam_core::snap::SnapTarget;
        use lightningbeam_core::tool::Tool;

        if !*shared.snap_enabled {
            return;
        }

        let is_vector_tool = matches!(
            *shared.selected_tool,
            Tool::Select | Tool::BezierEdit | Tool::Draw | Tool::Rectangle
            | Tool::Ellipse | Tool::Line | Tool::Polygon
        );

        // Recompute hover snap every frame when idle (not actively editing/drawing)
        let is_idle = matches!(*shared.tool_state, lightningbeam_core::tool::ToolState::Idle);
        if is_vector_tool && is_idle {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                if rect.contains(pos) {
                    let canvas_pos = pos - rect.min;
                    let doc_pos = egui::vec2(
                        (canvas_pos.x - self.pan_offset.x) / self.zoom,
                        (canvas_pos.y - self.pan_offset.y) / self.zoom,
                    );
                    let local = self.doc_to_clip_local(doc_pos, shared);
                    let point = vello::kurbo::Point::new(local.x as f64, local.y as f64);
                    self.snap_point(point, shared);
                } else {
                    self.current_snap = None;
                }
            } else {
                self.current_snap = None;
            }
        }

        let snap_result = match &self.current_snap {
            Some(r) => r,
            None => return,
        };

        let world_to_screen = |world_pos: vello::kurbo::Point| -> egui::Pos2 {
            let doc_pos = self.clip_local_to_doc(world_pos, shared);
            let screen_x = (doc_pos.x as f32 * self.zoom) + self.pan_offset.x + rect.min.x;
            let screen_y = (doc_pos.y as f32 * self.zoom) + self.pan_offset.y + rect.min.y;
            egui::pos2(screen_x, screen_y)
        };

        let painter = ui.painter_at(rect);
        let screen_pos = world_to_screen(snap_result.position);

        // Reuse existing vertex visual constants
        let vertex_hover_radius = 6.0_f32;
        let vertex_color = egui::Color32::WHITE;
        let vertex_hover_stroke = egui::Stroke::new(2.0, egui::Color32::from_rgb(60, 140, 255));

        match snap_result.target {
            SnapTarget::Vertex { .. } => {
                // Same circle as the existing vertex hover indicator
                painter.circle(screen_pos, vertex_hover_radius, vertex_color, vertex_hover_stroke);
            }
            SnapTarget::Midpoint { .. } => {
                // Square indicator, same style as vertex but square
                let s = vertex_hover_radius;
                painter.rect(
                    egui::Rect::from_center_size(screen_pos, egui::vec2(s * 2.0, s * 2.0)),
                    0.0,
                    vertex_color,
                    vertex_hover_stroke,
                    egui::StrokeKind::Middle,
                );
            }
            SnapTarget::Curve { edge_id, .. } => {
                // Stipple highlight on the snapped edge (matching existing curve hover)
                use lightningbeam_core::layer::AnyLayer;
                if let Some(layer_id) = *shared.active_layer_id {
                    if let Some(AnyLayer::Vector(vl)) = shared.action_executor.document().get_layer(&layer_id) {
                        if let Some(dcel) = vl.dcel_at_time(*shared.playback_time) {
                            let edge = dcel.edge(edge_id);
                            if !edge.deleted {
                                // Draw a small circle at the snap point on the curve
                                painter.circle(screen_pos, 4.0, egui::Color32::TRANSPARENT, vertex_hover_stroke);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draw the brush-size outline cursor for raster paint tools.
    ///
    /// Renders an alternating black/white dashed ellipse (marching-ants style) centred on
    /// `pos` (screen space). The ellipse shape reflects the brush's `elliptical_dab_ratio`
    /// and angle; for brushes with position jitter (`offset_by_random`) the radius is
    /// expanded so the outline marks the full extent where paint can land.
    fn draw_brush_cursor(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        pos: egui::Pos2,
        shared: &SharedPaneState,
    ) {
        use lightningbeam_core::tool::Tool;

        // Compute semi-axes (world pixels) and dab rotation angle.
        let (a_world, b_world, dab_angle_rad) = if matches!(*shared.selected_tool, Tool::QuickSelect) {
            let r = shared.raster_settings.quick_select_radius;
            (r, r, 0.0_f32)
        } else if let Some(def) = crate::tools::raster_tool_def(shared.selected_tool) {
            let r = def.cursor_radius(shared.raster_settings);
            // For the standard paint brush, also account for elliptical shape.
            if matches!(*shared.selected_tool,
                Tool::Draw | Tool::Pencil | Tool::Pen | Tool::Airbrush)
            {
                let bs = &shared.raster_settings.active_brush_settings;
                let ratio = bs.elliptical_dab_ratio.max(1.0);
                let expand = 1.0 + bs.offset_by_random;
                (r * expand, r * expand / ratio, bs.elliptical_dab_angle.to_radians())
            } else {
                (r, r, 0.0_f32)
            }
        } else {
            let bs = &shared.raster_settings.active_brush_settings;
            let r = shared.raster_settings.brush_radius;
            let ratio = bs.elliptical_dab_ratio.max(1.0);
            let expand = 1.0 + bs.offset_by_random;
            (r * expand, r * expand / ratio, bs.elliptical_dab_angle.to_radians())
        };

        let a = a_world * self.zoom; // major semi-axis in screen pixels
        let b = b_world * self.zoom; // minor semi-axis in screen pixels
        if a < 1.0 { return; }

        let painter = ui.painter_at(rect);
        let cos_a = dab_angle_rad.cos();
        let sin_a = dab_angle_rad.sin();

        // Approximate ellipse perimeter (Ramanujan) to decide how many dashes to draw.
        let h = ((a - b) / (a + b)).powi(2);
        let perimeter = std::f32::consts::PI * (a + b)
            * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
        let dash_px = 4.0_f32;
        let n = ((perimeter / dash_px).ceil() as usize).max(8);

        let pt = |i: usize| -> egui::Pos2 {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            let ex = a * t.cos();
            let ey = b * t.sin();
            pos + egui::vec2(ex * cos_a - ey * sin_a, ex * sin_a + ey * cos_a)
        };

        // Alternating black/white 1-px segments.
        for i in 0..n {
            let color = if i % 2 == 0 { egui::Color32::BLACK } else { egui::Color32::WHITE };
            painter.line_segment([pt(i), pt(i + 1)], egui::Stroke::new(1.0, color));
        }

        // Small crosshair at centre.
        let arm = 3.0_f32.min(a * 0.3).max(1.0);
        for (color, width) in [(egui::Color32::BLACK, 2.0_f32), (egui::Color32::WHITE, 1.0_f32)] {
            let s = egui::Stroke::new(width, color);
            painter.line_segment([pos - egui::vec2(arm, 0.0), pos + egui::vec2(arm, 0.0)], s);
            painter.line_segment([pos - egui::vec2(0.0, arm), pos + egui::vec2(0.0, arm)], s);
        }
    }
}



impl PaneRenderer for StagePane {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        ui.horizontal(|ui| {
            // Zoom to fit button
            if ui.button("⊡ Fit").on_hover_text("Zoom to fit canvas in view").clicked() {
                self.zoom_to_fit(shared);
            }

            ui.separator();

            // Zoom level display
            let text_style = shared.theme.style(".text-primary", ui.ctx());
            let text_color = text_style.text_color.unwrap_or(egui::Color32::from_gray(200));
            ui.colored_label(text_color, format!("Zoom: {:.0}%", self.zoom * 100.0));
        });
        true
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Store viewport rect for zoom-to-fit calculation
        self.last_viewport_rect = Some(rect);

        // Center the document in the viewport on first render
        if self.needs_initial_center {
            self.needs_initial_center = false;
            let document = shared.action_executor.document();
            let doc_width = document.width as f32;
            let doc_height = document.height as f32;
            let viewport_size = rect.size();
            let canvas_center = egui::vec2(doc_width / 2.0, doc_height / 2.0) * self.zoom;
            let viewport_center = viewport_size / 2.0;
            self.pan_offset = viewport_center - canvas_center;
        }

        // Check for completed raster stroke readbacks and create undo actions
        if let Ok(mut results) = RASTER_READBACK_RESULTS
            .get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
            .lock() {
            if let Some(readback) = results.remove(&self.instance_id) {
                if self.painting_float {
                    // Float stroke: update float.pixels, don't create a layer RasterStrokeAction.
                    if let Some((_, _, w, h, buffer_before)) = self.pending_undo_before.take() {
                        if let Some(ref mut float) = shared.selection.raster_floating {
                            // Apply float-local selection mask: restore pixels outside C to
                            // pre-stroke values so the stroke only affects the selected area.
                            let mut pixels = readback.pixels;
                            if let Some(ref sel) = self.stroke_clip_selection {
                                for fy in 0..h {
                                    for fx in 0..w {
                                        if !sel.contains_pixel(float.x + fx as i32, float.y + fy as i32) {
                                            let i = ((fy * w + fx) * 4) as usize;
                                            pixels[i..i + 4].copy_from_slice(&buffer_before[i..i + 4]);
                                        }
                                    }
                                }
                            }
                            float.pixels = std::sync::Arc::new(pixels);
                            // Invalidate the float's GPU canvas so the lazy-init
                            // in prepare() re-uploads the fresh pixels next frame.
                            self.pending_canvas_removals.push(float.canvas_id);
                        }
                    }
                    self.stroke_clip_selection = None;
                    self.painting_float = false;
                    // Keep float GPU canvas alive for the next stroke on the float.
                    // Don't schedule canvas_removal — just clear painting_canvas.
                    self.painting_canvas = None;
                } else {
                    // Layer stroke: existing behavior — create RasterStrokeAction on raw_pixels.
                    if let Some((layer_id, time, w, h, buffer_before)) = self.pending_undo_before.take() {
                        use lightningbeam_core::actions::RasterStrokeAction;
                        // If a selection was active at stroke-start, restore any pixels
                        // outside the selection outline to their pre-stroke values.
                        let canvas_after = match self.stroke_clip_selection.take() {
                            None => readback.pixels,
                            Some(sel) => {
                                let mut masked = readback.pixels;
                                for y in 0..h {
                                    for x in 0..w {
                                        if !sel.contains_pixel(x as i32, y as i32) {
                                            let i = ((y * w + x) * 4) as usize;
                                            masked[i..i + 4].copy_from_slice(&buffer_before[i..i + 4]);
                                        }
                                    }
                                }
                                masked
                            }
                        };
                        let action = RasterStrokeAction::new(
                            layer_id,
                            time,
                            buffer_before,
                            canvas_after,
                            w,
                            h,
                        );
                        // execute() sets raw_pixels = buffer_after so future Vello renders
                        // and file saves see the completed stroke.
                        let _ = shared.action_executor.execute(Box::new(action));
                    }
                    // raw_pixels is now up to date; switch compositing back to the Vello
                    // scene.  Schedule the GPU canvas for removal at the start of the next
                    // prepare() — keeping it alive for this frame's composite avoids a
                    // one-frame flash of the stale Vello scene.
                    if let Some((_, kf_id)) = self.painting_canvas.take() {
                        self.pending_canvas_removal = Some(kf_id);
                    }
                }
                // Unified tool cleanup: clear active_raster_tool and queue A/B/C for removal.
                // Runs after both the float and layer branches.
                if self.active_tool_awaiting_readback {
                    self.active_tool_awaiting_readback = false;
                    if let Some((_, ws)) = self.active_raster_tool.take() {
                        self.pending_canvas_removals.extend(ws.canvas_ids());
                    }
                }
            }
        }

        // Check for completed eyedropper samples from GPU readback and apply them
        if let Ok(mut results) = EYEDROPPER_RESULTS
            .get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
            .lock() {
            if let Some((color, color_mode)) = results.remove(&self.instance_id) {
                // Apply the sampled color to the appropriate mode
                match color_mode {
                    super::ColorMode::Fill => {
                        *shared.fill_color = color;
                    }
                    super::ColorMode::Stroke => {
                        *shared.stroke_color = color;
                    }
                }
                // Clear the pending request since we've processed it
                self.pending_eyedropper_sample = None;
            }
        }

        // Consume transform readback results: swap display canvas in as the new float canvas.
        if let Ok(mut results) = TRANSFORM_READBACK_RESULTS
            .get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
            .lock()
        {
            if let Some(rb) = results.remove(&self.instance_id) {
                if let Some(ref mut float) = shared.selection.raster_floating {
                    self.pending_canvas_removal = Some(float.canvas_id);
                    float.canvas_id = rb.display_canvas_id;
                    float.pixels = std::sync::Arc::new(rb.pixels);
                    float.width     = rb.width;
                    float.height    = rb.height;
                    float.x         = rb.x;
                    float.y         = rb.y;
                }
                // Update the selection border to match the new (transformed) float bounds,
                // so marching ants appear around the result after switching tools / Enter.
                // This also replaces the stale pre-transform rect so commit masking is correct.
                shared.selection.raster_selection = Some(
                    lightningbeam_core::selection::RasterSelection::Rect(
                        rb.x, rb.y,
                        rb.x + rb.width  as i32,
                        rb.y + rb.height as i32,
                    )
                );
                // Readback complete — clear transform state.
                self.raster_transform_state = None;
            }
        }

        // Consume warp/liquify readback results: create RasterFillAction and clean up.
        if let Ok(mut results) = WARP_READBACK_RESULTS
            .get_or_init(|| Arc::new(Mutex::new(std::collections::HashMap::new())))
            .lock()
        {
            if let Some(rb) = results.remove(&self.instance_id) {
                if rb.is_float_warp {
                    // Float warp: update the floating selection's pixel data and GPU canvas.
                    // Do NOT write to kf.raw_pixels (it belongs to the full-canvas keyframe).
                    if let Some(float_sel) = shared.selection.raster_floating.as_mut() {
                        float_sel.pixels = std::sync::Arc::new(rb.after_pixels);
                        float_sel.canvas_id = rb.display_canvas_id;
                    }
                    // Release the old anchor canvas (float's original canvas_id, now replaced).
                    self.pending_canvas_removal = Some(rb.anchor_canvas_id);
                } else {
                    use lightningbeam_core::actions::raster_fill::RasterFillAction;
                    let action = RasterFillAction::new(
                        rb.layer_id, rb.time,
                        rb.before_pixels, rb.after_pixels,
                        rb.width, rb.height,
                    ).with_description("Warp");
                    let _ = shared.action_executor.execute(Box::new(action));

                    // Clean up display canvas (deferred: keep alive this frame to avoid flash).
                    self.pending_canvas_removal = Some(rb.display_canvas_id);
                }

                self.painting_canvas = None;
                // Clear tool state.
                if let Some(ws) = self.warp_state.take() {
                    let _ = (ws.anchor_canvas_id, ws.disp_buf_id);
                }
                if let Some(ls) = self.liquify_state.take() {
                    let _ = (ls.anchor_canvas_id, ls.disp_buf_id);
                }
            }
        }

        // Clear transform state if the float was committed externally (by another tool),
        // or if the user switched away from the Transform tool without finishing.
        {
            use lightningbeam_core::tool::Tool;
            let float_gone  = shared.selection.raster_floating.is_none();
            let not_transform = !matches!(*shared.selected_tool, Tool::Transform);
            if (float_gone || not_transform) && self.raster_transform_state.is_some() {
                // If a transform was applied but not yet committed, queue the final dispatch now.
                let needs_dispatch = self.raster_transform_state.as_ref()
                    .map_or(false, |ts| ts.transform_applied && !ts.wants_apply);
                if needs_dispatch {
                    let dispatch = {
                        let ts = self.raster_transform_state.as_ref().unwrap();
                        let (new_w, new_h, new_x, new_y, a00, a01, a10, a11, b0, b1) =
                            Self::compute_transform_params(ts.anchor_w, ts.anchor_h, ts.cx, ts.cy, ts.scale_x, ts.scale_y, ts.angle);
                        PendingTransformDispatch {
                            anchor_canvas_id: ts.anchor_canvas_id,
                            anchor_pixels:    ts.anchor_pixels.clone(),
                            anchor_w: ts.anchor_w, anchor_h: ts.anchor_h,
                            display_canvas_id: ts.display_canvas_id,
                            new_x, new_y, new_w, new_h,
                            a00, a01, a10, a11, b0, b1,
                            is_final_commit: true,
                        }
                    };
                    self.pending_transform_dispatch = Some(dispatch);
                    self.raster_transform_state.as_mut().unwrap().wants_apply = true;
                    // Don't clear state yet — wait for readback (handles stay visible 1 frame).
                } else if !self.raster_transform_state.as_ref().map_or(false, |ts| ts.wants_apply) {
                    // No pending dispatch — just clear.
                    self.raster_transform_state = None;
                }
            }
        }

        // Clear warp/liquify state if user switched away without committing.
        {
            use lightningbeam_core::tool::Tool;
            let not_warp    = !matches!(*shared.selected_tool, Tool::Warp);
            let not_liquify = !matches!(*shared.selected_tool, Tool::Liquify);

            if not_warp && self.warp_state.is_some() {
                if let Some(ws) = self.warp_state.take() {
                    if ws.warp_applied && !ws.wants_commit {
                        // Queue final commit so work isn't lost.
                        let disp_data = Self::extract_grid_disps(&ws.control_points);
                        self.pending_warp_ops.push(PendingWarpOp::WarpApply {
                            anchor_canvas_id: ws.anchor_canvas_id,
                            disp_buf_id: ws.disp_buf_id,
                            display_canvas_id: ws.display_canvas_id,
                            disp_data: Some(disp_data),
                            grid_cols: ws.grid_cols,
                            grid_rows: ws.grid_rows,
                            w: ws.anchor_w, h: ws.anchor_h,
                            final_commit: true,
                            layer_id: ws.layer_id,
                            time: ws.time,
                            is_float_warp: ws.float_offset.is_some(),
                        });
                    } else {
                        // No changes or already committing — just discard.
                        self.pending_canvas_removal = Some(ws.display_canvas_id);
                        self.painting_canvas = None;
                    }
                }
            }

            if not_liquify && self.liquify_state.is_some() {
                if let Some(ls) = self.liquify_state.take() {
                    if ls.liquify_applied && !ls.wants_commit {
                        self.pending_warp_ops.push(PendingWarpOp::LiquifyApply {
                            anchor_canvas_id: ls.anchor_canvas_id,
                            disp_buf_id: ls.disp_buf_id,
                            display_canvas_id: ls.display_canvas_id,
                            w: ls.anchor_w, h: ls.anchor_h,
                            final_commit: true,
                            layer_id: ls.layer_id,
                            time: ls.time,
                            is_float_warp: ls.float_offset.is_some(),
                        });
                    } else {
                        self.pending_canvas_removal = Some(ls.display_canvas_id);
                        self.painting_canvas = None;
                    }
                }
            }
        }

        // Handle input for pan/zoom and tool controls
        self.handle_input(ui, rect, shared);

        // Handle asset drag-and-drop from Asset Library
        if let Some(dragging) = shared.dragging_asset.clone() {
            if let Some(pointer_pos) = ui.ctx().pointer_interact_pos() {
                // Check if pointer is over the stage
                if rect.contains(pointer_pos) {
                    // Visual feedback: draw ghost preview at cursor
                    let preview_size = egui::vec2(60.0, 40.0);
                    let preview_rect = egui::Rect::from_center_size(pointer_pos, preview_size);
                    ui.painter().rect_filled(
                        preview_rect,
                        4.0,
                        egui::Color32::from_rgba_unmultiplied(100, 150, 255, 100),
                    );
                    ui.painter().rect_stroke(
                        preview_rect,
                        4.0,
                        egui::Stroke::new(2.0, egui::Color32::WHITE),
                        egui::StrokeKind::Middle,
                    );
                    ui.painter().text(
                        preview_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &dragging.name,
                        egui::FontId::proportional(10.0),
                        egui::Color32::WHITE,
                    );

                    // Handle drop on mouse release
                    if self.rsp_any_released(ui) {
                        eprintln!("DEBUG STAGE DROP: Dropping clip type {:?}, linked_audio: {:?}",
                            dragging.clip_type, dragging.linked_audio_clip_id);

                        // Convert screen position to world coordinates
                        let canvas_pos = pointer_pos - rect.min;
                        let world_pos = (canvas_pos - self.pan_offset) / self.zoom;

                        // Use playhead time
                        let drop_time = *shared.playback_time;

                        // Find or create a compatible layer
                        let document = shared.action_executor.document();
                        let mut target_layer_id = None;

                        // Check if active layer is compatible
                        if let Some(active_id) = shared.active_layer_id {
                            if let Some(layer) = document.get_layer(active_id) {
                                if super::layer_matches_clip_type(layer, dragging.clip_type) {
                                    target_layer_id = Some(*active_id);
                                }
                            }
                        }

                        // If no compatible active layer, we need to create a new layer
                        if target_layer_id.is_none() {
                            // Create new layer
                            let layer_name = format!("{} Layer", match dragging.clip_type {
                                DragClipType::Vector => "Vector",
                                DragClipType::Video => "Video",
                                DragClipType::AudioSampled => "Audio",
                                DragClipType::AudioMidi => "MIDI",
                                DragClipType::Image => "Image",
                                DragClipType::Effect => "Effect",
                            });
                            let new_layer = super::create_layer_for_clip_type(dragging.clip_type, &layer_name);

                            // Create add layer action
                            let mut add_layer_action = lightningbeam_core::actions::AddLayerAction::new(new_layer);

                            // Execute immediately to get the layer ID
                            let _ = add_layer_action.execute(shared.action_executor.document_mut());
                            target_layer_id = add_layer_action.created_layer_id();

                            // Update active layer to the new layer
                            if let Some(layer_id) = target_layer_id {
                                *shared.active_layer_id = Some(layer_id);
                            }
                        }

                        // Add clip instance or shape to the target layer
                        if let Some(layer_id) = target_layer_id {
                            // For images, create a shape with image fill instead of a clip instance
                            if dragging.clip_type == DragClipType::Image {
                                // TODO: Image fills on DCEL faces are a separate feature.
                                let _ = (layer_id, world_pos);
                                eprintln!("Image drag to stage not yet supported with DCEL backend");
                            } else if dragging.clip_type == DragClipType::Effect {
                                // Handle effect drops specially
                                // Get effect definition from registry or document
                                let effect_def = lightningbeam_core::effect_registry::EffectRegistry::get_by_id(&dragging.clip_id)
                                    .or_else(|| shared.action_executor.document().get_effect_definition(&dragging.clip_id).cloned());

                                if let Some(def) = effect_def {
                                    // Ensure effect definition is in document (copy from registry if built-in)
                                    if shared.action_executor.document().get_effect_definition(&def.id).is_none() {
                                        shared.action_executor.document_mut().add_effect_definition(def.clone());
                                    }

                                    // Create clip instance for effect with 5 second default duration
                                    let clip_instance = ClipInstance::new(def.id)
                                        .with_timeline_start(drop_time)
                                        .with_timeline_duration(5.0);

                                    // Use AddEffectAction for effect layers
                                    let action = lightningbeam_core::actions::AddEffectAction::new(
                                        layer_id,
                                        clip_instance,
                                    );
                                    shared.pending_actions.push(Box::new(action));
                                }
                            } else {
                                // For clips, create a clip instance
                                let mut clip_instance = ClipInstance::new(dragging.clip_id)
                                    .with_timeline_start(drop_time);

                                // For video clips, scale to fit and center in document
                                if dragging.clip_type == DragClipType::Video {
                                    if let Some((video_width, video_height)) = dragging.dimensions {
                                        let doc_width = shared.action_executor.document().width;
                                        let doc_height = shared.action_executor.document().height;

                                        // Calculate scale to fit (use minimum to preserve aspect ratio)
                                        let scale_x = doc_width / video_width;
                                        let scale_y = doc_height / video_height;
                                        let uniform_scale = scale_x.min(scale_y);

                                        clip_instance.transform.scale_x = uniform_scale;
                                        clip_instance.transform.scale_y = uniform_scale;

                                        // Center the video in the document
                                        let scaled_width = video_width * uniform_scale;
                                        let scaled_height = video_height * uniform_scale;
                                        let center_x = (doc_width - scaled_width) / 2.0;
                                        let center_y = (doc_height - scaled_height) / 2.0;

                                        clip_instance.transform.x = center_x;
                                        clip_instance.transform.y = center_y;
                                    }
                                } else {
                                    // Audio clips use mouse drop position
                                    clip_instance.transform.x = world_pos.x as f64;
                                    clip_instance.transform.y = world_pos.y as f64;
                                }

                                // Save instance ID for potential grouping
                                let video_instance_id = clip_instance.id;

                                // Create and queue action for video
                                let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                    layer_id,
                                    clip_instance,
                                );
                                shared.pending_actions.push(Box::new(action));

                                // If video has linked audio, auto-place it and create group
                                if let Some(linked_audio_clip_id) = dragging.linked_audio_clip_id {
                                    eprintln!("DEBUG STAGE: Video has linked audio clip: {}", linked_audio_clip_id);

                                    // Find or create sampled audio track
                                    let audio_layer_id = {
                                        let doc = shared.action_executor.document();
                                        let result = super::find_sampled_audio_track(doc);
                                        if let Some(id) = result {
                                            eprintln!("DEBUG STAGE: Found existing audio track: {}", id);
                                        } else {
                                            eprintln!("DEBUG STAGE: No existing audio track found");
                                        }
                                        result
                                    }.unwrap_or_else(|| {
                                        eprintln!("DEBUG STAGE: Creating new audio track");
                                        // Create new sampled audio layer
                                        let audio_layer = AudioLayer::new_sampled("Audio Track");
                                        let layer_id = shared.action_executor.document_mut().root.add_child(
                                            AnyLayer::Audio(audio_layer)
                                        );
                                        eprintln!("DEBUG STAGE: Created audio layer with ID: {}", layer_id);
                                        layer_id
                                    });

                                    eprintln!("DEBUG STAGE: Using audio layer ID: {}", audio_layer_id);

                                    // Create audio clip instance at same timeline position
                                    let audio_instance = ClipInstance::new(linked_audio_clip_id)
                                        .with_timeline_start(drop_time);
                                    let audio_instance_id = audio_instance.id;

                                    eprintln!("DEBUG STAGE: Created audio instance: {} for clip: {}", audio_instance_id, linked_audio_clip_id);

                                    // Queue audio action
                                    let audio_action = lightningbeam_core::actions::AddClipInstanceAction::new(
                                        audio_layer_id,
                                        audio_instance,
                                    );
                                    shared.pending_actions.push(Box::new(audio_action));
                                    eprintln!("DEBUG STAGE: Queued audio action, total pending: {}", shared.pending_actions.len());

                                    // Create instance group linking video and audio
                                    let mut group = lightningbeam_core::instance_group::InstanceGroup::new();
                                    group.add_member(layer_id, video_instance_id);
                                    group.add_member(audio_layer_id, audio_instance_id);
                                    shared.action_executor.document_mut().add_instance_group(group);
                                    eprintln!("DEBUG STAGE: Created instance group");
                                } else {
                                    eprintln!("DEBUG STAGE: Video has NO linked audio clip!");
                                }
                            }
                        }

                        // Clear drag state
                        *shared.dragging_asset = None;
                    }
                }
            }
        }

        // Register handler for pending view actions (two-phase dispatch)
        // Priority: Mouse-over (0-99) > Fallback Stage(1000) > Fallback Timeline(1001) etc.
        const STAGE_MOUSE_OVER_PRIORITY: u32 = 0;
        const STAGE_FALLBACK_PRIORITY: u32 = 1000;

        let mouse_over = ui.rect_contains_pointer(rect);

        // Determine our priority for this action
        let our_priority = if mouse_over {
            STAGE_MOUSE_OVER_PRIORITY  // High priority - mouse is over this pane
        } else {
            STAGE_FALLBACK_PRIORITY    // Low priority - just a fallback option
        };

        // Check if we should register as a handler (better priority than current best)
        let should_register = shared.pending_view_action.is_some() &&
            shared.fallback_pane_priority.map_or(true, |p| our_priority < p);

        if should_register {
            // Update fallback priority tracker
            *shared.fallback_pane_priority = Some(our_priority);

            // Register as a handler (don't execute yet - that happens after all panes render)
            if let Some(action) = &shared.pending_view_action {
                use crate::menu::MenuAction;

                // Determine zoom center point
                let center = if mouse_over {
                    // Use mouse position for zoom-to-cursor
                    let mouse_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or(rect.center());
                    mouse_pos - rect.min
                } else {
                    // Use center of viewport for fallback
                    rect.size() / 2.0
                };

                // Only register for actions we can handle
                match action {
                    MenuAction::ZoomIn | MenuAction::ZoomOut |
                    MenuAction::ActualSize | MenuAction::RecenterView => {
                        shared.pending_handlers.push(super::ViewActionHandler {
                            priority: our_priority,
                            pane_path: _path.clone(),
                            zoom_center: center,
                        });
                    }
                    _ => {
                        // Not a view action we handle - reset priority so others can try
                        *shared.fallback_pane_priority = None;
                    }
                }
            }
        }

        // Calculate drag delta for preview rendering (clip-local space)
        let drag_delta = if let lightningbeam_core::tool::ToolState::DraggingSelection { ref start_mouse, .. } = shared.tool_state {
            // Get current mouse position in clip-local coordinates (matching start_mouse)
            if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let mouse_canvas_pos = mouse_pos - rect.min;
                let world_mouse_doc = (mouse_canvas_pos - self.pan_offset) / self.zoom;
                let world_mouse = self.doc_to_clip_local(world_mouse_doc, shared);

                let delta_x = world_mouse.x as f64 - start_mouse.x;
                let delta_y = world_mouse.y as f64 - start_mouse.y;

                Some(vello::kurbo::Vec2::new(delta_x, delta_y))
            } else {
                None
            }
        } else {
            None
        };

        // Compute mouse world position for hover hit testing in the Vello callback
        let mouse_world_pos = ui.input(|i| i.pointer.hover_pos())
            .filter(|pos| rect.contains(*pos))
            .map(|pos| {
                let canvas_pos = pos - rect.min;
                let doc_pos = (canvas_pos - self.pan_offset) / self.zoom;
                let local = self.doc_to_clip_local(doc_pos, shared);
                vello::kurbo::Point::new(local.x as f64, local.y as f64)
            });

        // Compute transform_display for the VelloCallback.
        // Only override the float blit once the display canvas has actual content
        // (transform_applied = true). Before the first drag, show the regular float canvas.
        let transform_display = self.raster_transform_state.as_ref()
            .filter(|ts| ts.transform_applied)
            .map(|ts| {
                let (new_w, new_h, new_x, new_y, ..) = Self::compute_transform_params(
                    ts.anchor_w, ts.anchor_h, ts.cx, ts.cy, ts.scale_x, ts.scale_y, ts.angle,
                );
                TransformDisplayInfo {
                    display_canvas_id: ts.display_canvas_id,
                    x: new_x, y: new_y, w: new_w, h: new_h,
                }
            });

        // Compute warp_display: show the warp/liquify display canvas in place of the layer
        // (for full-layer warp) or as float blit override (for float warp via transform_display).
        let warp_display = self.warp_state.as_ref()
            .filter(|ws| ws.warp_applied && ws.float_offset.is_none())
            .map(|ws| (ws.layer_id, ws.display_canvas_id))
            .or_else(|| self.liquify_state.as_ref()
                .filter(|ls| ls.liquify_applied && ls.float_offset.is_none())
                .map(|ls| (ls.layer_id, ls.display_canvas_id)));

        // For float warp/liquify: override the float blit with the display canvas.
        let transform_display = transform_display.or_else(|| {
            self.warp_state.as_ref()
                .and_then(|ws| ws.float_offset.map(|(ox, oy)| TransformDisplayInfo {
                    display_canvas_id: ws.display_canvas_id,
                    x: ox, y: oy, w: ws.anchor_w, h: ws.anchor_h,
                }))
        }).or_else(|| {
            self.liquify_state.as_ref()
                .and_then(|ls| ls.float_offset.map(|(ox, oy)| TransformDisplayInfo {
                    display_canvas_id: ls.display_canvas_id,
                    x: ox, y: oy, w: ls.anchor_w, h: ls.anchor_h,
                }))
        });

        // Scan for raster keyframes whose texture_dirty flag was set since last frame
        // (e.g. by undo/redo or a stroke action execute/rollback). Must run BEFORE
        // document_arc() is called below so that Arc::make_mut does not clone the document.
        {
            let doc = shared.action_executor.document_mut();
            fn collect_dirty(layers: &mut [lightningbeam_core::layer::AnyLayer], out: &mut Vec<uuid::Uuid>) {
                for layer in layers.iter_mut() {
                    if let lightningbeam_core::layer::AnyLayer::Raster(rl) = layer {
                        for kf in &mut rl.keyframes {
                            if kf.texture_dirty {
                                out.push(kf.id);
                                kf.texture_dirty = false;
                            }
                        }
                    }
                }
            }
            collect_dirty(&mut doc.root.children, &mut self.pending_layer_cache_removals);
        }

        // Use egui's custom painting callback for Vello
        // document_arc() returns Arc<Document> - cheap pointer copy, not deep clone
        let callback = VelloCallback { ctx: VelloRenderContext {
            rect,
            pan_offset: self.pan_offset,
            zoom: self.zoom,
            instance_id: self.instance_id,
            document: shared.action_executor.document_arc(),
            tool_state: shared.tool_state.clone(),
            active_layer_id: *shared.active_layer_id,
            drag_delta,
            selection: shared.selection.clone(),
            fill_color: *shared.fill_color,
            stroke_color: *shared.stroke_color,
            stroke_width: *shared.stroke_width,
            selected_tool: *shared.selected_tool,
            fill_enabled: *shared.fill_enabled,
            eyedropper_request: self.pending_eyedropper_sample,
            playback_time: *shared.playback_time,
            video_manager: shared.video_manager.clone(),
            target_format: shared.target_format,
            editing_clip_id: shared.editing_clip_id,
            editing_instance_id: shared.editing_instance_id,
            editing_parent_layer_id: shared.editing_parent_layer_id,
            region_selection: shared.region_selection.clone(),
            mouse_world_pos,
            webcam_frame: shared.webcam_frame.clone(),
            pending_raster_dabs: self.pending_raster_dabs.take(),
            pending_transform_dispatch: self.pending_transform_dispatch.take(),
            transform_display,
            pending_warp_ops: std::mem::take(&mut self.pending_warp_ops),
            warp_display,
            pending_gradient_op: self.pending_gradient_op.take(),
            instance_id_for_readback: self.instance_id,
            painting_canvas: self.painting_canvas,
            pending_canvas_removal: self.pending_canvas_removal.take(),
            painting_float: self.painting_float,
            brush_preview_pixels: shared.brush_preview_pixels.clone(),
            active_tool_render: self.active_raster_tool.as_ref().map(|(_, ws)| {
                crate::raster_tool::ActiveToolRender {
                    b_canvas_id: ws.b_canvas_id,
                    x: ws.x, y: ws.y,
                    width: ws.width, height: ws.height,
                    layer_id: match &ws.source {
                        crate::raster_tool::WorkspaceSource::Layer { layer_id, .. } => Some(*layer_id),
                        crate::raster_tool::WorkspaceSource::Float => None,
                    },
                }
            }),
            pending_canvas_removals: std::mem::take(&mut self.pending_canvas_removals),
            pending_workspace_init: self.pending_workspace_init.take(),
            pending_tool_gpu_work: self.active_raster_tool.as_mut()
                .and_then(|(tool, _)| tool.take_pending_gpu_work()),
            pending_layer_cache_removals: std::mem::take(&mut self.pending_layer_cache_removals),
            pending_tool_readback_b: self.pending_tool_readback_b.take(),
        }};

        let cb = egui_wgpu::Callback::new_paint_callback(
            rect,
            callback,
        );

        ui.painter().add(cb);

        // Gradient direction arrow overlay for vector gradient drags.
        if matches!(*shared.selected_tool, lightningbeam_core::tool::Tool::Gradient) {
            if let Some(ref gs) = self.vector_gradient_state {
                let mut painter = ui.ctx().layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("vgrad_arrow"),
                ));
                painter.set_clip_rect(rect);
                let w2s = |w: egui::Vec2| -> egui::Pos2 {
                    rect.min + self.pan_offset + w * self.zoom
                };
                let p0 = w2s(gs.start);
                let p1 = w2s(gs.end);
                painter.line_segment([p0, p1], egui::Stroke::new(2.0, egui::Color32::WHITE));
                painter.circle_stroke(p0, 5.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
                painter.circle_filled(p1, 4.0, egui::Color32::WHITE);
            }
        }

        // Show camera info overlay
        let info_color = shared.theme.text_color(&["#stage", ".text-secondary"], ui.ctx(), egui::Color32::from_gray(200));
        ui.painter().text(
            rect.min + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("Vello Stage (zoom: {:.2}, pan: {:.0},{:.0})",
                self.zoom, self.pan_offset.x, self.pan_offset.y),
            egui::FontId::proportional(14.0),
            info_color,
        );

        // Render breadcrumb navigation when inside a movie clip
        if shared.editing_clip_id.is_some() {
            let document = shared.action_executor.document();
            // Build breadcrumb names from the editing context
            // We only have the current clip_id, so show "Scene 1 > ClipName"
            let clip_name = shared.editing_clip_id
                .and_then(|id| document.get_vector_clip(&id))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            let breadcrumb_y = rect.min.y + 30.0;
            let breadcrumb_x = rect.min.x + 10.0;

            // Background pill
            let scene_text = "Scene 1";
            let separator = " > ";
            let full_text = format!("{}{}{}", scene_text, separator, clip_name);
            let font = egui::FontId::proportional(13.0);
            let galley = ui.painter().layout_no_wrap(full_text.clone(), font.clone(), egui::Color32::WHITE);
            let text_rect = egui::Rect::from_min_size(
                egui::pos2(breadcrumb_x, breadcrumb_y),
                galley.size() + egui::vec2(16.0, 8.0),
            );
            ui.painter().rect_filled(
                text_rect,
                4.0,
                egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
            );

            // "Scene 1" as clickable (exit clip)
            let scene_galley = ui.painter().layout_no_wrap(
                scene_text.to_string(), font.clone(), egui::Color32::from_rgb(120, 180, 255),
            );
            let scene_rect = egui::Rect::from_min_size(
                egui::pos2(breadcrumb_x + 8.0, breadcrumb_y + 4.0),
                scene_galley.size(),
            );
            let scene_response = ui.allocate_rect(scene_rect, egui::Sense::click());
            ui.painter().galley(scene_rect.min, scene_galley, egui::Color32::WHITE);
            if scene_response.clicked() {
                *shared.pending_exit_clip = true;
            }
            if scene_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }

            // Separator + clip name (not clickable, it's the current level)
            let rest_text = format!("{}{}", separator, clip_name);
            ui.painter().text(
                egui::pos2(scene_rect.max.x, breadcrumb_y + 4.0),
                egui::Align2::LEFT_TOP,
                rest_text,
                font,
                egui::Color32::WHITE,
            );
        }

        // Render vector editing overlays (vertices, control points, etc.)
        self.render_vector_editing_overlays(ui, rect, shared);

        // Raster selection overlays: marching ants + floating selection texture
        self.render_raster_selection_overlays(ui, rect, shared);

        // Raster transform handles (drawn after Vello scene so they appear on top)
        if let Some(ref ts) = self.raster_transform_state {
            let zoom = self.zoom;
            let pan  = self.pan_offset;
            Self::draw_raster_transform_handles_static(ui, rect, ts, zoom, pan);
        }

        // Render snap indicator (works for all tools, not just Select/BezierEdit)
        self.render_snap_indicator(ui, rect, shared);

        // Draw ghost cursor during test mode replay
        #[cfg(debug_assertions)]
        if let Some((wx, wy)) = shared.test_mode.replay_cursor_pos {
            // Convert world-space position to screen-space
            let screen_pos = rect.min + self.pan_offset + egui::vec2(wx as f32, wy as f32) * self.zoom;
            let painter = ui.painter_at(rect);
            // Crosshair
            let arm = 10.0;
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 100, 100, 200));
            painter.line_segment(
                [screen_pos - egui::vec2(arm, 0.0), screen_pos + egui::vec2(arm, 0.0)],
                stroke,
            );
            painter.line_segment(
                [screen_pos - egui::vec2(0.0, arm), screen_pos + egui::vec2(0.0, arm)],
                stroke,
            );
            // Circle
            painter.circle_stroke(
                screen_pos,
                6.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgba_unmultiplied(255, 100, 100, 200)),
            );
        }

        // Draw clone source indicator when clone stamp or healing brush tool is selected.
        let tool_uses_alt = crate::tools::raster_tool_def(shared.selected_tool)
            .map_or(false, |d| d.uses_alt_click());
        if tool_uses_alt {
            if let Some(src_world) = shared.raster_settings.clone_source {
                let src_canvas = egui::vec2(
                    src_world.x * self.zoom + self.pan_offset.x,
                    src_world.y * self.zoom + self.pan_offset.y,
                );
                let src_screen = rect.min + src_canvas;
                let painter = ui.painter_at(rect);
                let r = 8.0_f32;    // circle radius
                let arm = 14.0_f32; // arm half-length (extends past the circle)
                let gap = r + 2.0;  // gap between circle edge and arm start
                for (width, color) in [
                    (3.0_f32, egui::Color32::BLACK),
                    (1.5_f32, egui::Color32::WHITE),
                ] {
                    let s = egui::Stroke::new(width, color);
                    painter.circle_stroke(src_screen, r, s);
                    painter.line_segment([src_screen - egui::vec2(arm, 0.0), src_screen - egui::vec2(gap, 0.0)], s);
                    painter.line_segment([src_screen + egui::vec2(gap, 0.0), src_screen + egui::vec2(arm, 0.0)], s);
                    painter.line_segment([src_screen - egui::vec2(0.0, arm), src_screen - egui::vec2(0.0, gap)], s);
                    painter.line_segment([src_screen + egui::vec2(0.0, gap), src_screen + egui::vec2(0.0, arm)], s);
                }
            }
        }

        // Set custom tool cursor when pointer is over the stage canvas.
        // Raster paint tools get a brush-size outline; everything else uses the SVG cursor.
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pos) {
                use lightningbeam_core::tool::Tool;
                let is_raster_paint = matches!(
                    *shared.selected_tool,
                    Tool::Draw | Tool::Pencil | Tool::Pen | Tool::Airbrush
                    | Tool::Erase | Tool::Smudge
                    | Tool::CloneStamp | Tool::HealingBrush | Tool::PatternStamp
                    | Tool::DodgeBurn | Tool::Sponge | Tool::BlurSharpen
                    | Tool::QuickSelect
                ) && shared.active_layer_id.and_then(|id| {
                    shared.action_executor.document().get_layer(&id)
                }).map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));

                // Only override the cursor when no higher-order layer (e.g. a modal dialog)
                // is covering the canvas at this position.
                let canvas_is_topmost = ui.ctx()
                    .layer_id_at(pos)
                    .map_or(true, |l| l == ui.layer_id());

                if is_raster_paint && canvas_is_topmost {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    self.draw_brush_cursor(ui, rect, pos, shared);
                } else if is_raster_paint {
                    // A modal is covering the canvas — let the system cursor show normally.
                } else {
                    crate::custom_cursor::set(
                        ui.ctx(),
                        crate::custom_cursor::CustomCursor::from_tool(*shared.selected_tool),
                    );
                }
            }
        }
    }

    fn name(&self) -> &str {
        "Stage"
    }
}
