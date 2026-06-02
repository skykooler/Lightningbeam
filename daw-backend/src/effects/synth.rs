use super::Effect;
use crate::audio::midi::MidiEvent;
use std::f32::consts::PI;

/// Maximum number of simultaneous voices
const MAX_VOICES: usize = 16;

/// Envelope state for a voice
#[derive(Clone, Copy, PartialEq)]
enum EnvelopeState {
    Attack,
    Sustain,
    Release,
    Off,
}

/// A single synthesizer voice
#[derive(Clone)]
struct SynthVoice {
    active: bool,
    note: u8,
    channel: u8,
    velocity: u8,
    phase: f32,
    frequency: f32,
    age: u32, // For voice stealing

    // Envelope
    envelope_state: EnvelopeState,
    envelope_level: f32, // 0.0 to 1.0
}

impl SynthVoice {
    fn new() -> Self {
        Self {
            active: false,
            note: 0,
            channel: 0,
            velocity: 0,
            phase: 0.0,
            frequency: 0.0,
            age: 0,
            envelope_state: EnvelopeState::Off,
            envelope_level: 0.0,
        }
    }

    /// Calculate frequency from MIDI note number
    fn note_to_frequency(note: u8) -> f32 {
        440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0)
    }

    /// Start playing a note
    fn note_on(&mut self, channel: u8, note: u8, velocity: u8) {
        self.active = true;
        self.channel = channel;
        self.note = note;
        self.velocity = velocity;
        self.frequency = Self::note_to_frequency(note);
        self.phase = 0.0;
        self.age = 0;
        self.envelope_state = EnvelopeState::Attack;
        self.envelope_level = 0.0; // Start from silence
    }

    /// Stop playing (start release phase)
    fn note_off(&mut self) {
        // Don't stop immediately - start release phase
        if self.envelope_state != EnvelopeState::Off {
            self.envelope_state = EnvelopeState::Release;
        }
    }

    /// Generate one sample
    fn process_sample(&mut self, sample_rate: f32) -> f32 {
        if self.envelope_state == EnvelopeState::Off {
            return 0.0;
        }

        // Envelope timing constants (in seconds)
        const ATTACK_TIME: f32 = 0.005;   // 5ms attack
        const RELEASE_TIME: f32 = 0.05;   // 50ms release

        // Update envelope
        let attack_increment = 1.0 / (ATTACK_TIME * sample_rate);
        let release_decrement = 1.0 / (RELEASE_TIME * sample_rate);

        match self.envelope_state {
            EnvelopeState::Attack => {
                self.envelope_level += attack_increment;
                if self.envelope_level >= 1.0 {
                    self.envelope_level = 1.0;
                    self.envelope_state = EnvelopeState::Sustain;
                }
            }
            EnvelopeState::Sustain => {
                // Stay at full level
                self.envelope_level = 1.0;
            }
            EnvelopeState::Release => {
                self.envelope_level -= release_decrement;
                if self.envelope_level <= 0.0 {
                    self.envelope_level = 0.0;
                    self.envelope_state = EnvelopeState::Off;
                    self.active = false; // Now we can truly stop
                }
            }
            EnvelopeState::Off => {
                return 0.0;
            }
        }

        // Simple sine wave
        let sample = (self.phase * 2.0 * PI).sin() * (self.velocity as f32 / 127.0) * 0.3;

        // Update phase
        self.phase += self.frequency / sample_rate;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        self.age += 1;

        // Apply envelope
        sample * self.envelope_level
    }
}

/// Simple polyphonic synthesizer using sine waves
pub struct SimpleSynth {
    voices: Vec<SynthVoice>,
    sample_rate: f32,
    pub pending_events: Vec<MidiEvent>,
}

impl SimpleSynth {
    /// Create a new SimpleSynth
    pub fn new() -> Self {
        Self {
            voices: vec![SynthVoice::new(); MAX_VOICES],
            sample_rate: 44100.0,
            pending_events: Vec::new(),
        }
    }

    /// Find a free voice, or steal the oldest one
    fn find_voice_for_note_on(&mut self) -> usize {
        // First, look for an inactive voice
        for (i, voice) in self.voices.iter().enumerate() {
            if !voice.active {
                return i;
            }
        }

        // No free voices, steal the oldest one
        self.voices
            .iter()
            .enumerate()
            .max_by_key(|(_, v)| v.age)
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    /// Find the voice playing a specific note on a specific channel
    /// Only matches voices in Attack or Sustain state (not already releasing)
    fn find_voice_for_note_off(&mut self, channel: u8, note: u8) -> Option<usize> {
        self.voices
            .iter()
            .position(|v| {
                v.active
                    && v.channel == channel
                    && v.note == note
                    && (v.envelope_state == EnvelopeState::Attack
                        || v.envelope_state == EnvelopeState::Sustain)
            })
    }

    /// Handle a MIDI event
    pub fn handle_event(&mut self, event: &MidiEvent) {
        if event.is_note_on() {
            let voice_idx = self.find_voice_for_note_on();
            self.voices[voice_idx].note_on(event.channel(), event.data1, event.data2);
        } else if event.is_note_off() {
            if let Some(voice_idx) = self.find_voice_for_note_off(event.channel(), event.data1) {
                self.voices[voice_idx].note_off();
            }
        }
    }

    /// Queue a MIDI event to be processed
    pub fn queue_event(&mut self, event: MidiEvent) {
        self.pending_events.push(event);
    }

    /// Stop all currently playing notes immediately (no release envelope)
    pub fn all_notes_off(&mut self) {
        for voice in &mut self.voices {
            voice.active = false;
            voice.envelope_state = EnvelopeState::Off;
            voice.envelope_level = 0.0;
        }
        self.pending_events.clear();
    }

    /// Process all queued events
    fn process_events(&mut self) {
        // Collect events first to avoid borrowing issues
        let events: Vec<MidiEvent> = self.pending_events.drain(..).collect();
        for event in events {
            self.handle_event(&event);
        }
    }
}

impl Effect for SimpleSynth {
    fn process(&mut self, buffer: &mut [f32], channels: usize, sample_rate: u32) {
        self.sample_rate = sample_rate as f32;

        // Process any queued MIDI events
        self.process_events();

        // Generate audio from all active voices
        if channels == 1 {
            // Mono
            for sample in buffer.iter_mut() {
                let mut sum = 0.0;
                for voice in &mut self.voices {
                    sum += voice.process_sample(self.sample_rate);
                }
                *sample += sum;
            }
        } else if channels == 2 {
            // Stereo (duplicate mono signal)
            for frame in buffer.chunks_exact_mut(2) {
                let mut sum = 0.0;
                for voice in &mut self.voices {
                    sum += voice.process_sample(self.sample_rate);
                }
                frame[0] += sum;
                frame[1] += sum;
            }
        }
    }

    fn set_parameter(&mut self, id: u32, value: f32) {
        // Parameter 0: Note on
        // Parameter 1: Note off
        // This is a simple interface for testing without proper MIDI routing
        match id {
            0 => {
                let note = value as u8;
                let voice_idx = self.find_voice_for_note_on();
                self.voices[voice_idx].note_on(0, note, 100);
            }
            1 => {
                let note = value as u8;
                if let Some(voice_idx) = self.find_voice_for_note_off(0, note) {
                    self.voices[voice_idx].note_off();
                }
            }
            _ => {}
        }
    }

    fn get_parameter(&self, _id: u32) -> f32 {
        0.0
    }

    fn reset(&mut self) {
        for voice in &mut self.voices {
            voice.note_off();
        }
        self.pending_events.clear();
    }

    fn name(&self) -> &str {
        "SimpleSynth"
    }
}

impl Default for SimpleSynth {
    fn default() -> Self {
        Self::new()
    }
}
