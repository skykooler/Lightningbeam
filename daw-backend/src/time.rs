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
