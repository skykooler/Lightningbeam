//! Import a tiled VAAPI NV12 DMA-BUF as two wgpu textures (Y = R8, UV = RG8), aliasing
//! the one imported `VkDeviceMemory` at the plane offsets. Two single-format images are
//! used instead of one multi-planar image so each is an ordinary wgpu render target.
//!
//! Spike-grade: leaks the VkImages/memory on drop (process-scoped test). Cleanup
//! ordering (textures before memory) is a follow-up.

use crate::vaapi::MappedSurface;
use crate::vk_device::DrmDevice;
use ash::vk;

pub struct ImportedNv12 {
    /// Luma plane, `R8Unorm`, full resolution.
    pub y: wgpu::Texture,
    /// Chroma plane, `Rg8Unorm`, half resolution (interleaved U,V).
    pub uv: wgpu::Texture,
}

pub fn import(drm: &DrmDevice, surf: &MappedSurface) -> Result<ImportedNv12, String> {
    unsafe {
        let device = &drm.raw_device;
        let instance = &drm.raw_instance;

        let dup_fd = libc::dup(surf.fd);
        if dup_fd < 0 {
            return Err("dup(dma-buf fd) failed".into());
        }

        // --- create a single-plane DRM-modifier image ---
        let make_image = |format: vk::Format, w: u32, h: u32, pitch: u64| -> Result<vk::Image, String> {
            let mut ext = vk::ExternalMemoryImageCreateInfo::default()
                .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
            let plane_layouts = [vk::SubresourceLayout::default().offset(0).row_pitch(pitch)];
            let mut drm_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
                .drm_format_modifier(surf.modifier)
                .plane_layouts(&plane_layouts);
            let info = vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D { width: w, height: h, depth: 1 })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
                .usage(
                    vk::ImageUsageFlags::COLOR_ATTACHMENT
                        | vk::ImageUsageFlags::TRANSFER_SRC
                        | vk::ImageUsageFlags::TRANSFER_DST,
                )
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .push_next(&mut ext)
                .push_next(&mut drm_info);
            device
                .create_image(&info, None)
                .map_err(|e| format!("vkCreateImage(modifier) failed: {e:?}"))
        };

        let img_y = make_image(vk::Format::R8_UNORM, surf.width, surf.height, surf.y_pitch)?;
        let img_uv = make_image(vk::Format::R8G8_UNORM, surf.width / 2, surf.height / 2, surf.uv_pitch)?;

        // --- import the dma-buf as one VkDeviceMemory, bind both planes ---
        let fd_dev = ash::khr::external_memory_fd::Device::new(instance, device);
        let mut fd_props = vk::MemoryFdPropertiesKHR::default();
        fd_dev
            .get_memory_fd_properties(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT, dup_fd, &mut fd_props)
            .map_err(|e| format!("vkGetMemoryFdPropertiesKHR failed: {e:?}"))?;

        let req_y = device.get_image_memory_requirements(img_y);
        let req_uv = device.get_image_memory_requirements(img_uv);
        let type_bits = fd_props.memory_type_bits & req_y.memory_type_bits & req_uv.memory_type_bits;
        if type_bits == 0 {
            return Err("no memory type compatible with dma-buf + both plane images".into());
        }
        let mem_type = type_bits.trailing_zeros();

        let mut import_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(dup_fd);
        let alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(surf.size)
            .memory_type_index(mem_type)
            .push_next(&mut import_info);
        let memory = device
            .allocate_memory(&alloc, None)
            .map_err(|e| format!("vkAllocateMemory(import dma-buf) failed: {e:?}"))?;

        device
            .bind_image_memory(img_y, memory, surf.y_offset)
            .map_err(|e| format!("bind Y plane: {e:?}"))?;
        device
            .bind_image_memory(img_uv, memory, surf.uv_offset)
            .map_err(|e| format!("bind UV plane: {e:?}"))?;

        // --- wrap each VkImage as a wgpu texture ---
        let hal_device = drm
            .device
            .as_hal::<wgpu_hal::vulkan::Api>()
            .ok_or("device is not Vulkan")?;

        let wrap = |img: vk::Image, format: wgpu::TextureFormat, w: u32, h: u32| -> wgpu::Texture {
            let hal_desc = wgpu_hal::TextureDescriptor {
                label: Some("vaapi-plane"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu_types::TextureUses::COLOR_TARGET | wgpu_types::TextureUses::COPY_SRC,
                memory_flags: wgpu_hal::MemoryFlags::empty(),
                view_formats: vec![],
            };
            let hal_tex = hal_device.texture_from_raw(img, &hal_desc, None);
            drm.device.create_texture_from_hal::<wgpu_hal::vulkan::Api>(
                hal_tex,
                &wgpu::TextureDescriptor {
                    label: Some("vaapi-plane"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                },
            )
        };

        let y = wrap(img_y, wgpu::TextureFormat::R8Unorm, surf.width, surf.height);
        let uv = wrap(img_uv, wgpu::TextureFormat::Rg8Unorm, surf.width / 2, surf.height / 2);

        // NOTE: img_y/img_uv/memory intentionally leaked for the spike (process-scoped).
        Ok(ImportedNv12 { y, uv })
    }
}
