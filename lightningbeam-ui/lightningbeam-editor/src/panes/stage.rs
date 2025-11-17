/// Stage pane - main animation canvas with Vello rendering
///
/// Renders composited layers using Vello GPU renderer via egui callbacks.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};
use std::sync::{Arc, Mutex};

/// Shared Vello resources (created once, reused by all Stage panes)
struct SharedVelloResources {
    renderer: Arc<Mutex<vello::Renderer>>,
    blit_pipeline: wgpu::RenderPipeline,
    blit_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Per-instance Vello resources (created for each Stage pane)
struct InstanceVelloResources {
    texture: Option<wgpu::Texture>,
    texture_view: Option<wgpu::TextureView>,
    blit_bind_group: Option<wgpu::BindGroup>,
}

/// Container for all Vello instances, stored in egui's CallbackResources
pub struct VelloResourcesMap {
    shared: Option<Arc<SharedVelloResources>>,
    instances: std::collections::HashMap<u64, InstanceVelloResources>,
}

impl SharedVelloResources {
    pub fn new(device: &wgpu::Device) -> Result<Self, String> {
        let renderer = vello::Renderer::new(
            device,
            vello::RendererOptions {
                surface_format: None,
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: std::num::NonZeroUsize::new(1),
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
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm, // egui's target format
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

        println!("✅ Vello shared resources initialized (renderer and shaders)");

        Ok(Self {
            renderer: Arc::new(Mutex::new(renderer)),
            blit_pipeline,
            blit_bind_group_layout,
            sampler,
        })
    }
}

impl InstanceVelloResources {
    pub fn new() -> Self {
        Self {
            texture: None,
            texture_view: None,
            blit_bind_group: None,
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
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
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
}

/// Callback for Vello rendering within egui
struct VelloCallback {
    rect: egui::Rect,
    pan_offset: egui::Vec2,
    zoom: f32,
    instance_id: u64,
    document: lightningbeam_core::document::Document,
}

impl VelloCallback {
    fn new(rect: egui::Rect, pan_offset: egui::Vec2, zoom: f32, instance_id: u64, document: lightningbeam_core::document::Document) -> Self {
        Self { rect, pan_offset, zoom, instance_id, document }
    }
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
                SharedVelloResources::new(device).expect("Failed to initialize shared Vello resources")
            ));
        }

        let shared = map.shared.as_ref().unwrap().clone();

        // Get or create per-instance resources
        let instance_resources = map.instances.entry(self.instance_id).or_insert_with(|| {
            println!("✅ Creating instance resources for Stage pane #{}", self.instance_id);
            InstanceVelloResources::new()
        });

        // Ensure texture is the right size
        let width = self.rect.width() as u32;
        let height = self.rect.height() as u32;

        if width == 0 || height == 0 {
            return Vec::new();
        }

        instance_resources.ensure_texture(device, &shared, width, height);

        // Build Vello scene using the document renderer
        let mut scene = vello::Scene::new();

        // Build camera transform: translate for pan, scale for zoom
        use vello::kurbo::Affine;
        let camera_transform = Affine::translate((self.pan_offset.x as f64, self.pan_offset.y as f64))
            * Affine::scale(self.zoom as f64);

        // Render the document to the scene with camera transform
        lightningbeam_core::renderer::render_document_with_transform(&self.document, &mut scene, camera_transform);

        // Render scene to texture using shared renderer
        if let Some(texture_view) = &instance_resources.texture_view {
            let render_params = vello::RenderParams {
                base_color: vello::peniko::Color::rgb8(45, 45, 48), // Dark background
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
        let instance_resources = match map.instances.get(&self.instance_id) {
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
    // Interaction state
    is_panning: bool,
    last_pan_pos: Option<egui::Pos2>,
    // Unique ID for this stage instance (for Vello resources)
    instance_id: u64,
}

// Global counter for generating unique instance IDs
static INSTANCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl StagePane {
    pub fn new() -> Self {
        let instance_id = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            pan_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            is_panning: false,
            last_pan_pos: None,
            instance_id,
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

    fn handle_input(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Only process input if mouse is over the stage pane
        if !response.hovered() {
            self.is_panning = false;
            self.last_pan_pos = None;
            return;
        }

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let alt_held = ui.input(|i| i.modifiers.alt);
        let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);

        // Get mouse position for zoom-to-cursor
        let mouse_pos = response.hover_pos().unwrap_or(rect.center());
        let mouse_canvas_pos = mouse_pos - rect.min;

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

impl PaneRenderer for StagePane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Handle input for pan/zoom controls
        self.handle_input(ui, rect);

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

        // Use egui's custom painting callback for Vello
        let callback = VelloCallback::new(rect, self.pan_offset, self.zoom, self.instance_id, shared.document.clone());

        let cb = egui_wgpu::Callback::new_paint_callback(
            rect,
            callback,
        );

        ui.painter().add(cb);

        // Show camera info overlay
        ui.painter().text(
            rect.min + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            format!("Vello Stage (zoom: {:.2}, pan: {:.0},{:.0})",
                self.zoom, self.pan_offset.x, self.pan_offset.y),
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(200),
        );
    }

    fn name(&self) -> &str {
        "Stage"
    }
}
