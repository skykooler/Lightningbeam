//! End-to-end zero-copy H.264 encoder: render an RGBA wgpu texture straight into a VAAPI
//! NV12 surface (no CPU copy) and encode it with `h264_vaapi`. The caller renders frames
//! on [`ZeroCopyEncoder::device`] (the custom Vulkan device with DMA-BUF import enabled).
//!
//! Imports are cached by VASurface id, so the pooled surfaces are imported once each.

use crate::dmabuf::{self, ImportedNv12, Nv12DmaBuf};
use crate::render_nv12::Rgba2Nv12;
use crate::vk_device::{self, DrmDevice};
use ffmpeg_sys_next as ff;
use std::collections::HashMap;
use std::ffi::CString;
use std::ptr;

#[inline]
fn averror(e: i32) -> i32 {
    -e
}

pub struct ZeroCopyEncoder {
    drm: DrmDevice,
    renderer: Rgba2Nv12,
    hw_device: *mut ff::AVBufferRef,
    frames_ref: *mut ff::AVBufferRef,
    enc: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    width: u32,
    height: u32,
    pts: i64,
    cache: HashMap<usize, ImportedNv12>,
    out: Vec<u8>,
}

impl ZeroCopyEncoder {
    /// Build a zero-copy `h264_vaapi` encoder, or `Err` if VAAPI/the device is unavailable.
    pub fn new(width: u32, height: u32, framerate: i32, bitrate_kbps: u32) -> Result<Self, String> {
        let drm = vk_device::create()?;
        let renderer = Rgba2Nv12::new(&drm.device);
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
            let name = CString::new("h264_vaapi").unwrap();
            let codec = ff::avcodec_find_encoder_by_name(name.as_ptr());
            if codec.is_null() {
                ff::av_buffer_unref(&mut hw_device);
                return Err("h264_vaapi not found".into());
            }
            let enc = ff::avcodec_alloc_context3(codec);
            (*enc).width = width as i32;
            (*enc).height = height as i32;
            (*enc).time_base = ff::AVRational { num: 1, den: framerate };
            (*enc).framerate = ff::AVRational { num: framerate, den: 1 };
            (*enc).pix_fmt = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
            (*enc).bit_rate = (bitrate_kbps as i64) * 1000;

            let frames_ref = ff::av_hwframe_ctx_alloc(hw_device);
            {
                let fctx = (*frames_ref).data as *mut ff::AVHWFramesContext;
                (*fctx).format = ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*fctx).sw_format = ff::AVPixelFormat::AV_PIX_FMT_NV12;
                (*fctx).width = width as i32;
                (*fctx).height = height as i32;
                (*fctx).initial_pool_size = 16;
            }
            if ff::av_hwframe_ctx_init(frames_ref) < 0 {
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::avcodec_free_context(&mut (enc as *mut _));
                ff::av_buffer_unref(&mut hw_device);
                return Err("av_hwframe_ctx_init failed".into());
            }
            (*enc).hw_frames_ctx = ff::av_buffer_ref(frames_ref);

            if ff::avcodec_open2(enc, codec, ptr::null_mut()) < 0 {
                let mut fr = frames_ref;
                ff::av_buffer_unref(&mut fr);
                ff::avcodec_free_context(&mut (enc as *mut _));
                ff::av_buffer_unref(&mut hw_device);
                return Err("avcodec_open2(h264_vaapi) failed".into());
            }

            Ok(Self {
                drm,
                renderer,
                hw_device,
                frames_ref,
                enc,
                pkt: ff::av_packet_alloc(),
                width,
                height,
                pts: 0,
                cache: HashMap::new(),
                out: Vec::new(),
            })
        }
    }

    /// The wgpu device frames must be rendered on (so the RGBA texture is importable).
    pub fn device(&self) -> &wgpu::Device {
        &self.drm.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.drm.queue
    }

    /// Render `rgba` (an `Rgba8Unorm` texture on [`Self::device`], `TEXTURE_BINDING`)
    /// into a VAAPI surface and encode it. Appends any produced packets internally.
    pub fn encode_rgba(&mut self, rgba: &wgpu::Texture) -> Result<(), String> {
        unsafe {
            let surf = ff::av_frame_alloc();
            if ff::av_hwframe_get_buffer(self.frames_ref, surf, 0) < 0 {
                ff::av_frame_free(&mut (surf as *mut _));
                return Err("av_hwframe_get_buffer failed".into());
            }
            let id = (*surf).data[3] as usize; // VASurfaceID

            if !self.cache.contains_key(&id) {
                let drm_f = ff::av_frame_alloc();
                (*drm_f).format = ff::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
                let flags = ff::AV_HWFRAME_MAP_DIRECT as i32
                    | ff::AV_HWFRAME_MAP_READ as i32
                    | ff::AV_HWFRAME_MAP_WRITE as i32;
                if ff::av_hwframe_map(drm_f, surf, flags) < 0 {
                    ff::av_frame_free(&mut (drm_f as *mut _));
                    ff::av_frame_free(&mut (surf as *mut _));
                    return Err("av_hwframe_map failed".into());
                }
                let desc = (*drm_f).data[0] as *const ff::AVDRMFrameDescriptor;
                let obj = &(*desc).objects[0];
                let y = &(*desc).layers[0].planes[0];
                let uv = &(*desc).layers[1].planes[0];
                let buf = Nv12DmaBuf {
                    fd: obj.fd,
                    size: obj.size as u64,
                    modifier: obj.format_modifier,
                    width: self.width,
                    height: self.height,
                    y_offset: y.offset as u64,
                    y_pitch: y.pitch as u64,
                    uv_offset: uv.offset as u64,
                    uv_pitch: uv.pitch as u64,
                };
                let imported = match dmabuf::import_raw(&self.drm, &buf) {
                    Ok(i) => i,
                    Err(e) => {
                        ff::av_frame_free(&mut (drm_f as *mut _));
                        ff::av_frame_free(&mut (surf as *mut _));
                        return Err(e);
                    }
                };
                ff::av_frame_free(&mut (drm_f as *mut _)); // fd was dup'd into Vulkan
                self.cache.insert(id, imported);
            }

            // Render RGBA -> NV12 directly into the surface planes.
            let imp = self.cache.get(&id).unwrap();
            let rgba_view = rgba.create_view(&Default::default());
            let y_view = imp.y().create_view(&Default::default());
            let uv_view = imp.uv().create_view(&Default::default());
            let mut cmd = self.drm.device.create_command_encoder(&Default::default());
            self.renderer.convert(&self.drm.device, &mut cmd, &rgba_view, &y_view, &uv_view);
            self.drm.queue.submit(Some(cmd.finish()));
            let _ = self.drm.device.poll(wgpu::PollType::wait_indefinitely());

            // Encode the surface.
            (*surf).pts = self.pts;
            self.pts += 1;
            let r = ff::avcodec_send_frame(self.enc, surf);
            ff::av_frame_free(&mut (surf as *mut _));
            if r < 0 {
                return Err(format!("avcodec_send_frame failed: {r}"));
            }
            self.drain()
        }
    }

    unsafe fn drain(&mut self) -> Result<(), String> {
        loop {
            let r = ff::avcodec_receive_packet(self.enc, self.pkt);
            if r == averror(libc::EAGAIN) || r == ff::AVERROR_EOF {
                break;
            }
            if r < 0 {
                return Err(format!("avcodec_receive_packet failed: {r}"));
            }
            let data = std::slice::from_raw_parts((*self.pkt).data, (*self.pkt).size as usize);
            self.out.extend_from_slice(data);
            ff::av_packet_unref(self.pkt);
        }
        Ok(())
    }

    /// Flush the encoder and return the accumulated Annex-B H.264 bitstream.
    pub fn finish(mut self) -> Result<Vec<u8>, String> {
        unsafe {
            ff::avcodec_send_frame(self.enc, ptr::null_mut());
            self.drain()?;
        }
        Ok(std::mem::take(&mut self.out))
    }
}

impl Drop for ZeroCopyEncoder {
    fn drop(&mut self) {
        unsafe {
            self.cache.clear(); // frees imported Vulkan resources first
            ff::av_packet_free(&mut (self.pkt as *mut _));
            ff::avcodec_free_context(&mut (self.enc as *mut _));
            let mut fr = self.frames_ref;
            ff::av_buffer_unref(&mut fr);
            ff::av_buffer_unref(&mut self.hw_device);
        }
    }
}
