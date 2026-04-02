use crate::audio::midi::{MidiClip, MidiClipId, MidiEvent};
use crate::time::Beats;
use std::fs;
use std::path::Path;

/// Load a MIDI file and convert it to a MidiClip.
///
/// Event timestamps are stored as beat positions: `tick / ticks_per_beat`.
/// Tempo events in the MIDI file only affect wall-clock playback speed — not the
/// beat grid — so they are ignored here.
pub fn load_midi_file<P: AsRef<Path>>(
    path: P,
    clip_id: MidiClipId,
    _sample_rate: u32,
) -> Result<MidiClip, String> {
    let data = fs::read(path.as_ref()).map_err(|e| format!("Failed to read MIDI file: {}", e))?;
    let smf = midly::Smf::parse(&data).map_err(|e| format!("Failed to parse MIDI file: {}", e))?;

    let ticks_per_beat = match smf.header.timing {
        midly::Timing::Metrical(tpb) => tpb.as_int() as f64,
        midly::Timing::Timecode(fps, subframe) => {
            // Timecode-based MIDI: treat subframes as ticks per beat
            (fps.as_f32() * subframe as f32) as f64
        }
    };

    let mut events = Vec::new();
    let mut max_tick = 0u64;

    for track in &smf.tracks {
        let mut current_tick = 0u64;

        for event in track {
            current_tick += event.delta.as_int() as u64;
            max_tick = max_tick.max(current_tick);

            let timestamp = Beats(current_tick as f64 / ticks_per_beat);

            match event.kind {
                midly::TrackEventKind::Midi { channel, message } => {
                    let ch = channel.as_int();
                    match message {
                        midly::MidiMessage::NoteOn { key, vel } => {
                            let velocity = vel.as_int();
                            if velocity > 0 {
                                events.push(MidiEvent::note_on(timestamp, ch, key.as_int(), velocity));
                            } else {
                                events.push(MidiEvent::note_off(timestamp, ch, key.as_int(), 64));
                            }
                        }
                        midly::MidiMessage::NoteOff { key, vel } => {
                            events.push(MidiEvent::note_off(timestamp, ch, key.as_int(), vel.as_int()));
                        }
                        midly::MidiMessage::Controller { controller, value } => {
                            let status = 0xB0 | ch;
                            events.push(MidiEvent::new(timestamp, status, controller.as_int(), value.as_int()));
                        }
                        _ => {}
                    }
                }
                _ => {} // Tempo and other meta events don't affect beat positions
            }
        }
    }

    let duration = Beats(max_tick as f64 / ticks_per_beat);
    let clip = MidiClip::new(clip_id, events, duration, "Imported MIDI".to_string());
    Ok(clip)
}
