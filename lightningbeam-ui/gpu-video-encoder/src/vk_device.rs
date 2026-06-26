//! Custom wgpu Vulkan device that additionally enables `VK_EXT_image_drm_format_modifier`
//! (plus the external-memory extensions wgpu-hal already turns on), so we can import a
//! tiled VAAPI NV12 DMA-BUF as a Vulkan image. wgpu's safe API can't add arbitrary device
//! extensions, so we build the `VkDevice` ourselves and wrap it via `device_from_raw`.
//!
//! All `unsafe` is contained here. Returns owned handles the caller must keep alive
//! together (instance → adapter → device/queue).

use ash::vk;
use std::ffi::CStr;

/// A wgpu device/queue backed by a hand-built Vulkan device with DMA-BUF import enabled.
pub struct DrmDevice {
    // Order matters for drop; wgpu handles refcount internally but we keep these owned.
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub adapter: wgpu::Adapter,
    pub instance: wgpu::Instance,
    /// The raw VkDevice (for the ash image-import calls in `dmabuf.rs`).
    pub raw_device: ash::Device,
    pub raw_physical_device: vk::PhysicalDevice,
    pub raw_instance: ash::Instance,
}

/// Create a headless DMA-BUF-import device (encoder/decoder), or `Err` if Vulkan/the extension
/// isn't available (caller falls back).
pub fn create() -> Result<DrmDevice, String> {
    unsafe { create_inner(false) }
}

/// Like [`create`] but also enables `VK_KHR_swapchain` so the device can present to a window —
/// for use as the editor's **shared** wgpu device (eframe + compositor + decode + encode all on
/// one device, so hardware-decoded DMA-BUF textures are usable by the preview compositor).
pub fn create_windowed() -> Result<DrmDevice, String> {
    unsafe { create_inner(true) }
}

unsafe fn create_inner(windowed: bool) -> Result<DrmDevice, String> {
    use wgpu_hal::vulkan::Api as Vk;
    // Bring the HAL Instance trait into scope for `init` / `enumerate_adapters`.
    use wgpu_hal::Instance as _;

    // 1. HAL instance.
    let hal_instance = wgpu_hal::vulkan::Instance::init(&wgpu_hal::InstanceDescriptor {
        name: "gpu-video-encoder",
        flags: wgpu::InstanceFlags::empty(),
        memory_budget_thresholds: Default::default(),
        backend_options: Default::default(),
    })
    .map_err(|e| format!("vulkan instance init failed: {e:?}"))?;

    let ash_instance = hal_instance.shared_instance().raw_instance().clone();

    // 2. Pick an adapter (prefer the integrated/discrete GPU).
    let mut exposed_adapters = hal_instance.enumerate_adapters(None);
    if exposed_adapters.is_empty() {
        return Err("no Vulkan adapters".into());
    }
    // Prefer a real GPU over CPU/llvmpipe.
    exposed_adapters.sort_by_key(|a| match a.info.device_type {
        wgpu::DeviceType::DiscreteGpu => 0,
        wgpu::DeviceType::IntegratedGpu => 1,
        _ => 2,
    });
    let exposed = exposed_adapters.into_iter().next().unwrap();
    let phys = exposed.adapter.raw_physical_device();

    // 3. Queue family with graphics + compute.
    let qf_props = ash_instance.get_physical_device_queue_family_properties(phys);
    let family_index = qf_props
        .iter()
        .position(|p| {
            p.queue_flags
                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        })
        .ok_or("no graphics+compute queue family")? as u32;

    // 4. Extensions: what wgpu-hal wants + DRM modifier import set.
    let mut ext_names: Vec<&'static CStr> =
        exposed.adapter.required_device_extensions(exposed.features);
    // Only the genuine extensions; external_memory / bind_memory2 / ycbcr / format_list
    // are core in Vulkan 1.1+ (this device is 1.3) so they need no enabling.
    let mut extra: Vec<&'static CStr> = vec![
        ash::ext::image_drm_format_modifier::NAME,
        ash::khr::external_memory_fd::NAME,
        ash::ext::external_memory_dma_buf::NAME,
        ash::ext::queue_family_foreign::NAME,
    ];
    // Presentation (windowed shared device only): the WSI surface instance extensions are already
    // enabled by `Instance::init`; the device needs the swapchain extension to present.
    if windowed {
        extra.push(ash::khr::swapchain::NAME);
    }
    for e in extra {
        if !ext_names.contains(&e) {
            ext_names.push(e);
        }
    }
    let ext_ptrs: Vec<*const i8> = ext_names.iter().map(|c| c.as_ptr()).collect();

    // 5. Enable all supported physical-device features (so wgpu has what it needs) plus
    //    sampler YCbCr conversion (required for the NV12 multi-planar image).
    let supported = ash_instance.get_physical_device_features(phys);
    let mut ycbcr =
        vk::PhysicalDeviceSamplerYcbcrConversionFeatures::default().sampler_ycbcr_conversion(true);

    let priorities = [1.0f32];
    let queue_info = vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family_index)
        .queue_priorities(&priorities);
    let queue_infos = [queue_info];

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_infos)
        .enabled_extension_names(&ext_ptrs)
        .enabled_features(&supported)
        .push_next(&mut ycbcr);

    let ash_device = ash_instance
        .create_device(phys, &create_info, None)
        .map_err(|e| format!("vkCreateDevice failed: {e:?}"))?;

    // 6. Wrap the raw device into a hal OpenDevice, then a wgpu device.
    let open_device = exposed
        .adapter
        .device_from_raw(
            ash_device.clone(),
            None,
            &ext_names,
            exposed.features,
            &wgpu::MemoryHints::default(),
            family_index,
            0,
        )
        .map_err(|e| format!("device_from_raw failed: {e:?}"))?;

    let raw_physical_device = phys;

    let wgpu_instance = wgpu::Instance::from_hal::<Vk>(hal_instance);
    let wgpu_adapter = wgpu_instance.create_adapter_from_hal::<Vk>(exposed);
    let (device, queue) = wgpu_adapter
        .create_device_from_hal::<Vk>(
            open_device,
            &wgpu::DeviceDescriptor {
                label: Some("drm-import-device"),
                // R16/Rg16 plane textures for P010 (10-bit HDR) import need this; request it only
                // when the adapter supports it (else 10-bit falls back to software decode).
                required_features: wgpu_adapter.features()
                    & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
                // Vello's compute pipelines need more than downlevel limits (e.g.
                // max_storage_buffers_per_shader_stage >= 5). This device only ever runs on a
                // real VAAPI-capable GPU, so request the adapter's full limits.
                required_limits: wgpu_adapter.limits(),
                ..Default::default()
            },
        )
        .map_err(|e| format!("create_device_from_hal failed: {e:?}"))?;

    Ok(DrmDevice {
        device,
        queue,
        adapter: wgpu_adapter,
        instance: wgpu_instance,
        raw_device: ash_device,
        raw_physical_device,
        raw_instance: ash_instance,
    })
}
