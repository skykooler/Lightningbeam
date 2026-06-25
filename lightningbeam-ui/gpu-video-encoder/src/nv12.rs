//! GPU RGBA→NV12 conversion (BT.709 full-range), the pixel format hardware video
//! encoders (VAAPI/QSV/NVENC/VideoToolbox) consume.
//!
//! NV12 layout (what this writes, tight-packed into a storage buffer):
//! - `[0, W*H)`            Y plane, one byte/pixel, row stride `W`
//! - `[W*H, W*H + W*H/2)`  UV plane, interleaved `U,V` at 4:2:0, row stride `W`
//!   (`W/2` chroma columns × 2 bytes), `H/2` rows
//!
//! Same BT.709 full-range matrix as the editor's planar YUV420p path, so colors match.
//! Requires `W % 8 == 0 && H % 2 == 0` (the shader packs 4 bytes per `u32`).

/// `true` when [`Nv12Converter`] can handle these dimensions (else caller pads/falls back).
pub fn supports(width: u32, height: u32) -> bool {
    width % 8 == 0 && height % 2 == 0 && width > 0 && height > 0
}

/// Tight NV12 byte length for `width`×`height`.
pub fn nv12_len(width: u32, height: u32) -> usize {
    (width * height + width * (height / 2)) as usize
}

/// Compute pipeline: `Rgba8Unorm` texture → tight NV12 storage buffer.
pub struct Nv12Converter {
    y_pipeline: wgpu::ComputePipeline,
    uv_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl Nv12Converter {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("nv12_bgl"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("nv12_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("nv12_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let mk = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("nv12_pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        Self {
            y_pipeline: mk("y_main"),
            uv_pipeline: mk("uv_main"),
            bind_group_layout,
        }
    }

    /// Record RGBA→NV12 into `encoder`. `out_buffer` must be `STORAGE | COPY_SRC` of at
    /// least [`nv12_len`] bytes. Caller must ensure [`supports`]`(width, height)`.
    pub fn convert(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        rgba_view: &wgpu::TextureView,
        out_buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) {
        debug_assert!(supports(width, height));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("nv12_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(rgba_view) },
                wgpu::BindGroupEntry { binding: 1, resource: out_buffer.as_entire_binding() },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("nv12_pass"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);
        let wg = 8u32;
        // Y: one thread per 4 horizontal luma samples.
        pass.set_pipeline(&self.y_pipeline);
        pass.dispatch_workgroups(((width / 4) + wg - 1) / wg, (height + wg - 1) / wg, 1);
        // UV: one thread per 4 interleaved UV bytes = 2 chroma columns; (W/4)×(H/2) threads.
        pass.set_pipeline(&self.uv_pipeline);
        pass.dispatch_workgroups(((width / 4) + wg - 1) / wg, ((height / 2) + wg - 1) / wg, 1);
    }
}

/// CPU reference producing the exact bytes the shader should — used by tests to verify
/// the GPU output on real hardware.
pub fn cpu_reference(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut out = vec![0u8; nv12_len(width, height)];
    let to_byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let px = |x: usize, y: usize| {
        let i = (y * w + x) * 4;
        [rgba[i] as f32 / 255.0, rgba[i + 1] as f32 / 255.0, rgba[i + 2] as f32 / 255.0]
    };
    // Y
    for y in 0..h {
        for x in 0..w {
            let p = px(x, y);
            out[y * w + x] = to_byte(0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]);
        }
    }
    // Interleaved UV (2x2 box average)
    let y_size = w * h;
    for cy in 0..h / 2 {
        for cx in 0..w / 2 {
            let mut acc = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(2 * cx + dx, 2 * cy + dy);
                acc[0] += p[0]; acc[1] += p[1]; acc[2] += p[2];
            }
            let a = [acc[0] / 4.0, acc[1] / 4.0, acc[2] / 4.0];
            let u = -0.1146 * a[0] - 0.3854 * a[1] + 0.5000 * a[2] + 0.5;
            let v = 0.5000 * a[0] - 0.4542 * a[1] - 0.0458 * a[2] + 0.5;
            out[y_size + cy * w + 2 * cx] = to_byte(u);
            out[y_size + cy * w + 2 * cx + 1] = to_byte(v);
        }
    }
    out
}

const SHADER: &str = r#"
@group(0) @binding(0) var input_rgba: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> out_buf: array<u32>;

fn to_byte(v: f32) -> u32 { return u32(clamp(v, 0.0, 1.0) * 255.0 + 0.5); }

// Y plane: pack 4 horizontal luma bytes.
@compute @workgroup_size(8, 8, 1)
fn y_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input_rgba);
    let w = dims.x;
    let h = dims.y;
    let x4 = gid.x * 4u;
    let y = gid.y;
    if (x4 >= w || y >= h) { return; }
    var packed: u32 = 0u;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let c = textureLoad(input_rgba, vec2<u32>(x4 + i, y), 0).rgb;
        let yy = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
        packed = packed | (to_byte(yy) << (8u * i));
    }
    out_buf[(y * w + x4) / 4u] = packed;
}

// UV plane: each thread writes 4 interleaved bytes = U0 V0 U1 V1 for 2 chroma columns.
@compute @workgroup_size(8, 8, 1)
fn uv_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input_rgba);
    let w = dims.x;
    let h = dims.y;
    let k = gid.x;          // chroma-column pair index: covers columns 2k, 2k+1
    let cy = gid.y;
    if (k * 2u >= w / 2u || cy >= h / 2u) { return; }
    let y_size = w * h;

    var packed: u32 = 0u;
    for (var j = 0u; j < 2u; j = j + 1u) {
        let cx = 2u * k + j;            // chroma column
        let sx = 2u * cx;
        let sy = 2u * cy;
        let p00 = textureLoad(input_rgba, vec2<u32>(sx, sy), 0).rgb;
        let p10 = textureLoad(input_rgba, vec2<u32>(sx + 1u, sy), 0).rgb;
        let p01 = textureLoad(input_rgba, vec2<u32>(sx, sy + 1u), 0).rgb;
        let p11 = textureLoad(input_rgba, vec2<u32>(sx + 1u, sy + 1u), 0).rgb;
        let a = (p00 + p10 + p01 + p11) * 0.25;
        let u = -0.1146 * a.r - 0.3854 * a.g + 0.5000 * a.b + 0.5;
        let v =  0.5000 * a.r - 0.4542 * a.g - 0.0458 * a.b + 0.5;
        packed = packed | (to_byte(u) << (16u * j));        // byte 0 or 2
        packed = packed | (to_byte(v) << (16u * j + 8u));   // byte 1 or 3
    }
    // UV row stride is w bytes; this thread writes 4 bytes at column 4k.
    out_buf[(y_size + cy * w + 4u * k) / 4u] = packed;
}
"#;
