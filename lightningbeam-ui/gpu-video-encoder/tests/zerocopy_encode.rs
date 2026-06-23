//! Capstone: encode RGBA frames fully zero-copy (GPU render → VAAPI surface → h264_vaapi)
//! and verify the output is real H.264. Skips when VAAPI is unavailable.

#![cfg(target_os = "linux")]

use gpu_video_encoder::encoder::ZeroCopyEncoder;

#[test]
fn zerocopy_encode_h264() {
    let (w, h) = (640u32, 480u32);
    let mut enc = match ZeroCopyEncoder::new(w, h, 30, 4000) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("[zc-encode] unavailable, skipping: {e}");
            return;
        }
    };

    // Build one reusable RGBA source texture; update it per frame with a moving pattern.
    let device = enc.device();
    let src = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rgba"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    let n = 30;
    for f in 0..n {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.push(((x + f * 8) % 256) as u8);
                rgba.push(((y + f * 4) % 256) as u8);
                rgba.push(((x + y) % 256) as u8);
                rgba.push(255);
            }
        }
        enc.queue().write_texture(
            wgpu::TexelCopyTextureInfo { texture: &src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        enc.encode_rgba(&src).expect("encode_rgba");
    }

    let h264 = enc.finish().expect("finish");
    eprintln!("[zc-encode] {} frames -> {} bytes H.264", n, h264.len());
    assert!(h264.len() > 1000, "implausibly small output");
    assert!(
        h264.starts_with(&[0, 0, 0, 1]) || h264.starts_with(&[0, 0, 1]),
        "not Annex-B H.264"
    );

    // Write it out and ffprobe-verify if ffprobe is present.
    let out = std::env::temp_dir().join("gpu_video_encoder_zerocopy.h264");
    std::fs::write(&out, &h264).unwrap();
    eprintln!("[zc-encode] wrote {}", out.display());
    if let Ok(o) = std::process::Command::new("ffprobe")
        .args(["-hide_banner", "-v", "error", "-show_entries", "stream=codec_name,width,height", "-of", "default=noprint_wrappers=1"])
        .arg(&out)
        .output()
    {
        let s = String::from_utf8_lossy(&o.stdout);
        eprintln!("[zc-encode] ffprobe:\n{s}");
        assert!(s.contains("codec_name=h264"), "ffprobe didn't see H.264");
        assert!(s.contains(&format!("width={w}")), "wrong width");
    }
    eprintln!("[zc-encode] ✅ zero-copy H.264 encode verified");
}
