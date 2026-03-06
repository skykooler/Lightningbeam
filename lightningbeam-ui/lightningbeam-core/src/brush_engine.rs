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
//! Follows the libmypaint model: distance-based and time-based contributions are
//! **summed** into a single `partial_dabs` accumulator.  A dab fires whenever the
//! accumulator reaches 1.0.
//!
//! Rate (dabs per pixel) = dabs_per_actual_radius / actual_radius
//!                       + dabs_per_basic_radius  / base_radius
//! Time contribution added per call = dt × dabs_per_second
//!
//! ### Opacity
//! Matches libmypaint's `opaque_linearize` formula.  `dabs_per_pixel` is a fixed
//! brush-level estimate of how many dabs overlap at any pixel:
//!
//! `dabs_per_pixel = 1 + opaque_linearize × ((dabs_per_actual + dabs_per_basic) × 2 - 1)`
//! `per_dab_alpha  = 1 - (1 - raw_opacity) ^ (1 / dabs_per_pixel)`
//!
//! With `opaque_linearize = 0` the raw opacity is used directly per dab.
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
    /// Fractional dab accumulator — reaches 1.0 when the next dab should fire.
    /// Initialised to 1.0 so the very first call always emits at least one dab.
    pub partial_dabs: f32,
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
            // Start at 1.0 so the first call always emits the stroke-start dab.
            partial_dabs: 1.0,
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

/// xorshift32 — fast, no-alloc PRNG.  Returns a value in [0, 1).
#[inline]
fn xorshift(seed: &mut u32) -> f32 {
    let mut s = *seed;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *seed = s;
    (s as f32) / (u32::MAX as f32)
}

/// Box-Muller Gaussian sample with mean 0 and std-dev 1.
/// Consumes two xorshift samples; the second half of the pair is discarded
/// (acceptable for brush jitter which doesn't need correlated pairs).
#[inline]
fn gaussian(seed: &mut u32) -> f32 {
    let u1 = xorshift(seed).max(1e-7); // avoid ln(0)
    let u2 = xorshift(seed);
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
}

/// Convert linear RGB (not premultiplied) to HSV.
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

// ---------------------------------------------------------------------------
// Per-dab effects helper
// ---------------------------------------------------------------------------

/// Apply per-dab randomness and color-shift effects, matching libmypaint.
///
/// Returns `(ex, ey, radius, opacity, cr, cg, cb)` ready for the dab emitter.
///
/// Opacity uses the `opaque_linearize` formula (same fixed brush-level estimate
/// whether called from the single-point path or the drag path).
///
/// Jitter uses Gaussian noise (matching libmypaint), not uniform.
///
/// Radius jitter applies an opacity correction `× (base_r / jittered_r)²` to
/// keep perceived ink-amount constant as radius varies (matches libmypaint).
fn apply_dab_effects(
    state: &mut StrokeState,
    bs: &crate::brush_settings::BrushSettings,
    x: f32, y: f32,
    base_radius: f32,  // radius_at_pressure(pressure), before jitter
    pressure: f32,
    color: [f32; 4],
) -> (f32, f32, f32, f32, f32, f32, f32) {
    // ---- Opacity (libmypaint opaque_linearize formula) --------------------
    // Estimate average dab overlap per pixel from brush settings (fixed, not
    // speed-dependent), then convert stroke-level opacity to per-dab alpha.
    let raw_dpp = ((bs.dabs_per_actual_radius + bs.dabs_per_radius) * 2.0).max(1.0);
    let dabs_per_pixel = (1.0 + bs.opaque_linearize * (raw_dpp - 1.0)).max(1.0);
    let raw_o = bs.opacity_at_pressure(pressure);
    let mut opacity = 1.0 - (1.0 - raw_o).powf(1.0 / dabs_per_pixel);

    // ---- Radius jitter (Gaussian in log-space, matching libmypaint) -------
    let mut radius = base_radius;
    if bs.radius_by_random != 0.0 {
        let noise = gaussian(&mut state.rng_seed) * bs.radius_by_random;
        let jittered_log = bs.radius_log + noise;
        radius = jittered_log.exp().clamp(0.5, 500.0);
        // Opacity correction: keep ink-amount constant as radius varies.
        let alpha_correction = (base_radius / radius).powi(2);
        opacity = (opacity * alpha_correction).clamp(0.0, 1.0);
    }

    // ---- Position jitter + fixed offset (Gaussian, matching libmypaint) ---
    let mut ex = x;
    let mut ey = y;
    if bs.offset_by_random != 0.0 || bs.offset_x != 0.0 || bs.offset_y != 0.0 {
        // libmypaint uses base_radius (no-pressure) for the jitter scale.
        let base_r_fixed = bs.radius_log.exp();
        ex += gaussian(&mut state.rng_seed) * bs.offset_by_random * base_r_fixed
            + bs.offset_x * base_r_fixed;
        ey += gaussian(&mut state.rng_seed) * bs.offset_by_random * base_r_fixed
            + bs.offset_y * base_r_fixed;
    }

    // ---- Per-dab color phase shifts ---------------------------------------
    state.color_h_phase += bs.change_color_h;
    state.color_v_phase += bs.change_color_v;
    state.color_s_phase += bs.change_color_hsv_s;

    let (mut cr, mut cg, mut cb) = (color[0], color[1], color[2]);
    let ca = color[3];
    if ca > 1e-6
        && (bs.change_color_h != 0.0
            || bs.change_color_v != 0.0
            || bs.change_color_hsv_s != 0.0)
    {
        let (ur, ug, ub) = (cr / ca, cg / ca, cb / ca);
        let (mut h, mut s, mut v) = rgb_to_hsv(ur, ug, ub);
        h = (h + state.color_h_phase).rem_euclid(1.0);
        v = (v + state.color_v_phase).clamp(0.0, 1.0);
        s = (s + state.color_s_phase).clamp(0.0, 1.0);
        let (r2, g2, b2) = hsv_to_rgb(h, s, v);
        cr = r2 * ca;
        cg = g2 * ca;
        cb = b2 * ca;
    }

    (ex, ey, radius, opacity.clamp(0.0, 1.0), cr, cg, cb)
}

// ---------------------------------------------------------------------------
// Brush engine
// ---------------------------------------------------------------------------

/// Pure-Rust MyPaint-style Gaussian dab brush engine
pub struct BrushEngine;

impl BrushEngine {
    /// Compute the list of GPU dabs for a stroke segment.
    ///
    /// `dt` is the elapsed time in seconds since the previous call for this
    /// stroke.  Pass `0.0` on the very first call (stroke start).
    ///
    /// Follows the libmypaint spacing model: distance-based and time-based
    /// contributions are **summed** in a single `partial_dabs` accumulator.
    /// A dab is emitted whenever `partial_dabs` reaches 1.0.
    ///
    /// Also returns the union bounding box of all dabs as `(x0, y0, x1, y1)`.
    pub fn compute_dabs(
        stroke: &StrokeRecord,
        state: &mut StrokeState,
        dt: f32,
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
            RasterBlendMode::Normal     => 0u32,
            RasterBlendMode::Erase      => 1u32,
            RasterBlendMode::Smudge     => 2u32,
            RasterBlendMode::CloneStamp => 3u32,
            RasterBlendMode::Healing      => 4u32,
            RasterBlendMode::PatternStamp => 5u32,
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
                // Clone stamp: color_r/color_g hold source canvas X/Y, so color_a = 1.0
                // (blend strength is opa_weight × opacity × 1.0 in the shader).
                color_a: if blend_mode_u == 3 || blend_mode_u == 4 { 1.0 } else { stroke.color[3] },
                ndx, ndy, smudge_dist,
                blend_mode: blend_mode_u,
                elliptical_dab_ratio: bs.elliptical_dab_ratio.max(1.0),
                elliptical_dab_angle: bs.elliptical_dab_angle.to_radians(),
                lock_alpha: bs.lock_alpha,
            });
        };

        // Time-based accumulation: dt × dabs_per_second contributes to partial_dabs
        // regardless of whether the cursor moved.
        // Cap dt to 0.1 s to avoid a burst of dabs after a long pause.
        let dt_capped = dt.min(0.1);
        state.partial_dabs += dt_capped * bs.dabs_per_second;

        // ----------------------------------------------------------------
        // Single-point path: emit time-based (and stroke-start) dabs.
        // The caller is responsible for timing; we just fire whenever
        // partial_dabs ≥ 1.0.
        // ----------------------------------------------------------------
        if stroke.points.len() < 2 {
            if let Some(pt) = stroke.points.first() {
                if !state.smooth_initialized {
                    state.smooth_x = pt.x;
                    state.smooth_y = pt.y;
                    state.smooth_initialized = true;
                }
                while state.partial_dabs >= 1.0 {
                    state.partial_dabs -= 1.0;
                    let base_r = bs.radius_at_pressure(pt.pressure);
                    let (ex, ey, r, o, cr, cg, cb) = apply_dab_effects(
                        state, bs, pt.x, pt.y, base_r, pt.pressure, stroke.color,
                    );
                    if !matches!(base_blend, RasterBlendMode::Smudge) {
                        let (cr2, cg2, cb2, ndx2, ndy2) = if matches!(base_blend, RasterBlendMode::CloneStamp | RasterBlendMode::Healing) {
                            // Store offset in color_r/color_g; shader adds it per-pixel.
                            let (ox, oy) = stroke.clone_src_offset.unwrap_or((0.0, 0.0));
                            (ox, oy, 0.0, 0.0, 0.0)
                        } else if matches!(base_blend, RasterBlendMode::PatternStamp) {
                            // ndx = pattern_type, ndy = pattern_scale
                            (cr, cg, cb, stroke.pattern_type as f32, stroke.pattern_scale)
                        } else {
                            (cr, cg, cb, 0.0, 0.0)
                        };
                        push_dab(&mut dabs, &mut bbox, ex, ey, r, o, cr2, cg2, cb2,
                                 ndx2, ndy2, 0.0);
                    }
                }
            }
            return (dabs, bbox);
        }

        // ----------------------------------------------------------------
        // Drag path: walk the polyline, accumulating partial_dabs from
        // both distance-based and time-based contributions.
        // ----------------------------------------------------------------

        // Track the last smoothed position so that any residual time-based
        // dabs can be emitted at the end of the segment walk.
        let mut last_smooth_x = state.smooth_x;
        let mut last_smooth_y = state.smooth_y;
        let mut last_pressure = stroke.points.last()
            .map(|p| p.pressure)
            .unwrap_or(1.0);

        // Fixed base radius (no pressure) used for the basic-radius spacing rate.
        let base_radius_fixed = bs.radius_log.exp();

        for window in stroke.points.windows(2) {
            let p0 = &window[0];
            let p1 = &window[1];

            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let seg_len = (dx * dx + dy * dy).sqrt();
            if seg_len < 1e-4 { continue; }

            last_pressure = p1.pressure;

            let mut t = 0.0f32;
            while t < 1.0 {
                let pressure = p0.pressure + t * (p1.pressure - p0.pressure);
                let radius_for_rate = bs.radius_at_pressure(pressure);

                // Dab rate = sum of distance-based contributions (dabs per pixel).
                // Matches libmypaint: dabs_per_actual/actual_r + dabs_per_basic/base_r.
                // For elliptical brushes use the minor-axis radius so dabs connect
                // when moving perpendicular to the major axis.
                let eff_radius = if bs.elliptical_dab_ratio > 1.001 {
                    radius_for_rate / bs.elliptical_dab_ratio
                } else {
                    radius_for_rate
                };
                let rate_actual = if bs.dabs_per_actual_radius > 0.0 {
                    bs.dabs_per_actual_radius / eff_radius
                } else { 0.0 };
                let rate_basic = if bs.dabs_per_radius > 0.0 {
                    bs.dabs_per_radius / base_radius_fixed
                } else { 0.0 };
                let rate = rate_actual + rate_basic; // dabs per pixel

                let remaining = 1.0 - state.partial_dabs;
                let pixels_to_next = if rate > 1e-8 { remaining / rate } else { f32::MAX };
                let seg_t_to_next = (pixels_to_next / seg_len).max(0.0);

                if seg_t_to_next > 1.0 - t {
                    // Won't reach the next dab within this segment.
                    if rate > 1e-8 {
                        state.partial_dabs += (1.0 - t) * seg_len * rate;
                    }
                    break;
                }

                t += seg_t_to_next;
                let pressure2 = p0.pressure + t * (p1.pressure - p0.pressure);

                // Stroke threshold gating
                if pressure2 < bs.stroke_threshold {
                    state.partial_dabs = 0.0;
                    continue;
                }

                let base_r2 = bs.radius_at_pressure(pressure2);

                // Slow tracking: exponential position smoothing
                let x2 = p0.x + t * dx;
                let y2 = p0.y + t * dy;
                if !state.smooth_initialized {
                    state.smooth_x = x2; state.smooth_y = y2;
                    state.smooth_initialized = true;
                }
                // spacing_px ≈ 1 / rate (pixels per dab), used as time-constant scale
                let spacing_px = if rate > 1e-8 { 1.0 / rate } else { 1.0 };
                let k = if bs.slow_tracking > 0.0 {
                    (-spacing_px / bs.slow_tracking.max(0.1)).exp()
                } else { 0.0 };
                state.smooth_x = state.smooth_x * k + x2 * (1.0 - k);
                state.smooth_y = state.smooth_y * k + y2 * (1.0 - k);
                last_smooth_x = state.smooth_x;
                last_smooth_y = state.smooth_y;

                let (sx, sy) = (state.smooth_x, state.smooth_y);
                let (ex, ey, radius2, opacity2, cr, cg, cb) = apply_dab_effects(
                    state, bs, sx, sy, base_r2, pressure2, stroke.color,
                );

                if matches!(base_blend, RasterBlendMode::Smudge) {
                    let ndx = dx / seg_len;
                    let ndy = dy / seg_len;
                    // strength=1.0 → sample from 1 dab back (drag pixels with us).
                    // strength=0.0 → sample from current position (no change).
                    // smudge_radius_log is repurposed as a linear [0,1] strength value here.
                    let smudge_dist = spacing_px * bs.smudge_radius_log.clamp(0.0, 1.0);
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, cr, cg, cb,
                             ndx, ndy, smudge_dist);
                } else if matches!(base_blend, RasterBlendMode::CloneStamp | RasterBlendMode::Healing) {
                    // Store the offset (not absolute position) in color_r/color_g.
                    // The shader adds this to each pixel's own position for per-pixel sampling.
                    let (ox, oy) = stroke.clone_src_offset.unwrap_or((0.0, 0.0));
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, ox, oy, 0.0,
                             0.0, 0.0, 0.0);
                } else if matches!(base_blend, RasterBlendMode::PatternStamp) {
                    // ndx = pattern_type, ndy = pattern_scale
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, cr, cg, cb,
                             stroke.pattern_type as f32, stroke.pattern_scale, 0.0);
                } else {
                    push_dab(&mut dabs, &mut bbox,
                             ex, ey, radius2, opacity2, cr, cg, cb,
                             0.0, 0.0, 0.0);
                }

                state.partial_dabs = 0.0;
            }
        }

        // Emit any residual time-based dabs (partial_dabs ≥ 1.0 from the dt
        // contribution not consumed by distance-based movement) at the last
        // known cursor position.
        if state.partial_dabs >= 1.0 && !matches!(base_blend, RasterBlendMode::Smudge) {
            // Initialise smooth position if we never entered the segment loop.
            if !state.smooth_initialized {
                if let Some(pt) = stroke.points.last() {
                    state.smooth_x = pt.x;
                    state.smooth_y = pt.y;
                    state.smooth_initialized = true;
                    last_smooth_x = state.smooth_x;
                    last_smooth_y = state.smooth_y;
                }
            }
            while state.partial_dabs >= 1.0 {
                state.partial_dabs -= 1.0;
                let base_r = bs.radius_at_pressure(last_pressure);
                let (ex, ey, r, o, cr, cg, cb) = apply_dab_effects(
                    state, bs,
                    last_smooth_x, last_smooth_y,
                    base_r, last_pressure, stroke.color,
                );
                let (cr2, cg2, cb2, ndx2, ndy2) = if matches!(base_blend, RasterBlendMode::CloneStamp | RasterBlendMode::Healing) {
                    // Store offset in color_r/color_g; shader adds it per-pixel.
                    let (ox, oy) = stroke.clone_src_offset.unwrap_or((0.0, 0.0));
                    (ox, oy, 0.0, 0.0, 0.0)
                } else if matches!(base_blend, RasterBlendMode::PatternStamp) {
                    // ndx = pattern_type, ndy = pattern_scale
                    (cr, cg, cb, stroke.pattern_type as f32, stroke.pattern_scale)
                } else {
                    (cr, cg, cb, 0.0, 0.0)
                };
                push_dab(&mut dabs, &mut bbox, ex, ey, r, o, cr2, cg2, cb2,
                         ndx2, ndy2, 0.0);
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
