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
//! Spacing = 1 / max(dabs_per_basic_radius/radius, dabs_per_actual_radius/actual_radius).
//! Fractional remainder is tracked across consecutive calls via `StrokeState`.
//!
//! ### Blending
//! Normal mode uses the standard "over" operator on premultiplied RGBA.
//! Erase mode subtracts from destination alpha.

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
    /// Elliptical dab aspect ratio (1.0 = circle)
    pub elliptical_dab_ratio: f32,
    /// Elliptical dab rotation angle in radians
    pub elliptical_dab_angle: f32,
    /// Lock alpha: 0.0 = modify alpha normally, 1.0 = don't modify destination alpha
    pub lock_alpha: f32,
}

/// Transient brush stroke state (tracks position and randomness between segments)
pub struct StrokeState {
    /// Distance along the path already "consumed" toward the next dab (in pixels)
    pub distance_since_last_dab: f32,
    /// Exponentially-smoothed cursor X for slow_tracking
    pub smooth_x: f32,
    /// Exponentially-smoothed cursor Y for slow_tracking
    pub smooth_y: f32,
    /// Whether smooth_x/y have been initialised yet
    pub smooth_initialized: bool,
    /// xorshift32 seed for jitter and radius variation
    pub rng_seed: u32,
    /// Accumulated per-dab hue shift
    pub color_h_phase: f32,
    /// Accumulated per-dab value shift
    pub color_v_phase: f32,
    /// Accumulated per-dab saturation shift
    pub color_s_phase: f32,
}

impl StrokeState {
    pub fn new() -> Self {
        Self {
            distance_since_last_dab: 0.0,
            smooth_x: 0.0,
            smooth_y: 0.0,
            smooth_initialized: false,
            rng_seed: 0xDEAD_BEEF,
            color_h_phase: 0.0,
            color_v_phase: 0.0,
            color_s_phase: 0.0,
        }
    }
}

impl Default for StrokeState {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// xorshift32 — fast, no-alloc PRNG. Returns a value in [0, 1).
#[inline]
fn xorshift(seed: &mut u32) -> f32 {
    let mut s = *seed;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *seed = s;
    (s as f32) / (u32::MAX as f32)
}

/// Convert linear RGB (premultiplied, alpha already separated) to HSV.
/// Input: r, g, b in [0, 1] (not premultiplied; caller divides by alpha first).
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let v = max;
    let s = if max > 1e-6 { delta / max } else { 0.0 };
    let h = if delta < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    (h, s, v)
}

/// Convert HSV to linear RGB.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor() as i32;
    let f = h6 - i as f32;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

/// Pure-Rust MyPaint-style Gaussian dab brush engine
pub struct BrushEngine;

impl BrushEngine {
    /// Compute the list of GPU dabs for a stroke segment.
    ///
    /// Uses the MyPaint dab-spacing formula and produces [`GpuDab`] structs for
    /// upload to the GPU compute pipeline.
    ///
    /// Also returns the union bounding box of all dabs as `(x0, y0, x1, y1)` in
    /// integer canvas pixel coordinates (`x0==i32::MAX` when the Vec is empty).
    pub fn compute_dabs(
        stroke: &StrokeRecord,
        state: &mut StrokeState,
    ) -> (Vec<GpuDab>, (i32, i32, i32, i32)) {
        let mut dabs: Vec<GpuDab> = Vec::new();
        let mut bbox = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let bs = &stroke.brush_settings;

        // Determine blend mode, allowing brush settings to override Normal
        let base_blend = match stroke.blend_mode {
            RasterBlendMode::Normal if bs.eraser > 0.5 => RasterBlendMode::Erase,
            RasterBlendMode::Normal if bs.smudge  > 0.5 => RasterBlendMode::Smudge,
            other => other,
        };
        let blend_mode_u = match base_blend {
            RasterBlendMode::Normal => 0u32,
            RasterBlendMode::Erase  => 1u32,
            RasterBlendMode::Smudge => 2u32,
        };

        let push_dab = |dabs: &mut Vec<GpuDab>,
                        bbox: &mut (i32, i32, i32, i32),
                        x: f32, y: f32,
                        radius: f32, opacity: f32,
                        cr: f32, cg: f32, cb: f32,
                        ndx: f32, ndy: f32, smudge_dist: f32| {
            let r_fringe = radius + 1.0;
            bbox.0 = bbox.0.min((x - r_fringe).floor() as i32);
            bbox.1 = bbox.1.min((y - r_fringe).floor() as i32);
            bbox.2 = bbox.2.max((x + r_fringe).ceil() as i32);
            bbox.3 = bbox.3.max((y + r_fringe).ceil() as i32);
            dabs.push(GpuDab {
                x, y, radius,
                hardness: bs.hardness,
                opacity,
                color_r: cr,
                color_g: cg,
                color_b: cb,
                color_a: stroke.color[3],
                ndx, ndy, smudge_dist,
                blend_mode: blend_mode_u,
                elliptical_dab_ratio: bs.elliptical_dab_ratio.max(1.0),
                elliptical_dab_angle: bs.elliptical_dab_angle.to_radians(),
                lock_alpha: bs.lock_alpha,
            });
        };

        if stroke.points.len() < 2 {
            if let Some(pt) = stroke.points.first() {
                let r = bs.radius_at_pressure(pt.pressure);
                // Default dpr for a single tap: prefer actual_radius spacing
                let dpr = if bs.dabs_per_radius > 0.0 { bs.dabs_per_radius }
                          else { bs.dabs_per_actual_radius.max(0.01) };
                let raw_o = bs.opacity_at_pressure(pt.pressure);
                let o = (1.0 - (1.0 - raw_o).powf(dpr * 0.5)
                        * (1.0 + bs.opaque_multiply)).clamp(0.0, 1.0);
                if !matches!(base_blend, RasterBlendMode::Smudge) {
                    let (cr, cg, cb) = (stroke.color[0], stroke.color[1], stroke.color[2]);
                    push_dab(&mut dabs, &mut bbox, pt.x, pt.y, r, o, cr, cg, cb,
                             0.0, 0.0, 0.0);
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
                let radius2 = bs.radius_at_pressure(pressure);

                // Spacing: densest wins between basic-radius and actual-radius methods.
                // dabs_per_basic_radius = N dabs per basic_radius pixels → spacing = basic_r / N
                // dabs_per_actual_radius = N dabs per actual_radius pixels → spacing = actual_r / N
                let spacing_basic  = if bs.dabs_per_radius > 0.0 {
                    radius2 / bs.dabs_per_radius
                } else { f32::MAX };
                let spacing_actual = if bs.dabs_per_actual_radius > 0.0 {
                    radius2 / bs.dabs_per_actual_radius
                } else { f32::MAX };
                let spacing = spacing_basic.min(spacing_actual).max(0.5);

                let dist_to_next = spacing - state.distance_since_last_dab;
                let seg_t_to_next = (dist_to_next / seg_len).max(0.0);

                if seg_t_to_next > 1.0 - t {
                    state.distance_since_last_dab += seg_len * (1.0 - t);
                    break;
                }

                t += seg_t_to_next;
                let pressure2 = p0.pressure + t * (p1.pressure - p0.pressure);

                // Stroke threshold gating
                if pressure2 < bs.stroke_threshold {
                    state.distance_since_last_dab = 0.0;
                    continue;
                }

                let mut radius2 = bs.radius_at_pressure(pressure2);

                // Opacity — normalised so dense dabs don't saturate faster than sparse ones
                let dpr = radius2 / spacing;   // effective dabs per radius
                let raw_opacity = bs.opacity_at_pressure(pressure2);
                let mut opacity2 = 1.0 - (1.0 - raw_opacity).powf(dpr * 0.5);
                opacity2 = (opacity2 * (1.0 + bs.opaque_multiply)).clamp(0.0, 1.0);

                // Slow tracking: exponential position smoothing
                let x2 = p0.x + t * dx;
                let y2 = p0.y + t * dy;
                if !state.smooth_initialized {
                    state.smooth_x = x2; state.smooth_y = y2;
                    state.smooth_initialized = true;
                }
                let k = if bs.slow_tracking > 0.0 {
                    (-spacing / bs.slow_tracking.max(0.1)).exp()
                } else { 0.0 };
                state.smooth_x = state.smooth_x * k + x2 * (1.0 - k);
                state.smooth_y = state.smooth_y * k + y2 * (1.0 - k);
                let mut ex = state.smooth_x;
                let mut ey = state.smooth_y;

                // Radius jitter (log-scale)
                if bs.radius_by_random != 0.0 {
                    let r_rng = xorshift(&mut state.rng_seed) * 2.0 - 1.0;
                    radius2 = (radius2 * (bs.radius_by_random * r_rng).exp()).clamp(0.5, 500.0);
                }

                // Position jitter + fixed offset
                if bs.offset_by_random != 0.0 || bs.offset_x != 0.0 || bs.offset_y != 0.0 {
                    let jitter = bs.offset_by_random * radius2;
                    ex += (xorshift(&mut state.rng_seed) * 2.0 - 1.0) * jitter
                        + bs.offset_x * radius2;
                    ey += (xorshift(&mut state.rng_seed) * 2.0 - 1.0) * jitter
                        + bs.offset_y * radius2;
                }

                // Per-dab color phase shifts
                state.color_h_phase += bs.change_color_h;
                state.color_v_phase += bs.change_color_v;
                state.color_s_phase += bs.change_color_hsv_s;

                let (mut cr, mut cg, mut cb) = (
                    stroke.color[0], stroke.color[1], stroke.color[2],
                );
                let ca = stroke.color[3];
                if ca > 1e-6 {
                    // un-premultiply for HSV conversion
                    let (ur, ug, ub) = (cr / ca, cg / ca, cb / ca);
                    let (mut h, mut s, mut v) = rgb_to_hsv(ur, ug, ub);
                    if bs.change_color_h != 0.0 || bs.change_color_v != 0.0
                       || bs.change_color_hsv_s != 0.0 {
                        h = (h + state.color_h_phase).rem_euclid(1.0);
                        v = (v + state.color_v_phase).clamp(0.0, 1.0);
                        s = (s + state.color_s_phase).clamp(0.0, 1.0);
                        let (r2, g2, b2) = hsv_to_rgb(h, s, v);
                        cr = r2 * ca; cg = g2 * ca; cb = b2 * ca;
                    }
                }

                if matches!(base_blend, RasterBlendMode::Smudge) {
                    let ndx = dx / seg_len;
                    let ndy = dy / seg_len;
                    let smudge_dist =
                        (radius2 * dpr).max(1.0) * bs.smudge_radius_log.exp();
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, cr, cg, cb,
                             ndx, ndy, smudge_dist);
                } else {
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, cr, cg, cb,
                             0.0, 0.0, 0.0);
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
