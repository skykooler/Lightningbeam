/// Metronome for providing click track during playback
pub struct Metronome {
    enabled: bool,
    bpm: f32,
    time_signature_numerator: u32,
    time_signature_denominator: u32,
    last_beat: i64,  // Last beat number that was played (-1 = none)

    // Pre-generated click samples (mono)
    high_click: Vec<f32>,  // Accent click for first beat
    low_click: Vec<f32>,   // Normal click for other beats

    // Click playback state
    click_position: usize,  // Current position in the click sample (0 = not playing)
    playing_high_click: bool,  // Which click we're currently playing

    sample_rate: u32,
}

impl Metronome {
    /// Create a new metronome with pre-generated click sounds
    pub fn new(sample_rate: u32) -> Self {
        let (high_click, low_click) = Self::generate_clicks(sample_rate);

        Self {
            enabled: false,
            bpm: 120.0,
            time_signature_numerator: 4,
            time_signature_denominator: 4,
            last_beat: -1,
            high_click,
            low_click,
            click_position: 0,
            playing_high_click: false,
            sample_rate,
        }
    }

    /// Generate woodblock-style click samples
    fn generate_clicks(sample_rate: u32) -> (Vec<f32>, Vec<f32>) {
        let click_duration_ms = 10.0;  // 10ms click
        let click_samples = ((sample_rate as f32 * click_duration_ms) / 1000.0) as usize;

        // High click (accent): 1200 Hz + 2400 Hz (higher pitched woodblock)
        let high_freq1 = 1200.0;
        let high_freq2 = 2400.0;
        let mut high_click = Vec::with_capacity(click_samples);

        for i in 0..click_samples {
            let t = i as f32 / sample_rate as f32;
            let envelope = 1.0 - (i as f32 / click_samples as f32);  // Linear decay
            let envelope = envelope * envelope;  // Square for faster decay

            // Mix two sine waves for woodblock character
            let sample = 0.3 * (2.0 * std::f32::consts::PI * high_freq1 * t).sin()
                       + 0.2 * (2.0 * std::f32::consts::PI * high_freq2 * t).sin();

            // Add a bit of noise for attack transient
            let noise = (i as f32 * 0.1).sin() * 0.1;

            high_click.push((sample + noise) * envelope * 0.5);  // Scale down to avoid clipping
        }

        // Low click: 800 Hz + 1600 Hz (lower pitched woodblock)
        let low_freq1 = 800.0;
        let low_freq2 = 1600.0;
        let mut low_click = Vec::with_capacity(click_samples);

        for i in 0..click_samples {
            let t = i as f32 / sample_rate as f32;
            let envelope = 1.0 - (i as f32 / click_samples as f32);
            let envelope = envelope * envelope;

            let sample = 0.3 * (2.0 * std::f32::consts::PI * low_freq1 * t).sin()
                       + 0.2 * (2.0 * std::f32::consts::PI * low_freq2 * t).sin();

            let noise = (i as f32 * 0.1).sin() * 0.1;

            low_click.push((sample + noise) * envelope * 0.4);  // Slightly quieter than high click
        }

        (high_click, low_click)
    }

    /// Enable or disable the metronome
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.last_beat = -1;  // Reset beat tracking when disabled
            self.click_position = 0;  // Stop any playing click
        } else {
            // When enabling, don't trigger a click until the next beat
            self.click_position = usize::MAX;  // Set to max to prevent immediate click
        }
    }

    /// Update BPM and time signature
    pub fn update_timing(&mut self, bpm: f32, time_signature: (u32, u32)) {
        self.bpm = bpm;
        self.time_signature_numerator = time_signature.0;
        self.time_signature_denominator = time_signature.1;
    }

    /// Process audio and mix in metronome clicks
    pub fn process(
        &mut self,
        output: &mut [f32],
        playhead_samples: u64,
        playing: bool,
        sample_rate: u32,
        channels: u32,
    ) {
        if !self.enabled || !playing {
            self.click_position = 0;  // Reset if not playing
            return;
        }

        let frames = output.len() / channels as usize;

        for frame in 0..frames {
            let current_sample = playhead_samples + frame as u64;

            // Calculate current beat number
            let current_time_seconds = current_sample as f64 / sample_rate as f64;
            let beats_per_second = self.bpm as f64 / 60.0;
            let current_beat = (current_time_seconds * beats_per_second).floor() as i64;

            // Check if we crossed a beat boundary
            if current_beat != self.last_beat && current_beat >= 0 {
                self.last_beat = current_beat;

                // Only trigger a click if we're not in the "just enabled" state
                if self.click_position != usize::MAX {
                    // Determine which click to play
                    // Beat 1 of each measure gets the accent (high click)
                    let beat_in_measure = (current_beat as u32 % self.time_signature_numerator) as usize;
                    let is_first_beat = beat_in_measure == 0;

                    // Start playing the appropriate click
                    self.playing_high_click = is_first_beat;
                    self.click_position = 0;  // Start from beginning of click
                } else {
                    // We just got enabled - reset position but don't play yet
                    self.click_position = self.high_click.len();  // Set past end so no click plays
                }
            }

            // Continue playing click sample if we're currently in one
            let click = if self.playing_high_click {
                &self.high_click
            } else {
                &self.low_click
            };

            if self.click_position < click.len() {
                let click_sample = click[self.click_position];

                // Mix into all channels
                for ch in 0..channels as usize {
                    let output_idx = frame * channels as usize + ch;
                    output[output_idx] += click_sample;
                }

                self.click_position += 1;
            }
        }
    }
}
