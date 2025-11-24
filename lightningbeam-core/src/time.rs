/// A strongly-typed representation of a timestamp (seconds).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
pub struct Timestamp(f64);

/// A strongly-typed representation of a duration (seconds).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
pub struct Duration(f64);

/// A strongly-typed representation of a number of samples.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize)]
pub struct SampleCount(usize);

impl Timestamp {
    /// Create a new timestamp in seconds.
    pub fn new(seconds: f64) -> Self {
        Timestamp(seconds)
    }

    /// Create a new timestamp from seconds. (dummy method)
    pub fn from_seconds(seconds: f64) -> Self {
        Timestamp(seconds)
    }

    /// Create a new timestamp from milliseconds.
    pub fn from_millis(milliseconds: u64) -> Self {
        Timestamp(milliseconds as f64 / 1000.0)
    }

    /// Get the value in seconds.
    pub fn as_seconds(&self) -> f64 {
        self.0
    }

    /// Get the value in milliseconds.
    pub fn as_millis(&self) -> u64 {
        (self.0 * 1000.0).round() as u64
    }

    /// Add a duration to a timestamp, producing a new timestamp.
    pub fn add_duration(&self, duration: Duration) -> Timestamp {
        Timestamp(self.0 + duration.0)
    }

    /// Subtract a duration from a timestamp, producing a new timestamp.
    pub fn subtract_duration(&self, duration: Duration) -> Timestamp {
        Timestamp(self.0 - duration.0)
    }

    /// Subtract another timestamp, producing a duration.
    pub fn subtract_timestamp(&self, other: Timestamp) -> Duration {
        Duration(self.0 - other.0)
    }

    pub fn set(&mut self, other: Timestamp) {
        self.0 = other.as_seconds();
    }

    pub fn max(&self, other: Timestamp) -> Timestamp {
        Timestamp(self.0.max(other.0))
    }

    pub fn min(&self, other: Timestamp) -> Timestamp {
        Timestamp(self.0.min(other.0))
    }
}

impl Duration {
    /// Create a new duration in seconds.
    pub fn new(seconds: f64) -> Self {
        Duration(seconds)
    }

    /// Create a new duration from seconds. (dummy method)
    pub fn from_seconds(seconds: f64) -> Self {
        Duration(seconds)
    }

    /// Create a new duration from milliseconds.
    pub fn from_millis(milliseconds: u64) -> Self {
        Duration(milliseconds as f64 / 1000.0)
    }

    /// Create a new duration from frames, given a frame rate.
    pub fn from_frames(frames: u64, frame_rate: f64) -> Self {
        Duration(frames as f64 / frame_rate)
    }

    /// Get the value in seconds.
    pub fn as_seconds(&self) -> f64 {
        self.0
    }

    /// Get the value in milliseconds.
    pub fn as_millis(&self) -> u64 {
        (self.0 * 1000.0).round() as u64
    }

    /// Get the number of frames for this duration, given a frame rate.
    pub fn to_frames(&self, frame_rate: f64) -> u64 {
        (self.0 * frame_rate).round() as u64
    }

    /// Get the number of samples in this duration at a given sample rate
    pub fn to_samples(&self, sample_rate: u32) -> u64 {
        (self.0 * sample_rate as f64).round() as u64
    }

    pub fn to_std(&self) -> std::time::Duration {
        std::time::Duration::from_nanos((self.0/1_000_000_000.0).round() as u64)
    }

    /// Add two durations together.
    pub fn add(&self, other: Duration) -> Duration {
        Duration(self.0 + other.0)
    }

    /// Subtract one duration from another.
    pub fn subtract(&self, other: Duration) -> Duration {
        Duration(self.0 - other.0)
    }
}


impl SampleCount {
    /// Create a new count of samples.
    pub fn new(samples: usize) -> Self {
        SampleCount(samples)
    }

    pub fn as_usize(&self) -> usize {
        self.0
    }

    pub fn to_duration(&self, sample_rate: u32) -> Duration {
        Duration((self.0 as f64) / (sample_rate as f64))
    }

    pub fn max(&self, other: SampleCount) -> SampleCount {
        SampleCount(self.0.max(other.0))
    }

    pub fn min(&self, other: SampleCount) -> SampleCount {
        SampleCount(self.0.min(other.0))
    }
}

impl PartialEq<usize> for SampleCount{
    fn eq(&self, other: &usize) -> bool {
        self.0 == *other
    }
}

// Overloading operators for more natural usage
use std::ops::{Add, Sub, AddAssign, SubAssign};

impl Add<Duration> for Timestamp {
    type Output = Timestamp;

    fn add(self, duration: Duration) -> Timestamp {
        self.add_duration(duration)
    }
}

impl Sub<Duration> for Timestamp {
    type Output = Timestamp;

    fn sub(self, duration: Duration) -> Timestamp {
        self.subtract_duration(duration)
    }
}

impl Sub<Timestamp> for Timestamp {
    type Output = Duration;

    fn sub(self, other: Timestamp) -> Duration {
        self.subtract_timestamp(other)
    }
}

impl AddAssign<Duration> for Timestamp {
    fn add_assign(&mut self, duration: Duration) {
        self.0 += duration.0;
    }
}

impl SubAssign<Duration> for Timestamp {
    fn sub_assign(&mut self, duration: Duration) {
        self.0 -= duration.0;
    }
}


impl Add for SampleCount {
    type Output = SampleCount;
    fn add(self, other: SampleCount) -> SampleCount {
        SampleCount(self.0 + other.0)
    }
}

impl Sub for SampleCount {
    type Output = SampleCount;
    fn sub(self, other: SampleCount) -> SampleCount {
        SampleCount(self.0 - other.0)
    }
}

impl AddAssign<SampleCount> for SampleCount {
    fn add_assign(&mut self, other: SampleCount) {
        self.0 += other.0;
    }
}

impl SubAssign<SampleCount> for SampleCount {
    fn sub_assign(&mut self, other: SampleCount) {
        self.0 -= other.0;
    }
}

/// Represents a video frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA pixel data
}

impl Frame {
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Frame { width, height, data }
    }
}
