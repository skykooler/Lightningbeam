//! Level-1 spike: prove `h264_vaapi` encodes NV12 in this environment. Skips (passes)
//! when VAAPI isn't available so it's a no-op on CI/macOS/Windows.

#![cfg(target_os = "linux")]

use gpu_video_encoder::nv12::{cpu_reference, nv12_len};
use gpu_video_encoder::vaapi::encode_nv12_to_file;

/// A moving-gradient RGBA pattern → NV12 via the CPU reference, so we feed valid frames.
fn nv12_frames(w: u32, h: u32, n: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|f| {
            let mut rgba = Vec::with_capacity((w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    rgba.push(((x + f as u32 * 4) % 256) as u8);
                    rgba.push(((y + f as u32 * 2) % 256) as u8);
                    rgba.push(((x + y) % 256) as u8);
                    rgba.push(255);
                }
            }
            let v = cpu_reference(&rgba, w, h);
            assert_eq!(v.len(), nv12_len(w, h));
            v
        })
        .collect()
}

#[test]
fn vaapi_surface_drm_layout() {
    match gpu_video_encoder::vaapi::probe_surface_drm(1920, 1088) {
        Ok(s) => eprintln!("[vaapi-drm]\n{s}"),
        Err(e) => eprintln!("[vaapi-drm] unavailable, skipping: {e}"),
    }
}

#[test]
fn vaapi_h264_encode_smoke() {
    let (w, h) = (320u32, 240u32);
    let frames = nv12_frames(w, h, 30);
    let out = std::env::temp_dir().join("gpu_video_encoder_vaapi_smoke.h264");
    let out_str = out.to_str().unwrap();

    match encode_nv12_to_file(w, h, &frames, 30, out_str) {
        Ok(packets) => {
            let meta = std::fs::metadata(&out).expect("output file missing");
            eprintln!(
                "[vaapi] encoded {} packets, {} bytes -> {}",
                packets,
                meta.len(),
                out_str
            );
            assert!(packets > 0, "no packets produced");
            assert!(meta.len() > 0, "empty output file");
            // First frame should be an IDR; Annex-B starts with a start code.
            let head = std::fs::read(&out).unwrap();
            assert!(
                head.starts_with(&[0, 0, 0, 1]) || head.starts_with(&[0, 0, 1]),
                "output is not Annex-B H.264 (no start code)"
            );
        }
        Err(e) => {
            eprintln!("[vaapi] unavailable, skipping: {e}");
        }
    }
}
