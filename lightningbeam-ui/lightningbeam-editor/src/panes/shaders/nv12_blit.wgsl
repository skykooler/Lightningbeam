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
    // .x = full_range; .y = transfer (0 gamma, 1 PQ, 2 HLG); .z = primaries (0 BT.709, 1 BT.2020).
    // A vec4 keeps each block 16-aligned and the struct 80 bytes (Rust `[f32;4] + u32 + [u32;3]`).
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

// SMPTE ST 2084 (PQ) EOTF: encoded [0,1] → absolute luminance, then normalize so the 203-nit
// graphics white = 1.0 (HDR highlights exceed 1.0). Per-channel.
fn pq_to_linear(c: vec3<f32>) -> vec3<f32> {
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let e = pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / m2));
    let num = max(e - vec3<f32>(c1), vec3<f32>(0.0));
    let den = vec3<f32>(c2) - c3 * e;
    let nits = pow(num / den, vec3<f32>(1.0 / m1)) * 10000.0; // 0..10000 cd/m²
    return nits / 203.0;
}

// ARIB STD-B67 (HLG) inverse-OETF → scene light, normalized so reference white (signal 0.75) = 1.0.
// The display OOTF is omitted (scene-referred compositing); approximate but reasonable for SDR-out.
fn hlg_to_linear(c: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 0.28466892;
    let cc = 0.55991073;
    let lo = (c * c) / 3.0;
    let hi = (exp((c - vec3<f32>(cc)) / a) + vec3<f32>(b)) / 12.0;
    let scene = select(lo, hi, c > vec3<f32>(0.5));
    return scene / 0.26496256; // hlg_inv_oetf(0.75): put reference white at 1.0
}

// BT.2020 → BT.709 primaries, linear light (ITU-R BT.2087). Out-of-709 colours go negative → clamp.
fn bt2020_to_bt709(c: vec3<f32>) -> vec3<f32> {
    let r = 1.660491 * c.r - 0.587641 * c.g - 0.072850 * c.b;
    let g = -0.124550 * c.r + 1.132900 * c.g - 0.008349 * c.b;
    let b = -0.018151 * c.r - 0.100579 * c.g + 1.118730 * c.b;
    return max(vec3<f32>(r, g, b), vec3<f32>(0.0));
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
    // Valid encoded signal is [0,1]; clamp before the EOTF (HDR comes from the EOTF, not overshoot).
    let rgb_enc = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));

    // Encoded R'G'B' → scene-linear (graphics white = 1.0; HDR may exceed 1.0).
    var rgb_lin: vec3<f32>;
    if params.flags.y == 1u {
        rgb_lin = pq_to_linear(rgb_enc);
    } else if params.flags.y == 2u {
        rgb_lin = hlg_to_linear(rgb_enc);
    } else {
        rgb_lin = srgb_to_linear(rgb_enc);
    }

    // Wide-gamut → BT.709 in linear light to match the compositor's primaries.
    if params.flags.z == 1u {
        rgb_lin = bt2020_to_bt709(rgb_lin);
    }

    return vec4<f32>(rgb_lin, 1.0);
}
