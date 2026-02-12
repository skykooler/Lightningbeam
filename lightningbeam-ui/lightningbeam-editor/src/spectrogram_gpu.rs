/// GPU resources for spectrogram rendering.
///
/// Follows the same pattern as waveform_gpu.rs:
/// - SpectrogramGpuResources stored in CallbackResources (long-lived)
/// - SpectrogramCallback implements egui_wgpu::CallbackTrait (per-frame)
/// - R32Float texture holds magnitude data (time bins × freq bins)
/// - Fragment shader applies colormap and frequency mapping

use std::collections::HashMap;

/// GPU resources for all spectrograms (stored in egui_wgpu::CallbackResources)
pub struct SpectrogramGpuResources {
    pub entries: HashMap<usize, SpectrogramGpuEntry>,
    render_pipeline: wgpu::RenderPipeline,
    render_bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// Per-audio-pool GPU data for one spectrogram
#[allow(dead_code)]
pub struct SpectrogramGpuEntry {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub render_bind_group: wgpu::BindGroup,
    pub uniform_buffer: wgpu::Buffer,
    pub time_bins: u32,
    pub freq_bins: u32,
    pub sample_rate: u32,
    pub hop_size: u32,
    pub fft_size: u32,
    pub duration: f32,
}

/// Uniform buffer struct — must match spectrogram.wgsl Params exactly
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SpectrogramParams {
    pub clip_rect: [f32; 4],        // 16 bytes @ offset 0
    pub viewport_start_time: f32,    // 4 bytes @ offset 16
    pub pixels_per_second: f32,      // 4 bytes @ offset 20
    pub audio_duration: f32,         // 4 bytes @ offset 24
    pub sample_rate: f32,            // 4 bytes @ offset 28
    pub clip_start_time: f32,        // 4 bytes @ offset 32
    pub trim_start: f32,             // 4 bytes @ offset 36
    pub time_bins: f32,              // 4 bytes @ offset 40
    pub freq_bins: f32,              // 4 bytes @ offset 44
    pub hop_size: f32,               // 4 bytes @ offset 48
    pub fft_size: f32,               // 4 bytes @ offset 52
    pub scroll_y: f32,               // 4 bytes @ offset 56
    pub note_height: f32,            // 4 bytes @ offset 60
    pub screen_size: [f32; 2],       // 8 bytes @ offset 64
    pub min_note: f32,               // 4 bytes @ offset 72
    pub max_note: f32,               // 4 bytes @ offset 76
    pub gamma: f32,                  // 4 bytes @ offset 80
    pub _pad: [f32; 3],              // 12 bytes @ offset 84 (pad to 96 for WGSL struct alignment)
}
// Total: 96 bytes (multiple of 16 for vec4 alignment)

/// Data for a pending spectrogram texture upload
pub struct SpectrogramUpload {
    pub magnitudes: Vec<f32>,
    pub time_bins: u32,
    pub freq_bins: u32,
    pub sample_rate: u32,
    pub hop_size: u32,
    pub fft_size: u32,
    pub duration: f32,
}

/// Per-frame callback for rendering one spectrogram instance
pub struct SpectrogramCallback {
    pub pool_index: usize,
    pub params: SpectrogramParams,
    pub target_format: wgpu::TextureFormat,
    pub pending_upload: Option<SpectrogramUpload>,
}

impl SpectrogramGpuResources {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        // Shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectrogram_render_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/spectrogram.wgsl").into(),
            ),
        });

        // Bind group layout: texture + sampler + uniforms
        let render_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("spectrogram_render_bgl"),
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

        // Render pipeline
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectrogram_pipeline_layout"),
            bind_group_layouts: &[&render_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spectrogram_render_pipeline"),
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

        // Bilinear sampler for smooth frequency interpolation
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("spectrogram_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            entries: HashMap::new(),
            render_pipeline,
            render_bind_group_layout,
            sampler,
        }
    }

    /// Upload pre-computed spectrogram magnitude data as a GPU texture
    pub fn upload_spectrogram(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool_index: usize,
        upload: &SpectrogramUpload,
    ) {
        // Remove old entry
        self.entries.remove(&pool_index);

        if upload.time_bins == 0 || upload.freq_bins == 0 {
            return;
        }

        // Data layout: magnitudes[t * freq_bins + f] — each row is one time slice
        // with freq_bins values. So texture width = freq_bins, height = time_bins.
        // R8Unorm is filterable (unlike R32Float) for bilinear interpolation.
        let tex_width = upload.freq_bins;
        let tex_height = upload.time_bins;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("spectrogram_{}", pool_index)),
            size: wgpu::Extent3d {
                width: tex_width,
                height: tex_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Convert f32 magnitudes to u8 for R8Unorm, with row padding for alignment.
        // wgpu requires bytes_per_row to be a multiple of COPY_BYTES_PER_ROW_ALIGNMENT (256).
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let unpadded_row = tex_width; // 1 byte per texel for R8Unorm
        let padded_row = (unpadded_row + align - 1) / align * align;

        let mut texel_data = vec![0u8; padded_row as usize * tex_height as usize];
        for row in 0..tex_height as usize {
            let src_offset = row * tex_width as usize;
            let dst_offset = row * padded_row as usize;
            for col in 0..tex_width as usize {
                let m = upload.magnitudes[src_offset + col];
                texel_data[dst_offset + col] = (m.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }

        // Upload magnitude data
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texel_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(tex_height),
            },
            wgpu::Extent3d {
                width: tex_width,
                height: tex_height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("spectrogram_{}_view", pool_index)),
            ..Default::default()
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(&format!("spectrogram_{}_uniforms", pool_index)),
            size: std::mem::size_of::<SpectrogramParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&format!("spectrogram_{}_bg", pool_index)),
            layout: &self.render_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        self.entries.insert(
            pool_index,
            SpectrogramGpuEntry {
                texture,
                texture_view,
                render_bind_group,
                uniform_buffer,
                time_bins: upload.time_bins,
                freq_bins: upload.freq_bins,
                sample_rate: upload.sample_rate,
                hop_size: upload.hop_size,
                fft_size: upload.fft_size,
                duration: upload.duration,
            },
        );
    }
}

impl egui_wgpu::CallbackTrait for SpectrogramCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Initialize global resources on first use
        if !resources.contains::<SpectrogramGpuResources>() {
            resources.insert(SpectrogramGpuResources::new(device, self.target_format));
        }

        let gpu: &mut SpectrogramGpuResources = resources.get_mut().unwrap();

        // Handle pending upload
        if let Some(ref upload) = self.pending_upload {
            gpu.upload_spectrogram(device, queue, self.pool_index, upload);
        }

        // Update uniform buffer
        if let Some(entry) = gpu.entries.get(&self.pool_index) {
            queue.write_buffer(
                &entry.uniform_buffer,
                0,
                bytemuck::cast_slice(&[self.params]),
            );
        }

        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let gpu: &SpectrogramGpuResources = match resources.get() {
            Some(r) => r,
            None => return,
        };

        let entry = match gpu.entries.get(&self.pool_index) {
            Some(e) => e,
            None => return,
        };

        render_pass.set_pipeline(&gpu.render_pipeline);
        render_pass.set_bind_group(0, &entry.render_bind_group, &[]);
        render_pass.draw(0..3, 0..1); // Fullscreen triangle
    }
}
