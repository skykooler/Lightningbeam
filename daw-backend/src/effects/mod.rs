pub mod effect_trait;
pub mod eq;
pub mod gain;
pub mod pan;
pub mod synth;

pub use effect_trait::Effect;
pub use eq::SimpleEQ;
pub use gain::GainEffect;
pub use pan::PanEffect;
pub use synth::SimpleSynth;
