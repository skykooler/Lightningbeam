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
pub mod tui;

// Re-export commonly used types
pub use audio::{
    AudioPool, AudioTrack, AutomationLane, AutomationLaneId, AutomationPoint, BufferPool, Clip, ClipId, CurveType, Engine, EngineController,
    Metatrack, MidiClip, MidiClipId, MidiEvent, MidiTrack, ParameterId, PoolAudioFile, Project, RecordingState, RenderContext, Track, TrackId,
    TrackNode,
};
pub use audio::node_graph::{GraphPreset, InstrumentGraph, PresetMetadata, SerializedConnection, SerializedNode};
pub use command::{AudioEvent, Command, OscilloscopeData};
pub use io::{load_midi_file, AudioFile, WaveformPeak, WavWriter};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Trait for emitting audio events to external systems (UI, logging, etc.)
/// This allows the DAW backend to remain framework-agnostic
pub trait EventEmitter: Send + Sync {
    /// Emit an audio event
    fn emit(&self, event: AudioEvent);
}

/// Simple audio system that handles cpal initialization internally
pub struct AudioSystem {
    pub controller: EngineController,
    pub stream: cpal::Stream,
    pub sample_rate: u32,
    pub channels: u32,
}

impl AudioSystem {
    /// Initialize the audio system with default input and output devices
    ///
    /// # Arguments
    /// * `event_emitter` - Optional event emitter for pushing events to external systems
    pub fn new(event_emitter: Option<std::sync::Arc<dyn EventEmitter>>) -> Result<Self, String> {
        let host = cpal::default_host();

        // Get output device
        let output_device = host
            .default_output_device()
            .ok_or("No output device available")?;

        let default_output_config = output_device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = default_output_config.sample_rate().0;
        let channels = default_output_config.channels() as u32;

        // Create queues
        let (command_tx, command_rx) = rtrb::RingBuffer::new(256);
        let (event_tx, event_rx) = rtrb::RingBuffer::new(256);
        let (query_tx, query_rx) = rtrb::RingBuffer::new(16); // Smaller buffer for synchronous queries
        let (query_response_tx, query_response_rx) = rtrb::RingBuffer::new(16);

        // Create input ringbuffer for recording (large buffer for audio samples)
        // Buffer size: 10 seconds of audio at 48kHz stereo = 48000 * 2 * 10 = 960000 samples
        let input_buffer_size = (sample_rate * channels * 10) as usize;
        let (mut input_tx, input_rx) = rtrb::RingBuffer::new(input_buffer_size);

        // Create engine
        let mut engine = Engine::new(sample_rate, channels, command_rx, event_tx, query_rx, query_response_tx);
        engine.set_input_rx(input_rx);
        let controller = engine.get_controller(command_tx, query_tx, query_response_rx);

        // Build output stream
        let output_config: cpal::StreamConfig = default_output_config.clone().into();
        let mut output_buffer = vec![0.0f32; 16384];

        let output_stream = output_device
            .build_output_stream(
                &output_config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let buf = &mut output_buffer[..data.len()];
                    buf.fill(0.0);
                    engine.process(buf);
                    data.copy_from_slice(buf);
                },
                |err| eprintln!("Output stream error: {}", err),
                None,
            )
            .map_err(|e| e.to_string())?;

        // Get input device
        let input_device = match host.default_input_device() {
            Some(device) => device,
            None => {
                eprintln!("Warning: No input device available, recording will be disabled");
                // Start output stream and return without input
                output_stream.play().map_err(|e| e.to_string())?;

                // Spawn emitter thread if provided
                if let Some(emitter) = event_emitter {
                    Self::spawn_emitter_thread(event_rx, emitter);
                }

                return Ok(Self {
                    controller,
                    stream: output_stream,
                    sample_rate,
                    channels,
                });
            }
        };

        // Get input config matching output sample rate and channels if possible
        let input_config = match input_device.default_input_config() {
            Ok(config) => {
                let mut cfg: cpal::StreamConfig = config.into();
                // Try to match output sample rate and channels
                cfg.sample_rate = cpal::SampleRate(sample_rate);
                cfg.channels = channels as u16;
                cfg
            }
            Err(e) => {
                eprintln!("Warning: Could not get input config: {}, recording will be disabled", e);
                output_stream.play().map_err(|e| e.to_string())?;

                // Spawn emitter thread if provided
                if let Some(emitter) = event_emitter {
                    Self::spawn_emitter_thread(event_rx, emitter);
                }

                return Ok(Self {
                    controller,
                    stream: output_stream,
                    sample_rate,
                    channels,
                });
            }
        };

        // Build input stream that feeds into the ringbuffer
        let input_stream = input_device
            .build_input_stream(
                &input_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    // Push input samples to ringbuffer for recording
                    for &sample in data {
                        let _ = input_tx.push(sample);
                    }
                },
                |err| eprintln!("Input stream error: {}", err),
                None,
            )
            .map_err(|e| e.to_string())?;

        // Start both streams
        output_stream.play().map_err(|e| e.to_string())?;
        input_stream.play().map_err(|e| e.to_string())?;

        // Leak the input stream to keep it alive
        Box::leak(Box::new(input_stream));

        // Spawn emitter thread if provided
        if let Some(emitter) = event_emitter {
            Self::spawn_emitter_thread(event_rx, emitter);
        }

        Ok(Self {
            controller,
            stream: output_stream,
            sample_rate,
            channels,
        })
    }

    /// Spawn a background thread to emit events from the ringbuffer
    fn spawn_emitter_thread(mut event_rx: rtrb::Consumer<AudioEvent>, emitter: std::sync::Arc<dyn EventEmitter>) {
        std::thread::spawn(move || {
            loop {
                // Wait for events and emit them
                if let Ok(event) = event_rx.pop() {
                    emitter.emit(event);
                } else {
                    // No events available, sleep briefly to avoid busy-waiting
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        });
    }
}
