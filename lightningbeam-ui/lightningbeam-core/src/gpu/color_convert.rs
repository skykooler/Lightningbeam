//! Color space conversion pipelines for GPU rendering
//!
//! Provides sRGB ↔ linear color space conversion passes for the HDR compositing pipeline.
//! These are used to convert Vello's sRGB output to linear HDR for compositing,
//! and to convert the final HDR result back to sRGB for display.

use super::HDR_FORMAT;

/// Shared WGSL sRGB transfer functions — the single source of the sRGB OETF/EOTF
/// used by every gamma-aware shader. Prepend it to a shader's source (it defines
/// the functions before the body, so call order doesn't matter):
/// `srgb_to_linear_channel` / `linear_to_srgb_channel` (scalar) and
/// `srgb_to_linear` / `linear_to_srgb` (vec3). `linear_to_srgb_channel` clamps to
/// [0,1] (its outputs target 8-bit / SDR display surfaces).
pub const COLOR_WGSL: &str = r#"
fn srgb_to_linear_channel(c: f32) -> f32 {
    return select(pow((c + 0.055) / 1.055, 2.4), c / 12.92, c <= 0.04045);
}
fn linear_to_srgb_channel(c: f32) -> f32 {
    let x = clamp(c, 0.0, 1.0);
    return select(1.055 * pow(x, 1.0 / 2.4) - 0.055, x * 12.92, x <= 0.0031308);
}
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(srgb_to_linear_channel(c.r), srgb_to_linear_channel(c.g), srgb_to_linear_channel(c.b));
}
fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(linear_to_srgb_channel(c.r), linear_to_srgb_channel(c.g), linear_to_srgb_channel(c.b));
}
"#;

/// sRGB → linear for one channel in `[0, 1]` (CPU twin of the WGSL
/// `srgb_to_linear_channel`). The single source of the EOTF for CPU code.
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// linear → sRGB for one channel, clamped to `[0, 1]` (CPU twin of the WGSL
/// `linear_to_srgb_channel`). The single source of the OETF for CPU code.
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// GPU pipeline for sRGB to linear color space conversion
///
/// Converts Rgba8Srgb textures to Rgba16Float linear textures.
/// Used after Vello rendering to prepare layers for HDR compositing.
pub struct SrgbToLinearConverter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl SrgbToLinearConverter {
    /// Create a new sRGB to linear converter
    pub fn new(device: &wgpu::Device) -> Self {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("srgb_to_linear_bind_group_layout"),
            entries: &[
                // Source sRGB texture
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
                // Sampler
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
            label: Some("srgb_to_linear_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("srgb_to_linear_shader"),
            source: wgpu::ShaderSource::Wgsl(SRGB_TO_LINEAR_SHADER.into()),
        });

        // Create render pipeline - outputs to HDR format
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("srgb_to_linear_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT,
                    blend: None, // No blending - direct write
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("srgb_to_linear_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Convert an sRGB texture to linear HDR
    ///
    /// Reads from `source_view` (sRGB) and writes to `dest_view` (HDR linear).
    pub fn convert(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        dest_view: &wgpu::TextureView,
    ) {
        // Create bind group for this conversion
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("srgb_to_linear_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // Render pass
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("srgb_to_linear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dest_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }
}

/// WGSL shader for sRGB to linear conversion
const SRGB_TO_LINEAR_SHADER: &str = r#"
// sRGB to Linear color space conversion shader

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle strip
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);

    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);

    return out;
}

// sRGB to linear color space conversion (per channel)
fn srgb_to_linear_channel(c: f32) -> f32 {
    return select(
        pow((c + 0.055) / 1.055, 2.4),
        c / 12.92,
        c <= 0.04045
    );
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        srgb_to_linear_channel(color.r),
        srgb_to_linear_channel(color.g),
        srgb_to_linear_channel(color.b)
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_tex, source_sampler, in.uv);

    // Convert sRGB to linear
    let linear_rgb = srgb_to_linear(src.rgb);

    // Alpha stays unchanged
    return vec4<f32>(linear_rgb, src.a);
}
"#;
