//! Integration tests for `VideoAudioReader` (FFmpeg streaming audio source).
//!
//! These build the daw-backend lib in normal mode, so they're independent of
//! the crate's pre-existing broken `#[cfg(test)]` unit tests (automation.rs).
//! They synthesize a mono 32-bit-float WAV whose sample `i` has value `i/n`, so
//! a decoded sample's value identifies its frame index — letting us assert both
//! in-order decoding and **sample-accurate seeking** (the property video audio
//! needs to stay synced with other clips).

use daw_backend::audio::disk_reader::{
    build_waveform_pyramid, CompressedReader, SourceKind, VideoAudioReader,
};
use std::io::Write;
use std::path::Path;

fn write_ramp_wav(path: &Path, n: u32, sample_rate: u32) {
    let channels = 1u16;
    let bytes_per_sample = 4u32;
    let data_size = n * bytes_per_sample;
    let mut buf: Vec<u8> = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * channels as u32 * bytes_per_sample).to_le_bytes());
    buf.extend_from_slice(&((channels as u32 * bytes_per_sample) as u16).to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..n {
        buf.extend_from_slice(&((i as f32) / (n as f32)).to_le_bytes());
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&buf).unwrap();
}

/// Stereo ramp: frame `i` has left = `i/n`, right = `0.5 - i/n` (distinct per
/// channel), interleaved `[L0,R0,L1,R1,…]`. Exercises the channels>1 path.
fn write_stereo_ramp_wav(path: &Path, n: u32, sample_rate: u32) {
    let channels = 2u16;
    let bytes_per_sample = 4u32;
    let data_size = n * channels as u32 * bytes_per_sample;
    let mut buf: Vec<u8> = Vec::with_capacity(44 + data_size as usize);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_size).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * channels as u32 * bytes_per_sample).to_le_bytes());
    buf.extend_from_slice(&((channels as u32 * bytes_per_sample) as u16).to_le_bytes());
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..n {
        let l = i as f32 / n as f32;
        let r = 0.5 - i as f32 / n as f32;
        buf.extend_from_slice(&l.to_le_bytes());
        buf.extend_from_slice(&r.to_le_bytes());
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(&buf).unwrap();
}

fn temp_path(tag: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("lb_videoaudio_test_{}_{}.wav", std::process::id(), tag));
    p
}

#[test]
fn decodes_samples_in_order() {
    let n = 4000u32;
    let sr = 8000u32;
    let path = temp_path("seq");
    write_ramp_wav(&path, n, sr);

    let mut reader = VideoAudioReader::open(&path).unwrap();
    assert_eq!(reader.channels(), 1);
    assert_eq!(reader.sample_rate(), sr);
    // Probe estimate (used by add_video_audio_sync) should be ~n frames.
    let tf = reader.total_frames() as f64;
    assert!(
        (tf - n as f64).abs() < n as f64 * 0.1,
        "total_frames {} not ~{}",
        tf, n
    );

    let mut all = Vec::new();
    let mut buf = Vec::new();
    loop {
        let frames = reader.decode_next(&mut buf).unwrap();
        if frames == 0 {
            break;
        }
        all.extend_from_slice(&buf);
    }

    // Allow a couple of priming/flush samples of slack at the very end.
    assert!(all.len() + 4 >= n as usize, "decoded too few samples: {}", all.len());
    for (i, &v) in all.iter().enumerate().take(n as usize) {
        let expected = i as f32 / n as f32;
        assert!((v - expected).abs() < 1e-3, "sample {} = {}, expected {}", i, v, expected);
    }
    let _ = std::fs::remove_file(&path);
}

/// CompressedReader (symphonia) must seek **sample-accurately** too, so compressed
/// audio stays frame-synced with video audio. Symphonia decodes WAV via the same
/// path; its coarse seek lands on packet boundaries, exercising the decode-discard.
#[test]
fn compressed_reader_seek_is_sample_accurate() {
    let n = 4000u32;
    let sr = 8000u32;
    let path = temp_path("comp_seek");
    write_ramp_wav(&path, n, sr);

    let mut reader = CompressedReader::open(&path).unwrap();
    assert_eq!(reader.channels(), 1);
    assert_eq!(reader.sample_rate(), sr);

    for &target in &[2000u64, 137, 3500, 0] {
        let actual = reader.seek(target).unwrap();
        assert_eq!(actual, target, "seek should report the exact target");

        let mut buf = Vec::new();
        let mut frames = 0;
        for _ in 0..128 {
            frames = reader.decode_next(&mut buf).unwrap();
            if frames > 0 {
                break;
            }
        }
        assert!(frames > 0, "no samples after seek to {}", target);
        let expected = target as f32 / n as f32;
        assert!(
            (buf[0] - expected).abs() < 1e-3,
            "compressed seek to {}: first sample = {}, expected {}",
            target, buf[0], expected
        );
    }
    let _ = std::fs::remove_file(&path);
}

/// The decode→pyramid bridge should produce an envelope matching the signal,
/// through both reader backends (symphonia + ffmpeg), with bounded memory.
#[test]
fn waveform_pyramid_from_decode_matches_signal() {
    let n = 5000u32;
    let sr = 8000u32;
    let path = temp_path("pyr");
    write_ramp_wav(&path, n, sr); // ramp 0 .. (n-1)/n, all positive

    for kind in [SourceKind::CompressedAudio, SourceKind::VideoAudio] {
        let p = build_waveform_pyramid(&path, kind, 256).unwrap();
        assert_eq!(p.channels, 1);
        assert_eq!(p.root().len(), 1, "{:?}: root should be one texel", kind);
        let root = p.root()[0];
        assert!(root.l_min.abs() < 1e-2, "{:?}: root min {} ~ 0", kind, root.l_min);
        let expected_max = (n - 1) as f32 / n as f32;
        assert!(
            (root.l_max - expected_max).abs() < 1e-2,
            "{:?}: root max {} ~ {}", kind, root.l_max, expected_max
        );
        // Frame count is approximate across decoders (priming/resampler overhead);
        // the envelope above is the real check. Just confirm it's about right.
        assert!((p.total_frames as i64 - n as i64).abs() < 128, "{:?}: frames {}", kind, p.total_frames);
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn decodes_stereo_interleaved() {
    let n = 2000u32;
    let sr = 8000u32;
    let path = temp_path("stereo");
    write_stereo_ramp_wav(&path, n, sr);

    let mut reader = VideoAudioReader::open(&path).unwrap();
    assert_eq!(reader.channels(), 2);

    let mut all = Vec::new();
    let mut buf = Vec::new();
    loop {
        let frames = reader.decode_next(&mut buf).unwrap();
        if frames == 0 {
            break;
        }
        // Each decode_next returns whole interleaved frames.
        assert_eq!(buf.len() % 2, 0, "stereo decode returned a partial frame");
        all.extend_from_slice(&buf);
    }

    // Interleaved L/R, ~n frames.
    assert!(all.len() + 8 >= (n * 2) as usize, "decoded too few samples: {}", all.len());
    for i in 0..n as usize {
        let l = all[2 * i];
        let r = all[2 * i + 1];
        assert!((l - i as f32 / n as f32).abs() < 1e-3, "L[{}]={} expected {}", i, l, i as f32 / n as f32);
        assert!(
            (r - (0.5 - i as f32 / n as f32)).abs() < 1e-3,
            "R[{}]={} expected {}", i, r, 0.5 - i as f32 / n as f32
        );
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn seek_is_sample_accurate() {
    let n = 4000u32;
    let sr = 8000u32;
    let path = temp_path("seek");
    write_ramp_wav(&path, n, sr);

    let mut reader = VideoAudioReader::open(&path).unwrap();

    for &target in &[2000u64, 137, 3500, 0] {
        let actual = reader.seek(target).unwrap();
        assert_eq!(actual, target);

        // Pull the first non-empty decode after the seek.
        let mut buf = Vec::new();
        let mut frames = 0;
        for _ in 0..64 {
            frames = reader.decode_next(&mut buf).unwrap();
            if frames > 0 {
                break;
            }
        }
        assert!(frames > 0, "no samples after seek to {}", target);

        let expected = target as f32 / n as f32;
        assert!(
            (buf[0] - expected).abs() < 1e-3,
            "after seek to {}: first sample = {}, expected {}",
            target,
            buf[0],
            expected
        );
        // And the next few advance in order.
        for k in 0..frames.min(8) {
            let exp = (target as usize + k) as f32 / n as f32;
            assert!((buf[k] - exp).abs() < 1e-3, "seek {}+{}: {} vs {}", target, k, buf[k], exp);
        }
    }
    let _ = std::fs::remove_file(&path);
}
