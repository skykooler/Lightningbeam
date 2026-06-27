// Linear-HDR → PQ/HLG BT.2020 encode (for 10-bit HDR video export).
//
// Input: the compositor's Rgba16Float HDR accumulator — PREMULTIPLIED scene-linear, BT.709
// primaries, graphics white = 1.0, HDR highlights > 1.0.
// Output: gamma-encoded R'G'B' in BT.2020 primaries, PQ (mode 0) or HLG (mode 1), to an
// Rgba16Unorm target. A later CPU pass does only BT.2020 R'G'B'→Y'CbCr (no transfer) + 4:2:0 + 10-bit.
//
// This is the encode inverse of panes/shaders/nv12_blit.wgsl's decode (203-nit PQ white; HLG
// reference white at signal 0.75), so a decode→encode round-trip is the identity.

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> params: vec4<u32>; // .x = mode (0 = PQ, 1 = HLG)

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
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

// BT.709 → BT.2020 primaries, linear light (ITU-R BT.2087).
fn bt709_to_bt2020(c: vec3<f32>) -> vec3<f32> {
    let r = 0.627404 * c.r + 0.329283 * c.g + 0.043313 * c.b;
    let g = 0.069097 * c.r + 0.919540 * c.g + 0.011362 * c.b;
    let b = 0.016391 * c.r + 0.088013 * c.g + 0.895595 * c.b;
    return vec3<f32>(r, g, b);
}

// SMPTE ST 2084 (PQ) OETF: scene-linear (white = 1.0 = 203 nits) → PQ code [0,1].
fn pq_oetf(lin: vec3<f32>) -> vec3<f32> {
    let nits = max(lin, vec3<f32>(0.0)) * 203.0;
    let ln = min(nits / 10000.0, vec3<f32>(1.0));
    let m1 = 0.1593017578125;
    let m2 = 78.84375;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let lm = pow(ln, vec3<f32>(m1));
    return pow((vec3<f32>(c1) + c2 * lm) / (vec3<f32>(1.0) + c3 * lm), vec3<f32>(m2));
}

// ARIB STD-B67 (HLG) OETF: scene-linear (white = 1.0) → HLG signal [0,1]. Reference white maps to
// signal 0.75 (matching the decode's /0.26496256 normalization). Display OOTF omitted (scene-referred).
fn hlg_oetf(lin: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let e = clamp(lin * 0.26496256, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = sqrt(3.0 * e);
    let hi = a * log(12.0 * e - vec3<f32>(b)) + vec3<f32>(c);
    return select(lo, hi, e > vec3<f32>(1.0 / 12.0));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compositor stores PREMULTIPLIED linear; unpremultiply to straight (video is opaque, a≈1).
    let texel = textureSample(input_tex, input_sampler, in.uv);
    let a = texel.a;
    let straight = select(texel.rgb / a, vec3<f32>(0.0), a <= 0.0);

    let bt2020 = max(bt709_to_bt2020(straight), vec3<f32>(0.0));
    var enc: vec3<f32>;
    if params.x == 1u {
        enc = hlg_oetf(bt2020);
    } else {
        enc = pq_oetf(bt2020);
    }
    return vec4<f32>(enc, 1.0);
}
