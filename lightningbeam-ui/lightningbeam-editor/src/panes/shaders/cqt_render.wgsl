// CQT spectrogram render shader.
//
// Reads from a ring-buffer cache texture (Rgba16Float) where:
//   X = time column (ring buffer index), Y = CQT frequency bin
// CQT bins map directly to MIDI notes via: bin = (note - min_note) * bins_per_octave / 12
//
// Applies the same colormap as the old FFT spectrogram.

// Must match CqtRenderParams in cqt_gpu.rs exactly (96 bytes).
struct Params {
    clip_rect: vec4<f32>,         // 16 @ 0
    viewport_start_time: f32,     // 4 @ 16
    pixels_per_second: f32,       // 4 @ 20
    audio_duration: f32,          // 4 @ 24
    sample_rate: f32,             // 4 @ 28
    clip_start_time: f32,         // 4 @ 32
    trim_start: f32,              // 4 @ 36
    freq_bins: f32,               // 4 @ 40
    bins_per_octave: f32,         // 4 @ 44
    hop_size: f32,                // 4 @ 48
    scroll_y: f32,                // 4 @ 52
    note_height: f32,             // 4 @ 56
    min_note: f32,                // 4 @ 60
    max_note: f32,                // 4 @ 64
    gamma: f32,                   // 4 @ 68
    cache_capacity: f32,          // 4 @ 72
    cache_start_column: f32,      // 4 @ 76
    cache_valid_start: f32,       // 4 @ 80
    cache_valid_end: f32,         // 4 @ 84
    column_stride: f32,           // 4 @ 88
    _pad: f32,                    // 4 @ 92, total 96
}

@group(0) @binding(0) var cache_tex: texture_2d<f32>;
@group(0) @binding(1) var cache_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) % 2) * 4.0 - 1.0;
    out.position = vec4(x, y, 0.0, 1.0);
    out.uv = vec2((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

fn rounded_rect_sdf(pos: vec2<f32>, rect_min: vec2<f32>, rect_max: vec2<f32>, r: f32) -> f32 {
    let center = (rect_min + rect_max) * 0.5;
    let half_size = (rect_max - rect_min) * 0.5;
    let q = abs(pos - center) - half_size + vec2(r);
    return length(max(q, vec2(0.0))) - r;
}

// Colormap: black -> blue -> purple -> red -> orange -> yellow -> white
fn colormap(v: f32, gamma: f32) -> vec4<f32> {
    let t = pow(clamp(v, 0.0, 1.0), gamma);

    if t < 1.0 / 6.0 {
        let s = t * 6.0;
        return vec4(0.0, 0.0, s, 1.0);
    } else if t < 2.0 / 6.0 {
        let s = (t - 1.0 / 6.0) * 6.0;
        return vec4(s * 0.6, 0.0, 1.0 - s * 0.2, 1.0);
    } else if t < 3.0 / 6.0 {
        let s = (t - 2.0 / 6.0) * 6.0;
        return vec4(0.6 + s * 0.4, 0.0, 0.8 - s * 0.8, 1.0);
    } else if t < 4.0 / 6.0 {
        let s = (t - 3.0 / 6.0) * 6.0;
        return vec4(1.0, s * 0.5, 0.0, 1.0);
    } else if t < 5.0 / 6.0 {
        let s = (t - 4.0 / 6.0) * 6.0;
        return vec4(1.0, 0.5 + s * 0.5, 0.0, 1.0);
    } else {
        let s = (t - 5.0 / 6.0) * 6.0;
        return vec4(1.0, 1.0, s, 1.0);
    }
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let frag_x = in.position.x;
    let frag_y = in.position.y;

    // Clip to view rectangle
    if frag_x < params.clip_rect.x || frag_x > params.clip_rect.z ||
       frag_y < params.clip_rect.y || frag_y > params.clip_rect.w {
        discard;
    }

    // Compute the content rect in screen space
    let content_left = params.clip_rect.x + (params.clip_start_time - params.trim_start - params.viewport_start_time) * params.pixels_per_second;
    let content_right = content_left + params.audio_duration * params.pixels_per_second;
    let content_top = params.clip_rect.y - params.scroll_y;
    let content_bottom = params.clip_rect.y + (params.max_note - params.min_note + 1.0) * params.note_height - params.scroll_y;

    // Rounded corners
    let vis_top = max(content_top, params.clip_rect.y);
    let vis_bottom = min(content_bottom, params.clip_rect.w);
    let corner_radius = 6.0;
    let dist = rounded_rect_sdf(
        vec2(frag_x, frag_y),
        vec2(content_left, vis_top),
        vec2(content_right, vis_bottom),
        corner_radius
    );
    if dist > 0.0 {
        discard;
    }

    // Fragment X -> audio time -> global CQT column
    let timeline_time = params.viewport_start_time + (frag_x - params.clip_rect.x) / params.pixels_per_second;
    let audio_time = timeline_time - params.clip_start_time + params.trim_start;

    if audio_time < 0.0 || audio_time > params.audio_duration {
        discard;
    }

    let global_col = audio_time * params.sample_rate / params.hop_size;

    // Check if this column is in the cached range
    if global_col < params.cache_valid_start || global_col >= params.cache_valid_end {
        discard;
    }

    // Fragment Y -> MIDI note -> CQT bin (direct mapping!)
    let note = params.max_note - ((frag_y - params.clip_rect.y + params.scroll_y) / params.note_height);

    if note < params.min_note || note > params.max_note {
        discard;
    }

    // CQT bin: each octave has bins_per_octave bins, starting from min_note
    let bin = (note - params.min_note) * params.bins_per_octave / 12.0;

    if bin < 0.0 || bin >= params.freq_bins {
        discard;
    }

    // Map global column to ring buffer position (accounting for stride)
    let ring_pos = (global_col - params.cache_start_column) / params.column_stride;
    let cache_x = ring_pos % params.cache_capacity;

    // Sample cache texture with bilinear filtering
    let u = (cache_x + 0.5) / params.cache_capacity;
    let v = (bin + 0.5) / params.freq_bins;
    let magnitude = textureSampleLevel(cache_tex, cache_sampler, vec2(u, v), 0.0).r;

    return colormap(magnitude, params.gamma);
}
