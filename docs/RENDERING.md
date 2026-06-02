# GPU Rendering Architecture

This document describes Lightningbeam's GPU rendering pipeline, including Vello integration for vector graphics, custom WGSL shaders for waveforms, and wgpu integration patterns.

## Table of Contents

- [Overview](#overview)
- [Rendering Pipeline](#rendering-pipeline)
- [Vello Integration](#vello-integration)
- [Waveform Rendering](#waveform-rendering)
- [WGSL Shaders](#wgsl-shaders)
- [Uniform Buffer Alignment](#uniform-buffer-alignment)
- [Custom wgpu Integration](#custom-wgpu-integration)
- [Performance Optimization](#performance-optimization)
- [Debugging Rendering Issues](#debugging-rendering-issues)

## Overview

Lightningbeam uses GPU-accelerated rendering for high-performance 2D graphics:

- **Vello**: Compute shader-based 2D vector rendering
- **wgpu 27**: Cross-platform GPU API (Vulkan, Metal, D3D12)
- **egui-wgpu**: Integration layer between egui and wgpu
- **Custom WGSL shaders**: For specialized rendering (waveforms, effects)

### Supported Backends

- **Linux**: Vulkan (primary), OpenGL (fallback)
- **macOS**: Metal
- **Windows**: Vulkan, DirectX 12

## Rendering Pipeline

### High-Level Flow

```
┌─────────────────────────────────────────────────────────────┐
│                     Application Frame                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. egui Layout Phase                                        │
│     - Build UI tree                                          │
│     - Collect paint primitives                               │
│     - Register wgpu callbacks                                │
│                                                              │
│  2. Custom GPU Rendering (via egui_wgpu::Callback)          │
│     ┌────────────────────────────────────────────────┐      │
│     │  prepare():                                    │      │
│     │    - Build Vello scene from document          │      │
│     │    - Update uniform buffers                    │      │
│     │    - Generate waveform mipmaps (if needed)     │      │
│     └────────────────────────────────────────────────┘      │
│     ┌────────────────────────────────────────────────┐      │
│     │  paint():                                      │      │
│     │    - Render Vello scene to texture            │      │
│     │    - Render waveforms                          │      │
│     │    - Composite layers                          │      │
│     └────────────────────────────────────────────────┘      │
│                                                              │
│  3. egui Paint                                               │
│     - Render egui UI elements                                │
│     - Composite with custom rendering                        │
│                                                              │
│  4. Present to Screen                                        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Render Pass Structure

```
Main Render Pass
├─> Clear screen
├─> Custom wgpu callbacks (Stage pane, etc.)
│   ├─> Vello vector rendering
│   └─> Waveform rendering
└─> egui UI rendering (text, widgets, overlays)
```

## Vello Integration

Vello is a GPU-accelerated 2D rendering engine that uses compute shaders for high-performance vector graphics.

### Vello Architecture

```
Document Shapes
    ↓
Convert to kurbo paths
    ↓
Build Vello Scene
    ↓
Vello Renderer (compute shaders)
    ↓
Render to GPU texture
    ↓
Composite with UI
```

### Building a Vello Scene

```rust
use vello::{Scene, SceneBuilder, kurbo::{Affine, BezPath}};
use peniko::{Color, Fill, Brush};

fn build_vello_scene(document: &Document) -> Scene {
    let mut scene = Scene::new();
    let mut builder = SceneBuilder::for_scene(&mut scene);

    for layer in &document.layers {
        if let Layer::VectorLayer { clips, visible, .. } = layer {
            if !visible {
                continue;
            }

            for clip in clips {
                for shape_instance in &clip.shapes {
                    // Get transform for this shape
                    let transform = shape_instance.compute_world_transform();
                    let affine = to_vello_affine(transform);

                    // Convert shape to kurbo path
                    let path = shape_to_kurbo_path(&shape_instance.shape);

                    // Fill
                    if let Some(fill_color) = shape_instance.shape.fill {
                        let brush = Brush::Solid(to_peniko_color(fill_color));
                        builder.fill(
                            Fill::NonZero,
                            affine,
                            &brush,
                            None,
                            &path,
                        );
                    }

                    // Stroke
                    if let Some(stroke) = &shape_instance.shape.stroke {
                        let brush = Brush::Solid(to_peniko_color(stroke.color));
                        let stroke_style = vello::kurbo::Stroke::new(stroke.width);
                        builder.stroke(
                            &stroke_style,
                            affine,
                            &brush,
                            None,
                            &path,
                        );
                    }
                }
            }
        }
    }

    scene
}
```

### Shape to Kurbo Path Conversion

```rust
use kurbo::{BezPath, PathEl, Point};

fn shape_to_kurbo_path(shape: &Shape) -> BezPath {
    let mut path = BezPath::new();

    if shape.curves.is_empty() {
        return path;
    }

    // Start at first point
    path.move_to(Point::new(
        shape.curves[0].start.x as f64,
        shape.curves[0].start.y as f64,
    ));

    // Add curves
    for curve in &shape.curves {
        match curve.curve_type {
            CurveType::Linear => {
                path.line_to(Point::new(
                    curve.end.x as f64,
                    curve.end.y as f64,
                ));
            }
            CurveType::Quadratic => {
                path.quad_to(
                    Point::new(curve.control1.x as f64, curve.control1.y as f64),
                    Point::new(curve.end.x as f64, curve.end.y as f64),
                );
            }
            CurveType::Cubic => {
                path.curve_to(
                    Point::new(curve.control1.x as f64, curve.control1.y as f64),
                    Point::new(curve.control2.x as f64, curve.control2.y as f64),
                    Point::new(curve.end.x as f64, curve.end.y as f64),
                );
            }
        }
    }

    // Close path if needed
    if shape.closed {
        path.close_path();
    }

    path
}
```

### Vello Renderer Setup

```rust
use vello::{Renderer, RendererOptions, RenderParams};
use wgpu;

pub struct VelloRenderer {
    renderer: Renderer,
    surface_format: wgpu::TextureFormat,
}

impl VelloRenderer {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let renderer = Renderer::new(
            device,
            RendererOptions {
                surface_format: Some(surface_format),
                use_cpu: false,
                antialiasing_support: vello::AaSupport::all(),
                num_init_threads: None,
            },
        ).expect("Failed to create Vello renderer");

        Self {
            renderer,
            surface_format,
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &Scene,
        texture: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let params = RenderParams {
            base_color: peniko::Color::TRANSPARENT,
            width,
            height,
            antialiasing_method: vello::AaConfig::Msaa16,
        };

        self.renderer
            .render_to_texture(device, queue, scene, texture, &params)
            .expect("Failed to render Vello scene");
    }
}
```

## Waveform Rendering

Audio waveforms are rendered on the GPU using custom WGSL shaders with mipmapping for efficient zooming.

### Waveform GPU Resources

```rust
pub struct WaveformGPU {
    // Waveform data texture (min/max per sample)
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,

    // Mipmap chain for level-of-detail
    mip_levels: Vec<wgpu::TextureView>,

    // Render pipeline
    pipeline: wgpu::RenderPipeline,

    // Uniform buffer for view parameters
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}
```

### Waveform Texture Format

Each texel stores min/max amplitude for a sample range:

```
Texture Format: Rgba16Float (4 channels, 16-bit float each)
- R channel: Left channel minimum amplitude in range [-1, 1]
- G channel: Left channel maximum amplitude in range [-1, 1]
- B channel: Right channel minimum amplitude in range [-1, 1]
- A channel: Right channel maximum amplitude in range [-1, 1]

Mip level 0: Per-sample min/max (1x)
Mip level 1: Per-4-sample min/max (1/4x)
Mip level 2: Per-16-sample min/max (1/16x)
Mip level 3: Per-64-sample min/max (1/64x)
...

Each mip level reduces by 4x, not 2x, for efficient zooming.
```

### Generating Waveform Texture

```rust
fn generate_waveform_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    audio_samples: &[f32],
) -> wgpu::Texture {
    // Calculate mip levels
    let width = audio_samples.len() as u32;
    let mip_levels = (width as f32).log2().floor() as u32 + 1;

    // Create texture
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Waveform Texture"),
        size: wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D1,
        format: wgpu::TextureFormat::Rg32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    // Upload base level (per-sample min/max)
    let mut data: Vec<f32> = Vec::with_capacity(width as usize * 2);
    for &sample in audio_samples {
        data.push(sample); // min
        data.push(sample); // max
    }

    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&data),
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * 8), // 2 floats * 4 bytes
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height: 1,
            depth_or_array_layers: 1,
        },
    );

    texture
}
```

### Mipmap Generation (Compute Shader)

```rust
// Compute shader generates mipmaps by taking min/max of 4 parent samples
// Each mip level is 4x smaller than the previous level
fn generate_mipmaps(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    base_width: u32,
    base_height: u32,
    mip_count: u32,
    base_sample_count: u32,
) -> Vec<wgpu::CommandBuffer> {
    if mip_count <= 1 {
        return Vec::new();
    }

    let mut encoder = device.create_command_encoder(&Default::default());

    let mut src_width = base_width;
    let mut src_height = base_height;
    let mut src_sample_count = base_sample_count;

    for level in 1..mip_count {
        // Dimensions halve (2x2 texels -> 1 texel)
        let dst_width = (src_width / 2).max(1);
        let dst_height = (src_height / 2).max(1);
        // But sample count reduces by 4x (4 samples -> 1)
        let dst_sample_count = (src_sample_count + 3) / 4;

        let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: level - 1,
            mip_level_count: Some(1),
            ..Default::default()
        });

        let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
            base_mip_level: level,
            mip_level_count: Some(1),
            ..Default::default()
        });

        let params = MipgenParams {
            src_width,
            dst_width,
            src_sample_count,
            _pad: 0,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &mipgen_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&dst_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Dispatch compute shader
        let total_dst_texels = dst_width * dst_height;
        let workgroup_count = (total_dst_texels + 63) / 64;

        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&mipgen_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(workgroup_count, 1, 1);
        drop(pass);

        src_width = dst_width;
        src_height = dst_height;
        src_sample_count = dst_sample_count;
    }

    vec![encoder.finish()]
}
```

## WGSL Shaders

### Waveform Render Shader

```wgsl
// waveform.wgsl

struct WaveformParams {
    view_matrix: mat4x4<f32>,      // 64 bytes
    viewport_size: vec2<f32>,      // 8 bytes
    zoom: f32,                     // 4 bytes
    _pad1: f32,                    // 4 bytes (padding)
    tint_color: vec4<f32>,         // 16 bytes (requires 16-byte alignment)
    // Total: 96 bytes
}

@group(0) @binding(0) var<uniform> params: WaveformParams;
@group(0) @binding(1) var waveform_texture: texture_1d<f32>;
@group(0) @binding(2) var waveform_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Generate fullscreen quad
    var positions = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2( 1.0,  1.0),
        vec2(-1.0, -1.0),
        vec2( 1.0,  1.0),
        vec2(-1.0,  1.0),
    );

    var output: VertexOutput;
    output.position = vec4(positions[vertex_index], 0.0, 1.0);
    output.uv = (positions[vertex_index] + 1.0) * 0.5;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample waveform texture
    let sample_pos = input.uv.x;
    let waveform = textureSample(waveform_texture, waveform_sampler, sample_pos);

    // waveform.r = min amplitude, waveform.g = max amplitude
    let min_amp = waveform.r;
    let max_amp = waveform.g;

    // Map amplitude to vertical position
    let center_y = 0.5;
    let min_y = center_y - min_amp * 0.5;
    let max_y = center_y + max_amp * 0.5;

    // Check if pixel is within waveform range
    if (input.uv.y >= min_y && input.uv.y <= max_y) {
        return params.tint_color;
    } else {
        return vec4(0.0, 0.0, 0.0, 0.0); // Transparent
    }
}
```

### Mipmap Generation Shader

```wgsl
// waveform_mipgen.wgsl

struct MipgenParams {
    src_width: u32,
    dst_width: u32,
    src_sample_count: u32,
}

@group(0) @binding(0) var src_texture: texture_2d<f32>;
@group(0) @binding(1) var dst_texture: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> params: MipgenParams;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let linear_index = global_id.x;

    // Convert linear index to 2D coordinates
    let dst_x = linear_index % params.dst_width;
    let dst_y = linear_index / params.dst_width;

    // Each dst texel corresponds to 4 src samples (not 4 src texels)
    // But 2D texture layout halves in each dimension
    let src_x = dst_x * 2u;
    let src_y = dst_y * 2u;

    // Sample 4 texels from parent level (2x2 block)
    let s00 = textureLoad(src_texture, vec2<i32>(i32(src_x), i32(src_y)), 0);
    let s10 = textureLoad(src_texture, vec2<i32>(i32(src_x + 1u), i32(src_y)), 0);
    let s01 = textureLoad(src_texture, vec2<i32>(i32(src_x), i32(src_y + 1u)), 0);
    let s11 = textureLoad(src_texture, vec2<i32>(i32(src_x + 1u), i32(src_y + 1u)), 0);

    // Compute min/max across all 4 samples for each channel
    let left_min = min(min(s00.r, s10.r), min(s01.r, s11.r));
    let left_max = max(max(s00.g, s10.g), max(s01.g, s11.g));
    let right_min = min(min(s00.b, s10.b), min(s01.b, s11.b));
    let right_max = max(max(s00.a, s10.a), max(s01.a, s11.a));

    // Write to destination mip level
    textureStore(dst_texture, vec2<i32>(i32(dst_x), i32(dst_y)),
                 vec4(left_min, left_max, right_min, right_max));
}
```

## Uniform Buffer Alignment

WGSL has strict alignment requirements. The most common issue is `vec4<f32>` requiring 16-byte alignment.

### Alignment Rules

```rust
// ❌ Bad: tint_color not aligned to 16 bytes
#[repr(C)]
struct WaveformParams {
    view_matrix: [f32; 16],   // 64 bytes (offset 0)
    viewport_size: [f32; 2],  // 8 bytes (offset 64)
    zoom: f32,                // 4 bytes (offset 72)
    tint_color: [f32; 4],     // 16 bytes (offset 76) ❌ Not 16-byte aligned!
}

// ✅ Good: explicit padding for alignment
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct WaveformParams {
    view_matrix: [f32; 16],   // 64 bytes (offset 0)
    viewport_size: [f32; 2],  // 8 bytes (offset 64)
    zoom: f32,                // 4 bytes (offset 72)
    _pad1: f32,               // 4 bytes (offset 76) - padding
    tint_color: [f32; 4],     // 16 bytes (offset 80) ✅ 16-byte aligned!
}
// Total size: 96 bytes
```

### Common Alignment Requirements

| WGSL Type | Size | Alignment |
|-----------|------|-----------|
| `f32` | 4 bytes | 4 bytes |
| `vec2<f32>` | 8 bytes | 8 bytes |
| `vec3<f32>` | 12 bytes | 16 bytes ⚠️ |
| `vec4<f32>` | 16 bytes | 16 bytes |
| `mat4x4<f32>` | 64 bytes | 16 bytes |
| Struct | Sum of members | 16 bytes (uniform buffers) |

### Debug Alignment Issues

```rust
// Use static_assertions to catch alignment bugs at compile time
use static_assertions::const_assert_eq;

const_assert_eq!(std::mem::size_of::<WaveformParams>(), 96);
const_assert_eq!(std::mem::align_of::<WaveformParams>(), 16);

// Runtime validation
fn validate_uniform_buffer<T: bytemuck::Pod>(data: &T) {
    let size = std::mem::size_of::<T>();
    let align = std::mem::align_of::<T>();

    assert!(size % 16 == 0, "Uniform buffer size must be multiple of 16");
    assert!(align >= 16, "Uniform buffer must be 16-byte aligned");
}
```

## Custom wgpu Integration

### egui-wgpu Callback Pattern

```rust
use egui_wgpu::CallbackTrait;

struct CustomRenderCallback {
    // Data needed for rendering
    scene: Scene,
    params: UniformData,
}

impl CallbackTrait for CustomRenderCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        // Update GPU resources (buffers, textures, etc.)
        // This runs before rendering

        // Get or create renderer
        let renderer: &mut MyRenderer = resources.get_or_insert_with(|| {
            MyRenderer::new(device)
        });

        // Update uniform buffer
        queue.write_buffer(&renderer.uniform_buffer, 0, bytemuck::bytes_of(&self.params));

        vec![] // Return additional command buffers if needed
    }

    fn paint<'a>(
        &'a self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'a>,
        resources: &'a egui_wgpu::CallbackResources,
    ) {
        // Actual rendering
        let renderer: &MyRenderer = resources.get().unwrap();

        render_pass.set_pipeline(&renderer.pipeline);
        render_pass.set_bind_group(0, &renderer.bind_group, &[]);
        render_pass.draw(0..6, 0..1); // Draw fullscreen quad
    }
}
```

### Registering Callback in egui

```rust
// In Stage pane render method
let callback = egui_wgpu::Callback::new_paint_callback(
    rect,
    CustomRenderCallback {
        scene: self.build_scene(document),
        params: self.compute_params(),
    },
);

ui.painter().add(callback);
```

## Performance Optimization

### Minimize GPU↔CPU Transfer

```rust
// ❌ Bad: Update uniform buffer every frame
for frame in frames {
    queue.write_buffer(&uniform_buffer, 0, &params);
    render();
}

// ✅ Good: Only update when changed
if params_changed {
    queue.write_buffer(&uniform_buffer, 0, &params);
}
render();
```

### Reuse GPU Resources

```rust
// ✅ Good: Reuse textures and buffers
struct WaveformCache {
    textures: HashMap<Uuid, wgpu::Texture>,
}

impl WaveformCache {
    fn get_or_create(&mut self, clip_id: Uuid, audio_data: &[f32]) -> &wgpu::Texture {
        self.textures.entry(clip_id).or_insert_with(|| {
            generate_waveform_texture(device, queue, audio_data)
        })
    }
}
```

### Batch Draw Calls

```rust
// ❌ Bad: One draw call per shape
for shape in shapes {
    render_pass.set_bind_group(0, &shape.bind_group, &[]);
    render_pass.draw(0..shape.vertex_count, 0..1);
}

// ✅ Good: Batch into single draw call
let batched_vertices = batch_shapes(shapes);
render_pass.set_bind_group(0, &batched_bind_group, &[]);
render_pass.draw(0..batched_vertices.len(), 0..1);
```

### Use Mipmaps for Zooming

```rust
// ✅ Good: Select appropriate mip level based on zoom
let mip_level = ((1.0 / zoom).log2().floor() as u32).min(max_mip_level);
let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
    base_mip_level: mip_level,
    mip_level_count: Some(1),
    ..Default::default()
});
```

## Debugging Rendering Issues

### Enable wgpu Validation

```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::all(),
    dx12_shader_compiler: Default::default(),
    flags: wgpu::InstanceFlags::validation(), // Enable validation
    gles_minor_version: wgpu::Gles3MinorVersion::Automatic,
});
```

### Check for Errors

```rust
// Set error handler
device.on_uncaptured_error(Box::new(|error| {
    eprintln!("wgpu error: {:?}", error);
}));
```

### Capture GPU Frame

**Linux** (RenderDoc):
```bash
renderdoccmd capture ./lightningbeam-editor
```

**macOS** (Xcode):
- Run with GPU Frame Capture enabled
- Trigger capture with Cmd+Option+G

### Common Issues

#### Black Screen
- Check that vertex shader outputs correct clip-space coordinates
- Verify texture bindings are correct
- Check that render pipeline format matches surface format

#### Validation Errors
- Check uniform buffer alignment (see [Uniform Buffer Alignment](#uniform-buffer-alignment))
- Verify texture formats match shader expectations
- Ensure bind groups match pipeline layout

#### Performance Issues
- Use GPU profiler (RenderDoc, Xcode)
- Check for redundant buffer uploads
- Profile shader performance
- Reduce draw call count via batching

## Related Documentation

- [ARCHITECTURE.md](../ARCHITECTURE.md) - Overall system architecture
- [docs/UI_SYSTEM.md](UI_SYSTEM.md) - UI and pane integration
- [CONTRIBUTING.md](../CONTRIBUTING.md) - Development workflow
