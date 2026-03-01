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

        let mut push_dab = |dabs: &mut Vec<GpuDab>,
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
                // Smudge has no drag direction on a single tap — skip painting
                if !matches!(stroke.blend_mode, RasterBlendMode::Smudge) {
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
                    // Directional warp smudge: each pixel in the dab footprint
                    // samples from a position offset backwards along the stroke,
                    // preserving lateral color structure.
                    let ndx = dx / seg_len;
                    let ndy = dy / seg_len;
                    let smudge_dist = (radius2 * stroke.brush_settings.dabs_per_radius).max(1.0);
                    Self::render_smudge_dab(buffer, x2, y2, radius2,
                                            stroke.brush_settings.hardness,
                                            opacity2, ndx, ndy, smudge_dist);
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
                        // Multiplicative erase: each dab removes dab_alpha *fraction* of remaining
                        // alpha. This prevents dense overlapping dabs from summing past 1.0 and
                        // fully erasing at low opacity — opacity now controls the per-dab fraction
                        // removed rather than an absolute amount.
                        let new_a = dst[3] * (1.0 - dab_alpha);
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

    /// Render a smudge dab using directional per-pixel warp.
    ///
    /// Each pixel in the dab footprint samples from the canvas at a position offset
    /// backwards along `(ndx, ndy)` by `smudge_dist` pixels, then blends that
    /// sampled color over the current pixel weighted by the dab opacity.
    ///
    /// Because each pixel samples its own source position, lateral color structure
    /// is preserved: dragging over a 1-pixel dot with a 20-pixel brush produces a
    /// narrow streak rather than a uniform smear.
    ///
    /// Updates are collected before any writes to avoid read/write aliasing.
    fn render_smudge_dab(
        buffer: &mut RgbaImage,
        x: f32,
        y: f32,
        radius: f32,
        hardness: f32,
        opacity: f32,
        ndx: f32,        // normalized stroke direction x
        ndy: f32,        // normalized stroke direction y
        smudge_dist: f32,
    ) {
        if radius < 0.5 || opacity <= 0.0 {
            return;
        }

        let hardness = hardness.clamp(1e-3, 1.0);
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

        // Collect updates before writing to avoid aliasing between source and dest reads
        let mut updates: Vec<(u32, u32, [u8; 4])> = Vec::new();

        for py in y0..=y1 {
            for px in x0..=x1 {
                let fdx = px as f32 + 0.5 - x;
                let fdy = py as f32 + 0.5 - y;
                let rr = (fdx * fdx + fdy * fdy) * one_over_r2;

                if rr > 1.0 {
                    continue;
                }

                let opa_weight = if rr <= hardness {
                    seg1_offset + rr * seg1_slope
                } else {
                    seg2_offset + rr * seg2_slope
                }
                .clamp(0.0, 1.0);

                let alpha = opa_weight * opacity;
                if alpha <= 0.0 {
                    continue;
                }

                // Sample from one dab-spacing behind the current position along stroke
                let src_x = px as f32 + 0.5 - ndx * smudge_dist;
                let src_y = py as f32 + 0.5 - ndy * smudge_dist;
                let src = Self::sample_bilinear(buffer, src_x, src_y);

                let dst = buffer.get_pixel(px, py);
                let da = 1.0 - alpha;
                let out = [
                    ((alpha * src[0] + da * dst[0] as f32 / 255.0).clamp(0.0, 1.0) * 255.0) as u8,
                    ((alpha * src[1] + da * dst[1] as f32 / 255.0).clamp(0.0, 1.0) * 255.0) as u8,
                    ((alpha * src[2] + da * dst[2] as f32 / 255.0).clamp(0.0, 1.0) * 255.0) as u8,
                    ((alpha * src[3] + da * dst[3] as f32 / 255.0).clamp(0.0, 1.0) * 255.0) as u8,
                ];
                updates.push((px, py, out));
            }
        }

        for (px, py, rgba) in updates {
            let p = buffer.get_pixel_mut(px, py);
            p[0] = rgba[0];
            p[1] = rgba[1];
            p[2] = rgba[2];
            p[3] = rgba[3];
        }
    }

    /// Bilinearly sample a floating-point position from the buffer, clamped to bounds.
    fn sample_bilinear(buffer: &RgbaImage, x: f32, y: f32) -> [f32; 4] {
        let w = buffer.width() as i32;
        let h = buffer.height() as i32;
        let x0 = (x.floor() as i32).clamp(0, w - 1);
        let y0 = (y.floor() as i32).clamp(0, h - 1);
        let x1 = (x0 + 1).min(w - 1);
        let y1 = (y0 + 1).min(h - 1);
        let fx = (x - x0 as f32).clamp(0.0, 1.0);
        let fy = (y - y0 as f32).clamp(0.0, 1.0);

        let p00 = buffer.get_pixel(x0 as u32, y0 as u32);
        let p10 = buffer.get_pixel(x1 as u32, y0 as u32);
        let p01 = buffer.get_pixel(x0 as u32, y1 as u32);
        let p11 = buffer.get_pixel(x1 as u32, y1 as u32);

        let mut out = [0.0f32; 4];
        for i in 0..4 {
            let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
            let bot = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
            out[i] = (top * (1.0 - fy) + bot * fy) / 255.0;
        }
        out
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
