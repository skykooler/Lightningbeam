use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SampleRate, Stream, StreamConfig};
use cpal::{BufferSize, SupportedBufferSize};
use std::sync::{Arc, Mutex};
use crate::AudioOutput;

pub struct CpalAudioOutput {
  stream: Option<Stream>,
  buffer: Arc<Mutex<Vec<f32>>>, // Shared buffer for audio chunks
  sample_rate: u32,
  channels: u16,
}

impl CpalAudioOutput {
  pub fn new(sample_rate: u32, channels: u16) -> Self {
    Self {
      stream: None,
      buffer: Arc::new(Mutex::new(Vec::new())),
      sample_rate,
      channels,
    }
  }
}

impl AudioOutput for CpalAudioOutput {
  fn start(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let device = host
    .default_output_device()
    .ok_or("No output device available")?;
    let supported_config = device
    .default_output_config().unwrap();
    // .with_sample_rate(SampleRate(self.sample_rate));
    let config = StreamConfig {
      channels: self.channels,
      sample_rate: SampleRate(self.sample_rate),
      buffer_size: match supported_config.buffer_size() {
        SupportedBufferSize::Range { min, max: _ } => BufferSize::Fixed(*min),
        SupportedBufferSize::Unknown => BufferSize::Default,
      },
    };
    
    let buffer = self.buffer.clone();
    let sample_format = supported_config.sample_format();
    
    let stream = match sample_format {
      SampleFormat::F32 => device.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
          let mut buffer = buffer.lock().unwrap();
          for (out_sample, buffer_sample) in data.iter_mut().zip(buffer.iter()) {
            *out_sample = *buffer_sample;
          }
          buffer.clear(); // Clear buffer after playback
        },
        move |err| {
          eprintln!("Audio stream error: {:?}", err);
        },
      ),
      SampleFormat::I16 => device.build_output_stream(
        &config,
        move |data: &mut [i16], _| {
          let mut buffer = buffer.lock().unwrap();
          for (out_sample, buffer_sample) in data.iter_mut().zip(buffer.iter()) {
            *out_sample = (*buffer_sample * i16::MAX as f32) as i16;
          }
          buffer.clear();
        },
        move |err| {
          eprintln!("Audio stream error: {:?}", err);
        },
      ),
      SampleFormat::U16 => device.build_output_stream(
        &config,
        move |data: &mut [u16], _| {
          let mut buffer = buffer.lock().unwrap();
          for (out_sample, buffer_sample) in data.iter_mut().zip(buffer.iter()) {
            *out_sample = ((*buffer_sample + 1.0) * 0.5 * u16::MAX as f32) as u16;
          }
          buffer.clear();
        },
        move |err| {
          eprintln!("Audio stream error: {:?}", err);
        },
      ),
    };
    
    // If the stream creation failed, return the error
    let stream = stream.map_err(|e| {
      format!(
        "Failed to build output stream for sample format {:?}: {:?}",
        sample_format, e
      )
    })?;
    
    stream.play()?;
    self.stream = Some(stream);
    Ok(())
  }

  fn play_chunk(&mut self, chunk: Vec<f32>) {
    let mut buffer = self.buffer.lock().unwrap();
    buffer.extend(chunk);
  }
}    