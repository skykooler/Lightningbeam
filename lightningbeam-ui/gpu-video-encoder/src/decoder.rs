//! VAAPI hardware video decode → wgpu textures. The mirror of [`crate::encoder`]: the codec
//! decodes into a VAAPI NV12 surface, which is mapped to a DRM-PRIME DMA-BUF and imported as two
//! wgpu plane textures via [`crate::dmabuf::import_raw`] — the exact same path the encoder uses,
//! in the read direction. Stays GPU-resident: no CPU frame copy.

use crate::dmabuf::{self, ImportedNv12, Nv12DmaBuf};
use crate::vk_device::{self, DrmDevice};
use ffmpeg_sys_next as ff;
use std::ffi::CString;
use std::path::Path;
use std::ptr;

#[inline]
fn averror(e: i32) -> i32 {
    -e
}

/// `get_format` callback: pick VAAPI surfaces so the decoder outputs hardware frames. With
/// `hw_device_ctx` set, FFmpeg auto-allocates the matching frames context.
unsafe extern "C" fn get_vaapi_format(
    _ctx: *mut ff::AVCodecContext,
    mut fmts: *const ff::AVPixelFormat,
) -> ff::AVPixelFormat {
    while *fmts != ff::AVPixelFormat::AV_PIX_FMT_NONE {
        if *fmts == ff::AVPixelFormat::AV_PIX_FMT_VAAPI {
            return ff::AVPixelFormat::AV_PIX_FMT_VAAPI;
        }
        fmts = fmts.add(1);
    }
    ff::AVPixelFormat::AV_PIX_FMT_NONE
}

/// Hardware decoder for a single video file/stream. Frames come back as importable NV12 textures
/// on [`Self::device`].
pub struct VaapiDecoder {
    drm: DrmDevice,
    hw_device: *mut ff::AVBufferRef,
    fmt: *mut ff::AVFormatContext,
    dec: *mut ff::AVCodecContext,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    stream_index: i32,
    flushing: bool,
}

// Owns its FFmpeg/Vulkan handles exclusively; only moved, never shared (same as the encoder).
unsafe impl Send for VaapiDecoder {}

impl VaapiDecoder {
    /// Open `input_path` and set up VAAPI hardware decoding of its best video stream.
    pub fn new(input_path: &Path) -> Result<Self, String> {
        let drm = vk_device::create()?;
        unsafe {
            let mut hw_device = crate::vaapi::create_device()?;
            let cleanup_hw = |hw: *mut ff::AVBufferRef| {
                let mut h = hw;
                ff::av_buffer_unref(&mut h);
            };

            let path_c = CString::new(input_path.to_string_lossy().as_ref()).unwrap();
            let mut fmt: *mut ff::AVFormatContext = ptr::null_mut();
            if ff::avformat_open_input(&mut fmt, path_c.as_ptr(), ptr::null_mut(), ptr::null_mut()) < 0 {
                cleanup_hw(hw_device);
                return Err(format!("avformat_open_input {input_path:?} failed"));
            }
            if ff::avformat_find_stream_info(fmt, ptr::null_mut()) < 0 {
                ff::avformat_close_input(&mut fmt);
                cleanup_hw(hw_device);
                return Err("avformat_find_stream_info failed".into());
            }

            let mut decoder: *const ff::AVCodec = ptr::null();
            let stream_index = ff::av_find_best_stream(
                fmt,
                ff::AVMediaType::AVMEDIA_TYPE_VIDEO,
                -1,
                -1,
                &mut decoder,
                0,
            );
            if stream_index < 0 || decoder.is_null() {
                ff::avformat_close_input(&mut fmt);
                cleanup_hw(hw_device);
                return Err("no decodable video stream".into());
            }

            let dec = ff::avcodec_alloc_context3(decoder);
            let stream = *(*fmt).streams.add(stream_index as usize);
            if ff::avcodec_parameters_to_context(dec, (*stream).codecpar) < 0 {
                ff::avcodec_free_context(&mut (dec as *mut _));
                ff::avformat_close_input(&mut fmt);
                cleanup_hw(hw_device);
                return Err("avcodec_parameters_to_context failed".into());
            }
            (*dec).hw_device_ctx = ff::av_buffer_ref(hw_device);
            (*dec).get_format = Some(get_vaapi_format);

            if ff::avcodec_open2(dec, decoder, ptr::null_mut()) < 0 {
                ff::avcodec_free_context(&mut (dec as *mut _));
                ff::avformat_close_input(&mut fmt);
                cleanup_hw(hw_device);
                return Err("avcodec_open2 (vaapi decode) failed".into());
            }

            // `mut` only to satisfy the move into the struct; the binding above is consumed.
            let _ = &mut hw_device;
            Ok(Self {
                drm,
                hw_device,
                fmt,
                dec,
                pkt: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
                stream_index,
                flushing: false,
            })
        }
    }

    /// The wgpu device the decoded textures live on (the DMA-BUF-import device).
    pub fn device(&self) -> &wgpu::Device {
        &self.drm.device
    }
    pub fn queue(&self) -> &wgpu::Queue {
        &self.drm.queue
    }

    /// Decode the next frame and import it as NV12 plane textures, or `Ok(None)` at end of stream.
    pub fn next_frame(&mut self) -> Result<Option<ImportedNv12>, String> {
        unsafe {
            loop {
                let r = ff::avcodec_receive_frame(self.dec, self.frame);
                if r == 0 {
                    let imported = self.map_current();
                    ff::av_frame_unref(self.frame);
                    return imported.map(Some);
                }
                if r == ff::AVERROR_EOF {
                    return Ok(None);
                }
                if r != averror(libc::EAGAIN) {
                    return Err(format!("avcodec_receive_frame failed: {r}"));
                }
                if self.flushing {
                    return Ok(None); // already drained the flush
                }

                // Decoder wants more input: pump one packet (or signal EOF to flush).
                let rp = ff::av_read_frame(self.fmt, self.pkt);
                if rp < 0 {
                    self.flushing = true;
                    ff::avcodec_send_packet(self.dec, ptr::null());
                    continue;
                }
                if (*self.pkt).stream_index == self.stream_index {
                    let rs = ff::avcodec_send_packet(self.dec, self.pkt);
                    ff::av_packet_unref(self.pkt);
                    if rs < 0 && rs != averror(libc::EAGAIN) {
                        return Err(format!("avcodec_send_packet failed: {rs}"));
                    }
                } else {
                    ff::av_packet_unref(self.pkt);
                }
            }
        }
    }

    /// Map the just-decoded VAAPI surface (`self.frame`) to a DRM-PRIME DMA-BUF and import it.
    unsafe fn map_current(&self) -> Result<ImportedNv12, String> {
        let drm_f = ff::av_frame_alloc();
        (*drm_f).format = ff::AVPixelFormat::AV_PIX_FMT_DRM_PRIME as i32;
        let flags = ff::AV_HWFRAME_MAP_DIRECT as i32 | ff::AV_HWFRAME_MAP_READ as i32;
        if ff::av_hwframe_map(drm_f, self.frame, flags) < 0 {
            ff::av_frame_free(&mut (drm_f as *mut _));
            return Err("av_hwframe_map failed".into());
        }
        let desc = (*drm_f).data[0] as *const ff::AVDRMFrameDescriptor;
        let obj = &(*desc).objects[0];
        let width = (*self.frame).width as u32;
        let height = (*self.frame).height as u32;
        // NV12: Y then UV — either as two layers (one plane each) or one layer with two planes.
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
        let imported = dmabuf::import_raw(&self.drm, &buf);
        ff::av_frame_free(&mut (drm_f as *mut _)); // the fd was dup'd into Vulkan
        imported
    }
}

impl Drop for VaapiDecoder {
    fn drop(&mut self) {
        unsafe {
            ff::av_frame_free(&mut (self.frame as *mut _));
            ff::av_packet_free(&mut (self.pkt as *mut _));
            ff::avcodec_free_context(&mut (self.dec as *mut _));
            if !self.fmt.is_null() {
                ff::avformat_close_input(&mut self.fmt);
            }
            ff::av_buffer_unref(&mut self.hw_device);
        }
    }
}
