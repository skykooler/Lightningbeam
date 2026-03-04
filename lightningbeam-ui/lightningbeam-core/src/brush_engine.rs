//! Raster brush engine — pure-Rust MyPaint-style Gaussian dab renderer
//!
//! ## Algorithm
//!
//! Based on the libmypaint brush engine (ISC license, Martin Renold et al.).
//!
//! ### Dab shape
//! For each pixel at normalised distance `r = dist / radius` from the dab centre,
//! the opacity weight uses a flat inner core and smooth quadratic outer falloff:
//!
//! - `r > 1`: opa = 0 (outside dab)
//! - `r ≤ hardness` (or hardness = 1): opa = 1 (fully opaque core)
//! - `hardness < r ≤ 1`: `opa = ((1 - r) / (1 - hardness))²` (smooth falloff)
//!
//! The GPU compute shader (`brush_dab.wgsl`) is the authoritative implementation.
//!
//! ### Dab placement
//! Dabs are placed along the stroke polyline at intervals of
//! `spacing = radius * dabs_per_radius`.  Fractional remainder is tracked across
//! consecutive calls via `StrokeState`.
//!
//! ### Blending
//! Normal mode uses the standard "over" operator on premultiplied RGBA:
//! ```text
//! result_a = opa_a + (1 - opa_a) * bottom_a
//! result_rgb = opa_a * top_rgb + (1 - opa_a) * bottom_rgb
//! ```
//! Erase mode: subtract `opa_a` from the destination alpha and premultiply.

use image::RgbaImage;
use crate::raster_layer::{RasterBlendMode, StrokeRecord};

/// A single brush dab ready for GPU dispatch.
///
/// Padded to 64 bytes (4 × 16 bytes) for WGSL struct alignment in a storage buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuDab {
    /// Dab centre X (canvas pixels)
    pub x: f32,
    /// Dab centre Y (canvas pixels)
    pub y: f32,
    /// Dab radius (pixels)
    pub radius: f32,
    /// Hardness 0.0–1.0 (controls the falloff curve shape)
    pub hardness: f32,

    /// Composite opacity for this dab
    pub opacity: f32,
    /// Brush color R (linear, premultiplied)
    pub color_r: f32,
    /// Brush color G
    pub color_g: f32,
    /// Brush color B
    pub color_b: f32,

    /// Brush color A
    pub color_a: f32,
    /// Normalized stroke direction X (smudge only; 0 otherwise)
    pub ndx: f32,
    /// Normalized stroke direction Y (smudge only; 0 otherwise)
    pub ndy: f32,
    /// Distance to sample behind stroke for smudge (smudge only; 0 otherwise)
    pub smudge_dist: f32,

    /// Blend mode: 0 = Normal, 1 = Erase, 2 = Smudge
    pub blend_mode: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// Transient brush stroke state (tracks partial dab position between segments)
pub struct StrokeState {
    /// Distance along the path already "consumed" toward the next dab (in pixels)
    pub distance_since_last_dab: f32,
}

impl StrokeState {
    pub fn new() -> Self {
        Self { distance_since_last_dab: 0.0 }
    }
}

impl Default for StrokeState {
    fn default() -> Self { Self::new() }
}

/// Pure-Rust MyPaint-style Gaussian dab brush engine
pub struct BrushEngine;

impl BrushEngine {
    /// Compute the list of GPU dabs for a stroke segment.
    ///
    /// Uses the same dab-spacing logic as [`apply_stroke_with_state`] but produces
    /// [`GpuDab`] structs for upload to the GPU compute pipeline instead of painting
    /// into a pixel buffer.
    ///
    /// Also returns the union bounding box of all dabs as `(x0, y0, x1, y1)` in
    /// integer canvas pixel coordinates (clamped to non-negative values; `x0==i32::MAX`
    /// when the returned Vec is empty).
    pub fn compute_dabs(
        stroke: &StrokeRecord,
        state: &mut StrokeState,
    ) -> (Vec<GpuDab>, (i32, i32, i32, i32)) {
        let mut dabs: Vec<GpuDab> = Vec::new();
        let mut bbox = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);

        let blend_mode_u = match stroke.blend_mode {
            RasterBlendMode::Normal => 0u32,
            RasterBlendMode::Erase  => 1u32,
            RasterBlendMode::Smudge => 2u32,
        };

        let push_dab = |dabs: &mut Vec<GpuDab>,
                             bbox: &mut (i32, i32, i32, i32),
                             x: f32, y: f32,
                             radius: f32, opacity: f32,
                             ndx: f32, ndy: f32, smudge_dist: f32| {
            let r_fringe = radius + 1.0;
            bbox.0 = bbox.0.min((x - r_fringe).floor() as i32);
            bbox.1 = bbox.1.min((y - r_fringe).floor() as i32);
            bbox.2 = bbox.2.max((x + r_fringe).ceil() as i32);
            bbox.3 = bbox.3.max((y + r_fringe).ceil() as i32);
            dabs.push(GpuDab {
                x, y, radius,
                hardness: stroke.brush_settings.hardness,
                opacity,
                color_r: stroke.color[0],
                color_g: stroke.color[1],
                color_b: stroke.color[2],
                color_a: stroke.color[3],
                ndx, ndy, smudge_dist,
                blend_mode: blend_mode_u,
                _pad0: 0, _pad1: 0, _pad2: 0,
            });
        };

        if stroke.points.len() < 2 {
            if let Some(pt) = stroke.points.first() {
                let r = stroke.brush_settings.radius_at_pressure(pt.pressure);
                let o = stroke.brush_settings.opacity_at_pressure(pt.pressure);
                // Single-tap smudge has no direction — skip (same as CPU engine)
                if !matches!(stroke.blend_mode, RasterBlendMode::Smudge) {
                    push_dab(&mut dabs, &mut bbox, pt.x, pt.y, r, o, 0.0, 0.0, 0.0);
                }
                state.distance_since_last_dab = 0.0;
            }
            return (dabs, bbox);
        }

        for window in stroke.points.windows(2) {
            let p0 = &window[0];
            let p1 = &window[1];

            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let seg_len = (dx * dx + dy * dy).sqrt();
            if seg_len < 1e-4 { continue; }

            let mut t = 0.0f32;
            while t < 1.0 {
                let pressure = p0.pressure + t * (p1.pressure - p0.pressure);
                let radius = stroke.brush_settings.radius_at_pressure(pressure);
                let spacing = (radius * stroke.brush_settings.dabs_per_radius).max(0.5);

                let dist_to_next = spacing - state.distance_since_last_dab;
                let seg_t_to_next = (dist_to_next / seg_len).max(0.0);

                if seg_t_to_next > 1.0 - t {
                    state.distance_since_last_dab += seg_len * (1.0 - t);
                    break;
                }

                t += seg_t_to_next;
                let x2 = p0.x + t * dx;
                let y2 = p0.y + t * dy;
                let pressure2 = p0.pressure + t * (p1.pressure - p0.pressure);
                let radius2 = stroke.brush_settings.radius_at_pressure(pressure2);
                let opacity2 = stroke.brush_settings.opacity_at_pressure(pressure2);

                if matches!(stroke.blend_mode, RasterBlendMode::Smudge) {
                    let ndx = dx / seg_len;
                    let ndy = dy / seg_len;
                    let smudge_dist =
                        (radius2 * stroke.brush_settings.dabs_per_radius).max(1.0);
                    push_dab(&mut dabs, &mut bbox,
                             x2, y2, radius2, opacity2, ndx, ndy, smudge_dist);
                } else {
                    push_dab(&mut dabs, &mut bbox,
                             x2, y2, radius2, opacity2, 0.0, 0.0, 0.0);
                }

                state.distance_since_last_dab = 0.0;
            }
        }

        (dabs, bbox)
    }
}


/// Create an `RgbaImage` from a raw RGBA pixel buffer.
///
/// If `raw` is empty a blank (transparent) image of the given dimensions is returned.
/// Panics if `raw.len() != width * height * 4` (and `raw` is non-empty).
pub fn image_from_raw(raw: Vec<u8>, width: u32, height: u32) -> RgbaImage {
    if raw.is_empty() {
        RgbaImage::new(width, height)
    } else {
        RgbaImage::from_raw(width, height, raw)
            .expect("raw_pixels length mismatch")
    }
}

/// Encode an `RgbaImage` as a PNG byte vector
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, String> {
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("PNG encode error: {e}"))?;
    Ok(buf.into_inner())
}

/// Decode PNG bytes into an `RgbaImage`
pub fn decode_png(data: &[u8]) -> Result<RgbaImage, String> {
    image::load_from_memory(data)
        .map(|img| img.to_rgba8())
        .map_err(|e| format!("PNG decode error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_png_roundtrip() {
        let mut img = RgbaImage::new(64, 64);
        let px = img.get_pixel_mut(10, 10);
        *px = image::Rgba([255, 128, 0, 255]);
        let png = encode_png(&img).unwrap();
        let decoded = decode_png(&png).unwrap();
        assert_eq!(decoded.get_pixel(10, 10), img.get_pixel(10, 10));
    }
}
