//! Beat/measure ↔ seconds conversion utilities

use crate::document::TimeSignature;

/// Position expressed as measure, beat, tick
#[derive(Debug, Clone, Copy)]
pub struct MeasurePosition {
    pub measure: u32, // 1-indexed
    pub beat: u32,    // 1-indexed
    pub tick: u32,    // 0-999 (subdivision of beat)
}

/// Convert a time in seconds to a measure position
pub fn time_to_measure(time: f64, bpm: f64, time_sig: &TimeSignature) -> MeasurePosition {
    let beats_per_second = bpm / 60.0;
    let total_beats = (time * beats_per_second).max(0.0);
    let beats_per_measure = time_sig.numerator as f64;

    let measure = (total_beats / beats_per_measure).floor() as u32 + 1;
    let beat = (total_beats.rem_euclid(beats_per_measure)).floor() as u32 + 1;
    let tick = ((total_beats.rem_euclid(1.0)) * 1000.0).floor() as u32;

    MeasurePosition { measure, beat, tick }
}

/// Convert a measure position to seconds
pub fn measure_to_time(pos: MeasurePosition, bpm: f64, time_sig: &TimeSignature) -> f64 {
    let beats_per_measure = time_sig.numerator as f64;
    let total_beats = (pos.measure as f64 - 1.0) * beats_per_measure
        + (pos.beat as f64 - 1.0)
        + (pos.tick as f64 / 1000.0);
    let beats_per_second = bpm / 60.0;
    total_beats / beats_per_second
}

/// Get the duration of one beat in seconds
pub fn beat_duration(bpm: f64) -> f64 {
    60.0 / bpm
}

/// Get the duration of one measure in seconds
pub fn measure_duration(bpm: f64, time_sig: &TimeSignature) -> f64 {
    beat_duration(bpm) * time_sig.numerator as f64
}
