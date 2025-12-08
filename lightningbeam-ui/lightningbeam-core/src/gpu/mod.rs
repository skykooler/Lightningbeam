// GPU rendering infrastructure for HDR compositing pipeline
//
// This module provides:
// - Buffer pooling for efficient render target management
// - Compositor for layer blending with proper opacity
// - Effect pipeline for GPU shader effects

pub mod buffer_pool;
pub mod compositor;

// Re-export commonly used types
pub use buffer_pool::{BufferHandle, BufferPool, BufferSpec, BufferFormat};
pub use compositor::{Compositor, CompositorLayer, BlendMode};

/// Standard HDR internal texture format (16-bit float per channel)
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Display output format (8-bit sRGB)
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
