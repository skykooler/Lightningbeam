use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample};
use std::sync::{Arc, Mutex};
use crate::{TrackManager, Timestamp, Duration, SampleCount, AudioOutput, PlaybackState};
#[cfg(target_arch = "wasm32")]
use web_time::{Instant, Duration as StdDuration};
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Instant, Duration as StdDuration};
use std::sync::atomic::Ordering;
use std::sync::atomic::AtomicU32;
use std::cell::Cell;
use std::collections::VecDeque;

#[derive(PartialEq, Clone)]
enum AudioState {
  Suspended,
  Running,
}

const DELAY_HISTORY_SIZE: usize = 5;

#[derive(Default)]
struct StutterDetector {
  delay_history: Mutex<VecDeque<StdDuration>>,
  desired_buffer_size: AtomicU32,
  current_buffer_size: AtomicU32,
  max_buffer_size: AtomicU32,
  stutter_count: AtomicU32,
  max_stutter_count: AtomicU32,
  last_callback_time: Cell<Option<Instant>>,
  scheduling_threshold: AtomicU32,
}

pub struct CpalAudioOutput {
  track_manager: Option<Arc<Mutex<TrackManager>>>,
  _stream: Option<cpal::Stream>,
  playback_state: PlaybackState,
  audio_state: AudioState,
  timestamp: Arc<Mutex<Timestamp>>,
  chunk_size: usize,
  sample_rate: u32,
  stutter_detector: Arc<Mutex<StutterDetector>>,
  resize_sender: crossbeam_channel::Sender<()>,  // Or other channel implementation
  resize_receiver: crossbeam_channel::Receiver<()>,
}

impl StutterDetector {
  fn new() -> Self {
    Self {
      delay_history: Mutex::new(VecDeque::with_capacity(DELAY_HISTORY_SIZE)),
      desired_buffer_size: AtomicU32::new(256),
      current_buffer_size: AtomicU32::new(256),
      max_buffer_size: AtomicU32::new(8192),
      stutter_count: AtomicU32::new(0),
      max_stutter_count: AtomicU32::new(3),
      last_callback_time: Cell::new(None),
      scheduling_threshold: AtomicU32::new(1200), // 1.2 stored in fixed point
    }
  }
  pub fn reset(&mut self) {
    *self = Self::new();
  }
  fn get_scheduling_threshold(&self) -> f32 {
    self.scheduling_threshold.load(Ordering::Relaxed) as f32 / 1000.0
  }
}

impl CpalAudioOutput {
  pub fn new() -> Self {
    let (tx, rx) = crossbeam_channel::bounded(1);
    Self {
      track_manager: None,
      _stream: None,
      playback_state: PlaybackState::Stopped,
      audio_state: AudioState::Suspended,
      timestamp: Arc::new(Mutex::new(Timestamp::from_seconds(0.0))),
      chunk_size: 0,
      sample_rate: 44100,
      stutter_detector: Arc::new(Mutex::new(StutterDetector::new())),
      resize_sender: tx,
      resize_receiver: rx,
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
    let num_channels = supported_config.channels as usize;
    
    let stutter_detector = self.stutter_detector.clone();
    let resize_sender = self.resize_sender.clone();
    let sample_rate = self.sample_rate;
    
    let buffer_size_range = match config.buffer_size() {
      cpal::SupportedBufferSize::Range { min, max } => (*min, *max),
      cpal::SupportedBufferSize::Unknown => (256, 4096),
    };
    
    let detector_guard = self.stutter_detector.lock().unwrap();
    let desired_buffer_size = detector_guard.desired_buffer_size.load(Ordering::Relaxed);
    drop(detector_guard);
    
    let clamped_buffer_size = desired_buffer_size.clamp(buffer_size_range.0, buffer_size_range.1);
    let mut stream_config = supported_config.clone();
    stream_config.buffer_size = cpal::BufferSize::Fixed(clamped_buffer_size);
    
    log::info!("Starting stream with buffer size {}", clamped_buffer_size);
    
    let track_manager = self.track_manager.clone();
    let timestamp = self.timestamp.clone();
    
    let err_fn = |err| log::error!("Audio stream error: {:?}", err);
    
    let stream = device.build_output_stream(
      &stream_config,
      move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        // Timing measurement
        let processing_start = if cfg!(target_arch = "wasm32") {
          let perf = web_sys::window()
          .and_then(|w| w.performance())
          .expect("performance should be available");
          let now_ms = perf.now();
          Instant::now() + StdDuration::from_secs_f64(now_ms / 1000.0)
        } else {
          Instant::now()
        };
        
        // Initialize resize flag outside of lock scope
        let mut should_resize = false;
        let current_size;
        let buffer_duration;
        let scheduling_threshold;
        
        {
          let detector = stutter_detector.lock().unwrap();
          
          // Update detector state
          current_size = detector.current_buffer_size.load(Ordering::Relaxed);
          buffer_duration = StdDuration::from_secs_f64(current_size as f64 / sample_rate as f64);
          scheduling_threshold = detector.get_scheduling_threshold();
          
          // Calculate scheduling delay
          let last_time = detector.last_callback_time.get();
          // log::info!("Current size: {}", current_size);
          
          // Audio processing
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
              for channel in 0..num_channels {
                let index = i * num_channels + channel;
                if index < data.len() {
                  data[index] = sample;
                }
              }
            }
            
            *timestamp_guard += chunk_duration;
            
            // Stutter detection logic
            let processing_time = processing_start.elapsed();
            let processing_overrun = processing_time > buffer_duration;
            
            // Update delay history
            if let Some(last) = last_time {
              let interval = processing_start.duration_since(last);
              let mut history = detector.delay_history.lock().unwrap();
              if history.len() >= 5 {
                history.pop_front();
              }
              history.push_back(interval);
              // log::info!("Interval: {:?}", interval);
            }
            
            // Calculate average delay
            let avg_delay = {
              let history = detector.delay_history.lock().unwrap();
              if history.is_empty() {
                StdDuration::ZERO
              } else {
                history.iter().sum::<StdDuration>() / history.len() as u32
              }
            };
            
            // log::info!("Average delay: {:?}", avg_delay);
            
            // Determine stutter
            let stutter_detected = avg_delay > buffer_duration.mul_f32(scheduling_threshold)
            || processing_overrun;
            
            // Update stutter count with hysteresis
            let current_count = detector.stutter_count.load(Ordering::Relaxed);
            if stutter_detected {
              detector.stutter_count.store(
                (current_count + 1).min(detector.max_stutter_count.load(Ordering::Relaxed)),
                Ordering::Relaxed
              );
            } else {
              detector.stutter_count.store(
                current_count.saturating_sub(1),
                Ordering::Relaxed
              );
            }
            
            // Check for resize
            if detector.stutter_count.load(Ordering::Relaxed) >= detector.max_stutter_count.load(Ordering::Relaxed) {
              let desired_size = detector.desired_buffer_size.load(Ordering::Relaxed);
              let new_size = (desired_size * 2).min(detector.max_buffer_size.load(Ordering::Relaxed));
              
              if new_size != desired_size {
                detector.desired_buffer_size.store(new_size, Ordering::Relaxed);
                detector.stutter_count.store(0, Ordering::Relaxed);
                should_resize = true;
              }
            }
          }
          
          detector.last_callback_time.set(Some(processing_start));
        }
        
        // Send resize request outside of lock
        if should_resize {
          let _ = resize_sender.try_send(());
        }
      },
      err_fn,
      None,
    )?;
    
    // Update current buffer size after stream creation
    let detector = self.stutter_detector.lock().unwrap();
    detector.current_buffer_size.store(clamped_buffer_size, Ordering::Relaxed);
    
    Ok(stream)
  }
  
  fn recreate_stream(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    // Stop and destroy old stream first
    if let Some(old_stream) = self._stream.take() {
      old_stream.pause()?;
      // Explicitly drop the stream
      drop(old_stream);
    }
    
    // Add a small delay to ensure resources are freed (especially important in WASM)
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(std::time::Duration::from_millis(50));
    
    #[cfg(target_arch = "wasm32")]
    {
      use wasm_bindgen_futures::spawn_local;
      use gloo_timers::future::sleep;
      spawn_local(async {
        sleep(std::time::Duration::from_millis(50)).await;
      });
    }
    
    // Recreate stream with current configuration
    let host = cpal::default_host();
    let device = host.default_output_device()
    .ok_or_else(|| "No output device available")?;
    let supported_config = device.default_output_config()?;
    
    {
      let mut detector = self.stutter_detector.lock().unwrap();
      let desired_buffer_size = detector.desired_buffer_size.load(Ordering::Relaxed);
      detector.reset();
      detector.desired_buffer_size.store(desired_buffer_size, Ordering::Relaxed);
    }
    // let mut history = detector.delay_history.lock().unwrap();
    
    self._stream = Some(self.build_stream::<f32>(&device, supported_config)?);
    
    // Restart playback if needed
    if self.audio_state == AudioState::Running {
      self._stream.as_ref().unwrap().play()?;
    }
    
    Ok(())
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
    Ok(())
  }
  
  fn play(&mut self, start_timestamp: Timestamp) {
    self.timestamp.lock().unwrap().set(start_timestamp);
    self.playback_state = PlaybackState::Playing;
  }
  
  fn stop(&mut self) {
    self.playback_state = PlaybackState::Stopped;
  }
  
  fn resume(&mut self) -> Result<(), anyhow::Error> {
    if self.audio_state == AudioState::Suspended {
      if let Some(stream) = &self._stream {
        stream.play()?;
        self.audio_state = AudioState::Running;
        log::info!("Audio resumed");
      }
    }
    Ok(())
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
  fn check_resize(&mut self) -> Result<(), Box<dyn std::error::Error>> {
    // Process resize requests with timeout
    let timeout = StdDuration::from_millis(10);
    while let Ok(()) = self.resize_receiver.try_recv() {
      let start = Instant::now();
      
      // Try to lock, non-blocking
      {
        let detector = match self.stutter_detector.try_lock() {
          Ok(d) => d,
          Err(_) => {
            // Couldn't acquire lock immediately, skip this iteration
            return Ok(());
          }
        };
        
        // Quick check before heavy operation
        if detector.desired_buffer_size.load(Ordering::Relaxed) == detector.current_buffer_size.load(Ordering::Relaxed) {
          continue;
        }
        detector.current_buffer_size.store(detector.desired_buffer_size.load(Ordering::Relaxed), Ordering::Relaxed);
      }
      
      // Actual stream recreation
      log::info!("Restarting stream");
      let _ = self.recreate_stream()?;
      
      if Instant::now().duration_since(start) > timeout {
        break;
      }
    }
    Ok(())
  }
}