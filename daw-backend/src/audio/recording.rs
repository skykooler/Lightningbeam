/// Audio recording system for capturing microphone input
use crate::audio::{ClipId, MidiClipId, TrackId};
use crate::io::{WavWriter, WaveformPeak};
use crate::time::{Beats, Seconds};
use std::collections::HashMap;
use std::path::PathBuf;

/// Cycle-recording bookkeeping attached to a recording that started with a cycle region armed.
///
/// Takes are sliced **geometrically** at stop, in exact `loop_len_frames` multiples — not at the
/// instant the wrap was detected. The playhead advances before the capture block in `process()`, so
/// the wrap instant isn't sample-exact against the buffer that was just captured, but the geometry
/// is. `wrap_count` therefore only decides *whether* this is a multi-take recording, never where the
/// cuts land.
#[derive(Debug, Clone, Copy)]
pub struct CycleRecordInfo {
    /// Where the cycle region starts, in beats. Takes are laid down here, not at the punch-in point.
    pub loop_start: Beats,
    /// The cycle region's length in beats — what the take folder records as `recorded_loop_beats`.
    pub loop_len_beats: Beats,
    /// One cycle pass, in frames. The take size.
    pub loop_len_frames: usize,
    /// Frames between the region start and where capture actually began. Non-zero only for a
    /// punch-in (record while already rolling); take 1 gets this much silence prepended so it still
    /// spans the whole region.
    pub lead_pad_frames: usize,
    /// How many times the transport wrapped during this recording. Zero means the user stopped
    /// before completing a pass, which stays an ordinary single recording.
    pub wrap_count: usize,
}

/// Min/max waveform peaks for a finished buffer of interleaved samples.
///
/// The live recording path builds its peaks incrementally as samples arrive; cycle takes don't
/// exist until the recording is sliced at stop, so they get theirs in one pass here.
pub fn compute_peaks(samples: &[f32], channels: u32, frames_per_peak: usize) -> Vec<WaveformPeak> {
    let samples_per_peak = (frames_per_peak * channels.max(1) as usize).max(1);
    samples
        .chunks(samples_per_peak)
        .map(|chunk| {
            let mut min = 0.0f32;
            let mut max = 0.0f32;
            for s in chunk {
                min = min.min(*s);
                max = max.max(*s);
            }
            WaveformPeak { min, max }
        })
        .collect()
}

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
    /// Cycle-recording bookkeeping, when a cycle region was armed at record start.
    pub cycle: Option<CycleRecordInfo>,
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
            cycle: None,
        }
    }

    /// Slice the recording into cycle takes: one per pass, each spanning the FULL cycle region.
    ///
    /// Partial passes are padded with silence — the head of take 1 for a punch-in, the tail of the
    /// last take when the user stops mid-pass — so every take is the same length and aligned to the
    /// region. That uniformity is what makes comping-via-split work: take 1 on the left half and
    /// take 3 on the right always line up.
    ///
    /// Returns `None` if this wasn't a cycle recording or the transport never wrapped (an ordinary
    /// single recording, which keeps the existing path untouched).
    pub fn slice_takes(&self) -> Option<Vec<Vec<f32>>> {
        let cycle = self.cycle?;
        if cycle.wrap_count == 0 || cycle.loop_len_frames == 0 {
            return None;
        }

        let ch = self.channels.max(1) as usize;
        let take_len = cycle.loop_len_frames * ch;
        let lead = cycle.lead_pad_frames * ch;

        // The recording as positioned *within the region*: silence for the gap between the region
        // start and the punch-in, then the captured audio. Slicing this at whole-take boundaries is
        // the whole trick — take 1 comes out short-by-`lead` at the front, already padded.
        let virtual_len = lead + self.audio_data.len();
        let take_count = virtual_len.div_ceil(take_len);

        let mut takes: Vec<Vec<f32>> = Vec::with_capacity(take_count);
        for i in 0..take_count {
            let mut take = vec![0.0f32; take_len];
            let take_begin = i * take_len;
            for slot in 0..take_len {
                // Position in the virtual (lead-padded) buffer.
                let v = take_begin + slot;
                if v < lead {
                    continue; // still in the punch-in silence
                }
                match self.audio_data.get(v - lead) {
                    Some(s) => take[slot] = *s,
                    None => break, // past the end of capture; the rest stays silent
                }
            }
            takes.push(take);
        }

        // A final take holding only a sliver of real audio is a stop artifact (the user hit stop a
        // moment after the wrap), not a performance. Drop it — but only if it's actually a PARTIAL
        // pass, and never the only take. A pass that filled the region is a real take no matter how
        // short the region is.
        const MIN_TAKE_SECONDS: f64 = 0.05;
        if takes.len() > 1 {
            let last_real_samples = virtual_len - (takes.len() - 1) * take_len;
            let last_real_seconds = (last_real_samples / ch) as f64 / self.sample_rate as f64;
            if last_real_samples < take_len && last_real_seconds < MIN_TAKE_SECONDS {
                takes.pop();
            }
        }

        Some(takes)
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
    /// The cycle region's length in beats, when recording into a cycle.
    ///
    /// A cycle MIDI recording is anchored at the region start (`start_time == loop_start`), which is
    /// what makes MERGE fall out for free: the transport always wraps back into the region, so every
    /// note's offset already lands inside `[0, loop_len)` and successive passes overdub onto each
    /// other with no folding needed. Set only if the transport actually wrapped.
    pub cycle_loop_len: Option<Beats>,
}

impl MidiRecordingState {
    pub fn new(track_id: TrackId, clip_id: MidiClipId, start_time: Beats) -> Self {
        Self {
            track_id,
            clip_id,
            start_time,
            active_notes: HashMap::new(),
            completed_notes: Vec::new(),
            cycle_loop_len: None,
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

    /// Handle a transport cycle wrap during MIDI recording.
    ///
    /// Note times are stored as offsets from `start_time`, and the playhead jumps *backwards* at a
    /// wrap — so a note still held across the boundary would otherwise get a nonsensical (negative)
    /// duration, or never be closed at all. Write its note-off at `region_end` (exactly as
    /// `close_active_notes` does when recording stops), then re-open it at `region_start` so a key
    /// the player is still physically holding keeps being captured in the next pass. Mirrors the
    /// way `handle_start_midi_recording` re-injects already-held notes at the recording start.
    pub fn wrap_at_cycle(&mut self, region_end: Beats, region_start: Beats) {
        // Snapshot the held notes (close_active_notes drains them and loses the velocities).
        let held: Vec<(u8, u8)> = self
            .active_notes
            .values()
            .map(|n| (n.note, n.velocity))
            .collect();

        self.close_active_notes(region_end);

        for (note, velocity) in held {
            self.note_on(note, velocity, region_start);
        }

        // The transport wrapped, so this is a cycle recording: the clip spans the whole region
        // rather than however long the user happened to hold the record button.
        self.cycle_loop_len = Some(region_end - region_start);
    }
}

#[cfg(test)]
mod cycle_tests {
    use super::*;

    /// A recording state holding `audio_data`, armed for cycle recording. Mono, 100 Hz, so a frame
    /// is a sample and 5 frames is 50 ms (exactly the min-take threshold).
    fn rec(audio: Vec<f32>, loop_len_frames: usize, lead_pad_frames: usize, wraps: usize) -> RecordingState {
        let mut r = RecordingState::new(
            0,
            0,
            PathBuf::from("/dev/null"),
            WavWriter::create(&PathBuf::from("/dev/null"), 100, 1).expect("wav writer"),
            100,
            1,
            Beats(0.0),
            1.0,
        );
        r.audio_data = audio;
        r.cycle = Some(CycleRecordInfo {
            loop_start: Beats(0.0),
            loop_len_beats: Beats(4.0),
            loop_len_frames,
            lead_pad_frames,
            wrap_count: wraps,
        });
        r
    }

    #[test]
    fn no_wrap_is_not_a_cycle_recording() {
        // Stopping before the transport ever wraps stays an ordinary single recording — the whole
        // point of triggering on the wrap rather than on the cycle region merely existing.
        let r = rec(vec![1.0; 10], 4, 0, 0);
        assert!(r.slice_takes().is_none());
    }

    #[test]
    fn takes_are_cut_at_exact_loop_multiples() {
        // 12 frames of audio, 4-frame loop, started at the region start => 3 clean takes.
        let audio: Vec<f32> = (1..=12).map(|i| i as f32).collect();
        let takes = rec(audio, 4, 0, 2).slice_takes().expect("cycle takes");
        assert_eq!(takes.len(), 3);
        assert_eq!(takes[0], vec![1.0, 2.0, 3.0, 4.0]);
        assert_eq!(takes[1], vec![5.0, 6.0, 7.0, 8.0]);
        assert_eq!(takes[2], vec![9.0, 10.0, 11.0, 12.0]);
    }

    #[test]
    fn punch_in_pads_the_head_of_take_one() {
        // Punched in 2 frames into the region: take 1 gets 2 frames of silence at the FRONT so it
        // still spans the whole region and lines up with every other take.
        let audio: Vec<f32> = (1..=10).map(|i| i as f32).collect();
        let takes = rec(audio, 4, 2, 2).slice_takes().expect("cycle takes");
        assert_eq!(takes.len(), 3);
        assert_eq!(takes[0], vec![0.0, 0.0, 1.0, 2.0]);
        assert_eq!(takes[1], vec![3.0, 4.0, 5.0, 6.0]);
        assert_eq!(takes[2], vec![7.0, 8.0, 9.0, 10.0]);
    }

    #[test]
    fn stopping_mid_pass_pads_the_tail_of_the_last_take() {
        // 13 frames, 8-frame loop => the second take holds 5 real frames (50 ms at 100 Hz, right at
        // the keep threshold) and 3 of silence.
        let audio: Vec<f32> = (1..=13).map(|i| i as f32).collect();
        let takes = rec(audio, 8, 0, 1).slice_takes().expect("cycle takes");
        assert_eq!(takes.len(), 2);
        assert_eq!(takes[1], vec![9.0, 10.0, 11.0, 12.0, 13.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn every_take_is_the_same_length() {
        // Uniform length is the invariant comping-via-split depends on.
        let audio: Vec<f32> = (1..=23).map(|i| i as f32).collect();
        let takes = rec(audio, 8, 3, 3).slice_takes().expect("cycle takes");
        assert!(takes.iter().all(|t| t.len() == 8), "takes must be uniform");
    }

    #[test]
    fn a_sliver_of_a_final_take_is_dropped() {
        // Stopped 1 frame (10 ms at 100 Hz) after the wrap — below the 50 ms floor, so that stub of
        // a take is a stop artifact and goes.
        let audio: Vec<f32> = (1..=9).map(|i| i as f32).collect();
        let takes = rec(audio, 8, 0, 1).slice_takes().expect("cycle takes");
        assert_eq!(takes.len(), 1, "a 10ms tail take should be dropped");
        assert_eq!(takes[0].len(), 8);
    }

    #[test]
    fn a_full_final_take_is_never_dropped() {
        // Regression: the sliver rule must only fire on a PARTIAL pass. A pass that filled the
        // region is a real take however short the region is — an earlier version compared a full
        // take's duration to the floor and silently ate it.
        let audio: Vec<f32> = (1..=8).map(|i| i as f32).collect();
        let takes = rec(audio, 4, 0, 1).slice_takes().expect("cycle takes");
        assert_eq!(takes.len(), 2, "both passes filled the region");
        assert_eq!(takes[1], vec![5.0, 6.0, 7.0, 8.0]);
    }
}
