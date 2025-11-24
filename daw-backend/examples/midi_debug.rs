use daw_backend::load_midi_file;

fn main() {
    let clip = load_midi_file("darude-sandstorm.mid", 0, 44100).unwrap();

    println!("Clip duration: {:.2}s", clip.duration);
    println!("Total events: {}", clip.events.len());
    println!("\nEvent summary:");

    let mut note_on_count = 0;
    let mut note_off_count = 0;
    let mut other_count = 0;

    for event in &clip.events {
        if event.is_note_on() {
            note_on_count += 1;
        } else if event.is_note_off() {
            note_off_count += 1;
        } else {
            other_count += 1;
        }
    }

    println!("  Note On events: {}", note_on_count);
    println!("  Note Off events: {}", note_off_count);
    println!("  Other events: {}", other_count);

    // Show events around 28 seconds
    println!("\nEvents around 28 seconds (27-29s):");
    let sample_rate = 44100.0;
    let start_sample = (27.0 * sample_rate) as u64;
    let end_sample = (29.0 * sample_rate) as u64;

    for (i, event) in clip.events.iter().enumerate() {
        if event.timestamp >= start_sample && event.timestamp <= end_sample {
            let time_sec = event.timestamp as f64 / sample_rate;
            let event_type = if event.is_note_on() {
                "NoteOn"
            } else if event.is_note_off() {
                "NoteOff"
            } else {
                "Other"
            };
            println!("  [{:4}] {:.3}s: {} ch={} note={} vel={}",
                    i, time_sec, event_type, event.channel(), event.data1, event.data2);
        }
    }

    // Check for stuck notes - note ons without corresponding note offs
    println!("\nChecking for unmatched notes...");
    let mut active_notes = std::collections::HashMap::new();

    for (i, event) in clip.events.iter().enumerate() {
        if event.is_note_on() {
            let key = (event.channel(), event.data1);
            active_notes.insert(key, i);
        } else if event.is_note_off() {
            let key = (event.channel(), event.data1);
            active_notes.remove(&key);
        }
    }

    if !active_notes.is_empty() {
        println!("Found {} notes that never got note-off events:", active_notes.len());
        for ((ch, note), event_idx) in active_notes.iter().take(10) {
            let time_sec = clip.events[*event_idx].timestamp as f64 / sample_rate;
            println!("  Note {} on channel {} at {:.2}s (event #{})", note, ch, time_sec, event_idx);
        }
    } else {
        println!("All notes have matching note-off events!");
    }
}
