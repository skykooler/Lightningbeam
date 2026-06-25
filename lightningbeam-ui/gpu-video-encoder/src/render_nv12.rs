//! Fragment-shader RGBA→NV12 conversion that **renders** luma/chroma into the encoder
//! surface's plane textures (R8 Y, RG8 UV). Render targets (not compute storage) so it
//! works with the DMA-BUF-imported plane images, which aren't storage-writable.
//!
//! BT.709 full-range, matching `nv12::cpu_reference` and the encoder's color tags.

/// Converts a bound RGBA texture into a Y plane (R8) and a UV plane (RG8) via two passes.
pub struct Rgba2Nv12 {
    y_pipeline: wgpu::RenderPipeline,
    uv_pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
}

impl Rgba2Nv12 {
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rgba2nv12_bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rgba2nv12_pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rgba2nv12_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let mk = |fs: &str, fmt: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rgba2nv12_pipeline"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(fmt.into())],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        Self {
            y_pipeline: mk("y_fs", wgpu::TextureFormat::R8Unorm),
            uv_pipeline: mk("uv_fs", wgpu::TextureFormat::Rg8Unorm),
            bgl,
        }
    }

    /// Record both plane passes. `y_view`/`uv_view` are the R8/RG8 plane render targets.
    pub fn convert(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        rgba_view: &wgpu::TextureView,
        y_view: &wgpu::TextureView,
        uv_view: &wgpu::TextureView,
    ) {
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rgba2nv12_bg"),
            layout: &self.bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(rgba_view),
            }],
        });
        for (pipeline, view) in [(&self.y_pipeline, y_view), (&self.uv_pipeline, uv_view)] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("rgba2nv12_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

const SHADER: &str = r#"
@group(0) @binding(0) var input_rgba: texture_2d<f32>;

// Fullscreen triangle.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn load(p: vec2<i32>) -> vec3<f32> {
    return textureLoad(input_rgba, p, 0).rgb;
}

// Y plane (full res): one luma byte per pixel.
@fragment
fn y_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let c = load(vec2<i32>(i32(pos.x), i32(pos.y)));
    let y = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    return vec4<f32>(y, 0.0, 0.0, 1.0);
}

// UV plane (half res): 2x2 box-averaged chroma, interleaved into RG.
@fragment
fn uv_fs(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let sx = 2 * i32(pos.x);
    let sy = 2 * i32(pos.y);
    let a = (load(vec2<i32>(sx, sy)) + load(vec2<i32>(sx + 1, sy))
           + load(vec2<i32>(sx, sy + 1)) + load(vec2<i32>(sx + 1, sy + 1))) * 0.25;
    let u = -0.1146 * a.r - 0.3854 * a.g + 0.5000 * a.b + 0.5;
    let v =  0.5000 * a.r - 0.4542 * a.g - 0.0458 * a.b + 0.5;
    return vec4<f32>(u, v, 0.0, 1.0);
}
"#;
