// Canvas blit shader.
//
// Renders a GPU raster canvas (at document resolution) into an Rgba16Float HDR
// buffer (at viewport resolution), applying the camera transform (pan + zoom)
// to map document-space pixels to viewport-space pixels.
//
// The canvas stores premultiplied linear RGBA.  We output it as-is so the HDR
// compositor sees the same premultiplied-linear format it always works with,
// bypassing the sRGB intermediate used for Vello layers.
//
// Any viewport pixel whose corresponding document coordinate falls outside
// [0, canvas_w) × [0, canvas_h) outputs transparent black.

struct CameraParams {
    pan_x:      f32,
    pan_y:      f32,
    zoom:       f32,
    canvas_w:   f32,
    canvas_h:   f32,
    viewport_w: f32,
    viewport_h: f32,
    _pad:       f32,
}

@group(0) @binding(0) var canvas_tex:     texture_2d<f32>;
@group(0) @binding(1) var canvas_sampler: sampler;
@group(0) @binding(2) var<uniform> camera: CameraParams;
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
    // Map viewport UV [0,1] → viewport pixel
    let vp = in.uv * vec2<f32>(camera.viewport_w, camera.viewport_h);

    // Map viewport pixel → document pixel (inverse camera transform)
    let doc = (vp - vec2<f32>(camera.pan_x, camera.pan_y)) / camera.zoom;

    // Map document pixel → canvas UV [0,1]
    let canvas_uv = doc / vec2<f32>(camera.canvas_w, camera.canvas_h);

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
