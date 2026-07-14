/// Strongly-typed time units to prevent accidental beats/seconds confusion.
///
/// Convert between the two using `TempoMap::beats_to_seconds` / `TempoMap::seconds_to_beats`.
/// All internal scheduling and clip positions use `Beats`; only audio rendering
/// (sample offsets, file seeks) uses `Seconds`.
use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

/// A time position or duration expressed in **beats** (quarter-note beats).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Beats(pub f64);

/// A time position or duration expressed in **seconds**.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seconds(pub f64);

/// A time *inside a clip's own content*, in whatever unit that clip measures content in.
///
/// Clip content time is domain-polymorphic: SECONDS for sampled audio, video and vector, but BEATS
/// for MIDI (musical, so it survives tempo changes). `ClipInstance::trim_start`/`trim_end` are
/// content times, and storing them as bare `f64`s is what let a seconds delta get added to a MIDI
/// clip's beats trim — splitting a MIDI clip at beat 4 landed at beat 2 at 120 BPM.
///
/// This type is deliberately a **dead end**: it has no `.to_seconds()`, no `.to_beats()`, and no
/// arithmetic with `Seconds` or `Beats`. Content times can be compared and combined with each other
/// (that's domain-safe — both operands are in the same clip's domain), but the only way to get a
/// real timeline duration out is to resolve it against the clip that knows the domain, via
/// `AudioClip::resolve_content_time` / `Document::resolve_content_time`. So a passthrough costs
/// nothing, and mixing domains won't compile.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentTime(pub f64);

impl ContentTime {
    pub const ZERO: Self = Self(0.0);

    pub fn max(self, other: Self) -> Self { Self(self.0.max(other.0)) }
    pub fn min(self, other: Self) -> Self { Self(self.0.min(other.0)) }

    /// The raw magnitude, with the domain discarded.
    ///
    /// Only for code that is *already* working in this clip's content domain (trim arithmetic,
    /// serialization, drawing a waveform whose x-axis is the clip's own content). If you are about
    /// to combine this with a timeline position, resolve it against the clip instead.
    pub fn raw(self) -> f64 { self.0 }
}

impl Add for ContentTime {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}
impl Sub for ContentTime {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
}
impl Rem for ContentTime {
    type Output = Self;
    fn rem(self, rhs: Self) -> Self { Self(self.0 % rhs.0) }
}

impl Beats {
    pub const ZERO: Self = Self(0.0);

    pub fn max(self, other: Self) -> Self { Self(self.0.max(other.0)) }
    pub fn min(self, other: Self) -> Self { Self(self.0.min(other.0)) }
    pub fn abs(self) -> Self { Self(self.0.abs()) }
    pub fn ceil(self) -> Self { Self(self.0.ceil()) }
    pub fn floor(self) -> Self { Self(self.0.floor()) }
    pub fn beats_to_f64(self) -> f64 { self.0 }
}

impl Seconds {
    pub const ZERO: Self = Self(0.0);

    pub fn max(self, other: Self) -> Self { Self(self.0.max(other.0)) }
    pub fn min(self, other: Self) -> Self { Self(self.0.min(other.0)) }
    pub fn abs(self) -> Self { Self(self.0.abs()) }
    pub fn seconds_to_f64(self) -> f64 { self.0 }
}

// --- Beats arithmetic ---

impl Add for Beats {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}
impl Sub for Beats {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
}
impl Mul<f64> for Beats {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self { Self(self.0 * rhs) }
}
impl Div<f64> for Beats {
    type Output = Self;
    fn div(self, rhs: f64) -> Self { Self(self.0 / rhs) }
}
/// Beats / Beats = dimensionless ratio (f64)
impl Div<Beats> for Beats {
    type Output = f64;
    fn div(self, rhs: Beats) -> f64 { self.0 / rhs.0 }
}
impl Rem for Beats {
    type Output = Self;
    fn rem(self, rhs: Self) -> Self { Self(self.0 % rhs.0) }
}
impl Neg for Beats {
    type Output = Self;
    fn neg(self) -> Self { Self(-self.0) }
}
impl AddAssign for Beats {
    fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
}
impl SubAssign for Beats {
    fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; }
}

// --- Seconds arithmetic ---

impl Add for Seconds {
    type Output = Self;
    fn add(self, rhs: Self) -> Self { Self(self.0 + rhs.0) }
}
impl Sub for Seconds {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self { Self(self.0 - rhs.0) }
}
impl Mul<f64> for Seconds {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self { Self(self.0 * rhs) }
}
impl Div<f64> for Seconds {
    type Output = Self;
    fn div(self, rhs: f64) -> Self { Self(self.0 / rhs) }
}
/// Seconds / Seconds = dimensionless ratio (f64)
impl Div<Seconds> for Seconds {
    type Output = f64;
    fn div(self, rhs: Seconds) -> f64 { self.0 / rhs.0 }
}
impl Rem for Seconds {
    type Output = Self;
    fn rem(self, rhs: Self) -> Self { Self(self.0 % rhs.0) }
}
impl Neg for Seconds {
    type Output = Self;
    fn neg(self) -> Self { Self(-self.0) }
}
impl AddAssign for Seconds {
    fn add_assign(&mut self, rhs: Self) { self.0 += rhs.0; }
}
impl SubAssign for Seconds {
    fn sub_assign(&mut self, rhs: Self) { self.0 -= rhs.0; }
}

impl std::fmt::Display for Beats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for Seconds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
