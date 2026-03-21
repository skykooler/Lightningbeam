/// Audio recording system for capturing microphone input
use crate::audio::{ClipId, MidiClipId, TrackId};
use crate::io::{WavWriter, WaveformPeak};
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
    /// Timeline start position in seconds
    pub start_time: f64,
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
        start_time: f64,
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

    /// Get current recording duration in seconds
    pub fn duration(&self) -> f64 {
        self.frames_written as f64 / self.sample_rate as f64
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
    /// MIDI note number (0-127)
    note: u8,
    /// Velocity (0-127)
    velocity: u8,
    /// Absolute time when note started (seconds)
    start_time: f64,
}

/// State of an active MIDI recording session
pub struct MidiRecordingState {
    /// Track being recorded to
    pub track_id: TrackId,
    /// MIDI clip ID
    pub clip_id: MidiClipId,
    /// Timeline start position in seconds
    pub start_time: f64,
    /// Currently active notes (noteOn without matching noteOff)
    /// Maps note number to ActiveMidiNote
    active_notes: HashMap<u8, ActiveMidiNote>,
    /// Completed notes ready to be added to clip
    /// Format: (time_offset, note, velocity, duration)
    pub completed_notes: Vec<(f64, u8, u8, f64)>,
}

impl MidiRecordingState {
    /// Create a new MIDI recording state
    pub fn new(track_id: TrackId, clip_id: MidiClipId, start_time: f64) -> Self {
        Self {
            track_id,
            clip_id,
            start_time,
            active_notes: HashMap::new(),
            completed_notes: Vec::new(),
        }
    }

    /// Handle a MIDI note on event
    pub fn note_on(&mut self, note: u8, velocity: u8, absolute_time: f64) {
        self.active_notes.insert(note, ActiveMidiNote {
            note,
            velocity,
            start_time: absolute_time,
        });
    }

    /// Handle a MIDI note off event
    pub fn note_off(&mut self, note: u8, absolute_time: f64) {
        // Find the matching noteOn
        if let Some(active_note) = self.active_notes.remove(&note) {
            // If the note was fully released before the recording start (e.g. during count-in
            // pre-roll), discard it — only notes still held at the clip start are kept.
            if absolute_time <= self.start_time {
                return;
            }

            // Clamp note start to clip start: notes held across the recording boundary
            // are treated as starting at the clip position.
            let note_start = active_note.start_time.max(self.start_time);
            let time_offset = note_start - self.start_time;
            let duration = absolute_time - note_start;

            eprintln!("[MIDI_RECORDING_STATE] Completing note {}: note_start={:.3}s, note_end={:.3}s, recording_start={:.3}s, time_offset={:.3}s, duration={:.3}s",
                      note, note_start, absolute_time, self.start_time, time_offset, duration);

            self.completed_notes.push((
                time_offset,
                active_note.note,
                active_note.velocity,
                duration,
            ));
        }
        // If no matching noteOn found, ignore the noteOff
    }

    /// Get all completed notes
    pub fn get_notes(&self) -> &[(f64, u8, u8, f64)] {
        &self.completed_notes
    }

    /// Get the number of completed notes
    pub fn note_count(&self) -> usize {
        self.completed_notes.len()
    }

    /// Get all completed notes plus currently-held notes with a provisional duration.
    /// Used for live preview during recording so held notes appear immediately.
    pub fn get_notes_with_active(&self, current_time: f64) -> Vec<(f64, u8, u8, f64)> {
        let mut notes = self.completed_notes.clone();
        for active in self.active_notes.values() {
            let note_start = active.start_time.max(self.start_time);
            let time_offset = note_start - self.start_time;
            let provisional_dur = (current_time - note_start).max(0.0);
            notes.push((time_offset, active.note, active.velocity, provisional_dur));
        }
        notes
    }

    /// Get the note numbers of all currently held (active) notes
    pub fn active_note_numbers(&self) -> Vec<u8> {
        self.active_notes.keys().copied().collect()
    }

    /// Close out all active notes at the given time
    /// This should be called when stopping recording to end any held notes
    pub fn close_active_notes(&mut self, end_time: f64) {
        let active_notes: Vec<_> = self.active_notes.drain().collect();

        for (_note_num, active_note) in active_notes {
            let note_start = active_note.start_time.max(self.start_time);
            let time_offset = note_start - self.start_time;
            let duration = end_time - note_start;

            self.completed_notes.push((
                time_offset,
                active_note.note,
                active_note.velocity,
                duration,
            ));
        }
    }
}
