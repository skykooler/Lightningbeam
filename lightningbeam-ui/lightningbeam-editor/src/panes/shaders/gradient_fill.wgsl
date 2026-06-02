// GPU gradient fill shader.
//
// Reads the anchor canvas (before_pixels), composites a gradient over it, and
// writes the result to the display canvas.  All color values in the canvas are
// linear premultiplied RGBA.  The stop colors passed via `stops` are linear
// straight-alpha [0..1] (sRGB→linear conversion is done on the CPU).
//
// Dispatch: ceil(canvas_w / 8) × ceil(canvas_h / 8) × 1

struct Params {
    canvas_w:    u32,
    canvas_h:    u32,
    start_x:     f32,
    start_y:     f32,
    end_x:       f32,
    end_y:       f32,
    opacity:     f32,
    extend_mode: u32,  // 0 = Pad, 1 = Reflect, 2 = Repeat
    num_stops:   u32,
    kind:        u32,  // 0 = Linear, 1 = Radial
    _pad1:       u32,
    _pad2:       u32,
}

// 32 bytes per stop (8 × f32), matching `GpuGradientStop` on the Rust side.
struct GradientStop {
    position: f32,
    r:        f32,  // linear [0..1], straight-alpha
    g:        f32,
    b:        f32,
    a:        f32,
    _pad0:    f32,
    _pad1:    f32,
    _pad2:    f32,
}

@group(0) @binding(0) var<uniform>       params: Params;
@group(0) @binding(1) var                src:    texture_2d<f32>;
@group(0) @binding(2) var<storage, read> stops:  array<GradientStop>;
@group(0) @binding(3) var                dst:    texture_storage_2d<rgba8unorm, write>;

fn apply_extend(t: f32) -> f32 {
    if params.extend_mode == 0u {
        // Pad: clamp to [0, 1]
        return clamp(t, 0.0, 1.0);
    } else if params.extend_mode == 1u {
        // Reflect: 0→1→0→1→...
        let t_abs = abs(t);
        let period = floor(t_abs);
        let frac = t_abs - period;
        if (u32(period) & 1u) == 0u {
            return frac;
        } else {
            return 1.0 - frac;
        }
    } else {
        // Repeat: tile [0, 1)
        return t - floor(t);
    }
}

fn eval_gradient(t: f32) -> vec4<f32> {
    let n = params.num_stops;
    if n == 0u { return vec4<f32>(0.0); }

    let s0 = stops[0];
    if t <= s0.position {
        return vec4<f32>(s0.r, s0.g, s0.b, s0.a);
    }

    let sn = stops[n - 1u];
    if t >= sn.position {
        return vec4<f32>(sn.r, sn.g, sn.b, sn.a);
    }

    for (var i = 0u; i < n - 1u; i++) {
        let sa = stops[i];
        let sb = stops[i + 1u];
        if t >= sa.position && t <= sb.position {
            let span = sb.position - sa.position;
            let f = select(0.0, (t - sa.position) / span, span > 0.0001);
            return mix(
                vec4<f32>(sa.r, sa.g, sa.b, sa.a),
                vec4<f32>(sb.r, sb.g, sb.b, sb.a),
                f,
            );
        }
    }

    return vec4<f32>(sn.r, sn.g, sn.b, sn.a);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= params.canvas_w || gid.y >= params.canvas_h { return; }

    // Anchor pixel (linear premultiplied RGBA).
    let src_px = textureLoad(src, vec2<i32>(i32(gid.x), i32(gid.y)), 0);

    let dx = params.end_x - params.start_x;
    let dy = params.end_y - params.start_y;
    let px = f32(gid.x) + 0.5;
    let py = f32(gid.y) + 0.5;

    var t_raw: f32 = 0.0;
    if params.kind == 1u {
        // Radial: center at start point, radius = |end-start|.
        let radius = sqrt(dx * dx + dy * dy);
        if radius >= 0.5 {
            let pdx = px - params.start_x;
            let pdy = py - params.start_y;
            t_raw = sqrt(pdx * pdx + pdy * pdy) / radius;
        }
    } else {
        // Linear: project pixel centre onto gradient axis (start → end).
        let len2 = dx * dx + dy * dy;
        if len2 >= 1.0 {
            let fx = px - params.start_x;
            let fy = py - params.start_y;
            t_raw = (fx * dx + fy * dy) / len2;
        }
    }

    let t    = apply_extend(t_raw);
    let grad = eval_gradient(t);  // straight-alpha linear RGBA

    // Effective alpha: gradient alpha × tool opacity.
    let a = grad.a * params.opacity;

    // Alpha-over composite.
    // src_px.rgb is premultiplied (= straight_rgb * src_a).
    // Output is also premultiplied.
    let out_a   = a + src_px.a * (1.0 - a);
    let out_rgb = grad.rgb * a + src_px.rgb * (1.0 - a);

    textureStore(dst, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(out_rgb, out_a));
}
