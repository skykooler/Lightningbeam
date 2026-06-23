//! End-to-end zero-copy proof: import a VAAPI NV12 surface as wgpu textures, render
//! known values into them via Vulkan, read the surface back, and verify the bytes —
//! proving the GPU wrote straight into the encoder's surface with no CPU upload.

#![cfg(target_os = "linux")]

use gpu_video_encoder::{dmabuf, vaapi, vk_device};

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
    let y_view = imported.y.create_view(&Default::default());
    let uv_view = imported.uv.create_view(&Default::default());
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
