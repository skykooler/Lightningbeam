use daw_backend::{AudioEvent, AudioSystem, EventEmitter};
use daw_backend::tui::run_tui;
use std::env;
use std::sync::{Arc, Mutex};

/// Event emitter that pushes events to a ringbuffer for the TUI
struct TuiEventEmitter {
    tx: Arc<Mutex<rtrb::Producer<AudioEvent>>>,
}

impl TuiEventEmitter {
    fn new(tx: rtrb::Producer<AudioEvent>) -> Self {
        Self {
            tx: Arc::new(Mutex::new(tx)),
        }
    }
}

impl EventEmitter for TuiEventEmitter {
    fn emit(&self, event: AudioEvent) {
        if let Ok(mut tx) = self.tx.lock() {
            let _ = tx.push(event);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check if user wants the old CLI mode
    let args: Vec<String> = env::args().collect();
    if args.len() > 1 && args[1] == "--help" {
        print_usage();
        return Ok(());
    }

    println!("Lightningbeam DAW - Starting TUI...\n");
    println!("Controls:");
    println!("  ESC         - Enter Command mode (type commands like 'track MyTrack')");
    println!("  i           - Enter Play mode (play MIDI notes with keyboard)");
    println!("  awsedftgyhujkolp;' - Play MIDI notes (chromatic scale in Play mode)");
    println!("  r           - Release all notes (in Play mode)");
    println!("  SPACE       - Play/Pause");
    println!("  Ctrl+Q      - Quit");
    println!("\nStarting audio system...");

    // Create event channel for TUI
    let (event_tx, event_rx) = rtrb::RingBuffer::new(256);
    let emitter = Arc::new(TuiEventEmitter::new(event_tx));

    // Initialize audio system with event emitter
    let mut audio_system = AudioSystem::new(Some(emitter))?;

    println!("Audio system initialized:");
    println!("  Sample rate: {} Hz", audio_system.sample_rate);
    println!("  Channels: {}", audio_system.channels);

    // Create a test MIDI track to verify event handling
    audio_system.controller.create_midi_track("Test Track".to_string());

    println!("\nTUI starting...\n");
    std::thread::sleep(std::time::Duration::from_millis(100)); // Give time for event

    // Wrap event receiver for TUI
    let event_rx = Arc::new(Mutex::new(event_rx));

    // Run the TUI
    run_tui(audio_system.controller, event_rx)?;

    println!("\nGoodbye!");
    Ok(())
}

fn print_usage() {
    println!("Lightningbeam DAW - Terminal User Interface");
    println!("\nUsage: {} [OPTIONS]", env::args().next().unwrap());
    println!("\nOptions:");
    println!("  --help      Show this help message");
    println!("\nThe DAW will start in TUI mode with an empty project.");
    println!("Use commands to create tracks and load audio:");
    println!("  :track <name>         - Create MIDI track");
    println!("  :audiotrack <name>    - Create audio track");
    println!("  :play                 - Start playback");
    println!("  :stop                 - Stop playback");
    println!("  :quit                 - Exit application");
}
