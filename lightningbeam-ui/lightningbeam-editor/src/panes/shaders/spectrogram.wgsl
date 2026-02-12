// Spectrogram rendering shader for FFT magnitude data.
// Texture layout: X = frequency bin, Y = time bin
// Values: normalized magnitude (0.0 = silence, 1.0 = peak)
// Vertical axis maps MIDI notes to frequency bins (matching piano roll)

struct Params {
    clip_rect: vec4<f32>,
    viewport_start_time: f32,
    pixels_per_second: f32,
    audio_duration: f32,
    sample_rate: f32,
    clip_start_time: f32,
    trim_start: f32,
    time_bins: f32,
    freq_bins: f32,
    hop_size: f32,
    fft_size: f32,
    scroll_y: f32,
    note_height: f32,
    screen_size: vec2<f32>,
    min_note: f32,
    max_note: f32,
    gamma: f32,
}

@group(0) @binding(0) var spec_tex: texture_2d<f32>;
@group(0) @binding(1) var spec_sampler: sampler;
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

// Signed distance from point to rounded rectangle boundary
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
        // Black -> blue
        let s = t * 6.0;
        return vec4(0.0, 0.0, s, 1.0);
    } else if t < 2.0 / 6.0 {
        // Blue -> purple
        let s = (t - 1.0 / 6.0) * 6.0;
        return vec4(s * 0.6, 0.0, 1.0 - s * 0.2, 1.0);
    } else if t < 3.0 / 6.0 {
        // Purple -> red
        let s = (t - 2.0 / 6.0) * 6.0;
        return vec4(0.6 + s * 0.4, 0.0, 0.8 - s * 0.8, 1.0);
    } else if t < 4.0 / 6.0 {
        // Red -> orange
        let s = (t - 3.0 / 6.0) * 6.0;
        return vec4(1.0, s * 0.5, 0.0, 1.0);
    } else if t < 5.0 / 6.0 {
        // Orange -> yellow
        let s = (t - 4.0 / 6.0) * 6.0;
        return vec4(1.0, 0.5 + s * 0.5, 0.0, 1.0);
    } else {
        // Yellow -> white
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

    // Rounded corners: content edges on X, visible viewport edges on Y.
    // This rounds left/right where the clip starts/ends, and top/bottom at the view boundary.
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

    // Fragment X -> audio time -> time bin
    let timeline_time = params.viewport_start_time + (frag_x - params.clip_rect.x) / params.pixels_per_second;
    let audio_time = timeline_time - params.clip_start_time + params.trim_start;

    if audio_time < 0.0 || audio_time > params.audio_duration {
        discard;
    }

    let time_bin = audio_time * params.sample_rate / params.hop_size;
    if time_bin < 0.0 || time_bin >= params.time_bins {
        discard;
    }

    // Fragment Y -> MIDI note -> frequency -> frequency bin
    let note = params.max_note - ((frag_y - params.clip_rect.y + params.scroll_y) / params.note_height);

    if note < params.min_note || note > params.max_note {
        discard;
    }

    // MIDI note -> frequency: freq = 440 * 2^((note - 69) / 12)
    let freq = 440.0 * pow(2.0, (note - 69.0) / 12.0);

    // Frequency -> FFT bin index
    let freq_bin = freq * params.fft_size / params.sample_rate;

    if freq_bin < 0.0 || freq_bin >= params.freq_bins {
        discard;
    }

    // Sample texture with bilinear filtering
    let u = freq_bin / params.freq_bins;
    let v = time_bin / params.time_bins;
    let magnitude = textureSampleLevel(spec_tex, spec_sampler, vec2(u, v), 0.0).r;

    return colormap(magnitude, params.gamma);
}
