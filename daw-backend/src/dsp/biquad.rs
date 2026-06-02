use std::f32::consts::PI;

/// Biquad filter implementation (2-pole IIR filter)
///
/// Transfer function: H(z) = (b0 + b1*z^-1 + b2*z^-2) / (1 + a1*z^-1 + a2*z^-2)
#[derive(Clone)]
pub struct BiquadFilter {
    // Filter coefficients
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,

    // State variables (per channel, supporting up to 2 channels)
    x1: [f32; 2],
    x2: [f32; 2],
    y1: [f32; 2],
    y2: [f32; 2],
}

impl BiquadFilter {
    /// Create a new biquad filter with unity gain (pass-through)
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: [0.0; 2],
            x2: [0.0; 2],
            y1: [0.0; 2],
            y2: [0.0; 2],
        }
    }

    /// Create a lowpass filter
    ///
    /// # Arguments
    /// * `frequency` - Cutoff frequency in Hz
    /// * `q` - Quality factor (resonance), typically 0.707 for Butterworth
    /// * `sample_rate` - Sample rate in Hz
    pub fn lowpass(frequency: f32, q: f32, sample_rate: f32) -> Self {
        let mut filter = Self::new();
        filter.set_lowpass(frequency, q, sample_rate);
        filter
    }

    /// Create a highpass filter
    ///
    /// # Arguments
    /// * `frequency` - Cutoff frequency in Hz
    /// * `q` - Quality factor (resonance), typically 0.707 for Butterworth
    /// * `sample_rate` - Sample rate in Hz
    pub fn highpass(frequency: f32, q: f32, sample_rate: f32) -> Self {
        let mut filter = Self::new();
        filter.set_highpass(frequency, q, sample_rate);
        filter
    }

    /// Create a peaking EQ filter
    ///
    /// # Arguments
    /// * `frequency` - Center frequency in Hz
    /// * `q` - Quality factor (bandwidth)
    /// * `gain_db` - Gain in decibels
    /// * `sample_rate` - Sample rate in Hz
    pub fn peaking(frequency: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let mut filter = Self::new();
        filter.set_peaking(frequency, q, gain_db, sample_rate);
        filter
    }

    /// Set coefficients for a lowpass filter
    pub fn set_lowpass(&mut self, frequency: f32, q: f32, sample_rate: f32) {
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);

        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_omega) / 2.0) / a0;
        self.b1 = (1.0 - cos_omega) / a0;
        self.b2 = ((1.0 - cos_omega) / 2.0) / a0;
        self.a1 = (-2.0 * cos_omega) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Set coefficients for a highpass filter
    pub fn set_highpass(&mut self, frequency: f32, q: f32, sample_rate: f32) {
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let alpha = sin_omega / (2.0 * q);

        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 + cos_omega) / 2.0) / a0;
        self.b1 = -(1.0 + cos_omega) / a0;
        self.b2 = ((1.0 + cos_omega) / 2.0) / a0;
        self.a1 = (-2.0 * cos_omega) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    /// Set coefficients for a peaking EQ filter
    pub fn set_peaking(&mut self, frequency: f32, q: f32, gain_db: f32, sample_rate: f32) {
        let omega = 2.0 * PI * frequency / sample_rate;
        let sin_omega = omega.sin();
        let cos_omega = omega.cos();
        let a_gain = 10.0_f32.powf(gain_db / 40.0);
        let alpha = sin_omega / (2.0 * q);

        let a0 = 1.0 + alpha / a_gain;
        self.b0 = (1.0 + alpha * a_gain) / a0;
        self.b1 = (-2.0 * cos_omega) / a0;
        self.b2 = (1.0 - alpha * a_gain) / a0;
        self.a1 = (-2.0 * cos_omega) / a0;
        self.a2 = (1.0 - alpha / a_gain) / a0;
    }

    /// Process a single sample
    ///
    /// # Arguments
    /// * `input` - Input sample
    /// * `channel` - Channel index (0 or 1)
    ///
    /// # Returns
    /// Filtered output sample
    #[inline]
    pub fn process_sample(&mut self, input: f32, channel: usize) -> f32 {
        let channel = channel.min(1); // Clamp to 0 or 1

        // Direct Form II Transposed implementation
        let output = self.b0 * input + self.x1[channel];

        self.x1[channel] = self.b1 * input - self.a1 * output + self.x2[channel];
        self.x2[channel] = self.b2 * input - self.a2 * output;

        output
    }

    /// Process a buffer of interleaved samples
    ///
    /// # Arguments
    /// * `buffer` - Interleaved audio samples
    /// * `channels` - Number of channels
    pub fn process_buffer(&mut self, buffer: &mut [f32], channels: usize) {
        if channels == 1 {
            // Mono
            for sample in buffer.iter_mut() {
                *sample = self.process_sample(*sample, 0);
            }
        } else if channels == 2 {
            // Stereo
            for frame in buffer.chunks_exact_mut(2) {
                frame[0] = self.process_sample(frame[0], 0);
                frame[1] = self.process_sample(frame[1], 1);
            }
        }
    }

    /// Reset filter state (clear delay lines)
    pub fn reset(&mut self) {
        self.x1 = [0.0; 2];
        self.x2 = [0.0; 2];
        self.y1 = [0.0; 2];
        self.y2 = [0.0; 2];
    }
}

impl Default for BiquadFilter {
    fn default() -> Self {
        Self::new()
    }
}
