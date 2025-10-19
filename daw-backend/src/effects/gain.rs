use super::Effect;

/// Simple gain/volume effect
///
/// Parameters:
/// - 0: Gain in dB (-60.0 to +12.0)
pub struct GainEffect {
    gain_db: f32,
    gain_linear: f32,
}

impl GainEffect {
    /// Create a new gain effect with 0 dB (unity) gain
    pub fn new() -> Self {
        Self {
            gain_db: 0.0,
            gain_linear: 1.0,
        }
    }

    /// Create a gain effect with a specific dB value
    pub fn with_gain_db(gain_db: f32) -> Self {
        let gain_linear = db_to_linear(gain_db);
        Self {
            gain_db,
            gain_linear,
        }
    }

    /// Set gain in decibels
    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = gain_db.clamp(-60.0, 12.0);
        self.gain_linear = db_to_linear(self.gain_db);
    }

    /// Get current gain in decibels
    pub fn gain_db(&self) -> f32 {
        self.gain_db
    }
}

impl Default for GainEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for GainEffect {
    fn process(&mut self, buffer: &mut [f32], _channels: usize, _sample_rate: u32) {
        for sample in buffer.iter_mut() {
            *sample *= self.gain_linear;
        }
    }

    fn set_parameter(&mut self, id: u32, value: f32) {
        if id == 0 {
            self.set_gain_db(value);
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        if id == 0 {
            self.gain_db
        } else {
            0.0
        }
    }

    fn reset(&mut self) {
        // Gain has no state to reset
    }

    fn name(&self) -> &str {
        "Gain"
    }
}

/// Convert decibels to linear gain
#[inline]
fn db_to_linear(db: f32) -> f32 {
    if db <= -60.0 {
        0.0
    } else {
        10.0_f32.powf(db / 20.0)
    }
}

/// Convert linear gain to decibels
#[inline]
#[allow(dead_code)]
fn linear_to_db(linear: f32) -> f32 {
    if linear <= 0.0 {
        -60.0
    } else {
        20.0 * linear.log10()
    }
}
