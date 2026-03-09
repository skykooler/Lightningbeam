// GPU warp-apply shader.
//
// Two modes selected by grid_cols / grid_rows:
//
//   grid_cols == 0  (Liquify / per-pixel mode)
//     disp[] is a full canvas-sized array<vec2f>.  Each pixel reads its own entry.
//
//   grid_cols > 0   (Warp control-point mode)
//     disp[] contains only grid_cols * grid_rows vec2f displacements (one per
//     control point).  The shader bilinearly interpolates them so the CPU never
//     needs to build or upload the full per-pixel buffer.
//
// Dispatch: ceil(dst_w / 8) × ceil(dst_h / 8) × 1

struct Params {
    src_w:     u32,
    src_h:     u32,
    dst_w:     u32,
    dst_h:     u32,
    grid_cols: u32,   // 0 = per-pixel mode
    grid_rows: u32,
    _pad0:     u32,
    _pad1:     u32,
}

@group(0) @binding(0) var<uniform>       params: Params;
@group(0) @binding(1) var                src:    texture_2d<f32>;
@group(0) @binding(2) var<storage, read> disp:   array<vec2f>;
@group(0) @binding(3) var                dst:    texture_storage_2d<rgba8unorm, write>;

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

// Bilinearly interpolate the control-point displacement grid.
fn grid_displacement(px: u32, py: u32) -> vec2f {
    let cols = params.grid_cols;
    let rows = params.grid_rows;

    // Normalised position in grid space [0 .. cols-1] × [0 .. rows-1].
    let gx = f32(px) / f32(params.dst_w  - 1u) * f32(cols - 1u);
    let gy = f32(py) / f32(params.dst_h - 1u) * f32(rows - 1u);

    let col0 = u32(floor(gx));
    let row0 = u32(floor(gy));
    let col1 = min(col0 + 1u, cols - 1u);
    let row1 = min(row0 + 1u, rows - 1u);
    let fx = gx - floor(gx);
    let fy = gy - floor(gy);

    let d00 = disp[row0 * cols + col0];
    let d10 = disp[row0 * cols + col1];
    let d01 = disp[row1 * cols + col0];
    let d11 = disp[row1 * cols + col1];

    return d00 * (1.0 - fx) * (1.0 - fy)
         + d10 * fx          * (1.0 - fy)
         + d01 * (1.0 - fx)  * fy
         + d11 * fx          * fy;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.dst_w || gid.y >= params.dst_h { return; }

    var d: vec2f;
    if params.grid_cols > 0u {
        d = grid_displacement(gid.x, gid.y);
    } else {
        d = disp[gid.y * params.dst_w + gid.x];
    }

    let sx = f32(gid.x) + d.x;
    let sy = f32(gid.y) + d.y;

    var color: vec4<f32>;
    if sx < 0.0 || sy < 0.0 || sx >= f32(params.src_w) || sy >= f32(params.src_h) {
        color = vec4<f32>(0.0);
    } else {
        color = bilinear_sample(sx + 0.5, sy + 0.5);
    }

    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), color);
}
