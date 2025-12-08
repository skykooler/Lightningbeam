// sRGB to Linear color space conversion (compute shader)
//
// Converts an sRGB texture to linear color space for HDR processing.
// Input: RGBA8 sRGB texture
// Output: RGBA16Float linear texture

@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var output_tex: texture_storage_2d<rgba16float, write>;

// sRGB to linear conversion for a single channel
// Formula: c <= 0.04045 ? c/12.92 : pow((c+0.055)/1.055, 2.4)
fn srgb_to_linear(c: f32) -> f32 {
    return select(
        pow((c + 0.055) / 1.055, 2.4),
        c / 12.92,
        c <= 0.04045
    );
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(input_tex);

    // Bounds check
    if (gid.x >= dims.x || gid.y >= dims.y) {
        return;
    }

    // Load sRGB pixel
    let srgb = textureLoad(input_tex, gid.xy, 0);

    // Convert RGB channels to linear (alpha stays linear)
    let linear = vec4<f32>(
        srgb_to_linear(srgb.r),
        srgb_to_linear(srgb.g),
        srgb_to_linear(srgb.b),
        srgb.a
    );

    // Store linear result
    textureStore(output_tex, gid.xy, linear);
}
