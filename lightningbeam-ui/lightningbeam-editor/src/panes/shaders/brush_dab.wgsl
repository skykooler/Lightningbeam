// GPU brush dab compute shader.
//
// Renders all dabs for one stroke segment into the raster canvas.
// Uses a ping-pong pair: reads from `canvas_src` (texture_2d) via textureLoad,
// writes to `canvas_dst` (storage, write-only).
//
// `textureSample` is forbidden in compute shaders; bilinear filtering for the
// smudge tool is implemented manually using four textureLoad calls.
//
// Before this dispatch the caller copies `canvas_src` → `canvas_dst` so that pixels
// outside the union dab bounding box (not touched by the shader) remain unchanged.
//
// Dispatch: ceil(bbox_w / 8) × ceil(bbox_h / 8) × 1
// Each thread covers one pixel in the bounding-box-clamped canvas region.

// ---------------------------------------------------------------------------
// Data layout must match GpuDab in brush_engine.rs (64 bytes, 16-byte aligned).
// ---------------------------------------------------------------------------
struct GpuDab {
    x: f32, y: f32, radius: f32, hardness: f32,             // bytes  0–15
    opacity: f32, color_r: f32, color_g: f32, color_b: f32, // bytes 16–31
    color_a: f32, ndx: f32, ndy: f32, smudge_dist: f32,     // bytes 32–47
    blend_mode: u32, elliptical_dab_ratio: f32, elliptical_dab_angle: f32, lock_alpha: f32, // bytes 48–63
}

struct Params {
    bbox_x0:  i32,
    bbox_y0:  i32,
    bbox_w:   u32,
    bbox_h:   u32,
    num_dabs: u32,
    canvas_w: u32,
    canvas_h: u32,
    _pad:     u32,
}

@group(0) @binding(0) var<storage, read> dabs:       array<GpuDab>;
@group(0) @binding(1) var<uniform>       params:     Params;
@group(0) @binding(2) var               canvas_src: texture_2d<f32>;
@group(0) @binding(3) var               canvas_dst: texture_storage_2d<rgba8unorm, write>;

// ---------------------------------------------------------------------------
// Manual bilinear sample from canvas_src at sub-pixel coordinates (px, py).
// Out-of-bounds texels clamp to the canvas edge (replicates ClampToEdge).
// textureSample is forbidden in compute shaders; we use four textureLoad calls.
// ---------------------------------------------------------------------------
fn bilinear_sample(px: f32, py: f32) -> vec4<f32> {
    let cw = i32(params.canvas_w);
    let ch = i32(params.canvas_h);

    // Integer coords of the top-left sample
    let ix = i32(floor(px - 0.5));
    let iy = i32(floor(py - 0.5));

    // Fractional weights
    let fx = fract(px - 0.5);
    let fy = fract(py - 0.5);

    // Clamp to [0, dim-1]
    let x0 = clamp(ix,     0, cw - 1);
    let x1 = clamp(ix + 1, 0, cw - 1);
    let y0 = clamp(iy,     0, ch - 1);
    let y1 = clamp(iy + 1, 0, ch - 1);

    let s00 = textureLoad(canvas_src, vec2<i32>(x0, y0), 0);
    let s10 = textureLoad(canvas_src, vec2<i32>(x1, y0), 0);
    let s01 = textureLoad(canvas_src, vec2<i32>(x0, y1), 0);
    let s11 = textureLoad(canvas_src, vec2<i32>(x1, y1), 0);

    return mix(mix(s00, s10, fx), mix(s01, s11, fx), fy);
}

// ---------------------------------------------------------------------------
// Apply a single dab to `current` and return the updated colour.
// ---------------------------------------------------------------------------
fn apply_dab(current: vec4<f32>, dab: GpuDab, px: i32, py: i32) -> vec4<f32> {
    let dx = f32(px) + 0.5 - dab.x;
    let dy = f32(py) + 0.5 - dab.y;

    // Normalised squared distance — supports circular and elliptical dabs.
    var rr: f32;
    if dab.elliptical_dab_ratio > 1.001 {
        // Rotate into the dab's local frame.
        // Major axis is along dab.elliptical_dab_angle; minor axis is compressed by ratio.
        let c = cos(dab.elliptical_dab_angle);
        let s = sin(dab.elliptical_dab_angle);
        let dx_r =  dx * c + dy * s;                               // along major axis
        let dy_r = (-dx * s + dy * c) * dab.elliptical_dab_ratio;  // minor axis compressed
        rr = (dx_r * dx_r + dy_r * dy_r) / (dab.radius * dab.radius);
    } else {
        rr = (dx * dx + dy * dy) / (dab.radius * dab.radius);
    }
    if rr > 1.0 { return current; }

    // Quadratic falloff: flat inner core, smooth quadratic outer zone.
    // r is the actual normalised distance [0,1]; h controls the hard-core radius.
    // Inner zone (r ≤ h): fully opaque.
    // Outer zone (r > h): opa = ((1-r)/(1-h))^2, giving a smooth bell-shaped dab.
    let h = clamp(dab.hardness, 0.0, 1.0);
    let r = sqrt(rr);
    var opa_weight: f32;
    if h >= 1.0 || r <= h {
        opa_weight = 1.0;
    } else {
        let t = (1.0 - r) / (1.0 - h);
        opa_weight = t * t;
    }

    if dab.blend_mode == 0u {
        // Normal: "over" operator on premultiplied RGBA.
        // If lock_alpha > 0.5, preserve the destination alpha unchanged.
        let dab_a = opa_weight * dab.opacity * dab.color_a;
        if dab_a <= 0.0 { return current; }
        let ba = 1.0 - dab_a;
        let out_a = select(dab_a + ba * current.a, current.a, dab.lock_alpha > 0.5);
        return vec4<f32>(
            dab_a * dab.color_r + ba * current.r,
            dab_a * dab.color_g + ba * current.g,
            dab_a * dab.color_b + ba * current.b,
            out_a,
        );
    } else if dab.blend_mode == 1u {
        // Erase: multiplicative alpha reduction
        let dab_a = opa_weight * dab.opacity * dab.color_a;
        if dab_a <= 0.0 { return current; }
        let new_a = current.a * (1.0 - dab_a);
        let scale = select(0.0, new_a / current.a, current.a > 1e-6);
        return vec4<f32>(current.r * scale, current.g * scale, current.b * scale, new_a);
    } else if dab.blend_mode == 2u {
        // Smudge: directional warp — sample from position behind the stroke direction
        let alpha = opa_weight * dab.opacity;
        if alpha <= 0.0 { return current; }
        let src_x = f32(px) + 0.5 - dab.ndx * dab.smudge_dist;
        let src_y = f32(py) + 0.5 - dab.ndy * dab.smudge_dist;
        let src   = bilinear_sample(src_x, src_y);
        let da    = 1.0 - alpha;
        return vec4<f32>(
            alpha * src.r + da * current.r,
            alpha * src.g + da * current.g,
            alpha * src.b + da * current.b,
            alpha * src.a + da * current.a,
        );
    } else if dab.blend_mode == 3u {
        // Clone stamp: sample from (this_pixel + offset) in the source canvas.
        // color_r/color_g store the world-space offset (source_world - drag_start_world)
        // computed once when the stroke begins. Each pixel samples its own source texel.
        let alpha = opa_weight * dab.opacity;
        if alpha <= 0.0 { return current; }
        let src_x = f32(px) + 0.5 + dab.color_r;
        let src_y = f32(py) + 0.5 + dab.color_g;
        let src = bilinear_sample(src_x, src_y);
        let ba  = 1.0 - alpha;
        return vec4<f32>(
            alpha * src.r + ba * current.r,
            alpha * src.g + ba * current.g,
            alpha * src.b + ba * current.b,
            alpha * src.a + ba * current.a,
        );
    } else {
        return current;
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Bounds check within the bounding box
    if gid.x >= params.bbox_w || gid.y >= params.bbox_h { return; }

    let px = i32(gid.x) + params.bbox_x0;
    let py = i32(gid.y) + params.bbox_y0;

    // Bounds check within the canvas (bbox may extend past canvas edges)
    if px < 0 || py < 0 || u32(px) >= params.canvas_w || u32(py) >= params.canvas_h { return; }

    // Read current pixel from source (canvas_dst was pre-filled from canvas_src
    // by the caller, but we read from canvas_src to ensure consistency)
    var current = textureLoad(canvas_src, vec2<i32>(px, py), 0);

    // Apply all dabs for this frame (sequential in the thread, no races between threads
    // since each thread owns a unique output pixel)
    for (var i = 0u; i < params.num_dabs; i++) {
        current = apply_dab(current, dabs[i], px, py);
    }

    textureStore(canvas_dst, vec2<i32>(px, py), current);
}
