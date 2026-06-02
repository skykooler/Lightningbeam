//! TempoMap — beats ↔ seconds conversion with variable tempo support.
//!
//! Positions are stored in **beats** throughout the project; `TempoMap` converts
//! between beats and seconds at render / scheduling time.
//!
//! # Interpolation
//! Each `TempoEntry` has an `interpolation` field that controls how the BPM
//! changes between that entry and the next:
//! - `Step`: BPM is constant from this entry's beat until the next entry. Instant change.
//! - `Linear`: BPM linearly interpolates from this entry's BPM to the next entry's BPM
//!   over the beat range.  The seconds calculation uses the exact integral:
//!   `Δt = (60 / slope) * ln(bpm_end / bpm_start)` where slope = (bpm_end - bpm_start) / span_beats.
//!
//! # Format
//! `entries` is a sorted `Vec<TempoEntry>` where the first entry must always
//! have `beat == 0.0`.
//!
//! # Sequential-access optimisation
//! An `AtomicUsize` caches the index of the last segment visited by
//! `beats_to_seconds`.  When calls are in ascending order (the common case when
//! walking events in order) the scan starts from the cached index instead of
//! the beginning, giving amortised O(1) behaviour.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::time::{Beats, Seconds};

/// How the BPM transitions from one `TempoEntry` to the next.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub enum TempoInterpolation {
    /// BPM stays constant from this entry's beat until the next entry (instant change).
    #[default]
    Step,
    /// BPM linearly interpolates from this entry's BPM to the next entry's BPM
    /// over the beat span between the two entries.
    Linear,
}

/// A single tempo segment: from `beat` onwards the tempo changes according to `interpolation`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoEntry {
    /// Start of this tempo segment in beats (quarter-note beats).
    pub beat: f64,
    /// Tempo at the start of this segment in beats per minute.
    pub bpm: f64,
    /// Cumulative seconds elapsed at the start of this segment.
    /// **Derived** — not serialised; call [`TempoMap::rebuild_seconds`] after any
    /// mutation or after deserialization.
    #[serde(skip, default)]
    pub seconds: f64,
    /// How the BPM transitions from this entry to the next.
    #[serde(default)]
    pub interpolation: TempoInterpolation,
}

/// A piecewise tempo map used to convert between beats and seconds.
#[derive(Debug, Serialize, Deserialize)]
pub struct TempoMap {
    /// Sorted list of tempo segments.  Must always have at least one entry at beat 0.
    pub entries: Vec<TempoEntry>,
    /// Sequential-access cache: index of the last segment used by `beats_to_seconds`.
    #[serde(skip, default)]
    last_index: AtomicUsize,
}

impl Clone for TempoMap {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            last_index: AtomicUsize::new(self.last_index.load(Ordering::Relaxed)),
        }
    }
}

impl Default for TempoMap {
    fn default() -> Self {
        Self::constant(120.0)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Seconds elapsed traversing `span_beats` starting at `bpm_start`.
/// If `bpm_end` is `Some` (linear segment) and differs from `bpm_start`,
/// uses the exact logarithmic integral.
#[inline]
fn segment_duration(span_beats: f64, bpm_start: f64, bpm_end: Option<f64>) -> f64 {
    match bpm_end {
        None => span_beats * 60.0 / bpm_start,
        Some(b1) if (b1 - bpm_start).abs() < 1e-9 => span_beats * 60.0 / bpm_start,
        Some(b1) => {
            // Linear BPM: BPM(b) = bpm_start + slope * (b - b_start)
            // Δt = ∫₀^span 60 / BPM(b) db = (60/slope) * ln(b1/bpm_start)
            let slope = (b1 - bpm_start) / span_beats;
            (60.0 / slope) * (b1 / bpm_start).ln()
        }
    }
}

/// Beats elapsed given `delta_seconds` starting at `bpm_start`.
/// If `bpm_end` is `Some` (linear segment) and differs from `bpm_start`,
/// uses the exact exponential inverse.
#[inline]
fn segment_beats(delta_seconds: f64, span_beats: f64, bpm_start: f64, bpm_end: Option<f64>) -> f64 {
    match bpm_end {
        None => delta_seconds * bpm_start / 60.0,
        Some(b1) if (b1 - bpm_start).abs() < 1e-9 => delta_seconds * bpm_start / 60.0,
        Some(b1) => {
            // Inverse of the logarithmic integral:
            // b = b_start + (bpm_start / slope) * (exp(delta_t * slope / 60) - 1)
            let slope = (b1 - bpm_start) / span_beats;
            (bpm_start / slope) * ((delta_seconds * slope / 60.0).exp() - 1.0)
        }
    }
}

impl TempoMap {
    /// Create a constant-tempo map.
    pub fn constant(bpm: f64) -> Self {
        Self {
            entries: vec![TempoEntry { beat: 0.0, bpm, seconds: 0.0, interpolation: TempoInterpolation::Step }],
            last_index: AtomicUsize::new(0),
        }
    }

    /// Rebuild the `seconds` field on every entry from scratch.
    /// **Must be called** after any mutation (add/remove/reorder entry) and
    /// after deserialization.
    pub fn rebuild_seconds(&mut self) {
        let mut cumulative = 0.0_f64;
        let n = self.entries.len();
        for i in 0..n {
            self.entries[i].seconds = cumulative;
            if i + 1 < n {
                let span = self.entries[i + 1].beat - self.entries[i].beat;
                let bpm_end = if self.entries[i].interpolation == TempoInterpolation::Linear {
                    Some(self.entries[i + 1].bpm)
                } else {
                    None
                };
                cumulative += segment_duration(span, self.entries[i].bpm, bpm_end);
            }
        }
        self.last_index.store(0, Ordering::Relaxed);
    }

    /// Return the instantaneous BPM active at `beat`.
    /// For linear segments, returns the interpolated value at that beat.
    pub fn bpm_at(&self, beat: Beats) -> f64 {
        let n = self.entries.len();
        let idx = self.entries.partition_point(|e| e.beat <= beat.0).saturating_sub(1);
        let idx = idx.min(n - 1);
        let entry = &self.entries[idx];
        if entry.interpolation == TempoInterpolation::Linear && idx + 1 < n {
            let next = &self.entries[idx + 1];
            let t = (beat.0 - entry.beat) / (next.beat - entry.beat);
            entry.bpm + (next.bpm - entry.bpm) * t
        } else {
            entry.bpm
        }
    }

    /// Convert beats to seconds using the tempo map.
    ///
    /// Uses the sequential cache: if `beat` is at or after the last cached
    /// segment, the scan starts there instead of from the beginning.
    pub fn beats_to_seconds(&self, beat: Beats) -> Seconds {
        Seconds(self.transform(beat.0))
    }

    /// Convert seconds to beats using binary search on the cached `seconds` offsets.
    pub fn seconds_to_beats(&self, seconds: Seconds) -> Beats {
        Beats(self.inverse_transform(seconds.0))
    }

    /// Global BPM — the BPM of the first entry (at beat 0).
    pub fn global_bpm(&self) -> f64 {
        self.entries[0].bpm
    }

    /// Set the global BPM (first entry).  Rebuilds seconds.
    pub fn set_global_bpm(&mut self, bpm: f64) {
        self.entries[0].bpm = bpm;
        self.rebuild_seconds();
    }

    /// Convert local beats to parent time units (raw `f64`).
    ///
    /// At the root level the result is absolute seconds.  Inside a nested
    /// group the result is the *parent group's* beats.
    pub fn transform(&self, beat: f64) -> f64 {
        if beat <= 0.0 {
            return 0.0;
        }
        let n = self.entries.len();
        let cached = self.last_index.load(Ordering::Relaxed).min(n.saturating_sub(1));
        let start = if beat >= self.entries[cached].beat { cached } else { 0 };
        let mut idx = start;
        while idx + 1 < n && self.entries[idx + 1].beat <= beat {
            idx += 1;
        }
        self.last_index.store(idx, Ordering::Relaxed);

        let entry = &self.entries[idx];
        let beat_in_seg = beat - entry.beat;
        if entry.interpolation == TempoInterpolation::Linear && idx + 1 < n {
            let next = &self.entries[idx + 1];
            let span = next.beat - entry.beat;
            entry.seconds + segment_duration(beat_in_seg, entry.bpm, Some(entry.bpm + (next.bpm - entry.bpm) * beat_in_seg / span))
        } else {
            entry.seconds + beat_in_seg * 60.0 / entry.bpm
        }
    }

    /// Inverse of [`transform`]: convert parent time units back to local beats.
    pub fn inverse_transform(&self, parent_time: f64) -> f64 {
        if parent_time <= 0.0 {
            return 0.0;
        }
        let n = self.entries.len();
        let idx = self.entries.partition_point(|e| e.seconds <= parent_time).saturating_sub(1);
        let idx = idx.min(n - 1);
        let entry = &self.entries[idx];
        let delta_t = parent_time - entry.seconds;
        if entry.interpolation == TempoInterpolation::Linear && idx + 1 < n {
            let next = &self.entries[idx + 1];
            let span = next.beat - entry.beat;
            entry.beat + segment_beats(delta_t, span, entry.bpm, Some(next.bpm))
        } else {
            entry.beat + delta_t * entry.bpm / 60.0
        }
    }

    /// Build a `TempoMap` from a list of `(beat, bpm)` keyframes (step interpolation).
    /// Always inserts a beat-0 entry using the first keyframe's BPM (or 120.0 if empty).
    pub fn from_keyframes(keyframes: &[(f64, f64)]) -> Self {
        if keyframes.is_empty() {
            return Self::constant(120.0);
        }
        let mut entries: Vec<TempoEntry> = keyframes
            .iter()
            .map(|&(beat, bpm)| TempoEntry { beat, bpm, seconds: 0.0, interpolation: TempoInterpolation::Step })
            .collect();
        entries.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
        if entries[0].beat > 0.0 {
            entries.insert(0, TempoEntry { beat: 0.0, bpm: entries[0].bpm, seconds: 0.0, interpolation: TempoInterpolation::Step });
        }
        let mut map = Self { entries, last_index: AtomicUsize::new(0) };
        map.rebuild_seconds();
        map
    }
}

/// Convert local beats through a stack of tempo maps to absolute seconds.
pub fn beats_to_seconds_stack(beat: f64, stack: &[&TempoMap]) -> f64 {
    let mut t = beat;
    for tm in stack.iter().rev() {
        t = tm.transform(t);
    }
    t
}

/// Inverse of [`beats_to_seconds_stack`]: absolute seconds → local beats.
pub fn seconds_to_beats_stack(seconds: f64, stack: &[&TempoMap]) -> f64 {
    let mut t = seconds;
    for tm in stack.iter() {
        t = tm.inverse_transform(t);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_bpm_round_trip() {
        let m = TempoMap::constant(120.0);
        assert!((m.beats_to_seconds(Beats(2.0)).0 - 1.0).abs() < 1e-9);
        assert!((m.seconds_to_beats(Seconds(1.0)).0 - 2.0).abs() < 1e-9);
    }

    #[test]
    fn variable_tempo_step() {
        let m = TempoMap::from_keyframes(&[(0.0, 120.0), (4.0, 60.0)]);
        // Beat 0-4: 120 BPM → 4 beats = 2 seconds
        assert!((m.beats_to_seconds(Beats(4.0)).0 - 2.0).abs() < 1e-9, "got {}", m.beats_to_seconds(Beats(4.0)).0);
        // Beat 4-5: 60 BPM → 1 beat = 1 second
        assert!((m.beats_to_seconds(Beats(5.0)).0 - 3.0).abs() < 1e-9);
        assert!((m.seconds_to_beats(Seconds(3.0)).0 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn linear_interpolation_round_trip() {
        // 120→240 BPM over 4 beats: slope = (240-120)/4 = 30 BPM/beat
        // Δt = (60/30) * ln(240/120) = 2 * ln(2) ≈ 1.386s for beats 0-4
        let mut m = TempoMap::constant(120.0);
        m.entries.push(TempoEntry { beat: 4.0, bpm: 240.0, seconds: 0.0, interpolation: TempoInterpolation::Step });
        m.entries[0].interpolation = TempoInterpolation::Linear;
        m.rebuild_seconds();

        let expected = 2.0 * std::f64::consts::LN_2;
        let got = m.beats_to_seconds(Beats(4.0)).0;
        assert!((got - expected).abs() < 1e-9, "got {got}, expected {expected}");

        // Round-trip
        let beats_back = m.seconds_to_beats(Seconds(expected)).0;
        assert!((beats_back - 4.0).abs() < 1e-9, "round-trip got {beats_back}");
    }

    #[test]
    fn stack_composition() {
        let root = TempoMap::constant(120.0);
        let group = TempoMap::constant(60.0);
        let stack: Vec<&TempoMap> = vec![&root, &group];
        let secs = beats_to_seconds_stack(2.0, &stack);
        assert!((secs - 1.0).abs() < 1e-9, "got {secs}");
        let beats = seconds_to_beats_stack(1.0, &stack);
        assert!((beats - 2.0).abs() < 1e-9, "got {beats}");
    }

    #[test]
    fn sequential_cache() {
        let m = TempoMap::constant(120.0);
        for i in 0..10 {
            let secs = m.beats_to_seconds(Beats(i as f64));
            assert!((secs.0 - i as f64 * 0.5).abs() < 1e-9);
        }
    }
}
