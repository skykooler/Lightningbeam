use crate::audio::midi::{MidiClip, MidiClipId, MidiEvent};
use std::fs;
use std::path::Path;

/// Load a MIDI file and convert it to a MidiClip
pub fn load_midi_file<P: AsRef<Path>>(
    path: P,
    clip_id: MidiClipId,
    _sample_rate: u32,
) -> Result<MidiClip, String> {
    // Read the MIDI file
    let data = fs::read(path.as_ref()).map_err(|e| format!("Failed to read MIDI file: {}", e))?;

    // Parse with midly
    let smf = midly::Smf::parse(&data).map_err(|e| format!("Failed to parse MIDI file: {}", e))?;

    // Convert timing to ticks per second
    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as f64,
        midly::Timing::Timecode(fps, subframe) => {
            // For timecode, calculate equivalent ticks per second
            (fps.as_f32() * subframe as f32) as f64
        }
    };

    // First pass: collect all events with their tick positions and tempo changes
    #[derive(Debug)]
    enum RawEvent {
        Midi {
            tick: u64,
            channel: u8,
            message: midly::MidiMessage,
        },
        Tempo {
            tick: u64,
            microseconds_per_beat: f64,
        },
    }

    let mut raw_events = Vec::new();
    let mut max_time_ticks = 0u64;

    // Collect all events from all tracks with their absolute tick positions
    for track in &smf.tracks {
        let mut current_tick = 0u64;

        for event in track {
            current_tick += event.delta.as_int() as u64;
            max_time_ticks = max_time_ticks.max(current_tick);

            match event.kind {
                midly::TrackEventKind::Midi { channel, message } => {
                    raw_events.push(RawEvent::Midi {
                        tick: current_tick,
                        channel: channel.as_int(),
                        message,
                    });
                }
                midly::TrackEventKind::Meta(midly::MetaMessage::Tempo(tempo)) => {
                    raw_events.push(RawEvent::Tempo {
                        tick: current_tick,
                        microseconds_per_beat: tempo.as_int() as f64,
                    });
                }
                _ => {
                    // Ignore other meta events
                }
            }
        }
    }

    // Sort all events by tick position
    raw_events.sort_by_key(|e| match e {
        RawEvent::Midi { tick, .. } => *tick,
        RawEvent::Tempo { tick, .. } => *tick,
    });

    // Second pass: convert ticks to timestamps with proper tempo tracking
    let mut events = Vec::new();
    let mut microseconds_per_beat = 500000.0; // Default: 120 BPM
    let mut last_tick = 0u64;
    let mut accumulated_time = 0.0; // Time in seconds

    for raw_event in raw_events {
        match raw_event {
            RawEvent::Tempo {
                tick,
                microseconds_per_beat: new_tempo,
            } => {
                // Update accumulated time up to this tempo change
                let delta_ticks = tick - last_tick;
                let delta_time = (delta_ticks as f64 / ticks_per_beat)
                    * (microseconds_per_beat / 1_000_000.0);
                accumulated_time += delta_time;
                last_tick = tick;

                // Update tempo for future events
                microseconds_per_beat = new_tempo;
            }
            RawEvent::Midi {
                tick,
                channel,
                message,
            } => {
                // Calculate time for this event
                let delta_ticks = tick - last_tick;
                let delta_time = (delta_ticks as f64 / ticks_per_beat)
                    * (microseconds_per_beat / 1_000_000.0);
                accumulated_time += delta_time;
                last_tick = tick;

                // Store timestamp in seconds (sample-rate independent)
                let timestamp = accumulated_time;

                match message {
                    midly::MidiMessage::NoteOn { key, vel } => {
                        let velocity = vel.as_int();
                        if velocity > 0 {
                            events.push(MidiEvent::note_on(
                                timestamp,
                                channel,
                                key.as_int(),
                                velocity,
                            ));
                        } else {
                            events.push(MidiEvent::note_off(timestamp, channel, key.as_int(), 64));
                        }
                    }
                    midly::MidiMessage::NoteOff { key, vel } => {
                        events.push(MidiEvent::note_off(
                            timestamp,
                            channel,
                            key.as_int(),
                            vel.as_int(),
                        ));
                    }
                    midly::MidiMessage::Controller { controller, value } => {
                        let status = 0xB0 | channel;
                        events.push(MidiEvent::new(
                            timestamp,
                            status,
                            controller.as_int(),
                            value.as_int(),
                        ));
                    }
                    _ => {
                        // Ignore other MIDI messages
                    }
                }
            }
        }
    }

    // Calculate final clip duration
    let final_delta_ticks = max_time_ticks - last_tick;
    let final_delta_time =
        (final_delta_ticks as f64 / ticks_per_beat) * (microseconds_per_beat / 1_000_000.0);
    let duration_seconds = accumulated_time + final_delta_time;

    // Create the MIDI clip
    let mut clip = MidiClip::new(clip_id, 0.0, duration_seconds);
    clip.events = events;

    Ok(clip)
}
