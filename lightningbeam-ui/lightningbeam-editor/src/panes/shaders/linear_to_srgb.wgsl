// Linear to sRGB color space conversion (fragment shader)
//
// Blits from HDR composite texture to display output.
// Input: RGBA16Float HDR texture in LINEAR color space
// Output: RGBA8Unorm sRGB for display
//
// The HDR texture contains linear color values (compositor converts
// Vello's sRGB output to linear). This shader converts back to sRGB
// for correct display on standard monitors.

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle vertex shader (3 vertices for a full-screen triangle)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);

    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);

    return out;
}

// Linear to sRGB conversion for a single channel
// Formula: c <= 0.0031308 ? c*12.92 : 1.055*pow(c, 1/2.4) - 0.055
fn linear_to_srgb_channel(c: f32) -> f32 {
    let clamped = clamp(c, 0.0, 1.0);
    return select(
        1.055 * pow(clamped, 1.0 / 2.4) - 0.055,
        clamped * 12.92,
        clamped <= 0.0031308
    );
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample linear HDR texture
    let linear = textureSample(input_tex, input_sampler, in.uv);

    // Convert from linear to sRGB for display (alpha stays linear)
    return vec4<f32>(
        linear_to_srgb_channel(linear.r),
        linear_to_srgb_channel(linear.g),
        linear_to_srgb_channel(linear.b),
        linear.a
    );
}
