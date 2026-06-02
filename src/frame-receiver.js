// WebSocket frame receiver for zero-copy video playback
// Uses ArrayBuffer views to avoid copying data

export class FrameReceiver {
  constructor() {
    this.ws = null;
    this.port = null;
    this.connected = false;
    this.frameCallbacks = new Map(); // pool_index -> callback(imageData, timestamp)
  }

  async connect() {
    // Get WebSocket port from Tauri
    const { invoke } = window.__TAURI__.core;
    this.port = await invoke('get_frame_streamer_port');

    const wsUrl = `ws://127.0.0.1:${this.port}`;
    console.log(`[FrameReceiver] Connecting to ${wsUrl}`);

    return new Promise((resolve, reject) => {
      this.ws = new WebSocket(wsUrl);
      this.ws.binaryType = 'arraybuffer'; // Important: receive as ArrayBuffer for zero-copy

      this.ws.onopen = () => {
        console.log('[FrameReceiver] Connected');
        this.connected = true;
        resolve();
      };

      this.ws.onerror = (error) => {
        console.error('[FrameReceiver] WebSocket error:', error);
        reject(error);
      };

      this.ws.onclose = () => {
        console.log('[FrameReceiver] Disconnected');
        this.connected = false;
      };

      this.ws.onmessage = (event) => {
        this.handleFrame(event.data);
      };
    });
  }

  handleFrame(arrayBuffer) {
    // Frame format: [pool_index: u32][timestamp_ms: u32][width: u32][height: u32][rgba_data...]

    // Create DataView for reading header (zero-copy view into buffer)
    const view = new DataView(arrayBuffer);
    const poolIndex = view.getUint32(0, true); // little-endian
    const timestampMs = view.getUint32(4, true);
    const width = view.getUint32(8, true);
    const height = view.getUint32(12, true);

    // Get callback for this pool
    const callback = this.frameCallbacks.get(poolIndex);
    if (!callback) {
      // No subscriber for this pool
      return;
    }

    // Create zero-copy view of RGBA data (starts at byte 16)
    // IMPORTANT: Uint8ClampedArray is required for ImageData
    // Specify exact length to avoid stride issues
    const dataLength = width * height * 4;
    const rgbaData = new Uint8ClampedArray(arrayBuffer, 16, dataLength);


    // Create ImageData directly from the view (zero-copy!)
    const imageData = new ImageData(rgbaData, width, height);


    // Call subscriber with frame data
    const timestamp = timestampMs / 1000.0;
    callback(imageData, timestamp);
  }

  // Subscribe to frames for a specific video pool
  subscribe(poolIndex, callback) {
    console.log(`[FrameReceiver] Subscribing to pool ${poolIndex}`);
    this.frameCallbacks.set(poolIndex, callback);
  }

  // Unsubscribe from a video pool
  unsubscribe(poolIndex) {
    console.log(`[FrameReceiver] Unsubscribing from pool ${poolIndex}`);
    this.frameCallbacks.delete(poolIndex);
  }

  disconnect() {
    if (this.ws) {
      this.ws.close();
      this.ws = null;
    }
    this.connected = false;
    this.frameCallbacks.clear();
  }
}

// Global singleton instance
export const frameReceiver = new FrameReceiver();
