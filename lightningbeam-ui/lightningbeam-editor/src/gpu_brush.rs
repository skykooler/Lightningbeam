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
// Raster affine-transform pipeline
// ---------------------------------------------------------------------------

/// CPU-side parameters for the raster transform compute shader.
/// Must match the `Params` struct in `raster_transform.wgsl` (48 bytes, 16-byte aligned).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RasterTransformGpuParams {
    pub a00: f32, pub a01: f32,  // row 0 of 2×2 inverse affine matrix
    pub a10: f32, pub a11: f32,  // row 1
    pub b0:  f32, pub b1:  f32,  // translation (source pixel offset at output (0,0))
    pub src_w: u32, pub src_h: u32,
    pub dst_w: u32, pub dst_h: u32,
    pub _pad0: u32, pub _pad1: u32,
}

/// Compute pipeline for GPU-accelerated affine resampling of raster floats.
/// Created lazily on first transform use.
struct RasterTransformPipeline {
    pipeline:        wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl RasterTransformPipeline {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label:  Some("raster_transform_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/raster_transform.wgsl").into(),
            ),
        });

        let bind_group_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("raster_transform_bgl"),
                entries: &[
                    // 0: params uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // 1: source texture (anchor canvas, sampled)
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type:    wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled:   false,
                        },
                        count: None,
                    },
                    // 2: destination texture (float canvas dst, write-only storage)
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
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
                label:                Some("raster_transform_pl"),
                bind_group_layouts:   &[&bind_group_layout],
                push_constant_ranges: &[],
            },
        );

        let pipeline = device.create_compute_pipeline(
            &wgpu::ComputePipelineDescriptor {
                label:   Some("raster_transform_pipeline"),
                layout:  Some(&pipeline_layout),
                module:  &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            },
        );

        Self { pipeline, bind_group_layout }
    }

    /// Dispatch the transform shader: reads from `src_view`, writes to `dst_view`.
    /// The caller must call `dst_canvas.swap()` after this returns.
    fn render(
        &self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        src_view: &wgpu::TextureView,
        dst_view: &wgpu::TextureView,
        params:   RasterTransformGpuParams,
    ) {
        use wgpu::util::DeviceExt;

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("raster_transform_params"),
            contents: bytemuck::bytes_of(&params),
            usage:    wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("raster_transform_bg"),
            layout:  &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding:  0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding:  1,
                    resource: wgpu::BindingResource::TextureView(src_view),
                },
                wgpu::BindGroupEntry {
                    binding:  2,
                    resource: wgpu::BindingResource::TextureView(dst_view),
                },
            ],
        });

        let mut encoder = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("raster_transform_enc") },
        );
        {
            let mut pass = encoder.begin_compute_pass(
                &wgpu::ComputePassDescriptor { label: Some("raster_transform_pass"), timestamp_writes: None },
            );
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            let wg_x = params.dst_w.div_ceil(8);
            let wg_y = params.dst_h.div_ceil(8);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
        queue.submit(Some(encoder.finish()));
    }
}

// ---------------------------------------------------------------------------
// GpuBrushEngine
// ---------------------------------------------------------------------------

/// GPU brush engine — holds the compute pipeline and per-keyframe canvas pairs.
pub struct GpuBrushEngine {
    compute_pipeline:   wgpu::ComputePipeline,
    compute_bg_layout:  wgpu::BindGroupLayout,

    /// Lazily created on first raster transform use.
    transform_pipeline: Option<RasterTransformPipeline>,

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
            transform_pipeline: None,
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
    /// Paint/erase dabs are batched in a single GPU dispatch with a full canvas copy.
    /// Smudge dabs are dispatched sequentially (one per dab) with a bbox-only copy
    /// so each dab reads the canvas state written by the previous dab.
    ///
    /// If `dabs` is empty, does nothing.
    pub fn render_dabs(
        &mut self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        keyframe_id: Uuid,
        dabs:   &[GpuDab],
        bbox:   (i32, i32, i32, i32),
        canvas_w: u32,
        canvas_h: u32,
    ) {
        if dabs.is_empty() { return; }

        // Smudge dabs must be applied one at a time so each dab reads the canvas
        // state written by the previous dab.  Use bbox-only copies (union of current
        // and previous dab) to avoid an expensive full-canvas copy per dab.
        let is_smudge = dabs.first().map(|d| d.blend_mode == 2).unwrap_or(false);
        if is_smudge {
            let mut prev_bbox: Option<(i32, i32, i32, i32)> = None;
            for dab in dabs {
                let r = dab.radius + 1.0;
                let cur_bbox = (
                    (dab.x - r).floor() as i32,
                    (dab.y - r).floor() as i32,
                    (dab.x + r).ceil()  as i32,
                    (dab.y + r).ceil()  as i32,
                );
                // Expand copy region to include the previous dab's bbox so the
                // pixels it wrote are visible as the source for this dab's smudge.
                let copy_bbox = match prev_bbox {
                    Some(pb) => (cur_bbox.0.min(pb.0), cur_bbox.1.min(pb.1),
                                 cur_bbox.2.max(pb.2), cur_bbox.3.max(pb.3)),
                    None     => cur_bbox,
                };
                self.render_dabs_batch(device, queue, keyframe_id,
                    std::slice::from_ref(dab), cur_bbox, Some(copy_bbox), canvas_w, canvas_h);
                prev_bbox = Some(cur_bbox);
            }
        } else {
            self.render_dabs_batch(device, queue, keyframe_id, dabs, bbox, None, canvas_w, canvas_h);
        }
    }

    /// Inner batch dispatch.
    ///
    /// `dispatch_bbox` — region dispatched to the compute shader (usually the union of all dab bboxes).
    /// `copy_bbox`     — region to copy src→dst before dispatch:
    ///   - `None`      → copy the full canvas (required for paint/erase batches so
    ///                   dabs outside the current frame's region are preserved).
    ///   - `Some(r)`   → copy only region `r` (sufficient for sequential smudge dabs
    ///                   because both textures hold identical data outside previously
    ///                   touched regions, so no full copy is needed).
    fn render_dabs_batch(
        &mut self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        keyframe_id: Uuid,
        dabs:        &[GpuDab],
        dispatch_bbox: (i32, i32, i32, i32),
        copy_bbox:   Option<(i32, i32, i32, i32)>,
        canvas_w: u32,
        canvas_h: u32,
    ) {
        if dabs.is_empty() { return; }
        let canvas = match self.canvases.get_mut(&keyframe_id) {
            Some(c) => c,
            None => return,
        };

        // Clamp the dispatch bounding box to canvas bounds.
        let bbox = dispatch_bbox;
        let x0 = bbox.0.max(0) as u32;
        let y0 = bbox.1.max(0) as u32;
        let x1 = (bbox.2 as u32).min(canvas_w);
        let y1 = (bbox.3 as u32).min(canvas_h);
        if x1 <= x0 || y1 <= y0 { return; }
        let bbox_w = x1 - x0;
        let bbox_h = y1 - y0;

        // Step 1: Copy src→dst.
        // For paint/erase batches (copy_bbox = None): copy the ENTIRE canvas so dst
        //   starts with all previous dabs — a bbox-only copy would lose dabs outside
        //   this frame's region after swap.
        // For smudge (copy_bbox = Some(r)): copy only the union of the current and
        //   previous dab bboxes.  Outside that region both textures hold identical
        //   data so no full copy is needed.
        let mut copy_enc = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("canvas_copy_encoder") },
        );
        match copy_bbox {
            None => {
                copy_enc.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture:   canvas.src(),
                        mip_level: 0,
                        origin:    wgpu::Origin3d::ZERO,
                        aspect:    wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture:   canvas.dst(),
                        mip_level: 0,
                        origin:    wgpu::Origin3d::ZERO,
                        aspect:    wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d { width: canvas_w, height: canvas_h, depth_or_array_layers: 1 },
                );
            }
            Some(cb) => {
                let cx0 = cb.0.max(0) as u32;
                let cy0 = cb.1.max(0) as u32;
                let cx1 = (cb.2 as u32).min(canvas_w);
                let cy1 = (cb.3 as u32).min(canvas_h);
                if cx1 > cx0 && cy1 > cy0 {
                    copy_enc.copy_texture_to_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture:   canvas.src(),
                            mip_level: 0,
                            origin:    wgpu::Origin3d { x: cx0, y: cy0, z: 0 },
                            aspect:    wgpu::TextureAspect::All,
                        },
                        wgpu::TexelCopyTextureInfo {
                            texture:   canvas.dst(),
                            mip_level: 0,
                            origin:    wgpu::Origin3d { x: cx0, y: cy0, z: 0 },
                            aspect:    wgpu::TextureAspect::All,
                        },
                        wgpu::Extent3d { width: cx1 - cx0, height: cy1 - cy0, depth_or_array_layers: 1 },
                    );
                }
            }
        }
        queue.submit(Some(copy_enc.finish()));

        // Step 2: Upload all dabs as a single storage buffer.
        let dab_bytes = bytemuck::cast_slice(dabs);
        let dab_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("dab_storage_buf"),
            size:               dab_bytes.len() as u64,
            usage:              wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&dab_buf, 0, dab_bytes);

        let params = DabParams {
            bbox_x0: x0 as i32,
            bbox_y0: y0 as i32,
            bbox_w,
            bbox_h,
            num_dabs: dabs.len() as u32,
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
                wgpu::BindGroupEntry { binding: 0, resource: dab_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: params_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(canvas.src_view()) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(canvas.dst_view()) },
            ],
        });

        // Step 3: Single dispatch over the union bounding box.
        let mut compute_enc = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("brush_dab_encoder") },
        );
        {
            let mut pass = compute_enc.begin_compute_pass(
                &wgpu::ComputePassDescriptor { label: Some("brush_dab_pass"), timestamp_writes: None },
            );
            pass.set_pipeline(&self.compute_pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(bbox_w.div_ceil(8), bbox_h.div_ceil(8), 1);
        }
        queue.submit(Some(compute_enc.finish()));

        // Step 4: Swap once — dst (with all dabs applied) becomes the new src.
        canvas.swap();
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

    /// Render a set of dabs to an offscreen texture and return the raw pixels.
    ///
    /// This is a **blocking** GPU readback — intended for one-time renders such as
    /// brush preview thumbnails.  Do not call every frame on the hot path.
    ///
    /// The returned `Vec<u8>` is in **sRGB-encoded premultiplied RGBA** format,
    /// suitable for creating an `egui::ColorImage` via
    /// `ColorImage::from_rgba_premultiplied`.
    ///
    /// A dedicated scratch canvas keyed by a fixed UUID is reused across calls so
    /// no allocation is needed after the first invocation.
    pub fn render_to_image(
        &mut self,
        device: &wgpu::Device,
        queue:  &wgpu::Queue,
        dabs:   &[GpuDab],
        width:  u32,
        height: u32,
    ) -> Vec<u8> {
        use std::sync::OnceLock;
        static SCRATCH_ID: OnceLock<Uuid> = OnceLock::new();
        let scratch_id = *SCRATCH_ID.get_or_init(Uuid::new_v4);

        // Ensure a correctly-sized scratch canvas exists.
        self.ensure_canvas(device, scratch_id, width, height);

        // Clear to transparent so previous renders don't bleed through.
        let blank = vec![0u8; (width * height * 4) as usize];
        if let Some(canvas) = self.canvases.get(&scratch_id) {
            canvas.upload(queue, &blank);
        }

        if !dabs.is_empty() {
            // Compute the union bounding box of all dabs.
            let bbox = dabs.iter().fold(
                (i32::MAX, i32::MAX, i32::MIN, i32::MIN),
                |acc, d| {
                    let r = d.radius + 1.0;
                    (
                        acc.0.min((d.x - r).floor() as i32),
                        acc.1.min((d.y - r).floor() as i32),
                        acc.2.max((d.x + r).ceil()  as i32),
                        acc.3.max((d.y + r).ceil()  as i32),
                    )
                },
            );
            self.render_dabs(device, queue, scratch_id, dabs, bbox, width, height);
        }

        self.readback_canvas(device, queue, scratch_id).unwrap_or_default()
    }

    /// Remove the canvas pair for a keyframe (e.g. when the layer is deleted).
    pub fn remove_canvas(&mut self, keyframe_id: &Uuid) {
        self.canvases.remove(keyframe_id);
    }

    /// Dispatch the affine-resample transform shader from `anchor_id` → `float_id`.
    ///
    /// Reads from the anchor canvas's source view, writes into the float canvas's
    /// destination view, then swaps the float canvas so the result becomes the new source.
    ///
    /// `float_id` must already have been resized to `params.dst_w × params.dst_h` via
    /// `ensure_canvas` before calling this.
    pub fn render_transform(
        &mut self,
        device:     &wgpu::Device,
        queue:      &wgpu::Queue,
        anchor_id:  &Uuid,
        float_id:   &Uuid,
        params:     RasterTransformGpuParams,
    ) {
        // Lazily create the transform pipeline.
        let pipeline = self.transform_pipeline
            .get_or_insert_with(|| RasterTransformPipeline::new(device));

        // Borrow src_view and dst_view within a block so the borrows end before
        // we call swap() on the float canvas.
        let dispatched = {
            let anchor = self.canvases.get(anchor_id);
            let float  = self.canvases.get(float_id);
            if let (Some(anchor), Some(float)) = (anchor, float) {
                pipeline.render(device, queue, anchor.src_view(), float.dst_view(), params);
                true
            } else {
                false
            }
        };

        if dispatched {
            if let Some(float) = self.canvases.get_mut(float_id) {
                float.swap();
            }
        }
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
