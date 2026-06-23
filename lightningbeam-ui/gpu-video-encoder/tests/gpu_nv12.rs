//! Real-hardware test: run the RGBA→NV12 compute on the GPU and check it byte-matches
//! the CPU reference. Skips (passes) if no GPU adapter is available.

use gpu_video_encoder::nv12::{cpu_reference, nv12_len, Nv12Converter};

fn device_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("nv12-test"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        ..Default::default()
    }))
    .ok()
}

/// A deterministic, varied RGBA pattern so luma and 2x2 chroma subsampling are exercised.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.push(((x * 37 + y * 11) % 256) as u8); // R
            v.push(((x * 5 + y * 53) % 256) as u8); // G
            v.push(((x * 97 + y * 17) % 256) as u8); // B
            v.push(255);
        }
    }
    v
}

#[test]
fn gpu_nv12_matches_cpu_reference() {
    let Some((device, queue)) = device_queue() else {
        eprintln!("[gpu_nv12] no GPU adapter; skipping");
        return;
    };

    let (w, h) = (64u32, 16u32);
    let rgba = pattern(w, h);

    // Source RGBA texture.
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("src_rgba"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    let view = tex.create_view(&Default::default());

    let len = nv12_len(w, h) as u64;
    let out = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nv12_out"),
        size: len,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("nv12_staging"),
        size: len,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let conv = Nv12Converter::new(&device);
    let mut enc = device.create_command_encoder(&Default::default());
    conv.convert(&device, &mut enc, &view, &out, w, h);
    enc.copy_buffer_to_buffer(&out, 0, &staging, 0, len);
    queue.submit(Some(enc.finish()));

    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let gpu = slice.get_mapped_range().to_vec();

    let cpu = cpu_reference(&rgba, w, h);
    assert_eq!(gpu.len(), cpu.len(), "length mismatch");

    // Allow ±1 for rounding differences between GPU and CPU float paths.
    let mut max_diff = 0i32;
    let mut nbad = 0;
    for (i, (g, c)) in gpu.iter().zip(cpu.iter()).enumerate() {
        let d = (*g as i32 - *c as i32).abs();
        max_diff = max_diff.max(d);
        if d > 1 {
            nbad += 1;
            if nbad <= 8 {
                eprintln!("[gpu_nv12] byte {i}: gpu={g} cpu={c} (diff {d})");
            }
        }
    }
    eprintln!("[gpu_nv12] {}x{} NV12, max byte diff = {max_diff}", w, h);
    assert_eq!(nbad, 0, "{nbad} bytes differ from CPU reference by >1");
}
