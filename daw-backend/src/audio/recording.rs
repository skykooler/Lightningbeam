/// Audio recording system for capturing microphone input
use crate::audio::{ClipId, TrackId};
use crate::io::{WavWriter, WaveformPeak};
use std::path::PathBuf;

/// State of an active recording session
pub struct RecordingState {
    /// Track being recorded to
    pub track_id: TrackId,
    /// Clip ID for the intermediate clip
    pub clip_id: ClipId,
    /// Path to temporary WAV file
    pub temp_file_path: PathBuf,
    /// WAV file writer
    pub writer: WavWriter,
    /// Sample rate of recording
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u32,
    /// Timeline start position in seconds
    pub start_time: f64,
    /// Total frames written to disk
    pub frames_written: usize,
    /// Accumulation buffer for next flush
    pub buffer: Vec<f32>,
    /// Number of frames to accumulate before flushing
    pub flush_interval_frames: usize,
    /// Whether recording is currently paused
    pub paused: bool,
    /// Number of samples remaining to skip (to discard stale buffer data)
    pub samples_to_skip: usize,
    /// Waveform peaks generated incrementally during recording
    pub waveform: Vec<WaveformPeak>,
    /// Temporary buffer for collecting samples for next waveform peak
    pub waveform_buffer: Vec<f32>,
    /// Number of frames per waveform peak
    pub frames_per_peak: usize,
    /// All recorded audio data accumulated in memory (for fast finalization)
    pub audio_data: Vec<f32>,
}

impl RecordingState {
    /// Create a new recording state
    pub fn new(
        track_id: TrackId,
        clip_id: ClipId,
        temp_file_path: PathBuf,
        writer: WavWriter,
        sample_rate: u32,
        channels: u32,
        start_time: f64,
        flush_interval_seconds: f64,
    ) -> Self {
        let flush_interval_frames = (sample_rate as f64 * flush_interval_seconds) as usize;

        // Calculate frames per waveform peak
        // Target ~300 peaks per second with minimum 1000 samples per peak
        let target_peaks_per_second = 300;
        let frames_per_peak = (sample_rate / target_peaks_per_second).max(1000) as usize;

        Self {
            track_id,
            clip_id,
            temp_file_path,
            writer,
            sample_rate,
            channels,
            start_time,
            frames_written: 0,
            buffer: Vec::new(),
            flush_interval_frames,
            paused: false,
            samples_to_skip: 0, // Will be set by engine when it knows buffer size
            waveform: Vec::new(),
            waveform_buffer: Vec::new(),
            frames_per_peak,
            audio_data: Vec::new(),
        }
    }

    /// Add samples to the accumulation buffer
    /// Returns true if a flush occurred
    pub fn add_samples(&mut self, samples: &[f32]) -> Result<bool, std::io::Error> {
        if self.paused {
            return Ok(false);
        }

        // Determine which samples to process
        let samples_to_process = if self.samples_to_skip > 0 {
            let to_skip = self.samples_to_skip.min(samples.len());
            self.samples_to_skip -= to_skip;

            if to_skip == samples.len() {
                // Skip entire batch
                return Ok(false);
            }

            // Skip partial batch and process the rest
            &samples[to_skip..]
        } else {
            samples
        };

        // Add to disk buffer
        self.buffer.extend_from_slice(samples_to_process);

        // Add to audio data (accumulate in memory for fast finalization)
        self.audio_data.extend_from_slice(samples_to_process);

        // Add to waveform buffer and generate peaks incrementally
        self.waveform_buffer.extend_from_slice(samples_to_process);
        self.generate_waveform_peaks();

        // Check if we should flush to disk
        let frames_in_buffer = self.buffer.len() / self.channels as usize;
        if frames_in_buffer >= self.flush_interval_frames {
            self.flush()?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Generate waveform peaks from accumulated samples
    /// This is called incrementally as samples arrive
    fn generate_waveform_peaks(&mut self) {
        let samples_per_peak = self.frames_per_peak * self.channels as usize;

        while self.waveform_buffer.len() >= samples_per_peak {
            let mut min = 0.0f32;
            let mut max = 0.0f32;

            // Scan all samples for this peak
            for sample in &self.waveform_buffer[..samples_per_peak] {
                min = min.min(*sample);
                max = max.max(*sample);
            }

            self.waveform.push(WaveformPeak { min, max });

            // Remove processed samples from waveform buffer
            self.waveform_buffer.drain(..samples_per_peak);
        }
    }

    /// Flush accumulated samples to disk
    pub fn flush(&mut self) -> Result<(), std::io::Error> {
        if self.buffer.is_empty() {
            return Ok(());
        }

        // Write to WAV file
        self.writer.write_samples(&self.buffer)?;

        // Update frames written
        let frames_flushed = self.buffer.len() / self.channels as usize;
        self.frames_written += frames_flushed;

        // Clear buffer
        self.buffer.clear();

        Ok(())
    }

    /// Get current recording duration in seconds
    /// Includes both flushed frames and buffered frames
    pub fn duration(&self) -> f64 {
        let buffered_frames = self.buffer.len() / self.channels as usize;
        let total_frames = self.frames_written + buffered_frames;
        total_frames as f64 / self.sample_rate as f64
    }

    /// Finalize the recording and return the temp file path, waveform, and audio data
    pub fn finalize(mut self) -> Result<(PathBuf, Vec<WaveformPeak>, Vec<f32>), std::io::Error> {
        // Flush any remaining samples to disk
        self.flush()?;

        // Generate final waveform peak from any remaining samples
        if !self.waveform_buffer.is_empty() {
            let mut min = 0.0f32;
            let mut max = 0.0f32;

            for sample in &self.waveform_buffer {
                min = min.min(*sample);
                max = max.max(*sample);
            }

            self.waveform.push(WaveformPeak { min, max });
        }

        // Finalize the WAV file
        self.writer.finalize()?;

        Ok((self.temp_file_path, self.waveform, self.audio_data))
    }

    /// Pause recording
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Resume recording
    pub fn resume(&mut self) {
        self.paused = false;
    }
}
