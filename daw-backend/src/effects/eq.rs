use super::Effect;
use crate::dsp::BiquadFilter;

/// Simple 3-band EQ (low shelf, mid peak, high shelf)
///
/// Parameters:
/// - 0: Low gain in dB (-12.0 to +12.0)
/// - 1: Mid gain in dB (-12.0 to +12.0)
/// - 2: High gain in dB (-12.0 to +12.0)
/// - 3: Low frequency in Hz (default: 250)
/// - 4: Mid frequency in Hz (default: 1000)
/// - 5: High frequency in Hz (default: 8000)
pub struct SimpleEQ {
    low_gain: f32,
    mid_gain: f32,
    high_gain: f32,
    low_freq: f32,
    mid_freq: f32,
    high_freq: f32,

    low_filter: BiquadFilter,
    mid_filter: BiquadFilter,
    high_filter: BiquadFilter,

    sample_rate: f32,
}

impl SimpleEQ {
    /// Create a new SimpleEQ with flat response
    pub fn new() -> Self {
        Self {
            low_gain: 0.0,
            mid_gain: 0.0,
            high_gain: 0.0,
            low_freq: 250.0,
            mid_freq: 1000.0,
            high_freq: 8000.0,
            low_filter: BiquadFilter::new(),
            mid_filter: BiquadFilter::new(),
            high_filter: BiquadFilter::new(),
            sample_rate: 48000.0, // Default, will be updated on first process
        }
    }

    /// Set low band gain in decibels
    pub fn set_low_gain(&mut self, gain_db: f32) {
        self.low_gain = gain_db.clamp(-12.0, 12.0);
        self.update_filters();
    }

    /// Set mid band gain in decibels
    pub fn set_mid_gain(&mut self, gain_db: f32) {
        self.mid_gain = gain_db.clamp(-12.0, 12.0);
        self.update_filters();
    }

    /// Set high band gain in decibels
    pub fn set_high_gain(&mut self, gain_db: f32) {
        self.high_gain = gain_db.clamp(-12.0, 12.0);
        self.update_filters();
    }

    /// Set low band frequency
    pub fn set_low_freq(&mut self, freq: f32) {
        self.low_freq = freq.clamp(20.0, 500.0);
        self.update_filters();
    }

    /// Set mid band frequency
    pub fn set_mid_freq(&mut self, freq: f32) {
        self.mid_freq = freq.clamp(200.0, 5000.0);
        self.update_filters();
    }

    /// Set high band frequency
    pub fn set_high_freq(&mut self, freq: f32) {
        self.high_freq = freq.clamp(2000.0, 20000.0);
        self.update_filters();
    }

    /// Update filter coefficients based on current parameters
    fn update_filters(&mut self) {
        // Only update if sample rate has been set
        if self.sample_rate > 0.0 {
            // Use peaking filters for all bands
            // Q factor of 1.0 gives a moderate bandwidth
            self.low_filter.set_peaking(self.low_freq, 1.0, self.low_gain, self.sample_rate);
            self.mid_filter.set_peaking(self.mid_freq, 1.0, self.mid_gain, self.sample_rate);
            self.high_filter.set_peaking(self.high_freq, 1.0, self.high_gain, self.sample_rate);
        }
    }
}

impl Default for SimpleEQ {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for SimpleEQ {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32) {
        // Update sample rate if it changed
        let sr = sample_rate as f32;
        if (self.sample_rate - sr).abs() > 0.1 {
            self.sample_rate = sr;
            self.update_filters();
        }

        // Process through each filter in series
        self.low_filter.process_buffer(buffer, channels);
        self.mid_filter.process_buffer(buffer, channels);
        self.high_filter.process_buffer(buffer, channels);
    }

    fn set_parameter(&mut self, id: u32, value: f32) {
        match id {
            0 => self.set_low_gain(value),
            1 => self.set_mid_gain(value),
            2 => self.set_high_gain(value),
            3 => self.set_low_freq(value),
            4 => self.set_mid_freq(value),
            5 => self.set_high_freq(value),
            _ => {}
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        match id {
            0 => self.low_gain,
            1 => self.mid_gain,
            2 => self.high_gain,
            3 => self.low_freq,
            4 => self.mid_freq,
            5 => self.high_freq,
            _ => 0.0,
        }
    }

    fn reset(&mut self) {
        self.low_filter.reset();
        self.mid_filter.reset();
        self.high_filter.reset();
    }

    fn name(&self) -> &str {
        "SimpleEQ"
    }
}
