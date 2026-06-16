// Hue/Saturation/Lightness Effect Shader

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

// Convert RGB to HSL
fn rgb_to_hsl(c: vec3<f32>) -> vec3<f32> {
    let cmax = max(max(c.r, c.g), c.b);
    let cmin = min(min(c.r, c.g), c.b);
    let delta = cmax - cmin;

    var h = 0.0;
    var s = 0.0;
    let l = (cmax + cmin) / 2.0;

    if (delta > 0.0) {
        s = select(delta / (2.0 - cmax - cmin), delta / (cmax + cmin), l < 0.5);

        if (cmax == c.r) {
            h = (c.g - c.b) / delta + select(6.0, 0.0, c.g >= c.b);
        } else if (cmax == c.g) {
            h = (c.b - c.r) / delta + 2.0;
        } else {
            h = (c.r - c.g) / delta + 4.0;
        }
        h /= 6.0;
    }

    return vec3<f32>(h, s, l);
}

// Helper function for HSL to RGB
fn hue_to_rgb(p: f32, q: f32, t: f32) -> f32 {
    var tt = t;
    if (tt < 0.0) { tt += 1.0; }
    if (tt > 1.0) { tt -= 1.0; }
    if (tt < 1.0/6.0) { return p + (q - p) * 6.0 * tt; }
    if (tt < 1.0/2.0) { return q; }
    if (tt < 2.0/3.0) { return p + (q - p) * (2.0/3.0 - tt) * 6.0; }
    return p;
}

// Convert HSL to RGB
fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    if (hsl.y == 0.0) {
        return vec3<f32>(hsl.z, hsl.z, hsl.z);
    }

    let q = select(hsl.z + hsl.y - hsl.z * hsl.y, hsl.z * (1.0 + hsl.y), hsl.z < 0.5);
    let p = 2.0 * hsl.z - q;

    return vec3<f32>(
        hue_to_rgb(p, q, hsl.x + 1.0/3.0),
        hue_to_rgb(p, q, hsl.x),
        hue_to_rgb(p, q, hsl.x - 1.0/3.0)
    );
}

// The HDR pipeline feeds this shader LINEAR light, but the HSL model (and the
// lightness/saturation axes users expect) is defined on gamma-encoded sRGB.
// Convert to sRGB, run the HSL adjustment there, then convert back to linear.
// sRGB helpers (linear_to_srgb / srgb_to_linear) come from the prepended
// COLOR_WGSL prelude.

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_tex, source_sampler, in.uv);
    let hue_shift = uniforms.params0.x / 360.0;  // Convert degrees to 0-1 range
    let saturation = uniforms.params0.y;          // Multiplier (1.0 = no change)
    let lightness = uniforms.params0.z;           // Additive (-1 to 1)

    let src_srgb = linear_to_srgb(src.rgb);

    // Convert to HSL
    var hsl = rgb_to_hsl(src_srgb);

    // Apply adjustments
    hsl.x = fract(hsl.x + hue_shift);           // Shift hue (wrapping)
    hsl.y = clamp(hsl.y * saturation, 0.0, 1.0); // Multiply saturation
    hsl.z = clamp(hsl.z + lightness, 0.0, 1.0);  // Add lightness

    // Convert back to RGB
    let adjusted = hsl_to_rgb(hsl);

    let result_srgb = mix(src_srgb, adjusted, uniforms.mix);
    return vec4<f32>(srgb_to_linear(result_srgb), src.a);
}
