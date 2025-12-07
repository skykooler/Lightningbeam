//! Video export functionality
//!
//! Exports video from the timeline using FFmpeg encoding:
//! - H.264/H.265: MP4 container (most compatible)
//! - VP9: WebM container (web-friendly)
//! - ProRes422: MOV container (professional editing)

use ffmpeg_next as ffmpeg;
use std::sync::Arc;
use lightningbeam_core::document::Document;
use lightningbeam_core::renderer::ImageCache;
use lightningbeam_core::video::VideoManager;

/// Reusable frame buffers to avoid allocations
struct FrameBuffers {
    /// RGBA buffer from GPU readback (width * height * 4 bytes)
    rgba_buffer: Vec<u8>,
    /// Y plane for YUV420p (full resolution)
    y_plane: Vec<u8>,
    /// U plane for YUV420p (quarter resolution - 2×2 subsampling)
    u_plane: Vec<u8>,
    /// V plane for YUV420p (quarter resolution - 2×2 subsampling)
    v_plane: Vec<u8>,
}

impl FrameBuffers {
    /// Create new frame buffers for the given resolution
    fn new(width: u32, height: u32) -> Self {
        let rgba_size = (width * height * 4) as usize;
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;

        Self {
            rgba_buffer: vec![0u8; rgba_size],
            y_plane: vec![0u8; y_size],
            u_plane: vec![0u8; uv_size],
            v_plane: vec![0u8; uv_size],
        }
    }
}

/// Convert RGBA8 pixels to YUV420p format using BT.709 color space
///
/// # Arguments
/// * `rgba` - Interleaved RGBA8 pixels (4 bytes per pixel)
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
///
/// # Returns
/// Tuple of (Y plane, U plane, V plane) as separate byte vectors
///
/// # Color Space
/// Uses BT.709 (HDTV) color space conversion:
/// - Y  = 0.2126*R + 0.7152*G + 0.0722*B
/// - U  = -0.1146*R - 0.3854*G + 0.5000*B + 128
/// - V  = 0.5000*R - 0.4542*G - 0.0458*B + 128
///
/// # Format
/// YUV420p is a planar format with 2×2 chroma subsampling:
/// - Y plane: full resolution (width × height)
/// - U plane: quarter resolution (width/2 × height/2)
/// - V plane: quarter resolution (width/2 × height/2)
pub fn rgba_to_yuv420p(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = width as usize;
    let h = height as usize;

    // Round to multiples of 16 for H.264 macroblock alignment
    let aligned_w = (((width + 15) / 16) * 16) as usize;
    let aligned_h = (((height + 15) / 16) * 16) as usize;

    // Allocate Y plane (full aligned resolution, padded with black)
    let mut y_plane = Vec::with_capacity(aligned_w * aligned_h);

    // Convert each pixel to Y (luma), with padding
    for y in 0..aligned_h {
        for x in 0..aligned_w {
            let y_val = if y < h && x < w {
                let idx = (y * w + x) * 4;
                let r = rgba[idx] as f32;
                let g = rgba[idx + 1] as f32;
                let b = rgba[idx + 2] as f32;
                // BT.709 luma conversion
                (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 255.0) as u8
            } else {
                16 // Black in YUV (Y=16 is video black)
            };
            y_plane.push(y_val);
        }
    }

    // Allocate U and V planes (quarter resolution due to 2×2 subsampling)
    let mut u_plane = Vec::with_capacity((aligned_w * aligned_h) / 4);
    let mut v_plane = Vec::with_capacity((aligned_w * aligned_h) / 4);

    // Process 2×2 blocks for chroma subsampling (with padding for aligned dimensions)
    for y in (0..aligned_h).step_by(2) {
        for x in (0..aligned_w).step_by(2) {
            // Check if this block is in the padding region
            let in_padding = y >= h || x >= w;

            let (u_val, v_val) = if in_padding {
                // Padding region: use neutral chroma for black (U=128, V=128)
                (128, 128)
            } else {
                // Average RGB values from 2×2 block
                let mut r_sum = 0.0;
                let mut g_sum = 0.0;
                let mut b_sum = 0.0;

                for dy in 0..2 {
                    for dx in 0..2 {
                        if y + dy < h && x + dx < w {
                            let idx = ((y + dy) * w + (x + dx)) * 4;
                            r_sum += rgba[idx] as f32;
                            g_sum += rgba[idx + 1] as f32;
                            b_sum += rgba[idx + 2] as f32;
                        }
                    }
                }

                let r = r_sum / 4.0;
                let g = g_sum / 4.0;
                let b = b_sum / 4.0;

                // BT.709 chroma conversion (centered at 128)
                let u = (-0.1146 * r - 0.3854 * g + 0.5000 * b + 128.0).clamp(0.0, 255.0) as u8;
                let v = (0.5000 * r - 0.4542 * g - 0.0458 * b + 128.0).clamp(0.0, 255.0) as u8;
                (u, v)
            };

            u_plane.push(u_val);
            v_plane.push(v_val);
        }
    }

    (y_plane, u_plane, v_plane)
}

/// Setup FFmpeg video encoder for the specified codec
///
/// # Arguments
/// * `codec_id` - FFmpeg codec ID (H264, HEVC, VP9, PRORES, etc.)
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `framerate` - Frames per second
/// * `bitrate_kbps` - Target bitrate in kilobits per second
///
/// # Returns
/// Tuple of (opened encoder, codec) for stream setup
///
/// # Note
/// This function follows the same pattern as the working MP3 export:
/// 1. Find codec
/// 2. Create encoder context with codec
/// 3. Set ALL parameters (width, height, format, timebase, framerate, bitrate, GOP)
/// 4. Open encoder with open_as(codec)
/// 5. Caller should add stream AFTER opening and set parameters from opened encoder
pub fn setup_video_encoder(
    codec_id: ffmpeg::codec::Id,
    width: u32,
    height: u32,
    framerate: f64,
    bitrate_kbps: u32,
) -> Result<(ffmpeg::encoder::Video, ffmpeg::Codec), String> {
    // Try to find codec by ID first
    println!("🔍 Looking for codec: {:?}", codec_id);
    let codec = ffmpeg::encoder::find(codec_id);

    let codec = if codec.is_some() {
        println!("✅ Found codec by ID");
        codec
    } else {
        println!("⚠️  Codec {:?} not found by ID", codec_id);

        // If not found by ID, try by name (e.g., "libx264" for H264)
        let encoder_name = match codec_id {
            ffmpeg::codec::Id::H264 => "libx264",
            ffmpeg::codec::Id::HEVC => "libx265",
            ffmpeg::codec::Id::VP8 => "libvpx",
            ffmpeg::codec::Id::VP9 => "libvpx-vp9",
            ffmpeg::codec::Id::PRORES => "prores_ks",
            _ => {
                println!("❌ No fallback encoder name for {:?}", codec_id);
                return Err(format!("Unsupported codec: {:?}", codec_id));
            }
        };

        println!("🔍 Trying encoder by name: {}", encoder_name);
        let by_name = ffmpeg::encoder::find_by_name(encoder_name);

        if by_name.is_some() {
            println!("✅ Found encoder by name: {}", encoder_name);
        } else {
            println!("❌ Encoder {} not found", encoder_name);
        }

        by_name
    };

    let codec = codec.ok_or_else(|| {
        println!("❌ Failed to find codec: {:?}", codec_id);
        println!("💡 The static FFmpeg build is missing this encoder.");
        format!("Video encoder not found for codec: {:?}. Static build may be missing encoder libraries.", codec_id)
    })?;

    // Create encoder context with codec
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| format!("Failed to create video encoder: {}", e))?;

    // Round dimensions to multiples of 16 for H.264 macroblock alignment
    let aligned_width = ((width + 15) / 16) * 16;
    let aligned_height = ((height + 15) / 16) * 16;

    // Configure encoder parameters BEFORE opening (critical!)
    encoder.set_width(aligned_width);
    encoder.set_height(aligned_height);
    encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    encoder.set_time_base(ffmpeg::Rational(1, (framerate * 1000.0) as i32));
    encoder.set_frame_rate(Some(ffmpeg::Rational(framerate as i32, 1)));
    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_gop(framerate as u32); // 1 second GOP

    println!("📐 Video dimensions: {}×{} (aligned to {}×{} for H.264)",
             width, height, aligned_width, aligned_height);

    // Open encoder with codec (like working MP3 export)
    let encoder = encoder
        .open_as(codec)
        .map_err(|e| format!("Failed to open video encoder: {}", e))?;

    Ok((encoder, codec))
}

/// Receive encoded packets from encoder and write to output file
///
/// # Arguments
/// * `encoder` - FFmpeg video encoder
/// * `output` - FFmpeg output format context
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn receive_and_write_packets(
    encoder: &mut ffmpeg::encoder::Video,
    output: &mut ffmpeg::format::context::Output,
) -> Result<(), String> {
    let mut encoded = ffmpeg::Packet::empty();

    // Get time bases for rescaling
    let encoder_tb = encoder.time_base();
    let stream_tb = output.stream(0).ok_or("No output stream found")?.time_base();

    println!("🎬 [PACKET] Encoder TB: {}/{}, Stream TB: {}/{}",
             encoder_tb.0, encoder_tb.1, stream_tb.0, stream_tb.1);

    while encoder.receive_packet(&mut encoded).is_ok() {
        println!("🎬 [PACKET] Before rescale - PTS: {:?}, DTS: {:?}, Duration: {:?}",
                 encoded.pts(), encoded.dts(), encoded.duration());

        encoded.set_stream(0);
        // Rescale timestamps from encoder time base to stream time base
        encoded.rescale_ts(encoder_tb, stream_tb);

        println!("🎬 [PACKET] After rescale - PTS: {:?}, DTS: {:?}, Duration: {:?}",
                 encoded.pts(), encoded.dts(), encoded.duration());

        encoded
            .write_interleaved(output)
            .map_err(|e| format!("Failed to write packet: {}", e))?;
    }

    Ok(())
}

/// Render a document frame at a specific time and read back RGBA pixels from GPU
///
/// # Arguments
/// * `document` - Document to render (current_time will be modified)
/// * `timestamp` - Time in seconds to render at
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `device` - wgpu device
/// * `queue` - wgpu queue
/// * `renderer` - Vello renderer
/// * `image_cache` - Image cache for rendering
/// * `video_manager` - Video manager for video clips
/// * `rgba_buffer` - Output buffer for RGBA pixels (must be width * height * 4 bytes)
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn render_frame_to_rgba(
    document: &mut Document,
    timestamp: f64,
    width: u32,
    height: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    image_cache: &mut ImageCache,
    video_manager: &Arc<std::sync::Mutex<VideoManager>>,
    rgba_buffer: &mut [u8],
) -> Result<(), String> {
    // Set document time to the frame timestamp
    document.current_time = timestamp;

    // Create offscreen texture for rendering
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video_export_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
             | wgpu::TextureUsages::COPY_SRC
             | wgpu::TextureUsages::STORAGE_BINDING, // Required by Vello for compute shaders
        view_formats: &[],
    });

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Render document to Vello scene
    let mut scene = vello::Scene::new();
    lightningbeam_core::renderer::render_document(
        document,
        &mut scene,
        image_cache,
        video_manager,
    );

    // Render scene to texture
    let render_params = vello::RenderParams {
        base_color: vello::peniko::Color::BLACK,
        width,
        height,
        antialiasing_method: vello::AaConfig::Area,
    };

    renderer
        .render_to_texture(device, queue, &scene, &texture_view, &render_params)
        .map_err(|e| format!("Failed to render to texture: {}", e))?;

    // GPU readback: Create staging buffer with proper alignment
    let bytes_per_pixel = 4u32; // RGBA8
    let bytes_per_row_alignment = 256u32; // wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let bytes_per_row = ((unpadded_bytes_per_row + bytes_per_row_alignment - 1)
        / bytes_per_row_alignment) * bytes_per_row_alignment;
    let buffer_size = (bytes_per_row * height) as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("video_export_staging_buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Copy texture to staging buffer
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("video_export_copy_encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    // Map buffer and read pixels (synchronous)
    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).ok();
    });

    device.poll(wgpu::Maintain::Wait);

    receiver
        .recv()
        .map_err(|_| "Failed to receive buffer mapping result")?
        .map_err(|e| format!("Failed to map buffer: {:?}", e))?;

    // Copy data from mapped buffer to output, removing padding
    let data = buffer_slice.get_mapped_range();
    for y in 0..height as usize {
        let src_offset = y * bytes_per_row as usize;
        let dst_offset = y * unpadded_bytes_per_row as usize;
        let row_bytes = unpadded_bytes_per_row as usize;
        rgba_buffer[dst_offset..dst_offset + row_bytes]
            .copy_from_slice(&data[src_offset..src_offset + row_bytes]);
    }

    drop(data);
    staging_buffer.unmap();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgba_to_yuv420p_white() {
        // White: R=255, G=255, B=255
        let rgba = vec![255u8, 255, 255, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Expected: Y=255 (full brightness), U=128, V=128 (neutral chroma)
        assert_eq!(y[0], 255);
        assert_eq!(u[0], 128);
        assert_eq!(v[0], 128);
    }

    #[test]
    fn test_rgba_to_yuv420p_black() {
        // Black: R=0, G=0, B=0
        let rgba = vec![0u8, 0, 0, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Expected: Y=0 (no brightness), U=128, V=128 (neutral chroma)
        assert_eq!(y[0], 0);
        assert_eq!(u[0], 128);
        assert_eq!(v[0], 128);
    }

    #[test]
    fn test_rgba_to_yuv420p_red() {
        // Red: R=255, G=0, B=0
        let rgba = vec![255u8, 0, 0, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Red has:
        // - Y around 54 (low luma due to low green coefficient)
        // - U < 128 (negative blue component)
        // - V > 128 (positive red component)
        assert!(y[0] >= 50 && y[0] <= 60, "Y value: {}", y[0]);
        assert!(u[0] < 128, "U value: {}", u[0]);
        assert!(v[0] > 128, "V value: {}", v[0]);
    }

    #[test]
    fn test_rgba_to_yuv420p_dimensions() {
        // 4×4 image (16 pixels)
        let rgba = vec![0u8; 4 * 4 * 4]; // All black
        let (y, u, v) = rgba_to_yuv420p(&rgba, 4, 4);

        // Y should be full resolution: 4×4 = 16 pixels
        assert_eq!(y.len(), 16);

        // U and V should be quarter resolution: 2×2 = 4 pixels each
        assert_eq!(u.len(), 4);
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn test_rgba_to_yuv420p_2x2_subsampling() {
        // Create 2×2 image with different colors in each corner
        let mut rgba = vec![0u8; 2 * 2 * 4];

        // Top-left: Red
        rgba[0] = 255;
        rgba[1] = 0;
        rgba[2] = 0;
        rgba[3] = 255;

        // Top-right: Green
        rgba[4] = 0;
        rgba[5] = 255;
        rgba[6] = 0;
        rgba[7] = 255;

        // Bottom-left: Blue
        rgba[8] = 0;
        rgba[9] = 0;
        rgba[10] = 255;
        rgba[11] = 255;

        // Bottom-right: White
        rgba[12] = 255;
        rgba[13] = 255;
        rgba[14] = 255;
        rgba[15] = 255;

        let (y, u, v) = rgba_to_yuv420p(&rgba, 2, 2);

        // Y plane should have 4 distinct values (one per pixel)
        assert_eq!(y.len(), 4);

        // U and V should have 1 value each (averaged over 2×2 block)
        assert_eq!(u.len(), 1);
        assert_eq!(v.len(), 1);

        // The averaged chroma should be close to neutral (128)
        // since we have all primary colors + white
        assert!(u[0] >= 100 && u[0] <= 156, "U value: {}", u[0]);
        assert!(v[0] >= 100 && v[0] <= 156, "V value: {}", v[0]);
    }
}
