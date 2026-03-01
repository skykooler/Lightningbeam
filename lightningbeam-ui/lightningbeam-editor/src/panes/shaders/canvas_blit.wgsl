// Canvas blit shader.
//
// Renders a GPU raster canvas (at document resolution) into the layer's sRGB
// render buffer (at viewport resolution), applying the camera transform
// (pan + zoom) to map document-space pixels to viewport-space pixels.
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

// Linear → sRGB encoding for a single channel.
// Applied to premultiplied linear values so the downstream srgb_to_linear
// pass round-trips correctly without darkening semi-transparent edges.
fn linear_to_srgb(c: f32) -> f32 {
    return select(
        1.055 * pow(max(c, 0.0), 1.0 / 2.4) - 0.055,
        c * 12.92,
        c <= 0.0031308,
    );
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
    // The downstream pipeline (srgb_to_linear → compositor) expects the sRGB
    // buffer to contain straight-alpha sRGB, i.e. the same format Vello outputs:
    //   sRGB buffer: srgb(r_straight), srgb(g_straight), srgb(b_straight), a
    //   srgb_to_linear: r_straight, g_straight, b_straight, a   (linear straight)
    //   compositor:  r_straight * a * opacity  (premultiplied, correct)
    //
    // Without unpremultiplying, the compositor would double-premultiply:
    //   src = (premul_r, premul_g, premul_b, a)  → output = premul_r * a = r * a²
    // which produces a dark halo over transparent regions.
    let c = textureSample(canvas_tex, canvas_sampler, canvas_uv);
    let inv_a = select(0.0, 1.0 / c.a, c.a > 1e-6);
    return vec4<f32>(
        linear_to_srgb(c.r * inv_a),
        linear_to_srgb(c.g * inv_a),
        linear_to_srgb(c.b * inv_a),
        c.a,
    );
}
