// Sharpen Effect Shader
// Unsharp mask style sharpening

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
    let amount = uniforms.params0.x;  // Sharpen amount (0-3)
    let radius = uniforms.params0.y;  // Sample radius (0.5-5)

    let pixel_size = vec2<f32>(1.0 / uniforms.texture_width, 1.0 / uniforms.texture_height) * radius;

    // Sample neighbors for edge detection
    let left = textureSample(source_tex, source_sampler, in.uv - vec2<f32>(pixel_size.x, 0.0)).rgb;
    let right = textureSample(source_tex, source_sampler, in.uv + vec2<f32>(pixel_size.x, 0.0)).rgb;
    let top = textureSample(source_tex, source_sampler, in.uv - vec2<f32>(0.0, pixel_size.y)).rgb;
    let bottom = textureSample(source_tex, source_sampler, in.uv + vec2<f32>(0.0, pixel_size.y)).rgb;

    // Average of neighbors (blur)
    let blur = (left + right + top + bottom) * 0.25;

    // Unsharp mask: original + (original - blur) * amount
    let sharpened = src.rgb + (src.rgb - blur) * amount;

    // Clamp to valid range
    let clamped = clamp(sharpened, vec3<f32>(0.0), vec3<f32>(1.0));

    let result = mix(src.rgb, clamped, uniforms.mix);
    return vec4<f32>(result, src.a);
}
