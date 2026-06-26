//! NV12 → linear-RGB blit: composites a hardware-decoded video frame (two wgpu plane textures,
//! Y = R8Unorm + CbCr = Rg8Unorm) directly into the Rgba16Float HDR layer, with no CPU upload.
//! The colour math mirrors the software path (BT.709 → sRGB-encoded → linear) so hardware- and
//! software-decoded video look identical. See `panes/shaders/nv12_blit.wgsl`.

use crate::gpu_brush::BlitTransform;
use lightningbeam_core::video::{VideoPrimaries, VideoTransfer};

/// Uniform: the `viewport_uv → frame_uv` affine (same packing as [`BlitTransform`]), the Y'CbCr→RGB
/// matrix coefficients, and a flags vec4. 80 bytes (48 matrix + 16 coeffs + 16 flags).
/// `flags`: `[full_range, transfer (0 gamma / 1 PQ / 2 HLG), primaries (0 BT.709 / 1 BT.2020), pad]`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Nv12Params {
    transform: BlitTransform,
    coeffs: [f32; 4],
    flags: [u32; 4],
}

pub struct Nv12BlitPipeline {
    pipeline: wgpu::RenderPipeline,
    bg_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Nv12BlitPipeline {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("panes/shaders/nv12_blit.wgsl").into(),
            ),
        });

        let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12_blit_bgl"),
            entries: &[
                tex_entry(0), // Y plane (R8Unorm)
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
                tex_entry(3), // CbCr plane (Rg8Unorm)
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12_blit_pl"),
            bind_group_layouts: &[&bg_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("nv12_blit_pipeline"),
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
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Bilinear: the frame is scaled to the output size; nearest would look blocky.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("nv12_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self { pipeline, bg_layout, sampler }
    }

    /// Convert + blit the NV12 frame into `target_view` (Rgba16Float, cleared to transparent),
    /// positioned by `transform` (built like the RGBA video path's `BlitTransform`).
    #[allow(clippy::too_many_arguments)]
    pub fn blit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        y_view: &wgpu::TextureView,
        uv_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        transform: &BlitTransform,
        full_range: bool,
        coeffs: [f32; 4],
        transfer: VideoTransfer,
        primaries: VideoPrimaries,
    ) {
        let transfer_code = match transfer {
            VideoTransfer::Gamma => 0,
            VideoTransfer::Pq => 1,
            VideoTransfer::Hlg => 2,
        };
        let primaries_code = match primaries {
            VideoPrimaries::Bt709 => 0,
            VideoPrimaries::Bt2020 => 1,
        };
        let params = Nv12Params {
            transform: *transform,
            coeffs,
            flags: [full_range as u32, transfer_code, primaries_code, 0],
        };
        let param_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("nv12_blit_params"),
            size: std::mem::size_of::<Nv12Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&param_buf, 0, bytemuck::bytes_of(&params));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12_blit_bg"),
            layout: &self.bg_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(y_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: param_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(uv_view) },
            ],
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("nv12_blit_encoder"),
        });
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("nv12_blit_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..4, 0..1);
        }
        queue.submit(Some(encoder.finish()));
    }
}
