/// BPM Detection using autocorrelation and onset detection
///
/// This module provides both offline analysis (for audio import)
/// and real-time streaming analysis (for the BPM detector node)

use std::collections::VecDeque;

/// Detects BPM from a complete audio buffer (offline analysis)
pub fn detect_bpm_offline(audio: &[f32], sample_rate: u32) -> Option<f32> {
    if audio.is_empty() {
        return None;
    }

    // Convert to mono if needed (already mono in our case)
    // Downsample for efficiency (analyze every 4th sample for faster processing)
    let downsampled: Vec<f32> = audio.iter().step_by(4).copied().collect();
    let effective_sample_rate = sample_rate / 4;

    // Detect onsets using energy-based method
    let onsets = detect_onsets(&downsampled, effective_sample_rate);

    if onsets.len() < 4 {
        return None;
    }

    // Calculate onset strength function for autocorrelation
    let onset_envelope = calculate_onset_envelope(&onsets, downsampled.len(), effective_sample_rate);

    // Further downsample onset envelope for BPM analysis
    // For 60-200 BPM (1-3.33 Hz), we only need ~10 Hz sample rate by Nyquist
    // Use 100 Hz for good margin (100 samples per second)
    let tempo_sample_rate = 100.0;
    let downsample_factor = (effective_sample_rate as f32 / tempo_sample_rate) as usize;
    let downsampled_envelope: Vec<f32> = onset_envelope
        .iter()
        .step_by(downsample_factor.max(1))
        .copied()
        .collect();

    // Use autocorrelation to find the fundamental period
    let bpm = detect_bpm_autocorrelation(&downsampled_envelope, tempo_sample_rate as u32);

    bpm
}

/// Calculate an onset envelope from detected onsets
fn calculate_onset_envelope(onsets: &[usize], total_length: usize, sample_rate: u32) -> Vec<f32> {
    // Create a sparse representation of onsets with exponential decay
    let mut envelope = vec![0.0; total_length];
    let decay_samples = (sample_rate as f32 * 0.05) as usize; // 50ms decay

    for &onset in onsets {
        if onset < total_length {
            envelope[onset] = 1.0;
            // Add exponential decay after onset
            for i in 1..decay_samples.min(total_length - onset) {
                let decay_value = (-3.0 * i as f32 / decay_samples as f32).exp();
                envelope[onset + i] = f32::max(envelope[onset + i], decay_value);
            }
        }
    }

    envelope
}

/// Detect BPM using autocorrelation on onset envelope
fn detect_bpm_autocorrelation(onset_envelope: &[f32], sample_rate: u32) -> Option<f32> {
    // BPM range: 60-200 BPM
    let min_bpm = 60.0;
    let max_bpm = 200.0;

    let min_lag = (60.0 * sample_rate as f32 / max_bpm) as usize;
    let max_lag = (60.0 * sample_rate as f32 / min_bpm) as usize;

    if max_lag >= onset_envelope.len() / 2 {
        return None;
    }

    // Calculate autocorrelation for tempo range
    let mut best_lag = min_lag;
    let mut best_correlation = 0.0;

    for lag in min_lag..=max_lag {
        let mut correlation = 0.0;
        let mut count = 0;

        for i in 0..(onset_envelope.len() - lag) {
            correlation += onset_envelope[i] * onset_envelope[i + lag];
            count += 1;
        }

        if count > 0 {
            correlation /= count as f32;

            // Bias toward faster tempos slightly (common in EDM)
            let bias = 1.0 + (lag as f32 - min_lag as f32) / (max_lag - min_lag) as f32 * 0.1;
            correlation /= bias;

            if correlation > best_correlation {
                best_correlation = correlation;
                best_lag = lag;
            }
        }
    }

    // Convert best lag to BPM
    let bpm = 60.0 * sample_rate as f32 / best_lag as f32;

    // Check for octave errors by testing multiples
    // Common ranges: 60-90 (slow), 90-140 (medium), 140-200 (fast)
    let half_bpm = bpm / 2.0;
    let double_bpm = bpm * 2.0;
    let quad_bpm = bpm * 4.0;

    // Choose the octave that falls in the most common range (100-180 BPM for EDM/pop)
    let final_bpm = if quad_bpm >= 100.0 && quad_bpm <= 200.0 {
        // Very slow detection, multiply by 4
        quad_bpm
    } else if double_bpm >= 100.0 && double_bpm <= 200.0 {
        // Slow detection, multiply by 2
        double_bpm
    } else if bpm >= 100.0 && bpm <= 200.0 {
        // Already in good range
        bpm
    } else if half_bpm >= 100.0 && half_bpm <= 200.0 {
        // Too fast detection, divide by 2
        half_bpm
    } else {
        // Outside ideal range, use as-is
        bpm
    };

    // Round to nearest 0.5 BPM for cleaner values
    Some((final_bpm * 2.0).round() / 2.0)
}

/// Detect onsets (beat events) in audio using energy-based method
fn detect_onsets(audio: &[f32], sample_rate: u32) -> Vec<usize> {
    let mut onsets = Vec::new();

    // Window size for energy calculation (~20ms)
    let window_size = ((sample_rate as f32 * 0.02) as usize).max(1);
    let hop_size = window_size / 2;

    if audio.len() < window_size {
        return onsets;
    }

    // Calculate energy for each window
    let mut energies = Vec::new();
    let mut pos = 0;
    while pos + window_size <= audio.len() {
        let window = &audio[pos..pos + window_size];
        let energy: f32 = window.iter().map(|&s| s * s).sum();
        energies.push(energy / window_size as f32); // Normalize
        pos += hop_size;
    }

    if energies.len() < 3 {
        return onsets;
    }

    // Calculate energy differences (onset strength)
    let mut onset_strengths = Vec::new();
    for i in 1..energies.len() {
        let diff = (energies[i] - energies[i - 1]).max(0.0); // Only positive changes
        onset_strengths.push(diff);
    }

    // Find threshold (adaptive)
    let mean_strength: f32 = onset_strengths.iter().sum::<f32>() / onset_strengths.len() as f32;
    let threshold = mean_strength * 1.5; // 1.5x mean

    // Peak picking with minimum distance
    let min_distance = sample_rate as usize / 10; // Minimum 100ms between onsets
    let mut last_onset = 0;

    for (i, &strength) in onset_strengths.iter().enumerate() {
        if strength > threshold {
            let sample_pos = (i + 1) * hop_size;

            // Check if it's a local maximum and far enough from last onset
            let is_local_max = (i == 0 || onset_strengths[i - 1] <= strength) &&
                              (i == onset_strengths.len() - 1 || onset_strengths[i + 1] < strength);

            if is_local_max && (onsets.is_empty() || sample_pos - last_onset >= min_distance) {
                onsets.push(sample_pos);
                last_onset = sample_pos;
            }
        }
    }

    onsets
}

/// Real-time BPM detector for streaming audio
pub struct BpmDetectorRealtime {
    sample_rate: u32,

    // Circular buffer for recent audio (e.g., 10 seconds)
    audio_buffer: VecDeque<f32>,
    max_buffer_samples: usize,

    // Current BPM estimate
    current_bpm: f32,

    // Update interval (samples)
    samples_since_update: usize,
    update_interval: usize,

    // Smoothing
    bpm_history: VecDeque<f32>,
    history_size: usize,
}

impl BpmDetectorRealtime {
    pub fn new(sample_rate: u32, buffer_duration_seconds: f32) -> Self {
        let max_buffer_samples = (sample_rate as f32 * buffer_duration_seconds) as usize;
        let update_interval = sample_rate as usize; // Update every 1 second

        Self {
            sample_rate,
            audio_buffer: VecDeque::with_capacity(max_buffer_samples),
            max_buffer_samples,
            current_bpm: 120.0, // Default BPM
            samples_since_update: 0,
            update_interval,
            bpm_history: VecDeque::with_capacity(8),
            history_size: 8,
        }
    }

    /// Process a chunk of audio and return current BPM estimate
    pub fn process(&mut self, audio: &[f32]) -> f32 {
        // Add samples to buffer
        for &sample in audio {
            if self.audio_buffer.len() >= self.max_buffer_samples {
                self.audio_buffer.pop_front();
            }
            self.audio_buffer.push_back(sample);
        }

        self.samples_since_update += audio.len();

        // Periodically re-analyze
        if self.samples_since_update >= self.update_interval && self.audio_buffer.len() > self.sample_rate as usize {
            self.samples_since_update = 0;

            // Convert buffer to slice for analysis
            let buffer_vec: Vec<f32> = self.audio_buffer.iter().copied().collect();

            if let Some(detected_bpm) = detect_bpm_offline(&buffer_vec, self.sample_rate) {
                // Add to history for smoothing
                if self.bpm_history.len() >= self.history_size {
                    self.bpm_history.pop_front();
                }
                self.bpm_history.push_back(detected_bpm);

                // Use median of recent detections for stability
                let mut sorted_history: Vec<f32> = self.bpm_history.iter().copied().collect();
                sorted_history.sort_by(|a, b| a.partial_cmp(b).unwrap());
                self.current_bpm = sorted_history[sorted_history.len() / 2];
            }
        }

        self.current_bpm
    }

    pub fn get_bpm(&self) -> f32 {
        self.current_bpm
    }

    pub fn reset(&mut self) {
        self.audio_buffer.clear();
        self.bpm_history.clear();
        self.samples_since_update = 0;
        self.current_bpm = 120.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_120_bpm_detection() {
        let sample_rate = 48000;
        let bpm = 120.0;
        let beat_interval = 60.0 / bpm;
        let beat_samples = (sample_rate as f32 * beat_interval) as usize;

        // Generate 8 beats
        let mut audio = vec![0.0; beat_samples * 8];
        for beat in 0..8 {
            let pos = beat * beat_samples;
            // Add a sharp transient at each beat
            for i in 0..100 {
                audio[pos + i] = (1.0 - i as f32 / 100.0) * 0.8;
            }
        }

        let detected = detect_bpm_offline(&audio, sample_rate);
        assert!(detected.is_some());
        let detected_bpm = detected.unwrap();

        // Allow 5% tolerance
        assert!((detected_bpm - bpm).abs() / bpm < 0.05,
                "Expected ~{} BPM, got {}", bpm, detected_bpm);
    }
}
