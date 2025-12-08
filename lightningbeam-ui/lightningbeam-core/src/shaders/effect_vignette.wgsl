// Vignette Effect Shader
// Darkens edges of the image

struct Uniforms {
    // params packed as vec4s for proper 16-byte alignment
    params0: vec4<f32>,
    params1: vec4<f32>,
    params2: vec4<f32>,
    params3: vec4<f32>,
    texture_width: f32,
    texture_height: f32,
    time: f32,
    mix: f32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(0) @binding(2) var<uniform> uniforms: Uniforms;

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
    let src = textureSample(source_tex, source_sampler, in.uv);
    let radius = uniforms.params0.x;   // Vignette radius (0.5 = normal)
    let softness = uniforms.params0.y; // Edge softness (0-1)
    let amount = uniforms.params0.z;   // Darkness amount (0-1)

    // Calculate distance from center (normalized to -1 to 1)
    let center = vec2<f32>(0.5, 0.5);
    let dist = distance(in.uv, center);

    // Create vignette factor with smooth falloff
    let inner = radius;
    let outer = radius + softness;
    let vignette = 1.0 - smoothstep(inner, outer, dist) * amount;

    let vignetted = src.rgb * vignette;

    let result = mix(src.rgb, vignetted, uniforms.mix);
    return vec4<f32>(result, src.a);
}
