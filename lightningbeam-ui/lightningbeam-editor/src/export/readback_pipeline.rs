//! Async triple-buffered GPU readback pipeline for video export
//!
//! This module implements a pipelined export system that overlaps GPU rendering
//! with CPU encoding to maximize throughput. It uses triple buffering to keep
//! both GPU and CPU busy simultaneously:
//!
//! - Frame N: GPU rendering/conversion
//! - Frame N-1: GPU→CPU async transfer
//! - Frame N-2: CPU encoding
//!
//! Expected speedup: 5x over synchronous blocking approach

use std::sync::mpsc::{channel, Receiver, Sender};

/// Result from a completed async buffer mapping
#[derive(Debug)]
pub struct ReadbackResult {
    pub buffer_id: usize,
    pub frame_num: usize,
    pub timestamp: f64,
}

/// State of a pipeline buffer in the triple-buffering state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BufferState {
    /// Buffer is available for new frame rendering
    Free,
    /// GPU is currently rendering/converting to this buffer
    Rendering,
    /// Buffer readback submitted, waiting for GPU→CPU transfer
    ReadbackPending,
    /// Buffer mapped and ready for CPU to read
    Mapped,
    /// CPU is encoding this buffer's data
    Encoding,
}

/// A single buffer in the triple-buffering pipeline
struct PipelineBuffer {
    id: usize,
    /// RGBA texture for GPU rendering output (Rgba8Unorm)
    rgba_texture: wgpu::Texture,
    rgba_texture_view: wgpu::TextureView,
    /// Staging buffer for GPU→CPU transfer (MAP_READ)
    staging_buffer: wgpu::Buffer,
    /// Current state in the pipeline
    state: BufferState,
    /// Frame metadata (set when rendering starts)
    frame_num: Option<usize>,
    timestamp: Option<f64>,
}

/// Handle to an acquired buffer for rendering
pub struct AcquiredBuffer {
    pub id: usize,
    pub rgba_texture_view: wgpu::TextureView,
}

/// Triple-buffered async readback pipeline
///
/// Manages 3 buffers cycling through the pipeline:
/// Free → Rendering → ReadbackPending → Mapped → Encoding → Free
pub struct ReadbackPipeline {
    buffers: Vec<PipelineBuffer>,
    /// Channel for async map_async callbacks
    readback_rx: Receiver<ReadbackResult>,
    readback_tx: Sender<ReadbackResult>,
    /// wgpu device and queue references (needed for polling and buffer operations)
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Buffer dimensions
    width: u32,
    height: u32,
}

impl ReadbackPipeline {
    /// Create a new triple-buffered readback pipeline
    ///
    /// # Arguments
    /// * `device` - GPU device (will be cloned for async operations)
    /// * `queue` - GPU queue (will be cloned for async operations)
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) -> Self {
        let (readback_tx, readback_rx) = channel();

        // Create 3 buffers for triple buffering
        let mut buffers = Vec::new();
        for id in 0..3 {
            // RGBA texture (Rgba8Unorm)
            let rgba_texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("readback_rgba_texture_{}", id)),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });

            let rgba_texture_view = rgba_texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Staging buffer for GPU→CPU readback
            let rgba_buffer_size = (width * height * 4) as u64; // Rgba8Unorm = 4 bytes/pixel
            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("readback_staging_buffer_{}", id)),
                size: rgba_buffer_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            buffers.push(PipelineBuffer {
                id,
                rgba_texture,
                rgba_texture_view,
                staging_buffer,
                state: BufferState::Free,
                frame_num: None,
                timestamp: None,
            });
        }

        Self {
            buffers,
            readback_rx,
            readback_tx,
            device: device.clone(),
            queue: queue.clone(),
            width,
            height,
        }
    }

    /// Acquire a free buffer for rendering (non-blocking)
    ///
    /// Returns None if all buffers are in use (caller should poll and retry)
    pub fn acquire(&mut self, frame_num: usize, timestamp: f64) -> Option<AcquiredBuffer> {
        // Find first Free buffer
        for buffer in &mut self.buffers {
            if buffer.state == BufferState::Free {
                buffer.state = BufferState::Rendering;
                buffer.frame_num = Some(frame_num);
                buffer.timestamp = Some(timestamp);

                return Some(AcquiredBuffer {
                    id: buffer.id,
                    rgba_texture_view: buffer.rgba_texture_view.clone(),
                });
            }
        }

        None // All buffers busy
    }

    /// Submit GPU commands and initiate async readback
    ///
    /// # Arguments
    /// * `buffer_id` - ID of the buffer to submit (from AcquiredBuffer)
    /// * `encoder` - Command encoder with rendering commands
    pub fn submit_and_readback(&mut self, buffer_id: usize, mut encoder: wgpu::CommandEncoder) {
        let buffer = &mut self.buffers[buffer_id];
        assert_eq!(buffer.state, BufferState::Rendering, "Buffer not in Rendering state");

        // Copy RGBA texture to staging buffer
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &buffer.rgba_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer.staging_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.width * 4), // Rgba8Unorm
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        // Submit GPU commands (non-blocking)
        self.queue.submit(Some(encoder.finish()));

        // Initiate async buffer mapping
        let frame_num = buffer.frame_num.unwrap();
        let timestamp = buffer.timestamp.unwrap();
        let tx = self.readback_tx.clone();

        buffer.staging_buffer.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            if result.is_ok() {
                let _ = tx.send(ReadbackResult {
                    buffer_id,
                    frame_num,
                    timestamp,
                });
            }
        });

        buffer.state = BufferState::ReadbackPending;
    }

    /// Poll for completed readbacks (non-blocking)
    ///
    /// Returns list of buffers that are now ready for CPU encoding.
    /// Call this frequently to process completed transfers.
    pub fn poll_nonblocking(&mut self) -> Vec<ReadbackResult> {
        // Poll GPU without blocking
        let _ = self.device.poll(wgpu::PollType::Poll);

        // Collect all completed readbacks
        let mut results = Vec::new();
        while let Ok(result) = self.readback_rx.try_recv() {
            // Update buffer state to Mapped
            self.buffers[result.buffer_id].state = BufferState::Mapped;
            results.push(result);
        }

        results
    }

    /// Extract RGBA data from mapped buffer (for CPU YUV conversion)
    ///
    /// Buffer must be in Mapped state (after poll_nonblocking returned it).
    /// This immediately copies the RGBA data, allowing the buffer to be released.
    pub fn extract_rgba_data(&mut self, buffer_id: usize) -> Vec<u8> {
        let buffer = &mut self.buffers[buffer_id];
        assert_eq!(buffer.state, BufferState::Mapped, "Buffer not in Mapped state");

        buffer.state = BufferState::Encoding;

        // Map the buffer and copy RGBA data
        let slice = buffer.staging_buffer.slice(..);
        let data = slice.get_mapped_range();

        // Simple copy - RGBA data goes to CPU for conversion
        data.to_vec()
    }

    /// Release buffer after encoding completes, returning it to the free pool
    ///
    /// # Arguments
    /// * `buffer_id` - ID of buffer to release
    pub fn release(&mut self, buffer_id: usize) {
        let buffer = &mut self.buffers[buffer_id];
        assert_eq!(buffer.state, BufferState::Encoding, "Buffer not in Encoding state");

        // Unmap buffer
        buffer.staging_buffer.unmap();

        // Clear metadata
        buffer.frame_num = None;
        buffer.timestamp = None;

        // Return to free pool
        buffer.state = BufferState::Free;
    }

    /// Flush pipeline and wait for all pending operations
    ///
    /// Call this at the end of export to ensure all frames are processed
    #[allow(dead_code)]
    pub fn flush(&mut self) -> Vec<ReadbackResult> {
        let mut all_results = Vec::new();

        // Keep polling until all buffers are Free
        loop {
            // Poll for new completions
            let _ = self.device.poll(wgpu::PollType::Poll);

            while let Ok(result) = self.readback_rx.try_recv() {
                self.buffers[result.buffer_id].state = BufferState::Mapped;
                all_results.push(result);
            }

            // Check if all buffers are Free (or can be made Free)
            let mut all_free = true;
            for buffer in &self.buffers {
                match buffer.state {
                    BufferState::Free => {},
                    BufferState::Rendering | BufferState::ReadbackPending => {
                        all_free = false;
                        break;
                    },
                    BufferState::Mapped | BufferState::Encoding => {
                        // These should be handled by the caller, shouldn't happen during flush
                        panic!("Buffer in {} state during flush - caller should encode and release",
                               if buffer.state == BufferState::Mapped { "Mapped" } else { "Encoding" });
                    }
                }
            }

            if all_free {
                break;
            }

            // Small sleep to avoid busy-waiting
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        all_results
    }

}
