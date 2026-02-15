/// GPU-based Constant-Q Transform (CQT) spectrogram with streaming ring-buffer cache.
///
/// Replaces the old FFT spectrogram with a CQT that has logarithmic frequency spacing
/// (bins map directly to MIDI notes). Only the visible viewport is computed, with results
/// cached in a ring-buffer texture so scrolling only computes new columns.
///
/// Architecture:
/// - CqtGpuResources stored in CallbackResources (long-lived, holds pipelines)
/// - CqtCacheEntry per pool_index (cache texture, bin params, ring buffer state)
/// - CqtCallback implements CallbackTrait (per-frame compute + render)
/// - Compute shader reads audio from waveform mip-0 textures (already on GPU)
/// - Render shader reads from cache texture with colormap

use std::collections::HashMap;
use wgpu::util::DeviceExt;

use crate::waveform_gpu::WaveformGpuResources;

/// CQT parameters
const BINS_PER_OCTAVE: u32 = 24;
const FREQ_BINS: u32 = 174; // ceil(log2(4186.0 / 27.5) * 24) = ceil(173.95)
const HOP_SIZE: u32 = 512;
const CACHE_CAPACITY: u32 = 4096;
const MAX_COLS_PER_FRAME: u32 = 256;
const F_MIN: f64 = 27.5; // A0 = MIDI 21
const WAVEFORM_TEX_WIDTH: u32 = 2048;

/// Per-bin CQT kernel parameters, uploaded as a storage buffer.
/// Must match BinInfo in cqt_compute.wgsl.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CqtBinParams {
    window_length: u32,
    phase_step: f32, // 2*pi*Q / N_k
    _pad0: u32,
    _pad1: u32,
}

/// Compute shader uniform params. Must match CqtParams in cqt_compute.wgsl.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct CqtComputeParams {
    hop_size: u32,
    freq_bins: u32,
    cache_capacity: u32,
    cache_write_offset: u32,
    num_columns: u32,
    column_start: u32,
    tex_width: u32,
    total_frames: u32,
    sample_rate: f32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

/// Render shader uniform params. Must match Params in cqt_render.wgsl exactly.
/// Layout: clip_rect(16) + 18 × f32(72) + pad vec2(8) = 96 bytes
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CqtRenderParams {
    pub clip_rect: [f32; 4],          // 16 bytes @ offset 0
    pub viewport_start_time: f32,     // 4 @ 16
    pub pixels_per_second: f32,       // 4 @ 20
    pub audio_duration: f32,          // 4 @ 24
    pub sample_rate: f32,             // 4 @ 28
    pub clip_start_time: f32,         // 4 @ 32
    pub trim_start: f32,              // 4 @ 36
    pub freq_bins: f32,               // 4 @ 40
    pub bins_per_octave: f32,         // 4 @ 44
    pub hop_size: f32,                // 4 @ 48
    pub scroll_y: f32,                // 4 @ 52
    pub note_height: f32,             // 4 @ 56
    pub min_note: f32,                // 4 @ 60
    pub max_note: f32,                // 4 @ 64
    pub gamma: f32,                   // 4 @ 68
    pub cache_capacity: f32,          // 4 @ 72
    pub cache_start_column: f32,      // 4 @ 76
    pub cache_valid_start: f32,       // 4 @ 80
    pub cache_valid_end: f32,         // 4 @ 84
    pub _pad: [f32; 2],              // 8 @ 88, total 96
}

/// Per-pool-index cache entry with ring buffer and GPU resources.
#[allow(dead_code)]
struct CqtCacheEntry {
    // Cache texture (Rgba16Float for universal filterable + storage support)
    cache_texture: wgpu::Texture,
    cache_texture_view: wgpu::TextureView,
    cache_storage_view: wgpu::TextureView,
    cache_capacity: u32,
    freq_bins: u32,

    // Ring buffer state
    cache_start_column: i64,
    cache_valid_start: i64,
    cache_valid_end: i64,

    // CQT kernel data
    bin_params_buffer: wgpu::Buffer,

    // Waveform texture reference (cloned from WaveformGpuEntry)
    waveform_texture_view: wgpu::TextureView,
    waveform_total_frames: u64,

    // Bind groups
    compute_bind_group: wgpu::BindGroup,
    compute_uniform_buffer: wgpu::Buffer,
    render_bind_group: wgpu::BindGroup,
    render_uniform_buffer: wgpu::Buffer,

    // Metadata
    sample_rate: u32,
}

/// Global GPU resources for CQT (stored in egui_wgpu::CallbackResources).
pub struct CqtGpuResources {
    entries: HashMap<usize, CqtCacheEntry>,
    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group_layout: wgpu::BindGroupLayout,
    render_pipeline: wgpu::RenderPipeline,
    render_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Per-frame callback for computing and rendering a CQT spectrogram.
pub struct CqtCallback {
    pub pool_index: usize,
    pub params: CqtRenderParams,
    pub target_format: wgpu::TextureFormat,
    pub sample_rate: u32,
    /// Visible column range (global CQT column indices)
    pub visible_col_start: i64,
    pub visible_col_end: i64,
}

/// Precompute CQT bin parameters for a given sample rate.
fn precompute_bin_params(sample_rate: u32) -> Vec<CqtBinParams> {
    let b = BINS_PER_OCTAVE as f64;
    let q = 1.0 / (2.0_f64.powf(1.0 / b) - 1.0);

    (0..FREQ_BINS)
        .map(|k| {
            let f_k = F_MIN * 2.0_f64.powf(k as f64 / b);
            let n_k = (q * sample_rate as f64 / f_k).ceil() as u32;
            let phase_step = (2.0 * std::f64::consts::PI * q / n_k as f64) as f32;
            CqtBinParams {
                window_length: n_k,
                phase_step,
                _pad0: 0,
                _pad1: 0,
            }
        })
        .collect()
}

impl CqtGpuResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        // Compute shader
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cqt_compute_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/cqt_compute.wgsl").into(),
            ),
        });

        // Render shader
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cqt_render_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/cqt_render.wgsl").into(),
            ),
        });

        // Compute bind group layout:
        //   0: audio_tex (texture_2d<f32>, read)
        //   1: cqt_out (texture_storage_2d<rgba16float, write>)
        //   2: params (uniform)
        //   3: bins (storage, read)
        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cqt_compute_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format: wgpu::TextureFormat::Rgba16Float,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Render bind group layout: cache_tex + sampler + uniforms
        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cqt_render_bgl"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Compute pipeline
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cqt_compute_pipeline_layout"),
                bind_group_layouts: &[&compute_bind_group_layout],
                push_constant_ranges: &[],
            });

        let compute_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("cqt_compute_pipeline"),
                layout: Some(&compute_pipeline_layout),
                module: &compute_shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });

        // Render pipeline
        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cqt_render_pipeline_layout"),
                bind_group_layouts: &[&render_bind_group_layout],
                push_constant_ranges: &[],
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cqt_render_pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Bilinear sampler for smooth interpolation in render shader
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cqt_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            entries: HashMap::new(),
            compute_pipeline,
            compute_bind_group_layout,
            render_pipeline,
            render_bind_group_layout,
            sampler,
        }
    }

    /// Create a cache entry for a pool index, referencing the waveform texture.
    fn ensure_cache_entry(
        &mut self,
        device: &wgpu::Device,
        pool_index: usize,
        waveform_texture_view: wgpu::TextureView,
        total_frames: u64,
        sample_rate: u32,
    ) {
        if self.entries.contains_key(&pool_index) {
            return;
        }

        // Create cache texture (ring buffer)
        let cache_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("cqt_cache_{}", pool_index)),
            size: wgpu::Extent3d {
                width: CACHE_CAPACITY,
                height: FREQ_BINS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let cache_texture_view = cache_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("cqt_cache_{}_view", pool_index)),
            ..Default::default()
        });

        let cache_storage_view = cache_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("cqt_cache_{}_storage", pool_index)),
            ..Default::default()
        });

        // Precompute bin params
        let bin_params = precompute_bin_params(sample_rate);
        let bin_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("cqt_bins_{}", pool_index)),
            contents: bytemuck::cast_slice(&bin_params),
            usage: wgpu::BufferUsages::STORAGE,
        });

        // Compute uniform buffer
        let compute_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("cqt_compute_uniforms_{}", pool_index)),
            size: std::mem::size_of::<CqtComputeParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Render uniform buffer
        let render_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("cqt_render_uniforms_{}", pool_index)),
            size: std::mem::size_of::<CqtRenderParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Compute bind group
        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("cqt_compute_bg_{}", pool_index)),
            layout: &self.compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&waveform_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&cache_storage_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: compute_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: bin_params_buffer.as_entire_binding(),
                },
            ],
        });

        // Render bind group
        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("cqt_render_bg_{}", pool_index)),
            layout: &self.render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cache_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: render_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        self.entries.insert(
            pool_index,
            CqtCacheEntry {
                cache_texture,
                cache_texture_view,
                cache_storage_view,
                cache_capacity: CACHE_CAPACITY,
                freq_bins: FREQ_BINS,
                cache_start_column: 0,
                cache_valid_start: 0,
                cache_valid_end: 0,
                bin_params_buffer,
                waveform_texture_view,
                waveform_total_frames: total_frames,
                compute_bind_group,
                compute_uniform_buffer,
                render_bind_group,
                render_uniform_buffer,
                sample_rate,
            },
        );
    }

}

/// Dispatch compute shader to fill CQT columns in the cache.
/// Free function to avoid borrow conflicts with CqtGpuResources.entries.
fn dispatch_cqt_compute(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pipeline: &wgpu::ComputePipeline,
    entry: &CqtCacheEntry,
    start_col: i64,
    end_col: i64,
) -> Vec<wgpu::CommandBuffer> {
    let num_cols = (end_col - start_col) as u32;
    if num_cols == 0 {
        return Vec::new();
    }

    // Clamp to max per frame
    let num_cols = num_cols.min(MAX_COLS_PER_FRAME);

    // Calculate ring buffer write offset
    let cache_write_offset =
        ((start_col - entry.cache_start_column) as u32) % entry.cache_capacity;

    let params = CqtComputeParams {
        hop_size: HOP_SIZE,
        freq_bins: FREQ_BINS,
        cache_capacity: entry.cache_capacity,
        cache_write_offset,
        num_columns: num_cols,
        column_start: start_col.max(0) as u32,
        tex_width: WAVEFORM_TEX_WIDTH,
        total_frames: entry.waveform_total_frames as u32,
        sample_rate: entry.sample_rate as f32,
        _pad0: 0,
        _pad1: 0,
        _pad2: 0,
    };

    queue.write_buffer(
        &entry.compute_uniform_buffer,
        0,
        bytemuck::cast_slice(&[params]),
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cqt_compute_encoder"),
    });

    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("cqt_compute_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &entry.compute_bind_group, &[]);

        // Dispatch: X = ceil(freq_bins / 64), Y = num_columns
        let workgroups_x = (FREQ_BINS + 63) / 64;
        pass.dispatch_workgroups(workgroups_x, num_cols, 1);
    }

    vec![encoder.finish()]
}

impl egui_wgpu::CallbackTrait for CqtCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Initialize CQT resources if needed
        if !resources.contains::<CqtGpuResources>() {
            resources.insert(CqtGpuResources::new(device, self.target_format));
        }

        // First, check if waveform data is available and extract what we need
        let waveform_info: Option<(wgpu::TextureView, u64)> = {
            let waveform_gpu: Option<&WaveformGpuResources> = resources.get();
            waveform_gpu.and_then(|wgpu_res| {
                wgpu_res.entries.get(&self.pool_index).map(|entry| {
                    // Clone the texture view (Arc internally, cheap)
                    (entry.texture_views[0].clone(), entry.total_frames)
                })
            })
        };

        let (waveform_view, total_frames) = match waveform_info {
            Some(info) => info,
            None => return Vec::new(), // Waveform not uploaded yet
        };

        let cqt_gpu: &mut CqtGpuResources = resources.get_mut().unwrap();

        // Ensure cache entry exists
        cqt_gpu.ensure_cache_entry(
            device,
            self.pool_index,
            waveform_view,
            total_frames,
            self.sample_rate,
        );

        // Determine which columns need computing
        let vis_start = self.visible_col_start.max(0);
        let max_col = (total_frames as i64) / HOP_SIZE as i64;
        let vis_end = self.visible_col_end.min(max_col);

        // Read current cache state, compute what's needed, then update state.
        // We split borrows carefully: read entry state, compute, then write back.
        let cmds;
        {
            let entry = cqt_gpu.entries.get(&self.pool_index).unwrap();
            let cache_valid_start = entry.cache_valid_start;
            let cache_valid_end = entry.cache_valid_end;

            if vis_start >= vis_end {
                cmds = Vec::new();
            } else if vis_start >= cache_valid_start && vis_end <= cache_valid_end {
                // Fully cached
                cmds = Vec::new();
            } else if vis_start >= cache_valid_start
                && vis_start < cache_valid_end
                && vis_end > cache_valid_end
            {
                // Scrolling right
                let actual_end =
                    cache_valid_end + (vis_end - cache_valid_end).min(MAX_COLS_PER_FRAME as i64);
                cmds = dispatch_cqt_compute(
                    device, queue, &cqt_gpu.compute_pipeline, entry,
                    cache_valid_end, actual_end,
                );
                let entry = cqt_gpu.entries.get_mut(&self.pool_index).unwrap();
                entry.cache_valid_end = actual_end;
                if entry.cache_valid_end - entry.cache_valid_start > entry.cache_capacity as i64 {
                    entry.cache_valid_start = entry.cache_valid_end - entry.cache_capacity as i64;
                    entry.cache_start_column = entry.cache_valid_start;
                }
            } else if vis_end <= cache_valid_end
                && vis_end > cache_valid_start
                && vis_start < cache_valid_start
            {
                // Scrolling left
                let actual_start =
                    cache_valid_start - (cache_valid_start - vis_start).min(MAX_COLS_PER_FRAME as i64);
                cmds = dispatch_cqt_compute(
                    device, queue, &cqt_gpu.compute_pipeline, entry,
                    actual_start, cache_valid_start,
                );
                let entry = cqt_gpu.entries.get_mut(&self.pool_index).unwrap();
                entry.cache_valid_start = actual_start;
                entry.cache_start_column = actual_start;
                if entry.cache_valid_end - entry.cache_valid_start > entry.cache_capacity as i64 {
                    entry.cache_valid_end = entry.cache_valid_start + entry.cache_capacity as i64;
                }
            } else {
                // No overlap or first compute — reset cache
                let entry = cqt_gpu.entries.get_mut(&self.pool_index).unwrap();
                entry.cache_start_column = vis_start;
                entry.cache_valid_start = vis_start;
                entry.cache_valid_end = vis_start;

                let compute_end = vis_start + (vis_end - vis_start).min(MAX_COLS_PER_FRAME as i64);
                let entry = cqt_gpu.entries.get(&self.pool_index).unwrap();
                cmds = dispatch_cqt_compute(
                    device, queue, &cqt_gpu.compute_pipeline, entry,
                    vis_start, compute_end,
                );
                let entry = cqt_gpu.entries.get_mut(&self.pool_index).unwrap();
                entry.cache_valid_end = compute_end;
            }
        }

        // Update render uniform buffer
        let entry = cqt_gpu.entries.get(&self.pool_index).unwrap();
        let mut params = self.params;
        params.cache_start_column = entry.cache_start_column as f32;
        params.cache_valid_start = entry.cache_valid_start as f32;
        params.cache_valid_end = entry.cache_valid_end as f32;
        params.cache_capacity = entry.cache_capacity as f32;

        queue.write_buffer(
            &entry.render_uniform_buffer,
            0,
            bytemuck::cast_slice(&[params]),
        );

        cmds
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let cqt_gpu: &CqtGpuResources = match resources.get() {
            Some(r) => r,
            None => return,
        };

        let entry = match cqt_gpu.entries.get(&self.pool_index) {
            Some(e) => e,
            None => return,
        };

        // Don't render if nothing is cached yet
        if entry.cache_valid_start >= entry.cache_valid_end {
            return;
        }

        render_pass.set_pipeline(&cqt_gpu.render_pipeline);
        render_pass.set_bind_group(0, &entry.render_bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
