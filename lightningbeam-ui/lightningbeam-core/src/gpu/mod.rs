// GPU rendering infrastructure for HDR compositing pipeline
//
// This module provides:
// - Buffer pooling for efficient render target management
// - Compositor for layer blending with proper opacity
// - Effect pipeline for GPU shader effects
// - Color space conversion (sRGB ↔ linear)

pub mod buffer_pool;
pub mod color_convert;
pub mod compositor;
pub mod effect_processor;
pub mod yuv_converter;

// Re-export commonly used types
pub use buffer_pool::{BufferHandle, BufferPool, BufferSpec, BufferFormat};
pub use color_convert::SrgbToLinearConverter;
pub use compositor::{Compositor, CompositorLayer, BlendMode};
pub use effect_processor::{EffectProcessor, EffectUniforms};
pub use yuv_converter::YuvConverter;

/// Standard HDR internal texture format (16-bit float per channel)
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Display output format (8-bit sRGB)
pub const DISPLAY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
