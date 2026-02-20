use std::f32::consts::PI;

/// State Variable Filter mode
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SvfMode {
    Lowpass = 0,
    Highpass = 1,
    Bandpass = 2,
    Notch = 3,
}

impl SvfMode {
    pub fn from_f32(value: f32) -> Self {
        match value.round() as i32 {
            1 => SvfMode::Highpass,
            2 => SvfMode::Bandpass,
            3 => SvfMode::Notch,
            _ => SvfMode::Lowpass,
        }
    }
}

/// Linear trapezoidal integrated State Variable Filter (Simper/Cytomic)
///
/// Zero-delay feedback topology. Per-sample cutoff modulation is cheap —
/// just update `g` and `k` coefficients (no per-sample trig needed if
/// cutoff hasn't changed).
#[derive(Clone)]
pub struct SvfFilter {
    // Coefficients
    g: f32,  // frequency warping: tan(π * cutoff / sample_rate)
    k: f32,  // damping: 2 - 2*resonance
    a1: f32, // 1 / (1 + g*(g+k))
    a2: f32, // g * a1

    // State per channel (up to 2 for stereo)
    ic1eq: [f32; 2],
    ic2eq: [f32; 2],

    mode: SvfMode,
}

impl SvfFilter {
    /// Create a new SVF with default parameters (1kHz lowpass, no resonance)
    pub fn new() -> Self {
        let mut filter = Self {
            g: 0.0,
            k: 2.0,
            a1: 0.0,
            a2: 0.0,
            ic1eq: [0.0; 2],
            ic2eq: [0.0; 2],
            mode: SvfMode::Lowpass,
        };
        filter.set_params(1000.0, 0.0, 44100.0);
        filter
    }

    /// Set filter parameters
    ///
    /// # Arguments
    /// * `cutoff_hz` - Cutoff frequency in Hz (clamped to valid range)
    /// * `resonance` - Resonance 0.0 (none) to 1.0 (self-oscillation)
    /// * `sample_rate` - Sample rate in Hz
    #[inline]
    pub fn set_params(&mut self, cutoff_hz: f32, resonance: f32, sample_rate: f32) {
        // Clamp cutoff to avoid instability near Nyquist
        let cutoff = cutoff_hz.clamp(5.0, sample_rate * 0.49);
        let resonance = resonance.clamp(0.0, 1.0);

        self.g = (PI * cutoff / sample_rate).tan();
        self.k = 2.0 - 2.0 * resonance;
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
        self.a2 = self.g * self.a1;
    }

    /// Set filter mode
    pub fn set_mode(&mut self, mode: SvfMode) {
        self.mode = mode;
    }

    /// Process a single sample, returning all four outputs: (lowpass, highpass, bandpass, notch)
    #[inline]
    pub fn process_sample_quad(&mut self, input: f32, channel: usize) -> (f32, f32, f32, f32) {
        let ch = channel.min(1);

        let v3 = input - self.ic2eq[ch];
        let v1 = self.a1 * self.ic1eq[ch] + self.a2 * v3;
        let v2 = self.ic2eq[ch] + self.g * v1;

        self.ic1eq[ch] = 2.0 * v1 - self.ic1eq[ch];
        self.ic2eq[ch] = 2.0 * v2 - self.ic2eq[ch];

        let hp = input - self.k * v1 - v2;
        (v2, hp, v1, hp + v2)
    }

    /// Process a single sample with a selected mode
    #[inline]
    pub fn process_sample(&mut self, input: f32, channel: usize) -> f32 {
        let (lp, hp, bp, notch) = self.process_sample_quad(input, channel);
        match self.mode {
            SvfMode::Lowpass => lp,
            SvfMode::Highpass => hp,
            SvfMode::Bandpass => bp,
            SvfMode::Notch => notch,
        }
    }

    /// Process a buffer of interleaved samples
    pub fn process_buffer(&mut self, buffer: &mut [f32], channels: usize) {
        if channels == 1 {
            for sample in buffer.iter_mut() {
                *sample = self.process_sample(*sample, 0);
            }
        } else if channels == 2 {
            for frame in buffer.chunks_exact_mut(2) {
                frame[0] = self.process_sample(frame[0], 0);
                frame[1] = self.process_sample(frame[1], 1);
            }
        }
    }

    /// Reset filter state (clear delay lines)
    pub fn reset(&mut self) {
        self.ic1eq = [0.0; 2];
        self.ic2eq = [0.0; 2];
    }
}

impl Default for SvfFilter {
    fn default() -> Self {
        Self::new()
    }
}
