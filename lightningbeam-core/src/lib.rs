mod time;
use time::{Timestamp, Duration, Frame};
mod audio;
use audio::{CpalAudioOutput};

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

pub trait AudioTrack {
  /// Render a chunk of audio for the given timestamp and duration.
  fn render_chunk(&self, timestamp: Timestamp, duration: Duration) -> Vec<f32>;

  /// Get the sample rate of the audio track.
  fn sample_rate(&self) -> u32;
}

pub trait VideoTrack {
  /// Render a frame for the given timestamp.
  fn render_frame(&self, timestamp: Timestamp) -> Frame;

  /// Get the frame rate of the video track.
  fn frame_rate(&self) -> f64;
}

pub struct TrackManager {
  audio_tracks: Vec<Box<dyn AudioTrack>>,
  video_tracks: Vec<Box<dyn VideoTrack>>,
  sample_rate: u32,
  frame_duration: Duration, // Duration of each frame in seconds (e.g., 1/60 for 60 FPS)
}

impl TrackManager {
  pub fn new(sample_rate: u32, frame_duration: f64) -> Self {
    Self {
      audio_tracks: Vec::new(),
      video_tracks: Vec::new(),
      sample_rate,
      frame_duration: Duration::new(frame_duration),
    }
  }
  
  pub fn add_audio_track(&mut self, track: Box<dyn AudioTrack>) {
    self.audio_tracks.push(track);
  }
  
  pub fn add_video_track(&mut self, track: Box<dyn VideoTrack>) {
    self.video_tracks.push(track);
  }
  
  pub fn play(&mut self, timestamp: Timestamp, audio_output: &mut dyn AudioOutput, video_output: &mut dyn FrameTarget) {
    let mut timestamp = timestamp.clone();
    
    // Main playback loop
    loop {
      // Render and play audio chunks
      let mut audio_mix: Vec<f32> = vec![0.0; self.frame_duration.to_samples(self.sample_rate) as usize];
      for track in &mut self.audio_tracks {
        let chunk = track.render_chunk(timestamp, self.frame_duration);
        for (i, sample) in chunk.iter().enumerate() {
          audio_mix[i] += sample; // Simple mixing (sum of samples)
        }
      }
      audio_output.play_chunk(audio_mix);
      
      // Render video frames
      for track in &self.video_tracks {
        let track_frame = track.render_frame(timestamp);
      }
      
      // Update timestamp
      timestamp += self.frame_duration;
      
      // Break condition (e.g., end of tracks)
      if self.audio_tracks.iter().all(|t| t.render_chunk(timestamp, self.frame_duration).is_empty()) {
        break;
      }
    }
  }
}

pub trait AudioOutput {
  fn start(&mut self) -> Result<(), Box<dyn std::error::Error>>;
  fn play_chunk(&mut self, chunk: Vec<f32>);
}

pub trait FrameTarget {
  fn draw(&mut self, frame: &[u8], width: u32, height: u32);
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

    Ok(())
}