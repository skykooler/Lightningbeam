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
    AudioPool, AudioTrack, AutomationLane, AutomationLaneId, AutomationPoint, BufferPool, Clip, ClipId, CurveType, Engine, EngineController,
    Metatrack, MidiClip, MidiClipId, MidiEvent, MidiTrack, ParameterId, PoolAudioFile, Project, RecordingState, RenderContext, Track, TrackId,
    TrackNode,
};
pub use command::{AudioEvent, Command};
pub use effects::{Effect, GainEffect, PanEffect, SimpleEQ, SimpleSynth};
pub use io::{load_midi_file, AudioFile, WaveformPeak, WavWriter};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Simple audio system that handles cpal initialization internally
pub struct AudioSystem {
    pub controller: EngineController,
    pub stream: cpal::Stream,
    pub event_rx: rtrb::Consumer<AudioEvent>,
    pub sample_rate: u32,
    pub channels: u32,
}

impl AudioSystem {
    /// Initialize the audio system with default device
    pub fn new() -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No output device available")?;

        let default_config = device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = default_config.sample_rate().0;
        let channels = default_config.channels() as u32;

        // Create queues
        let (command_tx, command_rx) = rtrb::RingBuffer::new(256);
        let (event_tx, event_rx) = rtrb::RingBuffer::new(256);

        // Create engine
        let mut engine = Engine::new(sample_rate, channels, command_rx, event_tx);
        let controller = engine.get_controller(command_tx);

        // Build stream
        let config: cpal::StreamConfig = default_config.clone().into();
        let mut buffer = vec![0.0f32; 16384];

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let buf = &mut buffer[..data.len()];
                    buf.fill(0.0);
                    engine.process(buf);
                    data.copy_from_slice(buf);
                },
                |err| eprintln!("Stream error: {}", err),
                None,
            )
            .map_err(|e| e.to_string())?;

        stream.play().map_err(|e| e.to_string())?;

        Ok(Self {
            controller,
            stream,
            event_rx,
            sample_rate,
            channels,
        })
    }
}
