//! Step 1 of zero-copy: the custom Vulkan device with DMA-BUF import extensions builds
//! and can do a trivial GPU op. Skips (passes) when Vulkan is unavailable.

#![cfg(target_os = "linux")]

#[test]
fn drm_device_creates_and_works() {
    let dev = match gpu_video_encoder::vk_device::create() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[drm-device] unavailable, skipping: {e}");
            return;
        }
    };
    eprintln!("[drm-device] created custom Vulkan device OK");

    // Trivial sanity op: write+read a small buffer, proving the wrapped device is usable.
    let data: Vec<u8> = (0..256u32).map(|i| i as u8).collect();
    let src = wgpu::util::DeviceExt::create_buffer_init(
        &dev.device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("src"),
            contents: &data,
            usage: wgpu::BufferUsages::COPY_SRC,
        },
    );
    let dst = dev.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("dst"),
        size: 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = dev.device.create_command_encoder(&Default::default());
    enc.copy_buffer_to_buffer(&src, 0, &dst, 0, 256);
    dev.queue.submit(Some(enc.finish()));
    let slice = dst.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = dev.device.poll(wgpu::PollType::wait_indefinitely());
    let got = slice.get_mapped_range().to_vec();
    assert_eq!(got, data, "round-trip through custom device failed");
    eprintln!("[drm-device] buffer round-trip OK on custom device");
}
