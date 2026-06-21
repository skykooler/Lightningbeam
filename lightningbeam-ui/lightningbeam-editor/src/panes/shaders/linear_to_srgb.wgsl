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

// linear_to_srgb_channel is provided by the prepended COLOR_WGSL prelude.

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample linear HDR texture. The compositor accumulates PREMULTIPLIED
    // linear color, so unpremultiply before applying the sRGB OETF:
    // srgb(rgb*a) != srgb(rgb)*a, so encoding premultiplied color directly
    // corrupts antialiased edges and transparent pixels. We output straight
    // alpha to match the straight-alpha display blit and PNG export.
    let linear = textureSample(input_tex, input_sampler, in.uv);
    let a = linear.a;
    let straight = select(linear.rgb / a, vec3<f32>(0.0), a <= 0.0);

    return vec4<f32>(
        linear_to_srgb_channel(straight.r),
        linear_to_srgb_channel(straight.g),
        linear_to_srgb_channel(straight.b),
        a
    );
}
