// Canvas blit shader.
//
// Renders a GPU raster canvas (at document resolution) into an Rgba16Float HDR
// buffer (at viewport resolution), applying a general affine transform that maps
// viewport UV [0,1]² directly to canvas UV [0,1]².
//
// The combined inverse transform (viewport UV → canvas UV) is pre-computed on the
// CPU and uploaded as a column-major 3×3 matrix packed into three vec4 uniforms.
//
// The canvas stores premultiplied linear RGBA.  We output it as-is so the HDR
// compositor sees the same premultiplied-linear format it always works with,
// bypassing the sRGB intermediate used for Vello layers.
//
// Any viewport pixel whose corresponding canvas coordinate falls outside [0,1)²
// outputs transparent black.

struct BlitTransform {
    /// Column 0 of the viewport_uv → canvas_uv affine matrix (+ padding).
    col0: vec4<f32>,
    /// Column 1 (+ padding).
    col1: vec4<f32>,
    /// Column 2: translation column — col2.xy = translation, col2.z = 1 (+ padding).
    col2: vec4<f32>,
}

@group(0) @binding(0) var canvas_tex:     texture_2d<f32>;
@group(0) @binding(1) var canvas_sampler: sampler;
@group(0) @binding(2) var<uniform> transform: BlitTransform;
/// Selection mask: R8Unorm, 255 = inside selection (keep), 0 = outside (discard).
/// A 1×1 all-white texture is bound when no selection is active.
@group(0) @binding(3) var mask_tex:     texture_2d<f32>;
@group(0) @binding(4) var mask_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0)       uv:       vec2<f32>,
}

// Generates a fullscreen triangle strip (same pattern as blit.wgsl)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);
    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Apply the combined inverse transform: viewport UV → canvas UV.
    let m = mat3x3<f32>(transform.col0.xyz, transform.col1.xyz, transform.col2.xyz);
    let canvas_uv_h = m * vec3<f32>(in.uv.x, in.uv.y, 1.0);
    let canvas_uv = canvas_uv_h.xy;

    // Out-of-bounds → transparent
    if canvas_uv.x < 0.0 || canvas_uv.x > 1.0
    || canvas_uv.y < 0.0 || canvas_uv.y > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // The canvas stores premultiplied linear RGBA.
    // The compositor expects straight-alpha linear (it premultiplies by src_alpha itself),
    // so unpremultiply here.  No sRGB conversion — the HDR buffer is linear throughout.
    let c = textureSample(canvas_tex, canvas_sampler, canvas_uv);
    let mask = textureSample(mask_tex, mask_sampler, canvas_uv).r;
    let masked_a = c.a * mask;
    let inv_a = select(0.0, 1.0 / c.a, c.a > 1e-6);
    return vec4<f32>(c.r * inv_a, c.g * inv_a, c.b * inv_a, masked_a);
}
