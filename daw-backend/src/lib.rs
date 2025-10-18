// DAW Backend - Phase 4: Clips & Timeline
//
// A DAW backend with timeline-based playback, clips, and audio pool.
// Supports multiple tracks, mixing, per-track volume/mute/solo, and shared audio data.
// Uses lock-free command queues, cpal for audio I/O, and symphonia for audio file decoding.

pub mod audio;
pub mod command;
pub mod io;

// Re-export commonly used types
pub use audio::{AudioPool, Clip, ClipId, Engine, EngineController, PoolAudioFile, Track, TrackId};
pub use command::{AudioEvent, Command};
pub use io::AudioFile;
