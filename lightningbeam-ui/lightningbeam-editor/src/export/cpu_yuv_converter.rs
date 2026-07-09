//! CPU-based RGBA→YUV420p color space converter using FFmpeg's swscale
//!
//! This module provides a wrapper around FFmpeg's highly-optimized swscale library
//! for converting RGBA data to YUV420p format. Uses SIMD instructions when available
//! for maximum performance.

use ffmpeg_next as ffmpeg;

/// CPU-based RGBA→YUV420p converter using FFmpeg's swscale
///
/// This converter uses FFmpeg's swscale library which is highly optimized with SIMD
/// instructions (SSE, AVX) for fast color space conversion on the CPU.
pub struct CpuYuvConverter {
    width: u32,
    height: u32,
    /// swscale context + reusable source/dest frames, built once and reused every frame
    /// (creating them per call was a measurable per-output-frame export cost).
    scaler: ffmpeg::software::scaling::Context,
    rgba_frame: ffmpeg::frame::Video,
    yuv_frame: ffmpeg::frame::Video,
}

impl CpuYuvConverter {
    /// Create new converter for given dimensions
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(width: u32, height: u32, full_range: bool) -> Result<Self, String> {
        // BT.709 (HD) RGBA→YUV420p context, created once.
        let mut scaler = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::RGBA,
            width,
            height,
            ffmpeg::format::Pixel::YUV420P,
            width,
            height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| format!("Failed to create swscale context: {}", e))?;

        // swscale defaults to BT.601 + limited range; force BT.709 with the requested output
        // range so this fallback matches the GPU path and the encoder's color tags
        // (otherwise non-%8-width exports come out with shifted hue / wrong levels). There is
        // no safe ffmpeg-next wrapper for sws_setColorspaceDetails, so this is the raw call.
        unsafe {
            let coeffs = ffmpeg::ffi::sws_getCoefficients(ffmpeg::ffi::SWS_CS_ITU709 as i32);
            let dst_range = if full_range { 1 } else { 0 };
            let one = 1 << 16; // 16.16 fixed-point 1.0
            ffmpeg::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                coeffs, 1,          // source table (RGB input is full-range)
                coeffs, dst_range,  // dest table = BT.709, dest range = requested
                0, one, one,        // brightness, contrast, saturation (neutral)
            );
        }

        let rgba_frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);
        let yuv_frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::YUV420P, width, height);
        Ok(Self { width, height, scaler, rgba_frame, yuv_frame })
    }

    /// Convert RGBA data to YUV420p planes
    ///
    /// Performs color space conversion from RGBA (8-bit per channel, packed format)
    /// to YUV420p (8-bit per channel, planar format with subsampled chroma).
    ///
    /// Uses BT.709 color matrix (HD standard) for the conversion.
    ///
    /// # Arguments
    /// * `rgba_data` - Packed RGBA data (width * height * 4 bytes)
    ///
    /// # Returns
    /// Tuple of (y_plane, u_plane, v_plane) as separate Vec<u8>
    ///
    /// # Panics
    /// Panics if rgba_data length doesn't match width * height * 4
    pub fn convert(&mut self, rgba_data: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        let expected_size = (self.width * self.height * 4) as usize;
        assert_eq!(
            rgba_data.len(),
            expected_size,
            "RGBA data size mismatch: expected {} bytes, got {}",
            expected_size,
            rgba_data.len()
        );

        // Copy RGBA into the reused source frame, run the reused scaler into the reused
        // dest frame (SIMD-optimized), then extract planes.
        self.rgba_frame.data_mut(0).copy_from_slice(rgba_data);
        self.scaler
            .run(&self.rgba_frame, &mut self.yuv_frame)
            .map_err(|e| format!("swscale conversion failed: {}", e))?;

        // YUV420p planes: Y full-res, U/V quarter-res (2×2 subsampled).
        let y_plane = self.yuv_frame.data(0).to_vec();
        let u_plane = self.yuv_frame.data(1).to_vec();
        let v_plane = self.yuv_frame.data(2).to_vec();

        Ok((y_plane, u_plane, v_plane))
    }
}

/// CPU RGBA→YUV422P10LE converter (10-bit, 4:2:2) via swscale, for ProRes 422 export.
///
/// ProRes (`prores_ks`) requires a 10-bit 4:2:2 input; the SDR pipeline otherwise produces 8-bit
/// 4:2:0. Source is still 8-bit RGBA (bit-depth is promoted, not conjured), which is normal for
/// SDR ProRes. BT.709 with the requested range, matching the encoder's color tags.
pub struct CpuYuv422P10Converter {
    width: u32,
    height: u32,
    scaler: ffmpeg::software::scaling::Context,
    rgba_frame: ffmpeg::frame::Video,
    yuv_frame: ffmpeg::frame::Video,
}

impl CpuYuv422P10Converter {
    pub fn new(width: u32, height: u32, full_range: bool) -> Result<Self, String> {
        let mut scaler = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::RGBA, width, height,
            ffmpeg::format::Pixel::YUV422P10LE, width, height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| format!("Failed to create YUV422P10 swscale context: {}", e))?;

        // BT.709, requested output range (matches setup_video_encoder's SDR tags). No safe
        // ffmpeg-next wrapper for sws_setColorspaceDetails, so this is the raw call (as in
        // CpuYuvConverter::new above).
        unsafe {
            let coeffs = ffmpeg::ffi::sws_getCoefficients(ffmpeg::ffi::SWS_CS_ITU709 as i32);
            let dst_range = if full_range { 1 } else { 0 };
            let one = 1 << 16;
            ffmpeg::ffi::sws_setColorspaceDetails(
                scaler.as_mut_ptr(),
                coeffs, 1,
                coeffs, dst_range,
                0, one, one,
            );
        }

        let rgba_frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);
        let yuv_frame = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::YUV422P10LE, width, height);
        Ok(Self { width, height, scaler, rgba_frame, yuv_frame })
    }

    /// Convert packed RGBA (width*height*4) to tight YUV422P10LE planes (little-endian, 2 bytes per
    /// sample): Y is width×height, U and V are (width/2)×height. Planes are returned tight (stride
    /// padding stripped) to match what `encode_frame` expects.
    pub fn convert(&mut self, rgba_data: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        let expected = (self.width * self.height * 4) as usize;
        assert_eq!(rgba_data.len(), expected,
            "RGBA data size mismatch: expected {} bytes, got {}", expected, rgba_data.len());

        // Copy RGBA into the source frame honoring its stride (may be padded).
        let row_bytes = (self.width * 4) as usize;
        let src_stride = self.rgba_frame.stride(0);
        {
            let dst = self.rgba_frame.data_mut(0);
            for row in 0..self.height as usize {
                let s = row * row_bytes;
                let d = row * src_stride;
                dst[d..d + row_bytes].copy_from_slice(&rgba_data[s..s + row_bytes]);
            }
        }

        self.scaler
            .run(&self.rgba_frame, &mut self.yuv_frame)
            .map_err(|e| format!("YUV422P10 swscale conversion failed: {}", e))?;

        // Extract each plane tight (2 bytes/sample). Y: width samples/row × height rows.
        // Chroma (4:2:2): width/2 samples/row × height rows.
        let extract = |frame: &ffmpeg::frame::Video, idx: usize, samples_w: usize, rows: usize| {
            let bytes_per_row = samples_w * 2;
            let stride = frame.stride(idx);
            let data = frame.data(idx);
            let mut out = Vec::with_capacity(bytes_per_row * rows);
            for row in 0..rows {
                let start = row * stride;
                out.extend_from_slice(&data[start..start + bytes_per_row]);
            }
            out
        };
        let (w, h) = (self.width as usize, self.height as usize);
        let y_plane = extract(&self.yuv_frame, 0, w, h);
        let u_plane = extract(&self.yuv_frame, 1, w / 2, h);
        let v_plane = extract(&self.yuv_frame, 2, w / 2, h);

        Ok((y_plane, u_plane, v_plane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_converter_creation() {
        let converter = CpuYuvConverter::new(1920, 1080, true);
        assert!(converter.is_ok());
    }

    #[test]
    fn test_conversion_output_sizes() {
        let mut converter = CpuYuvConverter::new(1920, 1080, true).unwrap();

        // Create dummy RGBA data (all black)
        let rgba_data = vec![0u8; 1920 * 1080 * 4];

        let result = converter.convert(&rgba_data);
        assert!(result.is_ok());

        let (y, u, v) = result.unwrap();

        // Y plane should be full resolution
        assert_eq!(y.len(), 1920 * 1080);

        // U and V planes should be quarter resolution (subsampled 2x2)
        assert_eq!(u.len(), (1920 / 2) * (1080 / 2));
        assert_eq!(v.len(), (1920 / 2) * (1080 / 2));
    }

    #[test]
    fn test_yuv422p10_output_sizes() {
        // Use a width that forces swscale linesize padding (not a multiple of 32/64) to exercise
        // the stride-stripping extraction.
        let (w, h) = (1000u32, 720u32);
        let mut c = CpuYuv422P10Converter::new(w, h, false).unwrap();
        let rgba = vec![0u8; (w * h * 4) as usize];
        let (y, u, v) = c.convert(&rgba).unwrap();
        // 10-bit → 2 bytes/sample. Y full res; U/V half width, full height (4:2:2).
        assert_eq!(y.len(), (w * h * 2) as usize);
        assert_eq!(u.len(), ((w / 2) * h * 2) as usize);
        assert_eq!(v.len(), ((w / 2) * h * 2) as usize);
    }

    #[test]
    #[should_panic(expected = "RGBA data size mismatch")]
    fn test_wrong_input_size_panics() {
        let mut converter = CpuYuvConverter::new(1920, 1080, true).unwrap();

        // Wrong size input
        let rgba_data = vec![0u8; 1000];

        let _ = converter.convert(&rgba_data);
    }
}
