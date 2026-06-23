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
    /// In YUV mode: packed planar YUV420p the compute shader writes (STORAGE | COPY_SRC).
    /// `None` in RGBA fallback mode.
    yuv_buffer: Option<wgpu::Buffer>,
    /// Staging buffer for GPU→CPU transfer (MAP_READ). Holds YUV in YUV mode, RGBA otherwise.
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
    /// `Some` when converting RGBA→YUV420p on the GPU (skips the CPU swscale pass and
    /// reads back ~3 MB of planar YUV instead of 8 MB RGBA). `None` falls back to RGBA
    /// readback + CPU conversion for dimensions the packed shader can't handle.
    gpu_yuv: Option<super::gpu_yuv::GpuYuv>,
}

impl ReadbackPipeline {
    /// Create a new triple-buffered readback pipeline
    ///
    /// # Arguments
    /// * `device` - GPU device (will be cloned for async operations)
    /// * `queue` - GPU queue (will be cloned for async operations)
    /// * `width` - Frame width in pixels
    /// * `height` - Frame height in pixels
    /// `enable_gpu_yuv` should be `true` only when the caller has verified the encoder's
    /// `YUV420P` plane strides are tight (== width / width-2), so the packed GPU planes
    /// drop straight into the `AVFrame` without row misalignment.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32, enable_gpu_yuv: bool) -> Self {
        let (readback_tx, readback_rx) = channel();

        // GPU YUV conversion when enabled AND the dimensions fit the packed shader; else RGBA + CPU.
        let gpu_yuv = if enable_gpu_yuv && super::gpu_yuv::supports(width, height) {
            Some(super::gpu_yuv::GpuYuv::new(device))
        } else {
            None
        };
        let yuv_mode = gpu_yuv.is_some();

        // Staging size: planar YUV420p (W*H*3/2) in YUV mode, else RGBA (W*H*4).
        let staging_size = if yuv_mode {
            super::gpu_yuv::yuv420p_len(width, height) as u64
        } else {
            (width * height * 4) as u64
        };

        // Create 3 buffers for triple buffering
        let mut buffers = Vec::new();
        for id in 0..3 {
            // RGBA texture (Rgba8Unorm). TEXTURE_BINDING lets the YUV compute shader read it.
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
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            let rgba_texture_view = rgba_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let yuv_buffer = if yuv_mode {
                Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("readback_yuv_buffer_{}", id)),
                    size: staging_size,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }))
            } else {
                None
            };

            // Staging buffer for GPU→CPU readback
            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("readback_staging_buffer_{}", id)),
                size: staging_size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            buffers.push(PipelineBuffer {
                id,
                rgba_texture,
                rgba_texture_view,
                yuv_buffer,
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
            gpu_yuv,
        }
    }

    /// `true` when frames are read back as planar YUV420p (GPU-converted) — the caller
    /// should slice planes with [`Self::split_yuv`] instead of running the CPU converter.
    pub fn is_yuv_mode(&self) -> bool {
        self.gpu_yuv.is_some()
    }

    /// Split a YUV-mode readback buffer into tight (Y, U, V) planes.
    pub fn split_yuv(&self, data: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let y = (self.width * self.height) as usize;
        let c = ((self.width / 2) * (self.height / 2)) as usize;
        (data[..y].to_vec(), data[y..y + c].to_vec(), data[y + c..y + 2 * c].to_vec())
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

        if let (Some(gpu_yuv), Some(yuv_buffer)) = (self.gpu_yuv.as_ref(), buffer.yuv_buffer.as_ref()) {
            // GPU RGBA→YUV420p, then copy the packed YUV buffer to staging (~3 MB).
            gpu_yuv.convert(&self.device, &mut encoder, &buffer.rgba_texture_view, yuv_buffer, self.width, self.height);
            encoder.copy_buffer_to_buffer(yuv_buffer, 0, &buffer.staging_buffer, 0, buffer.staging_buffer.size());
        } else {
            // Fallback: copy the RGBA texture to staging (8 MB), CPU converts later.
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
        }

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
