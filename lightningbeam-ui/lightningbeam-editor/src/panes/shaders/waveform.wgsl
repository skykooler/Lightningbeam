// Waveform rendering shader for audio data stored in 2D Rgba16Float textures.
// Audio samples are packed row-major into 2D: frame_index = y * tex_width + x
// Mip levels use min/max reduction (4 consecutive samples per level).
//
// At full zoom, min≈max → renders as a continuous wave.
// At zoom-out, min/max spread → renders as filled peak region.
//
// display_mode: 0 = combined (mono mix), 1 = split (left top, right bottom)

struct Params {
    // Clip rectangle in screen pixels (min.x, min.y, max.x, max.y)
    clip_rect: vec4<f32>,
    // Timeline viewport parameters
    viewport_start_time: f32,
    pixels_per_second: f32,
    // Audio file properties
    audio_duration: f32,
    sample_rate: f32,
    // Clip placement
    clip_start_time: f32,
    trim_start: f32,
    // Texture layout
    tex_width: f32,
    total_frames: f32,      // total frame count in this texture segment
    segment_start_frame: f32, // first frame this texture covers (for multi-texture)
    display_mode: f32,       // 0 = combined, 1 = split stereo
    // Appearance
    tint_color: vec4<f32>,
    // Screen dimensions for coordinate conversion
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var peak_tex: texture_2d<f32>;
@group(0) @binding(1) var peak_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle (3 vertices, no vertex buffer)
@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.position = vec4(x, y, 0.0, 1.0);
    out.uv = vec2((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let frag_x = in.position.x;
    let frag_y = in.position.y;

    // Clip to the clip rectangle
    if frag_x < params.clip_rect.x || frag_x > params.clip_rect.z ||
       frag_y < params.clip_rect.y || frag_y > params.clip_rect.w {
        return vec4(0.0, 0.0, 0.0, 0.0);
    }

    // Fragment X position → audio time
    // clip_start_time is the screen X of the (unclamped) clip left edge.
    // (frag_x - clip_start_time) / pps gives the time offset from the clip's start.
    let audio_time = (frag_x - params.clip_start_time) / params.pixels_per_second + params.trim_start;

    // Audio time → frame index
    let frame_f = audio_time * params.sample_rate - params.segment_start_frame;
    if frame_f < 0.0 || frame_f >= params.total_frames {
        return vec4(0.0, 0.0, 0.0, 0.0);
    }

    // Determine mip level based on how many audio frames map to one pixel
    let frames_per_pixel = params.sample_rate / params.pixels_per_second;
    // Each mip level reduces by 4x in sample count (2x in each texture dimension)
    let mip_f = max(0.0, log2(frames_per_pixel) / 2.0);
    let max_mip = f32(textureNumLevels(peak_tex) - 1u);
    let mip = min(mip_f, max_mip);

    // Frame index at the chosen mip level
    let mip_floor = u32(mip);
    let reduction = pow(4.0, f32(mip_floor));
    let mip_frame = frame_f / reduction;

    // Convert 1D mip-space index to 2D UV coordinates
    let mip_tex_width = params.tex_width / pow(2.0, f32(mip_floor));
    let mip_tex_height = ceil(params.total_frames / reduction / mip_tex_width);
    let texel_x = mip_frame % mip_tex_width;
    let texel_y = floor(mip_frame / mip_tex_width);
    let uv = vec2((texel_x + 0.5) / mip_tex_width, (texel_y + 0.5) / mip_tex_height);

    // Sample the peak texture at computed mip level
    // R = left_min, G = left_max, B = right_min, A = right_max
    let peak = textureSampleLevel(peak_tex, peak_sampler, uv, mip);

    let clip_height = params.clip_rect.w - params.clip_rect.y;
    let clip_top = params.clip_rect.y;

    if params.display_mode < 0.5 {
        // Combined mode: merge both channels
        let wave_min = min(peak.r, peak.b);
        let wave_max = max(peak.g, peak.a);

        let center_y = clip_top + clip_height * 0.5;
        let scale = clip_height * 0.45;

        let y_top = center_y - wave_max * scale;
        let y_bot = center_y - wave_min * scale;

        // At least 1px tall for visibility
        let y_top_adj = min(y_top, center_y - 0.5);
        let y_bot_adj = max(y_bot, center_y + 0.5);

        if frag_y >= y_top_adj && frag_y <= y_bot_adj {
            return params.tint_color;
        }
        return vec4(0.0, 0.0, 0.0, 0.0);
    } else {
        // Split stereo mode: left channel in top half, right channel in bottom half
        let half_height = clip_height * 0.5;
        let mid_y = clip_top + half_height;

        // Determine which channel this fragment belongs to
        if frag_y < mid_y {
            // Top half: left channel
            let center_y = clip_top + half_height * 0.5;
            let scale = half_height * 0.45;

            let y_top = center_y - peak.g * scale; // left_max
            let y_bot = center_y - peak.r * scale; // left_min

            let y_top_adj = min(y_top, center_y - 0.5);
            let y_bot_adj = max(y_bot, center_y + 0.5);

            if frag_y >= y_top_adj && frag_y <= y_bot_adj {
                return params.tint_color;
            }
        } else {
            // Bottom half: right channel
            let center_y = mid_y + half_height * 0.5;
            let scale = half_height * 0.45;

            let y_top = center_y - peak.a * scale; // right_max
            let y_bot = center_y - peak.b * scale; // right_min

            let y_top_adj = min(y_top, center_y - 0.5);
            let y_bot_adj = max(y_bot, center_y + 0.5);

            if frag_y >= y_top_adj && frag_y <= y_bot_adj {
                return params.tint_color;
            }
        }
        return vec4(0.0, 0.0, 0.0, 0.0);
    }
}
