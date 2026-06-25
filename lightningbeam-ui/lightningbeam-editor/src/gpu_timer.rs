//! Minimal GPU timestamp timer for the composite pipeline.
//!
//! Brackets a section of GPU work with two timestamps and reads the elapsed GPU
//! time back asynchronously (no pipeline stall). Used to attribute the per-frame
//! composite cost (Vello render + sRGB→linear + compositor + tonemap) shown in F3.
//!
//! Requires `TIMESTAMP_QUERY` + `TIMESTAMP_QUERY_INSIDE_ENCODERS`; [`FrameGpuTimer::new`]
//! returns `None` when the adapter doesn't support them, and all call sites no-op.

use std::sync::{Arc, Mutex};

/// State of the single readback buffer (shared with the map callback).
#[derive(Clone, Copy, PartialEq)]
enum Readback {
    /// Available to resolve into this frame.
    Free,
    /// Submitted + `map_async` in flight; don't touch until the callback fires.
    Mapping,
    /// Mapped and ready to read.
    Ready,
}

/// Times one GPU section (two timestamps) per frame with intermittent async readback.
pub struct FrameGpuTimer {
    query_set: wgpu::QuerySet,
    resolve_buf: wgpu::Buffer,
    readback_buf: wgpu::Buffer,
    state: Arc<Mutex<Readback>>,
    /// Nanoseconds per timestamp tick.
    period_ns: f32,
    /// Most recent measured GPU time for the bracketed section, in milliseconds.
    last_ms: f64,
}

impl FrameGpuTimer {
    /// Required device features for GPU timestamp timing.
    pub fn required_features() -> wgpu::Features {
        wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS
    }

    /// Create a timer, or `None` if the device lacks timestamp support.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(Self::required_features()) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("composite_gpu_timer"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        // 2 timestamps × u64.
        let size = 2 * std::mem::size_of::<u64>() as u64;
        let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite_gpu_timer_resolve"),
            size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("composite_gpu_timer_readback"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            query_set,
            resolve_buf,
            readback_buf,
            state: Arc::new(Mutex::new(Readback::Free)),
            period_ns: queue.get_timestamp_period(),
            last_ms: 0.0,
        })
    }

    /// Write the **start** timestamp (call just before the bracketed GPU work).
    pub fn start(&self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("composite_gpu_timer_start"),
        });
        enc.write_timestamp(&self.query_set, 0);
        queue.submit(Some(enc.finish()));
    }

    /// Write the **end** timestamp and, if the readback buffer is free, resolve +
    /// kick off an async read. Also consumes a previously-completed read into
    /// `last_ms`. Call just after the bracketed GPU work.
    pub fn end(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // 1. Consume a completed readback first (so the buffer is free to reuse).
        let cur = *self.state.lock().unwrap();
        if cur == Readback::Ready {
            {
                let view = self.readback_buf.slice(..).get_mapped_range();
                let t0 = u64::from_le_bytes(view[0..8].try_into().unwrap());
                let t1 = u64::from_le_bytes(view[8..16].try_into().unwrap());
                // Timestamps can wrap or arrive out of order across queue resets; guard.
                let ticks = t1.saturating_sub(t0);
                self.last_ms = ticks as f64 * self.period_ns as f64 / 1.0e6;
            }
            self.readback_buf.unmap();
            *self.state.lock().unwrap() = Readback::Free;
        }

        // 2. End timestamp + resolve + copy, only when the buffer is free.
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("composite_gpu_timer_end"),
        });
        enc.write_timestamp(&self.query_set, 1);

        let can_read = *self.state.lock().unwrap() == Readback::Free;
        if can_read {
            enc.resolve_query_set(&self.query_set, 0..2, &self.resolve_buf, 0);
            enc.copy_buffer_to_buffer(
                &self.resolve_buf,
                0,
                &self.readback_buf,
                0,
                2 * std::mem::size_of::<u64>() as u64,
            );
        }
        queue.submit(Some(enc.finish()));

        if can_read {
            *self.state.lock().unwrap() = Readback::Mapping;
            let state = Arc::clone(&self.state);
            self.readback_buf.slice(..).map_async(wgpu::MapMode::Read, move |res| {
                *state.lock().unwrap() = if res.is_ok() { Readback::Ready } else { Readback::Free };
            });
        }
    }

    /// Most recently measured GPU time of the bracketed section, in milliseconds.
    pub fn last_ms(&self) -> f64 {
        self.last_ms
    }
}
