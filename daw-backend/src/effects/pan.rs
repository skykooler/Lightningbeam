use super::Effect;

/// Stereo panning effect using constant-power panning law
///
/// Parameters:
/// - 0: Pan position (-1.0 = full left, 0.0 = center, +1.0 = full right)
pub struct PanEffect {
    pan: f32,
    left_gain: f32,
    right_gain: f32,
}

impl PanEffect {
    /// Create a new pan effect with center panning
    pub fn new() -> Self {
        let mut effect = Self {
            pan: 0.0,
            left_gain: 1.0,
            right_gain: 1.0,
        };
        effect.update_gains();
        effect
    }

    /// Create a pan effect with a specific pan position
    pub fn with_pan(pan: f32) -> Self {
        let mut effect = Self {
            pan: pan.clamp(-1.0, 1.0),
            left_gain: 1.0,
            right_gain: 1.0,
        };
        effect.update_gains();
        effect
    }

    /// Set pan position (-1.0 = left, 0.0 = center, +1.0 = right)
    pub fn set_pan(&mut self, pan: f32) {
        self.pan = pan.clamp(-1.0, 1.0);
        self.update_gains();
    }

    /// Get current pan position
    pub fn pan(&self) -> f32 {
        self.pan
    }

    /// Update left/right gains using constant-power panning law
    fn update_gains(&mut self) {
        use std::f32::consts::PI;

        // Constant-power panning: pan from -1 to +1 maps to angle 0 to PI/2
        let angle = (self.pan + 1.0) * 0.5 * PI / 2.0;

        self.left_gain = angle.cos();
        self.right_gain = angle.sin();
    }
}

impl Default for PanEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for PanEffect {
    fn process(&mut self, buffer: &mut [f32], channels: usize, _sample_rate: u32) {
        if channels == 2 {
            // Stereo processing
            for frame in buffer.chunks_exact_mut(2) {
                frame[0] *= self.left_gain;
                frame[1] *= self.right_gain;
            }
        }
        // Mono and other channel counts: no panning applied
    }

    fn set_parameter(&mut self, id: u32, value: f32) {
        if id == 0 {
            self.set_pan(value);
        }
    }

    fn get_parameter(&self, id: u32) -> f32 {
        if id == 0 {
            self.pan
        } else {
            0.0
        }
    }

    fn reset(&mut self) {
        // Pan has no state to reset
    }

    fn name(&self) -> &str {
        "Pan"
    }
}
