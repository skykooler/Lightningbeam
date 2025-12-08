// Compositor for blending layers with proper opacity
//
// Handles alpha-over compositing with per-layer opacity and blend modes.
// All processing is done in HDR (RGBA16Float) linear color space.

use super::buffer_pool::{BufferHandle, BufferPool};

/// Blend mode for layer compositing
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Standard alpha-over compositing (Porter-Duff "over")
    #[default]
    Normal,
    /// Additive blending (src + dst)
    Add,
    /// Multiply (src * dst)
    Multiply,
    /// Screen (1 - (1-src) * (1-dst))
    Screen,
    /// Overlay (multiply dark, screen light)
    Overlay,
    /// Soft light
    SoftLight,
    /// Hard light
    HardLight,
    /// Color dodge
    ColorDodge,
    /// Color burn
    ColorBurn,
    /// Darken (min)
    Darken,
    /// Lighten (max)
    Lighten,
    /// Difference (abs(src - dst))
    Difference,
    /// Exclusion
    Exclusion,
}

impl BlendMode {
    /// Get the blend mode index for shader uniform
    pub fn to_index(&self) -> u32 {
        match self {
            BlendMode::Normal => 0,
            BlendMode::Add => 1,
            BlendMode::Multiply => 2,
            BlendMode::Screen => 3,
            BlendMode::Overlay => 4,
            BlendMode::SoftLight => 5,
            BlendMode::HardLight => 6,
            BlendMode::ColorDodge => 7,
            BlendMode::ColorBurn => 8,
            BlendMode::Darken => 9,
            BlendMode::Lighten => 10,
            BlendMode::Difference => 11,
            BlendMode::Exclusion => 12,
        }
    }

    /// Get all available blend modes
    pub fn all() -> &'static [BlendMode] {
        &[
            BlendMode::Normal,
            BlendMode::Add,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::HardLight,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::Difference,
            BlendMode::Exclusion,
        ]
    }

    /// Get display name for UI
    pub fn display_name(&self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Add => "Add",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::SoftLight => "Soft Light",
            BlendMode::HardLight => "Hard Light",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
        }
    }
}

/// A layer to be composited
#[derive(Clone, Debug)]
pub struct CompositorLayer {
    /// Handle to the layer's rendered buffer
    pub buffer: BufferHandle,
    /// Layer opacity (0.0 to 1.0)
    pub opacity: f32,
    /// Blend mode for this layer
    pub blend_mode: BlendMode,
}

impl CompositorLayer {
    pub fn new(buffer: BufferHandle, opacity: f32, blend_mode: BlendMode) -> Self {
        Self {
            buffer,
            opacity: opacity.clamp(0.0, 1.0),
            blend_mode,
        }
    }

    pub fn normal(buffer: BufferHandle, opacity: f32) -> Self {
        Self::new(buffer, opacity, BlendMode::Normal)
    }
}

/// Uniform data for the composite shader
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CompositeUniforms {
    /// Layer opacity (0.0 to 1.0)
    pub opacity: f32,
    /// Blend mode index
    pub blend_mode: u32,
    /// Padding for alignment
    pub _padding: [u32; 2],
}

/// Compositor for blending layers
///
/// Handles the final compositing pass that combines all rendered layers
/// with proper opacity and blend modes.
pub struct Compositor {
    /// Render pipeline for compositing
    pipeline: wgpu::RenderPipeline,
    /// Bind group layout for layer textures
    bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for texture sampling
    sampler: wgpu::Sampler,
}

impl Compositor {
    /// Create a new compositor
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor_bind_group_layout"),
            entries: &[
                // Source layer texture
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
                // Uniforms
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

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("compositor_shader"),
            source: wgpu::ShaderSource::Wgsl(COMPOSITE_SHADER.into()),
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compositor_pipeline"),
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
                    format: output_format,
                    // Use premultiplied alpha blending for compositing
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
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
            label: Some("compositor_sampler"),
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

    /// Create a bind group for compositing a layer
    pub fn create_layer_bind_group(
        &self,
        device: &wgpu::Device,
        layer_view: &wgpu::TextureView,
        uniforms_buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor_layer_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(layer_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniforms_buffer.as_entire_binding(),
                },
            ],
        })
    }

    /// Composite layers onto the output texture
    ///
    /// Layers are composited in order (first layer is bottom, last is top).
    /// The output texture should be cleared before calling this method.
    pub fn composite(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        layers: &[CompositorLayer],
        buffer_pool: &BufferPool,
        output: &wgpu::TextureView,
        clear_color: Option<[f32; 4]>,
    ) {
        // Create uniforms buffer (reused for all layers)
        let uniforms_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("compositor_uniforms"),
            size: std::mem::size_of::<CompositeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        for (i, layer) in layers.iter().enumerate() {
            let Some(layer_view) = buffer_pool.get_view(layer.buffer) else {
                continue;
            };

            // Update uniforms
            let uniforms = CompositeUniforms {
                opacity: layer.opacity,
                blend_mode: layer.blend_mode.to_index(),
                _padding: [0, 0],
            };
            queue.write_buffer(&uniforms_buffer, 0, bytemuck::bytes_of(&uniforms));

            // Create bind group for this layer
            let bind_group = self.create_layer_bind_group(device, layer_view, &uniforms_buffer);

            // Determine load operation (clear on first layer if requested)
            let load_op = if i == 0 {
                if let Some(color) = clear_color {
                    wgpu::LoadOp::Clear(wgpu::Color {
                        r: color[0] as f64,
                        g: color[1] as f64,
                        b: color[2] as f64,
                        a: color[3] as f64,
                    })
                } else {
                    wgpu::LoadOp::Load
                }
            } else {
                wgpu::LoadOp::Load
            };

            // Render pass for this layer
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("composite_layer_{}", i)),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: load_op,
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

    /// Get the bind group layout (for external use)
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }
}

/// WGSL shader for layer compositing
const COMPOSITE_SHADER: &str = r#"
// Compositor shader - blends a source layer onto the destination with opacity and blend modes

struct Uniforms {
    opacity: f32,
    blend_mode: u32,
    _padding: vec2<u32>,
}

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

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

// Blend mode implementations
// NOTE: All inputs are expected to be in linear HDR color space.
// sRGB to linear conversion happens in a separate pass before compositing.
fn blend_normal(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return src;
}

fn blend_add(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return src + dst;
}

fn blend_multiply(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return src * dst;
}

fn blend_screen(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return 1.0 - (1.0 - src) * (1.0 - dst);
}

fn blend_overlay_channel(s: f32, d: f32) -> f32 {
    return select(
        1.0 - 2.0 * (1.0 - s) * (1.0 - d),
        2.0 * s * d,
        d < 0.5
    );
}

fn blend_overlay(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        blend_overlay_channel(src.r, dst.r),
        blend_overlay_channel(src.g, dst.g),
        blend_overlay_channel(src.b, dst.b)
    );
}

fn blend_soft_light_channel(s: f32, d: f32) -> f32 {
    return select(
        d - (1.0 - 2.0 * s) * d * (1.0 - d),
        d + (2.0 * s - 1.0) * (select(
            ((16.0 * d - 12.0) * d + 4.0) * d,
            sqrt(d),
            d > 0.25
        ) - d),
        s <= 0.5
    );
}

fn blend_soft_light(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        blend_soft_light_channel(src.r, dst.r),
        blend_soft_light_channel(src.g, dst.g),
        blend_soft_light_channel(src.b, dst.b)
    );
}

fn blend_hard_light(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    // Hard light is overlay with src and dst swapped
    return blend_overlay(dst, src);
}

fn blend_color_dodge(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return select(
        min(vec3<f32>(1.0), dst / (1.0 - src)),
        vec3<f32>(1.0),
        src.r >= 1.0 || src.g >= 1.0 || src.b >= 1.0
    );
}

fn blend_color_burn(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return select(
        1.0 - min(vec3<f32>(1.0), (1.0 - dst) / src),
        vec3<f32>(0.0),
        src.r <= 0.0 || src.g <= 0.0 || src.b <= 0.0
    );
}

fn blend_darken(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return min(src, dst);
}

fn blend_lighten(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return max(src, dst);
}

fn blend_difference(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return abs(src - dst);
}

fn blend_exclusion(src: vec3<f32>, dst: vec3<f32>) -> vec3<f32> {
    return src + dst - 2.0 * src * dst;
}

fn apply_blend(src: vec3<f32>, dst: vec3<f32>, mode: u32) -> vec3<f32> {
    switch (mode) {
        case 0u: { return blend_normal(src, dst); }      // Normal
        case 1u: { return blend_add(src, dst); }         // Add
        case 2u: { return blend_multiply(src, dst); }    // Multiply
        case 3u: { return blend_screen(src, dst); }      // Screen
        case 4u: { return blend_overlay(src, dst); }     // Overlay
        case 5u: { return blend_soft_light(src, dst); }  // Soft Light
        case 6u: { return blend_hard_light(src, dst); }  // Hard Light
        case 7u: { return blend_color_dodge(src, dst); } // Color Dodge
        case 8u: { return blend_color_burn(src, dst); }  // Color Burn
        case 9u: { return blend_darken(src, dst); }      // Darken
        case 10u: { return blend_lighten(src, dst); }    // Lighten
        case 11u: { return blend_difference(src, dst); } // Difference
        case 12u: { return blend_exclusion(src, dst); }  // Exclusion
        default: { return blend_normal(src, dst); }
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_tex, source_sampler, in.uv);

    // Input is already in linear HDR color space (converted in separate pass)
    // Apply opacity
    let src_alpha = src.a * uniforms.opacity;

    // Output premultiplied alpha in linear color space
    return vec4<f32>(src.rgb * src_alpha, src_alpha);
}
"#;
