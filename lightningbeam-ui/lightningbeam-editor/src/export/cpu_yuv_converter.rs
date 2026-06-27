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
    #[should_panic(expected = "RGBA data size mismatch")]
    fn test_wrong_input_size_panics() {
        let mut converter = CpuYuvConverter::new(1920, 1080, true).unwrap();

        // Wrong size input
        let rgba_data = vec![0u8; 1000];

        let _ = converter.convert(&rgba_data);
    }
}
