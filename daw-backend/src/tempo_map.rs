//! TempoMap — beats ↔ seconds conversion with variable tempo support.
//!
//! Positions are stored in **beats** throughout the project; `TempoMap` converts
//! between beats and seconds at render / scheduling time.
//!
//! # Format
//! `entries` is a sorted `Vec<TempoEntry>` where each entry marks where a new
//! constant BPM segment begins.  Step interpolation is used: BPM is constant
//! from `entry.beat` until the next entry.  The first entry must always have
//! `beat == 0.0`.
//!
//! # Sequential-access optimisation
//! An `AtomicUsize` caches the index of the last segment visited by
//! `beats_to_seconds`.  When calls are in ascending order (the common case when
//! walking events in order) the scan starts from the cached index instead of
//! the beginning, giving amortised O(1) behaviour.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use crate::time::{Beats, Seconds};

/// A single tempo segment: from `beat` onwards the tempo is `bpm`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TempoEntry {
    /// Start of this tempo segment in beats (quarter-note beats).
    pub beat: f64,
    /// Tempo for this segment in beats per minute.
    pub bpm: f64,
    /// Cumulative seconds elapsed at the start of this segment.
    /// **Derived** — not serialised; call [`TempoMap::rebuild_seconds`] after any
    /// mutation or after deserialization.
    #[serde(skip, default)]
    pub seconds: f64,
}

/// A piecewise-constant tempo map used to convert between beats and seconds.
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

impl TempoMap {
    /// Create a constant-tempo map.
    pub fn constant(bpm: f64) -> Self {
        Self {
            entries: vec![TempoEntry { beat: 0.0, bpm, seconds: 0.0 }],
            last_index: AtomicUsize::new(0),
        }
    }

    /// Rebuild the `seconds` field on every entry from scratch.
    /// **Must be called** after any mutation (add/remove/reorder entry) and
    /// after deserialization.
    pub fn rebuild_seconds(&mut self) {
        let mut cumulative = 0.0_f64;
        for i in 0..self.entries.len() {
            self.entries[i].seconds = cumulative;
            if i + 1 < self.entries.len() {
                let span_beats = self.entries[i + 1].beat - self.entries[i].beat;
                cumulative += span_beats * 60.0 / self.entries[i].bpm;
            }
        }
        self.last_index.store(0, Ordering::Relaxed);
    }

    /// Return the BPM active at `beat`.
    pub fn bpm_at(&self, beat: Beats) -> f64 {
        let idx = self.entries.partition_point(|e| e.beat <= beat.0).saturating_sub(1);
        self.entries[idx.max(0)].bpm
    }

    /// Convert beats to seconds using the tempo map.
    ///
    /// Uses the sequential cache: if `beat` is at or after the last cached
    /// segment, the scan starts there instead of from the beginning.
    pub fn beats_to_seconds(&self, beat: Beats) -> Seconds {
        if beat.0 <= 0.0 {
            return Seconds::ZERO;
        }
        let n = self.entries.len();
        let cached = self.last_index.load(Ordering::Relaxed).min(n.saturating_sub(1));
        let start = if beat.0 >= self.entries[cached].beat { cached } else { 0 };

        let mut idx = start;
        while idx + 1 < n && self.entries[idx + 1].beat <= beat.0 {
            idx += 1;
        }
        self.last_index.store(idx, Ordering::Relaxed);

        let entry = &self.entries[idx];
        Seconds(entry.seconds + (beat.0 - entry.beat) * 60.0 / entry.bpm)
    }

    /// Convert seconds to beats using binary search on the cached `seconds` offsets.
    pub fn seconds_to_beats(&self, seconds: Seconds) -> Beats {
        if seconds.0 <= 0.0 {
            return Beats::ZERO;
        }
        let n = self.entries.len();
        let idx = self.entries.partition_point(|e| e.seconds <= seconds.0).saturating_sub(1);
        let idx = idx.min(n - 1);
        let entry = &self.entries[idx];
        Beats(entry.beat + (seconds.0 - entry.seconds) * entry.bpm / 60.0)
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
    /// group the result is the *parent group's* beats — the caller is
    /// responsible for interpreting the units correctly.
    ///
    /// This is the same arithmetic as `beats_to_seconds` but without the
    /// `Seconds` newtype so it can be composed across multiple levels.
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
        entry.seconds + (beat - entry.beat) * 60.0 / entry.bpm
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
        entry.beat + (parent_time - entry.seconds) * entry.bpm / 60.0
    }

    /// Build a `TempoMap` from a list of `(beat, bpm)` keyframes.
    /// Always inserts a beat-0 entry using the first keyframe's BPM (or 120.0 if empty).
    pub fn from_keyframes(keyframes: &[(f64, f64)]) -> Self {
        if keyframes.is_empty() {
            return Self::constant(120.0);
        }
        let mut entries: Vec<TempoEntry> = keyframes
            .iter()
            .map(|&(beat, bpm)| TempoEntry { beat, bpm, seconds: 0.0 })
            .collect();
        entries.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
        if entries[0].beat > 0.0 {
            entries.insert(0, TempoEntry { beat: 0.0, bpm: entries[0].bpm, seconds: 0.0 });
        }
        let mut map = Self { entries, last_index: AtomicUsize::new(0) };
        map.rebuild_seconds();
        map
    }
}

/// Convert local beats through a stack of tempo maps to absolute seconds.
///
/// `stack[0]` is the outermost map (root/master); `stack[last]` is the
/// innermost (deepest group).  Conversion applies maps from innermost to
/// outermost — each map's output is the input to the next outer map.
///
/// ```text
/// clip_beats → [stack[last]] → … → [stack[0]] → absolute seconds
/// ```
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
    fn variable_tempo() {
        let m = TempoMap::from_keyframes(&[(0.0, 120.0), (4.0, 60.0)]);
        assert!((m.beats_to_seconds(Beats(4.0)).0 - 2.0).abs() < 1e-9);
        assert!((m.beats_to_seconds(Beats(5.0)).0 - 3.0).abs() < 1e-9);
        assert!((m.seconds_to_beats(Seconds(3.0)).0 - 5.0).abs() < 1e-9);
    }

    #[test]
    fn stack_composition() {
        // Root: 120 BPM (1 beat = 0.5s)
        // Group: 60 BPM  (1 local beat = 1 "parent beat" worth of time)
        // Clip at local beat 2.0 → 2 parent beats → 1.0s absolute
        let root = TempoMap::constant(120.0);
        let group = TempoMap::constant(60.0);
        let stack: Vec<&TempoMap> = vec![&root, &group];
        let secs = beats_to_seconds_stack(2.0, &stack);
        // group.transform(2.0) = 2.0*60/60 = 2.0 parent_beats
        // root.transform(2.0) = 2.0*60/120 = 1.0s
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
