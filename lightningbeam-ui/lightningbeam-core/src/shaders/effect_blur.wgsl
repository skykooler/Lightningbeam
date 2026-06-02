// Gaussian Blur Effect Shader
// Simple box blur approximation (real Gaussian would need multiple passes)

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
    let radius = uniforms.params0.x;   // Blur radius in pixels
    let quality = uniforms.params0.y;  // Quality (0-1, affects sample count)

    if (radius < 0.5) {
        return vec4<f32>(mix(src.rgb, src.rgb, uniforms.mix), src.a);
    }

    let pixel_size = vec2<f32>(1.0 / uniforms.texture_width, 1.0 / uniforms.texture_height);

    // Sample count based on quality (5-13 samples per direction)
    let samples = i32(5.0 + quality * 8.0);
    let half_samples = samples / 2;

    var color = vec3<f32>(0.0);
    var total_weight = 0.0;

    // Simple box blur with gaussian-like weighting
    for (var y = -half_samples; y <= half_samples; y++) {
        for (var x = -half_samples; x <= half_samples; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * pixel_size * radius / f32(half_samples);
            let sample_pos = in.uv + offset;

            // Gaussian-like weight based on distance
            let dist = length(vec2<f32>(f32(x), f32(y))) / f32(half_samples);
            let weight = exp(-dist * dist * 2.0);

            color += textureSample(source_tex, source_sampler, sample_pos).rgb * weight;
            total_weight += weight;
        }
    }

    color /= total_weight;

    let result = mix(src.rgb, color, uniforms.mix);
    return vec4<f32>(result, src.a);
}
