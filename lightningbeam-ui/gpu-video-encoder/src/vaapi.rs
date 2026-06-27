//! VAAPI hardware H.264 encoding (Linux/Intel/AMD).
//!
//! Level 1 (this module first): a CPU-fed encoder — upload NV12 frames to VAAPI
//! surfaces (`av_hwframe_transfer_data`) and encode with `h264_vaapi`. This proves the
//! encoder works and establishes the FFI scaffolding. Level 2 (zero-copy: GPU writes
//! NV12 straight into the VAAPI surface via DMA-BUF) builds on this.
//!
//! All `unsafe` FFmpeg FFI is contained here.

use ffmpeg_sys_next as ff;
use std::ffi::CString;
use std::ptr;

#[inline]
fn averror(e: i32) -> i32 {
    -e
}

/// Create a VAAPI hwdevice on `/dev/dri/renderD128`, trying driver names in turn.
///
/// libva's auto-selection can pick a driver that doesn't support the GPU — notably it
/// chooses the legacy `i965` driver on newer Intel parts (Gen 11+) where the modern `iHD`
/// driver is required. Each `av_hwdevice_ctx_create` opens a fresh VADisplay, so
/// `LIBVA_DRIVER_NAME` is re-read per attempt. We try `iHD` first (modern Intel), then the
/// caller's original setting, then `i965` (older Intel) and `radeonsi` (AMD). On success the
/// working driver name is left in the env; on total failure the original value is restored.
pub fn create_device() -> Result<*mut ff::AVBufferRef, String> {
    unsafe {
        let node = CString::new("/dev/dri/renderD128").unwrap();
        let original = std::env::var_os("LIBVA_DRIVER_NAME");
        let attempts: [Option<&str>; 4] = [Some("iHD"), None, Some("i965"), Some("radeonsi")];
        for drv in attempts {
            match drv {
                Some(d) => std::env::set_var("LIBVA_DRIVER_NAME", d),
                // `None` = the caller's original setting (or libva auto if unset).
                None => match &original {
                    Some(v) => std::env::set_var("LIBVA_DRIVER_NAME", v),
                    None => std::env::remove_var("LIBVA_DRIVER_NAME"),
                },
            }
            let mut hw: *mut ff::AVBufferRef = ptr::null_mut();
            if ff::av_hwdevice_ctx_create(
                &mut hw,
                ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                node.as_ptr(),
                ptr::null_mut(),
                0,
            ) >= 0
            {
                return Ok(hw);
            }
        }
        match &original {
            Some(v) => std::env::set_var("LIBVA_DRIVER_NAME", v),
            None => std::env::remove_var("LIBVA_DRIVER_NAME"),
        }
        Err("av_hwdevice_ctx_create(VAAPI) failed for all drivers (iHD/i965/radeonsi)".into())
    }
}

/// Copy tight NV12 (`Y` then interleaved `UV`) into an AVFrame's planes, respecting
/// each plane's linesize (which FFmpeg may pad).
unsafe fn fill_nv12(frame: *mut ff::AVFrame, nv12: &[u8], width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    // Y plane: h rows of w bytes.
    let dst_y = (*frame).data[0];
    let ls_y = (*frame).linesize[0] as usize;
    for row in 0..h {
        let src = &nv12[row * w..row * w + w];
        ptr::copy_nonoverlapping(src.as_ptr(), dst_y.add(row * ls_y), w);
    }
    // UV plane: h/2 rows of w bytes (interleaved U,V), source offset starts at w*h.
    let dst_uv = (*frame).data[1];
    let ls_uv = (*frame).linesize[1] as usize;
    let uv_off = w * h;
    for row in 0..h / 2 {
        let src = &nv12[uv_off + row * w..uv_off + row * w + w];
        ptr::copy_nonoverlapping(src.as_ptr(), dst_uv.add(row * ls_uv), w);
    }
}

/// A VAAPI NV12 surface mapped to a DMA-BUF, with its layout extracted for Vulkan import.
/// Keeps the FFmpeg handles alive; the `fd` stays valid until drop (dup it for Vulkan).
pub struct MappedSurface {
    hw_device: *mut ff::AVBufferRef,
    frames_ref: *mut ff::AVBufferRef,
    surf: *mut ff::AVFrame,
    drm: *mut ff::AVFrame,
    pub width: u32,
    pub height: u32,
    pub fd: i32,
    pub size: u64,
    pub modifier: u64,
    pub y_offset: u64,
    pub y_pitch: u64,
    pub uv_offset: u64,
    pub uv_pitch: u64,
}

impl MappedSurface {
    /// Allocate a VAAPI NV12 surface and map it to DRM-PRIME.
    pub fn alloc(width: u32, height: u32) -> Result<Self, String> {
        unsafe {
            let mut hw_device: *mut ff::AVBufferRef = ptr::null_mut();
            let node = CString::new("/dev/dri/renderD128").unwrap();
            if ff::av_hwdevice_ctx_create(
                &mut hw_device,
                ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                node.as_ptr(),
                ptr::null_mut(),
                0,
            ) < 0
            {
                return Err("av_hwdevice_ctx_create failed".into());
            }
            let frames_ref = ff::av_hwframe_ctx_alloc(hw_device);
            if frames_ref.is_null() {
                ff::av_buffer_unref(&mut hw_device);
                return Err("av_hwframe_ctx_alloc failed".into());
            }
            {
                let fctx = (*frames_ref).data as *mut ff::AVHWFramesContext;
                (*fctx).format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*fctx).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
                (*fctx).width = width as i32;
                (*fctx).height = height as i32;
                (*fctx).initial_pool_size = 4;
            }
            if ff::av_hwframe_ctx_init(frames_ref) < 0 {
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::av_buffer_unref(&mut hw_device);
                return Err("av_hwframe_ctx_init failed".into());
            }
            let surf = ff::av_frame_alloc();
            if ff::av_hwframe_get_buffer(frames_ref, surf, 0) < 0 {
                ff::av_frame_free(&mut (surf as *mut _));
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::av_buffer_unref(&mut hw_device);
                return Err("av_hwframe_get_buffer failed".into());
            }
            let drm = ff::av_frame_alloc();
            (*drm).format = ff::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
            let flags = ff::AV_HWFRAME_MAP_DIRECT as i32
                | ff::AV_HWFRAME_MAP_READ as i32
                | ff::AV_HWFRAME_MAP_WRITE as i32;
            if ff::av_hwframe_map(drm, surf, flags) < 0 {
                ff::av_frame_free(&mut (drm as *mut _));
                ff::av_frame_free(&mut (surf as *mut _));
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::av_buffer_unref(&mut hw_device);
                return Err("av_hwframe_map failed".into());
            }
            let desc = (*drm).data[0] as *const ff::AVDRMFrameDescriptor;
            // Expect 1 object, 2 layers (Y=R8, UV=GR88).
            if (*desc).nb_objects != 1 || (*desc).nb_layers != 2 {
                let msg = format!(
                    "unexpected DRM layout: {} objects, {} layers",
                    (*desc).nb_objects, (*desc).nb_layers
                );
                // Free everything mapped/allocated above (this path was leaking the device,
                // frames context, and both AVFrames on every odd-layout surface).
                ff::av_frame_free(&mut (drm as *mut _));
                ff::av_frame_free(&mut (surf as *mut _));
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::av_buffer_unref(&mut hw_device);
                return Err(msg);
            }
            let obj = &(*desc).objects[0];
            let y = &(*desc).layers[0].planes[0];
            let uv = &(*desc).layers[1].planes[0];
            Ok(MappedSurface {
                hw_device,
                frames_ref,
                surf,
                drm,
                width,
                height,
                fd: obj.fd,
                size: obj.size as u64,
                modifier: obj.format_modifier,
                y_offset: y.offset as u64,
                y_pitch: y.pitch as u64,
                uv_offset: uv.offset as u64,
                uv_pitch: uv.pitch as u64,
            })
        }
    }

    /// The underlying VASurface AVFrame (to hand to the encoder).
    pub fn av_frame(&self) -> *mut ff::AVFrame {
        self.surf
    }

    /// Read the surface back to tight CPU NV12 (for verifying what the GPU wrote).
    pub fn readback_nv12(&self) -> Result<Vec<u8>, String> {
        unsafe {
            let sw = ff::av_frame_alloc();
            (*sw).format = ff::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
            (*sw).width = self.width as i32;
            (*sw).height = self.height as i32;
            if ff::av_frame_get_buffer(sw, 0) < 0 {
                ff::av_frame_free(&mut (sw as *mut _));
                return Err("av_frame_get_buffer failed".into());
            }
            if ff::av_hwframe_transfer_data(sw, self.surf, 0) < 0 {
                ff::av_frame_free(&mut (sw as *mut _));
                return Err("av_hwframe_transfer_data (download) failed".into());
            }
            let w = self.width as usize;
            let h = self.height as usize;
            let mut out = vec![0u8; w * h + w * (h / 2)];
            let ls_y = (*sw).linesize[0] as usize;
            for row in 0..h {
                let src = (*sw).data[0].add(row * ls_y);
                ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(row * w), w);
            }
            let ls_uv = (*sw).linesize[1] as usize;
            let uv_off = w * h;
            for row in 0..h / 2 {
                let src = (*sw).data[1].add(row * ls_uv);
                ptr::copy_nonoverlapping(src, out.as_mut_ptr().add(uv_off + row * w), w);
            }
            ff::av_frame_free(&mut (sw as *mut _));
            Ok(out)
        }
    }
}

impl Drop for MappedSurface {
    fn drop(&mut self) {
        unsafe {
            ff::av_frame_free(&mut (self.drm as *mut _));
            ff::av_frame_free(&mut (self.surf as *mut _));
            let mut fr = self.frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::av_buffer_unref(&mut self.hw_device);
        }
    }
}

/// Allocate one VAAPI NV12 surface, map it to a DRM-PRIME descriptor, and return a
/// human-readable dump of its DMA-BUF layout (object fds/size/modifier; layer fourcc;
/// per-plane object/offset/pitch). The format **modifier** decides the zero-copy path:
/// `0` = LINEAR (compute can write a linear NV12 buffer/image), anything else = tiled
/// (needs a GPU copy into the tiled surface, or a linear import VAAPI accepts).
pub fn probe_surface_drm(width: u32, height: u32) -> Result<String, String> {
    unsafe {
        let mut hw_device: *mut ff::AVBufferRef = ptr::null_mut();
        let node = CString::new("/dev/dri/renderD128").unwrap();
        if ff::av_hwdevice_ctx_create(
            &mut hw_device,
            ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            node.as_ptr(),
            ptr::null_mut(),
            0,
        ) < 0
        {
            return Err("av_hwdevice_ctx_create(VAAPI) failed".into());
        }

        let frames_ref = ff::av_hwframe_ctx_alloc(hw_device);
        if frames_ref.is_null() {
            ff::av_buffer_unref(&mut hw_device);
            return Err("av_hwframe_ctx_alloc failed".into());
        }
        {
            let fctx = (*frames_ref).data as *mut ff::AVHWFramesContext;
            (*fctx).format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*fctx).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
            (*fctx).width = width as i32;
            (*fctx).height = height as i32;
            (*fctx).initial_pool_size = 2;
        }
        if ff::av_hwframe_ctx_init(frames_ref) < 0 {
            let mut fr = frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::av_buffer_unref(&mut hw_device);
            return Err("av_hwframe_ctx_init failed".into());
        }

        let surf = ff::av_frame_alloc();
        if ff::av_hwframe_get_buffer(frames_ref, surf, 0) < 0 {
            ff::av_frame_free(&mut (surf as *mut _));
            let mut fr = frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::av_buffer_unref(&mut hw_device);
            return Err("av_hwframe_get_buffer failed".into());
        }

        let drm = ff::av_frame_alloc();
        (*drm).format = ff::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
        let flags = ff::AV_HWFRAME_MAP_DIRECT as i32
            | ff::AV_HWFRAME_MAP_READ as i32
            | ff::AV_HWFRAME_MAP_WRITE as i32;
        let r = ff::av_hwframe_map(drm, surf, flags);
        if r < 0 {
            ff::av_frame_free(&mut (drm as *mut _));
            ff::av_frame_free(&mut (surf as *mut _));
            let mut fr = frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::av_buffer_unref(&mut hw_device);
            return Err(format!("av_hwframe_map(DRM_PRIME) failed: {r}"));
        }

        let desc = (*drm).data[0] as *const ff::AVDRMFrameDescriptor;
        let mut s = format!("VAAPI NV12 {width}x{height} surface as DRM-PRIME:\n");
        s += &format!("  nb_objects = {}\n", (*desc).nb_objects);
        for o in 0..(*desc).nb_objects as usize {
            let obj = &(*desc).objects[o];
            s += &format!(
                "  object[{o}]: fd={} size={} format_modifier=0x{:016x}{}\n",
                obj.fd,
                obj.size,
                obj.format_modifier,
                if obj.format_modifier == 0 { " (LINEAR)" } else { " (tiled)" },
            );
        }
        s += &format!("  nb_layers = {}\n", (*desc).nb_layers);
        for l in 0..(*desc).nb_layers as usize {
            let lay = &(*desc).layers[l];
            let f = lay.format;
            let fourcc = [(f & 0xff) as u8, ((f >> 8) & 0xff) as u8, ((f >> 16) & 0xff) as u8, ((f >> 24) & 0xff) as u8];
            s += &format!(
                "  layer[{l}]: format='{}' (0x{:08x}) nb_planes={}\n",
                String::from_utf8_lossy(&fourcc),
                f,
                lay.nb_planes,
            );
            for p in 0..lay.nb_planes as usize {
                let pl = &lay.planes[p];
                s += &format!(
                    "    plane[{p}]: object_index={} offset={} pitch={}\n",
                    pl.object_index, pl.offset, pl.pitch,
                );
            }
        }

        ff::av_frame_free(&mut (drm as *mut _));
        ff::av_frame_free(&mut (surf as *mut _));
        let mut fr = frames_ref;
        ff::av_buffer_unref(&mut fr);
        ff::av_buffer_unref(&mut hw_device);
        Ok(s)
    }
}

/// Encode NV12 frames with `h264_vaapi` and write the raw Annex-B H.264 to `out_path`.
/// Returns the number of encoded packets. `Err` (rather than panic) when VAAPI/the
/// encoder is unavailable, so callers can fall back.
pub fn encode_nv12_to_file(
    width: u32,
    height: u32,
    frames: &[Vec<u8>],
    framerate: i32,
    out_path: &str,
) -> Result<usize, String> {
    unsafe {
        // 1. VAAPI device.
        let mut hw_device: *mut ff::AVBufferRef = ptr::null_mut();
        let node = CString::new("/dev/dri/renderD128").unwrap();
        let r = ff::av_hwdevice_ctx_create(
            &mut hw_device,
            ff::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
            node.as_ptr(),
            ptr::null_mut(),
            0,
        );
        if r < 0 {
            return Err(format!("av_hwdevice_ctx_create(VAAPI) failed: {r}"));
        }

        let cleanup_dev = |dev: *mut ff::AVBufferRef| {
            let mut d = dev;
            ff::av_buffer_unref(&mut d);
        };

        // 2. Encoder.
        let name = CString::new("h264_vaapi").unwrap();
        let codec = ff::avcodec_find_encoder_by_name(name.as_ptr());
        if codec.is_null() {
            cleanup_dev(hw_device);
            return Err("encoder h264_vaapi not found in this FFmpeg build".into());
        }
        let enc = ff::avcodec_alloc_context3(codec);
        if enc.is_null() {
            cleanup_dev(hw_device);
            return Err("avcodec_alloc_context3 failed".into());
        }
        (*enc).width = width as i32;
        (*enc).height = height as i32;
        (*enc).time_base = ff::AVRational { num: 1, den: framerate };
        (*enc).framerate = ff::AVRational { num: framerate, den: 1 };
        (*enc).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;

        // 3. HW frames context (VAAPI surfaces with NV12 sw layout).
        let frames_ref = ff::av_hwframe_ctx_alloc(hw_device);
        if frames_ref.is_null() {
            ff::avcodec_free_context(&mut (enc as *mut _));
            cleanup_dev(hw_device);
            return Err("av_hwframe_ctx_alloc failed".into());
        }
        {
            let fctx = (*frames_ref).data as *mut ff::AVHWFramesContext;
            (*fctx).format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*fctx).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
            (*fctx).width = width as i32;
            (*fctx).height = height as i32;
            (*fctx).initial_pool_size = 8;
        }
        let r = ff::av_hwframe_ctx_init(frames_ref);
        if r < 0 {
            let mut fr = frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::avcodec_free_context(&mut (enc as *mut _));
            cleanup_dev(hw_device);
            return Err(format!("av_hwframe_ctx_init failed: {r}"));
        }
        (*enc).hw_frames_ctx = ff::av_buffer_ref(frames_ref);

        // 4. Open.
        let r = ff::avcodec_open2(enc, codec, ptr::null_mut());
        if r < 0 {
            let mut fr = frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::avcodec_free_context(&mut (enc as *mut _));
            cleanup_dev(hw_device);
            return Err(format!("avcodec_open2(h264_vaapi) failed: {r}"));
        }

        let mut out: Vec<u8> = Vec::new();
        let pkt = ff::av_packet_alloc();
        let mut count = 0usize;

        // Drain helper: pull packets and append to `out`.
        let drain = |enc: *mut ff::AVCodecContext, out: &mut Vec<u8>, count: &mut usize| -> Result<(), String> {
            loop {
                let r = ff::avcodec_receive_packet(enc, pkt);
                if r == averror(libc::EAGAIN) || r == ff::AVERROR_EOF {
                    break;
                }
                if r < 0 {
                    return Err(format!("avcodec_receive_packet failed: {r}"));
                }
                let data = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                out.extend_from_slice(data);
                *count += 1;
                ff::av_packet_unref(pkt);
            }
            Ok(())
        };

        let mut err: Option<String> = None;
        for (i, nv12) in frames.iter().enumerate() {
            // Software NV12 frame.
            let sw = ff::av_frame_alloc();
            (*sw).format = ff::AVPixelFormat::AV_PIX_FMT_NV12 as i32;
            (*sw).width = width as i32;
            (*sw).height = height as i32;
            if ff::av_frame_get_buffer(sw, 0) < 0 {
                err = Some("av_frame_get_buffer(sw) failed".into());
                ff::av_frame_free(&mut (sw as *mut _));
                break;
            }
            fill_nv12(sw, nv12, width, height);

            // VAAPI surface frame + upload.
            let hw = ff::av_frame_alloc();
            if ff::av_hwframe_get_buffer(frames_ref, hw, 0) < 0 {
                err = Some("av_hwframe_get_buffer failed".into());
                ff::av_frame_free(&mut (sw as *mut _));
                ff::av_frame_free(&mut (hw as *mut _));
                break;
            }
            if ff::av_hwframe_transfer_data(hw, sw, 0) < 0 {
                err = Some("av_hwframe_transfer_data failed".into());
                ff::av_frame_free(&mut (sw as *mut _));
                ff::av_frame_free(&mut (hw as *mut _));
                break;
            }
            (*hw).pts = i as i64;

            let r = ff::avcodec_send_frame(enc, hw);
            ff::av_frame_free(&mut (sw as *mut _));
            ff::av_frame_free(&mut (hw as *mut _));
            if r < 0 {
                err = Some(format!("avcodec_send_frame failed: {r}"));
                break;
            }
            if let Err(e) = drain(enc, &mut out, &mut count) {
                err = Some(e);
                break;
            }
        }

        // Flush.
        if err.is_none() {
            ff::avcodec_send_frame(enc, ptr::null_mut());
            if let Err(e) = drain(enc, &mut out, &mut count) {
                err = Some(e);
            }
        }

        // Cleanup.
        ff::av_packet_free(&mut (pkt as *mut _));
        let mut fr = frames_ref;
        ff::av_buffer_unref(&mut fr);
        ff::avcodec_free_context(&mut (enc as *mut _));
        cleanup_dev(hw_device);

        if let Some(e) = err {
            return Err(e);
        }
        std::fs::write(out_path, &out).map_err(|e| format!("write {out_path}: {e}"))?;
        Ok(count)
    }
}
