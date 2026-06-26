//! Hardware video decode glue (Linux/VAAPI). The editor implements core's [`HwVideoImporter`]:
//! it maps a decoded VAAPI surface to a DRM-PRIME DMA-BUF and imports it as wgpu NV12 plane
//! textures on the **shared** device (the one eframe + the compositor run on, which has the
//! DMA-BUF-import extensions). [`install`] creates the VAAPI device and wires it into the
//! `VideoManager`.

use ffmpeg_next::ffi as ff;
use gpu_video_encoder::dmabuf::{self, Nv12DmaBuf};
use lightningbeam_core::video::{
    ycbcr_coeffs, GpuVideoFrame, HwDeviceHandle, HwVideoImporter, VideoManager,
};
use std::sync::{Arc, Mutex};

/// Imports decoded VAAPI surfaces onto the shared wgpu device. Holds clones of the shared
/// device + adapter (Arc-backed, cheap).
struct SharedHwImporter {
    device: wgpu::Device,
    adapter: wgpu::Adapter,
}

impl HwVideoImporter for SharedHwImporter {
    unsafe fn import(&self, av_frame: *mut std::ffi::c_void) -> Option<GpuVideoFrame> {
        let frame = av_frame as *mut ff::AVFrame;

        // Map the VAAPI surface to a DRM-PRIME DMA-BUF (read-only).
        let drm_f = ff::av_frame_alloc();
        (*drm_f).format = ff::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
        let flags = ff::AV_HWFRAME_MAP_DIRECT as i32 | ff::AV_HWFRAME_MAP_READ as i32;
        if ff::av_hwframe_map(drm_f, frame, flags) < 0 {
            ff::av_frame_free(&mut (drm_f as *mut _));
            return None;
        }

        let desc = (*drm_f).data[0] as *const ff::AVDRMFrameDescriptor;
        let obj = &(*desc).objects[0];
        let width = (*frame).width as u32;
        let height = (*frame).height as u32;
        // NV12: Y then UV — two layers (one plane each) or one layer with two planes.
        let (y_pl, uv_pl) = if (*desc).nb_layers >= 2 {
            (&(*desc).layers[0].planes[0], &(*desc).layers[1].planes[0])
        } else {
            (&(*desc).layers[0].planes[0], &(*desc).layers[0].planes[1])
        };
        let buf = Nv12DmaBuf {
            fd: obj.fd,
            size: obj.size as u64,
            modifier: obj.format_modifier,
            width,
            height,
            y_offset: y_pl.offset as u64,
            y_pitch: y_pl.pitch as u64,
            uv_offset: uv_pl.offset as u64,
            uv_pitch: uv_pl.pitch as u64,
        };
        let full_range = (*frame).color_range == ff::AVColorRange::AVCOL_RANGE_JPEG;

        // Luma weights (kr, kb) from the frame's matrix coefficients, so SD (BT.601) and HD/UHD
        // (BT.709) clips each convert with the right matrix. Unspecified → guess by height, as
        // players/swscale do. SMPTE240M and BT.2020 are handled too (the latter's transfer is still
        // approximated as sRGB — fine for SDR; true HDR is out of scope).
        let (kr, kb) = match (*frame).colorspace {
            ff::AVColorSpace::AVCOL_SPC_BT709 => (0.2126, 0.0722),
            ff::AVColorSpace::AVCOL_SPC_BT470BG | ff::AVColorSpace::AVCOL_SPC_SMPTE170M => {
                (0.299, 0.114)
            }
            ff::AVColorSpace::AVCOL_SPC_SMPTE240M => (0.212, 0.087),
            ff::AVColorSpace::AVCOL_SPC_BT2020_NCL | ff::AVColorSpace::AVCOL_SPC_BT2020_CL => {
                (0.2627, 0.0593)
            }
            _ => {
                if height <= 576 {
                    (0.299, 0.114) // SD → BT.601
                } else {
                    (0.2126, 0.0722) // HD/UHD → BT.709
                }
            }
        };
        let coeffs = ycbcr_coeffs(kr, kb);

        let imported = dmabuf::import_raw(&self.device, &self.adapter, &buf);
        ff::av_frame_free(&mut (drm_f as *mut _)); // the fd was dup'd into Vulkan
        let (y, uv) = imported.ok()?.into_planes();
        Some(GpuVideoFrame {
            y: Arc::new(y),
            uv: Arc::new(uv),
            width,
            height,
            full_range,
            coeffs,
        })
    }
}

/// Create the VAAPI hardware device and install hardware decode into `vm`, importing onto the
/// shared `device`/`adapter`. Logs and no-ops if VAAPI is unavailable (→ software decode).
pub fn install(vm: &Arc<Mutex<VideoManager>>, device: &wgpu::Device, adapter: &wgpu::Adapter) {
    match gpu_video_encoder::vaapi::create_device() {
        Ok(hw_device) => {
            let importer = Arc::new(SharedHwImporter {
                device: device.clone(),
                adapter: adapter.clone(),
            });
            if let Ok(mut vm) = vm.lock() {
                vm.set_hardware_decode(
                    HwDeviceHandle(hw_device as *mut std::ffi::c_void),
                    importer,
                );
            }
            println!("🎞  Hardware video decode enabled (VAAPI → shared device)");
        }
        Err(e) => {
            println!("🎞  Hardware video decode unavailable ({e}); using software decode");
        }
    }
}
