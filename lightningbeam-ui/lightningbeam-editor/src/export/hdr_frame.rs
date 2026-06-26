//! 10-bit HDR frame production for video export (isolated from the SDR readback pipeline).
//!
//! Takes the compositor's Rgba16Float HDR accumulator and produces YUV420P10LE planes:
//!   1. GPU pass `linear_to_pq.wgsl` → PQ/HLG-encoded BT.2020 R'G'B' into an Rgba16Unorm texture
//!      (the expensive per-pixel transfer + gamut work).
//!   2. Synchronous GPU→CPU readback of that texture.
//!   3. CPU BT.2020 R'G'B'→Y'CbCr (limited range), 4:2:0 average, 10-bit little-endian pack.
//!
//! Synchronous (no triple-buffering); HDR export favors correctness/simplicity over throughput.

use lightningbeam_core::export::HdrExportMode;

/// Round up to the wgpu copy row alignment (256 bytes).
fn align_256(n: u32) -> u32 {
    (n + 255) & !255
}

pub struct HdrFramePipeline {
    width: u32,
    height: u32,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    mode_buf: wgpu::Buffer,
    /// PQ/HLG-encoded BT.2020 R'G'B' (Rgba16Unorm) render target.
    enc_texture_view: wgpu::TextureView,
    enc_texture: wgpu::Texture,
    /// Staging buffer for readback; rows padded to 256-byte alignment.
    staging: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

impl HdrFramePipeline {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("linear_to_pq_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/linear_to_pq.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("linear_to_pq_bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("linear_to_pq_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("linear_to_pq_pipeline"),
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
                    // Rgba16Float (not Unorm) so no TEXTURE_FORMAT_16BIT_NORM feature is needed; PQ/HLG
                    // values are in [0,1] where f16 has ~11 effective bits — ample for 10-bit output.
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear_to_pq_sampler"),
            ..Default::default()
        });

        let mode_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("linear_to_pq_mode"),
            size: 16, // vec4<u32>
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let enc_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr_enc_texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let enc_texture_view = enc_texture.create_view(&Default::default());

        let padded_bytes_per_row = align_256(width * 8); // Rgba16Unorm = 8 bytes/texel
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("hdr_enc_staging"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            width,
            height,
            pipeline,
            bind_group_layout,
            sampler,
            mode_buf,
            enc_texture_view,
            enc_texture,
            staging,
            padded_bytes_per_row,
        }
    }

    /// Encode the composited HDR texture (`hdr_view`, Rgba16Float linear) to YUV420P10LE planes.
    pub fn render_to_yuv10(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        hdr_view: &wgpu::TextureView,
        mode: HdrExportMode,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mode_code: u32 = if matches!(mode, HdrExportMode::Hlg) { 1 } else { 0 };
        queue.write_buffer(&self.mode_buf, 0, bytemuck::cast_slice(&[mode_code, 0u32, 0, 0]));

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("linear_to_pq_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(hdr_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: self.mode_buf.as_entire_binding() },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("hdr_frame_encoder"),
        });
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("linear_to_pq_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.enc_texture_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bind_group, &[]);
            rp.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.enc_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d { width: self.width, height: self.height, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));

        // Synchronous map + wait.
        let slice = self.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        let _ = rx.recv();

        let w = self.width as usize;
        let h = self.height as usize;
        let mapped = slice.get_mapped_range();
        // Un-pad rows; decode f16 → f32 into a tight RGBA buffer.
        let mut rgba = vec![0f32; w * h * 4];
        let row_bytes = w * 8;
        for row in 0..h {
            let src = row * self.padded_bytes_per_row as usize;
            let dst = row * w * 4;
            let bytes = &mapped[src..src + row_bytes];
            for px in 0..w * 4 {
                let half = u16::from_le_bytes([bytes[px * 2], bytes[px * 2 + 1]]);
                rgba[dst + px] = f16_to_f32(half);
            }
        }
        drop(mapped);
        self.staging.unmap();

        rgba_to_yuv420p10le(&rgba, w, h)
    }
}

/// Decode an IEEE 754 half-float. Inputs are in [0,1] so the inf/NaN paths don't occur in practice.
fn f16_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) & 1;
    let exp = (h >> 10) & 0x1f;
    let mant = h & 0x3ff;
    let v = if exp == 0 {
        (mant as f32) * 2f32.powi(-24) // subnormal
    } else if exp == 31 {
        if mant == 0 { f32::INFINITY } else { f32::NAN }
    } else {
        (1.0 + mant as f32 / 1024.0) * 2f32.powi(exp as i32 - 15)
    };
    if sign == 1 { -v } else { v }
}

/// BT.2020 non-constant-luminance R'G'B'→Y'CbCr, limited range, 4:2:0, 10-bit little-endian.
/// Input R'G'B' is already gamma-encoded (PQ/HLG) in [0,1].
fn rgba_to_yuv420p10le(rgba: &[f32], w: usize, h: usize) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    const KR: f32 = 0.2627;
    const KB: f32 = 0.0593;
    let kg = 1.0 - KR - KB;

    let luma = |r: f32, g: f32, b: f32| KR * r + kg * g + KB * b;
    // 10-bit limited: Y' [64,940] (scale 876), Cb/Cr center 512, excursion ±0.5 → scale 896.
    let pack_y = |y: f32| ((y * 876.0 + 64.0).round().clamp(0.0, 1023.0)) as u16;
    let pack_c = |c: f32| ((c * 896.0 + 512.0).round().clamp(0.0, 1023.0)) as u16;

    let mut y_plane = vec![0u8; w * h * 2];
    for j in 0..h {
        for i in 0..w {
            let p = (j * w + i) * 4;
            let y10 = pack_y(luma(rgba[p], rgba[p + 1], rgba[p + 2]));
            let o = (j * w + i) * 2;
            y_plane[o] = (y10 & 0xff) as u8;
            y_plane[o + 1] = (y10 >> 8) as u8;
        }
    }

    let (cw, ch) = (w / 2, h / 2);
    let mut u_plane = vec![0u8; cw * ch * 2];
    let mut v_plane = vec![0u8; cw * ch * 2];
    for j in 0..ch {
        for i in 0..cw {
            let (mut cb, mut cr) = (0.0f32, 0.0f32);
            for dy in 0..2 {
                for dx in 0..2 {
                    let p = ((j * 2 + dy) * w + (i * 2 + dx)) * 4;
                    let (r, g, b) = (rgba[p], rgba[p + 1], rgba[p + 2]);
                    let yy = luma(r, g, b);
                    cb += (b - yy) / (2.0 * (1.0 - KB));
                    cr += (r - yy) / (2.0 * (1.0 - KR));
                }
            }
            let cb10 = pack_c(cb / 4.0);
            let cr10 = pack_c(cr / 4.0);
            let o = (j * cw + i) * 2;
            u_plane[o] = (cb10 & 0xff) as u8;
            u_plane[o + 1] = (cb10 >> 8) as u8;
            v_plane[o] = (cr10 & 0xff) as u8;
            v_plane[o + 1] = (cr10 >> 8) as u8;
        }
    }

    (y_plane, u_plane, v_plane)
}
