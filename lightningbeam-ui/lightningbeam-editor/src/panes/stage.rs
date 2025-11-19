/// Stage pane - main animation canvas with Vello rendering
///
/// Renders composited layers using Vello GPU renderer via egui callbacks.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};
use std::sync::{Arc, Mutex, OnceLock};
use vello::kurbo::Shape;

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
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
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
    tool_state: lightningbeam_core::tool::ToolState,
    active_layer_id: Option<uuid::Uuid>,
    drag_delta: Option<vello::kurbo::Vec2>, // Delta for drag preview (world space)
    selection: lightningbeam_core::selection::Selection,
    fill_color: egui::Color32, // Current fill color for previews
    stroke_color: egui::Color32, // Current stroke color for previews
    selected_tool: lightningbeam_core::tool::Tool, // Current tool for rendering mode-specific UI
    eyedropper_request: Option<(egui::Pos2, super::ColorMode)>, // Pending eyedropper sample
}

impl VelloCallback {
    fn new(
        rect: egui::Rect,
        pan_offset: egui::Vec2,
        zoom: f32,
        instance_id: u64,
        document: lightningbeam_core::document::Document,
        tool_state: lightningbeam_core::tool::ToolState,
        active_layer_id: Option<uuid::Uuid>,
        drag_delta: Option<vello::kurbo::Vec2>,
        selection: lightningbeam_core::selection::Selection,
        fill_color: egui::Color32,
        stroke_color: egui::Color32,
        selected_tool: lightningbeam_core::tool::Tool,
        eyedropper_request: Option<(egui::Pos2, super::ColorMode)>,
    ) -> Self {
        Self { rect, pan_offset, zoom, instance_id, document, tool_state, active_layer_id, drag_delta, selection, fill_color, stroke_color, selected_tool, eyedropper_request }
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

        // Render drag preview objects with transparency
        if let (Some(delta), Some(active_layer_id)) = (self.drag_delta, self.active_layer_id) {
            if let Some(layer) = self.document.get_layer(&active_layer_id) {
                if let lightningbeam_core::layer::AnyLayer::Vector(vector_layer) = layer {
                    if let lightningbeam_core::tool::ToolState::DraggingSelection { ref original_positions, .. } = self.tool_state {
                        use vello::peniko::{Color, Fill, Brush};

                        // Render each object at its preview position (original + delta)
                        for (object_id, original_pos) in original_positions {
                            if let Some(_object) = vector_layer.get_object(object_id) {
                                if let Some(shape) = vector_layer.get_shape(&_object.shape_id) {
                                    // New position = original + delta
                                    let new_x = original_pos.x + delta.x;
                                    let new_y = original_pos.y + delta.y;

                                    // Build transform for preview position
                                    let object_transform = Affine::translate((new_x, new_y));
                                    let combined_transform = camera_transform * object_transform;

                                    // Render shape with semi-transparent fill (light blue, 40% opacity)
                                    let alpha_color = Color::rgba8(100, 150, 255, 100);
                                    scene.fill(
                                        Fill::NonZero,
                                        combined_transform,
                                        &Brush::Solid(alpha_color),
                                        None,
                                        shape.path(),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // Render selection overlays (outlines, handles, marquee)
        if let Some(active_layer_id) = self.active_layer_id {
            if let Some(layer) = self.document.get_layer(&active_layer_id) {
                if let lightningbeam_core::layer::AnyLayer::Vector(vector_layer) = layer {
                    use vello::peniko::{Color, Fill};
                    use vello::kurbo::{Circle, Rect as KurboRect, Shape as KurboShape, Stroke};

                    let selection_color = Color::rgb8(0, 120, 255); // Blue
                    let stroke_width = 2.0 / self.zoom.max(0.5) as f64;

                    // 1. Draw selection outlines around selected objects
                    // NOTE: Skip this if Transform tool is active (it has its own handles)
                    if !self.selection.is_empty() && !matches!(self.selected_tool, Tool::Transform) {
                        for &object_id in self.selection.objects() {
                            if let Some(object) = vector_layer.get_object(&object_id) {
                                if let Some(shape) = vector_layer.get_shape(&object.shape_id) {
                                    // Get shape bounding box
                                    let bbox = shape.path().bounding_box();

                                    // Apply object transform and camera transform
                                    let object_transform = Affine::translate((object.transform.x, object.transform.y));
                                    let combined_transform = camera_transform * object_transform;

                                    // Create selection rectangle
                                    let selection_rect = KurboRect::new(bbox.x0, bbox.y0, bbox.x1, bbox.y1);

                                    // Draw selection outline
                                    scene.stroke(
                                        &Stroke::new(stroke_width),
                                        combined_transform,
                                        selection_color,
                                        None,
                                        &selection_rect,
                                    );

                                    // Draw corner handles (4 circles at corners)
                                    let handle_radius = (6.0 / self.zoom.max(0.5) as f64).max(4.0);
                                    let corners = [
                                        (bbox.x0, bbox.y0),
                                        (bbox.x1, bbox.y0),
                                        (bbox.x1, bbox.y1),
                                        (bbox.x0, bbox.y1),
                                    ];

                                    for (x, y) in corners {
                                        let corner_circle = Circle::new((x, y), handle_radius);
                                        // Fill with blue
                                        scene.fill(
                                            Fill::NonZero,
                                            combined_transform,
                                            selection_color,
                                            None,
                                            &corner_circle,
                                        );
                                        // White outline
                                        scene.stroke(
                                            &Stroke::new(1.0),
                                            combined_transform,
                                            Color::rgb8(255, 255, 255),
                                            None,
                                            &corner_circle,
                                        );
                                    }
                                }
                            }
                        }
                    }

                    // 2. Draw marquee selection rectangle
                    if let lightningbeam_core::tool::ToolState::MarqueeSelecting { ref start, ref current } = self.tool_state {
                        let marquee_rect = KurboRect::new(
                            start.x.min(current.x),
                            start.y.min(current.y),
                            start.x.max(current.x),
                            start.y.max(current.y),
                        );

                        // Semi-transparent fill
                        let marquee_fill = Color::rgba8(0, 120, 255, 100);
                        scene.fill(
                            Fill::NonZero,
                            camera_transform,
                            marquee_fill,
                            None,
                            &marquee_rect,
                        );

                        // Border stroke
                        scene.stroke(
                            &Stroke::new(1.0),
                            camera_transform,
                            selection_color,
                            None,
                            &marquee_rect,
                        );
                    }

                    // 3. Draw rectangle creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingRectangle { ref start_point, ref current_point, centered, constrain_square, .. } = self.tool_state {
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
                            let preview_transform = camera_transform * Affine::translate((position.x, position.y));

                            // Use actual fill color (same as final shape)
                            let fill_color = Color::rgba8(
                                self.fill_color.r(),
                                self.fill_color.g(),
                                self.fill_color.b(),
                                self.fill_color.a(),
                            );
                            scene.fill(
                                Fill::NonZero,
                                preview_transform,
                                fill_color,
                                None,
                                &rect,
                            );
                        }
                    }

                    // 4. Draw ellipse creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingEllipse { ref start_point, ref current_point, corner_mode, constrain_circle, .. } = self.tool_state {
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
                            let preview_transform = camera_transform * Affine::translate((position.x, position.y));

                            // Use actual fill color (same as final shape)
                            let fill_color = Color::rgba8(
                                self.fill_color.r(),
                                self.fill_color.g(),
                                self.fill_color.b(),
                                self.fill_color.a(),
                            );

                            // Render circle or ellipse directly (can't use Box<dyn> due to trait constraints)
                            if rx == ry {
                                // Circle
                                let circle = KurboCircle::new((0.0, 0.0), rx);
                                scene.fill(
                                    Fill::NonZero,
                                    preview_transform,
                                    fill_color,
                                    None,
                                    &circle,
                                );
                            } else {
                                // Ellipse
                                let ellipse = Ellipse::new((0.0, 0.0), (rx, ry), 0.0);
                                scene.fill(
                                    Fill::NonZero,
                                    preview_transform,
                                    fill_color,
                                    None,
                                    &ellipse,
                                );
                            }
                        }
                    }

                    // 5. Draw line creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingLine { ref start_point, ref current_point, .. } = self.tool_state {
                        use vello::kurbo::Line;

                        // Calculate line length
                        let dx = current_point.x - start_point.x;
                        let dy = current_point.y - start_point.y;
                        let length = (dx * dx + dy * dy).sqrt();

                        if length > 0.0 {
                            // Use actual stroke color for line preview
                            let stroke_color = Color::rgba8(
                                self.stroke_color.r(),
                                self.stroke_color.g(),
                                self.stroke_color.b(),
                                self.stroke_color.a(),
                            );

                            // Draw the line directly
                            let line = Line::new(*start_point, *current_point);
                            scene.stroke(
                                &Stroke::new(2.0),
                                camera_transform,
                                stroke_color,
                                None,
                                &line,
                            );
                        }
                    }

                    // 6. Draw polygon creation preview
                    if let lightningbeam_core::tool::ToolState::CreatingPolygon { ref center, ref current_point, num_sides, .. } = self.tool_state {
                        use vello::kurbo::{BezPath, Point};
                        use std::f64::consts::PI;

                        // Calculate radius
                        let dx = current_point.x - center.x;
                        let dy = current_point.y - center.y;
                        let radius = (dx * dx + dy * dy).sqrt();

                        if radius > 5.0 && num_sides >= 3 {
                            let preview_transform = camera_transform * Affine::translate((center.x, center.y));

                            // Use actual fill color (same as final shape)
                            let fill_color = Color::rgba8(
                                self.fill_color.r(),
                                self.fill_color.g(),
                                self.fill_color.b(),
                                self.fill_color.a(),
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

                            scene.fill(
                                Fill::NonZero,
                                preview_transform,
                                fill_color,
                                None,
                                &path,
                            );
                        }
                    }

                    // 7. Draw path drawing preview
                    if let lightningbeam_core::tool::ToolState::DrawingPath { ref points, .. } = self.tool_state {
                        use vello::kurbo::{BezPath, Point};

                        if points.len() >= 2 {
                            // Build a simple line path from the raw points for preview
                            let mut preview_path = BezPath::new();
                            preview_path.move_to(points[0]);
                            for point in &points[1..] {
                                preview_path.line_to(*point);
                            }

                            // Draw the preview path with stroke
                            let stroke_width = (2.0 / self.zoom.max(0.5) as f64).max(1.0);
                            let stroke_color = Color::rgb8(
                                self.stroke_color.r(),
                                self.stroke_color.g(),
                                self.stroke_color.b(),
                            );

                            scene.stroke(
                                &Stroke::new(stroke_width),
                                camera_transform,
                                stroke_color,
                                None,
                                &preview_path,
                            );
                        }
                    }

                    // 6. Draw transform tool handles (when Transform tool is active)
                    use lightningbeam_core::tool::Tool;
                    if matches!(self.selected_tool, Tool::Transform) && !self.selection.is_empty() {
                        // For single object: use object-aligned (rotated) bounding box
                        // For multiple objects: use axis-aligned bounding box (simpler for now)

                        if self.selection.objects().len() == 1 {
                            // Single object - draw rotated bounding box
                            let object_id = *self.selection.objects().iter().next().unwrap();

                            if let Some(object) = vector_layer.get_object(&object_id) {
                                if let Some(shape) = vector_layer.get_shape(&object.shape_id) {
                                    let handle_size = (8.0 / self.zoom.max(0.5) as f64).max(6.0);
                                    let handle_color = Color::rgb8(0, 120, 255); // Blue
                                    let rotation_handle_offset = 20.0 / self.zoom.max(0.5) as f64;

                                    // Get shape's local bounding box
                                    let local_bbox = shape.path().bounding_box();

                                    // Calculate the 4 corners in local space
                                    let local_corners = [
                                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y0), // Top-left
                                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y0), // Top-right
                                        vello::kurbo::Point::new(local_bbox.x1, local_bbox.y1), // Bottom-right
                                        vello::kurbo::Point::new(local_bbox.x0, local_bbox.y1), // Bottom-left
                                    ];

                                    // Build skew transforms around shape center
                                    let center_x = (local_bbox.x0 + local_bbox.x1) / 2.0;
                                    let center_y = (local_bbox.y0 + local_bbox.y1) / 2.0;

                                    let skew_transform = if object.transform.skew_x != 0.0 || object.transform.skew_y != 0.0 {
                                        let skew_x_affine = if object.transform.skew_x != 0.0 {
                                            let tan_skew = object.transform.skew_x.to_radians().tan();
                                            Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
                                        } else {
                                            Affine::IDENTITY
                                        };

                                        let skew_y_affine = if object.transform.skew_y != 0.0 {
                                            let tan_skew = object.transform.skew_y.to_radians().tan();
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
                                    let obj_transform = Affine::translate((object.transform.x, object.transform.y))
                                        * Affine::rotate(object.transform.rotation.to_radians())
                                        * Affine::scale_non_uniform(object.transform.scale_x, object.transform.scale_y)
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
                                        camera_transform,
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
                                            camera_transform,
                                            handle_color,
                                            None,
                                            &handle_rect,
                                        );

                                        // White outline
                                        scene.stroke(
                                            &Stroke::new(1.0),
                                            camera_transform,
                                            Color::rgb8(255, 255, 255),
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
                                            camera_transform,
                                            handle_color,
                                            None,
                                            &edge_circle,
                                        );

                                        // White outline
                                        scene.stroke(
                                            &Stroke::new(1.0),
                                            camera_transform,
                                            Color::rgb8(255, 255, 255),
                                            None,
                                            &edge_circle,
                                        );
                                    }

                                    // Draw rotation handle (circle above top edge center)
                                    let top_center = edge_midpoints[0];
                                    // Calculate offset vector in object's rotated coordinate space
                                    let rotation_rad = object.transform.rotation.to_radians();
                                    let cos_r = rotation_rad.cos();
                                    let sin_r = rotation_rad.sin();
                                    // Rotate the offset vector (0, -offset) by the object's rotation
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
                                        camera_transform,
                                        Color::rgb8(50, 200, 50),
                                        None,
                                        &rotation_circle,
                                    );

                                    // White outline
                                    scene.stroke(
                                        &Stroke::new(1.0),
                                        camera_transform,
                                        Color::rgb8(255, 255, 255),
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
                                        camera_transform,
                                        Color::rgb8(50, 200, 50),
                                        None,
                                        &line_path,
                                    );
                                }
                            }
                        } else {
                            // Multiple objects - use axis-aligned bbox (existing code)
                            let mut combined_bbox: Option<KurboRect> = None;

                            for &object_id in self.selection.objects() {
                                if let Some(object) = vector_layer.get_object(&object_id) {
                                    if let Some(shape) = vector_layer.get_shape(&object.shape_id) {
                                        let shape_bbox = shape.path().bounding_box();
                                        let transform = Affine::translate((object.transform.x, object.transform.y))
                                            * Affine::rotate(object.transform.rotation.to_radians())
                                            * Affine::scale_non_uniform(object.transform.scale_x, object.transform.scale_y);
                                        let transformed_bbox = transform.transform_rect_bbox(shape_bbox);

                                        combined_bbox = Some(match combined_bbox {
                                            None => transformed_bbox,
                                            Some(existing) => existing.union(transformed_bbox),
                                        });
                                    }
                                }
                            }

                            if let Some(bbox) = combined_bbox {
                                let handle_size = (8.0 / self.zoom.max(0.5) as f64).max(6.0);
                                let handle_color = Color::rgb8(0, 120, 255);
                                let rotation_handle_offset = 20.0 / self.zoom.max(0.5) as f64;

                                scene.stroke(&Stroke::new(stroke_width), camera_transform, handle_color, None, &bbox);

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
                                    scene.fill(Fill::NonZero, camera_transform, handle_color, None, &handle_rect);
                                    scene.stroke(&Stroke::new(1.0), camera_transform, Color::rgb8(255, 255, 255), None, &handle_rect);
                                }

                                let edges = [
                                    vello::kurbo::Point::new(bbox.center().x, bbox.y0),
                                    vello::kurbo::Point::new(bbox.x1, bbox.center().y),
                                    vello::kurbo::Point::new(bbox.center().x, bbox.y1),
                                    vello::kurbo::Point::new(bbox.x0, bbox.center().y),
                                ];

                                for edge in &edges {
                                    let edge_circle = Circle::new(*edge, handle_size / 2.0);
                                    scene.fill(Fill::NonZero, camera_transform, handle_color, None, &edge_circle);
                                    scene.stroke(&Stroke::new(1.0), camera_transform, Color::rgb8(255, 255, 255), None, &edge_circle);
                                }

                                let rotation_handle_pos = vello::kurbo::Point::new(bbox.center().x, bbox.y0 - rotation_handle_offset);
                                let rotation_circle = Circle::new(rotation_handle_pos, handle_size / 2.0);
                                scene.fill(Fill::NonZero, camera_transform, Color::rgb8(50, 200, 50), None, &rotation_circle);
                                scene.stroke(&Stroke::new(1.0), camera_transform, Color::rgb8(255, 255, 255), None, &rotation_circle);

                                let line_path = {
                                    let mut path = vello::kurbo::BezPath::new();
                                    path.move_to(rotation_handle_pos);
                                    path.line_to(vello::kurbo::Point::new(bbox.center().x, bbox.y0));
                                    path
                                };
                                scene.stroke(&Stroke::new(1.0), camera_transform, Color::rgb8(50, 200, 50), None, &line_path);
                            }
                        }
                    }
                }
            }
        }

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

        // Handle eyedropper pixel sampling if requested
        if let Some((screen_pos, color_mode)) = self.eyedropper_request {
            if let Some(texture) = &instance_resources.texture {
                // Convert screen position to texture coordinates
                let tex_x = ((screen_pos.x - self.rect.min.x).max(0.0).min(self.rect.width())) as u32;
                let tex_y = ((screen_pos.y - self.rect.min.y).max(0.0).min(self.rect.height())) as u32;

                // Clamp to texture bounds
                if tex_x < width && tex_y < height {
                    // Create a staging buffer to read back the pixel
                    let bytes_per_pixel = 4; // RGBA8
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
                        wgpu::ImageCopyTexture {
                            texture,
                            mip_level: 0,
                            origin: wgpu::Origin3d { x: tex_x, y: tex_y, z: 0 },
                            aspect: wgpu::TextureAspect::All,
                        },
                        wgpu::ImageCopyBuffer {
                            buffer: &staging_buffer,
                            layout: wgpu::ImageDataLayout {
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
                    device.poll(wgpu::Maintain::Wait);

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
                                results.insert(self.instance_id, (sampled_color, color_mode));
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
    // Eyedropper state
    pending_eyedropper_sample: Option<(egui::Pos2, super::ColorMode)>,
}

// Global counter for generating unique instance IDs
static INSTANCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// Global storage for eyedropper results (instance_id -> (color, color_mode))
static EYEDROPPER_RESULTS: OnceLock<Arc<Mutex<std::collections::HashMap<u64, (egui::Color32, super::ColorMode)>>>> = OnceLock::new();

impl StagePane {
    pub fn new() -> Self {
        let instance_id = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            pan_offset: egui::Vec2::ZERO,
            zoom: 1.0,
            is_panning: false,
            last_pan_pos: None,
            instance_id,
            pending_eyedropper_sample: None,
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
        use lightningbeam_core::hit_test;
        use vello::kurbo::{Point, Rect as KurboRect, Affine};

        // Check if we have an active vector layer
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return, // No active layer
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        let vector_layer = match active_layer {
            AnyLayer::Vector(vl) => vl,
            _ => return, // Not a vector layer
        };

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start interaction (use drag_started for immediate feedback)
        if response.drag_started() || response.clicked() {
            // Hit test at click position
            let hit = hit_test::hit_test_layer(vector_layer, point, 5.0, Affine::IDENTITY);

            if let Some(object_id) = hit {
                // Object was hit
                if shift_held {
                    // Shift: toggle selection
                    shared.selection.toggle_object(object_id);
                } else {
                    // No shift: replace selection
                    if !shared.selection.contains_object(&object_id) {
                        shared.selection.select_only_object(object_id);
                    }
                }

                // If object is now selected, prepare for dragging
                if shared.selection.contains_object(&object_id) {
                    // Store original positions of all selected objects
                    let mut original_positions = std::collections::HashMap::new();
                    for &obj_id in shared.selection.objects() {
                        if let Some(obj) = vector_layer.get_object(&obj_id) {
                            original_positions.insert(
                                obj_id,
                                Point::new(obj.transform.x, obj.transform.y),
                            );
                        }
                    }

                    *shared.tool_state = ToolState::DraggingSelection {
                        start_pos: point,
                        start_mouse: point,
                        original_positions,
                    };
                }
            } else {
                // Nothing hit - start marquee selection
                if !shift_held {
                    shared.selection.clear();
                }

                *shared.tool_state = ToolState::MarqueeSelecting {
                    start: point,
                    current: point,
                };
            }
        }

        // Mouse drag: update tool state
        if response.dragged() {
            match shared.tool_state {
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
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::DraggingSelection { .. } | ToolState::MarqueeSelecting { .. })) {
            match shared.tool_state.clone() {
                ToolState::DraggingSelection { start_mouse, original_positions, .. } => {
                    // Calculate total delta
                    let delta = point - start_mouse;

                    if delta.x.abs() > 0.01 || delta.y.abs() > 0.01 {
                        // Create move action with new positions
                        use std::collections::HashMap;
                        let mut object_positions = HashMap::new();

                        for (object_id, original_pos) in original_positions {
                            let new_pos = Point::new(
                                original_pos.x + delta.x,
                                original_pos.y + delta.y,
                            );
                            object_positions.insert(object_id, (original_pos, new_pos));
                        }

                        // Create and submit the action
                        use lightningbeam_core::actions::MoveObjectsAction;
                        let action = MoveObjectsAction::new(*active_layer_id, object_positions);
                        shared.pending_actions.push(Box::new(action));
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

                    // Hit test all objects in rectangle
                    let hits = hit_test::hit_test_objects_in_rect(
                        vector_layer,
                        selection_rect,
                        Affine::IDENTITY,
                    );

                    // Add to selection
                    for obj_id in hits {
                        if shift_held {
                            shared.selection.add_object(obj_id);
                        } else {
                            // First hit replaces selection
                            if shared.selection.is_empty() {
                                shared.selection.add_object(obj_id);
                            } else {
                                // Subsequent hits add to selection
                                shared.selection.add_object(obj_id);
                            }
                        }
                    }

                    // Reset tool state
                    *shared.tool_state = ToolState::Idle;
                }
                _ => {}
            }
        }
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
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start creating rectangle (clears any previous preview)
        if response.drag_started() || response.clicked() {
            *shared.tool_state = ToolState::CreatingRectangle {
                start_point: point,
                current_point: point,
                centered: ctrl_held,
                constrain_square: shift_held,
            };
        }

        // Mouse drag: update rectangle
        if response.dragged() {
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
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::CreatingRectangle { .. })) {
            if let ToolState::CreatingRectangle { start_point, current_point, centered, constrain_square } = shared.tool_state.clone() {
                // Calculate rectangle bounds based on mode
                let (width, height, position) = if centered {
                    // Centered mode: start_point is center
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
                    // Corner mode: start_point is corner
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

                // Only create shape if rectangle has non-zero size
                if width > 1.0 && height > 1.0 {
                    use lightningbeam_core::shape::{Shape, ShapeColor};
                    use lightningbeam_core::object::Object;
                    use lightningbeam_core::actions::AddShapeAction;

                    // Create shape with rectangle path (built from lines)
                    let path = Self::create_rectangle_path(width, height);
                    let shape = Shape::new(path).with_fill(ShapeColor::from_egui(*shared.fill_color));

                    // Create object at the calculated position
                    let object = Object::new(shape.id).with_position(position.x, position.y);

                    // Create and execute action immediately
                    let action = AddShapeAction::new(*active_layer_id, shape, object);
                    shared.action_executor.execute(Box::new(action));

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
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start creating ellipse (clears any previous preview)
        if response.drag_started() || response.clicked() {
            *shared.tool_state = ToolState::CreatingEllipse {
                start_point: point,
                current_point: point,
                corner_mode: !ctrl_held,  // Inverted: Ctrl = centered (like rectangle)
                constrain_circle: shift_held,
            };
        }

        // Mouse drag: update ellipse
        if response.dragged() {
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
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::CreatingEllipse { .. })) {
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
                    use lightningbeam_core::shape::{Shape, ShapeColor};
                    use lightningbeam_core::object::Object;
                    use lightningbeam_core::actions::AddShapeAction;

                    // Create shape with ellipse path (built from bezier curves)
                    let path = Self::create_ellipse_path(rx, ry);
                    let shape = Shape::new(path).with_fill(ShapeColor::from_egui(*shared.fill_color));

                    // Create object at the calculated position
                    let object = Object::new(shape.id).with_position(position.x, position.y);

                    // Create and execute action immediately
                    let action = AddShapeAction::new(*active_layer_id, shape, object);
                    shared.action_executor.execute(Box::new(action));

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
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start creating line
        if response.drag_started() || response.clicked() {
            *shared.tool_state = ToolState::CreatingLine {
                start_point: point,
                current_point: point,
            };
        }

        // Mouse drag: update line
        if response.dragged() {
            if let ToolState::CreatingLine { start_point, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingLine {
                    start_point: *start_point,
                    current_point: point,
                };
            }
        }

        // Mouse up: create the line shape
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::CreatingLine { .. })) {
            if let ToolState::CreatingLine { start_point, current_point } = shared.tool_state.clone() {
                // Calculate line length to ensure it's not too small
                let dx = current_point.x - start_point.x;
                let dy = current_point.y - start_point.y;
                let length = (dx * dx + dy * dy).sqrt();

                // Only create shape if line has reasonable length
                if length > 1.0 {
                    use lightningbeam_core::shape::{Shape, ShapeColor, StrokeStyle};
                    use lightningbeam_core::object::Object;
                    use lightningbeam_core::actions::AddShapeAction;

                    // Create shape with line path
                    let path = Self::create_line_path(dx, dy);

                    // Lines should have stroke by default, not fill
                    let shape = Shape::new(path)
                        .with_stroke(
                            ShapeColor::from_egui(*shared.stroke_color),
                            StrokeStyle {
                                width: 2.0,
                                ..Default::default()
                            }
                        );

                    // Create object at the start point
                    let object = Object::new(shape.id).with_position(start_point.x, start_point.y);

                    // Create and execute action immediately
                    let action = AddShapeAction::new(*active_layer_id, shape, object);
                    shared.action_executor.execute(Box::new(action));

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
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start creating polygon (center point)
        if response.drag_started() || response.clicked() {
            *shared.tool_state = ToolState::CreatingPolygon {
                center: point,
                current_point: point,
                num_sides: 5,  // Default to 5 sides (pentagon)
            };
        }

        // Mouse drag: update polygon radius
        if response.dragged() {
            if let ToolState::CreatingPolygon { center, num_sides, .. } = shared.tool_state {
                *shared.tool_state = ToolState::CreatingPolygon {
                    center: *center,
                    current_point: point,
                    num_sides: *num_sides,
                };
            }
        }

        // Mouse up: create the polygon shape
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::CreatingPolygon { .. })) {
            if let ToolState::CreatingPolygon { center, current_point, num_sides } = shared.tool_state.clone() {
                // Calculate radius
                let dx = current_point.x - center.x;
                let dy = current_point.y - center.y;
                let radius = (dx * dx + dy * dy).sqrt();

                // Only create shape if polygon has reasonable size
                if radius > 5.0 {
                    use lightningbeam_core::shape::{Shape, ShapeColor};
                    use lightningbeam_core::object::Object;
                    use lightningbeam_core::actions::AddShapeAction;

                    // Create shape with polygon path
                    let path = Self::create_polygon_path(num_sides, radius);
                    let shape = Shape::new(path).with_fill(ShapeColor::from_egui(*shared.fill_color));

                    // Create object at the center point
                    let object = Object::new(shape.id).with_position(center.x, center.y);

                    // Create and execute action immediately
                    let action = AddShapeAction::new(*active_layer_id, shape, object);
                    shared.action_executor.execute(Box::new(action));

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
        if response.clicked() {
            self.pending_eyedropper_sample = Some((screen_pos, *shared.active_color_mode));
        }
    }

    /// Create a rectangle path from lines (easier for curve editing later)
    fn create_rectangle_path(width: f64, height: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};

        let mut path = BezPath::new();

        // Start at top-left
        path.move_to(Point::new(0.0, 0.0));

        // Top-right
        path.line_to(Point::new(width, 0.0));

        // Bottom-right
        path.line_to(Point::new(width, height));

        // Bottom-left
        path.line_to(Point::new(0.0, height));

        // Close path (back to top-left)
        path.close_path();

        path
    }

    /// Create an ellipse path from bezier curves (easier for curve editing later)
    /// Uses 4 cubic bezier segments to approximate the ellipse
    fn create_ellipse_path(rx: f64, ry: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};

        // Magic constant for circular arc approximation with cubic beziers
        // k = 4/3 * (sqrt(2) - 1) ≈ 0.5522847498
        const KAPPA: f64 = 0.5522847498;

        let kx = rx * KAPPA;
        let ky = ry * KAPPA;

        let mut path = BezPath::new();

        // Start at right point (rx, 0)
        path.move_to(Point::new(rx, 0.0));

        // Top-right quadrant (to top point)
        path.curve_to(
            Point::new(rx, -ky),      // control point 1
            Point::new(kx, -ry),      // control point 2
            Point::new(0.0, -ry),     // end point (top)
        );

        // Top-left quadrant (to left point)
        path.curve_to(
            Point::new(-kx, -ry),     // control point 1
            Point::new(-rx, -ky),     // control point 2
            Point::new(-rx, 0.0),     // end point (left)
        );

        // Bottom-left quadrant (to bottom point)
        path.curve_to(
            Point::new(-rx, ky),      // control point 1
            Point::new(-kx, ry),      // control point 2
            Point::new(0.0, ry),      // end point (bottom)
        );

        // Bottom-right quadrant (back to right point)
        path.curve_to(
            Point::new(kx, ry),       // control point 1
            Point::new(rx, ky),       // control point 2
            Point::new(rx, 0.0),      // end point (right)
        );

        path.close_path();

        path
    }

    /// Create a line path from start to end point
    fn create_line_path(dx: f64, dy: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};

        let mut path = BezPath::new();

        // Start at origin (object position will be the start point)
        path.move_to(Point::new(0.0, 0.0));

        // Line to end point
        path.line_to(Point::new(dx, dy));

        path
    }

    /// Create a regular polygon path centered at origin
    ///
    /// # Arguments
    /// * `num_sides` - Number of sides for the polygon (must be >= 3)
    /// * `radius` - Radius from center to vertices
    fn create_polygon_path(num_sides: u32, radius: f64) -> vello::kurbo::BezPath {
        use vello::kurbo::{BezPath, Point};
        use std::f64::consts::PI;

        let mut path = BezPath::new();

        if num_sides < 3 {
            return path;
        }

        // Calculate angle between vertices
        let angle_step = 2.0 * PI / num_sides as f64;

        // Start at top (angle = -PI/2 so first vertex is at top)
        let start_angle = -PI / 2.0;

        // First vertex
        let first_x = radius * (start_angle).cos();
        let first_y = radius * (start_angle).sin();
        path.move_to(Point::new(first_x, first_y));

        // Add remaining vertices
        for i in 1..num_sides {
            let angle = start_angle + angle_step * i as f64;
            let x = radius * angle.cos();
            let y = radius * angle.sin();
            path.line_to(Point::new(x, y));
        }

        // Close the path back to first vertex
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
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => return,
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // Mouse down: start drawing path
        if response.drag_started() || response.clicked() {
            *shared.tool_state = ToolState::DrawingPath {
                points: vec![point],
                simplify_mode: *shared.draw_simplify_mode,
            };
        }

        // Mouse drag: add points to path
        if response.dragged() {
            if let ToolState::DrawingPath { points, simplify_mode } = &mut *shared.tool_state {
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

        // Mouse up: complete the path and create shape
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::DrawingPath { .. })) {
            if let ToolState::DrawingPath { points, simplify_mode } = shared.tool_state.clone() {
                // Only create shape if we have enough points
                if points.len() >= 2 {
                    use lightningbeam_core::path_fitting::{
                        simplify_rdp, fit_bezier_curves, RdpConfig, SchneiderConfig,
                    };
                    use lightningbeam_core::shape::{Shape, ShapeColor};
                    use lightningbeam_core::object::Object;
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
                        // Calculate bounding box to position the object
                        let bbox = path.bounding_box();
                        let position = Point::new(bbox.x0, bbox.y0);

                        // Translate path to be relative to position (0,0 at top-left of bbox)
                        use vello::kurbo::Affine;
                        let transform = Affine::translate((-bbox.x0, -bbox.y0));
                        let translated_path = transform * path;

                        // Create shape with both fill and stroke
                        use lightningbeam_core::shape::StrokeStyle;
                        let shape = Shape::new(translated_path)
                            .with_fill(ShapeColor::from_egui(*shared.fill_color))
                            .with_stroke(
                                ShapeColor::from_egui(*shared.stroke_color),
                                StrokeStyle::default(),
                            );

                        // Create object at the calculated position
                        let object = Object::new(shape.id).with_position(position.x, position.y);

                        // Create and execute action immediately
                        let action = AddShapeAction::new(*active_layer_id, shape, object);
                        shared.action_executor.execute(Box::new(action));
                    }
                }

                // Clear tool state to stop preview rendering
                *shared.tool_state = ToolState::Idle;
            }
        }
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
            None => {
                println!("Paint bucket: No active layer");
                return;
            }
        };

        let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
            Some(layer) => layer,
            None => {
                println!("Paint bucket: Layer not found");
                return;
            }
        };

        // Only work on VectorLayer
        if !matches!(active_layer, AnyLayer::Vector(_)) {
            println!("Paint bucket: Not a vector layer");
            return;
        }

        // On click: execute paint bucket fill
        if response.clicked() {
            let click_point = Point::new(world_pos.x as f64, world_pos.y as f64);
            let fill_color = ShapeColor::from_egui(*shared.fill_color);

            println!("Paint bucket clicked at ({:.1}, {:.1})", click_point.x, click_point.y);

            // Create and execute paint bucket action
            let action = PaintBucketAction::new(
                *active_layer_id,
                click_point,
                fill_color,
                2.0, // tolerance - could be made configurable
                lightningbeam_core::gap_handling::GapHandlingMode::BridgeSegment,
            );
            shared.action_executor.execute(Box::new(action));
            println!("Paint bucket action executed");
        }
    }

    /// Apply transform preview to objects based on current mouse position
    fn apply_transform_preview(
        vector_layer: &mut lightningbeam_core::layer::VectorLayer,
        mode: &lightningbeam_core::tool::TransformMode,
        original_transforms: &std::collections::HashMap<uuid::Uuid, lightningbeam_core::object::Transform>,
        pivot: vello::kurbo::Point,
        start_mouse: vello::kurbo::Point,
        current_mouse: vello::kurbo::Point,
        original_bbox: vello::kurbo::Rect,
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

                // Apply scale to all selected objects
                for (object_id, original_transform) in original_transforms {
                    println!("\nObject {:?}:", object_id);
                    println!("  Original pos: ({:.1}, {:.1})", original_transform.x, original_transform.y);
                    println!("  Original rotation: {:.1}°", original_transform.rotation);
                    println!("  Original scale: ({:.3}, {:.3})", original_transform.scale_x, original_transform.scale_y);

                    vector_layer.modify_object_internal(object_id, |obj| {
                        // Get object's rotation in radians
                        let rotation_rad = original_transform.rotation.to_radians();
                        let cos_r = rotation_rad.cos();
                        let sin_r = rotation_rad.sin();

                        // Transform scale from world space to object's local space
                        // The object's local axes are rotated by rotation_rad from world axes
                        // We need to figure out how much to scale along each local axis
                        // to achieve the world-space scaling

                        // For a rotated object, world-space scale affects local-space scale as:
                        // local_x axis aligns with (cos(r), sin(r)) in world space
                        // local_y axis aligns with (-sin(r), cos(r)) in world space
                        // When we scale by (sx, sy) in world, the local scale changes by:
                        let cos_r_sq = cos_r * cos_r;
                        let sin_r_sq = sin_r * sin_r;
                        let sx_abs = scale_x_world.abs();
                        let sy_abs = scale_y_world.abs();

                        // Compute how much the object grows along its local axes
                        // when the world-space bbox is scaled
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
                }
            }

            TransformMode::ScaleEdge { axis, origin } => {
                // Calculate scale along one axis
                let (scale_x_world, scale_y_world) = match axis {
                    Axis::Horizontal => {
                        let start_dist = (start_mouse.x - origin.x).abs();
                        let current_dist = (current_mouse.x - origin.x).abs();
                        let scale = if start_dist > 0.001 {
                            current_dist / start_dist
                        } else {
                            1.0
                        };
                        (scale, 1.0)
                    }
                    Axis::Vertical => {
                        let start_dist = (start_mouse.y - origin.y).abs();
                        let current_dist = (current_mouse.y - origin.y).abs();
                        let scale = if start_dist > 0.001 {
                            current_dist / start_dist
                        } else {
                            1.0
                        };
                        (1.0, scale)
                    }
                };

                // Apply scale to all selected objects
                for (object_id, original_transform) in original_transforms {
                    vector_layer.modify_object_internal(object_id, |obj| {
                        // Get object's rotation in radians
                        let rotation_rad = original_transform.rotation.to_radians();
                        let cos_r = rotation_rad.cos();
                        let sin_r = rotation_rad.sin();

                        // Transform scale from world space to local space (same as corner mode)
                        let cos_r_sq = cos_r * cos_r;
                        let sin_r_sq = sin_r * sin_r;
                        let sx_abs = scale_x_world.abs();
                        let sy_abs = scale_y_world.abs();

                        let local_scale_x = (cos_r_sq * sx_abs * sx_abs + sin_r_sq * sy_abs * sy_abs).sqrt();
                        let local_scale_y = (sin_r_sq * sx_abs * sx_abs + cos_r_sq * sy_abs * sy_abs).sqrt();

                        // Scale position relative to origin in world space
                        let rel_x = original_transform.x - origin.x;
                        let rel_y = original_transform.y - origin.y;

                        obj.transform.x = origin.x + rel_x * scale_x_world;
                        obj.transform.y = origin.y + rel_y * scale_y_world;

                        // Apply local-space scale
                        obj.transform.scale_x = original_transform.scale_x * local_scale_x;
                        obj.transform.scale_y = original_transform.scale_y * local_scale_y;

                        // Keep rotation unchanged
                        obj.transform.rotation = original_transform.rotation;
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
                    // Get the object to find its shape
                    let object = vector_layer.get_object(object_id);

                    // Calculate the world-space center where the renderer applies skew
                    // This is the shape's bounding box center transformed to world space
                    let shape_center_world = if let Some(obj) = object {
                        if let Some(shape) = vector_layer.get_shape(&obj.shape_id) {
                            use kurbo::Shape as KurboShape;
                            let shape_bbox = shape.path().bounding_box();
                            let local_center_x = (shape_bbox.x0 + shape_bbox.x1) / 2.0;
                            let local_center_y = (shape_bbox.y0 + shape_bbox.y1) / 2.0;

                            // Transform to world space (same as renderer)
                            let world_center = kurbo::Affine::translate((original_transform.x, original_transform.y))
                                * kurbo::Affine::rotate(original_transform.rotation.to_radians())
                                * kurbo::Affine::scale_non_uniform(original_transform.scale_x, original_transform.scale_y)
                                * kurbo::Point::new(local_center_x, local_center_y);
                            (world_center.x, world_center.y)
                        } else {
                            // Fallback to object position if shape not found
                            (original_transform.x, original_transform.y)
                        }
                    } else {
                        // Fallback to object position if object not found
                        (original_transform.x, original_transform.y)
                    };

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

        // Check if we have an active vector layer
        let active_layer_id = match shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Only work on VectorLayer - just check type, don't hold reference
        {
            let active_layer = match shared.action_executor.document().get_layer(active_layer_id) {
                Some(layer) => layer,
                None => return,
            };

            if !matches!(active_layer, AnyLayer::Vector(_)) {
                return;
            }
        }

        // Need a selection to transform
        if shared.selection.is_empty() {
            return;
        }

        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

        // For single object: use rotated bounding box
        // For multiple objects: use axis-aligned bounding box
        if shared.selection.objects().len() == 1 {
            // Single object - rotated bounding box
            self.handle_transform_single_object(ui, response, point, active_layer_id, shared);
        } else {
            // Multiple objects - axis-aligned bounding box
            // Calculate combined bounding box for handle hit testing
            let mut combined_bbox: Option<vello::kurbo::Rect> = None;

            // Get immutable reference just for bbox calculation
            if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                for &object_id in shared.selection.objects() {
                    if let Some(object) = vector_layer.get_object(&object_id) {
                        if let Some(shape) = vector_layer.get_shape(&object.shape_id) {
                            // Get shape's local bounding box
                            let shape_bbox = shape.path().bounding_box();

                            // Transform to world space: translate by object position
                            // Then apply scale and rotation around that position
                            use vello::kurbo::Affine;
                            let transform = Affine::translate((object.transform.x, object.transform.y))
                                * Affine::rotate(object.transform.rotation.to_radians())
                                * Affine::scale_non_uniform(object.transform.scale_x, object.transform.scale_y);

                            let transformed_bbox = transform.transform_rect_bbox(shape_bbox);

                            combined_bbox = Some(match combined_bbox {
                                None => transformed_bbox,
                                Some(existing) => existing.union(transformed_bbox),
                            });
                        }
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
            if response.drag_started() || response.clicked() {
                let tolerance = 10.0; // Click tolerance in world space

                if let Some(mode) = Self::hit_test_transform_handle(point, bbox, tolerance) {
                // Store original transforms of all selected objects
                use std::collections::HashMap;
                let mut original_transforms = HashMap::new();

                if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                    for &object_id in shared.selection.objects() {
                        if let Some(object) = vector_layer.get_object(&object_id) {
                            original_transforms.insert(object_id, object.transform.clone());
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
            if response.dragged() {
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
                            );
                        }
                    }
                }
            }

            // Mouse up: finalize transform
            if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::Transforming { .. })) {
                if let ToolState::Transforming { original_transforms, .. } = shared.tool_state.clone() {
                    use std::collections::HashMap;
                    use lightningbeam_core::actions::TransformObjectsAction;

                    let mut object_transforms = HashMap::new();

                    // Get current transforms and pair with originals
                    if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                        for (object_id, original) in original_transforms {
                            if let Some(object) = vector_layer.get_object(&object_id) {
                                let new_transform = object.transform.clone();
                                object_transforms.insert(object_id, (original, new_transform));
                            }
                        }
                    }

                    if !object_transforms.is_empty() {
                        let action = TransformObjectsAction::new(*active_layer_id, object_transforms);
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

        let object_id = *shared.selection.objects().iter().next().unwrap();

        // Calculate rotated bounding box corners
        let (local_bbox, world_corners, obj_transform, object) = {
            if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                if let Some(object) = vector_layer.get_object(&object_id) {
                    if let Some(shape) = vector_layer.get_shape(&object.shape_id) {
                        let local_bbox = shape.path().bounding_box();

                        let local_corners = [
                            vello::kurbo::Point::new(local_bbox.x0, local_bbox.y0),
                            vello::kurbo::Point::new(local_bbox.x1, local_bbox.y0),
                            vello::kurbo::Point::new(local_bbox.x1, local_bbox.y1),
                            vello::kurbo::Point::new(local_bbox.x0, local_bbox.y1),
                        ];

                        // Build skew transforms around shape center
                        let center_x = (local_bbox.x0 + local_bbox.x1) / 2.0;
                        let center_y = (local_bbox.y0 + local_bbox.y1) / 2.0;

                        let skew_transform = if object.transform.skew_x != 0.0 || object.transform.skew_y != 0.0 {
                            let skew_x_affine = if object.transform.skew_x != 0.0 {
                                let tan_skew = object.transform.skew_x.to_radians().tan();
                                Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
                            } else {
                                Affine::IDENTITY
                            };

                            let skew_y_affine = if object.transform.skew_y != 0.0 {
                                let tan_skew = object.transform.skew_y.to_radians().tan();
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

                        let obj_transform = Affine::translate((object.transform.x, object.transform.y))
                            * Affine::rotate(object.transform.rotation.to_radians())
                            * Affine::scale_non_uniform(object.transform.scale_x, object.transform.scale_y)
                            * skew_transform;

                        let world_corners: Vec<vello::kurbo::Point> = local_corners
                            .iter()
                            .map(|&p| obj_transform * p)
                            .collect();

                        (local_bbox, world_corners, obj_transform, object.clone())
                    } else {
                        return;
                    }
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
        let rotation_rad = object.transform.rotation.to_radians();
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
                                    hovering_handle = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        // === Mouse down: hit test handles (using the same handle positions and order as cursor logic) ===
        if response.drag_started() || response.clicked() {
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
                original_transforms.insert(object_id, object.transform.clone());

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
                    original_transforms.insert(object_id, object.transform.clone());

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
                    original_transforms.insert(object_id, object.transform.clone());

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
                                original_transforms.insert(object_id, object.transform.clone());

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
        if response.dragged() {
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
                                    // Get the object and shape's bounding box
                                    if let Some(obj) = vector_layer.get_object(&object_id) {
                                        if let Some(shape) = vector_layer.get_shape(&obj.shape_id) {
                                        use kurbo::Shape as KurboShape;
                                        let shape_bbox = shape.path().bounding_box();

                                        // Transform origin to local space to determine which edge
                                        let original_transform = Affine::translate((original.x, original.y))
                                            * Affine::rotate(original.rotation.to_radians())
                                            * Affine::scale_non_uniform(original.scale_x, original.scale_y);
                                        let inv_original_transform = original_transform.inverse();
                                        let local_origin = inv_original_transform * origin;
                                        let local_current = inv_original_transform * point;

                                        use lightningbeam_core::tool::Axis;
                                        // Calculate skew angle such that edge follows mouse
                                        let skew_radians = match axis {
                                            Axis::Horizontal => {
                                                // Determine which horizontal edge we're dragging
                                                let edge_y = if (local_origin.y - shape_bbox.y0).abs() < 0.1 {
                                                    shape_bbox.y1 // Origin at top, dragging bottom
                                                } else {
                                                    shape_bbox.y0 // Origin at bottom, dragging top
                                                };
                                                let distance = edge_y - local_origin.y;
                                                if distance.abs() > 0.1 {
                                                    let tan_skew = (local_current.x - local_origin.x) / distance;
                                                    tan_skew.atan()
                                                } else {
                                                    0.0
                                                }
                                            }
                                            Axis::Vertical => {
                                                // Determine which vertical edge we're dragging
                                                let edge_x = if (local_origin.x - shape_bbox.x0).abs() < 0.1 {
                                                    shape_bbox.x1 // Origin at left, dragging right
                                                } else {
                                                    shape_bbox.x0 // Origin at right, dragging left
                                                };
                                                let distance = edge_x - local_origin.x;
                                                if distance.abs() > 0.1 {
                                                    let tan_skew = (local_current.y - local_origin.y) / distance;
                                                    tan_skew.atan()
                                                } else {
                                                    0.0
                                                }
                                            }
                                        };
                                        let skew_degrees = skew_radians.to_degrees();

                                        vector_layer.modify_object_internal(&object_id, |obj| {
                                            // Apply skew based on axis
                                            match axis {
                                                Axis::Horizontal => {
                                                    obj.transform.skew_x = original.skew_x + skew_degrees;
                                                }
                                                Axis::Vertical => {
                                                    obj.transform.skew_y = original.skew_y + skew_degrees;
                                                }
                                            }

                                            // Keep other transform properties unchanged
                                            obj.transform.x = original.x;
                                            obj.transform.y = original.y;
                                            obj.transform.rotation = original.rotation;
                                            obj.transform.scale_x = original.scale_x;
                                            obj.transform.scale_y = original.scale_y;
                                        });
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Mouse up: finalize
        if response.drag_stopped() || (ui.input(|i| i.pointer.any_released()) && matches!(shared.tool_state, ToolState::Transforming { .. })) {
            if let ToolState::Transforming { original_transforms, .. } = shared.tool_state.clone() {
                use std::collections::HashMap;
                use lightningbeam_core::actions::TransformObjectsAction;

                let mut object_transforms = HashMap::new();

                if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                    for (obj_id, original) in original_transforms {
                        if let Some(object) = vector_layer.get_object(&obj_id) {
                            object_transforms.insert(obj_id, (original, object.transform.clone()));
                        }
                    }
                }

                if !object_transforms.is_empty() {
                    let action = TransformObjectsAction::new(*active_layer_id, object_transforms);
                    shared.pending_actions.push(Box::new(action));
                }

                *shared.tool_state = ToolState::Idle;
            }
        }
    }

    fn handle_input(&mut self, ui: &mut egui::Ui, rect: egui::Rect, shared: &mut SharedPaneState) {
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Check for mouse release to complete drag operations (even if mouse is offscreen)
        use lightningbeam_core::tool::ToolState;
        use vello::kurbo::Point;

        if ui.input(|i| i.pointer.any_released()) {
            match shared.tool_state.clone() {
                ToolState::DraggingSelection { start_mouse, original_positions, .. } => {
                    // Get last known mouse position (will be at edge if offscreen)
                    if let Some(mouse_pos) = ui.input(|i| i.pointer.latest_pos()) {
                        let mouse_canvas_pos = mouse_pos - rect.min;
                        let world_pos = (mouse_canvas_pos - self.pan_offset) / self.zoom;
                        let point = Point::new(world_pos.x as f64, world_pos.y as f64);

                        let delta = point - start_mouse;

                        if delta.x.abs() > 0.01 || delta.y.abs() > 0.01 {
                            if let Some(active_layer_id) = shared.active_layer_id {
                                use std::collections::HashMap;
                                let mut object_positions = HashMap::new();

                                for (object_id, original_pos) in original_positions {
                                    let new_pos = Point::new(
                                        original_pos.x + delta.x,
                                        original_pos.y + delta.y,
                                    );
                                    object_positions.insert(object_id, (original_pos, new_pos));
                                }

                                use lightningbeam_core::actions::MoveObjectsAction;
                                let action = MoveObjectsAction::new(*active_layer_id, object_positions);
                                shared.pending_actions.push(Box::new(action));
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

                        if let Some(AnyLayer::Vector(vector_layer)) = shared.action_executor.document().get_layer(active_layer_id) {
                            // Create selection rectangle
                            let min_x = start.x.min(current.x);
                            let min_y = start.y.min(current.y);
                            let max_x = start.x.max(current.x);
                            let max_y = start.y.max(current.y);

                            let selection_rect = KurboRect::new(min_x, min_y, max_x, max_y);

                            // Hit test all objects in rectangle
                            let hits = hit_test::hit_test_objects_in_rect(
                                vector_layer,
                                selection_rect,
                                Affine::IDENTITY,
                            );

                            // Add to selection
                            for obj_id in hits {
                                shared.selection.add_object(obj_id);
                            }
                        }
                    }

                    *shared.tool_state = ToolState::Idle;
                }
                _ => {}
            }
        }

        // Only process input if mouse is over the stage pane
        if !response.hovered() {
            self.is_panning = false;
            self.last_pan_pos = None;
            return;
        }

        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        let alt_held = ui.input(|i| i.modifiers.alt);
        let ctrl_held = ui.input(|i| i.modifiers.ctrl || i.modifiers.command);
        let shift_held = ui.input(|i| i.modifiers.shift);

        // Get mouse position for zoom-to-cursor
        let mouse_pos = response.hover_pos().unwrap_or(rect.center());
        let mouse_canvas_pos = mouse_pos - rect.min;

        // Convert screen position to world position (accounting for pan and zoom)
        let world_pos = (mouse_canvas_pos - self.pan_offset) / self.zoom;

        // Handle tool input (only if not using Alt modifier for panning)
        if !alt_held {
            use lightningbeam_core::tool::Tool;

            match *shared.selected_tool {
                Tool::Select => {
                    self.handle_select_tool(ui, &response, world_pos, shift_held, shared);
                }
                Tool::Rectangle => {
                    self.handle_rectangle_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Ellipse => {
                    self.handle_ellipse_tool(ui, &response, world_pos, shift_held, ctrl_held, shared);
                }
                Tool::Draw => {
                    self.handle_draw_tool(ui, &response, world_pos, shared);
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
                _ => {
                    // Other tools not implemented yet
                }
            }
        }

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

        // Calculate drag delta for preview rendering (world space)
        let drag_delta = if let lightningbeam_core::tool::ToolState::DraggingSelection { ref start_mouse, .. } = shared.tool_state {
            // Get current mouse position in world coordinates
            if let Some(mouse_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let mouse_canvas_pos = mouse_pos - rect.min;
                let world_mouse = (mouse_canvas_pos - self.pan_offset) / self.zoom;

                let delta_x = world_mouse.x as f64 - start_mouse.x;
                let delta_y = world_mouse.y as f64 - start_mouse.y;

                Some(vello::kurbo::Vec2::new(delta_x, delta_y))
            } else {
                None
            }
        } else {
            None
        };

        // Use egui's custom painting callback for Vello
        let callback = VelloCallback::new(
            rect,
            self.pan_offset,
            self.zoom,
            self.instance_id,
            shared.action_executor.document().clone(),
            shared.tool_state.clone(),
            *shared.active_layer_id,
            drag_delta,
            shared.selection.clone(),
            *shared.fill_color,
            *shared.stroke_color,
            *shared.selected_tool,
            self.pending_eyedropper_sample,
        );

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
