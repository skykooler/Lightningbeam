//! Tight GPU RGBA→YUV420p converter for video export.
//!
//! Unlike [`lightningbeam_core::gpu::YuvConverter`] (which writes one byte per
//! `Rgba8Unorm` texel — a 4× readback), this writes **packed planar YUV420p** into a
//! storage buffer, so the readback is exactly `W*H*3/2` bytes (~3.1 MB at 1080p vs
//! 8.3 MB RGBA) and — more importantly — the per-frame CPU `rgba_to_yuv420p` (swscale)
//! is eliminated.
//!
//! Color math is BT.709 **full-range** (JPEG range), matching the encoder color tags
//! set in `setup_video_encoder` (`Space::BT709` + `Range::JPEG`).
//!
//! Output buffer layout (tight, little-endian byte packing into `array<u32>`):
//! - `[0, W*H)`            Y plane, row stride `W`
//! - `[W*H, W*H + CW*CH)`  U plane, row stride `CW` (`CW=W/2`, `CH=H/2`)
//! - `[W*H+CW*CH, end)`    V plane, row stride `CW`
//!
//! Dimension requirement: `W % 8 == 0 && H % 2 == 0` (so `W/4` and `CW/4` are whole —
//! the shader packs 4 bytes per `u32`). [`GpuYuv::supports`] reports this; callers
//! fall back to the CPU converter otherwise.

/// `true` when [`GpuYuv`] can convert these dimensions (else use the CPU path).
pub fn supports(width: u32, height: u32) -> bool {
    width % 8 == 0 && height % 2 == 0 && width > 0 && height > 0
}

/// Tight planar YUV420p byte length for `width`×`height`.
pub fn yuv420p_len(width: u32, height: u32) -> usize {
    let y = (width * height) as usize;
    let c = ((width / 2) * (height / 2)) as usize;
    y + 2 * c
}

/// GPU compute pipeline: `Rgba8Unorm` texture → tight planar YUV420p storage buffer.
pub struct GpuYuv {
    y_pipeline: wgpu::ComputePipeline,
    uv_pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl GpuYuv {
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu_yuv_bgl"),
            entries: &[
                // 0: input RGBA (non-filterable, read via textureLoad)
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
                // 1: output packed YUV (read_write so 4-byte packing writes whole u32s)
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
            label: Some("gpu_yuv_pl"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_yuv_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let mk = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("gpu_yuv_pipeline"),
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

    /// Record the RGBA→YUV420p conversion into `encoder`.
    ///
    /// `rgba_view` is the rendered frame (`Rgba8Unorm`, `width`×`height`, must have
    /// `TEXTURE_BINDING` usage). `yuv_buffer` must be a `STORAGE | COPY_SRC` buffer of
    /// at least [`yuv420p_len`] bytes (rounded up to 4). Caller must ensure
    /// [`supports`]`(width, height)`.
    pub fn convert(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        rgba_view: &wgpu::TextureView,
        yuv_buffer: &wgpu::Buffer,
        width: u32,
        height: u32,
    ) {
        debug_assert!(supports(width, height));
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_yuv_bg"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(rgba_view) },
                wgpu::BindGroupEntry { binding: 1, resource: yuv_buffer.as_entire_binding() },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu_yuv_pass"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);

        // Y: one thread per 4 horizontal luma samples → (W/4)×H threads.
        pass.set_pipeline(&self.y_pipeline);
        let wg = 8u32;
        pass.dispatch_workgroups(((width / 4) + wg - 1) / wg, (height + wg - 1) / wg, 1);

        // UV: one thread per 4 horizontal chroma samples → (CW/4)×CH = (W/8)×(H/2) threads.
        pass.set_pipeline(&self.uv_pipeline);
        let cw = width / 2;
        let ch = height / 2;
        pass.dispatch_workgroups(((cw / 4) + wg - 1) / wg, (ch + wg - 1) / wg, 1);
    }
}

/// CPU reference for the exact math/layout the shader produces — used by unit tests so
/// the packing and BT.709 coefficients stay verifiable without a GPU.
#[cfg(test)]
fn cpu_reference(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let cw = w / 2;
    let ch = h / 2;
    let mut out = vec![0u8; yuv420p_len(width, height)];
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
    // U/V (2x2 average)
    let y_size = w * h;
    let uv_size = cw * ch;
    for cy in 0..ch {
        for cx in 0..cw {
            let mut acc = [0.0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px(2 * cx + dx, 2 * cy + dy);
                acc[0] += p[0]; acc[1] += p[1]; acc[2] += p[2];
            }
            let a = [acc[0] / 4.0, acc[1] / 4.0, acc[2] / 4.0];
            let u = -0.1146 * a[0] - 0.3854 * a[1] + 0.5000 * a[2] + 0.5;
            let v = 0.5000 * a[0] - 0.4542 * a[1] - 0.0458 * a[2] + 0.5;
            out[y_size + cy * cw + cx] = to_byte(u);
            out[y_size + uv_size + cy * cw + cx] = to_byte(v);
        }
    }
    out
}

const SHADER: &str = r#"
// RGBA -> tight planar YUV420p (BT.709 full-range), packed 4 bytes/u32.
@group(0) @binding(0) var input_rgba: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> out_buf: array<u32>;

fn to_byte(v: f32) -> u32 { return u32(clamp(v, 0.0, 1.0) * 255.0 + 0.5); }

// Y plane: each thread packs 4 horizontal luma bytes.
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

// U/V planes: each thread packs 4 horizontal chroma bytes (2x2 box-averaged).
@compute @workgroup_size(8, 8, 1)
fn uv_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input_rgba);
    let w = dims.x;
    let h = dims.y;
    let cw = w / 2u;
    let ch = h / 2u;
    let cx4 = gid.x * 4u;
    let cy = gid.y;
    if (cx4 >= cw || cy >= ch) { return; }
    let y_size = w * h;
    let uv_size = cw * ch;
    var up: u32 = 0u;
    var vp: u32 = 0u;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let cx = cx4 + i;
        let sx = 2u * cx;
        let sy = 2u * cy;
        let p00 = textureLoad(input_rgba, vec2<u32>(sx, sy), 0).rgb;
        let p10 = textureLoad(input_rgba, vec2<u32>(sx + 1u, sy), 0).rgb;
        let p01 = textureLoad(input_rgba, vec2<u32>(sx, sy + 1u), 0).rgb;
        let p11 = textureLoad(input_rgba, vec2<u32>(sx + 1u, sy + 1u), 0).rgb;
        let a = (p00 + p10 + p01 + p11) * 0.25;
        let u = -0.1146 * a.r - 0.3854 * a.g + 0.5000 * a.b + 0.5;
        let v =  0.5000 * a.r - 0.4542 * a.g - 0.0458 * a.b + 0.5;
        up = up | (to_byte(u) << (8u * i));
        vp = vp | (to_byte(v) << (8u * i));
    }
    out_buf[(y_size + cy * cw + cx4) / 4u] = up;
    out_buf[(y_size + uv_size + cy * cw + cx4) / 4u] = vp;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_dims() {
        assert!(supports(1920, 1080));
        assert!(supports(1280, 720));
        assert!(supports(8, 2));
        assert!(!supports(6, 2)); // width not %8
        assert!(!supports(8, 3)); // height odd
        assert!(!supports(0, 0));
    }

    #[test]
    fn len_matches() {
        assert_eq!(yuv420p_len(1920, 1080), 1920 * 1080 * 3 / 2);
        assert_eq!(yuv420p_len(8, 2), 8 * 2 + 2 * (4 * 1));
    }

    #[test]
    fn reference_known_colors() {
        // 8x2 solid white → Y≈255, U≈V≈128. Solid black → Y=0, U=V≈128.
        let white = vec![255u8; 8 * 2 * 4];
        let out = cpu_reference(&white, 8, 2);
        let (cw, ch) = (4usize, 1usize);
        let y_size = 8 * 2;
        for &y in &out[..y_size] { assert!(y >= 254, "white Y={y}"); }
        for &u in &out[y_size..y_size + cw * ch] { assert!((u as i32 - 128).abs() <= 1, "white U={u}"); }

        let black = vec![0u8; 8 * 2 * 4];
        let out = cpu_reference(&black, 8, 2);
        for &y in &out[..y_size] { assert_eq!(y, 0); }
        for &v in &out[y_size + cw * ch..] { assert!((v as i32 - 128).abs() <= 1, "black V={v}"); }
    }

    #[test]
    fn reference_red_bt709() {
        // Solid red (255,0,0): Y=0.2126*255≈54; V high, U low (full range).
        let red: Vec<u8> = (0..8 * 2).flat_map(|_| [255u8, 0, 0, 255]).collect();
        let out = cpu_reference(&red, 8, 2);
        assert!((out[0] as i32 - 54).abs() <= 1, "red Y={}", out[0]);
        let y_size = 8 * 2;
        let u = out[y_size];
        let v = out[y_size + 4];
        // U = -0.1146*1*255+128 ≈ 99 ; V = 0.5*255+128 → clamps to 255
        assert!((u as i32 - 99).abs() <= 2, "red U={u}");
        assert_eq!(v, 255, "red V={v}");
    }
}
