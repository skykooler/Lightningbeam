//! GPU effect processor for shader-based visual effects
//!
//! Compiles effect shaders and applies them to textures in the compositing pipeline.

use crate::effect::{EffectDefinition, EffectInstance};
use std::collections::HashMap;
use uuid::Uuid;
use super::buffer_pool::{BufferHandle, BufferPool, BufferSpec, BufferFormat};

/// Uniform data for effect shaders
///
/// Parameters are packed as vec4s (4 floats each) for proper GPU alignment.
/// - params0: parameters 0-3
/// - params1: parameters 4-7
/// - params2: parameters 8-11
/// - params3: parameters 12-15
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EffectUniforms {
    /// Parameters 0-3 (packed as vec4 for 16-byte alignment)
    pub params0: [f32; 4],
    /// Parameters 4-7
    pub params1: [f32; 4],
    /// Parameters 8-11
    pub params2: [f32; 4],
    /// Parameters 12-15
    pub params3: [f32; 4],
    /// Source texture width
    pub texture_width: f32,
    /// Source texture height
    pub texture_height: f32,
    /// Current time in seconds
    pub time: f32,
    /// Mix/blend amount (0.0 = original, 1.0 = full effect)
    pub mix: f32,
}

impl Default for EffectUniforms {
    fn default() -> Self {
        Self {
            params0: [0.0; 4],
            params1: [0.0; 4],
            params2: [0.0; 4],
            params3: [0.0; 4],
            texture_width: 1.0,
            texture_height: 1.0,
            time: 0.0,
            mix: 1.0,
        }
    }
}

impl EffectUniforms {
    /// Set parameters from a flat array of up to 16 floats
    pub fn set_params(&mut self, params: &[f32]) {
        for (i, &val) in params.iter().take(16).enumerate() {
            match i / 4 {
                0 => self.params0[i % 4] = val,
                1 => self.params1[i % 4] = val,
                2 => self.params2[i % 4] = val,
                3 => self.params3[i % 4] = val,
                _ => {}
            }
        }
    }
}

/// A compiled effect ready for GPU execution
struct CompiledEffect {
    /// The render pipeline for this effect
    pipeline: wgpu::RenderPipeline,
}

/// GPU processor for visual effects
///
/// Manages shader compilation and execution for effect layers.
/// Effects are applied as fullscreen passes that read from a source texture
/// and write to a destination texture.
pub struct EffectProcessor {
    /// Compiled effect pipelines keyed by effect definition ID
    compiled_effects: HashMap<Uuid, CompiledEffect>,
    /// Bind group layout for effect shaders (shared across all effects)
    bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for texture sampling
    sampler: wgpu::Sampler,
    /// Output texture format
    output_format: wgpu::TextureFormat,
}

impl EffectProcessor {
    /// Create a new effect processor
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat) -> Self {
        // Create bind group layout matching effect shader expectations
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect_bind_group_layout"),
            entries: &[
                // Source texture (binding 0)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Sampler (binding 1)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Uniforms (binding 2)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Create sampler for effect textures
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("effect_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            compiled_effects: HashMap::new(),
            bind_group_layout,
            sampler,
            output_format,
        }
    }

    /// Compile an effect definition into a GPU pipeline
    ///
    /// Returns true if compilation was successful, false if the shader failed to compile.
    pub fn compile_effect(&mut self, device: &wgpu::Device, definition: &EffectDefinition) -> bool {
        // Check if already compiled
        if self.compiled_effects.contains_key(&definition.id) {
            return true;
        }

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&format!("effect_pipeline_layout_{}", definition.name)),
            bind_group_layouts: &[&self.bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create shader module from embedded WGSL
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&format!("effect_shader_{}", definition.name)),
            source: wgpu::ShaderSource::Wgsl(definition.shader_code.as_str().into()),
        });

        // Create render pipeline
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("effect_pipeline_{}", definition.name)),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: self.output_format,
                    // No blending - effect completely replaces the pixel
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.compiled_effects.insert(definition.id, CompiledEffect {
            pipeline,
        });

        true
    }

    /// Remove a compiled effect (e.g., when an effect definition is removed from the document)
    pub fn remove_effect(&mut self, effect_id: &Uuid) {
        self.compiled_effects.remove(effect_id);
    }

    /// Check if an effect is compiled
    pub fn is_compiled(&self, effect_id: &Uuid) -> bool {
        self.compiled_effects.contains_key(effect_id)
    }

    /// Apply an effect instance
    ///
    /// Renders from source_view to dest_view using the effect shader.
    /// Parameters are evaluated at the given time.
    pub fn apply_effect(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        definition: &EffectDefinition,
        instance: &EffectInstance,
        source_view: &wgpu::TextureView,
        dest_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        time: f64,
    ) -> bool {
        // Get compiled effect
        let Some(compiled) = self.compiled_effects.get(&definition.id) else {
            return false;
        };

        // Build uniforms from instance parameters
        let param_values = instance.get_uniform_params(time, &definition.parameters);
        let mut uniforms = EffectUniforms {
            texture_width: width as f32,
            texture_height: height as f32,
            time: time as f32,
            mix: instance.mix as f32,
            ..Default::default()
        };
        uniforms.set_params(&param_values);

        // Create uniform buffer
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect_uniforms"),
            size: std::mem::size_of::<EffectUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        // Create bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Render pass
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&format!("effect_pass_{}", definition.name)),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: dest_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&compiled.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..4, 0..1);

        true
    }

    /// Apply a chain of effects, ping-ponging between buffers
    ///
    /// This is the main entry point for applying multiple effects to a composition.
    /// Effects are applied in order, with the output of each becoming the input of the next.
    pub fn apply_effect_chain(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        buffer_pool: &mut BufferPool,
        definitions: &HashMap<Uuid, EffectDefinition>,
        instances: &[&EffectInstance],
        source: BufferHandle,
        width: u32,
        height: u32,
        time: f64,
    ) -> Option<BufferHandle> {
        if instances.is_empty() {
            return Some(source);
        }

        // We need two buffers for ping-ponging
        let spec = BufferSpec::new(width, height, BufferFormat::Rgba16Float);
        let mut current_source = source;
        let mut temp_buffer: Option<BufferHandle> = None;

        for instance in instances.iter() {
            // Skip disabled effects
            if !instance.enabled {
                continue;
            }

            // Get effect definition
            let Some(definition) = definitions.get(&instance.effect_id) else {
                continue;
            };

            // Acquire destination buffer (reuse temp buffer if available)
            let dest = if let Some(buf) = temp_buffer.take() {
                buf
            } else {
                buffer_pool.acquire(device, spec)
            };

            // Get views
            let Some(source_view) = buffer_pool.get_view(current_source) else {
                continue;
            };
            let Some(dest_view) = buffer_pool.get_view(dest) else {
                continue;
            };

            // Apply effect
            if self.apply_effect(
                device,
                queue,
                encoder,
                definition,
                instance,
                source_view,
                dest_view,
                width,
                height,
                time,
            ) {
                // Swap buffers for next iteration
                // Previous source becomes temp (can be reused)
                if current_source != source {
                    temp_buffer = Some(current_source);
                }
                current_source = dest;
            } else {
                // Effect failed, release the dest buffer
                buffer_pool.release(dest);
            }
        }

        // Release temp buffer if we still have one
        if let Some(buf) = temp_buffer {
            buffer_pool.release(buf);
        }

        // Return final result (if we processed any effects, it's different from source)
        Some(current_source)
    }

    /// Get the bind group layout (for external use if needed)
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    /// Get the number of compiled effects
    pub fn compiled_count(&self) -> usize {
        self.compiled_effects.len()
    }

    /// Clear all compiled effects (e.g., on device loss)
    pub fn clear(&mut self) {
        self.compiled_effects.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_effect_uniforms_size() {
        // Verify uniform struct is properly sized for GPU alignment
        let size = std::mem::size_of::<EffectUniforms>();
        // 16 floats (64 bytes) + 4 floats (16 bytes) = 80 bytes
        assert_eq!(size, 80);
    }

    #[test]
    fn test_effect_uniforms_default() {
        let uniforms = EffectUniforms::default();
        assert_eq!(uniforms.params0, [0.0; 4]);
        assert_eq!(uniforms.params1, [0.0; 4]);
        assert_eq!(uniforms.params2, [0.0; 4]);
        assert_eq!(uniforms.params3, [0.0; 4]);
        assert_eq!(uniforms.mix, 1.0);
    }

    #[test]
    fn test_effect_uniforms_set_params() {
        let mut uniforms = EffectUniforms::default();
        uniforms.set_params(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(uniforms.params0, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(uniforms.params1, [5.0, 6.0, 0.0, 0.0]);
    }
}
