// Alpha composite compute shader.
//
// Composites the accumulated-dab scratch buffer C on top of the source buffer A,
// writing the result into the output buffer B:
//
//   B[px] = C[px] + A[px] * (1 − C[px].a)    (Porter-Duff src-over, C over A)
//
// All textures are Rgba16Float, linear premultiplied RGBA.
// Dispatch: ceil(w/8) × ceil(h/8) × 1.

@group(0) @binding(0) var tex_a: texture_2d<f32>;                        // source (A)
@group(0) @binding(1) var tex_c: texture_2d<f32>;                        // accumulated dabs (C)
@group(0) @binding(2) var tex_b: texture_storage_2d<rgba16float, write>;  // output (B)

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(tex_a);
    if gid.x >= dims.x || gid.y >= dims.y { return; }

    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(tex_a, coord, 0);
    let c = textureLoad(tex_c, coord, 0);

    // Porter-Duff src-over: C is the foreground (dabs), A is the background.
    // out = c + a * (1 - c.a)
    textureStore(tex_b, coord, c + a * (1.0 - c.a));
}
