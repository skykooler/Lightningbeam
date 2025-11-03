use crate::audio::track::TrackId;
use crate::command::Command;
use midir::{MidiInput, MidiInputConnection};
use std::sync::{Arc, Mutex};

/// Manages external MIDI input devices and routes MIDI to the currently active track
pub struct MidiInputManager {
    connections: Vec<ActiveMidiConnection>,
    active_track_id: Arc<Mutex<Option<TrackId>>>,
}

struct ActiveMidiConnection {
    #[allow(dead_code)]
    device_name: String,
    #[allow(dead_code)]
    connection: MidiInputConnection<()>,
}

impl MidiInputManager {
    /// Create a new MIDI input manager and auto-connect to all available devices
    pub fn new(command_tx: rtrb::Producer<Command>) -> Result<Self, String> {
        let active_track_id = Arc::new(Mutex::new(None));
        let mut connections = Vec::new();

        // Wrap command producer in Arc<Mutex> for sharing across MIDI callbacks
        let shared_command_tx = Arc::new(Mutex::new(command_tx));

        // Initialize MIDI input
        let mut midi_in = MidiInput::new("Lightningbeam")
            .map_err(|e| format!("Failed to initialize MIDI input: {}", e))?;

        // Get all available MIDI input ports
        let ports = midi_in.ports();

        println!("MIDI Input: Found {} device(s)", ports.len());

        // We need to recreate MidiInput for each connection since connect() consumes it
        // Store port info first
        let mut port_infos = Vec::new();
        for port in &ports {
            if let Ok(port_name) = midi_in.port_name(port) {
                port_infos.push((port.clone(), port_name));
            }
        }

        // Connect to each available device
        for (port, port_name) in port_infos {
            println!("MIDI: Connecting to device: {}", port_name);

            // Recreate MidiInput for this connection
            midi_in = MidiInput::new("Lightningbeam")
                .map_err(|e| format!("Failed to recreate MIDI input: {}", e))?;

            let device_name = port_name.clone();
            let cmd_tx = shared_command_tx.clone();
            let active_id = active_track_id.clone();

            match midi_in.connect(
                &port,
                &format!("lightningbeam-{}", port_name),
                move |_timestamp, message, _| {
                    Self::on_midi_message(message, &cmd_tx, &active_id, &device_name);
                },
                (),
            ) {
                Ok(connection) => {
                    connections.push(ActiveMidiConnection {
                        device_name: port_name.clone(),
                        connection,
                    });
                    println!("MIDI: Connected to: {}", port_name);

                    // Need to recreate MidiInput for next iteration
                    midi_in = MidiInput::new("Lightningbeam")
                        .map_err(|e| format!("Failed to recreate MIDI input: {}", e))?;
                }
                Err(e) => {
                    eprintln!("MIDI: Failed to connect to {}: {}", port_name, e);
                    // Recreate MidiInput to continue with other ports
                    midi_in = MidiInput::new("Lightningbeam")
                        .map_err(|e| format!("Failed to recreate MIDI input: {}", e))?;
                }
            }
        }

        println!("MIDI Input: Connected to {} device(s)", connections.len());

        Ok(Self {
            connections,
            active_track_id,
        })
    }

    /// MIDI input callback - parses MIDI messages and sends commands to audio engine
    fn on_midi_message(
        message: &[u8],
        command_tx: &Mutex<rtrb::Producer<Command>>,
        active_track_id: &Arc<Mutex<Option<TrackId>>>,
        device_name: &str,
    ) {
        if message.is_empty() {
            return;
        }

        // Get the currently active track
        let track_id = {
            let active = active_track_id.lock().unwrap();
            match *active {
                Some(id) => id,
                None => {
                    // No active track, ignore MIDI input
                    return;
                }
            }
        };

        let status_byte = message[0];
        let status = status_byte & 0xF0;
        let _channel = status_byte & 0x0F;

        match status {
            0x90 => {
                // Note On
                if message.len() >= 3 {
                    let note = message[1];
                    let velocity = message[2];

                    // Treat velocity 0 as Note Off (per MIDI spec)
                    if velocity == 0 {
                        let mut tx = command_tx.lock().unwrap();
                        let _ = tx.push(Command::SendMidiNoteOff(track_id, note));
                        println!("MIDI [{}] Note Off: {} (velocity 0)", device_name, note);
                    } else {
                        let mut tx = command_tx.lock().unwrap();
                        let _ = tx.push(Command::SendMidiNoteOn(track_id, note, velocity));
                        println!("MIDI [{}] Note On: {} vel {}", device_name, note, velocity);
                    }
                }
            }
            0x80 => {
                // Note Off
                if message.len() >= 3 {
                    let note = message[1];
                    let mut tx = command_tx.lock().unwrap();
                    let _ = tx.push(Command::SendMidiNoteOff(track_id, note));
                    println!("MIDI [{}] Note Off: {}", device_name, note);
                }
            }
            0xB0 => {
                // Control Change
                if message.len() >= 3 {
                    let controller = message[1];
                    let value = message[2];
                    println!("MIDI [{}] CC: {} = {}", device_name, controller, value);
                    // TODO: Map to automation lanes in Phase 5
                }
            }
            0xE0 => {
                // Pitch Bend
                if message.len() >= 3 {
                    let lsb = message[1] as u16;
                    let msb = message[2] as u16;
                    let value = (msb << 7) | lsb;
                    println!("MIDI [{}] Pitch Bend: {}", device_name, value);
                    // TODO: Map to pitch automation in Phase 5
                }
            }
            _ => {
                // Other MIDI messages (aftertouch, program change, etc.)
                // Ignore for now
            }
        }
    }

    /// Set the currently active MIDI track
    pub fn set_active_track(&self, track_id: Option<TrackId>) {
        let mut active = self.active_track_id.lock().unwrap();
        *active = track_id;

        match track_id {
            Some(id) => println!("MIDI Input: Routing to track {}", id),
            None => println!("MIDI Input: No active track"),
        }
    }

    /// Get the number of connected devices
    pub fn device_count(&self) -> usize {
        self.connections.len()
    }
}
