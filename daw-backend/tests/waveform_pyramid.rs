//! Integration tests for the streaming waveform LOD pyramid builder.
//!
//! Convention B: `levels[0]` is the root (coarsest), `levels.last()` the floor
//! (finest). Tests use the `.root()` / `.floor()` accessors so they don't depend
//! on the raw index ordering.

use daw_backend::audio::waveform_pyramid::{Texel, WaveformPyramid, WaveformPyramidBuilder};

fn build_mono(samples: &[f32], floor: u32) -> WaveformPyramid {
    let mut b = WaveformPyramidBuilder::new(1, floor);
    b.push_interleaved(samples);
    b.finish()
}

#[test]
fn floor_level_min_max_per_bucket() {
    // 8 samples, floor 4 → two floor texels covering [0..4) and [4..8).
    let s: Vec<f32> = (0..8).map(|i| i as f32).collect();
    let p = build_mono(&s, 4);
    assert_eq!(p.floor().len(), 2);
    assert_eq!(p.floor()[0], Texel { l_min: 0.0, l_max: 3.0, r_min: 0.0, r_max: 3.0 });
    assert_eq!(p.floor()[1], Texel { l_min: 4.0, l_max: 7.0, r_min: 4.0, r_max: 7.0 });
    // Root reduces the two floor texels into the envelope [0..8).
    assert_eq!(p.root().len(), 1);
    assert_eq!(p.root()[0], Texel { l_min: 0.0, l_max: 7.0, r_min: 0.0, r_max: 7.0 });
}

#[test]
fn partial_trailing_bucket_is_flushed() {
    // 6 samples, floor 4 → texels [0..4) and a ragged [4..6).
    let s: Vec<f32> = (0..6).map(|i| i as f32).collect();
    let p = build_mono(&s, 4);
    assert_eq!(p.floor().len(), 2);
    assert_eq!(p.floor()[1], Texel { l_min: 4.0, l_max: 5.0, r_min: 4.0, r_max: 5.0 });
    assert_eq!(p.total_frames, 6);
}

#[test]
fn multi_level_envelope_matches_global_min_max() {
    let s: Vec<f32> = (0..1000).map(|i| ((i as f32) * 0.01).sin()).collect();
    let g_min = s.iter().cloned().fold(f32::INFINITY, f32::min);
    let g_max = s.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let p = build_mono(&s, 16);
    assert_eq!(p.root().len(), 1);
    assert!((p.root()[0].l_min - g_min).abs() < 1e-6);
    assert!((p.root()[0].l_max - g_max).abs() < 1e-6);
    // Every level's overall min/max equals the global (extremes are lossless).
    for level in &p.levels {
        let lmin = level.iter().map(|t| t.l_min).fold(f32::INFINITY, f32::min);
        let lmax = level.iter().map(|t| t.l_max).fold(f32::NEG_INFINITY, f32::max);
        assert!((lmin - g_min).abs() < 1e-6);
        assert!((lmax - g_max).abs() < 1e-6);
    }
}

#[test]
fn levels_are_root_first_and_get_finer() {
    let s: Vec<f32> = (0..1000).map(|i| i as f32).collect();
    let p = build_mono(&s, 16);
    // Root first, floor last; strictly finer (more texels) as depth increases.
    assert_eq!(p.root().len(), 1);
    assert!(p.depth() >= 3);
    for w in p.levels.windows(2) {
        assert!(w[1].len() >= w[0].len(), "deeper level should be finer");
    }
    // Floor has ceil(1000/16) = 63 texels.
    assert_eq!(p.floor().len(), 63);
}

#[test]
fn stereo_channels_tracked_separately() {
    // L ramps up, R ramps down; interleaved.
    let n = 64;
    let mut s = Vec::new();
    for i in 0..n {
        s.push(i as f32); // L
        s.push(-(i as f32)); // R
    }
    let mut b = WaveformPyramidBuilder::new(2, 16);
    b.push_interleaved(&s);
    let p = b.finish();
    assert_eq!(p.root().len(), 1);
    assert_eq!(p.root()[0].l_min, 0.0);
    assert_eq!(p.root()[0].l_max, (n - 1) as f32);
    assert_eq!(p.root()[0].r_min, -((n - 1) as f32));
    assert_eq!(p.root()[0].r_max, 0.0);
}

#[test]
fn pyramid_size_is_bounded() {
    let n = 100_000usize;
    let s: Vec<f32> = (0..n).map(|i| (i % 7) as f32).collect();
    let floor = 256u32;
    let p = build_mono(&s, floor);
    let total: usize = p.levels.iter().map(|l| l.len()).sum();
    let floor_texels = (n as u32).div_ceil(floor) as usize;
    // Geometric bound: < floor_texels * branch/(branch-1) + small per-level slack.
    let bound = floor_texels * 4 / 3 + p.depth() + 2;
    assert!(total <= bound, "pyramid too big: {} > {}", total, bound);
}

#[test]
fn bytes_round_trip() {
    let s: Vec<f32> = (0..3333).map(|i| ((i as f32) * 0.013).sin()).collect();
    let p = build_mono(&s, 64);
    let bytes = p.to_bytes();
    let q = WaveformPyramid::from_bytes(&bytes).unwrap();
    assert_eq!(p.floor_samples_per_texel, q.floor_samples_per_texel);
    assert_eq!(p.branch, q.branch);
    assert_eq!(p.channels, q.channels);
    assert_eq!(p.total_frames, q.total_frames);
    assert_eq!(p.levels, q.levels);
    // Truncated/garbage input is rejected, not panicking.
    assert!(WaveformPyramid::from_bytes(&bytes[..bytes.len() - 4]).is_err());
    assert!(WaveformPyramid::from_bytes(b"nope").is_err());
}

#[test]
fn pushing_in_arbitrary_chunks_matches() {
    // The streaming builder must be agnostic to how samples are chunked.
    let s: Vec<f32> = (0..5000).map(|i| ((i * 13) % 97) as f32 - 48.0).collect();
    let whole = build_mono(&s, 32);

    let mut b = WaveformPyramidBuilder::new(1, 32);
    b.reserve_for_frames(5000);
    for chunk in s.chunks(37) {
        b.push_interleaved(chunk);
    }
    let chunked = b.finish();

    assert_eq!(whole.depth(), chunked.depth());
    for (a, c) in whole.levels.iter().zip(chunked.levels.iter()) {
        assert_eq!(a, c);
    }
}
