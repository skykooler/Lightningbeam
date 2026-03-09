//! Gradient types for vector and raster fills.

use crate::shape::ShapeColor;
use kurbo::Point;
use serde::{Deserialize, Serialize};
use vello::peniko::{self, Brush, Extend, Gradient};

// ── Stop ────────────────────────────────────────────────────────────────────

/// One colour stop in a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Normalised position in [0.0, 1.0].
    pub position: f32,
    pub color: ShapeColor,
}

// ── Kind / Extend ────────────────────────────────────────────────────────────

/// Whether the gradient transitions along a line or radiates from a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GradientType {
    #[default]
    Linear,
    Radial,
}

/// Behaviour outside the gradient's natural [0, 1] range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GradientExtend {
    /// Clamp to edge colour (default).
    #[default]
    Pad,
    /// Mirror the gradient.
    Reflect,
    /// Repeat the gradient.
    Repeat,
}

impl From<GradientExtend> for Extend {
    fn from(e: GradientExtend) -> Self {
        match e {
            GradientExtend::Pad     => Extend::Pad,
            GradientExtend::Reflect => Extend::Reflect,
            GradientExtend::Repeat  => Extend::Repeat,
        }
    }
}

// ── ShapeGradient ────────────────────────────────────────────────────────────

/// A serialisable gradient description.
///
/// Stops are kept sorted by position (ascending).  There are always ≥ 2 stops.
///
/// *Rendering*: call [`to_peniko_brush`](ShapeGradient::to_peniko_brush) with
/// explicit start/end canvas-space points.  For vector faces the caller derives
/// the points from the bounding box + `angle`; for the raster tool the caller
/// uses the drag start/end directly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeGradient {
    pub kind:   GradientType,
    /// Colour stops, sorted by position.
    pub stops:  Vec<GradientStop>,
    /// Angle in degrees for Linear (0 = left→right, 90 = top→bottom).
    /// Ignored for Radial.
    pub angle:  f32,
    pub extend: GradientExtend,
}

impl Default for ShapeGradient {
    fn default() -> Self {
        Self {
            kind:   GradientType::Linear,
            stops:  vec![
                GradientStop { position: 0.0, color: ShapeColor::rgba(0, 0, 0, 255) },
                GradientStop { position: 1.0, color: ShapeColor::rgba(0, 0, 0, 0) },
            ],
            angle:  0.0,
            extend: GradientExtend::Pad,
        }
    }
}

impl ShapeGradient {
    // ── CPU evaluation ───────────────────────────────────────────────────────

    /// Sample RGBA at `t ∈ [0,1]` by linear interpolation between adjacent stops.
    /// Stops must be sorted ascending by position.
    pub fn eval(&self, t: f32) -> [u8; 4] {
        let t = t.clamp(0.0, 1.0);
        if self.stops.is_empty() {
            return [0, 0, 0, 0];
        }
        if self.stops.len() == 1 {
            let c = self.stops[0].color;
            return [c.r, c.g, c.b, c.a];
        }
        // Find first stop with position > t
        let i = self.stops.partition_point(|s| s.position <= t);
        if i == 0 {
            let c = self.stops[0].color;
            return [c.r, c.g, c.b, c.a];
        }
        if i >= self.stops.len() {
            let c = self.stops.last().unwrap().color;
            return [c.r, c.g, c.b, c.a];
        }
        let s0 = self.stops[i - 1];
        let s1 = self.stops[i];
        let span = s1.position - s0.position;
        let f = if span <= 0.0 { 0.0 } else { (t - s0.position) / span };
        fn lerp(a: u8, b: u8, f: f32) -> u8 {
            (a as f32 + (b as f32 - a as f32) * f).round().clamp(0.0, 255.0) as u8
        }
        [
            lerp(s0.color.r, s1.color.r, f),
            lerp(s0.color.g, s1.color.g, f),
            lerp(s0.color.b, s1.color.b, f),
            lerp(s0.color.a, s1.color.a, f),
        ]
    }

    /// Apply `extend` mode to a raw t value, returning t ∈ [0,1].
    pub fn apply_extend(&self, t_raw: f32) -> f32 {
        match self.extend {
            GradientExtend::Pad => t_raw.clamp(0.0, 1.0),
            GradientExtend::Repeat => {
                let t = t_raw.rem_euclid(1.0);
                if t < 0.0 { t + 1.0 } else { t }
            }
            GradientExtend::Reflect => {
                let t = t_raw.rem_euclid(2.0).abs();
                if t > 1.0 { 2.0 - t } else { t }
            }
        }
    }

    // ── GPU / peniko rendering ───────────────────────────────────────────────

    /// Build a `peniko::Brush` from explicit start/end canvas-coordinate points.
    ///
    /// `opacity` in [0,1] is multiplied into all stop alphas.
    pub fn to_peniko_brush(&self, start: Point, end: Point, opacity: f32) -> Brush {
        // Convert stops to peniko tuples.
        let peniko_stops: Vec<(f32, peniko::Color)> = self.stops.iter().map(|s| {
            let a_scaled = (s.color.a as f32 * opacity).round().clamp(0.0, 255.0) as u8;
            let col = peniko::Color::from_rgba8(s.color.r, s.color.g, s.color.b, a_scaled);
            (s.position, col)
        }).collect();

        let extend: Extend = self.extend.into();

        match self.kind {
            GradientType::Linear => {
                Brush::Gradient(
                    Gradient::new_linear(start, end)
                        .with_extend(extend)
                        .with_stops(peniko_stops.as_slice()),
                )
            }
            GradientType::Radial => {
                let mid = Point::new(
                    (start.x + end.x) * 0.5,
                    (start.y + end.y) * 0.5,
                );
                let dx = end.x - start.x;
                let dy = end.y - start.y;
                let radius = ((dx * dx + dy * dy).sqrt() * 0.5) as f32;
                Brush::Gradient(
                    Gradient::new_radial(mid, radius)
                        .with_extend(extend)
                        .with_stops(peniko_stops.as_slice()),
                )
            }
        }
    }
}
