use daw_backend::load_midi_file;

fn main() {
    let clip = load_midi_file("darude-sandstorm.mid", 0, 44100).unwrap();

    println!("Clip duration: {:.3}s", clip.duration);
    println!("Total events: {}", clip.events.len());

    // Show the last 30 events
    println!("\nLast 30 events:");
    let sample_rate = 44100.0;
    let start_idx = clip.events.len().saturating_sub(30);

    for (i, event) in clip.events.iter().enumerate().skip(start_idx) {
        let time_sec = event.timestamp as f64 / sample_rate;
        let event_type = if event.is_note_on() {
            "NoteOn "
        } else if event.is_note_off() {
            "NoteOff"
        } else {
            "Other  "
        };
        println!("  [{:4}] {:.3}s: {} ch={} note={:3} vel={:3}",
                i, time_sec, event_type, event.channel(), event.data1, event.data2);
    }

    // Find notes that are still active at the end of the clip
    println!("\nNotes active at end of clip ({:.3}s):", clip.duration);
    let mut active_notes = std::collections::HashMap::new();

    for event in &clip.events {
        let time_sec = event.timestamp as f64 / sample_rate;

        if event.is_note_on() {
            let key = (event.channel(), event.data1);
            active_notes.insert(key, time_sec);
        } else if event.is_note_off() {
            let key = (event.channel(), event.data1);
            active_notes.remove(&key);
        }
    }

    if !active_notes.is_empty() {
        println!("Found {} notes still active after all events:", active_notes.len());
        for ((ch, note), start_time) in &active_notes {
            println!("  Channel {} Note {} started at {:.3}s (no note-off before clip end)",
                    ch, note, start_time);
        }
    } else {
        println!("All notes are turned off by the end!");
    }

    // Check maximum polyphony
    println!("\nAnalyzing polyphony...");
    let mut max_polyphony = 0;
    let mut current_notes = std::collections::HashSet::new();

    for event in &clip.events {
        if event.is_note_on() {
            let key = (event.channel(), event.data1);
            current_notes.insert(key);
            max_polyphony = max_polyphony.max(current_notes.len());
        } else if event.is_note_off() {
            let key = (event.channel(), event.data1);
            current_notes.remove(&key);
        }
    }

    println!("Maximum simultaneous notes: {}", max_polyphony);
    println!("Available synth voices: 16");
    if max_polyphony > 16 {
        println!("WARNING: Polyphony exceeds available voices! Voice stealing will occur.");
    }
}
