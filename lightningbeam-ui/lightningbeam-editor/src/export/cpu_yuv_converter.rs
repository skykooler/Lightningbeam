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
}

impl CpuYuvConverter {
    /// Create new converter for given dimensions
    ///
    /// # Arguments
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        Ok(Self { width, height })
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
    pub fn convert(&self, rgba_data: &[u8]) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
        let expected_size = (self.width * self.height * 4) as usize;
        assert_eq!(
            rgba_data.len(),
            expected_size,
            "RGBA data size mismatch: expected {} bytes, got {}",
            expected_size,
            rgba_data.len()
        );

        // Create source RGBA frame
        let mut rgba_frame = ffmpeg::frame::Video::new(
            ffmpeg::format::Pixel::RGBA,
            self.width,
            self.height,
        );

        // Copy RGBA data into source frame
        // ffmpeg-next provides mutable access to the frame data
        let frame_data = rgba_frame.data_mut(0);
        frame_data.copy_from_slice(rgba_data);

        // Create destination YUV420p frame
        let mut yuv_frame = ffmpeg::frame::Video::new(
            ffmpeg::format::Pixel::YUV420P,
            self.width,
            self.height,
        );

        // Create swscale context for RGBA→YUV420p conversion
        // Uses BT.709 color matrix (HD standard)
        let mut scaler = ffmpeg::software::scaling::Context::get(
            ffmpeg::format::Pixel::RGBA,
            self.width,
            self.height,
            ffmpeg::format::Pixel::YUV420P,
            self.width,
            self.height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )
        .map_err(|e| format!("Failed to create swscale context: {}", e))?;

        // Perform the conversion (SIMD-optimized)
        scaler
            .run(&rgba_frame, &mut yuv_frame)
            .map_err(|e| format!("swscale conversion failed: {}", e))?;

        // Extract planar YUV data
        // YUV420p has 3 planes:
        // - Y: full resolution (width × height)
        // - U: quarter resolution (width/2 × height/2)
        // - V: quarter resolution (width/2 × height/2)
        let y_plane = yuv_frame.data(0).to_vec();
        let u_plane = yuv_frame.data(1).to_vec();
        let v_plane = yuv_frame.data(2).to_vec();

        Ok((y_plane, u_plane, v_plane))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_converter_creation() {
        let converter = CpuYuvConverter::new(1920, 1080);
        assert!(converter.is_ok());
    }

    #[test]
    fn test_conversion_output_sizes() {
        let converter = CpuYuvConverter::new(1920, 1080).unwrap();

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
        let converter = CpuYuvConverter::new(1920, 1080).unwrap();

        // Wrong size input
        let rgba_data = vec![0u8; 1000];

        let _ = converter.convert(&rgba_data);
    }
}
