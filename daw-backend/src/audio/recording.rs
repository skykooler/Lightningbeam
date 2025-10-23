/// Audio recording system for capturing microphone input
use crate::audio::{ClipId, TrackId};
use crate::io::WavWriter;
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
        }
    }

    /// Add samples to the accumulation buffer
    /// Returns true if a flush occurred
    pub fn add_samples(&mut self, samples: &[f32]) -> Result<bool, std::io::Error> {
        if self.paused {
            return Ok(false);
        }

        // Skip stale samples from the buffer
        if self.samples_to_skip > 0 {
            let to_skip = self.samples_to_skip.min(samples.len());
            self.samples_to_skip -= to_skip;

            if to_skip == samples.len() {
                // Skip entire batch
                return Ok(false);
            }

            // Skip partial batch and process the rest
            self.buffer.extend_from_slice(&samples[to_skip..]);
        } else {
            self.buffer.extend_from_slice(samples);
        }

        // Check if we should flush
        let frames_in_buffer = self.buffer.len() / self.channels as usize;
        if frames_in_buffer >= self.flush_interval_frames {
            self.flush()?;
            return Ok(true);
        }

        Ok(false)
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

    /// Finalize the recording and return the temp file path
    pub fn finalize(mut self) -> Result<PathBuf, std::io::Error> {
        // Flush any remaining samples
        self.flush()?;

        // Finalize the WAV file
        self.writer.finalize()?;

        Ok(self.temp_file_path)
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
