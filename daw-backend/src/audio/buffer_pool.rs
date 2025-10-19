/// Pool of reusable audio buffers for recursive group rendering
///
/// This pool allows groups to acquire temporary buffers for submixing
/// child tracks without allocating memory in the audio thread.
pub struct BufferPool {
    buffers: Vec<Vec<f32>>,
    available: Vec<usize>,
    buffer_size: usize,
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
        }
    }

    /// Acquire a buffer from the pool
    ///
    /// Returns a zeroed buffer ready for use. If no buffers are available,
    /// allocates a new one (though this should be avoided in the audio thread).
    pub fn acquire(&mut self) -> Vec<f32> {
        if let Some(idx) = self.available.pop() {
            // Reuse an existing buffer
            let mut buf = std::mem::take(&mut self.buffers[idx]);
            buf.fill(0.0);
            buf
        } else {
            // No buffers available, allocate a new one
            // This should be rare if the pool is sized correctly
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
}

impl Default for BufferPool {
    fn default() -> Self {
        // Default: 8 buffers of 4096 samples (enough for 85ms at 48kHz stereo)
        Self::new(8, 4096)
    }
}
