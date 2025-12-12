//! GPU-accelerated RGBA to YUV420p color space conversion
//!
//! Provides a compute shader-based converter for transforming RGBA textures
//! to YUV420p planar format using the BT.709 color matrix (HD video standard).
//! This replaces the CPU-based conversion with GPU parallel processing.

/// GPU pipeline for RGBA to YUV420p color space conversion
///
/// Converts Rgba8Unorm textures to YUV420p planar format using BT.709 colorspace.
/// The Y plane is full resolution, while U and V planes are subsampled 4:2:0.
///
/// Output texture layout:
/// - Rows 0 to height-1: Y plane (luma, full resolution)
/// - Rows height to height + height/4 - 1: U plane (chroma, half resolution)
/// - Rows height + height/4 to height + height/2 - 1: V plane (chroma, half resolution)
pub struct YuvConverter {
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
}

impl YuvConverter {
    /// Create a new RGBA to YUV420p converter
    pub fn new(device: &wgpu::Device) -> Self {
        // Create bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv_converter_bind_group_layout"),
            entries: &[
                // Input RGBA texture (binding 0)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Output YUV texture (Rgba8Unorm storage texture, binding 1)
                // Note: R8Unorm doesn't support storage binding, so we use Rgba8Unorm and write to .r channel
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        // Create pipeline layout
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("yuv_converter_pipeline_layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        // Create shader module
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("yuv_converter_shader"),
            source: wgpu::ShaderSource::Wgsl(YUV_CONVERTER_SHADER.into()),
        });

        // Create compute pipeline
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("yuv_converter_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Self {
            pipeline,
            bind_group_layout,
        }
    }

    /// Convert RGBA texture to YUV420p planar format
    ///
    /// Reads from `rgba_view` and writes Y, U, V planes to `yuv_output_view`.
    /// The output texture must be R8Unorm format with height = input_height * 1.5
    /// to accommodate the packed YUV planes.
    ///
    /// # Arguments
    /// * `device` - GPU device
    /// * `encoder` - Command encoder to record GPU commands
    /// * `rgba_view` - Source RGBA texture view
    /// * `yuv_output_view` - Destination YUV planar texture view (R8Unorm, height*1.5)
    /// * `width` - Width of the source RGBA texture
    /// * `height` - Height of the source RGBA texture
    pub fn convert(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        rgba_view: &wgpu::TextureView,
        yuv_output_view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        // Create bind group for this conversion
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv_converter_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(rgba_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(yuv_output_view),
                },
            ],
        });

        // Compute pass
        let mut compute_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("yuv_conversion_pass"),
            timestamp_writes: None,
        });

        compute_pass.set_pipeline(&self.pipeline);
        compute_pass.set_bind_group(0, &bind_group, &[]);

        // Dispatch workgroups: 8x8 threads per workgroup
        // Each thread processes one pixel for the Y plane
        // Chroma planes are processed by threads at even coordinates
        let workgroup_size = 8;
        let workgroups_x = (width + workgroup_size - 1) / workgroup_size;
        let workgroups_y = (height + workgroup_size - 1) / workgroup_size;
        compute_pass.dispatch_workgroups(workgroups_x, workgroups_y, 1);
    }
}

/// WGSL compute shader for RGBA to YUV420p conversion
const YUV_CONVERTER_SHADER: &str = r#"
// RGBA to YUV420p Compute Shader
// BT.709 color space for HD video (ITU-R BT.709-6 standard)
//
// Color matrix:
// Y  =  0.2126*R + 0.7152*G + 0.0722*B
// U  = -0.1146*R - 0.3854*G + 0.5000*B + 0.5
// V  =  0.5000*R - 0.4542*G - 0.0458*B + 0.5
//
// Output texture layout (packed planar, side-by-side U/V):
// - Rows [0, height): Y plane (full resolution, full width)
// - Rows [height, height + height/2): U plane (left half, columns 0 to width/2-1)
//                                      V plane (right half, columns width/2 to width-1)

@group(0) @binding(0) var input_rgba: texture_2d<f32>;
@group(0) @binding(1) var output_yuv: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let dims = textureDimensions(input_rgba);
    let pos = global_id.xy;

    // Bounds check
    if (pos.x >= dims.x || pos.y >= dims.y) {
        return;
    }

    // Load RGBA pixel
    let rgba = textureLoad(input_rgba, pos, 0);
    let r = rgba.r;
    let g = rgba.g;
    let b = rgba.b;

    // Compute Y (luma) - full resolution, BT.709
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;

    // Write Y value to Y plane (rows 0 to height-1)
    textureStore(output_yuv, pos, vec4<f32>(y, 0.0, 0.0, 0.0));

    // Compute U and V (chroma) - subsampled 4:2:0
    // Only process even coordinates (top-left of 2x2 blocks)
    if (pos.x % 2u == 0u && pos.y % 2u == 0u) {
        // Sample 2x2 block for chroma subsampling
        var r_sum = r;
        var g_sum = g;
        var b_sum = b;
        var count = 1.0;

        // Sample right neighbor (x+1, y)
        if (pos.x + 1u < dims.x) {
            let rgba_r = textureLoad(input_rgba, pos + vec2<u32>(1u, 0u), 0);
            r_sum += rgba_r.r;
            g_sum += rgba_r.g;
            b_sum += rgba_r.b;
            count += 1.0;
        }

        // Sample bottom neighbor (x, y+1)
        if (pos.y + 1u < dims.y) {
            let rgba_b = textureLoad(input_rgba, pos + vec2<u32>(0u, 1u), 0);
            r_sum += rgba_b.r;
            g_sum += rgba_b.g;
            b_sum += rgba_b.b;
            count += 1.0;
        }

        // Sample bottom-right neighbor (x+1, y+1)
        if (pos.x + 1u < dims.x && pos.y + 1u < dims.y) {
            let rgba_br = textureLoad(input_rgba, pos + vec2<u32>(1u, 1u), 0);
            r_sum += rgba_br.r;
            g_sum += rgba_br.g;
            b_sum += rgba_br.b;
            count += 1.0;
        }

        // Average the 2x2 block
        let r_avg = r_sum / count;
        let g_avg = g_sum / count;
        let b_avg = b_sum / count;

        // Compute chroma components (BT.709, centered at 0.5 for unsigned 8-bit)
        let u = -0.1146 * r_avg - 0.3854 * g_avg + 0.5000 * b_avg + 0.5;
        let v =  0.5000 * r_avg - 0.4542 * g_avg - 0.0458 * b_avg + 0.5;

        // Compute chroma plane positions (half resolution)
        // Pack U and V side-by-side: U on left half, V on right half
        let chroma_x = pos.x / 2u;
        let chroma_y = pos.y / 2u;

        // U plane: left half (columns 0 to width/2-1), rows height to height+height/2-1
        let u_pos = vec2<u32>(chroma_x, dims.y + chroma_y);

        // V plane: right half (columns width/2 to width-1), rows height to height+height/2-1
        let v_pos = vec2<u32>(dims.x / 2u + chroma_x, dims.y + chroma_y);

        // Write U and V values to their respective planes
        textureStore(output_yuv, u_pos, vec4<f32>(u, 0.0, 0.0, 0.0));
        textureStore(output_yuv, v_pos, vec4<f32>(v, 0.0, 0.0, 0.0));
    }
}
"#;
