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
    /// Producer end of the input ring-buffer. Taken into the closure when the
    /// input stream is opened; `None` after `open_input_stream()` has been called.
    input_tx: Option<rtrb::Producer<f32>>,
    /// The live microphone/line-in stream. `None` until `open_input_stream()` is called.
    input_stream: Option<cpal::Stream>,
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
        let _debug_audio = std::env::var("DAW_AUDIO_DEBUG").map_or(false, |v| v == "1");

        eprintln!("[AUDIO] Device: {:?}, format={:?}, rate={}, channels={}",
            output_device.description().map(|d| d.name().to_string()).unwrap_or_default(), default_output_config.sample_format(), sample_rate, channels);

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

        // Start output stream
        output_stream.play().map_err(|e| e.to_string())?;

        // Spawn emitter thread if provided, or store event_rx for manual polling
        let event_rx_option = if let Some(emitter) = event_emitter {
            Self::spawn_emitter_thread(event_rx, emitter);
            None
        } else {
            Some(event_rx)
        };

        // Input stream is NOT opened here — call open_input_stream() when an
        // audio input track is actually selected, to avoid constant ALSA wakeups.
        Ok(Self {
            controller,
            stream: output_stream,
            sample_rate,
            channels,
            event_rx: event_rx_option,
            recording_mirror_rx: Some(mirror_rx),
            input_tx: Some(input_tx),
            input_stream: None,
        })
    }

    /// Take the recording mirror consumer for streaming recorded audio to UI
    pub fn take_recording_mirror_rx(&mut self) -> Option<rtrb::Consumer<f32>> {
        self.recording_mirror_rx.take()
    }

    /// Open the microphone/line-in input stream.
    ///
    /// Call this as soon as an audio input track is selected so the stream is
    /// ready before recording starts. The stream is opened with the same fixed
    /// buffer size as the output stream to avoid ALSA spinning at high callback
    /// rates with its tiny default buffer.
    ///
    /// No-ops if the stream is already open.
    pub fn open_input_stream(&mut self, buffer_size: u32) -> Result<(), String> {
        if self.input_stream.is_some() {
            return Ok(());
        }
        let mut input_tx = match self.input_tx.take() {
            Some(tx) => tx,
            None => return Err("Input ring-buffer already consumed".into()),
        };

        let host = cpal::default_host();
        let input_device = host.default_input_device()
            .ok_or("No input device available")?;

        let default_cfg = input_device.default_input_config()
            .map_err(|e| e.to_string())?;

        let mut input_config: cpal::StreamConfig = default_cfg.into();
        // Match the output buffer size so ALSA wakes up at the same rate as
        // the output thread — prevents the ~750 wakeups/sec that the default
        // 64-frame buffer causes.
        if !cfg!(target_os = "windows") {
            input_config.buffer_size = cpal::BufferSize::Fixed(buffer_size);
        }

        let input_sample_rate  = input_config.sample_rate;
        let input_channels     = input_config.channels as u32;
        let output_sample_rate = self.sample_rate;
        let output_channels    = self.channels;
        let needs_resample = input_sample_rate != output_sample_rate
            || input_channels != output_channels;

        if needs_resample {
            eprintln!("[AUDIO] Input: {}Hz {}ch → resampling to {}Hz {}ch",
                input_sample_rate, input_channels, output_sample_rate, output_channels);
        }

        let stream = input_device.build_input_stream(
            &input_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !needs_resample {
                    for &s in data { let _ = input_tx.push(s); }
                } else {
                    let in_ch  = input_channels as usize;
                    let out_ch = output_channels as usize;
                    let ratio  = output_sample_rate as f64 / input_sample_rate as f64;
                    let in_frames  = data.len() / in_ch;
                    let out_frames = (in_frames as f64 * ratio) as usize;
                    for i in 0..out_frames {
                        let src_pos = i as f64 / ratio;
                        let src_idx = src_pos as usize;
                        let frac    = (src_pos - src_idx as f64) as f32;
                        for ch in 0..out_ch {
                            let ic = ch.min(in_ch - 1);
                            let s0 = data.get(src_idx * in_ch + ic).copied().unwrap_or(0.0);
                            let s1 = data.get((src_idx + 1) * in_ch + ic).copied().unwrap_or(s0);
                            let _ = input_tx.push(s0 + frac * (s1 - s0));
                        }
                    }
                }
            },
            |err| eprintln!("Input stream error: {err}"),
            None,
        ).map_err(|e| format!("Failed to build input stream: {e}"))?;

        stream.play().map_err(|e| e.to_string())?;
        self.input_stream = Some(stream);
        Ok(())
    }

    /// Close the input stream (e.g. when the last audio input track is removed).
    pub fn close_input_stream(&mut self) {
        self.input_stream = None; // Drop stops the stream
    }

    /// Extract an [`InputStreamOpener`] that can be stored independently and
    /// used to open the microphone/line-in stream on demand.
    /// Returns `None` if called a second time.
    pub fn take_input_opener(&mut self) -> Option<InputStreamOpener> {
        self.input_tx.take().map(|tx| InputStreamOpener {
            input_tx:    tx,
            sample_rate: self.sample_rate,
            channels:    self.channels,
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

/// Self-contained handle for opening the microphone/line-in stream on demand.
///
/// Obtained via [`AudioSystem::take_input_opener`]. Call [`open`](Self::open)
/// when the user selects an audio input track; store the returned
/// `cpal::Stream` to keep it alive (dropping it stops the stream).
pub struct InputStreamOpener {
    input_tx:    rtrb::Producer<f32>,
    sample_rate: u32,
    channels:    u32,
}

impl InputStreamOpener {
    /// Open and start the input stream with the given buffer size.
    ///
    /// Uses the same `buffer_size` as the output stream so ALSA wakes up at
    /// the same rate (~187/s at 256 frames) rather than the ~750/s it defaults
    /// to with 64-frame buffers.
    pub fn open(mut self, buffer_size: u32) -> Result<cpal::Stream, String> {
        let host = cpal::default_host();
        let device = host.default_input_device()
            .ok_or("No input device available")?;

        let default_cfg = device.default_input_config()
            .map_err(|e| e.to_string())?;

        let mut cfg: cpal::StreamConfig = default_cfg.into();
        if !cfg!(target_os = "windows") {
            cfg.buffer_size = cpal::BufferSize::Fixed(buffer_size);
        }

        let in_rate = cfg.sample_rate;
        let in_ch   = cfg.channels as u32;
        let out_rate = self.sample_rate;
        let out_ch   = self.channels;
        let needs_resample = in_rate != out_rate || in_ch != out_ch;

        if needs_resample {
            eprintln!("[AUDIO] Input: {}Hz {}ch → resampling to {}Hz {}ch",
                in_rate, in_ch, out_rate, out_ch);
        }

        let stream = device.build_input_stream(
            &cfg,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if !needs_resample {
                    for &s in data { let _ = self.input_tx.push(s); }
                } else {
                    let ic  = in_ch as usize;
                    let oc  = out_ch as usize;
                    let ratio = out_rate as f64 / in_rate as f64;
                    let in_frames  = data.len() / ic;
                    let out_frames = (in_frames as f64 * ratio) as usize;
                    for i in 0..out_frames {
                        let src = i as f64 / ratio;
                        let si  = src as usize;
                        let f   = (src - si as f64) as f32;
                        for ch in 0..oc {
                            let ich = ch.min(ic - 1);
                            let s0 = data.get(si * ic + ich).copied().unwrap_or(0.0);
                            let s1 = data.get((si + 1) * ic + ich).copied().unwrap_or(s0);
                            let _ = self.input_tx.push(s0 + f * (s1 - s0));
                        }
                    }
                }
            },
            |err| eprintln!("Input stream error: {err}"),
            None,
        ).map_err(|e| format!("Failed to build input stream: {e}"))?;

        stream.play().map_err(|e| e.to_string())?;
        Ok(stream)
    }
}
