// GPU affine-transform resample shader.
//
// For each output pixel, computes the corresponding source pixel via an inverse
// 2D affine transform (no perspective) and bilinear-samples from the source texture.
//
// Used by the raster selection transform tool: the source is the immutable "anchor"
// canvas (original float pixels), the destination is the current float canvas.
//
// CPU precomputes the inverse affine matrix components and the output bounding box.
// The shader just does the per-pixel mapping and bilinear interpolation.
//
// Dispatch: ceil(dst_w / 8) × ceil(dst_h / 8) × 1

struct Params {
    // Inverse affine: src_pixel = A * out_pixel + b
    // For output pixel center (ox, oy), source pixel is:
    //   sx = a00*ox + a01*oy + b0
    //   sy = a10*ox + a11*oy + b1
    a00: f32, a01: f32,
    a10: f32, a11: f32,
    b0:  f32, b1:  f32,
    src_w: u32, src_h: u32,
    dst_w: u32, dst_h: u32,
    _pad0: u32, _pad1: u32,  // pad to 48 bytes (3 × 16)
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var          src:    texture_2d<f32>;
@group(0) @binding(2) var          dst:    texture_storage_2d<rgba16float, write>;

// Manual bilinear sample with clamp-to-edge (textureSample forbidden in compute shaders).
fn bilinear_sample(px: f32, py: f32) -> vec4<f32> {
    let sw = i32(params.src_w);
    let sh = i32(params.src_h);

    let ix = i32(floor(px - 0.5));
    let iy = i32(floor(py - 0.5));
    let fx = fract(px - 0.5);
    let fy = fract(py - 0.5);

    let x0 = clamp(ix,     0, sw - 1);
    let x1 = clamp(ix + 1, 0, sw - 1);
    let y0 = clamp(iy,     0, sh - 1);
    let y1 = clamp(iy + 1, 0, sh - 1);

    let s00 = textureLoad(src, vec2<i32>(x0, y0), 0);
    let s10 = textureLoad(src, vec2<i32>(x1, y0), 0);
    let s01 = textureLoad(src, vec2<i32>(x0, y1), 0);
    let s11 = textureLoad(src, vec2<i32>(x1, y1), 0);

    return mix(mix(s00, s10, fx), mix(s01, s11, fx), fy);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.dst_w || gid.y >= params.dst_h { return; }

    let ox = f32(gid.x);
    let oy = f32(gid.y);

    // Map output pixel index → source pixel position via inverse affine.
    // We use pixel centers (ox + 0.5, oy + 0.5) in the forward transform, but the
    // b0/b1 precomputation on the CPU already accounts for the +0.5 offset, so ox/oy
    // are used directly here (the CPU bakes +0.5 into b).
    let sx = params.a00 * ox + params.a01 * oy + params.b0;
    let sy = params.a10 * ox + params.a11 * oy + params.b1;

    var color: vec4<f32>;
    if sx < 0.0 || sy < 0.0 || sx >= f32(params.src_w) || sy >= f32(params.src_h) {
        // Outside source bounds → transparent
        color = vec4<f32>(0.0);
    } else {
        // Bilinear sample at pixel center
        color = bilinear_sample(sx + 0.5, sy + 0.5);
    }

    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), color);
}
