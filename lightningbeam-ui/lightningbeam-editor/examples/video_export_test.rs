/// Test program to validate video export with synthetic frames
///
/// This creates a simple 5-second video with:
/// - Red frame for 1 second
/// - Green frame for 1 second
/// - Blue frame for 1 second
/// - White frame for 1 second
/// - Black frame for 1 second
///
/// Run with: cargo run --example video_export_test

use std::path::Path;

fn main() -> Result<(), String> {
    println!("Testing H.264 video export with synthetic frames...\n");

    // Initialize FFmpeg
    ffmpeg_next::init().map_err(|e| format!("Failed to initialize FFmpeg: {}", e))?;

    // Output file
    let output_path = "/tmp/test_synthetic.mp4";
    let width = 1920u32;
    let height = 1080u32;
    let framerate = 30.0;
    let bitrate_kbps = 5000; // 5 Mbps
    let duration_secs = 5.0;
    let total_frames = (duration_secs * framerate) as usize;

    println!("Settings:");
    println!("  Resolution: {}×{}", width, height);
    println!("  Framerate: {} fps", framerate);
    println!("  Bitrate: {} kbps", bitrate_kbps);
    println!("  Duration: {} seconds ({} frames)", duration_secs, total_frames);
    println!();

    // Find H.264 encoder
    let encoder_codec = ffmpeg_next::encoder::find(ffmpeg_next::codec::Id::H264)
        .ok_or("H.264 encoder not found")?;

    println!("Using encoder: {}", encoder_codec.name());

    // Create output format context
    let mut output = ffmpeg_next::format::output(&output_path)
        .map_err(|e| format!("Failed to create output file: {}", e))?;

    // Create encoder from codec
    let mut encoder = ffmpeg_next::codec::Context::new_with_codec(encoder_codec)
        .encoder()
        .video()
        .map_err(|e| format!("Failed to create encoder: {}", e))?;

    // Configure encoder parameters BEFORE opening (like working MP3 code)
    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(ffmpeg_next::format::Pixel::YUV420P);
    encoder.set_time_base(ffmpeg_next::Rational(1, (framerate * 1000.0) as i32));
    encoder.set_frame_rate(Some(ffmpeg_next::Rational(framerate as i32, 1)));
    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_gop(framerate as u32); // 1 second GOP

    println!("Opening encoder with open_as()...");

    // Open encoder with codec (like working MP3 code)
    let mut encoder = encoder
        .open_as(encoder_codec)
        .map_err(|e| format!("Failed to open encoder: {}", e))?;

    println!("✅ H.264 encoder opened successfully!");
    println!("Opened encoder format: {:?}", encoder.format());

    // Add stream AFTER opening encoder (like working MP3 code)
    {
        let mut stream = output
            .add_stream(encoder_codec)
            .map_err(|e| format!("Failed to add stream: {}", e))?;
        stream.set_parameters(&encoder);
    }

    output
        .write_header()
        .map_err(|e| format!("Failed to write header: {}", e))?;

    println!("✅ Output file created: {}", output_path);
    println!();

    // Generate and encode frames
    println!("Encoding frames...");
    let frame_size_rgba = (width * height * 4) as usize;
    let mut rgba_buffer = vec![0u8; frame_size_rgba];

    for frame_num in 0..total_frames {
        // Fill RGBA buffer with color based on time
        let color = match frame_num / 30 {
            0 => (255, 0, 0, 255),       // Red (0-1s)
            1 => (0, 255, 0, 255),       // Green (1-2s)
            2 => (0, 0, 255, 255),       // Blue (2-3s)
            3 => (255, 255, 255, 255),   // White (3-4s)
            _ => (0, 0, 0, 255),         // Black (4-5s)
        };

        for pixel in rgba_buffer.chunks_mut(4) {
            pixel[0] = color.0;
            pixel[1] = color.1;
            pixel[2] = color.2;
            pixel[3] = color.3;
        }

        // Convert RGBA to YUV420p
        let (y, u, v) = rgba_to_yuv420p(&rgba_buffer, width, height);

        // Create video frame
        let mut video_frame = ffmpeg_next::frame::Video::new(
            ffmpeg_next::format::Pixel::YUV420P,
            width,
            height,
        );

        // Copy YUV planes (safe slice copy)
        let y_plane = video_frame.data_mut(0);
        let y_len = y.len().min(y_plane.len());
        y_plane[..y_len].copy_from_slice(&y[..y_len]);

        let u_plane = video_frame.data_mut(1);
        let u_len = u.len().min(u_plane.len());
        u_plane[..u_len].copy_from_slice(&u[..u_len]);

        let v_plane = video_frame.data_mut(2);
        let v_len = v.len().min(v_plane.len());
        v_plane[..v_len].copy_from_slice(&v[..v_len]);

        // Set PTS
        let timestamp = frame_num as f64 / framerate;
        video_frame.set_pts(Some((timestamp * 1000.0) as i64));

        // Encode frame
        encoder
            .send_frame(&video_frame)
            .map_err(|e| format!("Failed to send frame: {}", e))?;

        // Receive and write packets
        let mut encoded = ffmpeg_next::Packet::empty();
        while encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(0);
            encoded
                .write_interleaved(&mut output)
                .map_err(|e| format!("Failed to write packet: {}", e))?;
        }

        // Progress indicator
        if (frame_num + 1) % 30 == 0 || frame_num + 1 == total_frames {
            let percent = ((frame_num + 1) as f64 / total_frames as f64 * 100.0) as u32;
            println!("  Frame {}/{} ({}%)", frame_num + 1, total_frames, percent);
        }
    }

    // Flush encoder
    encoder
        .send_eof()
        .map_err(|e| format!("Failed to send EOF: {}", e))?;

    let mut encoded = ffmpeg_next::Packet::empty();
    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(0);
        encoded
            .write_interleaved(&mut output)
            .map_err(|e| format!("Failed to write packet: {}", e))?;
    }

    output
        .write_trailer()
        .map_err(|e| format!("Failed to write trailer: {}", e))?;

    // Check output file
    if Path::new(output_path).exists() {
        let metadata = std::fs::metadata(output_path).unwrap();
        println!();
        println!("✅ Video export successful!");
        println!("  Output: {} ({:.2} MB)", output_path, metadata.len() as f64 / 1_048_576.0);
        println!();
        println!("Test with: ffplay {}", output_path);
        println!("Or: vlc {}", output_path);
    } else {
        return Err("Output file was not created!".to_string());
    }

    Ok(())
}

/// Convert RGBA8 to YUV420p using BT.709 color space
fn rgba_to_yuv420p(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = width as usize;
    let h = height as usize;

    // Y plane (full resolution)
    let mut y_plane = Vec::with_capacity(w * h);

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) * 4;
            let r = rgba[idx] as f32;
            let g = rgba[idx + 1] as f32;
            let b = rgba[idx + 2] as f32;

            // BT.709 luma
            let y_val = (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 255.0) as u8;
            y_plane.push(y_val);
        }
    }

    // U and V planes (quarter resolution)
    let mut u_plane = Vec::with_capacity((w * h) / 4);
    let mut v_plane = Vec::with_capacity((w * h) / 4);

    for y in (0..h).step_by(2) {
        for x in (0..w).step_by(2) {
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

            // BT.709 chroma (centered at 128)
            let u_val = (-0.1146 * r - 0.3854 * g + 0.5000 * b + 128.0).clamp(0.0, 255.0) as u8;
            let v_val = (0.5000 * r - 0.4542 * g - 0.0458 * b + 128.0).clamp(0.0, 255.0) as u8;

            u_plane.push(u_val);
            v_plane.push(v_val);
        }
    }

    (y_plane, u_plane, v_plane)
}
