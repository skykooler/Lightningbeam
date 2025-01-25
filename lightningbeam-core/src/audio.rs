use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample};
use std::sync::{Arc, Mutex};
use crate::{TrackManager, Timestamp, Duration, SampleCount, AudioOutput, PlaybackState};


// #[cfg(feature = "wasm")]
// use wasm_bindgen::prelude::*;

// #[cfg(feature="wasm")]
// #[wasm_bindgen]
pub struct CpalAudioOutput {
  track_manager: Option<Arc<Mutex<TrackManager>>>,
  _stream: Option<cpal::Stream>,
  playback_state: PlaybackState,
  timestamp: Arc<Mutex<Timestamp>>,
  chunk_size: usize,
  sample_rate: u32,
}

// #[cfg(feature="wasm")]
// #[wasm_bindgen]
impl CpalAudioOutput {
  pub fn new() -> Self {
    Self {
      track_manager: None,
      _stream: None,
      playback_state: PlaybackState::Stopped,
      timestamp: Arc::new(Mutex::new(Timestamp::from_seconds(0.0))),
      chunk_size: 0,
      sample_rate: 44100, // Default sample rate, updated later
    }
  }
  
  fn build_stream<T>(
    &mut self,
    device: &cpal::Device,
    config: cpal::SupportedStreamConfig,
) -> Result<cpal::Stream, anyhow::Error>
where
    T: Sample + From<f32> + cpal::SizedSample,
{
    let supported_config = config.config();
    self.sample_rate = supported_config.sample_rate.0;
    let num_channels = supported_config.channels as usize; // Get channel count

    let buffer_size_range = match config.buffer_size() {
      cpal::SupportedBufferSize::Range { min, max } => (*min, *max),
      cpal::SupportedBufferSize::Unknown => {
          // Use a reasonable default range if the device doesn't specify
          (256, 4096)
      }
  };

    // Define the desired buffer size and clamp it to the supported range
    let desired_buffer_size = 2048;
    let clamped_buffer_size = desired_buffer_size.clamp(buffer_size_range.0, buffer_size_range.1);

    let mut stream_config = supported_config.clone();
    stream_config.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);

    let track_manager = self.track_manager.clone();
    let timestamp = self.timestamp.clone();
    let sample_rate = self.sample_rate;

    let err_fn = |err| eprintln!("Audio stream error: {:?}", err);

    let stream = device.build_output_stream(
        &stream_config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if let Some(track_manager) = &track_manager {
                let num_frames = data.len() / num_channels; // Stereo: divide by 2
                let sample_count = SampleCount::new(num_frames);
                let chunk_duration = Duration::new(num_frames as f64 / sample_rate as f64);

                let mut track_manager = track_manager.lock().unwrap();

                let mut timestamp_guard = timestamp.lock().unwrap();
                let timestamp = &mut *timestamp_guard;

                let chunk = track_manager.update_audio(
                    timestamp.clone(),
                    sample_count,
                    sample_rate,
                );

                // Write samples (interleaved stereo)
                for (i, frame) in chunk.iter().enumerate() {
                    let sample = T::from(*frame);
                    data[i * num_channels] = sample; // Left channel
                    data[i * num_channels + 1] = sample; // Right channel (or process separately)
                }

                *timestamp_guard += chunk_duration;
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
  }
}

impl AudioOutput for CpalAudioOutput {
  fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
    .default_output_device()
    .ok_or_else(|| "No output device available")?;
    let supported_config = device.default_output_config()?;
    self._stream = Some(self.build_stream::<f32>(&device, supported_config)?);
    if let Some(stream) = self._stream.as_ref() {
      stream.play().unwrap();
    } else {
      eprintln!("Stream is not initialized!");
    }
    Ok(())
  }
  
  fn play(&mut self, start_timestamp: Timestamp) {
    self.timestamp.lock().unwrap().set(start_timestamp);
    self.playback_state = PlaybackState::Playing;
  }
  
  fn stop(&mut self) {
    self.playback_state = PlaybackState::Stopped;
  }
  
  fn register_track_manager(&mut self, track_manager: Arc<Mutex<TrackManager>>) {
    self.track_manager = Some(track_manager);
  }
  
  fn get_timestamp(&mut self) -> Timestamp {
    *self.timestamp.lock().unwrap()
  }
  fn set_chunk_size(&mut self, chunk_size: usize) {
    self.chunk_size = chunk_size
  }
}
