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
        let renderer = vello::Renderer::new(
            device,
            vello::RendererOptions {
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: std::num::NonZeroUsize::new(1),
                pipeline_cache: None,
            },
        ).map_err(|e| format!("Failed to create Vello renderer: {}", e))?;

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
                SharedVelloResources::new(device, self.ctx.video_manager.clone(), self.ctx.target_format).expect("Failed to initialize shared Vello resources")
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
                                float_sel.pixels.clone()
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
                                clone_src_offset: None,
                                pattern_type: 0,
                                pattern_scale: 32.0,
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

            let mut image_cache = shared.image_cache.lock().unwrap();

            let composite_result = lightningbeam_core::renderer::render_document_for_compositing(
                &self.ctx.document,
                camera_transform,
                &mut image_cache,
                &shared.video_manager,
                self.ctx.webcam_frame.as_ref(),
            );
            drop(image_cache);

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
                // Check if this raster layer has a live GPU canvas that should be
                // blitted every frame, even when no new dabs arrived this frame.
                // `painting_canvas` persists for the entire stroke duration.
                // When painting into float (B), the GPU canvas is B's canvas — don't
                // use it to replace the Vello scene for the layer (A must still render
                // via Vello).
                let gpu_canvas_kf: Option<uuid::Uuid> = if self.ctx.painting_float {
                    None
                } else {
                    self.ctx.painting_canvas
                        .filter(|(layer_id, _)| *layer_id == rendered_layer.layer_id)
                        .map(|(_, kf_id)| kf_id)
                };

                if !rendered_layer.has_content && gpu_canvas_kf.is_none() {
                    continue;
                }

                match &rendered_layer.layer_type {
                    RenderedLayerType::Content => {
                        // Regular content layer - render to sRGB, convert to linear, then composite
                        let srgb_handle = buffer_pool.acquire(device, layer_spec);
                        let hdr_layer_handle = buffer_pool.acquire(device, hdr_spec);

                        if let (Some(srgb_view), Some(hdr_layer_view), Some(hdr_view)) = (
                            buffer_pool.get_view(srgb_handle),
                            buffer_pool.get_view(hdr_layer_handle),
                            &instance_resources.hdr_texture_view,
                        ) {
                            // GPU canvas blit path: if a live GPU canvas exists for this
                            // raster layer, blit it directly into the HDR buffer (premultiplied
                            // linear → Rgba16Float), bypassing the sRGB intermediate entirely.
                            // Vello path: render to sRGB buffer → srgb_to_linear → HDR buffer.
                            let used_gpu_canvas = if let Some(kf_id) = gpu_canvas_kf {
                                let mut used = false;
                                if let Ok(gpu_brush) = shared.gpu_brush.lock() {
                                    if let Some(canvas) = gpu_brush.canvases.get(&kf_id) {
                                        let camera = crate::gpu_brush::CameraParams {
                                            pan_x:      self.ctx.pan_offset.x,
                                            pan_y:      self.ctx.pan_offset.y,
                                            zoom:       self.ctx.zoom,
                                            canvas_w:   canvas.width as f32,
                                            canvas_h:   canvas.height as f32,
                                            viewport_w: width as f32,
                                            viewport_h: height as f32,
                                            _pad: 0.0,
                                        };
                                        shared.canvas_blit.blit(
                                            device, queue,
                                            canvas.src_view(),
                                            hdr_layer_view,  // blit directly to HDR
                                            &camera,
                                            None,  // no mask on layer canvas blit
                                        );
                                        used = true;
                                    }
                                }
                                used
                            } else {
                                false
                            };

                            if !used_gpu_canvas {
                                // Render layer scene to sRGB buffer, then convert to HDR
                                if let Ok(mut renderer) = shared.renderer.lock() {
                                    renderer.render_to_texture(device, queue, &rendered_layer.scene, srgb_view, &layer_render_params).ok();
                                }
                                let mut convert_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                    label: Some("layer_srgb_to_linear_encoder"),
                                });
                                shared.srgb_to_linear.convert(device, &mut convert_encoder, srgb_view, hdr_layer_view);
                                queue.submit(Some(convert_encoder.finish()));
                            }

                            // Composite this layer onto the HDR accumulator with its opacity
                            let compositor_layer = lightningbeam_core::gpu::CompositorLayer::new(
                                hdr_layer_handle,
                                rendered_layer.opacity,
                                rendered_layer.blend_mode,
                            );

                            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                                label: Some("layer_composite_encoder"),
                            });
                            shared.compositor.composite(
                                device,
                                queue,
                                &mut encoder,
                                &[compositor_layer],
                                &buffer_pool,
                                hdr_view,
                                None, // Don't clear - blend onto existing content
                            );
                            queue.submit(Some(encoder.finish()));
                        }

                        buffer_pool.release(srgb_handle);
                        buffer_pool.release(hdr_layer_handle);
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

            // Blit the float GPU canvas on top of all composited layers.
            // The float_mask_view clips to the selection shape (None = full float visible).
            if let Some(ref float_sel) = self.ctx.selection.raster_floating {
                let float_canvas_id = float_sel.canvas_id;
                let float_x = float_sel.x;
                let float_y = float_sel.y;
                let float_w = float_sel.width;
                let float_h = float_sel.height;
                if let Ok(gpu_brush) = shared.gpu_brush.lock() {
                    if let Some(canvas) = gpu_brush.canvases.get(&float_canvas_id) {
                        let float_hdr_handle = buffer_pool.acquire(device, hdr_spec);
                        if let (Some(fhdr_view), Some(hdr_view)) = (
                            buffer_pool.get_view(float_hdr_handle),
                            &instance_resources.hdr_texture_view,
                        ) {
                            let fcamera = crate::gpu_brush::CameraParams {
                                pan_x:      self.ctx.pan_offset.x + float_x as f32 * self.ctx.zoom,
                                pan_y:      self.ctx.pan_offset.y + float_y as f32 * self.ctx.zoom,
                                zoom:       self.ctx.zoom,
                                canvas_w:   float_w as f32,
                                canvas_h:   float_h as f32,
                                viewport_w: width as f32,
                                viewport_h: height as f32,
                                _pad: 0.0,
                            };
                            // Blit directly to HDR (straight-alpha linear, no sRGB step)
                            shared.canvas_blit.blit(
                                device, queue,
                                canvas.src_view(),
                                fhdr_view,
                                &fcamera,
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

            // Advance frame counter for buffer cleanup
            buffer_pool.next_frame();
            drop(buffer_pool);

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
                            if face.fill_color.is_none() && face.image_fill.is_none() { continue; }
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


                    // 3. Draw rectangle creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingRectangle { ref start_point, ref current_point, centered, constrain_square, .. } = self.ctx.tool_state {
                        use vello::kurbo::Point;

                        // Calculate rectangle bounds based on mode (same logic as in handler)
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
                                let width = max_x - min_x;
                                let height = max_y - min_y;
                                let size = width.max(height);

                                if current_point.x > start_point.x {
                                    max_x = min_x + size;
                                } else {
                                    min_x = max_x - size;
                                }

                                if current_point.y > start_point.y {
                                    max_y = min_y + size;
                                } else {
                                    min_y = max_y - size;
                                }
                            }

                            (max_x - min_x, max_y - min_y, Point::new(min_x, min_y))
                        };

                        if width > 0.0 && height > 0.0 {
                            let rect = KurboRect::new(0.0, 0.0, width, height);
                            let preview_transform = overlay_transform * Affine::translate((position.x, position.y));

                            if self.ctx.fill_enabled {
                                let fill_color = Color::from_rgba8(
                                    self.ctx.fill_color.r(),
                                    self.ctx.fill_color.g(),
                                    self.ctx.fill_color.b(),
                                    self.ctx.fill_color.a(),
                                );
                                scene.fill(
                                    Fill::NonZero,
                                    preview_transform,
                                    fill_color,
                                    None,
                                    &rect,
                                );
                            }

                            let stroke_color = Color::from_rgba8(
                                self.ctx.stroke_color.r(),
                                self.ctx.stroke_color.g(),
                                self.ctx.stroke_color.b(),
                                self.ctx.stroke_color.a(),
                            );
                            scene.stroke(
                                &Stroke::new(self.ctx.stroke_width),
                                preview_transform,
                                stroke_color,
                                None,
                                &rect,
                            );
                        }
                    }

                    // 4. Draw ellipse creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingEllipse { ref start_point, ref current_point, corner_mode, constrain_circle, .. } = self.ctx.tool_state {
                        use vello::kurbo::{Point, Circle as KurboCircle, Ellipse};

                        // Calculate ellipse parameters based on mode (same logic as in handler)
                        let (rx, ry, position) = if corner_mode {
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
                            let dx = (current_point.x - start_point.x).abs();
                            let dy = (current_point.y - start_point.y).abs();

                            let (rx, ry) = if constrain_circle {
                                let radius = (dx * dx + dy * dy).sqrt();
                                (radius, radius)
                            } else {
                                (dx, dy)
                            };

                            (rx, ry, *start_point)
                        };

                        if rx > 0.0 && ry > 0.0 {
                            let preview_transform = overlay_transform * Affine::translate((position.x, position.y));

                            let fill_color = Color::from_rgba8(
                                self.ctx.fill_color.r(),
                                self.ctx.fill_color.g(),
                                self.ctx.fill_color.b(),
                                self.ctx.fill_color.a(),
                            );
                            let stroke_color = Color::from_rgba8(
                                self.ctx.stroke_color.r(),
                                self.ctx.stroke_color.g(),
                                self.ctx.stroke_color.b(),
                                self.ctx.stroke_color.a(),
                            );

                            if rx == ry {
                                let circle = KurboCircle::new((0.0, 0.0), rx);
                                if self.ctx.fill_enabled {
                                    scene.fill(Fill::NonZero, preview_transform, fill_color, None, &circle);
                                }
                                scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, stroke_color, None, &circle);
                            } else {
                                let ellipse = Ellipse::new((0.0, 0.0), (rx, ry), 0.0);
                                if self.ctx.fill_enabled {
                                    scene.fill(Fill::NonZero, preview_transform, fill_color, None, &ellipse);
                                }
                                scene.stroke(&Stroke::new(self.ctx.stroke_width), preview_transform, stroke_color, None, &ellipse);
                            }
                        }
                    }

                    // 5. Draw line creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingLine { ref start_point, ref current_point, .. } = self.ctx.tool_state {
                        use vello::kurbo::Line;

                        // Calculate line length
                        let dx = current_point.x - start_point.x;
                        let dy = current_point.y - start_point.y;
                        let length = (dx * dx + dy * dy).sqrt();

                        if length > 0.0 {
                            // Use actual stroke color for line preview
                            let stroke_color = Color::from_rgba8(
                                self.ctx.stroke_color.r(),
                                self.ctx.stroke_color.g(),
                                self.ctx.stroke_color.b(),
                                self.ctx.stroke_color.a(),
                            );

                            // Draw the line directly
                            let line = Line::new(*start_point, *current_point);
                            scene.stroke(
                                &Stroke::new(2.0),
                                overlay_transform,
                                stroke_color,
                                None,
                                &line,
                            );
                        }
                    }

                    // 6. Draw polygon creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingPolygon { ref center, ref current_point, num_sides, .. } = self.ctx.tool_state {
                        use vello::kurbo::{BezPath, Point};
                        use std::f64::consts::PI;

                        // Calculate radius
                        let dx = current_point.x - center.x;
                        let dy = current_point.y - center.y;
                        let radius = (dx * dx + dy * dy).sqrt();

                        if radius > 5.0 && num_sides >= 3 {
                            let preview_transform = overlay_transform * Affine::translate((center.x, center.y));

                            // Use actual fill color (same as final shape)
                            let fill_color = Color::from_rgba8(
                                self.ctx.fill_color.r(),
                                self.ctx.fill_color.g(),
                                self.ctx.fill_color.b(),
                                self.ctx.fill_color.a(),
                            );

                            // Create the polygon path inline
                            let mut path = BezPath::new();
                            let angle_step = 2.0 * PI / num_sides as f64;
                            let start_angle = -PI / 2.0;

                            // First vertex
                            let first_x = radius * start_angle.cos();
                            let first_y = radius * start_angle.sin();
                            path.move_to(Point::new(first_x, first_y));

                            // Add remaining vertices
                            for i in 1..num_sides {
                                let angle = start_angle + angle_step * i as f64;
                                let x = radius * angle.cos();
                                let y = radius * angle.sin();
                                path.line_to(Point::new(x, y));
                            }

                            path.close_path();

                            if self.ctx.fill_enabled {
                                scene.fill(
                                    Fill::NonZero,
                                    preview_transform,
                                    fill_color,
                                    None,
                                    &path,
                                );
                            }

                            let stroke_color = Color::from_rgba8(
                                self.ctx.stroke_color.r(),
                                self.ctx.stroke_color.g(),
                                self.ctx.stroke_color.b(),
                                self.ctx.stroke_color.a(),
                            );
                            scene.stroke(
                                &Stroke::new(self.ctx.stroke_width),
                                preview_transform,
                                stroke_color,
                                None,
                                &path,
                            );
                        }
                    }

                    // 7. Draw path drawing preview
                    if let lightningbeam_core::tool::ToolState::DrawingPath { ref points, .. } = self.ctx.tool_state {
                        use vello::kurbo::BezPath;

                        if points.len() >= 2 {
                            // Build a simple line path from the raw points for preview
                            let mut preview_path = BezPath::new();
                            preview_path.move_to(points[0]);
                            for point in &points[1..] {
                                preview_path.line_to(*point);
                            }

                            // Draw fill if enabled
                            if self.ctx.fill_enabled {
                                let fill_color = Color::from_rgba8(
                                    self.ctx.fill_color.r(),
                                    self.ctx.fill_color.g(),
                                    self.ctx.fill_color.b(),
                                    self.ctx.fill_color.a(),
                                );
                                scene.fill(
                                    Fill::NonZero,
                                    overlay_transform,
                                    fill_color,
                                    None,
                                    &preview_path,
                                );
                            }

                            let stroke_color = Color::from_rgba8(
                                self.ctx.stroke_color.r(),
                                self.ctx.stroke_color.g(),
                                self.ctx.stroke_color.b(),
                                self.ctx.stroke_color.a(),
                            );

                            scene.stroke(
                                &Stroke::new(self.ctx.stroke_width),
                                overlay_transform,
                                stroke_color,
                                None,
                                &preview_path,
                            );
                        }
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
    /// Clone stamp: world-space source point set by Alt+click.
    clone_source: Option<egui::Vec2>,
    /// Clone stamp: (source_world - drag_start_world) computed at stroke start.
    /// Constant for the entire stroke; cleared when the stroke ends.
    clone_stroke_offset: Option<(f32, f32)>,
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
            clone_source: None,
            clone_stroke_offset: None,
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
        _shift_held: bool,
        _ctrl_held: bool,
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

        let point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

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
                // Calculate line length to ensure it's not too small
                let dx = current_point.x - start_point.x;
                let dy = current_point.y - start_point.y;
                let length = (dx * dx + dy * dy).sqrt();

                // Only create shape if line has reasonable length
                if length > 1.0 {
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

                    // Clear tool state to stop preview rendering
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

        let point = self.snap_point(Point::new(world_pos.x as f64, world_pos.y as f64), shared);

        // Mouse down: start creating polygon (center point)
        if self.rsp_drag_started(response) || self.rsp_clicked(response) {
            *shared.tool_state = ToolState::CreatingPolygon {
                center: point,
                current_point: point,
                num_sides: 5,  // Default to 5 sides (pentagon)
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
            float.canvas_before, canvas_after,
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
            pixels: float_pixels,
            width: w,
            height: h,
            x: x0,
            y: y0,
            layer_id,
            time,
            canvas_before,
            canvas_id: uuid::Uuid::new_v4(),
        });
    }

    /// `self.pending_raster_dabs` for dispatch by `VelloCallback::prepare()`.
    ///
    /// The actual pixel rendering happens on the GPU (compute shader).  The CPU
    /// only does dab placement arithmetic (cheap).  On stroke end a readback is
    /// requested so the undo system can capture the final pixel state.
    fn handle_raster_stroke_tool(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        world_pos: egui::Vec2,
        blend_mode: lightningbeam_core::raster_layer::RasterBlendMode,
        shared: &mut SharedPaneState,
    ) {
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
            // Start from the active preset for this tool, then override the
            // user-controlled slider values.
            use lightningbeam_core::raster_layer::RasterBlendMode;
            let (base_settings, radius, opacity, hardness, spacing) = match blend_mode {
                RasterBlendMode::Erase => (
                    shared.active_eraser_settings.clone(),
                    *shared.eraser_radius,
                    *shared.eraser_opacity,
                    *shared.eraser_hardness,
                    *shared.eraser_spacing,
                ),
                RasterBlendMode::Smudge => (
                    lightningbeam_core::brush_settings::BrushSettings::default(),
                    *shared.smudge_radius,
                    1.0, // opacity fixed at 1.0; strength is a separate smudge_dist multiplier
                    *shared.smudge_hardness,
                    *shared.smudge_spacing,
                ),
                _ => (
                    shared.active_brush_settings.clone(),
                    *shared.brush_radius,
                    *shared.brush_opacity,
                    *shared.brush_hardness,
                    *shared.brush_spacing,
                ),
            };
            let mut b = base_settings;
            // Compensate for pressure_radius_gain so that the UI-chosen radius is the
            // actual rendered radius at our fixed mouse pressure of 1.0.
            // radius_at_pressure(1.0) = exp(radius_log + gain × 0.5)
            // → radius_log = ln(radius) - gain × 0.5
            b.radius_log      = radius.ln() - b.pressure_radius_gain * 0.5;
            b.hardness        = hardness;
            b.opaque          = opacity;
            b.dabs_per_radius = spacing;
            if matches!(blend_mode, RasterBlendMode::Smudge) {
                // Zero dabs_per_actual_radius so the spacing slider is the sole density control.
                b.dabs_per_actual_radius = 0.0;
                // strength controls how far behind the stroke to sample (smudge_dist multiplier).
                // smudge_dist = radius * exp(smudge_radius_log), so log(strength) gives the ratio.
                b.smudge_radius_log = *shared.smudge_strength; // linear [0,1] strength
            }
            b
        };

        let color = if matches!(blend_mode, lightningbeam_core::raster_layer::RasterBlendMode::Erase) {
            [1.0f32, 1.0, 1.0, 1.0]
        } else {
            let c = if *shared.brush_use_fg { *shared.stroke_color } else { *shared.fill_color };
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
                self.clone_stroke_offset = self.clone_source.map(|s| (
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
                    let buf = float.pixels.clone();
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
                    clone_src_offset: self.clone_stroke_offset,
                    pattern_type: *shared.pattern_type,
                    pattern_scale: *shared.pattern_scale,
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
                    clone_src_offset: self.clone_stroke_offset,
                    pattern_type: *shared.pattern_type,
                    pattern_scale: *shared.pattern_scale,
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
                        let clone_src_offset = self.clone_stroke_offset;
                        let seg = StrokeRecord {
                            brush_settings: brush.clone(),
                            color,
                            blend_mode,
                            clone_src_offset,
                            pattern_type: *shared.pattern_type,
                            pattern_scale: *shared.pattern_scale,
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
                            clone_src_offset: self.clone_stroke_offset,
                            pattern_type: *shared.pattern_type,
                            pattern_scale: *shared.pattern_scale,
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

        let Some(layer_id) = *shared.active_layer_id else { return };
        let doc = shared.action_executor.document();
        let Some(kf) = doc.get_layer(&layer_id).and_then(|l| {
            if let AnyLayer::Raster(rl) = l { rl.keyframe_at(*shared.playback_time) } else { None }
        }) else { return };
        let (canvas_w, canvas_h) = (kf.width as i32, kf.height as i32);

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
                    shared.selection.raster_selection = Some(RasterSelection::Rect(x0, y0, x1, y1));
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
                        shared.selection.raster_selection = Some(RasterSelection::Rect(x0, y0, x1, y1));
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
        use lightningbeam_core::shape::ShapeColor;
        use lightningbeam_core::actions::PaintBucketAction;
        use vello::kurbo::Point;

        // Check if we have an active vector layer
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(&active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        if self.rsp_clicked(response) {
            let click_point = Point::new(world_pos.x as f64, world_pos.y as f64);
            let fill_color = ShapeColor::from_egui(*shared.fill_color);

            let action = PaintBucketAction::new(
                *active_layer_id,
                *shared.playback_time,
                click_point,
                fill_color,
            );
            let _ = shared.action_executor.execute(Box::new(action));
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

        // Clone stamp / healing brush: Alt+click sets the source point regardless of the alt-pan guard below.
        {
            use lightningbeam_core::tool::Tool;
            if matches!(*shared.selected_tool, Tool::CloneStamp | Tool::HealingBrush)
                && alt_held
                && self.rsp_primary_pressed(ui)
                && response.hovered()
            {
                eprintln!("[clone/healing] set clone source to ({:.1}, {:.1})", world_pos.x, world_pos.y);
                self.clone_source = Some(world_pos);
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
                        self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::Normal, shared);
                    } else {
                        self.handle_draw_tool(ui, &response, world_pos, shared);
                    }
                }
                Tool::Pencil | Tool::Pen | Tool::Airbrush => {
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::Normal, shared);
                }
                Tool::Erase => {
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::Erase, shared);
                }
                Tool::Smudge => {
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::Smudge, shared);
                }
                Tool::CloneStamp => {
                    // Alt+click (source-setting) is handled before this block.
                    // Here alt_held is always false, so just paint.
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::CloneStamp, shared);
                }
                Tool::HealingBrush => {
                    // Alt+click (source-setting) is handled before this block.
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::Healing, shared);
                }
                Tool::PatternStamp => {
                    self.handle_raster_stroke_tool(ui, &response, world_pos, lightningbeam_core::raster_layer::RasterBlendMode::PatternStamp, shared);
                }
                Tool::SelectLasso => {
                    self.handle_raster_lasso_tool(ui, &response, world_pos, shared);
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
        let (a_world, b_world, dab_angle_rad) = match *shared.selected_tool {
            Tool::Erase => (*shared.eraser_radius, *shared.eraser_radius, 0.0_f32),
            Tool::Smudge
            | Tool::BlurSharpen
            | Tool::DodgeBurn
            | Tool::Sponge => (*shared.smudge_radius, *shared.smudge_radius, 0.0_f32),
            _ => {
                let bs = &shared.active_brush_settings;
                let r = *shared.brush_radius;
                let ratio = bs.elliptical_dab_ratio.max(1.0);
                // Expand radius to cover the full jitter extent.
                let expand = 1.0 + bs.offset_by_random;
                (r * expand, r * expand / ratio, bs.elliptical_dab_angle.to_radians())
            }
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
                            float.pixels = pixels;
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
            instance_id_for_readback: self.instance_id,
            painting_canvas: self.painting_canvas,
            pending_canvas_removal: self.pending_canvas_removal.take(),
            painting_float: self.painting_float,
            brush_preview_pixels: shared.brush_preview_pixels.clone(),
        }};

        let cb = egui_wgpu::Callback::new_paint_callback(
            rect,
            callback,
        );

        ui.painter().add(cb);

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
        if matches!(*shared.selected_tool, lightningbeam_core::tool::Tool::CloneStamp | lightningbeam_core::tool::Tool::HealingBrush) {
            if let Some(src_world) = self.clone_source {
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
                ) && shared.active_layer_id.and_then(|id| {
                    shared.action_executor.document().get_layer(&id)
                }).map_or(false, |l| matches!(l, lightningbeam_core::layer::AnyLayer::Raster(_)));

                if is_raster_paint {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                    self.draw_brush_cursor(ui, rect, pos, shared);
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
