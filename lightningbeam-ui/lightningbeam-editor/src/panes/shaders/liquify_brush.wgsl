// GPU liquify-brush shader.
//
// Updates a per-pixel displacement map (array of vec2f) for one brush step.
// Each pixel within the brush radius receives a displacement contribution
// weighted by a Gaussian falloff.
//
// Modes:
//   0 = Push        — displace in brush-drag direction (dx, dy)
//   1 = Pucker      — pull toward brush center
//   2 = Bloat       — push away from brush center
//   3 = Smooth      — blend toward average of 4 cardinal neighbours
//   4 = Reconstruct — blend toward zero (gradually undo)
//
// Dispatch: ceil((2*radius+1) / 8) × ceil((2*radius+1) / 8) × 1
// The CPU clips invocation IDs to the valid map range.

struct Params {
    cx:       f32,   // brush center x (canvas pixels)
    cy:       f32,   // brush center y
    radius:   f32,   // brush radius (canvas pixels)
    strength: f32,   // effect strength [0..1]
    dx:       f32,   // push direction x (normalised by caller, Push mode only)
    dy:       f32,   // push direction y
    mode:     u32,   // 0=Push 1=Pucker 2=Bloat 3=Smooth 4=Reconstruct
    map_w:    u32,
    map_h:    u32,
    _pad0:    u32,
    _pad1:    u32,
}

@group(0) @binding(0) var<uniform>             params: Params;
@group(0) @binding(1) var<storage, read_write> disp:   array<vec2f>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    // Offset invocation into the brush bounding box so gid(0,0) = (cx-r, cy-r).
    let base_x = floor(params.cx - params.radius);
    let base_y = floor(params.cy - params.radius);
    let px = base_x + f32(gid.x);
    let py = base_y + f32(gid.y);

    // Clip to displacement map bounds.
    if px < 0.0 || py < 0.0 { return; }
    let map_x = u32(px);
    let map_y = u32(py);
    if map_x >= params.map_w || map_y >= params.map_h { return; }

    let ddx = px - params.cx;
    let ddy = py - params.cy;
    let dist2 = ddx * ddx + ddy * ddy;
    let r2    = params.radius * params.radius;

    if dist2 > r2 { return; }

    // Gaussian influence: 1 at center, ~0.01 at edge (sigma = radius/2.15)
    let influence = params.strength * exp(-dist2 / (r2 * 0.2));

    let idx = map_y * params.map_w + map_x;
    var d = disp[idx];

    switch params.mode {
        case 0u: { // Push
            d = d + vec2f(params.dx, params.dy) * influence * params.radius;
        }
        case 1u: { // Pucker — toward center
            let len = sqrt(dist2) + 0.0001;
            d = d + vec2f(-ddx / len, -ddy / len) * influence * params.radius;
        }
        case 2u: { // Bloat — away from center
            let len = sqrt(dist2) + 0.0001;
            d = d + vec2f(ddx / len, ddy / len) * influence * params.radius;
        }
        case 3u: { // Smooth — blend toward average of 4 neighbours
            let xi = i32(map_x);
            let yi = i32(map_y);
            let w  = i32(params.map_w);
            let h  = i32(params.map_h);
            let l  = disp[u32(clamp(yi,     0, h-1)) * params.map_w + u32(clamp(xi - 1, 0, w-1))];
            let r  = disp[u32(clamp(yi,     0, h-1)) * params.map_w + u32(clamp(xi + 1, 0, w-1))];
            let u  = disp[u32(clamp(yi - 1, 0, h-1)) * params.map_w + u32(clamp(xi,     0, w-1))];
            let dn = disp[u32(clamp(yi + 1, 0, h-1)) * params.map_w + u32(clamp(xi,     0, w-1))];
            let avg = (l + r + u + dn) * 0.25;
            d = mix(d, avg, influence * 0.5);
        }
        case 4u: { // Reconstruct — blend toward zero
            d = mix(d, vec2f(0.0), influence * 0.5);
        }
        default: {}
    }

    disp[idx] = d;
}
