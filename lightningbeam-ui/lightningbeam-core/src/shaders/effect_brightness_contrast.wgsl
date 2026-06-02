// Brightness/Contrast Effect Shader

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
    let brightness = uniforms.params0.x; // -1 to 1
    let contrast = uniforms.params0.y;   // 0 to 3

    // Apply brightness (additive)
    var color = src.rgb + vec3<f32>(brightness);

    // Apply contrast (multiply around midpoint 0.5)
    color = (color - vec3<f32>(0.5)) * contrast + vec3<f32>(0.5);

    // Clamp to valid range
    color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));

    let result = mix(src.rgb, color, uniforms.mix);
    return vec4<f32>(result, src.a);
}
