use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use tungstenite::{accept, Message};

pub struct FrameStreamer {
    port: u16,
    clients: Arc<Mutex<Vec<tungstenite::WebSocket<std::net::TcpStream>>>>,
}

impl FrameStreamer {
    pub fn new() -> Result<Self, String> {
        // Bind to localhost on a random available port
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to create WebSocket listener: {}", e))?;

        let port = listener.local_addr()
            .map_err(|e| format!("Failed to get listener address: {}", e))?
            .port();

        // eprintln!("[Frame Streamer] WebSocket server started on port {}", port);

        let clients = Arc::new(Mutex::new(Vec::new()));
        let clients_clone = clients.clone();

        // Spawn acceptor thread
        thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        // eprintln!("[Frame Streamer] New WebSocket connection from {:?}", stream.peer_addr());
                        match accept(stream) {
                            Ok(websocket) => {
                                let mut clients = clients_clone.lock().unwrap();
                                clients.push(websocket);
                                // eprintln!("[Frame Streamer] Client connected, total clients: {}", clients.len());
                            }
                            Err(_e) => {
                                // eprintln!("[Frame Streamer] Failed to accept WebSocket: {}", e);
                            }
                        }
                    }
                    Err(_e) => {
                        // eprintln!("[Frame Streamer] Failed to accept connection: {}", e);
                    }
                }
            }
        });

        Ok(Self {
            port,
            clients,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Send a decoded frame to all connected clients
    /// Frame format: [pool_index: u32][timestamp_ms: u32][width: u32][height: u32][rgba_data...]
    pub fn send_frame(&self, pool_index: usize, timestamp: f64, width: u32, height: u32, rgba_data: &[u8]) {
        let mut clients = self.clients.lock().unwrap();

        // Build frame message (rgba_data is already in RGBA format from decoder)
        let mut frame_msg = Vec::with_capacity(16 + rgba_data.len());
        frame_msg.extend_from_slice(&(pool_index as u32).to_le_bytes());
        frame_msg.extend_from_slice(&((timestamp * 1000.0) as u32).to_le_bytes());
        frame_msg.extend_from_slice(&width.to_le_bytes());
        frame_msg.extend_from_slice(&height.to_le_bytes());
        frame_msg.extend_from_slice(rgba_data);

        // Send to all clients, remove disconnected ones
        clients.retain_mut(|client| {
            match client.write_message(Message::Binary(frame_msg.clone())) {
                Ok(_) => true,
                Err(_e) => {
                    // eprintln!("[Frame Streamer] Client disconnected: {}", e);
                    false
                }
            }
        });
    }
}
