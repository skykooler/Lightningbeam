/// Audio effect processor trait
///
/// All effects must be Send to be usable in the audio thread.
/// Effects should be real-time safe: no allocations, no blocking operations.
pub trait Effect: Send {
    /// Process audio buffer in-place
    ///
    /// # Arguments
    /// * `buffer` - Interleaved audio samples to process
    /// * `channels` - Number of audio channels (2 for stereo)
    /// * `sample_rate` - Sample rate in Hz
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32);

    /// Set an effect parameter
    ///
    /// # Arguments
    /// * `id` - Parameter identifier
    /// * `value` - Parameter value (normalized or specific units depending on parameter)
    fn set_parameter(&mut self, id: u32, value: f32);

    /// Get an effect parameter value
    ///
    /// # Arguments
    /// * `id` - Parameter identifier
    ///
    /// # Returns
    /// Current parameter value
    fn get_parameter(&self, id: u32) -> f32;

    /// Reset effect state (clear delays, resonances, etc.)
    fn reset(&mut self);

    /// Get the effect name
    fn name(&self) -> &str;
}
