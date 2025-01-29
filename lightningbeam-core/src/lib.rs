mod time;
use time::{Timestamp, Duration, Frame, SampleCount};
mod audio;
use audio::{CpalAudioOutput};
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};
use rubato::{FftFixedIn, Resampler};
use std::io::Cursor;

use std::sync::{Arc, Mutex};
use std::error::Error;

use symphonia::core::{
  audio::AudioBufferRef,
  audio::Signal,
  codecs::{DecoderOptions},
  formats::{FormatOptions},
  io::MediaSourceStream,
  meta::MetadataOptions,
  probe::Hint,
};

#[cfg(not(target_arch = "wasm32"))]
use std::io;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Write;


#[cfg(target_arch = "wasm32")]
use std::fmt;

#[cfg(feature = "wasm")]
mod wasm_imports {
  pub use wasm_bindgen::prelude::*;
  pub use web_sys::console;
}
#[cfg(feature = "wasm")]
use wasm_imports::*;


pub trait Track: Send {
  fn get_name(&self) -> &str {
    "Unnamed Track"
  }
  
  fn set_name(&mut self, _name: String) {
  }
  /// Render audio for the given timestamp and duration.
  /// Returns `None` if this track doesn't produce audio.
  fn render_audio(&mut self, _timestamp: Timestamp, _duration: SampleCount, _sample_rate: u32, _playing: bool) -> Option<Vec<f32>> {
    None
  }
  
  /// Render a video frame for the given timestamp.
  /// Returns `None` if this track doesn't produce video.
  fn render_video(&self, _timestamp: Timestamp, _playing: bool) -> Option<Frame> {
    None
  }
  
  // Set the sample rate of any audio this track might contain
  fn set_sample_rate(&mut self, _sample_rate: u32) {
  }
}

pub struct TrackManager {
  tracks: Vec<Box<dyn Track>>,
  timestamp: Timestamp,
  playback_state: PlaybackState,
}

impl TrackManager {
  pub fn new() -> Self {
    Self {
      tracks: Vec::new(),
      timestamp: Timestamp::from_seconds(0.0),
      playback_state: PlaybackState::Stopped,
    }
  }
  
  pub fn add_track(&mut self, track: Box<dyn Track>) {
    self.tracks.push(track);
  }
  
  pub fn update_audio(&mut self, timestamp: Timestamp, chunk_size: SampleCount, sample_rate: u32) -> Vec<f32> {
    
    let mut mixed = vec![0.0; chunk_size.as_usize()];
    let playing = matches!(self.playback_state, PlaybackState::Playing);
    
    for track in &mut self.tracks {
      if let Some(samples) = track.render_audio(timestamp, chunk_size, sample_rate, playing) {
        for (i, sample) in samples.iter().enumerate() {
          mixed[i] += *sample;
        }
      }
    }
    
    mixed
  }
  
  pub fn update_video(&self, timestamp: Timestamp) -> Vec<Frame> {
    let playing = matches!(self.playback_state, PlaybackState::Playing);
    self.tracks
    .iter()
    .filter_map(|track| track.render_video(timestamp, playing))
    .collect()
  }
  
  pub fn play(&mut self, start_timestamp: Timestamp) {
    self.timestamp = start_timestamp;
    self.playback_state = PlaybackState::Playing;
  }
  pub fn stop(&mut self) {
    self.playback_state = PlaybackState::Stopped;
  }
  pub fn get_tracks(&self) -> &Vec<Box<dyn Track>> {
    &self.tracks
  }
}

pub trait AudioOutput {
  fn start(&mut self) -> Result<(), Box<dyn std::error::Error>>;
  fn play(&mut self, start_timestamp: Timestamp);
  fn stop(&mut self);
  fn resume(&mut self) -> Result<(), anyhow::Error>;
  fn register_track_manager(&mut self, track_manager: Arc<Mutex<TrackManager>>);
  fn get_timestamp(&mut self) -> Timestamp;
  fn set_chunk_size(&mut self, chunk_size: usize);
}

pub trait FrameTarget {
  fn draw(&mut self, frame: &[u8], width: u32, height: u32);
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
enum PlaybackState {
  Playing,
  Stopped,
}

pub struct SineWaveTrack {
  frequency: f32,
  phase: f32,
  name: String,
}

impl SineWaveTrack {
  pub fn new(frequency: f32) -> Self {
    Self {
      frequency,
      phase: 0.0,
      name: "Sine Wave Track".to_string(),
    }
  }
}

impl Track for SineWaveTrack {
  fn get_name(&self) -> &str {
    &self.name
  }
  fn set_name(&mut self, name: String) {
    self.name = name;
  }
  fn render_audio(&mut self, _timestamp: Timestamp, chunk_size: SampleCount, sample_rate: u32, playing: bool) -> Option<Vec<f32>> {
    let mut chunk = Vec::with_capacity(chunk_size.as_usize());
    let phase_increment = (2.0 * std::f32::consts::PI * self.frequency) / sample_rate as f32;
    
    for _ in 0..chunk_size.as_usize() {
      if playing {
        chunk.push((self.phase).sin()*0.25);
      } else {
        chunk.push(0.0);
      }
      self.phase += phase_increment;
      if self.phase > 2.0 * std::f32::consts::PI {
        self.phase -= 2.0 * std::f32::consts::PI;
      }
    }
    
    Some(chunk)
  }
}

#[derive(Debug, Clone)]
struct AudioBuffer {
  original_data: Vec<f32>,
  original_sample_rate: u32,
  resampled_data: Vec<f32>,
  start_time: Timestamp,
}

impl AudioBuffer {
  fn duration(&self) -> Duration {
    if self.resampled_data.is_empty() {
      Duration::from_seconds(0.0)
    } else {
      Duration::from_seconds(
        self.resampled_data.len() as f64 / 
        self.original_sample_rate as f64
      )
    }
  }
}


pub struct RecordedAudioTrack {
  name: String,
  buffers: Vec<AudioBuffer>,
  target_sample_rate: Option<u32>,
}


impl RecordedAudioTrack {
  pub fn new(name: &str) -> Self {
    Self {
      name: name.to_string(),
      buffers: Vec::new(),
      target_sample_rate: None,
    }
  }
  
  pub fn add_buffer(&mut self, start_time: Timestamp, sample_rate: u32, data: Vec<f32>) {
    let resampled_data = match self.target_sample_rate {
      Some(target_rate) if sample_rate != target_rate => 
      self::resample(&data, sample_rate, target_rate),
      Some(_target_rate) => 
      data.clone(), // Already at target rate
      None => 
      Vec::new(), // Will be resampled later
    };
    
    self.buffers.push(AudioBuffer {
      original_data: data,
      original_sample_rate: sample_rate,
      resampled_data,
      start_time,
    });
    
    // Keep buffers sorted by start time
    self.buffers.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
  }
}

impl Track for RecordedAudioTrack {
  fn get_name(&self) -> &str {
    &self.name
  }
  fn set_sample_rate(&mut self, target_rate: u32) {
    self.target_sample_rate = Some(target_rate);
    
    for buffer in &mut self.buffers {
      if buffer.original_sample_rate == target_rate {
        buffer.resampled_data = buffer.original_data.clone();
      } else {
        buffer.resampled_data = self::resample(
          &buffer.original_data,
          buffer.original_sample_rate,
          target_rate
        );
      }
    }
  }
  
  fn render_audio(
    &mut self,
    timestamp: Timestamp,
    duration: SampleCount,
    sample_rate: u32,
    playing: bool,
  ) -> Option<Vec<f32>> {
    if !playing || self.target_sample_rate != Some(sample_rate) {
      return Some(vec![0.0; duration.as_usize()]);
    }
    
    // let chunk_samples = duration.as_usize();
    let mut output = vec![0.0; duration.as_usize()];
    let mut remaining_samples = duration;
    let mut current_time = timestamp;
    
    // Find the first buffer that overlaps with the requested time
    let mut buffer_index = match self.buffers.binary_search_by(|b| {
      b.start_time.partial_cmp(&current_time).unwrap()
    }) {
      Ok(i) => i,
      Err(i) if i > 0 => i - 1, // Check previous buffer if timestamp is between buffers
      _ => 0,
    };
    
    while remaining_samples.as_usize() > 0 && buffer_index < self.buffers.len() {
      let buffer = &self.buffers[buffer_index];
      
      // Calculate overlap with current buffer
      let buffer_start = buffer.start_time;
      let buffer_end = buffer_start + buffer.duration();
      
      if current_time >= buffer_end {
        // Move to next buffer
        buffer_index += 1;
        continue;
      }
      
      // Calculate how many samples we can take from this buffer
      let buffer_offset = ((current_time - buffer_start).as_seconds() * sample_rate as f64) as usize;
      let available_samples = SampleCount::new(buffer.resampled_data.len().saturating_sub(buffer_offset));
      let samples_to_take = remaining_samples.min(available_samples);
      
      if samples_to_take == 0 {
        // No more samples in this buffer
        buffer_index += 1;
        continue;
      }
      
      // Copy samples from buffer to output
      let output_offset = duration - remaining_samples;
      output[output_offset.as_usize()..(output_offset + samples_to_take).as_usize()]
      .copy_from_slice(&buffer.resampled_data[buffer_offset..buffer_offset + samples_to_take.as_usize()]);
      
      // Update state
      remaining_samples -= samples_to_take;
      current_time += samples_to_take.to_duration(sample_rate);
    }
    
    Some(output)
  }
}

fn resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
  if input_rate == output_rate {
    return input.to_vec();
  }
  
  let input_rate = input_rate.try_into().unwrap();
  let output_rate = output_rate.try_into().unwrap();
  let chunk_size = input.len();
  
  let mut resampler = FftFixedIn::new(
    output_rate,
    input_rate,
    chunk_size,
    1, // channel count
    2, // fft size
  ).unwrap();
  
  let output = resampler.process(&[input], None).unwrap();
  output[0].clone()
}

pub trait AudioLoader {
  fn load_audio(
    &self,
    track: &mut RecordedAudioTrack,
    start_time: Timestamp,
    audio_data: &[u8],
  ) -> Result<(), Box<dyn Error>>;
}

pub struct GenericAudioLoader;

impl AudioLoader for GenericAudioLoader {
  fn load_audio(
    &self,
    track: &mut RecordedAudioTrack,
    start_time: Timestamp,
    audio_data: &[u8],
  ) -> Result<(), Box<dyn Error>> {
    decode_audio(track, start_time, audio_data)
  }
}

fn decode_audio(
  track: &mut RecordedAudioTrack,
  start_time: Timestamp,
  audio_data: &[u8],
) -> Result<(), Box<dyn Error>> {
  // Create a media source from the byte slice
  let mss = MediaSourceStream::new(
      Box::new(Cursor::new(audio_data.to_vec())),
      Default::default(),
  );

  // Use a fresh hint (no extension specified) for format detection
  let hint = Hint::new();

  // Probe the media source for a supported format
  let probed = symphonia::default::get_probe()
      .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())?;

  // Get the format reader
  let mut format = probed.format;

  // Find the first supported audio track
  let default_track = format
      .tracks()
      .iter()
      .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
      .ok_or("No supported audio track found")?;

  // Create a decoder for the track
  let mut decoder = symphonia::default::get_codecs()
      .make(&default_track.codec_params, &DecoderOptions::default())?;

  // Get the sample rate from the track
  let sample_rate = default_track.codec_params.sample_rate.ok_or("Unknown sample rate")?;
  let mut decoded_samples = Vec::new();

  // Decode loop
  loop {
      let packet = match format.next_packet() {
          Ok(packet) => packet,
          Err(_) => break, // End of stream
      };

      match decoder.decode(&packet)? {
          AudioBufferRef::F32(buf) => {
              for i in 0..buf.frames() {
                  for c in 0..buf.spec().channels.count() {
                      decoded_samples.push(buf.chan(c)[i]);
                  }
              }
          }
          AudioBufferRef::S16(buf) => {
              for i in 0..buf.frames() {
                  for c in 0..buf.spec().channels.count() {
                      decoded_samples.push(buf.chan(c)[i] as f32 / 32768.0);
                  }
              }
          }
          _ => return Err("Unsupported audio format".into()),
      }
  }

  // Add the decoded audio to the track
  track.add_buffer(start_time, sample_rate, decoded_samples);

  Ok(())
}

#[cfg(feature="wasm")]
#[wasm_bindgen]
pub struct JsTrack {
  name: String,
}

#[cfg(feature="wasm")]
#[wasm_bindgen]
impl JsTrack {
  #[wasm_bindgen(getter)]
  pub fn name(&self) -> String {
    self.name.clone()
  }
}
#[cfg(feature="wasm")]
impl fmt::Display for JsTrack {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "JsTrack {{ name: {} }}", self.name)
  }
}

#[cfg(feature="wasm")]
#[wasm_bindgen]
impl JsTrack {
  #[wasm_bindgen(js_name = toString)]
  pub fn to_string(&self) -> String {
    format!("{}", self) // Calls the Display implementation
  }
}

#[cfg(feature="wasm")]
#[wasm_bindgen]
pub struct CoreInterface {
  #[wasm_bindgen(skip)]
  track_manager: Arc<Mutex<TrackManager>>,
  #[wasm_bindgen(skip)]
  cpal_audio_output: Box<dyn AudioOutput>,
}

#[cfg(feature="wasm")]
#[wasm_bindgen]
impl CoreInterface {
  #[wasm_bindgen(constructor)]
  pub fn new() -> Self {
    Self {
      track_manager: Arc::new(Mutex::new(TrackManager::new())),
      cpal_audio_output: Box::new(CpalAudioOutput::new())
    }
  }
  pub fn init(&mut self) {
    println!("Init CoreInterface");
    let track_manager_clone = self.track_manager.clone();
    self.cpal_audio_output.register_track_manager(track_manager_clone);
    let _ = self.cpal_audio_output.start();
  }
  pub fn play(&mut self, timestamp: f64) {
    // Lock the Mutex to get access to TrackManager
    let mut track_manager = self.track_manager.lock().unwrap();
    track_manager.play(Timestamp::new(timestamp));
  }
  pub fn stop(&mut self) {
    // Lock the Mutex to get access to TrackManager
    let mut track_manager = self.track_manager.lock().unwrap();
    track_manager.stop();
  }
  pub fn resume_audio(&mut self) -> Result<(), JsValue> {
    // Call this on user gestures if audio gets suspended
    self.cpal_audio_output.resume()
    .map_err(|e| JsValue::from_str(&format!("Failed to resume audio: {}", e)))
  }
  pub fn add_sine_track(&mut self, frequency: f32) -> Result<(), String> {
    if frequency.is_nan() || frequency.is_infinite() || frequency <= 0.0 {
      return Err(format!("Invalid frequency: {}", frequency));
    }
    log::info!("Freq: {}", frequency);
    let mut track_manager = self.track_manager.lock().unwrap();
    let sine_track = SineWaveTrack::new(frequency);
    track_manager.add_track(Box::new(sine_track));
    
    Ok(())
  }
  
  pub fn get_timestamp(&mut self) -> f64 {
    self.cpal_audio_output.get_timestamp().as_seconds()
  }
  pub fn get_tracks(&mut self) -> Vec<JsTrack> {
    let track_manager = self.track_manager.lock().unwrap();
    let tracks = track_manager.get_tracks();
    tracks
    .iter()
    .map(|track| JsTrack {
      name: track.get_name().to_string(),
    })
    .collect()
  }
}


struct PlainTextLogger;

impl Log for PlainTextLogger {
  fn enabled(&self, metadata: &Metadata) -> bool {
    metadata.level() <= Level::Info
  }
  
  fn log(&self, record: &Record) {
    if self.enabled(record.metadata()) {
      #[cfg(target_arch = "wasm32")]
      {
        // WASM: Log to the JS console
        console::log_1(
          &format!(
            "{} [{}:{}] {}",
            record.level(),
            record.file().unwrap_or("unknown"),
            record.line().unwrap_or(0),
            record.args()
          )
          .into(),
        );
      }
      
      #[cfg(not(target_arch = "wasm32"))]
      {
        // Native: Log to stderr
        let _ = writeln!(
          io::stderr(),
          "{} [{}:{}] {}",
          record.level(),
          record.file().unwrap_or("unknown"),
          record.line().unwrap_or(0),
          record.args()
        );
      }
    }
  }
  
  fn flush(&self) {
    #[cfg(not(target_arch = "wasm32"))]
    {
      let _ = io::stderr().flush();
    }
  }
}

pub fn init_plain_text_logger() -> Result<(), SetLoggerError> {
  log::set_boxed_logger(Box::new(PlainTextLogger))?;
  log::set_max_level(LevelFilter::Info);
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  
  // #[test]
  // fn it_works() {
  //     let result = add(2, 2);
  //     assert_eq!(result, 4);
  // }
}

// This is like the `main` function, except for JavaScript.
#[cfg(feature="wasm")]
#[wasm_bindgen(start)]
pub fn main_js() -> Result<(), JsValue> {
  // This provides better error messages in debug mode.
  // It's disabled in release mode so it doesn't bloat up the file size.
  #[cfg(debug_assertions)]
  console_error_panic_hook::set_once();
  init_plain_text_logger().expect("Failed to initialize plain text logger");
  
  
  log::info!("Logger initialized!");
  
  Ok(())
}