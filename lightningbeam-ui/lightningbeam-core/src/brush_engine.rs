//! Raster brush engine — pure-Rust MyPaint-style Gaussian dab renderer
//!
//! ## Algorithm
//!
//! Based on the libmypaint brush engine (ISC license, Martin Renold et al.).
//!
//! ### Dab shape
//! For each pixel at normalised squared distance `rr = (dist / radius)²` from the
//! dab centre, the opacity weight is calculated using two linear segments:
//!
//! ```text
//! opa
//! ^
//! *   .
//! |        *
//! |          .
//! +-----------*> rr
//! 0  hardness  1
//! ```
//!
//! - segment 1 (rr ≤ hardness): `opa = 1 + rr * (-(1/hardness - 1))`
//! - segment 2 (hardness < rr ≤ 1): `opa = hardness/(1-hardness) - rr * hardness/(1-hardness)`
//! - rr > 1: opa = 0
//!
//! ### Dab placement
//! Dabs are placed along the stroke polyline at intervals of
//! `spacing = radius * dabs_per_radius`.  Fractional remainder is tracked across
//! consecutive `apply_stroke` calls via `StrokeState`.
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

/// Transient brush stroke state (tracks partial dab position between segments)
pub struct StrokeState {
    /// Distance along the path already "consumed" toward the next dab (in pixels)
    pub distance_since_last_dab: f32,
    /// Accumulated canvas color for smudge mode (RGBA linear, updated each dab)
    pub smudge_color: [f32; 4],
}

impl StrokeState {
    pub fn new() -> Self {
        Self { distance_since_last_dab: 0.0, smudge_color: [0.0; 4] }
    }
}

impl Default for StrokeState {
    fn default() -> Self { Self::new() }
}

/// Pure-Rust MyPaint-style Gaussian dab brush engine
pub struct BrushEngine;

impl BrushEngine {
    /// Apply a complete stroke to a pixel buffer.
    ///
    /// A fresh [`StrokeState`] is created for each stroke (starts with full dab
    /// placement spacing so the first dab lands at the very first point).
    pub fn apply_stroke(buffer: &mut RgbaImage, stroke: &StrokeRecord) {
        let mut state = StrokeState::new();
        // Ensure the very first point always gets a dab
        state.distance_since_last_dab = f32::MAX;
        Self::apply_stroke_with_state(buffer, stroke, &mut state);
    }

    /// Apply a stroke segment to a buffer while preserving dab-placement state.
    ///
    /// Use this when building up a stroke incrementally (e.g. live drawing) so
    /// that dab spacing is consistent across motion events.
    pub fn apply_stroke_with_state(
        buffer: &mut RgbaImage,
        stroke: &StrokeRecord,
        state: &mut StrokeState,
    ) {
        if stroke.points.len() < 2 {
            // Single-point "tap": draw one dab at the given pressure
            if let Some(pt) = stroke.points.first() {
                let r = stroke.brush_settings.radius_at_pressure(pt.pressure);
                let o = stroke.brush_settings.opacity_at_pressure(pt.pressure);
                if matches!(stroke.blend_mode, RasterBlendMode::Smudge) {
                    // Seed smudge color from canvas at the tap position
                    state.smudge_color = Self::sample_average(buffer, pt.x, pt.y, r);
                    Self::render_dab(buffer, pt.x, pt.y, r, stroke.brush_settings.hardness,
                                     o, state.smudge_color, RasterBlendMode::Normal);
                } else {
                    Self::render_dab(buffer, pt.x, pt.y, r, stroke.brush_settings.hardness,
                                     o, stroke.color, stroke.blend_mode);
                }
                state.distance_since_last_dab = 0.0;
            }
            return;
        }

        for window in stroke.points.windows(2) {
            let p0 = &window[0];
            let p1 = &window[1];

            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let seg_len = (dx * dx + dy * dy).sqrt();
            if seg_len < 1e-4 {
                continue;
            }

            // Interpolate across this segment
            let mut t = 0.0f32;
            while t < 1.0 {
                let pressure = p0.pressure + t * (p1.pressure - p0.pressure);

                let radius = stroke.brush_settings.radius_at_pressure(pressure);
                let spacing = radius * stroke.brush_settings.dabs_per_radius;
                let spacing = spacing.max(0.5); // at least half a pixel

                let dist_to_next = spacing - state.distance_since_last_dab;
                let seg_t_to_next = (dist_to_next / seg_len).max(0.0);

                if seg_t_to_next > 1.0 - t {
                    // Not enough distance left in this segment for another dab
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
                    // Sample canvas under dab, blend into running smudge color
                    let sampled = Self::sample_average(buffer, x2, y2, radius2);
                    const PICK_UP: f32 = 0.15;
                    for i in 0..4 {
                        state.smudge_color[i] = state.smudge_color[i] * (1.0 - PICK_UP)
                            + sampled[i] * PICK_UP;
                    }
                    Self::render_dab(buffer, x2, y2, radius2,
                                     stroke.brush_settings.hardness,
                                     opacity2, state.smudge_color, RasterBlendMode::Normal);
                } else {
                    Self::render_dab(buffer, x2, y2, radius2,
                                     stroke.brush_settings.hardness,
                                     opacity2, stroke.color, stroke.blend_mode);
                }

                state.distance_since_last_dab = 0.0;
            }
        }
    }

    /// Render a single Gaussian dab at pixel position (x, y).
    ///
    /// Uses the two-segment linear falloff from MyPaint/libmypaint for the
    /// opacity mask, then blends using the requested `blend_mode`.
    pub fn render_dab(
        buffer: &mut RgbaImage,
        x: f32,
        y: f32,
        radius: f32,
        hardness: f32,
        opacity: f32,
        color: [f32; 4],
        blend_mode: RasterBlendMode,
    ) {
        if radius < 0.5 || opacity <= 0.0 {
            return;
        }

        let hardness = hardness.clamp(1e-3, 1.0);

        // Pre-compute the two linear-segment coefficients (from libmypaint render_dab_mask)
        let seg1_offset = 1.0f32;
        let seg1_slope = -(1.0 / hardness - 1.0);
        let seg2_offset = hardness / (1.0 - hardness);
        let seg2_slope = -hardness / (1.0 - hardness);

        let r_fringe = radius + 1.0;
        let x0 = ((x - r_fringe).floor() as i32).max(0) as u32;
        let y0 = ((y - r_fringe).floor() as i32).max(0) as u32;
        let x1 = ((x + r_fringe).ceil() as i32).min(buffer.width() as i32 - 1).max(0) as u32;
        let y1 = ((y + r_fringe).ceil() as i32).min(buffer.height() as i32 - 1).max(0) as u32;

        let one_over_r2 = 1.0 / (radius * radius);

        for py in y0..=y1 {
            for px in x0..=x1 {
                let dx = px as f32 + 0.5 - x;
                let dy = py as f32 + 0.5 - y;
                let rr = (dx * dx + dy * dy) * one_over_r2;

                if rr > 1.0 {
                    continue;
                }

                // Two-segment opacity (identical to libmypaint calculate_opa)
                let opa_weight = if rr <= hardness {
                    seg1_offset + rr * seg1_slope
                } else {
                    seg2_offset + rr * seg2_slope
                }
                .clamp(0.0, 1.0);

                let dab_alpha = opa_weight * opacity * color[3];
                if dab_alpha <= 0.0 {
                    continue;
                }

                let pixel = buffer.get_pixel_mut(px, py);
                let dst = [
                    pixel[0] as f32 / 255.0,
                    pixel[1] as f32 / 255.0,
                    pixel[2] as f32 / 255.0,
                    pixel[3] as f32 / 255.0,
                ];

                let (out_r, out_g, out_b, out_a) = match blend_mode {
                    RasterBlendMode::Normal | RasterBlendMode::Smudge => {
                        // Standard "over" operator (smudge pre-computes its color upstream)
                        let oa = dab_alpha;
                        let ba = 1.0 - oa;
                        let out_a = oa + ba * dst[3];
                        let out_r = oa * color[0] + ba * dst[0];
                        let out_g = oa * color[1] + ba * dst[1];
                        let out_b = oa * color[2] + ba * dst[2];
                        (out_r, out_g, out_b, out_a)
                    }
                    RasterBlendMode::Erase => {
                        // Reduce destination alpha by dab_alpha
                        let new_a = (dst[3] - dab_alpha).max(0.0);
                        let scale = if dst[3] > 1e-6 { new_a / dst[3] } else { 0.0 };
                        (dst[0] * scale, dst[1] * scale, dst[2] * scale, new_a)
                    }
                };

                pixel[0] = (out_r.clamp(0.0, 1.0) * 255.0) as u8;
                pixel[1] = (out_g.clamp(0.0, 1.0) * 255.0) as u8;
                pixel[2] = (out_b.clamp(0.0, 1.0) * 255.0) as u8;
                pixel[3] = (out_a.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }

    /// Sample the average RGBA color in a circular region of `radius` around (x, y).
    ///
    /// Used by smudge to pick up canvas color before painting each dab.
    fn sample_average(buffer: &RgbaImage, x: f32, y: f32, radius: f32) -> [f32; 4] {
        let sample_r = (radius * 0.5).max(1.0);
        let x0 = ((x - sample_r).floor() as i32).max(0) as u32;
        let y0 = ((y - sample_r).floor() as i32).max(0) as u32;
        let x1 = ((x + sample_r).ceil() as i32).min(buffer.width() as i32 - 1).max(0) as u32;
        let y1 = ((y + sample_r).ceil() as i32).min(buffer.height() as i32 - 1).max(0) as u32;

        let mut sum = [0.0f32; 4];
        let mut count = 0u32;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let p = buffer.get_pixel(px, py);
                sum[0] += p[0] as f32 / 255.0;
                sum[1] += p[1] as f32 / 255.0;
                sum[2] += p[2] as f32 / 255.0;
                sum[3] += p[3] as f32 / 255.0;
                count += 1;
            }
        }
        if count > 0 {
            let n = count as f32;
            [sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n]
        } else {
            [0.0; 4]
        }
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
    use crate::raster_layer::{StrokePoint, StrokeRecord, RasterBlendMode};
    use crate::brush_settings::BrushSettings;

    fn make_stroke(color: [f32; 4]) -> StrokeRecord {
        StrokeRecord {
            brush_settings: BrushSettings::default_round_hard(),
            color,
            blend_mode: RasterBlendMode::Normal,
            points: vec![
                StrokePoint { x: 10.0, y: 10.0, pressure: 0.8, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0 },
                StrokePoint { x: 50.0, y: 10.0, pressure: 0.8, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.1 },
            ],
        }
    }

    #[test]
    fn test_stroke_modifies_buffer() {
        let mut img = RgbaImage::new(100, 100);
        let stroke = make_stroke([1.0, 0.0, 0.0, 1.0]); // red
        BrushEngine::apply_stroke(&mut img, &stroke);
        // The center pixel should have some red
        let px = img.get_pixel(30, 10);
        assert!(px[0] > 0, "expected red paint");
    }

    #[test]
    fn test_erase_reduces_alpha() {
        let mut img = RgbaImage::from_pixel(100, 100, image::Rgba([200, 100, 50, 255]));
        let stroke = StrokeRecord {
            brush_settings: BrushSettings::default_round_hard(),
            color: [0.0, 0.0, 0.0, 1.0],
            blend_mode: RasterBlendMode::Erase,
            points: vec![
                StrokePoint { x: 50.0, y: 50.0, pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, timestamp: 0.0 },
            ],
        };
        BrushEngine::apply_stroke(&mut img, &stroke);
        let px = img.get_pixel(50, 50);
        assert!(px[3] < 255, "alpha should be reduced by erase");
    }

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
