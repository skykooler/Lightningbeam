use std::sync::atomic::{AtomicUsize, Ordering};

/// Pool of reusable audio buffers for recursive group rendering
///
/// This pool allows groups to acquire temporary buffers for submixing
/// child tracks without allocating memory in the audio thread.
pub struct BufferPool {
    buffers: Vec<Vec<f32>>,
    available: Vec<usize>,
    buffer_size: usize,
    /// Tracks the number of times a buffer had to be allocated (not reused)
    /// This should be zero during steady-state playback
    total_allocations: AtomicUsize,
    /// Peak number of buffers simultaneously in use
    peak_usage: AtomicUsize,
}

impl BufferPool {
    /// Create a new buffer pool
    ///
    /// # Arguments
    /// * `initial_capacity` - Number of buffers to pre-allocate
    /// * `buffer_size` - Size of each buffer in samples
    pub fn new(initial_capacity: usize, buffer_size: usize) -> Self {
        let mut buffers = Vec::with_capacity(initial_capacity);
        let mut available = Vec::with_capacity(initial_capacity);

        // Pre-allocate buffers
        for i in 0..initial_capacity {
            buffers.push(vec![0.0; buffer_size]);
            available.push(i);
        }

        Self {
            buffers,
            available,
            buffer_size,
            total_allocations: AtomicUsize::new(0),
            peak_usage: AtomicUsize::new(0),
        }
    }

    /// Acquire a buffer from the pool
    ///
    /// Returns a zeroed buffer ready for use. If no buffers are available,
    /// allocates a new one (though this should be avoided in the audio thread).
    pub fn acquire(&mut self) -> Vec<f32> {
        // Track peak usage
        let current_in_use = self.buffers.len() - self.available.len();
        let peak = self.peak_usage.load(Ordering::Relaxed);
        if current_in_use > peak {
            self.peak_usage.store(current_in_use, Ordering::Relaxed);
        }

        if let Some(idx) = self.available.pop() {
            // Reuse an existing buffer
            let mut buf = std::mem::take(&mut self.buffers[idx]);
            buf.fill(0.0);
            buf
        } else {
            // No buffers available, allocate a new one
            // This should be rare if the pool is sized correctly
            self.total_allocations.fetch_add(1, Ordering::Relaxed);
            vec![0.0; self.buffer_size]
        }
    }

    /// Release a buffer back to the pool
    ///
    /// # Arguments
    /// * `buffer` - The buffer to return to the pool
    pub fn release(&mut self, buffer: Vec<f32>) {
        // Only add to pool if it's the correct size
        if buffer.len() == self.buffer_size {
            let idx = self.buffers.len();
            self.buffers.push(buffer);
            self.available.push(idx);
        }
        // Otherwise, drop the buffer (wrong size, shouldn't happen normally)
    }

    /// Get the configured buffer size
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Get the number of available buffers
    pub fn available_count(&self) -> usize {
        self.available.len()
    }

    /// Get the total number of buffers in the pool
    pub fn total_count(&self) -> usize {
        self.buffers.len()
    }

    /// Get the total number of allocations that occurred (excluding pre-allocated buffers)
    ///
    /// This should be zero during steady-state playback. If non-zero, the pool
    /// should be resized to avoid allocations in the audio thread.
    pub fn allocation_count(&self) -> usize {
        self.total_allocations.load(Ordering::Relaxed)
    }

    /// Get the peak number of buffers simultaneously in use
    ///
    /// Use this to determine the optimal initial_capacity for your workload.
    pub fn peak_usage(&self) -> usize {
        self.peak_usage.load(Ordering::Relaxed)
    }

    /// Reset allocation statistics
    ///
    /// Useful for benchmarking steady-state performance after warmup.
    pub fn reset_stats(&mut self) {
        self.total_allocations.store(0, Ordering::Relaxed);
        self.peak_usage.store(0, Ordering::Relaxed);
    }

    /// Get comprehensive pool statistics
    pub fn stats(&self) -> BufferPoolStats {
        BufferPoolStats {
            total_buffers: self.total_count(),
            available_buffers: self.available_count(),
            in_use_buffers: self.total_count() - self.available_count(),
            peak_usage: self.peak_usage(),
            total_allocations: self.allocation_count(),
            buffer_size: self.buffer_size,
        }
    }
}

/// Statistics about buffer pool usage
#[derive(Debug, Clone, Copy)]
pub struct BufferPoolStats {
    pub total_buffers: usize,
    pub available_buffers: usize,
    pub in_use_buffers: usize,
    pub peak_usage: usize,
    pub total_allocations: usize,
    pub buffer_size: usize,
}

impl Default for BufferPool {
    fn default() -> Self {
        // Default: 8 buffers of 4096 samples (enough for 85ms at 48kHz stereo)
        Self::new(8, 4096)
    }
}
