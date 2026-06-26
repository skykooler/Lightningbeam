// NV12 → linear-RGB blit shader.
//
// Samples a hardware-decoded video frame stored as two planes — Y (R8Unorm) and
// interleaved CbCr (Rg8Unorm, half-res) — converts BT.709 Y'CbCr → gamma-encoded
// R'G'B', then sRGB→linear, and writes straight-alpha linear into the Rgba16Float
// HDR layer. This mirrors the software path (swscale → sRGB RGBA8 → sampled as
// Rgba8UnormSrgb → linear) so hardware- and software-decoded video match.
//
// The affine transform (viewport UV → frame UV) is the same packing as
// canvas_blit.wgsl's BlitTransform; `full_range` selects full vs. studio-swing
// de-quantization.

struct Nv12Params {
    col0: vec4<f32>,
    col1: vec4<f32>,
    col2: vec4<f32>,
    // Y'CbCr→R'G'B' matrix from the source colorspace: [Cr→R, Cb→G, Cr→G, Cb→B].
    coeffs: vec4<f32>,
    // .x = full_range flag; .yzw padding. A vec4 keeps each block 16-aligned and the struct size
    // matching the Rust `[f32;4] + u32 + [u32;3]` (80 bytes).
    flags: vec4<u32>,
}

@group(0) @binding(0) var y_tex:        texture_2d<f32>;
@group(0) @binding(1) var samp:         sampler;
@group(0) @binding(2) var<uniform> params: Nv12Params;
@group(0) @binding(3) var uv_tex:       texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(lo, hi, c > vec3<f32>(0.04045));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let m = mat3x3<f32>(params.col0.xyz, params.col1.xyz, params.col2.xyz);
    let frame_uv = (m * vec3<f32>(in.uv.x, in.uv.y, 1.0)).xy;

    if frame_uv.x < 0.0 || frame_uv.x > 1.0
    || frame_uv.y < 0.0 || frame_uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let yv = textureSample(y_tex, samp, frame_uv).r;
    let cbcr = textureSample(uv_tex, samp, frame_uv).rg;

    var Y: f32;
    var Cb: f32;
    var Cr: f32;
    if params.flags.x != 0u {
        // Full ("JPEG") range: [0,255] luma, chroma centered at 128.
        Y = yv;
        Cb = cbcr.r - 0.5;
        Cr = cbcr.g - 0.5;
    } else {
        // Studio swing: Y'∈[16,235], Cb/Cr∈[16,240].
        Y = (yv * 255.0 - 16.0) / 219.0;
        Cb = (cbcr.r * 255.0 - 128.0) / 224.0;
        Cr = (cbcr.g * 255.0 - 128.0) / 224.0;
    }

    // Y'CbCr → gamma-encoded R'G'B' using the source colorspace's matrix.
    let r = Y + params.coeffs.x * Cr;
    let g = Y + params.coeffs.y * Cb + params.coeffs.z * Cr;
    let b = Y + params.coeffs.w * Cb;
    let rgb_gamma = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));

    // R'G'B' is gamma-encoded; the HDR target is linear → undo the transfer.
    let rgb_lin = srgb_to_linear(rgb_gamma);
    return vec4<f32>(rgb_lin, 1.0);
}
