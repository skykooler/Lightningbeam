//! Brush settings for the raster paint engine
//!
//! Settings that describe the appearance and behavior of a paint brush.
//! Compatible with MyPaint .myb brush file format (subset).

use serde::{Deserialize, Serialize};

/// Settings for a paint brush
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrushSettings {
    /// log(radius) base value; actual radius = exp(radius_log)
    pub radius_log: f32,
    /// Edge hardness 0.0 (fully soft/gaussian) to 1.0 (hard edge)
    pub hardness: f32,
    /// Base opacity 0.0–1.0
    pub opaque: f32,
    /// Dab spacing as fraction of radius (smaller = denser strokes)
    pub dabs_per_radius: f32,
    /// HSV hue (0.0–1.0); usually overridden by stroke color
    pub color_h: f32,
    /// HSV saturation (0.0–1.0)
    pub color_s: f32,
    /// HSV value (0.0–1.0)
    pub color_v: f32,
    /// How much pressure increases/decreases radius
    /// Final radius = exp(radius_log + pressure_radius_gain * pressure)
    pub pressure_radius_gain: f32,
    /// How much pressure increases/decreases opacity
    /// Final opacity = opaque * (1 + pressure_opacity_gain * (pressure - 0.5))
    pub pressure_opacity_gain: f32,
}

impl BrushSettings {
    /// Default soft round brush (smooth Gaussian falloff)
    pub fn default_round_soft() -> Self {
        Self {
            radius_log: 2.0,     // radius ≈ 7.4 px
            hardness: 0.1,
            opaque: 0.8,
            dabs_per_radius: 0.25,
            color_h: 0.0,
            color_s: 0.0,
            color_v: 0.0,
            pressure_radius_gain: 0.5,
            pressure_opacity_gain: 1.0,
        }
    }

    /// Default hard round brush (sharp edge)
    pub fn default_round_hard() -> Self {
        Self {
            radius_log: 2.0,
            hardness: 0.9,
            opaque: 1.0,
            dabs_per_radius: 0.2,
            color_h: 0.0,
            color_s: 0.0,
            color_v: 0.0,
            pressure_radius_gain: 0.3,
            pressure_opacity_gain: 0.8,
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

    /// Parse a MyPaint .myb JSON brush file (subset).
    ///
    /// Reads `radius_logarithmic`, `hardness`, `opaque`, `dabs_per_basic_radius`,
    /// `color_h`, `color_s`, `color_v` from the `settings` key's `base_value` fields.
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

        // Pressure dynamics: read from the "inputs" mapping of radius/opacity
        // For simplicity, look for the pressure input point in radius_logarithmic
        let pressure_radius_gain = settings
            .get("radius_logarithmic")
            .and_then(|s| s.get("inputs"))
            .and_then(|inp| inp.get("pressure"))
            .and_then(|pts| pts.as_array())
            .and_then(|arr| {
                // arr = [[x0,y0],[x1,y1],...] – approximate as linear gain at x=1.0
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
            radius_log: read_base("radius_logarithmic", 2.0),
            hardness: read_base("hardness", 0.5).clamp(0.0, 1.0),
            opaque: read_base("opaque", 1.0).clamp(0.0, 1.0),
            dabs_per_radius: read_base("dabs_per_basic_radius", 0.25).clamp(0.01, 10.0),
            color_h: read_base("color_h", 0.0),
            color_s: read_base("color_s", 0.0),
            color_v: read_base("color_v", 0.0),
            pressure_radius_gain,
            pressure_opacity_gain,
        })
    }
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self::default_round_soft()
    }
}
