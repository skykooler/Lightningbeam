//! Brush settings for the raster paint engine
//!
//! Settings that describe the appearance and behavior of a paint brush.
//! Compatible with MyPaint .myb brush file format.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Settings for a paint brush — mirrors the MyPaint .myb settings schema.
///
/// All fields correspond directly to MyPaint JSON keys.  Fields marked
/// "parse-only" are stored so that .myb files round-trip cleanly; they will
/// be used when the dynamic-input system is wired up in a future task.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrushSettings {
    // ── Core shape ──────────────────────────────────────────────────────────
    /// log(radius) base value; actual radius = exp(radius_log)
    pub radius_log: f32,
    /// Edge hardness 0.0 (fully soft/gaussian) to 1.0 (hard edge)
    pub hardness: f32,
    /// Base opacity 0.0–1.0
    pub opaque: f32,
    /// Additional opacity multiplier (opaque_multiply)
    pub opaque_multiply: f32,
    /// Dabs per basic_radius distance (MyPaint: dabs_per_basic_radius)
    pub dabs_per_radius: f32,
    /// Dabs per actual (pressure-modified) radius distance
    pub dabs_per_actual_radius: f32,

    // ── Elliptical dab ──────────────────────────────────────────────────────
    /// Dab aspect ratio ≥ 1.0 (1.0 = circle, 3.0 = 3:1 ellipse)
    pub elliptical_dab_ratio: f32,
    /// Elliptical dab rotation angle in degrees (0–180)
    pub elliptical_dab_angle: f32,

    // ── Jitter / offset ─────────────────────────────────────────────────────
    /// Random radius variation (log-scale, 0 = none)
    pub radius_by_random: f32,
    /// Random positional jitter in units of radius
    pub offset_by_random: f32,
    /// Fixed X offset in units of radius
    pub offset_x: f32,
    /// Fixed Y offset in units of radius
    pub offset_y: f32,

    // ── Position tracking ───────────────────────────────────────────────────
    /// Slow position tracking — higher = brush lags behind cursor more
    pub slow_tracking: f32,
    /// Per-dab position tracking smoothing
    pub slow_tracking_per_dab: f32,

    // ── Color ───────────────────────────────────────────────────────────────
    /// HSV hue (0.0–1.0); usually overridden by stroke color
    pub color_h: f32,
    /// HSV saturation (0.0–1.0)
    pub color_s: f32,
    /// HSV value (0.0–1.0)
    pub color_v: f32,
    /// Per-dab hue shift (accumulates over the stroke)
    pub change_color_h: f32,
    /// Per-dab HSV value shift
    pub change_color_v: f32,
    /// Per-dab HSV saturation shift
    pub change_color_hsv_s: f32,
    /// Per-dab HSL lightness shift
    pub change_color_l: f32,
    /// Per-dab HSL saturation shift
    pub change_color_hsl_s: f32,

    // ── Blend ───────────────────────────────────────────────────────────────
    /// Lock alpha channel (0 = off, 1 = on — don't modify destination alpha)
    pub lock_alpha: f32,
    /// Eraser strength (>0.5 activates erase blend when tool mode is Normal)
    pub eraser: f32,

    // ── Smudge ──────────────────────────────────────────────────────────────
    /// Smudge amount (>0.5 activates smudge blend when tool mode is Normal)
    pub smudge: f32,
    /// How quickly the smudge color updates (0 = instant, 1 = slow)
    pub smudge_length: f32,
    /// Smudge pickup radius offset (log-scale added to radius_log)
    pub smudge_radius_log: f32,

    // ── Stroke gating ───────────────────────────────────────────────────────
    /// Minimum pressure required to emit dabs (0 = always emit)
    pub stroke_threshold: f32,

    // ── Pressure dynamics ───────────────────────────────────────────────────
    /// How much pressure increases/decreases radius
    pub pressure_radius_gain: f32,
    /// How much pressure increases/decreases opacity
    pub pressure_opacity_gain: f32,

    // ── Parse-only: future input curve system ───────────────────────────────
    pub opaque_linearize: f32,
    pub anti_aliasing: f32,
    pub dabs_per_second: f32,
    pub offset_by_speed: f32,
    pub offset_by_speed_slowness: f32,
    pub speed1_slowness: f32,
    pub speed2_slowness: f32,
    pub speed1_gamma: f32,
    pub speed2_gamma: f32,
    pub direction_filter: f32,
    pub stroke_duration_log: f32,
    pub stroke_holdtime: f32,
    pub pressure_gain_log: f32,
    pub smudge_transparency: f32,
    pub smudge_length_log: f32,
    pub smudge_bucket: f32,
    pub paint_mode: f32,
    pub colorize: f32,
    pub posterize: f32,
    pub posterize_num: f32,
    pub snap_to_pixel: f32,
    pub custom_input: f32,
    pub custom_input_slowness: f32,
    pub gridmap_scale: f32,
    pub gridmap_scale_x: f32,
    pub gridmap_scale_y: f32,
    pub restore_color: f32,
    pub offset_angle: f32,
    pub offset_angle_asc: f32,
    pub offset_angle_view: f32,
    pub offset_angle_2: f32,
    pub offset_angle_2_asc: f32,
    pub offset_angle_2_view: f32,
    pub offset_angle_adj: f32,
    pub offset_multiplier: f32,
}

impl BrushSettings {
    /// Default soft round brush (smooth Gaussian falloff)
    pub fn default_round_soft() -> Self {
        Self {
            radius_log: 2.0,
            hardness: 0.1,
            opaque: 0.8,
            opaque_multiply: 0.0,
            dabs_per_radius: 2.0,
            dabs_per_actual_radius: 2.0,
            elliptical_dab_ratio: 1.0,
            elliptical_dab_angle: 90.0,
            radius_by_random: 0.0,
            offset_by_random: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            slow_tracking: 0.0,
            slow_tracking_per_dab: 0.0,
            color_h: 0.0,
            color_s: 0.0,
            color_v: 0.0,
            change_color_h: 0.0,
            change_color_v: 0.0,
            change_color_hsv_s: 0.0,
            change_color_l: 0.0,
            change_color_hsl_s: 0.0,
            lock_alpha: 0.0,
            eraser: 0.0,
            smudge: 0.0,
            smudge_length: 0.5,
            smudge_radius_log: 0.0,
            stroke_threshold: 0.0,
            pressure_radius_gain: 0.5,
            pressure_opacity_gain: 1.0,
            opaque_linearize: 0.9,
            anti_aliasing: 1.0,
            dabs_per_second: 0.0,
            offset_by_speed: 0.0,
            offset_by_speed_slowness: 1.0,
            speed1_slowness: 0.04,
            speed2_slowness: 0.8,
            speed1_gamma: 4.0,
            speed2_gamma: 4.0,
            direction_filter: 2.0,
            stroke_duration_log: 4.0,
            stroke_holdtime: 0.0,
            pressure_gain_log: 0.0,
            smudge_transparency: 0.0,
            smudge_length_log: 0.0,
            smudge_bucket: 0.0,
            paint_mode: 1.0,
            colorize: 0.0,
            posterize: 0.0,
            posterize_num: 0.05,
            snap_to_pixel: 0.0,
            custom_input: 0.0,
            custom_input_slowness: 0.0,
            gridmap_scale: 0.0,
            gridmap_scale_x: 1.0,
            gridmap_scale_y: 1.0,
            restore_color: 0.0,
            offset_angle: 0.0,
            offset_angle_asc: 0.0,
            offset_angle_view: 0.0,
            offset_angle_2: 0.0,
            offset_angle_2_asc: 0.0,
            offset_angle_2_view: 0.0,
            offset_angle_adj: 0.0,
            offset_multiplier: 0.0,
        }
    }

    /// Default hard round brush (sharp edge)
    pub fn default_round_hard() -> Self {
        Self {
            hardness: 0.9,
            opaque: 1.0,
            dabs_per_radius: 2.0,
            pressure_radius_gain: 0.3,
            pressure_opacity_gain: 0.8,
            ..Self::default_round_soft()
        }
    }

    /// Compute actual radius at a given pressure level
    pub fn radius_at_pressure(&self, pressure: f32) -> f32 {
        let r = self.radius_log + self.pressure_radius_gain * (pressure - 0.5);
        r.exp().clamp(0.5, 500.0)
    }

    /// Compute actual opacity at a given pressure level
    pub fn opacity_at_pressure(&self, pressure: f32) -> f32 {
        let o = self.opaque * (1.0 + self.pressure_opacity_gain * (pressure - 0.5));
        o.clamp(0.0, 1.0)
    }

    /// Parse a MyPaint .myb JSON brush file.
    ///
    /// Reads all known settings from `settings[key].base_value`.
    /// Unknown keys are silently ignored for forward compatibility.
    pub fn from_myb(json: &str) -> Result<Self, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("JSON parse error: {e}"))?;

        let settings = v.get("settings").ok_or("Missing 'settings' key")?;

        let read_base = |name: &str, default: f32| -> f32 {
            settings
                .get(name)
                .and_then(|s| s.get("base_value"))
                .and_then(|bv| bv.as_f64())
                .map(|f| f as f32)
                .unwrap_or(default)
        };

        // Pressure dynamics: approximate from the pressure input curve endpoints
        let pressure_radius_gain = settings
            .get("radius_logarithmic")
            .and_then(|s| s.get("inputs"))
            .and_then(|inp| inp.get("pressure"))
            .and_then(|pts| pts.as_array())
            .and_then(|arr| {
                if arr.len() >= 2 {
                    let y0 = arr[0].get(1)?.as_f64()? as f32;
                    let y1 = arr[arr.len() - 1].get(1)?.as_f64()? as f32;
                    Some((y1 - y0) * 0.5)
                } else {
                    None
                }
            })
            .unwrap_or(0.5);

        let pressure_opacity_gain = settings
            .get("opaque")
            .and_then(|s| s.get("inputs"))
            .and_then(|inp| inp.get("pressure"))
            .and_then(|pts| pts.as_array())
            .and_then(|arr| {
                if arr.len() >= 2 {
                    let y0 = arr[0].get(1)?.as_f64()? as f32;
                    let y1 = arr[arr.len() - 1].get(1)?.as_f64()? as f32;
                    Some(y1 - y0)
                } else {
                    None
                }
            })
            .unwrap_or(1.0);

        Ok(Self {
            // Core shape
            radius_log:              read_base("radius_logarithmic",     2.0),
            hardness:                read_base("hardness",               0.8).clamp(0.0, 1.0),
            opaque:                  read_base("opaque",                 1.0).clamp(0.0, 2.0),
            opaque_multiply:         read_base("opaque_multiply",        0.0),
            dabs_per_radius:         read_base("dabs_per_basic_radius",  0.0).max(0.0),
            dabs_per_actual_radius:  read_base("dabs_per_actual_radius", 2.0).max(0.0),
            // Elliptical dab
            elliptical_dab_ratio:    read_base("elliptical_dab_ratio",   1.0).max(1.0),
            elliptical_dab_angle:    read_base("elliptical_dab_angle",  90.0),
            // Jitter / offset
            radius_by_random:        read_base("radius_by_random",       0.0),
            offset_by_random:        read_base("offset_by_random",       0.0),
            offset_x:                read_base("offset_x",               0.0),
            offset_y:                read_base("offset_y",               0.0),
            // Tracking
            slow_tracking:           read_base("slow_tracking",          0.0),
            slow_tracking_per_dab:   read_base("slow_tracking_per_dab", 0.0),
            // Color
            color_h:                 read_base("color_h",                0.0),
            color_s:                 read_base("color_s",                0.0),
            color_v:                 read_base("color_v",                0.0),
            change_color_h:          read_base("change_color_h",         0.0),
            change_color_v:          read_base("change_color_v",         0.0),
            change_color_hsv_s:      read_base("change_color_hsv_s",     0.0),
            change_color_l:          read_base("change_color_l",         0.0),
            change_color_hsl_s:      read_base("change_color_hsl_s",     0.0),
            // Blend
            lock_alpha:              read_base("lock_alpha",             0.0).clamp(0.0, 1.0),
            eraser:                  read_base("eraser",                 0.0).clamp(0.0, 1.0),
            // Smudge
            smudge:                  read_base("smudge",                 0.0).clamp(0.0, 1.0),
            smudge_length:           read_base("smudge_length",          0.5).clamp(0.0, 1.0),
            smudge_radius_log:       read_base("smudge_radius_log",      0.0),
            // Stroke gating
            stroke_threshold:        read_base("stroke_threshold",       0.0).clamp(0.0, 0.5),
            // Pressure dynamics
            pressure_radius_gain,
            pressure_opacity_gain,
            // Parse-only
            opaque_linearize:        read_base("opaque_linearize",       0.9),
            anti_aliasing:           read_base("anti_aliasing",          1.0),
            dabs_per_second:         read_base("dabs_per_second",        0.0),
            offset_by_speed:         read_base("offset_by_speed",        0.0),
            offset_by_speed_slowness: read_base("offset_by_speed_slowness", 1.0),
            speed1_slowness:         read_base("speed1_slowness",        0.04),
            speed2_slowness:         read_base("speed2_slowness",        0.8),
            speed1_gamma:            read_base("speed1_gamma",           4.0),
            speed2_gamma:            read_base("speed2_gamma",           4.0),
            direction_filter:        read_base("direction_filter",       2.0),
            stroke_duration_log:     read_base("stroke_duration_logarithmic", 4.0),
            stroke_holdtime:         read_base("stroke_holdtime",        0.0),
            pressure_gain_log:       read_base("pressure_gain_log",      0.0),
            smudge_transparency:     read_base("smudge_transparency",    0.0),
            smudge_length_log:       read_base("smudge_length_log",      0.0),
            smudge_bucket:           read_base("smudge_bucket",          0.0),
            paint_mode:              read_base("paint_mode",             1.0),
            colorize:                read_base("colorize",               0.0),
            posterize:               read_base("posterize",              0.0),
            posterize_num:           read_base("posterize_num",          0.05),
            snap_to_pixel:           read_base("snap_to_pixel",          0.0),
            custom_input:            read_base("custom_input",           0.0),
            custom_input_slowness:   read_base("custom_input_slowness",  0.0),
            gridmap_scale:           read_base("gridmap_scale",          0.0),
            gridmap_scale_x:         read_base("gridmap_scale_x",        1.0),
            gridmap_scale_y:         read_base("gridmap_scale_y",        1.0),
            restore_color:           read_base("restore_color",          0.0),
            offset_angle:            read_base("offset_angle",           0.0),
            offset_angle_asc:        read_base("offset_angle_asc",       0.0),
            offset_angle_view:       read_base("offset_angle_view",      0.0),
            offset_angle_2:          read_base("offset_angle_2",         0.0),
            offset_angle_2_asc:      read_base("offset_angle_2_asc",     0.0),
            offset_angle_2_view:     read_base("offset_angle_2_view",    0.0),
            offset_angle_adj:        read_base("offset_angle_adj",       0.0),
            offset_multiplier:       read_base("offset_multiplier",      0.0),
        })
    }
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self::default_round_soft()
    }
}

// ---------------------------------------------------------------------------
// Bundled brush presets
// ---------------------------------------------------------------------------

/// A named brush preset backed by a bundled .myb file.
pub struct BrushPreset {
    pub name: &'static str,
    pub settings: BrushSettings,
}

/// Returns the list of bundled brush presets (parsed once from embedded .myb files).
///
/// Sources: mypaint/mypaint-brushes — CC0 1.0 Universal (Public Domain)
pub fn bundled_brushes() -> &'static [BrushPreset] {
    static CACHE: OnceLock<Vec<BrushPreset>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let mut v = Vec::new();
        macro_rules! brush {
            ($name:literal, $path:literal) => {
                if let Ok(s) = BrushSettings::from_myb(include_str!($path)) {
                    v.push(BrushPreset { name: $name, settings: s });
                }
            };
        }
        brush!("Pencil",      "../../../src/assets/brushes/pencil.myb");
        brush!("Pen",         "../../../src/assets/brushes/pen.myb");
        brush!("Charcoal",    "../../../src/assets/brushes/charcoal.myb");
        brush!("Brush",       "../../../src/assets/brushes/brush.myb");
        brush!("Dry Brush",   "../../../src/assets/brushes/dry_brush.myb");
        brush!("Ink",         "../../../src/assets/brushes/ink_blot.myb");
        brush!("Calligraphy", "../../../src/assets/brushes/calligraphy.myb");
        brush!("Airbrush",    "../../../src/assets/brushes/airbrush.myb");
        brush!("Chalk",       "../../../src/assets/brushes/chalk.myb");
        brush!("Liner",       "../../../src/assets/brushes/liner.myb");
        v
    })
}
