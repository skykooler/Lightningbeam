// Buffer pool for efficient render target management
//
// Provides acquire/release semantics for GPU textures used in the compositing pipeline.
// Buffers are reused when possible to minimize allocation overhead.

use wgpu;

/// Handle to a pooled render buffer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferHandle(pub(crate) u32);

impl BufferHandle {
    /// Returns the raw handle ID (for debugging)
    pub fn id(&self) -> u32 {
        self.0
    }
}

/// Texture format for render buffers
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BufferFormat {
    /// 8-bit linear (Vello output format - needs STORAGE_BINDING)
    /// Note: Using Rgba8Unorm instead of Rgba8UnormSrgb because sRGB doesn't support storage binding
    Rgba8Srgb,
    /// 16-bit float HDR (internal processing format)
    Rgba16Float,
}

impl BufferFormat {
    /// Convert to wgpu texture format
    pub fn to_wgpu(&self) -> wgpu::TextureFormat {
        match self {
            // Use Rgba8Unorm for Vello compatibility (STORAGE_BINDING required)
            // Vello handles color space conversion internally
            BufferFormat::Rgba8Srgb => wgpu::TextureFormat::Rgba8Unorm,
            BufferFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        }
    }
}

/// Specification for a render buffer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BufferSpec {
    pub width: u32,
    pub height: u32,
    pub format: BufferFormat,
}

impl BufferSpec {
    pub fn new(width: u32, height: u32, format: BufferFormat) -> Self {
        Self { width, height, format }
    }

    pub fn hdr(width: u32, height: u32) -> Self {
        Self::new(width, height, BufferFormat::Rgba16Float)
    }
}

/// Internal pooled buffer storage
struct PooledBuffer {
    handle: BufferHandle,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    spec: BufferSpec,
    in_use: bool,
    /// Frame counter when last used (for cleanup)
    last_used_frame: u64,
}

/// Buffer pool for render target management
///
/// Provides efficient allocation and reuse of GPU textures for the compositing pipeline.
/// Buffers are acquired for rendering and released when no longer needed.
pub struct BufferPool {
    buffers: Vec<PooledBuffer>,
    next_id: u32,
    current_frame: u64,
    /// Maximum number of unused frames before a buffer is eligible for cleanup
    max_unused_frames: u64,
}

impl BufferPool {
    /// Create a new empty buffer pool
    pub fn new() -> Self {
        Self {
            buffers: Vec::new(),
            next_id: 0,
            current_frame: 0,
            max_unused_frames: 60, // ~1 second at 60fps
        }
    }

    /// Acquire a buffer matching the given specification
    ///
    /// Returns a handle to a buffer that can be used for rendering.
    /// The buffer may be newly created or reused from the pool.
    pub fn acquire(&mut self, device: &wgpu::Device, spec: BufferSpec) -> BufferHandle {
        // First, try to find a free buffer with matching spec
        for buffer in &mut self.buffers {
            if !buffer.in_use && buffer.spec == spec {
                buffer.in_use = true;
                buffer.last_used_frame = self.current_frame;
                return buffer.handle;
            }
        }

        // No matching buffer found, create a new one
        let handle = BufferHandle(self.next_id);
        self.next_id += 1;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("pool_buffer_{}", handle.0)),
            size: wgpu::Extent3d {
                width: spec.width,
                height: spec.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: spec.format.to_wgpu(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        self.buffers.push(PooledBuffer {
            handle,
            texture,
            view,
            spec,
            in_use: true,
            last_used_frame: self.current_frame,
        });

        handle
    }

    /// Release a buffer back to the pool
    ///
    /// The buffer becomes available for reuse by future acquire calls.
    pub fn release(&mut self, handle: BufferHandle) {
        if let Some(buffer) = self.buffers.iter_mut().find(|b| b.handle == handle) {
            buffer.in_use = false;
        }
    }

    /// Get the texture view for a buffer handle
    pub fn get_view(&self, handle: BufferHandle) -> Option<&wgpu::TextureView> {
        self.buffers
            .iter()
            .find(|b| b.handle == handle)
            .map(|b| &b.view)
    }

    /// Get the texture for a buffer handle
    pub fn get_texture(&self, handle: BufferHandle) -> Option<&wgpu::Texture> {
        self.buffers
            .iter()
            .find(|b| b.handle == handle)
            .map(|b| &b.texture)
    }

    /// Get the spec for a buffer handle
    pub fn get_spec(&self, handle: BufferHandle) -> Option<BufferSpec> {
        self.buffers
            .iter()
            .find(|b| b.handle == handle)
            .map(|b| b.spec)
    }

    /// Check if a buffer is currently in use
    pub fn is_in_use(&self, handle: BufferHandle) -> bool {
        self.buffers
            .iter()
            .find(|b| b.handle == handle)
            .map(|b| b.in_use)
            .unwrap_or(false)
    }

    /// Advance to the next frame
    ///
    /// Call this once per frame to track buffer usage over time.
    pub fn next_frame(&mut self) {
        self.current_frame += 1;
    }

    /// Clear buffers that haven't been used for a while
    ///
    /// Removes buffers that are not in use and haven't been used for
    /// more than `max_unused_frames` frames.
    pub fn clear_unused(&mut self) {
        let current = self.current_frame;
        let max_unused = self.max_unused_frames;

        self.buffers.retain(|b| {
            b.in_use || (current - b.last_used_frame) < max_unused
        });
    }

    /// Force clear all unused buffers immediately
    pub fn clear_all_unused(&mut self) {
        self.buffers.retain(|b| b.in_use);
    }

    /// Get statistics about the pool
    pub fn stats(&self) -> BufferPoolStats {
        let total = self.buffers.len();
        let in_use = self.buffers.iter().filter(|b| b.in_use).count();
        let total_bytes: u64 = self.buffers.iter().map(|b| {
            let bytes_per_pixel = match b.spec.format {
                BufferFormat::Rgba8Srgb => 4,
                BufferFormat::Rgba16Float => 8,
            };
            (b.spec.width as u64) * (b.spec.height as u64) * bytes_per_pixel
        }).sum();

        BufferPoolStats {
            total_buffers: total,
            buffers_in_use: in_use,
            total_bytes,
        }
    }
}

impl Default for BufferPool {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about buffer pool usage
#[derive(Clone, Debug)]
pub struct BufferPoolStats {
    pub total_buffers: usize,
    pub buffers_in_use: usize,
    pub total_bytes: u64,
}

impl BufferPoolStats {
    pub fn total_megabytes(&self) -> f64 {
        self.total_bytes as f64 / (1024.0 * 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests require a wgpu device, so they're marked as ignored
    // Run with: cargo test -- --ignored

    #[test]
    #[ignore]
    fn test_buffer_pool_basics() {
        // Would need wgpu device setup for actual testing
    }
}
