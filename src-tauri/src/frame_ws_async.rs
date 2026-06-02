use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use flume::{Sender, unbounded};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::Duration;

/// Playback state for a video pool
#[derive(Clone, Debug)]
pub struct PlaybackState {
    pub is_playing: bool,
    pub target_fps: f64,
    pub current_time: f64,
}

/// Shared server state
pub struct ServerState {
    /// Connected clients
    clients: RwLock<Vec<Sender<Vec<u8>>>>,
    /// Playback state per pool index
    playback_state: RwLock<HashMap<usize, PlaybackState>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self {
            clients: RwLock::new(Vec::new()),
            playback_state: RwLock::new(HashMap::new()),
        }
    }

    /// Register a new client
    pub async fn add_client(&self, sender: Sender<Vec<u8>>) {
        let mut clients = self.clients.write().await;
        clients.push(sender);
        eprintln!("[Async Frame Streamer] Client registered, total: {}", clients.len());
    }

    /// Broadcast a frame to all connected clients
    pub async fn broadcast_frame(&self, frame_data: Vec<u8>) {
        let clients = self.clients.read().await;

        // Send to all clients
        for client in clients.iter() {
            // Non-blocking send, drop frame if client is slow
            let _ = client.try_send(frame_data.clone());
        }
    }

    /// Remove disconnected clients
    pub async fn cleanup_clients(&self) {
        let mut clients = self.clients.write().await;
        clients.retain(|client| !client.is_disconnected());
        eprintln!("[Async Frame Streamer] Cleaned up clients, remaining: {}", clients.len());
    }

    /// Update playback state for a pool
    pub async fn set_playback_state(&self, pool_index: usize, state: PlaybackState) {
        let mut states = self.playback_state.write().await;
        states.insert(pool_index, state);
    }

    /// Get playback state for a pool
    pub async fn get_playback_state(&self, pool_index: usize) -> Option<PlaybackState> {
        let states = self.playback_state.read().await;
        states.get(&pool_index).cloned()
    }
}

pub struct AsyncFrameStreamer {
    port: u16,
    state: Arc<ServerState>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl AsyncFrameStreamer {
    pub async fn new() -> Result<Self, String> {
        let state = Arc::new(ServerState::new());

        // Create router with WebSocket upgrade handler
        let app_state = state.clone();
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(app_state);

        // Bind to localhost on a random port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Failed to bind: {}", e))?;

        let port = listener
            .local_addr()
            .map_err(|e| format!("Failed to get address: {}", e))?
            .port();

        eprintln!("[Async Frame Streamer] WebSocket server starting on port {}", port);

        // Spawn server task
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
                .expect("Server error");
        });

        eprintln!("[Async Frame Streamer] Server started");

        Ok(Self {
            port,
            state,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Send a frame to all connected clients for a specific pool
    /// Frame format: [pool_index: u32][timestamp_ms: u32][width: u32][height: u32][rgba_data...]
    pub async fn send_frame(&self, pool_index: usize, timestamp: f64, width: u32, height: u32, rgba_data: &[u8]) {
        // Build frame message
        let mut frame_msg = Vec::with_capacity(16 + rgba_data.len());
        frame_msg.extend_from_slice(&(pool_index as u32).to_le_bytes());
        frame_msg.extend_from_slice(&((timestamp * 1000.0) as u32).to_le_bytes());
        frame_msg.extend_from_slice(&width.to_le_bytes());
        frame_msg.extend_from_slice(&height.to_le_bytes());
        frame_msg.extend_from_slice(rgba_data);

        // Broadcast to all connected clients
        self.state.broadcast_frame(frame_msg).await;
    }

    /// Start streaming frames for a pool at a target FPS
    pub async fn start_stream(&self, pool_index: usize, fps: f64) {
        let state = PlaybackState {
            is_playing: true,
            target_fps: fps,
            current_time: 0.0,
        };
        self.state.set_playback_state(pool_index, state).await;
        eprintln!("[Async Frame Streamer] Started streaming pool {} at {} FPS", pool_index, fps);
    }

    /// Stop streaming frames for a pool
    pub async fn stop_stream(&self, pool_index: usize) {
        if let Some(mut state) = self.state.get_playback_state(pool_index).await {
            state.is_playing = false;
            self.state.set_playback_state(pool_index, state).await;
            eprintln!("[Async Frame Streamer] Stopped streaming pool {}", pool_index);
        }
    }

    /// Seek to a specific time in a pool
    pub async fn seek(&self, pool_index: usize, timestamp: f64) {
        if let Some(mut state) = self.state.get_playback_state(pool_index).await {
            state.current_time = timestamp;
            self.state.set_playback_state(pool_index, state).await;
        }
    }
}

impl Drop for AsyncFrameStreamer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// WebSocket handler
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<ServerState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Handle individual WebSocket connection
async fn handle_socket(mut socket: WebSocket, state: Arc<ServerState>) {
    eprintln!("[Async Frame Streamer] New WebSocket connection");

    // Create a channel for this client
    let (tx, rx) = unbounded::<Vec<u8>>();

    // Register this client
    state.add_client(tx).await;

    // Spawn task to send frames to this client
    let mut rx = rx;
    let mut send_task = tokio::spawn(async move {
        while let Ok(frame) = rx.recv_async().await {
            if socket.send(Message::Binary(frame)).await.is_err() {
                eprintln!("[Async Frame Streamer] Failed to send frame to client");
                break;
            }
        }
        eprintln!("[Async Frame Streamer] Send task ended");
    });

    // Keep connection alive with ping/pong
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                // Connection alive, no need to ping in this simple implementation
            }
            _ = &mut send_task => {
                eprintln!("[Async Frame Streamer] Send task completed, closing connection");
                break;
            }
        }
    }

    // Cleanup
    state.cleanup_clients().await;
}
