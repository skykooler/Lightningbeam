/// Audio recording system for capturing microphone input
use crate::audio::{ClipId, MidiClipId, TrackId};
use crate::io::{WavWriter, WaveformPeak};
use crate::time::{Beats, Seconds};
use std::collections::HashMap;
use std::path::PathBuf;

/// State of an active recording session
pub struct RecordingState {
    /// Track being recorded to
    pub track_id: TrackId,
    /// Clip ID for the intermediate clip
    pub clip_id: ClipId,
    /// Path to temporary WAV file
    pub temp_file_path: PathBuf,
    /// WAV file writer (only used at finalization, not during recording)
    pub writer: WavWriter,
    /// Sample rate of recording
    pub sample_rate: u32,
    /// Number of channels
    pub channels: u32,
    /// Timeline start position
    pub start_time: Beats,
    /// Total frames recorded
    pub frames_written: usize,
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
    /// All recorded audio data accumulated in memory (written to disk at finalization)
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
        start_time: Beats,
        _flush_interval_seconds: f64, // No longer used - kept for API compatibility
    ) -> Self {
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

        // Add to audio data (accumulate in memory - disk write happens at finalization only)
        self.audio_data.extend_from_slice(samples_to_process);

        // Add to waveform buffer and generate peaks incrementally
        self.waveform_buffer.extend_from_slice(samples_to_process);
        self.generate_waveform_peaks();

        // Track frames for duration calculation (no disk I/O in audio callback!)
        let frames_added = samples_to_process.len() / self.channels as usize;
        self.frames_written += frames_added;

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

    /// Get current recording duration
    pub fn duration(&self) -> Seconds {
        Seconds(self.frames_written as f64 / self.sample_rate as f64)
    }

    /// Finalize the recording and return the temp file path, waveform, and audio data
    pub fn finalize(mut self) -> Result<(PathBuf, Vec<WaveformPeak>, Vec<f32>), std::io::Error> {
        // Write all audio data to disk at once (outside audio callback - safe to do I/O)
        if !self.audio_data.is_empty() {
            self.writer.write_samples(&self.audio_data)?;
        }

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

/// Active MIDI note waiting for its noteOff event
#[derive(Debug, Clone)]
struct ActiveMidiNote {
    note: u8,
    velocity: u8,
    start_time: Beats,
}

/// State of an active MIDI recording session.
pub struct MidiRecordingState {
    pub track_id: TrackId,
    pub clip_id: MidiClipId,
    pub start_time: Beats,
    active_notes: HashMap<u8, ActiveMidiNote>,
    /// Completed notes: (time_offset, note, velocity, duration) — all times in beats
    pub completed_notes: Vec<(Beats, u8, u8, Beats)>,
}

impl MidiRecordingState {
    pub fn new(track_id: TrackId, clip_id: MidiClipId, start_time: Beats) -> Self {
        Self {
            track_id,
            clip_id,
            start_time,
            active_notes: HashMap::new(),
            completed_notes: Vec::new(),
        }
    }

    pub fn note_on(&mut self, note: u8, velocity: u8, absolute_time: Beats) {
        self.active_notes.insert(note, ActiveMidiNote { note, velocity, start_time: absolute_time });
    }

    pub fn note_off(&mut self, note: u8, absolute_time: Beats) {
        if let Some(active_note) = self.active_notes.remove(&note) {
            if absolute_time <= self.start_time {
                return;
            }
            let note_start = active_note.start_time.max(self.start_time);
            self.completed_notes.push((
                note_start - self.start_time,
                active_note.note,
                active_note.velocity,
                absolute_time - note_start,
            ));
        }
    }

    pub fn get_notes(&self) -> &[(Beats, u8, u8, Beats)] {
        &self.completed_notes
    }

    pub fn note_count(&self) -> usize {
        self.completed_notes.len()
    }

    /// Get all completed notes plus currently-held notes with a provisional duration.
    pub fn get_notes_with_active(&self, current_time: Beats) -> Vec<(Beats, u8, u8, Beats)> {
        let mut notes = self.completed_notes.clone();
        for active in self.active_notes.values() {
            let note_start = active.start_time.max(self.start_time);
            notes.push((
                note_start - self.start_time,
                active.note,
                active.velocity,
                (current_time - note_start).max(Beats::ZERO),
            ));
        }
        notes
    }

    pub fn active_note_numbers(&self) -> Vec<u8> {
        self.active_notes.keys().copied().collect()
    }

    pub fn close_active_notes(&mut self, end_time: Beats) {
        let active_notes: Vec<_> = self.active_notes.drain().collect();

        for (_note_num, active_note) in active_notes {
            let note_start = active_note.start_time.max(self.start_time);
            self.completed_notes.push((
                note_start - self.start_time,
                active_note.note,
                active_note.velocity,
                end_time - note_start,
            ));
        }
    }
}
