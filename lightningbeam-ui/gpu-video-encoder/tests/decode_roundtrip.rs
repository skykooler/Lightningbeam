//! Round-trip: encode solid frames with the zero-copy encoder, then hardware-decode them back
//! into a wgpu texture and read the Y plane. Verifies the VAAPI decode → DMA-BUF → wgpu import
//! path produces real pixels on the GPU. Skips when VAAPI is unavailable.

#![cfg(target_os = "linux")]

use gpu_video_encoder::decoder::VaapiDecoder;
use gpu_video_encoder::encoder::ZeroCopyEncoder;

#[test]
fn vaapi_decode_roundtrip() {
    // 256-wide so the R8 Y readback row (256 B) is already 256-aligned.
    let (w, h) = (256u32, 256u32);
    let out = std::env::temp_dir().join("gpu_video_encoder_decode_rt.mp4");
    let _ = std::fs::remove_file(&out);

    // --- Encode 10 frames of solid mid-gray. Full range → Y == luma ≈ 128. ---
    {
        let mut enc = match ZeroCopyEncoder::new(w, h, 30, 4000, &out, true) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[decode-rt] encode unavailable, skipping: {e}");
                return;
            }
        };
        let device = enc.device();
        let src = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gray"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let gray = vec![128u8; (w * h * 4) as usize];
        enc.queue().write_texture(
            wgpu::TexelCopyTextureInfo { texture: &src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &gray,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        for _ in 0..10 {
            enc.encode_rgba(&src).expect("encode_rgba");
        }
        enc.finish().expect("finish");
    }

    // --- Decode it back on the GPU. ---
    let mut dec = match VaapiDecoder::new(&out) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[decode-rt] decode unavailable, skipping: {e}");
            return;
        }
    };
    let frame = dec.next_frame().expect("next_frame").expect("expected at least one frame");
    assert_eq!(frame.y().width(), w, "decoded Y width");
    assert_eq!(frame.y().height(), h, "decoded Y height");

    // Read back the Y plane (R8) and check it's ≈ the gray we encoded.
    let device = dec.device();
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("y_readback"),
        size: (w * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut cmd = device.create_command_encoder(&Default::default());
    cmd.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: frame.y(), mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo {
            buffer: &buf,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
        },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    dec.queue().submit(Some(cmd.finish()));
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    let data = buf.slice(..).get_mapped_range();
    let mean = data.iter().map(|&b| b as f64).sum::<f64>() / data.len() as f64;
    eprintln!("[decode-rt] decoded {w}x{h}, mean Y = {mean:.1}");
    assert!(
        (mean - 128.0).abs() < 12.0,
        "mean Y {mean} not ≈ 128 — decode produced wrong pixels"
    );
    eprintln!("[decode-rt] ✅ VAAPI decode → wgpu texture verified");
}
