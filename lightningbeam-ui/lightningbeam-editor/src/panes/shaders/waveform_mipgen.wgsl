// Min/max mipmap generation for 1D audio data packed into 2D textures.
// Each dest texel reduces 4 CONSECUTIVE source texels in audio order,
// using 1D→2D coordinate conversion since the 2D layout is linearized 1D.
// Row wrapping is handled by the modulo/division mapping:
//   x = index % width, y = index / width
// So sample index 2048 with width=2048 maps to (0, 1), not (0, 0).
//
// Texture format: Rgba16Float
//   R = left_min, G = left_max, B = right_min, A = right_max
//   At mip 0 (raw samples): R=G=left_sample, B=A=right_sample

struct MipParams {
    src_width: u32,
    dst_width: u32,
    src_sample_count: u32, // valid texels in source level
    _pad: u32,
}

@group(0) @binding(0) var src_mip: texture_2d<f32>;
@group(0) @binding(1) var dst_mip: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: MipParams;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    // Each thread handles one dest texel
    let dst_1d = id.x;
    let dst_size = textureDimensions(dst_mip);
    if dst_1d >= dst_size.x * dst_size.y {
        return;
    }

    let dst_x = dst_1d % params.dst_width;
    let dst_y = dst_1d / params.dst_width;

    // Map to 4 consecutive source texels in 1D audio order
    let src_base = dst_1d * 4u;
    var result = vec4(0.0);
    var initialized = false;

    for (var i = 0u; i < 4u; i++) {
        let src_1d = src_base + i;
        if src_1d >= params.src_sample_count {
            break;
        }
        // 1D → 2D: wraps across rows naturally
        let src_x = src_1d % params.src_width;
        let src_y = src_1d / params.src_width;
        let s = textureLoad(src_mip, vec2(src_x, src_y), 0);

        if !initialized {
            result = s;
            initialized = true;
        } else {
            result = vec4(
                min(result.r, s.r), // left_min
                max(result.g, s.g), // left_max
                min(result.b, s.b), // right_min
                max(result.a, s.a), // right_max
            );
        }
    }

    textureStore(dst_mip, vec2(dst_x, dst_y), result);
}
