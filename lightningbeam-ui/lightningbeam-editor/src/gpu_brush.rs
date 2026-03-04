//! GPU-accelerated raster brush engine.
//!
//! [`GpuBrushEngine`] wraps the `brush_dab.wgsl` compute pipeline and manages
//! per-keyframe canvas texture pairs (ping-pong) used as the live canvas during
//! raster painting.
//!
//! ## Lifecycle
//!
//! 1. **Stroke start** — caller supplies the initial pixel data; the engine uploads
//!    it to both canvas textures so either can serve as source/dest.
//! 2. **Each drag event** — [`GpuBrushEngine::render_dabs`] copies src→dst,
//!    dispatches the compute shader, then swaps src/dst.
//! 3. **Stroke end** — [`GpuBrushEngine::readback_canvas`] copies the current
//!    source texture into a staging buffer and returns the raw RGBA bytes
//!    (blocking — uses `device.poll(Maintain::Wait)`).
//! 4. **Idle** — canvas textures are kept alive for the next stroke (no re-upload
//!    needed if the layer has not changed).

use std::collections::HashMap;
use uuid::Uuid;
use lightningbeam_core::brush_engine::GpuDab;

// ---------------------------------------------------------------------------
// Colour-space helpers
// ---------------------------------------------------------------------------

/// Decode one sRGB-encoded byte to linear float [0, 1].
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode one linear float [0, 1] to an sRGB-encoded byte.
fn linear_to_srgb_byte(c: u8) -> u8 {
    let f = c as f32 / 255.0;
    let encoded = if f <= 0.0031308 {
        f * 12.92
    } else {
        1.055 * f.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}

// ---------------------------------------------------------------------------
// Per-keyframe canvas texture pair (ping-pong)
// ---------------------------------------------------------------------------

/// A pair of textures used for double-buffered canvas rendering.
///
/// `current` indexes the texture that holds the up-to-date canvas state.
pub struct CanvasPair {
    pub textures: [wgpu::Texture; 2],
    pub views:    [wgpu::TextureView; 2],
    /// Index (0 or 1) of the texture that is the current "source" (authoritative).
    pub current: usize,
    pub width:   u32,
    pub height:  u32,
}

impl CanvasPair {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let desc = wgpu::TextureDescriptor {
            label:  Some("raster_canvas"),
            size:   wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage:  wgpu::TextureUsages::TEXTURE_BINDING
                  | wgpu::TextureUsages::STORAGE_BINDING
                  | wgpu::TextureUsages::COPY_SRC
                  | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let t0 = device.create_texture(&desc);
        let t1 = device.create_texture(&desc);
        let v0 = t0.create_view(&wgpu::TextureViewDescriptor::default());
        let v1 = t1.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            textures: [t0, t1],
            views:    [v0, v1],
            current: 0,
            width,
            height,
        }
    }

    /// Upload raw RGBA bytes to both textures (call once at stroke start).
    ///
    /// `pixels` is expected to be **sRGB-encoded premultiplied** (the format stored
    /// in `raw_pixels` / PNG files).  The values are decoded to linear premultiplied
    /// before being written to the canvas, which operates entirely in linear space.
    pub fn upload(&self, queue: &wgpu::Queue, pixels: &[u8]) {
        // Decode sRGB-premultiplied → linear premultiplied for the GPU canvas.
        let linear: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| {
            let r = (srgb_to_linear(p[0] as f32 / 255.0) * 255.0 + 0.5) as u8;
            let g = (srgb_to_linear(p[1] as f32 / 255.0) * 255.0 + 0.5) as u8;
            let b = (srgb_to_linear(p[2] as f32 / 255.0) * 255.0 + 0.5) as u8;
            [r, g, b, p[3]]
        }).collect();

        let layout = wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(self.width * 4),
            rows_per_image: Some(self.height),
        };
        let extent = wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        };
        for tex in &self.textures {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &linear,
                layout,
                extent,
            );
        }
    }

    /// Source (current, authoritative) texture.
    pub fn src(&self) -> &wgpu::Texture { &self.textures[self.current] }
    /// Source texture view.
    pub fn src_view(&self) -> &wgpu::TextureView { &self.views[self.current] }
    /// Destination (write target) texture.
    pub fn dst(&self) -> &wgpu::Texture { &self.textures[1 - self.current] }
    /// Destination texture view.
    pub fn dst_view(&self) -> &wgpu::TextureView { &self.views[1 - self.current] }
    /// Commit the just-completed dispatch: make dst the new source.
    pub fn swap(&mut self) { self.current = 1 - self.current; }
}

// ---------------------------------------------------------------------------
// GpuBrushEngine
// ---------------------------------------------------------------------------

/// GPU brush engine — holds the compute pipeline and per-keyframe canvas pairs.
pub struct GpuBrushEngine {
    compute_pipeline:   wgpu::ComputePipeline,
    compute_bg_layout:  wgpu::BindGroupLayout,

    /// Canvas texture pairs keyed by keyframe UUID.
    pub canvases: HashMap<Uuid, CanvasPair>,
}

/// CPU-side parameters uniform for the compute shader.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DabParams {
    bbox_x0:  i32,
    bbox_y0:  i32,
    bbox_w:   u32,
    bbox_h:   u32,
    num_dabs: u32,
    canvas_w: u32,
    canvas_h: u32,
    _pad:     u32,
}

impl GpuBrushEngine {
    /// Create the pipeline.  Returns `Err` if the device lacks the required
    /// storage-texture capability for `Rgba8Unorm`.
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("brush_dab_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/brush_dab.wgsl").into(),
            ),
        });

        let compute_bg_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("brush_dab_bgl"),
                entries: &[
                    // 0: dab storage buffer (read-only)
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty:                wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size:  None,
                        },
                        count: None,
                    },
                    // 1: params uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty:                wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size:  None,
                        },
                        count: None,
                    },
                    // 2: canvas source (sampled)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled:   false,
                        },
                        count: None,
                    },
                    // 3: canvas destination (write-only storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access:         wgpu::StorageTextureAccess::WriteOnly,
                            format:         wgpu::TextureFormat::Rgba8Unorm,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                ],
            },
        );

        let pipeline_layout = device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label:                Some("brush_dab_pl"),
                bind_group_layouts:   &[&compute_bg_layout],
                push_constant_ranges: &[],
            },
        );

        let compute_pipeline = device.create_compute_pipeline(
            &wgpu::ComputePipelineDescriptor {
                label:   Some("brush_dab_pipeline"),
                layout:  Some(&pipeline_layout),
                module:  &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            },
        );

        Self {
            compute_pipeline,
            compute_bg_layout,
            canvases: HashMap::new(),
        }
    }

    /// Ensure a canvas pair exists for `keyframe_id` at the given dimensions.
    ///
    /// If the canvas exists but has different dimensions it is replaced.
    pub fn ensure_canvas(
        &mut self,
        device: &wgpu::Device,
        keyframe_id: Uuid,
        width: u32,
        height: u32,
    ) -> &mut CanvasPair {
        let needs_new = self.canvases.get(&keyframe_id)
            .map_or(true, |c| c.width != width || c.height != height);
        if needs_new {
            self.canvases.insert(keyframe_id, CanvasPair::new(device, width, height));
        } else {
        }
        self.canvases.get_mut(&keyframe_id).unwrap()
    }

    /// Dispatch the brush compute shader for `dabs` onto the canvas of `keyframe_id`.
    ///
    /// Each dab is dispatched as a separate copy+compute+swap so that every dab
    /// reads the result of the previous one.  This is required for the smudge tool:
    /// if all dabs were batched into one dispatch they would all read the pre-batch
    /// canvas state, breaking the carry-forward that makes smudge drag pixels along.
    ///
    /// `dab_bbox` is the union bounding box (unused here; kept for API compat).
    /// If `dabs` is empty, does nothing.
    pub fn render_dabs(
        &mut self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        keyframe_id: Uuid,
        dabs:   &[GpuDab],
        _bbox:  (i32, i32, i32, i32),
        canvas_w: u32,
        canvas_h: u32,
    ) {
        if dabs.is_empty() { return; }

        if !self.canvases.contains_key(&keyframe_id) { return; }

        let full_extent = wgpu::Extent3d {
            width:  self.canvases[&keyframe_id].width,
            height: self.canvases[&keyframe_id].height,
            depth_or_array_layers: 1,
        };

        for dab in dabs {
            // Per-dab bounding box
            let r_fringe = dab.radius + 1.0;
            let dx0 = (dab.x - r_fringe).floor() as i32;
            let dy0 = (dab.y - r_fringe).floor() as i32;
            let dx1 = (dab.x + r_fringe).ceil()  as i32;
            let dy1 = (dab.y + r_fringe).ceil()  as i32;

            let x0 = dx0.max(0) as u32;
            let y0 = dy0.max(0) as u32;
            let x1 = (dx1.min(canvas_w as i32 - 1)).max(0) as u32;
            let y1 = (dy1.min(canvas_h as i32 - 1)).max(0) as u32;
            if x1 < x0 || y1 < y0 { continue; }

            let bbox_w = x1 - x0 + 1;
            let bbox_h = y1 - y0 + 1;

            let canvas = self.canvases.get_mut(&keyframe_id).unwrap();

            // Pre-fill dst from src so pixels outside this dab's bbox are preserved.
            let mut copy_enc = device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("canvas_copy_encoder") },
            );
            copy_enc.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture:  canvas.src(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture:  canvas.dst(),
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                full_extent,
            );
            queue.submit(Some(copy_enc.finish()));

            // Upload single-dab buffer and params
            let dab_bytes = bytemuck::bytes_of(dab);
            let dab_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("dab_storage_buf"),
                size:               dab_bytes.len() as u64,
                usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&dab_buf, 0, dab_bytes);

            let params = DabParams {
                bbox_x0:  x0 as i32,
                bbox_y0:  y0 as i32,
                bbox_w,
                bbox_h,
                num_dabs: 1,
                canvas_w,
                canvas_h,
                _pad: 0,
            };
            let params_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label:              Some("dab_params_buf"),
                size:               std::mem::size_of::<DabParams>() as u64,
                usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&params_buf, 0, bytemuck::bytes_of(&params));

            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label:  Some("brush_dab_bg"),
                layout: &self.compute_bg_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: dab_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(canvas.src_view()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(canvas.dst_view()),
                    },
                ],
            });

            let mut compute_enc = device.create_command_encoder(
                &wgpu::CommandEncoderDescriptor { label: Some("brush_dab_encoder") },
            );
            {
                let mut pass = compute_enc.begin_compute_pass(
                    &wgpu::ComputePassDescriptor {
                        label: Some("brush_dab_pass"),
                        timestamp_writes: None,
                    },
                );
                pass.set_pipeline(&self.compute_pipeline);
                pass.set_bind_group(0, &bg, &[]);
                pass.dispatch_workgroups(bbox_w.div_ceil(8), bbox_h.div_ceil(8), 1);
            }
            queue.submit(Some(compute_enc.finish()));

            // Swap: the just-written dst becomes src for the next dab.
            canvas.swap();
        }
    }

    /// Read the current canvas back to a CPU `Vec<u8>` (raw RGBA, row-major).
    ///
    /// **Blocks** until the GPU work is complete (`Maintain::Wait`).
    /// Should only be called at stroke end, not every frame.
    ///
    /// Returns `None` if no canvas exists for `keyframe_id`.
    pub fn readback_canvas(
        &self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        keyframe_id: Uuid,
    ) -> Option<Vec<u8>> {
        let canvas = self.canvases.get(&keyframe_id)?;
        let width  = canvas.width;
        let height = canvas.height;

        // wgpu requires bytes_per_row to be a multiple of 256
        let bytes_per_row_aligned =
            ((width * 4 + 255) / 256) * 256;
        let total_bytes = (bytes_per_row_aligned * height) as u64;

        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("canvas_readback_buf"),
            size:               total_bytes,
            usage:              wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("canvas_readback_encoder") },
        );
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture:  canvas.src(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset:         0,
                    bytes_per_row:  Some(bytes_per_row_aligned),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));

        // Block until complete
        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if rx.recv().ok()?.is_err() { return None; }

        let mapped = slice.get_mapped_range();

        // De-stride: copy only `width * 4` bytes per row (drop alignment padding)
        let bytes_per_row_tight = (width * 4) as usize;
        let bytes_per_row_src   = bytes_per_row_aligned as usize;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for row in 0..height as usize {
            let src = &mapped[row * bytes_per_row_src .. row * bytes_per_row_src + bytes_per_row_tight];
            let dst = &mut pixels[row * bytes_per_row_tight .. (row + 1) * bytes_per_row_tight];
            dst.copy_from_slice(src);
        }

        drop(mapped);
        staging.unmap();

        // Encode linear premultiplied → sRGB-encoded premultiplied so the returned
        // bytes match what Vello expects (ImageAlphaType::Premultiplied with sRGB
        // channels).  Alpha is left unchanged.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[0] = linear_to_srgb_byte(pixel[0]);
            pixel[1] = linear_to_srgb_byte(pixel[1]);
            pixel[2] = linear_to_srgb_byte(pixel[2]);
        }

        Some(pixels)
    }

    /// Remove the canvas pair for a keyframe (e.g. when the layer is deleted).
    pub fn remove_canvas(&mut self, keyframe_id: &Uuid) {
        self.canvases.remove(keyframe_id);
    }
}

// ---------------------------------------------------------------------------
// Canvas blit pipeline (renders canvas texture to layer sRGB buffer)
// ---------------------------------------------------------------------------

/// Bind group layout + pipeline for blitting a canvas texture (at document
/// resolution) into a layer render buffer (at viewport resolution), applying
/// the camera transform.
pub struct CanvasBlitPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub bg_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    /// Nearest-neighbour sampler used for the selection mask texture.
    pub mask_sampler: wgpu::Sampler,
}

/// Camera parameters uniform for canvas_blit.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraParams {
    pub pan_x:      f32,
    pub pan_y:      f32,
    pub zoom:       f32,
    pub canvas_w:   f32,
    pub canvas_h:   f32,
    pub viewport_w: f32,
    pub viewport_h: f32,
    pub _pad:       f32,
}

impl CanvasBlitPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("canvas_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/canvas_blit.wgsl").into(),
            ),
        });

        let bg_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("canvas_blit_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled:   false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty:                wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size:  None,
                        },
                        count: None,
                    },
                    // Binding 3: selection mask texture (R8Unorm; 1×1 white = no mask)
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled:   false,
                        },
                        count: None,
                    },
                    // Binding 4: nearest sampler for mask (sharp selection edges)
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            },
        );

        let pipeline_layout = device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label:                Some("canvas_blit_pl"),
                bind_group_layouts:   &[&bg_layout],
                push_constant_ranges: &[],
            },
        );

        let pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label:  Some("canvas_blit_pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module:  &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module:  &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format:     wgpu::TextureFormat::Rgba16Float,
                        blend:      None, // canvas already stores premultiplied alpha
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample:   wgpu::MultisampleState::default(),
                multiview:     None,
                cache:         None,
            },
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("canvas_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            mipmap_filter:  wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let mask_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label:          Some("canvas_mask_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            mipmap_filter:  wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self { pipeline, bg_layout, sampler, mask_sampler }
    }

    /// Render the canvas texture into `target_view` (Rgba16Float) with the given camera.
    ///
    /// `target_view` is cleared to transparent before writing.
    /// `mask_view` is an R8Unorm texture in canvas-pixel space: 255 = keep, 0 = discard.
    /// Pass `None` to use the built-in 1×1 all-white default (no masking).
    pub fn blit(
        &self,
        device:      &wgpu::Device,
        queue:       &wgpu::Queue,
        canvas_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        camera:      &CameraParams,
        mask_view:   Option<&wgpu::TextureView>,
    ) {
        // When no mask is provided, create a temporary 1×1 all-white texture.
        // (queue is already available here, unlike in new())
        let tmp_mask_tex;
        let tmp_mask_view;
        let mask_view: &wgpu::TextureView = match mask_view {
            Some(v) => v,
            None => {
                tmp_mask_tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("canvas_default_mask"),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &tmp_mask_tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &[255u8],
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1), rows_per_image: Some(1) },
                    wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                );
                tmp_mask_view = tmp_mask_tex.create_view(&Default::default());
                &tmp_mask_view
            }
        };
        // Upload camera params
        let cam_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("canvas_blit_cam_buf"),
            size:               std::mem::size_of::<CameraParams>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&cam_buf, 0, bytemuck::bytes_of(camera));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:  Some("canvas_blit_bg"),
            layout: &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: wgpu::BindingResource::TextureView(canvas_view),
                },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding:  2,
                    resource: cam_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding:  3,
                    resource: wgpu::BindingResource::TextureView(mask_view),
                },
                wgpu::BindGroupEntry {
                    binding:  4,
                    resource: wgpu::BindingResource::Sampler(&self.mask_sampler),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("canvas_blit_encoder") },
        );
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("canvas_blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view:           target_view,
                    resolve_target: None,
                    depth_slice:    None,
                    ops:            wgpu::Operations {
                        load:  wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set:      None,
                timestamp_writes:         None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..4, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }
}
