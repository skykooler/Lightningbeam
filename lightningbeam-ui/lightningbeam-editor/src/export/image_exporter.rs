//! Image encoding — save raw RGBA bytes as PNG / JPEG / WebP.

use lightningbeam_core::export::ImageFormat;
use std::path::Path;

/// Encode `pixels` (raw RGBA8, top-left origin) and write to `path`.
///
/// * `allow_transparency` — when true the alpha channel is preserved (PNG/WebP);
///   when false each pixel is composited onto black before encoding.
pub fn save_rgba_image(
    pixels: &[u8],
    width: u32,
    height: u32,
    format: ImageFormat,
    quality: u8,
    allow_transparency: bool,
    path: &Path,
) -> Result<(), String> {
    use image::{ImageBuffer, Rgba};

    let img = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| "Pixel buffer size mismatch".to_string())?;

    match format {
        ImageFormat::Png => {
            if allow_transparency {
                img.save(path).map_err(|e| format!("PNG save failed: {e}"))
            } else {
                let flat = flatten_alpha(img);
                flat.save(path).map_err(|e| format!("PNG save failed: {e}"))
            }
        }
        ImageFormat::Jpeg => {
            use image::codecs::jpeg::JpegEncoder;
            use image::DynamicImage;
            use std::fs::File;
            use std::io::BufWriter;

            // Flatten alpha onto black before JPEG encoding (JPEG has no alpha).
            let flat = flatten_alpha(img);
            let rgb_img = DynamicImage::ImageRgb8(flat).to_rgb8();
            let file = File::create(path).map_err(|e| format!("Cannot create file: {e}"))?;
            let writer = BufWriter::new(file);
            let mut encoder = JpegEncoder::new_with_quality(writer, quality);
            encoder.encode_image(&rgb_img).map_err(|e| format!("JPEG encode failed: {e}"))
        }
        ImageFormat::WebP => {
            // `image` 0.25's WebP encoder is lossless-only, which ignored the quality slider and
            // produced needlessly large files. Encode lossy WebP via ffmpeg's libwebp instead so
            // the quality control is real; alpha is preserved (as YUVA420P) when requested.
            save_webp_ffmpeg(pixels, width, height, quality, allow_transparency, path)
        }
    }
}

/// Encode a single frame as lossy WebP via ffmpeg's `libwebp` encoder.
///
/// `quality` is libwebp's 0–100 quality factor. When `allow_transparency` is true the source is
/// converted to YUVA420P so libwebp keeps the alpha channel; otherwise it's flattened onto black
/// and converted to YUV420P. Uses swscale's default BT.601 conversion (matching a plain
/// `ffmpeg -i in.png out.webp`).
fn save_webp_ffmpeg(
    pixels: &[u8],
    width: u32,
    height: u32,
    quality: u8,
    allow_transparency: bool,
    path: &Path,
) -> Result<(), String> {
    use ffmpeg_next as ffmpeg;

    ffmpeg::init().map_err(|e| format!("Failed to initialize ffmpeg: {e}"))?;

    let codec = ffmpeg::encoder::find_by_name("libwebp")
        .or_else(|| ffmpeg::encoder::find(ffmpeg::codec::Id::WEBP))
        .ok_or("libwebp encoder not available in this ffmpeg build")?;

    // Flatten onto black up front when alpha isn't wanted, so the source is fully opaque.
    let src_rgba: Vec<u8> = if allow_transparency {
        pixels.to_vec()
    } else {
        let mut v = pixels.to_vec();
        for px in v.chunks_exact_mut(4) {
            let a = px[3] as u32;
            px[0] = (px[0] as u32 * a / 255) as u8;
            px[1] = (px[1] as u32 * a / 255) as u8;
            px[2] = (px[2] as u32 * a / 255) as u8;
            px[3] = 255;
        }
        v
    };

    let dst_pix = if allow_transparency {
        ffmpeg::format::Pixel::YUVA420P
    } else {
        ffmpeg::format::Pixel::YUV420P
    };

    // RGBA → YUV(A)420P (swscale defaults: BT.601, limited range — what libwebp expects).
    let mut scaler = ffmpeg::software::scaling::Context::get(
        ffmpeg::format::Pixel::RGBA, width, height,
        dst_pix, width, height,
        ffmpeg::software::scaling::Flags::BILINEAR,
    )
    .map_err(|e| format!("Failed to create swscale context: {e}"))?;

    let mut src = ffmpeg::frame::Video::new(ffmpeg::format::Pixel::RGBA, width, height);
    // Copy row-by-row honoring the frame's stride (may exceed width*4 due to alignment padding).
    let stride = src.stride(0);
    let row_bytes = (width * 4) as usize;
    {
        let dst = src.data_mut(0);
        for y in 0..height as usize {
            let s = y * row_bytes;
            let d = y * stride;
            dst[d..d + row_bytes].copy_from_slice(&src_rgba[s..s + row_bytes]);
        }
    }

    let mut yuv = ffmpeg::frame::Video::new(dst_pix, width, height);
    scaler.run(&src, &mut yuv).map_err(|e| format!("swscale conversion failed: {e}"))?;
    yuv.set_pts(Some(0));

    let mut octx = ffmpeg::format::output(&path)
        .map_err(|e| format!("Failed to create WebP output: {e}"))?;

    let mut enc = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| format!("Failed to create WebP encoder: {e}"))?;
    enc.set_width(width);
    enc.set_height(height);
    enc.set_format(dst_pix);
    enc.set_time_base(ffmpeg::Rational(1, 1));

    // libwebp private options: quality 0–100, lossy.
    let mut opts = ffmpeg::Dictionary::new();
    opts.set("quality", &quality.to_string());
    opts.set("lossless", "0");
    let mut enc = enc
        .open_with(opts)
        .map_err(|e| format!("Failed to open libwebp encoder: {e}"))?;

    {
        let mut stream = octx.add_stream(codec)
            .map_err(|e| format!("Failed to add WebP stream: {e}"))?;
        stream.set_parameters(&enc);
        stream.set_time_base(ffmpeg::Rational(1, 1));
    }

    octx.write_header().map_err(|e| format!("Failed to write WebP header: {e}"))?;
    enc.send_frame(&yuv).map_err(|e| format!("Failed to send WebP frame: {e}"))?;
    enc.send_eof().map_err(|e| format!("Failed to flush WebP encoder: {e}"))?;

    let mut packet = ffmpeg::Packet::empty();
    while enc.receive_packet(&mut packet).is_ok() {
        packet.set_stream(0);
        packet
            .write_interleaved(&mut octx)
            .map_err(|e| format!("Failed to write WebP packet: {e}"))?;
    }

    octx.write_trailer().map_err(|e| format!("Failed to finalize WebP: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lightningbeam_core::export::ImageFormat;

    /// A gradient RGBA image so the encoder has real content to quantize/compress.
    fn gradient(width: u32, height: u32) -> Vec<u8> {
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                px.push((x * 255 / width.max(1)) as u8);
                px.push((y * 255 / height.max(1)) as u8);
                px.push(128);
                px.push(255);
            }
        }
        px
    }

    /// The ffmpeg libwebp path must produce a valid *lossy* WebP (RIFF/WEBP container with a
    /// `VP8 ` chunk — lossless would be `VP8L`), and the quality knob must actually change size.
    #[test]
    fn webp_export_is_real_lossy() {
        let (w, h) = (96u32, 64u32);
        let px = gradient(w, h);
        let dir = std::env::temp_dir();
        let lo = dir.join("lb_webp_q10_test.webp");
        let hi = dir.join("lb_webp_q95_test.webp");

        save_webp_ffmpeg(&px, w, h, 10, false, &lo).expect("low-quality webp encode");
        save_webp_ffmpeg(&px, w, h, 95, false, &hi).expect("high-quality webp encode");

        let lo_bytes = std::fs::read(&lo).unwrap();
        let hi_bytes = std::fs::read(&hi).unwrap();

        // RIFF....WEBP container.
        assert_eq!(&lo_bytes[0..4], b"RIFF", "not a RIFF container");
        assert_eq!(&lo_bytes[8..12], b"WEBP", "not a WEBP file");
        // Lossy VP8 chunk (`VP8 ` with trailing space), NOT lossless `VP8L`.
        assert_eq!(&lo_bytes[12..16], b"VP8 ", "expected lossy VP8, got {:?}", &lo_bytes[12..16]);
        // The quality knob is honored: q10 is meaningfully smaller than q95.
        assert!(lo_bytes.len() < hi_bytes.len(),
            "quality ignored: q10 {} bytes >= q95 {} bytes", lo_bytes.len(), hi_bytes.len());

        std::fs::remove_file(&lo).ok();
        std::fs::remove_file(&hi).ok();
    }

    /// The format enum still advertises a quality control for WebP (now that it works).
    #[test]
    fn webp_has_quality() {
        assert!(ImageFormat::WebP.has_quality());
        assert!(ImageFormat::Jpeg.has_quality());
        assert!(!ImageFormat::Png.has_quality());
    }
}

/// Composite RGBA pixels onto an opaque black background, returning an RGB image.
fn flatten_alpha(img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>>) -> image::ImageBuffer<image::Rgb<u8>, Vec<u8>> {
    use image::{ImageBuffer, Rgb};
    ImageBuffer::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        let a = p[3] as f32 / 255.0;
        Rgb([
            (p[0] as f32 * a) as u8,
            (p[1] as f32 * a) as u8,
            (p[2] as f32 * a) as u8,
        ])
    })
}
