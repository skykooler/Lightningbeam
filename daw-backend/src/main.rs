use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use daw_backend::{load_midi_file, AudioEvent, AudioFile, Clip, Engine, PoolAudioFile, Track};
use std::env;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get audio file paths from command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <audio_file1> [audio_file2] [audio_file3] ...", args[0]);
        eprintln!("Example: {} track1.wav track2.wav", args[0]);
        return Ok(());
    }

    println!("DAW Backend - Phase 6: Hierarchical Tracks\n");

    // Load all audio files
    let mut audio_files = Vec::new();
    let mut max_sample_rate = 0;
    let mut max_channels = 0;

    for (i, path) in args.iter().skip(1).enumerate() {
        println!("Loading file {}: {}", i + 1, path);
        match AudioFile::load(path) {
            Ok(audio_file) => {
                let duration = audio_file.frames as f64 / audio_file.sample_rate as f64;
                println!(
                    "  {} Hz, {} channels, {} frames ({:.2}s)",
                    audio_file.sample_rate, audio_file.channels, audio_file.frames, duration
                );

                max_sample_rate = max_sample_rate.max(audio_file.sample_rate);
                max_channels = max_channels.max(audio_file.channels);

                audio_files.push((
                    Path::new(path)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    PathBuf::from(path),
                    audio_file,
                ));
            }
            Err(e) => {
                eprintln!("  Error loading {}: {}", path, e);
                eprintln!("  Skipping this file...");
            }
        }
    }

    if audio_files.is_empty() {
        eprintln!("No audio files loaded. Exiting.");
        return Ok(());
    }

    println!("\nProject settings:");
    println!("  Sample rate: {} Hz", max_sample_rate);
    println!("  Channels: {}", max_channels);
    println!("  Files: {}", audio_files.len());

    // Initialize cpal
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No output device available")?;
    println!("\nUsing audio device: {}", device.name()?);

    // Get the default output config to determine sample format
    let default_config = device.default_output_config()?;
    let sample_format = default_config.sample_format();

    // Create a custom config matching the project settings
    let config = cpal::StreamConfig {
        channels: max_channels as u16,
        sample_rate: cpal::SampleRate(max_sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    println!("Output config: {:?} with format {:?}", config, sample_format);

    // Create lock-free command and event queues
    let (command_tx, command_rx) = rtrb::RingBuffer::<daw_backend::Command>::new(256);
    let (event_tx, event_rx) = rtrb::RingBuffer::<AudioEvent>::new(256);

    // Create the audio engine
    let mut engine = Engine::new(max_sample_rate, max_channels, command_rx, event_tx);

    // Add all files to the audio pool and create tracks with clips
    let track_ids = Arc::new(Mutex::new(Vec::new()));
    let mut clip_info = Vec::new(); // Store (track_id, clip_id, name, duration)
    let mut max_duration = 0.0f64;
    let mut clip_id_counter = 0u32;

    println!("\nCreating tracks and clips:");
    for (name, path, audio_file) in audio_files.into_iter() {
        let duration = audio_file.frames as f64 / audio_file.sample_rate as f64;
        max_duration = max_duration.max(duration);

        // Add audio file to pool
        let pool_file = PoolAudioFile::new(
            path,
            audio_file.data,
            audio_file.channels,
            audio_file.sample_rate,
        );
        let pool_index = engine.audio_pool_mut().add_file(pool_file);

        // Create track (the ID passed to Track::new is ignored; Project assigns IDs)
        let mut track = Track::new(0, name.clone());

        // Create clip that plays the entire file starting at time 0
        let clip_id = clip_id_counter;
        let clip = Clip::new(
            clip_id,
            pool_index,
            0.0,        // start at beginning of timeline
            duration,   // full duration
            0.0,        // no offset into file
        );
        clip_id_counter += 1;

        track.add_clip(clip);

        // Capture the ACTUAL track ID assigned by the project
        let actual_track_id = engine.add_track(track);
        track_ids.lock().unwrap().push(actual_track_id);
        clip_info.push((actual_track_id, clip_id, name.clone(), duration));

        println!("  Track {}: {} (clip {} at 0.0s, duration {:.2}s)", actual_track_id, name, clip_id, duration);
    }

    println!("\nTimeline duration: {:.2}s", max_duration);

    let mut controller = engine.get_controller(command_tx);

    // Build the output stream - Engine moves into the audio thread (no Arc, no Mutex!)
    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, engine)?,
        cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, engine)?,
        cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, engine)?,
        _ => return Err("Unsupported sample format".into()),
    };

    // Start the audio stream
    stream.play()?;
    println!("\nAudio stream started!");
    print_help();
    {
        let ids = track_ids.lock().unwrap();
        print_status(0.0, max_duration, &ids);
    }

    // Spawn event listener thread
    let event_rx = Arc::new(Mutex::new(event_rx));
    let event_rx_clone = Arc::clone(&event_rx);
    let track_ids_clone = Arc::clone(&track_ids);
    let _event_thread = thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(50));
            let mut rx = event_rx_clone.lock().unwrap();
            while let Ok(event) = rx.pop() {
                match event {
                    AudioEvent::PlaybackPosition(pos) => {
                        // Clear the line and show position
                        print!("\r\x1b[K");
                        print!("Position: {:.2}s / {:.2}s", pos, max_duration);
                        print!("  [");
                        let bar_width = 30;
                        let filled = ((pos / max_duration) * bar_width as f64) as usize;
                        for i in 0..bar_width {
                            if i < filled {
                                print!("=");
                            } else if i == filled {
                                print!(">");
                            } else {
                                print!(" ");
                            }
                        }
                        print!("]");
                        io::stdout().flush().ok();
                    }
                    AudioEvent::PlaybackStopped => {
                        print!("\r\x1b[K");
                        println!("Playback stopped (end of timeline)");
                        print!("> ");
                        io::stdout().flush().ok();
                    }
                    AudioEvent::BufferUnderrun => {
                        eprintln!("\nWarning: Buffer underrun detected");
                    }
                    AudioEvent::TrackCreated(track_id, is_metatrack, name) => {
                        print!("\r\x1b[K");
                        if is_metatrack {
                            println!("Metatrack {} created: '{}' (ID: {})", track_id, name, track_id);
                        } else {
                            println!("Track {} created: '{}' (ID: {})", track_id, name, track_id);
                        }
                        track_ids_clone.lock().unwrap().push(track_id);
                        print!("> ");
                        io::stdout().flush().ok();
                    }
                    AudioEvent::BufferPoolStats(stats) => {
                        print!("\r\x1b[K");
                        println!("\n=== Buffer Pool Statistics ===");
                        println!("  Total buffers:     {}", stats.total_buffers);
                        println!("  Available buffers: {}", stats.available_buffers);
                        println!("  In-use buffers:    {}", stats.in_use_buffers);
                        println!("  Peak usage:        {}", stats.peak_usage);
                        println!("  Total allocations: {}", stats.total_allocations);
                        println!("  Buffer size:       {} samples", stats.buffer_size);
                        if stats.total_allocations == 0 {
                            println!("  Status: \x1b[32mOK\x1b[0m - Zero allocations during playback");
                        } else {
                            println!("  Status: \x1b[33mWARNING\x1b[0m - {} allocation(s) occurred", stats.total_allocations);
                            println!("  Recommendation: Increase initial buffer pool capacity to {}", stats.peak_usage + 2);
                        }
                        println!();
                        print!("> ");
                        io::stdout().flush().ok();
                    }
                }
            }
        }
    });

    // Simple command loop
    loop {
        let mut input = String::new();
        print!("\r\x1b[K> ");
        io::stdout().flush()?;
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // Parse input
        if input.is_empty() {
            controller.play();
            println!("Playing...");
        } else if input == "q" || input == "quit" {
            println!("Quitting...");
            break;
        } else if input == "s" || input == "stop" {
            controller.stop();
            println!("Stopped (reset to beginning)");
        } else if input == "p" || input == "play" {
            controller.play();
            println!("Playing...");
        } else if input == "pause" {
            controller.pause();
            println!("Paused");
        } else if input.starts_with("seek ") {
            // Parse seek time
            if let Ok(seconds) = input[5..].trim().parse::<f64>() {
                if seconds >= 0.0 {
                    controller.seek(seconds);
                    println!("Seeking to {:.2}s", seconds);
                } else {
                    println!("Invalid seek time (must be >= 0.0)");
                }
            } else {
                println!("Invalid seek format. Usage: seek <seconds>");
            }
        } else if input.starts_with("volume ") {
            // Parse: volume <track_id> <volume>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(volume)) = (parts[1].parse::<u32>(), parts[2].parse::<f32>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.set_track_volume(track_id, volume);
                        println!("Set track {} volume to {:.2}", track_id, volume);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: volume <track_id> <volume>");
                }
            } else {
                println!("Usage: volume <track_id> <volume>");
            }
        } else if input.starts_with("mute ") {
            // Parse: mute <track_id>
            if let Ok(track_id) = input[5..].trim().parse::<u32>() {
                let ids = track_ids.lock().unwrap();
                if ids.contains(&track_id) {
                    drop(ids);
                    controller.set_track_mute(track_id, true);
                    println!("Muted track {}", track_id);
                } else {
                    println!("Invalid track ID. Available tracks: {:?}", *ids);
                }
            } else {
                println!("Usage: mute <track_id>");
            }
        } else if input.starts_with("unmute ") {
            // Parse: unmute <track_id>
            if let Ok(track_id) = input[7..].trim().parse::<u32>() {
                let ids = track_ids.lock().unwrap();
                if ids.contains(&track_id) {
                    drop(ids);
                    controller.set_track_mute(track_id, false);
                    println!("Unmuted track {}", track_id);
                } else {
                    println!("Invalid track ID. Available tracks: {:?}", *ids);
                }
            } else {
                println!("Usage: unmute <track_id>");
            }
        } else if input.starts_with("solo ") {
            // Parse: solo <track_id>
            if let Ok(track_id) = input[5..].trim().parse::<u32>() {
                let ids = track_ids.lock().unwrap();
                if ids.contains(&track_id) {
                    drop(ids);
                    controller.set_track_solo(track_id, true);
                    println!("Soloed track {}", track_id);
                } else {
                    println!("Invalid track ID. Available tracks: {:?}", *ids);
                }
            } else {
                println!("Usage: solo <track_id>");
            }
        } else if input.starts_with("unsolo ") {
            // Parse: unsolo <track_id>
            if let Ok(track_id) = input[7..].trim().parse::<u32>() {
                let ids = track_ids.lock().unwrap();
                if ids.contains(&track_id) {
                    drop(ids);
                    controller.set_track_solo(track_id, false);
                    println!("Unsoloed track {}", track_id);
                } else {
                    println!("Invalid track ID. Available tracks: {:?}", *ids);
                }
            } else {
                println!("Usage: unsolo <track_id>");
            }
        } else if input.starts_with("move ") {
            // Parse: move <track_id> <clip_id> <new_start_time>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 4 {
                if let (Ok(track_id), Ok(clip_id), Ok(time)) =
                    (parts[1].parse::<u32>(), parts[2].parse::<u32>(), parts[3].parse::<f64>()) {
                    // Validate track and clip exist
                    if let Some((_tid, _cid, name, _)) = clip_info.iter().find(|(t, c, _, _)| *t == track_id && *c == clip_id) {
                        controller.move_clip(track_id, clip_id, time);
                        println!("Moved clip {} ('{}') on track {} to {:.2}s", clip_id, name, track_id, time);
                    } else {
                        println!("Invalid track ID or clip ID");
                        println!("Available clips:");
                        for (tid, cid, name, dur) in &clip_info {
                            println!("  Track {}, Clip {} ('{}', duration {:.2}s)", tid, cid, name, dur);
                        }
                    }
                } else {
                    println!("Invalid format. Usage: move <track_id> <clip_id> <time>");
                }
            } else {
                println!("Usage: move <track_id> <clip_id> <time>");
            }
        } else if input == "tracks" {
            let ids = track_ids.lock().unwrap();
            println!("Available tracks: {:?}", *ids);
        } else if input == "clips" {
            // Display clips from the tracked clip_info
            println!("Available clips:");
            if clip_info.is_empty() {
                println!("  (no clips)");
            } else {
                for (tid, cid, name, dur) in &clip_info {
                    println!("  Track {}, Clip {} ('{}', duration {:.2}s)", tid, cid, name, dur);
                }
            }
        } else if input.starts_with("gain ") {
            // Parse: gain <track_id> <gain_db>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(gain_db)) = (parts[1].parse::<u32>(), parts[2].parse::<f32>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.add_gain_effect(track_id, gain_db);
                        println!("Set gain on track {} to {:.1} dB", track_id, gain_db);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: gain <track_id> <gain_db>");
                }
            } else {
                println!("Usage: gain <track_id> <gain_db>");
            }
        } else if input.starts_with("pan ") {
            // Parse: pan <track_id> <pan>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(pan)) = (parts[1].parse::<u32>(), parts[2].parse::<f32>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        let clamped_pan = pan.clamp(-1.0, 1.0);
                        controller.add_pan_effect(track_id, clamped_pan);
                        let pos = if clamped_pan < -0.01 {
                            format!("{:.0}% left", -clamped_pan * 100.0)
                        } else if clamped_pan > 0.01 {
                            format!("{:.0}% right", clamped_pan * 100.0)
                        } else {
                            "center".to_string()
                        };
                        println!("Set pan on track {} to {} ({:.2})", track_id, pos, clamped_pan);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: pan <track_id> <pan>");
                }
            } else {
                println!("Usage: pan <track_id> <pan> (where pan is -1.0=left, 0.0=center, 1.0=right)");
            }
        } else if input.starts_with("eq ") {
            // Parse: eq <track_id> <low_db> <mid_db> <high_db>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 5 {
                if let (Ok(track_id), Ok(low), Ok(mid), Ok(high)) =
                    (parts[1].parse::<u32>(), parts[2].parse::<f32>(), parts[3].parse::<f32>(), parts[4].parse::<f32>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.add_eq_effect(track_id, low, mid, high);
                        println!("Set EQ on track {}: Low {:.1} dB, Mid {:.1} dB, High {:.1} dB",
                                track_id, low, mid, high);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: eq <track_id> <low_db> <mid_db> <high_db>");
                }
            } else {
                println!("Usage: eq <track_id> <low_db> <mid_db> <high_db>");
            }
        } else if input.starts_with("clearfx ") {
            // Parse: clearfx <track_id>
            if let Ok(track_id) = input[8..].trim().parse::<u32>() {
                let ids = track_ids.lock().unwrap();
                if ids.contains(&track_id) {
                    drop(ids);
                    controller.clear_effects(track_id);
                    println!("Cleared all effects from track {}", track_id);
                } else {
                    println!("Invalid track ID. Available tracks: {:?}", *ids);
                }
            } else {
                println!("Usage: clearfx <track_id>");
            }
        } else if input.starts_with("meta ") {
            // Parse: meta <name>
            let name = input[5..].trim().to_string();
            if !name.is_empty() {
                controller.create_metatrack(name.clone());
                println!("Created metatrack '{}'", name);
            } else {
                println!("Usage: meta <name>");
            }
        } else if input.starts_with("addtometa ") {
            // Parse: addtometa <track_id> <metatrack_id>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(metatrack_id)) = (parts[1].parse::<u32>(), parts[2].parse::<u32>()) {
                    controller.add_to_metatrack(track_id, metatrack_id);
                    println!("Added track {} to metatrack {}", track_id, metatrack_id);
                } else {
                    println!("Invalid format. Usage: addtometa <track_id> <metatrack_id>");
                }
            } else {
                println!("Usage: addtometa <track_id> <metatrack_id>");
            }
        } else if input.starts_with("removefrommeta ") {
            // Parse: removefrommeta <track_id>
            if let Ok(track_id) = input[15..].trim().parse::<u32>() {
                controller.remove_from_metatrack(track_id);
                println!("Removed track {} from its metatrack", track_id);
            } else {
                println!("Usage: removefrommeta <track_id>");
            }
        } else if input.starts_with("midi ") {
            // Parse: midi <name>
            let name = input[5..].trim().to_string();
            if !name.is_empty() {
                controller.create_midi_track(name.clone());
                println!("Created MIDI track '{}'", name);
            } else {
                println!("Usage: midi <name>");
            }
        } else if input.starts_with("midiclip ") {
            // Parse: midiclip <track_id> <start_time> <duration>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 4 {
                if let (Ok(track_id), Ok(start_time), Ok(duration)) =
                    (parts[1].parse::<u32>(), parts[2].parse::<f64>(), parts[3].parse::<f64>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.create_midi_clip(track_id, start_time, duration);
                        println!("Created MIDI clip on track {} at {:.2}s (duration {:.2}s)",
                                track_id, start_time, duration);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: midiclip <track_id> <start_time> <duration>");
                }
            } else {
                println!("Usage: midiclip <track_id> <start_time> <duration>");
            }
        } else if input.starts_with("note ") {
            // Parse: note <track_id> <clip_id> <time_offset> <note> <velocity> <duration>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 7 {
                if let (Ok(track_id), Ok(clip_id), Ok(time_offset), Ok(note), Ok(velocity), Ok(duration)) =
                    (parts[1].parse::<u32>(), parts[2].parse::<u32>(), parts[3].parse::<f64>(),
                     parts[4].parse::<u8>(), parts[5].parse::<u8>(), parts[6].parse::<f64>()) {
                    if note > 127 || velocity > 127 {
                        println!("Note and velocity must be 0-127");
                    } else {
                        controller.add_midi_note(track_id, clip_id, time_offset, note, velocity, duration);
                        println!("Added note {} (velocity {}) to clip {} on track {} at offset {:.2}s (duration {:.2}s)",
                                note, velocity, clip_id, track_id, time_offset, duration);
                    }
                } else {
                    println!("Invalid format. Usage: note <track_id> <clip_id> <time_offset> <note> <velocity> <duration>");
                }
            } else {
                println!("Usage: note <track_id> <clip_id> <time_offset> <note> <velocity> <duration>");
            }
        } else if input.starts_with("loadmidi ") {
            // Parse: loadmidi <track_id> <file_path> [start_time]
            let parts: Vec<&str> = input.splitn(4, ' ').collect();
            if parts.len() >= 3 {
                if let Ok(track_id) = parts[1].parse::<u32>() {
                    let file_path = parts[2];
                    let start_time = if parts.len() == 4 {
                        parts[3].parse::<f64>().unwrap_or(0.0)
                    } else {
                        0.0
                    };

                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);

                        // Load the MIDI file (this happens on the UI thread, not audio thread)
                        match load_midi_file(file_path, clip_id_counter, max_sample_rate) {
                            Ok(mut clip) => {
                                clip.start_time = start_time;
                                let event_count = clip.events.len();
                                let duration = clip.duration;
                                let clip_id = clip.id;
                                clip_id_counter += 1;

                                controller.add_loaded_midi_clip(track_id, clip);
                                println!("Loaded MIDI file '{}' to track {} as clip {} at {:.2}s ({} events, duration {:.2}s)",
                                        file_path, track_id, clip_id, start_time, event_count, duration);
                            }
                            Err(e) => {
                                println!("Error loading MIDI file: {}", e);
                            }
                        }
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: loadmidi <track_id> <file_path> [start_time]");
                }
            } else {
                println!("Usage: loadmidi <track_id> <file_path> [start_time]");
            }
        } else if input.starts_with("stretch ") {
            // Parse: stretch <track_id> <factor>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(stretch)) = (parts[1].parse::<u32>(), parts[2].parse::<f32>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.set_time_stretch(track_id, stretch);
                        let speed = if stretch < 0.99 {
                            format!("{:.0}% speed (slower)", stretch * 100.0)
                        } else if stretch > 1.01 {
                            format!("{:.0}% speed (faster)", stretch * 100.0)
                        } else {
                            "normal speed".to_string()
                        };
                        println!("Set time stretch on track {} to {:.2}x ({})", track_id, stretch, speed);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: stretch <track_id> <factor>");
                }
            } else {
                println!("Usage: stretch <track_id> <factor> (0.5=half speed, 1.0=normal, 2.0=double speed)");
            }
        } else if input.starts_with("offset ") {
            // Parse: offset <track_id> <seconds>
            let parts: Vec<&str> = input.split_whitespace().collect();
            if parts.len() == 3 {
                if let (Ok(track_id), Ok(offset)) = (parts[1].parse::<u32>(), parts[2].parse::<f64>()) {
                    let ids = track_ids.lock().unwrap();
                    if ids.contains(&track_id) {
                        drop(ids);
                        controller.set_offset(track_id, offset);
                        let direction = if offset > 0.01 {
                            format!("{:.2}s later", offset)
                        } else if offset < -0.01 {
                            format!("{:.2}s earlier", -offset)
                        } else {
                            "no offset".to_string()
                        };
                        println!("Set time offset on track {} to {:.2}s (content shifted {})", track_id, offset, direction);
                    } else {
                        println!("Invalid track ID. Available tracks: {:?}", *ids);
                    }
                } else {
                    println!("Invalid format. Usage: offset <track_id> <seconds>");
                }
            } else {
                println!("Usage: offset <track_id> <seconds> (positive=later, negative=earlier)");
            }
        } else if input == "stats" || input == "buffers" {
            controller.request_buffer_pool_stats();
        } else if input == "help" || input == "h" {
            print_help();
        } else {
            println!("Unknown command: {}. Type 'help' for commands.", input);
        }
    }

    // Drop the stream to stop playback
    drop(stream);
    println!("Goodbye!");

    Ok(())
}

fn print_help() {
    println!("\nTransport Commands:");
    println!("  ENTER           - Play");
    println!("  p, play         - Play");
    println!("  pause           - Pause");
    println!("  s, stop         - Stop and reset to beginning");
    println!("  seek <time>     - Seek to position in seconds (e.g. 'seek 10.5')");
    println!("\nTrack Commands:");
    println!("  tracks          - List all track IDs");
    println!("  volume <id> <v> - Set track volume (e.g. 'volume 0 0.5' for 50%)");
    println!("  mute <id>       - Mute a track");
    println!("  unmute <id>     - Unmute a track");
    println!("  solo <id>       - Solo a track (only soloed tracks play)");
    println!("  unsolo <id>     - Unsolo a track");
    println!("\nClip Commands:");
    println!("  clips           - List all clips");
    println!("  move <t> <c> <s> - Move clip to new timeline position");
    println!("                    (e.g. 'move 0 0 5.0' moves clip 0 on track 0 to 5.0s)");
    println!("\nEffect Commands:");
    println!("  gain <id> <db>  - Add/update gain effect (e.g. 'gain 0 6.0' for +6dB)");
    println!("  pan <id> <pan>  - Add/update pan effect (-1.0=left, 0.0=center, 1.0=right)");
    println!("  eq <id> <l> <m> <h> - Add/update 3-band EQ (low, mid, high in dB)");
    println!("                    (e.g. 'eq 0 3.0 0.0 -2.0')");
    println!("  clearfx <id>    - Clear all effects from a track");
    println!("\nMetatrack Commands:");
    println!("  meta <name>     - Create a new metatrack");
    println!("  addtometa <t> <m> - Add track to metatrack (e.g. 'addtometa 0 2')");
    println!("  removefrommeta <t> - Remove track from its parent metatrack");
    println!("  stretch <id> <f> - Set time stretch (0.5=half speed, 1.0=normal, 2.0=double)");
    println!("  offset <id> <s> - Set time offset in seconds (positive=later, negative=earlier)");
    println!("\nMIDI Commands:");
    println!("  midi <name>     - Create a new MIDI track");
    println!("  midiclip <t> <s> <d> - Create MIDI clip on track (start, duration)");
    println!("                    (e.g. 'midiclip 0 0.0 4.0')");
    println!("  note <t> <c> <o> <n> <v> <d> - Add note to MIDI clip");
    println!("                    (track, clip, time_offset, note, velocity, duration)");
    println!("                    (e.g. 'note 0 0 0.0 60 100 0.5' adds middle C)");
    println!("  loadmidi <t> <file> [start] - Load .mid file into track");
    println!("                    (e.g. 'loadmidi 0 song.mid 0.0')");
    println!("\nDiagnostics:");
    println!("  stats, buffers  - Show buffer pool statistics");
    println!("\nOther:");
    println!("  h, help         - Show this help");
    println!("  q, quit         - Quit");
    println!();
}

fn print_status(position: f64, duration: f64, track_ids: &[u32]) {
    println!("Position: {:.2}s / {:.2}s", position, duration);
    println!("Tracks: {:?}", track_ids);
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut engine: Engine,
) -> Result<cpal::Stream, Box<dyn std::error::Error>>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let err_fn = |err| eprintln!("Audio stream error: {}", err);

    // Preallocate a large buffer for format conversion to avoid allocations in audio callback
    // Size it generously to handle typical buffer sizes (up to 8192 samples = 2048 frames * stereo * 2x safety)
    let mut conversion_buffer = vec![0.0f32; 16384];

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            // NO MUTEX LOCK! Engine lives entirely on audio thread with ownership

            // Safety check - if buffer is too small, we have a problem
            if conversion_buffer.len() < data.len() {
                eprintln!("ERROR: Audio buffer size {} exceeds preallocated buffer size {}",
                         data.len(), conversion_buffer.len());
                return;
            }

            // Get a slice of the preallocated buffer
            let buffer_slice = &mut conversion_buffer[..data.len()];
            buffer_slice.fill(0.0);

            // Process audio - completely lock-free!
            engine.process(buffer_slice);

            // Convert f32 samples to output format
            for (i, sample) in data.iter_mut().enumerate() {
                *sample = cpal::Sample::from_sample(buffer_slice[i]);
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
