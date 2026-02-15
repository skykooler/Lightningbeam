// GPU Constant-Q Transform (CQT) compute shader.
//
// Reads raw audio samples from a waveform mip-0 texture (Rgba16Float, packed
// row-major at TEX_WIDTH=2048) and computes CQT magnitude for each
// (freq_bin, time_column) pair, writing normalized dB values into a ring-buffer
// cache texture (R32Float, width=cache_capacity, height=freq_bins).
//
// Dispatch: (ceil(freq_bins / 64), num_columns, 1)
// Each thread handles one frequency bin for one time column.

struct CqtParams {
    hop_size: u32,
    freq_bins: u32,
    cache_capacity: u32,
    cache_write_offset: u32,  // ring buffer position to start writing
    num_columns: u32,         // how many columns in this dispatch
    column_start: u32,        // global CQT column index of first column
    tex_width: u32,           // waveform texture width (2048)
    total_frames: u32,        // total audio frames in waveform texture
    sample_rate: f32,
    column_stride: u32,
    _pad1: u32,
    _pad2: u32,
}

struct BinInfo {
    window_length: u32,
    phase_step: f32, // 2*pi*Q / N_k
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var audio_tex: texture_2d<f32>;
@group(0) @binding(1) var cqt_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: CqtParams;
@group(0) @binding(3) var<storage, read> bins: array<BinInfo>;

const PI2: f32 = 6.283185307;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let bin_k = gid.x;
    let col_rel = gid.y; // relative to this dispatch batch

    if bin_k >= params.freq_bins || col_rel >= params.num_columns {
        return;
    }

    let global_col = params.column_start + col_rel * params.column_stride;
    let sample_start = global_col * params.hop_size;

    let info = bins[bin_k];
    let n_k = info.window_length;

    // Center the analysis window: offset by half the window length so the
    // column timestamp refers to the center of the window, not the start.
    // This gives better time alignment, especially for low-frequency bins
    // that have very long windows.
    let half_win = n_k / 2u;

    // Accumulate complex inner product: sum of x[n] * w[n] * exp(-i * phase_step * n)
    var sum_re: f32 = 0.0;
    var sum_im: f32 = 0.0;

    for (var n = 0u; n < n_k; n++) {
        // Center the window around the hop position
        let raw_idx = i32(sample_start) + i32(n) - i32(half_win);
        if raw_idx < 0 || u32(raw_idx) >= params.total_frames {
            continue;
        }
        let sample_idx = u32(raw_idx);

        // Read audio sample from 2D waveform texture (mip 0)
        // At mip 0: R=G=left, B=A=right; average to mono
        let tx = sample_idx % params.tex_width;
        let ty = sample_idx / params.tex_width;
        let texel = textureLoad(audio_tex, vec2<i32>(i32(tx), i32(ty)), 0);
        let sample_val = (texel.r + texel.b) * 0.5;

        // Hann window computed analytically
        let window = 0.5 * (1.0 - cos(PI2 * f32(n) / f32(n_k)));

        // Complex exponential: exp(-i * phase_step * n)
        let angle = info.phase_step * f32(n);
        let windowed = sample_val * window;
        sum_re += windowed * cos(angle);
        sum_im -= windowed * sin(angle);
    }

    // Magnitude, normalized by window length
    let mag = sqrt(sum_re * sum_re + sum_im * sum_im) / f32(n_k);

    // Convert to dB, map -80dB..0dB -> 0.0..1.0
    // WGSL log() is natural log, so log10(x) = log(x) / log(10)
    let db = 20.0 * log(mag + 1e-10) / 2.302585093;
    let normalized = clamp((db + 80.0) / 80.0, 0.0, 1.0);

    // Write to ring buffer cache texture
    let cache_x = (params.cache_write_offset + col_rel) % params.cache_capacity;
    textureStore(cqt_out, vec2<i32>(i32(cache_x), i32(bin_k)), vec4(normalized, 0.0, 0.0, 1.0));
}
