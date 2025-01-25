mod time;
use time::{Timestamp, Duration, Frame, SampleCount};
mod audio;
use audio::{CpalAudioOutput};
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

use std::sync::{Arc, Mutex};
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
      console::log_1(&format!(
        "{} [{}:{}] {}",
        record.level(),
        record.file().unwrap_or("unknown"),
        record.line().unwrap_or(0),
        record.args()
      ).into());
    }
  }
  
  fn flush(&self) {}
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