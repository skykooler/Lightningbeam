// DAW Backend - Phase 6: Hierarchical Tracks
//
// A DAW backend with timeline-based playback, clips, audio pool, effects, and hierarchical track groups.
// Supports multiple tracks, mixing, per-track volume/mute/solo, shared audio data, effect chains, and nested groups.
// Uses lock-free command queues, cpal for audio I/O, and symphonia for audio file decoding.

pub mod audio;
pub mod command;
pub mod dsp;
pub mod effects;
pub mod io;

// Re-export commonly used types
pub use audio::{
    AudioPool, AudioTrack, BufferPool, Clip, ClipId, Engine, EngineController,
    Metatrack, MidiClip, MidiClipId, MidiEvent, MidiTrack, PoolAudioFile, Project, RenderContext, Track, TrackId, TrackNode,
};
pub use command::{AudioEvent, Command};
pub use effects::{Effect, GainEffect, PanEffect, SimpleEQ, SimpleSynth};
pub use io::{load_midi_file, AudioFile};
