// Canvas readback color conversion.
//
// Converts the Rgba16Float linear-PREMULTIPLIED canvas into an Rgba8Unorm
// sRGB-encoded premultiplied texture so the CPU readback is a pure row memcpy
// instead of a per-pixel sRGB decode. Matches the bytes the previous CPU decode
// produced: the sRGB OETF is applied per channel to the premultiplied RGB, and
// alpha (which is not gamma-encoded) is passed through.

// linear_to_srgb_channel is provided by the prepended COLOR_WGSL prelude.
@group(0) @binding(0) var src: texture_2d<f32>;                       // linear premultiplied
@group(0) @binding(1) var dst: texture_storage_2d<rgba8unorm, write>; // sRGB premultiplied

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dim = textureDimensions(src);
    if gid.x >= dim.x || gid.y >= dim.y {
        return;
    }
    let p = vec2<i32>(i32(gid.x), i32(gid.y));
    let c = textureLoad(src, p, 0);
    let srgb = vec3<f32>(
        linear_to_srgb_channel(c.r),
        linear_to_srgb_channel(c.g),
        linear_to_srgb_channel(c.b),
    );
    textureStore(dst, p, vec4<f32>(srgb, c.a));
}
