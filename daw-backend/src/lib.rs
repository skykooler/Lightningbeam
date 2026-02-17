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
    AudioClipInstanceId, AudioPool, AudioTrack, AutomationLane, AutomationLaneId, AutomationPoint, BufferPool, Clip, ClipId, CurveType, Engine, EngineController,
    Metatrack, MidiClip, MidiClipId, MidiClipInstance, MidiClipInstanceId, MidiEvent, MidiTrack, ParameterId, PoolAudioFile, Project, RecordingState, RenderContext, Track, TrackId,
    TrackNode,
};
pub use audio::node_graph::{GraphPreset, AudioGraph, PresetMetadata, SerializedConnection, SerializedNode};
pub use command::{AudioEvent, Command, OscilloscopeData};
pub use command::types::AutomationKeyframeData;
pub use io::{load_midi_file, AudioFile, WaveformChunk, WaveformChunkKey, WaveformPeak, WavWriter};

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
    /// Event receiver for polling audio events (only present when no EventEmitter is provided)
    pub event_rx: Option<rtrb::Consumer<AudioEvent>>,
    /// Consumer for recording audio mirror (streams recorded samples to UI for live waveform)
    recording_mirror_rx: Option<rtrb::Consumer<f32>>,
}

impl AudioSystem {
    /// Initialize the audio system with default input and output devices
    ///
    /// # Arguments
    /// * `event_emitter` - Optional event emitter for pushing events to external systems
    /// * `buffer_size` - Audio buffer size in frames (128, 256, 512, 1024, etc.)
    ///                   Smaller = lower latency but higher CPU usage. Default: 256
    ///
    /// # Environment Variables
    /// * `DAW_AUDIO_DEBUG=1` - Enable audio callback timing diagnostics. Logs:
    ///   - Device and config info at startup
    ///   - First 10 callback buffer sizes (to detect ALSA buffer variance)
    ///   - Per-overrun timing breakdown (command vs render time)
    ///   - Periodic (~5s) timing summaries (avg/worst/overrun rate)
    pub fn new(
        event_emitter: Option<std::sync::Arc<dyn EventEmitter>>,
        buffer_size: u32,
    ) -> Result<Self, String> {
        let host = cpal::default_host();

        // Get output device
        let output_device = host
            .default_output_device()
            .ok_or("No output device available")?;

        let default_output_config = output_device.default_output_config().map_err(|e| e.to_string())?;
        let sample_rate = default_output_config.sample_rate();
        let channels = default_output_config.channels() as u32;
        let debug_audio = std::env::var("DAW_AUDIO_DEBUG").map_or(false, |v| v == "1");

        eprintln!("[AUDIO] Device: {:?}, format={:?}, rate={}, channels={}",
            output_device.name().unwrap_or_default(), default_output_config.sample_format(), sample_rate, channels);

        // Create queues
        let (command_tx, command_rx) = rtrb::RingBuffer::new(512); // Larger buffer for MIDI + UI commands
        let (event_tx, event_rx) = rtrb::RingBuffer::new(256);
        let (query_tx, query_rx) = rtrb::RingBuffer::new(16); // Smaller buffer for synchronous queries
        let (query_response_tx, query_response_rx) = rtrb::RingBuffer::new(16);

        // Create input ringbuffer for recording (large buffer for audio samples)
        // Buffer size: 10 seconds of audio at 48kHz stereo = 48000 * 2 * 10 = 960000 samples
        let input_buffer_size = (sample_rate * channels * 10) as usize;
        let (mut input_tx, input_rx) = rtrb::RingBuffer::new(input_buffer_size);

        // Create mirror ringbuffer for streaming recorded audio to UI (live waveform)
        let (mirror_tx, mirror_rx) = rtrb::RingBuffer::new(input_buffer_size);

        // Create engine
        let mut engine = Engine::new(sample_rate, channels, command_rx, event_tx, query_rx, query_response_tx);
        engine.set_input_rx(input_rx);
        engine.set_recording_mirror_tx(mirror_tx);
        let controller = engine.get_controller(command_tx, query_tx, query_response_rx);

        // Initialize MIDI input manager for external MIDI devices
        // Create a separate command channel for MIDI input
        let (midi_command_tx, midi_command_rx) = rtrb::RingBuffer::new(256);
        match io::MidiInputManager::new(midi_command_tx) {
            Ok(midi_manager) => {
                println!("MIDI input initialized successfully");
                engine.set_midi_input_manager(midi_manager);
                engine.set_midi_command_rx(midi_command_rx);
            }
            Err(e) => {
                eprintln!("Warning: Failed to initialize MIDI input: {}", e);
                eprintln!("External MIDI controllers will not be available");
            }
        }

        // Build output stream
        let mut output_config: cpal::StreamConfig = default_output_config.into();

        // WASAPI shared mode on Windows does not support fixed buffer sizes.
        // Use the device default on Windows; honor the requested size on other platforms.
        if cfg!(target_os = "windows") {
            output_config.buffer_size = cpal::BufferSize::Default;
        } else {
            output_config.buffer_size = cpal::BufferSize::Fixed(buffer_size);
        }

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
            .map_err(|e| format!("Failed to build output stream: {e:?}"))?;

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
                    event_rx: None, // No event receiver when audio device unavailable
                    recording_mirror_rx: None,
                });
            }
        };

        // Get input config - use the input device's own default config
        let input_config = match input_device.default_input_config() {
            Ok(config) => {
                let cfg: cpal::StreamConfig = config.into();
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
                    event_rx: None,
                    recording_mirror_rx: None,
                });
            }
        };

        // Build input stream that feeds into the ringbuffer
        let input_stream = match input_device
            .build_input_stream(
                &input_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    for &sample in data {
                        let _ = input_tx.push(sample);
                    }
                },
                |err| eprintln!("Input stream error: {}", err),
                None,
            ) {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("Warning: Could not build input stream: {}, recording will be disabled", e);
                output_stream.play().map_err(|e| e.to_string())?;

                if let Some(emitter) = event_emitter {
                    Self::spawn_emitter_thread(event_rx, emitter);
                }

                return Ok(Self {
                    controller,
                    stream: output_stream,
                    sample_rate,
                    channels,
                    event_rx: None,
                    recording_mirror_rx: None,
                });
            }
        };

        // Start both streams
        output_stream.play().map_err(|e| e.to_string())?;
        input_stream.play().map_err(|e| e.to_string())?;

        // Leak the input stream to keep it alive
        Box::leak(Box::new(input_stream));

        // Spawn emitter thread if provided, or store event_rx for manual polling
        let event_rx_option = if let Some(emitter) = event_emitter {
            Self::spawn_emitter_thread(event_rx, emitter);
            None
        } else {
            Some(event_rx)
        };

        Ok(Self {
            controller,
            stream: output_stream,
            sample_rate,
            channels,
            event_rx: event_rx_option,
            recording_mirror_rx: Some(mirror_rx),
        })
    }

    /// Take the recording mirror consumer for streaming recorded audio to UI
    pub fn take_recording_mirror_rx(&mut self) -> Option<rtrb::Consumer<f32>> {
        self.recording_mirror_rx.take()
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
