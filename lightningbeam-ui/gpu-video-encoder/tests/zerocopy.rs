//! End-to-end zero-copy proof: import a VAAPI NV12 surface as wgpu textures, render
//! known values into them via Vulkan, read the surface back, and verify the bytes —
//! proving the GPU wrote straight into the encoder's surface with no CPU upload.

#![cfg(target_os = "linux")]

use gpu_video_encoder::{dmabuf, nv12, render_nv12, vaapi, vk_device};

/// Render a real RGBA frame into the VAAPI surface (zero-copy) and verify the surface's
/// NV12 matches the CPU reference for that frame.
#[test]
fn zerocopy_real_frame_render() {
    let drm = match vk_device::create() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[zerocopy-real] no Vulkan, skipping: {e}");
            return;
        }
    };
    let (w, h) = (640u32, 480u32);
    let surf = match vaapi::MappedSurface::alloc(w, h) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[zerocopy-real] no VAAPI, skipping: {e}");
            return;
        }
    };
    let imported = dmabuf::import(&drm, &surf).expect("import");

    // A varied RGBA pattern.
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.push(((x * 3 + y) % 256) as u8);
            rgba.push(((x + y * 2) % 256) as u8);
            rgba.push(((x * 2 + y * 3) % 256) as u8);
            rgba.push(255);
        }
    }
    let src = drm.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("rgba_src"),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    drm.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        &rgba,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );

    let conv = render_nv12::Rgba2Nv12::new(&drm.device);
    let src_view = src.create_view(&Default::default());
    let y_view = imported.y().create_view(&Default::default());
    let uv_view = imported.uv().create_view(&Default::default());
    let mut enc = drm.device.create_command_encoder(&Default::default());
    conv.convert(&drm.device, &mut enc, &src_view, &y_view, &uv_view);
    drm.queue.submit(Some(enc.finish()));
    let _ = drm.device.poll(wgpu::PollType::wait_indefinitely());

    let got = surf.readback_nv12().expect("readback");
    let want = nv12::cpu_reference(&rgba, w, h);
    assert_eq!(got.len(), want.len());
    let mut max_diff = 0i32;
    let mut nbad = 0;
    for (g, c) in got.iter().zip(want.iter()) {
        let d = (*g as i32 - *c as i32).abs();
        max_diff = max_diff.max(d);
        if d > 2 {
            nbad += 1;
        }
    }
    eprintln!("[zerocopy-real] {}x{} real-frame render, max diff={max_diff}, bad={nbad}/{}", w, h, got.len());
    assert!(nbad * 100 < got.len(), "too many bytes differ from CPU NV12 reference");
    eprintln!("[zerocopy-real] ✅ real RGBA frame rendered into VAAPI surface, NV12 matches reference");
}

#[test]
fn zerocopy_render_into_vaapi_surface() {
    let drm = match vk_device::create() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[zerocopy] no Vulkan device, skipping: {e}");
            return;
        }
    };
    let surf = match vaapi::MappedSurface::alloc(640, 480) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[zerocopy] no VAAPI surface, skipping: {e}");
            return;
        }
    };
    eprintln!(
        "[zerocopy] surface: modifier=0x{:016x} y(off={},pitch={}) uv(off={},pitch={}) size={}",
        surf.modifier, surf.y_offset, surf.y_pitch, surf.uv_offset, surf.uv_pitch, surf.size
    );

    let imported = match dmabuf::import(&drm, &surf) {
        Ok(i) => i,
        Err(e) => panic!("dma-buf import failed: {e}"),
    };
    eprintln!("[zerocopy] imported surface as wgpu Y(R8) + UV(RG8) textures");

    // Render known constants via clear: Y=0.5(->128), U=0.25(->64), V=0.75(->191).
    let y_view = imported.y().create_view(&Default::default());
    let uv_view = imported.uv().create_view(&Default::default());
    let mut enc = drm.device.create_command_encoder(&Default::default());
    {
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-y"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &y_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.5, g: 0.0, b: 0.0, a: 0.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear-uv"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &uv_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.25, g: 0.75, b: 0.0, a: 0.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }
    drm.queue.submit(Some(enc.finish()));
    let _ = drm.device.poll(wgpu::PollType::wait_indefinitely());

    // Read the VAAPI surface back and check what the GPU wrote.
    let nv12 = surf.readback_nv12().expect("readback");
    let (w, h) = (640usize, 480usize);
    let y_plane = &nv12[..w * h];
    let uv_plane = &nv12[w * h..];

    let near = |v: u8, t: i32| (v as i32 - t).abs() <= 3;
    let y_ok = y_plane.iter().filter(|&&v| near(v, 128)).count();
    let u_ok = uv_plane.iter().step_by(2).filter(|&&v| near(v, 64)).count();
    let v_ok = uv_plane.iter().skip(1).step_by(2).filter(|&&v| near(v, 191)).count();
    eprintln!(
        "[zerocopy] Y~128: {}/{}, U~64: {}/{}, V~191: {}/{}",
        y_ok, w * h, u_ok, uv_plane.len() / 2, v_ok, uv_plane.len() / 2
    );

    let frac = |ok: usize, n: usize| ok as f64 / n as f64;
    assert!(frac(y_ok, w * h) > 0.98, "Y plane not the rendered value (sample {:?})", &y_plane[..8]);
    assert!(frac(u_ok, uv_plane.len() / 2) > 0.98, "U not rendered value");
    assert!(frac(v_ok, uv_plane.len() / 2) > 0.98, "V not rendered value");
    eprintln!("[zerocopy] ✅ GPU rendered straight into the VAAPI surface (verified via readback)");
}
