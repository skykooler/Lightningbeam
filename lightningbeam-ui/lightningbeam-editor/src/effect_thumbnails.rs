//! GPU-rendered effect thumbnails
//!
//! Generates preview thumbnails for effects by applying them to a source image
//! using the actual WGSL shaders.

use lightningbeam_core::effect::{EffectDefinition, EffectInstance};
use lightningbeam_core::gpu::effect_processor::EffectProcessor;
use std::collections::HashMap;
use uuid::Uuid;

/// Size of effect thumbnails in pixels
pub const EFFECT_THUMBNAIL_SIZE: u32 = 64;

use lightningbeam_core::gpu::{srgb_to_linear, linear_to_srgb};

/// sRGB-u8 RGBA → linear-`f16` RGBA bytes (little-endian). Feeds the effect
/// shaders linear light at float precision, matching the live HDR pipeline (an
/// 8-bit linear intermediate would band in shadows). RGB go through the sRGB
/// EOTF; alpha is linear.
fn srgb_image_to_linear_f16(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() * 2);
    for px in rgba.chunks_exact(4) {
        for &c in &px[..3] {
            out.extend_from_slice(&half::f16::from_f32(srgb_to_linear(c as f32 / 255.0)).to_le_bytes());
        }
        out.extend_from_slice(&half::f16::from_f32(px[3] as f32 / 255.0).to_le_bytes());
    }
    out
}

/// linear-`f16` RGBA bytes → sRGB-u8 RGBA. Inverse of [`srgb_image_to_linear_f16`].
fn linear_f16_to_srgb_image(f16_rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(f16_rgba.len() / 2);
    for texel in f16_rgba.chunks_exact(8) {
        let ch = |i: usize| half::f16::from_le_bytes([texel[i], texel[i + 1]]).to_f32();
        out.push((linear_to_srgb(ch(0)) * 255.0 + 0.5) as u8);
        out.push((linear_to_srgb(ch(2)) * 255.0 + 0.5) as u8);
        out.push((linear_to_srgb(ch(4)) * 255.0 + 0.5) as u8);
        out.push((ch(6).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    }
    out
}

/// Embedded still-life image for effect preview thumbnails
const EFFECT_PREVIEW_IMAGE_BYTES: &[u8] = include_bytes!("../../../src/assets/still-life.jpg");

/// Generator for GPU-rendered effect thumbnails
pub struct EffectThumbnailGenerator {
    /// Effect processor for compiling and applying shaders
    effect_processor: EffectProcessor,
    /// Source texture (still-life image scaled to thumbnail size)
    #[allow(dead_code)] // Must stay alive — source_view is a view into this texture
    source_texture: wgpu::Texture,
    /// View of the source texture
    source_view: wgpu::TextureView,
    /// Destination texture for rendered effects
    dest_texture: wgpu::Texture,
    /// View of the destination texture
    dest_view: wgpu::TextureView,
    /// Buffer for reading back rendered thumbnails
    readback_buffer: wgpu::Buffer,
    /// Cached rendered thumbnails (effect_id -> RGBA data)
    thumbnail_cache: HashMap<Uuid, Vec<u8>>,
    /// Effects that need thumbnail generation
    pending_effects: Vec<Uuid>,
}

impl EffectThumbnailGenerator {
    /// Create a new effect thumbnail generator
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // Load and decode the source image
        // The effect shaders operate in LINEAR light (matching the live HDR
        // pipeline, which feeds them a linear Rgba16Float texture). The preview
        // image is sRGB-encoded, so linearize it before upload and re-encode the
        // result after readback. This keeps thumbnails consistent with the live
        // render for every effect, including the gamma-space perceptual ones.
        // Linearize to f16 (float precision — an 8-bit linear intermediate would
        // band in shadows, the reason the live canvas is Rgba16Float).
        let source_f16 = srgb_image_to_linear_f16(&Self::load_source_image());

        // Effect processor + textures use Rgba16Float linear, matching the live
        // pipeline so thumbnails render identically to the on-canvas effect.
        let effect_processor = EffectProcessor::new(device, wgpu::TextureFormat::Rgba16Float);

        // Create source texture
        let source_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effect_thumbnail_source"),
            size: wgpu::Extent3d {
                width: EFFECT_THUMBNAIL_SIZE,
                height: EFFECT_THUMBNAIL_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload source image data (Rgba16Float = 8 bytes/texel).
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &source_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &source_f16,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(EFFECT_THUMBNAIL_SIZE * 8),
                rows_per_image: Some(EFFECT_THUMBNAIL_SIZE),
            },
            wgpu::Extent3d {
                width: EFFECT_THUMBNAIL_SIZE,
                height: EFFECT_THUMBNAIL_SIZE,
                depth_or_array_layers: 1,
            },
        );

        let source_view = source_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create destination texture
        let dest_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("effect_thumbnail_dest"),
            size: wgpu::Extent3d {
                width: EFFECT_THUMBNAIL_SIZE,
                height: EFFECT_THUMBNAIL_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let dest_view = dest_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create readback buffer (Rgba16Float = 8 bytes/texel, rows 256-aligned).
        let aligned_bytes_per_row = ((EFFECT_THUMBNAIL_SIZE * 8 + 255) / 256) * 256;
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect_thumbnail_readback"),
            size: (aligned_bytes_per_row * EFFECT_THUMBNAIL_SIZE) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        Self {
            effect_processor,
            source_texture,
            source_view,
            dest_texture,
            dest_view,
            readback_buffer,
            thumbnail_cache: HashMap::new(),
            pending_effects: Vec::new(),
        }
    }

    /// Load and resize the source image to thumbnail size
    fn load_source_image() -> Vec<u8> {
        // Try to load the embedded image
        if let Ok(img) = image::load_from_memory(EFFECT_PREVIEW_IMAGE_BYTES) {
            // Resize to thumbnail size
            let resized = img.resize_exact(
                EFFECT_THUMBNAIL_SIZE,
                EFFECT_THUMBNAIL_SIZE,
                image::imageops::FilterType::Lanczos3,
            );
            return resized.to_rgba8().into_raw();
        }

        // Fallback: generate a gradient image
        let size = EFFECT_THUMBNAIL_SIZE as usize;
        let mut rgba = vec![0u8; size * size * 4];
        for y in 0..size {
            for x in 0..size {
                let idx = (y * size + x) * 4;
                // Create a colorful gradient
                rgba[idx] = (x * 255 / size) as u8;     // R: horizontal gradient
                rgba[idx + 1] = (y * 255 / size) as u8; // G: vertical gradient
                rgba[idx + 2] = 128;                     // B: constant
                rgba[idx + 3] = 255;                     // A: opaque
            }
        }
        rgba
    }

    /// Request thumbnail generation for an effect
    pub fn request_thumbnail(&mut self, effect_id: Uuid) {
        if !self.thumbnail_cache.contains_key(&effect_id) && !self.pending_effects.contains(&effect_id) {
            self.pending_effects.push(effect_id);
        }
    }

    /// Get a cached thumbnail, or None if not yet generated
    #[allow(dead_code)]
    pub fn get_thumbnail(&self, effect_id: &Uuid) -> Option<&Vec<u8>> {
        self.thumbnail_cache.get(effect_id)
    }

    /// Check if a thumbnail is cached
    #[allow(dead_code)]
    pub fn has_thumbnail(&self, effect_id: &Uuid) -> bool {
        self.thumbnail_cache.contains_key(effect_id)
    }

    /// Invalidate a cached thumbnail (e.g., when effect shader changes)
    pub fn invalidate(&mut self, effect_id: &Uuid) {
        self.thumbnail_cache.remove(effect_id);
        self.effect_processor.remove_effect(effect_id);
    }

    /// Generate thumbnails for pending effects (call once per frame)
    ///
    /// Returns the number of thumbnails generated this frame.
    pub fn generate_pending(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        effect_definitions: &HashMap<Uuid, EffectDefinition>,
        max_per_frame: usize,
    ) -> usize {
        let mut generated = 0;

        while generated < max_per_frame && !self.pending_effects.is_empty() {
            let effect_id = self.pending_effects.remove(0);

            // Get effect definition
            let Some(definition) = effect_definitions.get(&effect_id) else {
                continue;
            };

            // Try to generate thumbnail
            if let Some(rgba) = self.render_effect_thumbnail(device, queue, definition) {
                self.thumbnail_cache.insert(effect_id, rgba);
                generated += 1;
            }
        }

        generated
    }

    /// Render a single effect thumbnail
    fn render_effect_thumbnail(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        definition: &EffectDefinition,
    ) -> Option<Vec<u8>> {
        // Compile the effect if not already compiled
        if !self.effect_processor.compile_effect(device, definition) {
            eprintln!("Failed to compile effect shader: {}", definition.name);
            return None;
        }

        // Create a default effect instance (default parameter values)
        let instance = EffectInstance::new(definition, 0.0, 1.0);

        // Create command encoder
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("effect_thumbnail_encoder"),
        });

        // Apply effect
        let success = self.effect_processor.apply_effect(
            device,
            queue,
            &mut encoder,
            definition,
            &instance,
            &self.source_view,
            &self.dest_view,
            EFFECT_THUMBNAIL_SIZE,
            EFFECT_THUMBNAIL_SIZE,
            0.0, // time = 0
        );

        if !success {
            eprintln!("Failed to apply effect: {}", definition.name);
            return None;
        }

        // Copy result to readback buffer (Rgba16Float = 8 bytes/texel).
        let aligned_bytes_per_row = ((EFFECT_THUMBNAIL_SIZE * 8 + 255) / 256) * 256;
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.dest_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(aligned_bytes_per_row),
                    rows_per_image: Some(EFFECT_THUMBNAIL_SIZE),
                },
            },
            wgpu::Extent3d {
                width: EFFECT_THUMBNAIL_SIZE,
                height: EFFECT_THUMBNAIL_SIZE,
                depth_or_array_layers: 1,
            },
        );

        // Submit commands
        queue.submit(std::iter::once(encoder.finish()));

        // Map buffer and read data
        let buffer_slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        // Wait for GPU
        let _ = device.poll(wgpu::PollType::wait_indefinitely());

        // Check if mapping succeeded
        if rx.recv().ok()?.is_err() {
            eprintln!("Failed to map readback buffer");
            return None;
        }

        // De-stride the linear-f16 result (drop the 256-byte row padding).
        let data = buffer_slice.get_mapped_range();
        let row_tight = (EFFECT_THUMBNAIL_SIZE * 8) as usize;
        let mut f16_rgba = Vec::with_capacity(row_tight * EFFECT_THUMBNAIL_SIZE as usize);
        for row in 0..EFFECT_THUMBNAIL_SIZE {
            let row_start = (row * aligned_bytes_per_row) as usize;
            f16_rgba.extend_from_slice(&data[row_start..row_start + row_tight]);
        }

        drop(data);
        self.readback_buffer.unmap();

        // Result is linear f16 (the effect ran in linear light); re-encode to
        // sRGB-u8 for display, mirroring the live pipeline's linear→sRGB output.
        Some(linear_f16_to_srgb_image(&f16_rgba))
    }

    /// Get all effect IDs that have pending thumbnail requests
    pub fn pending_count(&self) -> usize {
        self.pending_effects.len()
    }

    /// Get read-only access to the thumbnail cache
    pub fn thumbnail_cache(&self) -> &HashMap<Uuid, Vec<u8>> {
        &self.thumbnail_cache
    }

    /// Add multiple thumbnail requests at once
    pub fn request_thumbnails(&mut self, effect_ids: &[Uuid]) {
        for id in effect_ids {
            self.request_thumbnail(*id);
        }
    }
}
