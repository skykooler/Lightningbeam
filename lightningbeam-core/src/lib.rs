mod time;
use time::{Timestamp, Duration, Frame, SampleCount};
mod audio;
use audio::{CpalAudioOutput};
use log::{Level, LevelFilter, Log, Metadata, Record, SetLoggerError};

use std::sync::{Arc, Mutex};

#[cfg(feature = "wasm")]
mod wasm_imports {
    pub use wasm_bindgen::prelude::*;
    pub use web_sys::console;
    use wasm_logger;
}
#[cfg(feature = "wasm")]
use wasm_imports::*;


pub trait AudioTrack: Send {
  /// Render a chunk of audio for the given timestamp and duration.
  fn render_chunk(&mut self, timestamp: Timestamp, duration: SampleCount) -> Vec<f32>;

  /// Get the sample rate of the audio track.
  fn sample_rate(&self) -> u32;
}

pub trait VideoTrack: Send {
  /// Render a frame for the given timestamp.
  fn render_frame(&self, timestamp: Timestamp) -> Frame;

  /// Get the frame rate of the video track.
  fn frame_rate(&self) -> f64;
}

pub struct TrackManager {
  audio_tracks: Vec<Box<dyn AudioTrack>>,
  video_tracks: Vec<Box<dyn VideoTrack>>,
  // sample_rate: u32,
  // frame_duration: Duration, // Duration of each frame in seconds (e.g., 1/60 for 60 FPS)
  timestamp: Timestamp,
  playback_state: PlaybackState,
}

impl TrackManager {
  pub fn new(sample_rate: u32, frame_duration: f64) -> Self {
    Self {
      audio_tracks: Vec::new(),
      video_tracks: Vec::new(),
      // sample_rate,
      // frame_duration: Duration::new(frame_duration),
      playback_state: PlaybackState::Stopped,
      timestamp: Timestamp::from_seconds(0.0),
    }
  }
  
  pub fn add_audio_track(&mut self, track: Box<dyn AudioTrack>) {
    self.audio_tracks.push(track);
  }
  
  pub fn add_video_track(&mut self, track: Box<dyn VideoTrack>) {
    self.video_tracks.push(track);
  }

  pub fn play(&mut self, start_timestamp: Timestamp) {
    self.timestamp = start_timestamp;
    self.playback_state = PlaybackState::Playing;
  }

  pub fn stop(&mut self) {
      self.playback_state = PlaybackState::Stopped;
  }
  
  // pub fn play(&mut self, timestamp: Timestamp, audio_output: &mut dyn AudioOutput, video_output: &mut dyn FrameTarget) {
  //   let mut timestamp = timestamp.clone();
    
  //   // Main playback loop
  //   loop {
  //     // Render and play audio chunks
  //     let mut audio_mix: Vec<f32> = vec![0.0; self.frame_duration.to_samples(self.sample_rate) as usize];
  //     for track in &mut self.audio_tracks {
  //       let chunk = track.render_chunk(timestamp, self.frame_duration);
  //       for (i, sample) in chunk.iter().enumerate() {
  //         audio_mix[i] += sample; // Simple mixing (sum of samples)
  //       }
  //     }
  //     audio_output.play_chunk(audio_mix);
      
  //     // Render video frames
  //     for track in &self.video_tracks {
  //       let track_frame = track.render_frame(timestamp);
  //     }
      
  //     // Update timestamp
  //     timestamp += self.frame_duration;
      
  //     // Break condition (e.g., end of tracks)
  //     if self.audio_tracks.iter().all(|t| t.render_chunk(timestamp, self.frame_duration).is_empty()) {
  //       break;
  //     }
  //   }
  // }
  pub fn update_audio(&mut self, timestamp: Timestamp, playing: bool, chunk_size: SampleCount, sample_rate: u32) -> Vec<f32> {
    let mut mixed_audio = vec![0.0; chunk_size.as_usize()];

    // TODO: render video
    for track in &mut self.audio_tracks {
      let track_audio = track.render_chunk(timestamp, chunk_size);
      for (i, sample) in track_audio.iter().enumerate() {
          mixed_audio[i] += *sample; // Simple mixing, add samples together
      }
    }

    mixed_audio
  }
}

pub trait AudioOutput {
  fn start(&mut self) -> Result<(), Box<dyn std::error::Error>>;
  fn play(&mut self, start_timestamp: Timestamp);
  fn stop(&mut self);
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
  sample_rate: u32,
}

impl SineWaveTrack {
  pub fn new(frequency: f32, sample_rate: u32) -> Self {
    Self {
      frequency,
      phase: 0.0,
      sample_rate,
    }
  }
}

impl AudioTrack for SineWaveTrack {
  fn render_chunk(&mut self, timestamp: Timestamp, chunk_size: SampleCount) -> Vec<f32> {
    let mut chunk = Vec::with_capacity(chunk_size.as_usize());
    let phase_increment = (2.0 * std::f32::consts::PI * self.frequency) / self.sample_rate as f32;

    for _ in 0..chunk_size.as_usize() {
        chunk.push((self.phase).sin());
        self.phase += phase_increment;
        if self.phase > 2.0 * std::f32::consts::PI {
            self.phase -= 2.0 * std::f32::consts::PI;
        }
    }

    chunk
  }
  fn sample_rate(&self) -> u32 {
    self.sample_rate
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
  pub fn new(sample_rate: u32, frame_duration: f64) -> Self {
    Self {
      track_manager: Arc::new(Mutex::new(TrackManager::new(sample_rate, frame_duration))),
      cpal_audio_output: Box::new(CpalAudioOutput::new())
    }
  }
  pub fn init(&mut self) {
    println!("Init CoreInterface");
    let track_manager_clone = self.track_manager.clone();
    self.cpal_audio_output.register_track_manager(track_manager_clone);
    self.cpal_audio_output.start();
  }
  pub fn play(&mut self, timestamp: f64) {
    // Lock the Mutex to get access to TrackManager
    let mut track_manager = self.track_manager.lock().unwrap();
    track_manager.play(Timestamp::new(timestamp));
  }
  pub fn get_timestamp(&mut self) -> f64 {
    self.cpal_audio_output.get_timestamp().as_seconds()
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