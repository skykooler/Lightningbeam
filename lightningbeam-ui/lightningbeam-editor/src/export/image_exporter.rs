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
            if allow_transparency {
                img.save(path).map_err(|e| format!("WebP save failed: {e}"))
            } else {
                let flat = flatten_alpha(img);
                flat.save(path).map_err(|e| format!("WebP save failed: {e}"))
            }
        }
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
