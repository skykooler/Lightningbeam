#![allow(dead_code)]
//! Video export functionality
//!
//! Exports video from the timeline using FFmpeg encoding:
//! - H.264/H.265: MP4 container (most compatible)
//! - VP9: WebM container (web-friendly)
//! - ProRes422: MOV container (professional editing)

use ffmpeg_next as ffmpeg;
use std::sync::Arc;
use lightningbeam_core::document::Document;
use lightningbeam_core::renderer::{ImageCache, render_document_for_compositing, RenderedLayerType};
use lightningbeam_core::video::VideoManager;
use lightningbeam_core::gpu::{
    BufferPool, BufferSpec, BufferFormat, Compositor, CompositorLayer,
    SrgbToLinearConverter, EffectProcessor, YuvConverter, HDR_FORMAT,
};

/// Reusable frame buffers to avoid allocations
struct FrameBuffers {
    /// RGBA buffer from GPU readback (width * height * 4 bytes)
    rgba_buffer: Vec<u8>,
    /// Y plane for YUV420p (full resolution)
    y_plane: Vec<u8>,
    /// U plane for YUV420p (quarter resolution - 2×2 subsampling)
    u_plane: Vec<u8>,
    /// V plane for YUV420p (quarter resolution - 2×2 subsampling)
    v_plane: Vec<u8>,
}

impl FrameBuffers {
    /// Create new frame buffers for the given resolution
    fn new(width: u32, height: u32) -> Self {
        let rgba_size = (width * height * 4) as usize;
        let y_size = (width * height) as usize;
        let uv_size = ((width / 2) * (height / 2)) as usize;

        Self {
            rgba_buffer: vec![0u8; rgba_size],
            y_plane: vec![0u8; y_size],
            u_plane: vec![0u8; uv_size],
            v_plane: vec![0u8; uv_size],
        }
    }
}

/// GPU resources for HDR export pipeline
///
/// This mirrors the resources in stage.rs SharedVelloResources but is owned
/// by the export system to avoid lifetime/locking issues during export.
pub struct ExportGpuResources {
    /// Buffer pool for intermediate render targets
    pub buffer_pool: BufferPool,
    /// HDR compositor for layer blending
    pub compositor: Compositor,
    /// sRGB to linear color converter
    pub srgb_to_linear: SrgbToLinearConverter,
    /// Effect processor for shader effects
    pub effect_processor: EffectProcessor,
    /// GPU-accelerated RGBA to YUV420p converter
    pub yuv_converter: YuvConverter,
    /// HDR accumulator texture for compositing
    pub hdr_texture: wgpu::Texture,
    /// View for HDR texture
    pub hdr_texture_view: wgpu::TextureView,
    /// Persistent RGBA output texture (sRGB, reused for all frames)
    pub output_texture: wgpu::Texture,
    /// View for persistent output texture
    pub output_texture_view: wgpu::TextureView,
    /// Persistent YUV texture for GPU conversion (R8Unorm, height*1.5, reused for all frames)
    pub yuv_texture: wgpu::Texture,
    /// View for persistent YUV texture
    pub yuv_texture_view: wgpu::TextureView,
    /// Persistent staging buffer for GPU→CPU readback (reused for all frames)
    pub staging_buffer: wgpu::Buffer,
    /// Linear to sRGB blit pipeline for final output
    pub linear_to_srgb_pipeline: wgpu::RenderPipeline,
    /// Bind group layout for linear to sRGB blit
    pub linear_to_srgb_bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for linear to sRGB conversion
    pub linear_to_srgb_sampler: wgpu::Sampler,
    /// Canvas blit pipeline for raster/video/float layers (bypasses Vello).
    pub canvas_blit: crate::gpu_brush::CanvasBlitPipeline,
    /// Per-keyframe GPU texture cache for raster layers during export.
    pub raster_cache: std::collections::HashMap<uuid::Uuid, crate::gpu_brush::CanvasPair>,
}

impl ExportGpuResources {
    /// Create new export GPU resources for the given dimensions
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let buffer_pool = BufferPool::new();
        let compositor = Compositor::new(device, HDR_FORMAT);
        let srgb_to_linear = SrgbToLinearConverter::new(device);
        let effect_processor = EffectProcessor::new(device, HDR_FORMAT);
        let yuv_converter = YuvConverter::new(device);

        // Create HDR accumulator texture
        let hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_hdr_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let hdr_texture_view = hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create persistent RGBA output texture (sRGB, reused for all frames)
        let output_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_output_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_texture_view = output_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create persistent YUV texture (Rgba8Unorm, height*1.5 for packed Y+U+V planes)
        // Note: Using Rgba8Unorm instead of R8Unorm because R8Unorm doesn't support STORAGE_BINDING
        let yuv_height = height + height / 2; // Y plane + U plane + V plane
        let yuv_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_yuv_texture"),
            size: wgpu::Extent3d {
                width,
                height: yuv_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let yuv_texture_view = yuv_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create persistent staging buffer for GPU→CPU readback
        let yuv_buffer_size = (width * yuv_height * 4) as u64; // Rgba8Unorm = 4 bytes per pixel
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("export_staging_buffer"),
            size: yuv_buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        // Create linear to sRGB blit pipeline
        let linear_to_srgb_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("linear_to_srgb_bind_group_layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("linear_to_srgb_pipeline_layout"),
            bind_group_layouts: &[&linear_to_srgb_bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("linear_to_srgb_shader"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{}\n{}", lightningbeam_core::gpu::COLOR_WGSL, LINEAR_TO_SRGB_SHADER).into(),
            ),
        });

        let linear_to_srgb_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("linear_to_srgb_pipeline"),
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
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

        let linear_to_srgb_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("linear_to_srgb_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let canvas_blit = crate::gpu_brush::CanvasBlitPipeline::new(device);

        Self {
            buffer_pool,
            compositor,
            srgb_to_linear,
            effect_processor,
            yuv_converter,
            hdr_texture,
            hdr_texture_view,
            output_texture,
            output_texture_view,
            yuv_texture,
            yuv_texture_view,
            staging_buffer,
            linear_to_srgb_pipeline,
            linear_to_srgb_bind_group_layout,
            linear_to_srgb_sampler,
            canvas_blit,
            raster_cache: std::collections::HashMap::new(),
        }
    }

    /// Resize the HDR texture if dimensions changed
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        self.hdr_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_hdr_texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.hdr_texture_view = self.hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
    }
}

/// WGSL shader for linear to sRGB conversion (for final export output)
const LINEAR_TO_SRGB_SHADER: &str = r#"
// Linear to sRGB color space conversion shader

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// Fullscreen triangle strip
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    let x = f32((vertex_index & 1u) << 1u);
    let y = f32(vertex_index & 2u);

    out.position = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);

    return out;
}

// linear_to_srgb / linear_to_srgb_channel are provided by the prepended
// COLOR_WGSL prelude (see the create_shader_module call site).

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_tex, source_sampler, in.uv);

    // The compositor accumulates PREMULTIPLIED linear color. Unpremultiply
    // before the sRGB OETF (srgb(rgb*a) != srgb(rgb)*a) and emit STRAIGHT
    // alpha, which is what PNG export / the readback path expect. For opaque
    // pixels (a == 1, the normal video case) this is an exact identity.
    let a = src.a;
    let straight = select(src.rgb / a, vec3<f32>(0.0), a <= 0.0);

    // Convert linear HDR to sRGB
    let srgb = linear_to_srgb(straight);

    return vec4<f32>(srgb, a);
}
"#;

/// Convert RGBA8 pixels to YUV420p format using BT.709 color space
///
/// # Arguments
/// * `rgba` - Interleaved RGBA8 pixels (4 bytes per pixel)
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
///
/// # Returns
/// Tuple of (Y plane, U plane, V plane) as separate byte vectors
///
/// # Color Space
/// Uses BT.709 (HDTV) color space conversion:
/// - Y  = 0.2126*R + 0.7152*G + 0.0722*B
/// - U  = -0.1146*R - 0.3854*G + 0.5000*B + 128
/// - V  = 0.5000*R - 0.4542*G - 0.0458*B + 128
///
/// # Format
/// YUV420p is a planar format with 2×2 chroma subsampling:
/// - Y plane: full resolution (width × height)
/// - U plane: quarter resolution (width/2 × height/2)
/// - V plane: quarter resolution (width/2 × height/2)
pub fn rgba_to_yuv420p(rgba: &[u8], width: u32, height: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let w = width as usize;
    let h = height as usize;

    // Round to multiples of 16 for H.264 macroblock alignment
    let aligned_w = (((width + 15) / 16) * 16) as usize;
    let aligned_h = (((height + 15) / 16) * 16) as usize;

    // Allocate Y plane (full aligned resolution, padded with black)
    let mut y_plane = Vec::with_capacity(aligned_w * aligned_h);

    // Convert each pixel to Y (luma), with padding
    for y in 0..aligned_h {
        for x in 0..aligned_w {
            let y_val = if y < h && x < w {
                let idx = (y * w + x) * 4;
                let r = rgba[idx] as f32;
                let g = rgba[idx + 1] as f32;
                let b = rgba[idx + 2] as f32;
                // BT.709 luma conversion
                (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 255.0) as u8
            } else {
                16 // Black in YUV (Y=16 is video black)
            };
            y_plane.push(y_val);
        }
    }

    // Allocate U and V planes (quarter resolution due to 2×2 subsampling)
    let mut u_plane = Vec::with_capacity((aligned_w * aligned_h) / 4);
    let mut v_plane = Vec::with_capacity((aligned_w * aligned_h) / 4);

    // Process 2×2 blocks for chroma subsampling (with padding for aligned dimensions)
    for y in (0..aligned_h).step_by(2) {
        for x in (0..aligned_w).step_by(2) {
            // Check if this block is in the padding region
            let in_padding = y >= h || x >= w;

            let (u_val, v_val) = if in_padding {
                // Padding region: use neutral chroma for black (U=128, V=128)
                (128, 128)
            } else {
                // Average RGB values from 2×2 block
                let mut r_sum = 0.0;
                let mut g_sum = 0.0;
                let mut b_sum = 0.0;

                for dy in 0..2 {
                    for dx in 0..2 {
                        if y + dy < h && x + dx < w {
                            let idx = ((y + dy) * w + (x + dx)) * 4;
                            r_sum += rgba[idx] as f32;
                            g_sum += rgba[idx + 1] as f32;
                            b_sum += rgba[idx + 2] as f32;
                        }
                    }
                }

                let r = r_sum / 4.0;
                let g = g_sum / 4.0;
                let b = b_sum / 4.0;

                // BT.709 chroma conversion (centered at 128)
                let u = (-0.1146 * r - 0.3854 * g + 0.5000 * b + 128.0).clamp(0.0, 255.0) as u8;
                let v = (0.5000 * r - 0.4542 * g - 0.0458 * b + 128.0).clamp(0.0, 255.0) as u8;
                (u, v)
            };

            u_plane.push(u_val);
            v_plane.push(v_val);
        }
    }

    (y_plane, u_plane, v_plane)
}

/// Setup FFmpeg video encoder for the specified codec
///
/// # Arguments
/// * `codec_id` - FFmpeg codec ID (H264, HEVC, VP9, PRORES, etc.)
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `framerate` - Frames per second
/// * `bitrate_kbps` - Target bitrate in kilobits per second
///
/// # Returns
/// Tuple of (opened encoder, codec) for stream setup
///
/// # Note
/// This function follows the same pattern as the working MP3 export:
/// 1. Find codec
/// 2. Create encoder context with codec
/// 3. Set ALL parameters (width, height, format, timebase, framerate, bitrate, GOP)
/// 4. Open encoder with open_as(codec)
/// 5. Caller should add stream AFTER opening and set parameters from opened encoder
pub fn setup_video_encoder(
    codec_id: ffmpeg::codec::Id,
    width: u32,
    height: u32,
    framerate: f64,
    bitrate_kbps: u32,
) -> Result<(ffmpeg::encoder::Video, ffmpeg::Codec), String> {
    // Try to find codec by ID first
    println!("🔍 Looking for codec: {:?}", codec_id);
    let codec = ffmpeg::encoder::find(codec_id);

    let codec = if codec.is_some() {
        println!("✅ Found codec by ID");
        codec
    } else {
        println!("⚠️  Codec {:?} not found by ID", codec_id);

        // If not found by ID, try by name (e.g., "libx264" for H264)
        let encoder_name = match codec_id {
            ffmpeg::codec::Id::H264 => "libx264",
            ffmpeg::codec::Id::HEVC => "libx265",
            ffmpeg::codec::Id::VP8 => "libvpx",
            ffmpeg::codec::Id::VP9 => "libvpx-vp9",
            ffmpeg::codec::Id::PRORES => "prores_ks",
            _ => {
                println!("❌ No fallback encoder name for {:?}", codec_id);
                return Err(format!("Unsupported codec: {:?}", codec_id));
            }
        };

        println!("🔍 Trying encoder by name: {}", encoder_name);
        let by_name = ffmpeg::encoder::find_by_name(encoder_name);

        if by_name.is_some() {
            println!("✅ Found encoder by name: {}", encoder_name);
        } else {
            println!("❌ Encoder {} not found", encoder_name);
        }

        by_name
    };

    let codec = codec.ok_or_else(|| {
        println!("❌ Failed to find codec: {:?}", codec_id);
        println!("💡 The static FFmpeg build is missing this encoder.");
        format!("Video encoder not found for codec: {:?}. Static build may be missing encoder libraries.", codec_id)
    })?;

    // Create encoder context with codec
    let mut encoder = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .video()
        .map_err(|e| format!("Failed to create video encoder: {}", e))?;

    // Round dimensions to multiples of 16 for H.264 macroblock alignment
    let aligned_width = ((width + 15) / 16) * 16;
    let aligned_height = ((height + 15) / 16) * 16;

    // Configure encoder parameters BEFORE opening (critical!)
    encoder.set_width(aligned_width);
    encoder.set_height(aligned_height);
    encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    encoder.set_time_base(ffmpeg::Rational(1, (framerate * 1000.0) as i32));
    encoder.set_frame_rate(Some(ffmpeg::Rational(framerate as i32, 1)));
    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_gop(framerate as u32); // 1 second GOP

    // Tag the color metadata so players interpret the YUV correctly. Our
    // RGB→YUV conversion uses the BT.709 matrix with FULL-range (0–255) luma
    // and no transfer applied to the already-sRGB-encoded RGB. Tagging this
    // as full-range BT.709 (matrix/primaries/transfer) prevents the level/
    // hue shift that occurs when a player assumes limited-range or BT.601.
    // colorspace (matrix) and range have safe setters; primaries and trc are
    // generic AVCodecContext options set via the open dictionary below.
    encoder.set_colorspace(ffmpeg::color::Space::BT709);
    encoder.set_color_range(ffmpeg::color::Range::JPEG); // full range

    println!("📐 Video dimensions: {}×{} (aligned to {}×{} for H.264)",
             width, height, aligned_width, aligned_height);

    // Open encoder with codec (like working MP3 export). color_primaries and
    // color_trc have no typed setter on the encoder, so pass them as generic
    // AVCodecContext options (BT.709) through the open dictionary.
    let mut color_opts = ffmpeg::Dictionary::new();
    color_opts.set("color_primaries", "bt709");
    color_opts.set("color_trc", "bt709");
    let encoder = encoder
        .open_as_with(codec, color_opts)
        .map_err(|e| format!("Failed to open video encoder: {}", e))?;

    Ok((encoder, codec))
}

/// Receive encoded packets from encoder and write to output file
///
/// # Arguments
/// * `encoder` - FFmpeg video encoder
/// * `output` - FFmpeg output format context
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn receive_and_write_packets(
    encoder: &mut ffmpeg::encoder::Video,
    output: &mut ffmpeg::format::context::Output,
) -> Result<(), String> {
    let mut encoded = ffmpeg::Packet::empty();

    // Get time bases for rescaling
    let encoder_tb = encoder.time_base();
    let stream_tb = output.stream(0).ok_or("No output stream found")?.time_base();

    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(0);
        // Rescale timestamps from encoder time base to stream time base
        encoded.rescale_ts(encoder_tb, stream_tb);

        encoded
            .write_interleaved(output)
            .map_err(|e| format!("Failed to write packet: {}", e))?;
    }

    Ok(())
}

/// Render a document frame at a specific time and read back RGBA pixels from GPU
///
/// # Arguments
/// * `document` - Document to render (current_time will be modified)
/// * `timestamp` - Time in seconds to render at
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `device` - wgpu device
/// * `queue` - wgpu queue
/// * `renderer` - Vello renderer
/// * `image_cache` - Image cache for rendering
/// * `video_manager` - Video manager for video clips
/// * `rgba_buffer` - Output buffer for RGBA pixels (must be width * height * 4 bytes)
///
/// # Returns
/// Ok(()) on success, Err with message on failure
pub fn render_frame_to_rgba(
    document: &mut Document,
    timestamp: f64,
    width: u32,
    height: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    image_cache: &mut ImageCache,
    video_manager: &Arc<std::sync::Mutex<VideoManager>>,
    rgba_buffer: &mut [u8],
) -> Result<(), String> {
    // Set document time to the frame timestamp
    document.current_time = timestamp;

    // Create offscreen texture for rendering
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("video_export_texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
             | wgpu::TextureUsages::COPY_SRC
             | wgpu::TextureUsages::STORAGE_BINDING, // Required by Vello for compute shaders
        view_formats: &[],
    });

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Render document to Vello scene
    let mut scene = vello::Scene::new();
    lightningbeam_core::renderer::render_document(
        document,
        &mut scene,
        image_cache,
        video_manager,
    );

    // Render scene to texture
    let render_params = vello::RenderParams {
        base_color: vello::peniko::Color::BLACK,
        width,
        height,
        antialiasing_method: vello::AaConfig::Area,
    };

    renderer
        .render_to_texture(device, queue, &scene, &texture_view, &render_params)
        .map_err(|e| format!("Failed to render to texture: {}", e))?;

    // GPU readback: Create staging buffer with proper alignment
    let bytes_per_pixel = 4u32; // RGBA8
    let bytes_per_row_alignment = 256u32; // wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
    let unpadded_bytes_per_row = width * bytes_per_pixel;
    let bytes_per_row = ((unpadded_bytes_per_row + bytes_per_row_alignment - 1)
        / bytes_per_row_alignment) * bytes_per_row_alignment;
    let buffer_size = (bytes_per_row * height) as u64;

    let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("video_export_staging_buffer"),
        size: buffer_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Copy texture to staging buffer
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("video_export_copy_encoder"),
    });

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    // Map buffer and read pixels (synchronous)
    let buffer_slice = staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).ok();
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    receiver
        .recv()
        .map_err(|_| "Failed to receive buffer mapping result")?
        .map_err(|e| format!("Failed to map buffer: {:?}", e))?;

    // Copy data from mapped buffer to output, removing padding
    let data = buffer_slice.get_mapped_range();
    for y in 0..height as usize {
        let src_offset = y * bytes_per_row as usize;
        let dst_offset = y * unpadded_bytes_per_row as usize;
        let row_bytes = unpadded_bytes_per_row as usize;
        rgba_buffer[dst_offset..dst_offset + row_bytes]
            .copy_from_slice(&data[src_offset..src_offset + row_bytes]);
    }

    drop(data);
    staging_buffer.unmap();

    Ok(())
}

/// Composite all layers from `composite_result` into `gpu_resources.hdr_texture_view`.
///
/// Shared by both export functions. Handles every layer type:
/// - Vector/Group: Vello scene → sRGB → linear → composite
/// - Raster: upload pixels to `raster_cache` (if needed) → GPU blit → composite
/// - Video: sRGB straight-alpha → linear premultiplied → transient GPU texture → blit → composite
/// - Float: sRGB-premultiplied → linear → transient GPU texture → blit → composite
/// - Effect: apply post-process on the HDR accumulator
fn composite_document_to_hdr(
    composite_result: &lightningbeam_core::renderer::CompositeRenderResult,
    document: &Document,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    gpu_resources: &mut ExportGpuResources,
    width: u32,
    height: u32,
    allow_transparency: bool,
) -> Result<(), String> {
    use vello::kurbo::Affine;

    let layer_spec = BufferSpec::new(width, height, BufferFormat::Rgba8Srgb);
    let hdr_spec = BufferSpec::new(width, height, BufferFormat::Rgba16Float);
    let layer_render_params = vello::RenderParams {
        base_color: vello::peniko::Color::TRANSPARENT,
        width, height,
        antialiasing_method: vello::AaConfig::Area,
    };

    // --- Background ---
    let bg_srgb = gpu_resources.buffer_pool.acquire(device, layer_spec);
    let bg_hdr  = gpu_resources.buffer_pool.acquire(device, hdr_spec);
    if let (Some(bg_srgb_view), Some(bg_hdr_view)) = (
        gpu_resources.buffer_pool.get_view(bg_srgb),
        gpu_resources.buffer_pool.get_view(bg_hdr),
    ) {
        renderer.render_to_texture(device, queue, &composite_result.background, bg_srgb_view, &layer_render_params)
            .map_err(|e| format!("Failed to render background: {e}"))?;
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_bg_srgb_to_linear") });
        gpu_resources.srgb_to_linear.convert(device, &mut enc, bg_srgb_view, bg_hdr_view);
        queue.submit(Some(enc.finish()));
        let bg_layer = CompositorLayer::normal(bg_hdr, 1.0);
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_bg_composite") });
        // When transparency is allowed, start from transparent black so the background's
        // native alpha is preserved. Otherwise force an opaque black underlay.
        let clear = if allow_transparency { [0.0, 0.0, 0.0, 0.0] } else { [0.0, 0.0, 0.0, 1.0] };
        gpu_resources.compositor.composite(device, queue, &mut enc, &[bg_layer],
            &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, Some(clear));
        queue.submit(Some(enc.finish()));
    }
    gpu_resources.buffer_pool.release(bg_srgb);
    gpu_resources.buffer_pool.release(bg_hdr);

    // --- Layers ---
    for rendered_layer in &composite_result.layers {
        if !rendered_layer.has_content { continue; }

        match &rendered_layer.layer_type {
            RenderedLayerType::Vector => {
                let srgb_handle = gpu_resources.buffer_pool.acquire(device, layer_spec);
                let hdr_layer_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                if let (Some(srgb_view), Some(hdr_layer_view)) = (
                    gpu_resources.buffer_pool.get_view(srgb_handle),
                    gpu_resources.buffer_pool.get_view(hdr_layer_handle),
                ) {
                    renderer.render_to_texture(device, queue, &rendered_layer.scene, srgb_view, &layer_render_params)
                        .map_err(|e| format!("Failed to render layer: {e}"))?;
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_layer_srgb_to_linear") });
                    gpu_resources.srgb_to_linear.convert(device, &mut enc, srgb_view, hdr_layer_view);
                    queue.submit(Some(enc.finish()));
                    let compositor_layer = CompositorLayer::new(hdr_layer_handle, rendered_layer.opacity, rendered_layer.blend_mode);
                    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_layer_composite") });
                    gpu_resources.compositor.composite(device, queue, &mut enc, &[compositor_layer], &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, None);
                    queue.submit(Some(enc.finish()));
                }
                gpu_resources.buffer_pool.release(srgb_handle);
                gpu_resources.buffer_pool.release(hdr_layer_handle);
            }
            RenderedLayerType::Raster { kf_id, width: cw, height: ch, transform: layer_transform, dirty: _ } => {
                let raw_pixels = document.get_layer(&rendered_layer.layer_id)
                    .and_then(|l| match l {
                        lightningbeam_core::layer::AnyLayer::Raster(rl) => rl.keyframe_at(document.current_time),
                        _ => None,
                    })
                    .filter(|kf| !kf.raw_pixels.is_empty())
                    .map(|kf| kf.raw_pixels.clone());
                if let Some(pixels) = raw_pixels {
                    if !gpu_resources.raster_cache.contains_key(kf_id) {
                        let canvas = crate::gpu_brush::CanvasPair::new(device, *cw, *ch);
                        canvas.upload(queue, &pixels);
                        gpu_resources.raster_cache.insert(*kf_id, canvas);
                    }
                    if let Some(canvas) = gpu_resources.raster_cache.get(kf_id) {
                        let hdr_layer_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                        if let Some(hdr_layer_view) = gpu_resources.buffer_pool.get_view(hdr_layer_handle) {
                            let bt = crate::gpu_brush::BlitTransform::new(*layer_transform, *cw, *ch, width, height);
                            gpu_resources.canvas_blit.blit(device, queue, canvas.src_view(), hdr_layer_view, &bt, None);
                            let compositor_layer = CompositorLayer::new(hdr_layer_handle, rendered_layer.opacity, rendered_layer.blend_mode);
                            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_raster_composite") });
                            gpu_resources.compositor.composite(device, queue, &mut enc, &[compositor_layer], &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, None);
                            queue.submit(Some(enc.finish()));
                        }
                        gpu_resources.buffer_pool.release(hdr_layer_handle);
                    }
                }
            }
            RenderedLayerType::Video { instances } => {
                for inst in instances {
                    if inst.rgba_data.is_empty() { continue; }
                    let hdr_layer_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                    if let Some(hdr_layer_view) = gpu_resources.buffer_pool.get_view(hdr_layer_handle) {
                        // sRGB straight-alpha → linear premultiplied
                        let linear: Vec<u8> = inst.rgba_data.chunks_exact(4).flat_map(|p| {
                            let a = p[3] as f32 / 255.0;
                            let lin = |c: u8| -> f32 {
                                let f = c as f32 / 255.0;
                                if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) }
                            };
                            let r = (lin(p[0]) * a * 255.0 + 0.5) as u8;
                            let g = (lin(p[1]) * a * 255.0 + 0.5) as u8;
                            let b = (lin(p[2]) * a * 255.0 + 0.5) as u8;
                            [r, g, b, p[3]]
                        }).collect();
                        let tex = upload_transient_texture(device, queue, &linear, inst.width, inst.height, Some("export_video_frame_tex"));
                        let tex_view = tex.create_view(&Default::default());
                        let bt = crate::gpu_brush::BlitTransform::new(inst.transform, inst.width, inst.height, width, height);
                        gpu_resources.canvas_blit.blit(device, queue, &tex_view, hdr_layer_view, &bt, None);
                        let compositor_layer = CompositorLayer::new(hdr_layer_handle, inst.opacity, lightningbeam_core::gpu::BlendMode::Normal);
                        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_video_composite") });
                        gpu_resources.compositor.composite(device, queue, &mut enc, &[compositor_layer], &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, None);
                        queue.submit(Some(enc.finish()));
                    }
                    gpu_resources.buffer_pool.release(hdr_layer_handle);
                }
            }
            RenderedLayerType::Float { x: float_x, y: float_y, width: fw, height: fh, transform: layer_transform, pixels, .. } => {
                if !pixels.is_empty() {
                    // sRGB-premultiplied → linear-premultiplied
                    let linear: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| {
                        let lin = |c: u8| -> u8 {
                            let f = c as f32 / 255.0;
                            let l = if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) };
                            (l * 255.0 + 0.5) as u8
                        };
                        [lin(p[0]), lin(p[1]), lin(p[2]), p[3]]
                    }).collect();
                    let tex = upload_transient_texture(device, queue, &linear, *fw, *fh, Some("export_float_tex"));
                    let tex_view = tex.create_view(&Default::default());
                    let hdr_layer_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                    if let Some(hdr_layer_view) = gpu_resources.buffer_pool.get_view(hdr_layer_handle) {
                        let float_to_vp = *layer_transform * Affine::translate((*float_x as f64, *float_y as f64));
                        let bt = crate::gpu_brush::BlitTransform::new(float_to_vp, *fw, *fh, width, height);
                        gpu_resources.canvas_blit.blit(device, queue, &tex_view, hdr_layer_view, &bt, None);
                        let compositor_layer = CompositorLayer::normal(hdr_layer_handle, 1.0);
                        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_float_composite") });
                        gpu_resources.compositor.composite(device, queue, &mut enc, &[compositor_layer], &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, None);
                        queue.submit(Some(enc.finish()));
                    }
                    gpu_resources.buffer_pool.release(hdr_layer_handle);
                }
            }
            RenderedLayerType::Effect { effect_instances } => {
                let current_time = document.current_time;
                for effect_instance in effect_instances {
                    let Some(effect_def) = document.get_effect_definition(&effect_instance.clip_id) else { continue; };
                    if !gpu_resources.effect_processor.is_compiled(&effect_def.id) {
                        let success = gpu_resources.effect_processor.compile_effect(device, effect_def);
                        if !success { eprintln!("Failed to compile effect: {}", effect_def.name); continue; }
                    }
                    let effect_inst = lightningbeam_core::effect::EffectInstance::new(
                        effect_def,
                        effect_instance.timeline_start,
                        effect_instance.timeline_start + effect_instance.effective_duration(lightningbeam_core::effect::EFFECT_DURATION, document.tempo_map()),
                    );
                    let effect_output_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                    if let Some(effect_output_view) = gpu_resources.buffer_pool.get_view(effect_output_handle) {
                        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_effect") });
                        let applied = gpu_resources.effect_processor.apply_effect(
                            device, queue, &mut enc, effect_def, &effect_inst,
                            &gpu_resources.hdr_texture_view, effect_output_view, width, height, current_time,
                        );
                        if applied {
                            queue.submit(Some(enc.finish()));
                            let effect_layer = CompositorLayer::normal(effect_output_handle, rendered_layer.opacity);
                            let mut copy_enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_effect_copy") });
                            // Replace the accumulator with the processed result.
                            gpu_resources.compositor.composite(device, queue, &mut copy_enc, &[effect_layer], &gpu_resources.buffer_pool, &gpu_resources.hdr_texture_view, Some([0.0, 0.0, 0.0, 0.0]));
                            queue.submit(Some(copy_enc.finish()));
                        }
                    }
                    gpu_resources.buffer_pool.release(effect_output_handle);
                }
            }
        }
    }

    gpu_resources.buffer_pool.next_frame();
    Ok(())
}

/// Upload `pixels` to a transient `Rgba8Unorm` GPU texture (TEXTURE_BINDING | COPY_DST).
fn upload_transient_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &[u8],
    width: u32,
    height: u32,
    label: Option<&'static str>,
) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        pixels,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    tex
}

/// Render a document frame using the HDR compositing pipeline with effects
///
/// This function uses the same rendering pipeline as the stage preview,
/// ensuring effects are applied correctly during export.
///
/// # Arguments
/// * `document` - Document to render (current_time will be modified)
/// * `timestamp` - Time in seconds to render at
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `device` - wgpu device
/// * `queue` - wgpu queue
/// * `renderer` - Vello renderer
/// * `image_cache` - Image cache for rendering
/// * `video_manager` - Video manager for video clips
/// * `gpu_resources` - HDR GPU resources for compositing
///
/// # Returns
/// Ok((y_plane, u_plane, v_plane)) with YUV420p planes on success, Err with message on failure
pub fn render_frame_to_rgba_hdr(
    document: &mut Document,
    timestamp: f64,
    width: u32,
    height: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    image_cache: &mut ImageCache,
    video_manager: &Arc<std::sync::Mutex<VideoManager>>,
    gpu_resources: &mut ExportGpuResources,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    use vello::kurbo::Affine;

    // Set document time to the frame timestamp
    document.current_time = timestamp;

    // Scale the document to the export resolution. The core renderer bakes this
    // base transform into every layer (vector scenes, raster and video layer
    // transforms), so the whole stage scales up/down to fill the output. When the
    // export size matches the document this is the identity.
    let base_transform = if document.width > 0.0 && document.height > 0.0 {
        Affine::scale_non_uniform(
            width as f64 / document.width,
            height as f64 / document.height,
        )
    } else {
        Affine::IDENTITY
    };

    // Render document for compositing (returns per-layer scenes)
    let composite_result = render_document_for_compositing(
        document,
        base_transform,
        image_cache,
        video_manager,
        None, // No webcam during export
        None, // No floating selection during export
        false, // No checkerboard in export
    );

    // Video export is never transparent.
    composite_document_to_hdr(&composite_result, document, device, queue, renderer, gpu_resources, width, height, false)?;

    // Use persistent output texture (already created in ExportGpuResources)
    let output_view = &gpu_resources.output_texture_view;

    // Convert HDR to sRGB for output
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("export_linear_to_srgb_bind_group"),
        layout: &gpu_resources.linear_to_srgb_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&gpu_resources.hdr_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&gpu_resources.linear_to_srgb_sampler),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("export_linear_to_srgb_encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("export_linear_to_srgb_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&gpu_resources.linear_to_srgb_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }

    queue.submit(Some(encoder.finish()));

    // GPU YUV conversion: Convert RGBA output to YUV420p
    let mut yuv_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("export_yuv_conversion_encoder"),
    });

    gpu_resources.yuv_converter.convert(
        device,
        &mut yuv_encoder,
        output_view,
        &gpu_resources.yuv_texture_view,
        width,
        height,
    );

    // Copy YUV texture to persistent staging buffer
    let yuv_height = height + height / 2; // Y plane + U plane + V plane
    yuv_encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu_resources.yuv_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &gpu_resources.staging_buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4), // Rgba8Unorm = 4 bytes per pixel
                rows_per_image: Some(yuv_height),
            },
        },
        wgpu::Extent3d {
            width,
            height: yuv_height,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(yuv_encoder.finish()));

    // Map buffer and read YUV pixels (synchronous)
    let buffer_slice = gpu_resources.staging_buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
        sender.send(result).ok();
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());

    receiver
        .recv()
        .map_err(|_| "Failed to receive buffer mapping result")?
        .map_err(|e| format!("Failed to map buffer: {:?}", e))?;

    // Extract Y, U, V planes from packed YUV buffer
    let data = buffer_slice.get_mapped_range();
    let width_usize = width as usize;
    let height_usize = height as usize;

    // Y plane: rows 0 to height-1 (extract R channel from Rgba8Unorm)
    let y_plane_size = width_usize * height_usize;
    let mut y_plane = vec![0u8; y_plane_size];
    for y in 0..height_usize {
        let src_row_offset = y * width_usize * 4; // 4 bytes per pixel (Rgba8Unorm)
        let dst_row_offset = y * width_usize;
        for x in 0..width_usize {
            y_plane[dst_row_offset + x] = data[src_row_offset + x * 4]; // Extract R channel
        }
    }

    // U and V planes: rows height to height + height/2 - 1 (half resolution, side-by-side layout)
    // U plane is in left half (columns 0 to width/2-1), V plane is in right half (columns width/2 to width-1)
    let chroma_width = width_usize / 2;
    let chroma_height = height_usize / 2;
    let chroma_row_start = height_usize * width_usize * 4; // Start of chroma rows in bytes

    let mut u_plane = vec![0u8; chroma_width * chroma_height];
    let mut v_plane = vec![0u8; chroma_width * chroma_height];

    for y in 0..chroma_height {
        let row_offset = chroma_row_start + y * width_usize * 4; // Full width rows in chroma region

        // Extract U plane (left half: columns 0 to chroma_width-1)
        let u_start = row_offset;
        let dst_offset = y * chroma_width;
        for x in 0..chroma_width {
            u_plane[dst_offset + x] = data[u_start + x * 4]; // Extract R channel
        }

        // Extract V plane (right half: columns width/2 to width/2+chroma_width-1)
        let v_start = row_offset + chroma_width * 4;
        for x in 0..chroma_width {
            v_plane[dst_offset + x] = data[v_start + x * 4]; // Extract R channel
        }
    }

    drop(data);
    gpu_resources.staging_buffer.unmap();

    Ok((y_plane, u_plane, v_plane))
}

/// Render frame to GPU RGBA texture (non-blocking, for async pipeline)
///
/// Similar to render_frame_to_rgba_hdr but renders to an external RGBA texture view
/// (provided by ReadbackPipeline) and returns the command encoder WITHOUT blocking on readback.
/// The caller (ReadbackPipeline) will submit the encoder and handle async readback.
///
/// # Arguments
/// * `document` - Document to render
/// * `timestamp` - Time in seconds to render at
/// * `width` - Frame width in pixels
/// * `height` - Frame height in pixels
/// * `device` - wgpu device
/// * `queue` - wgpu queue
/// * `renderer` - Vello renderer
/// * `image_cache` - Image cache for rendering
/// * `video_manager` - Video manager for video clips
/// * `gpu_resources` - HDR GPU resources for compositing
/// * `rgba_texture_view` - External RGBA texture view (from ReadbackPipeline)
///
/// # Returns
/// Command encoder ready for submission (caller submits via ReadbackPipeline)
/// Fault in raster keyframe pixels needed to composite the document at its current
/// time, decoding them from the project `.beam` container via `raster_store`.
///
/// Mutates the document in place: for every raster layer's active keyframe whose
/// `raw_pixels` are empty, loads + sets them (and marks `texture_dirty`). A no-op
/// when `raster_store` is `None`/unsaved or everything is already resident.
fn fault_in_raster_for_frame(
    document: &mut Document,
    raster_store: Option<&lightningbeam_core::raster_store::RasterStore>,
) {
    let store = match raster_store {
        Some(s) if s.has_path() => s,
        _ => return,
    };
    let now = document.current_time;
    for layer in document.all_layers_mut() {
        if let lightningbeam_core::layer::AnyLayer::Raster(rl) = layer {
            // Resolve the active keyframe id at the current time, then fault it in.
            let kf_id = match rl.keyframe_at(now) {
                Some(kf) if kf.raw_pixels.is_empty() && kf.needs_fault_in => kf.id,
                _ => continue,
            };
            if let Some(kf) = rl.keyframes.iter_mut().find(|kf| kf.id == kf_id) {
                if let Some(pixels) = store.load_pixels(kf_id) {
                    kf.raw_pixels = pixels;
                    kf.texture_dirty = true;
                }
                kf.needs_fault_in = false;
            }
        }
    }
}

pub fn render_frame_to_gpu_rgba(
    document: &mut Document,
    timestamp: f64,
    width: u32,
    height: u32,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut vello::Renderer,
    image_cache: &mut ImageCache,
    video_manager: &Arc<std::sync::Mutex<VideoManager>>,
    gpu_resources: &mut ExportGpuResources,
    rgba_texture_view: &wgpu::TextureView,
    floating_selection: Option<&lightningbeam_core::selection::RasterFloatingSelection>,
    allow_transparency: bool,
    raster_store: Option<&lightningbeam_core::raster_store::RasterStore>,
) -> Result<wgpu::CommandEncoder, String> {
    use vello::kurbo::Affine;

    // Set document time to the frame timestamp
    document.current_time = timestamp;

    // Fault in raster keyframe pixels for this frame (Phase 3 paging). Offline
    // export renders synchronously with no "next frame", so unlike the live canvas
    // we must page the pixels in here, before compositing. Cheap no-op when every
    // keyframe is already resident or when the document is unsaved (no store path).
    fault_in_raster_for_frame(document, raster_store);

    // Scale the document to the export resolution. The core renderer bakes this
    // base transform into every layer (vector scenes, raster and video layer
    // transforms), so the whole stage scales up/down to fill the output. When the
    // export size matches the document this is the identity.
    let base_transform = if document.width > 0.0 && document.height > 0.0 {
        Affine::scale_non_uniform(
            width as f64 / document.width,
            height as f64 / document.height,
        )
    } else {
        Affine::IDENTITY
    };

    // Render document for compositing (returns per-layer scenes)
    let composite_result = render_document_for_compositing(
        document,
        base_transform,
        image_cache,
        video_manager,
        None, // No webcam during export
        floating_selection,
        false, // No checkerboard in export
    );

    composite_document_to_hdr(&composite_result, document, device, queue, renderer, gpu_resources, width, height, allow_transparency)?;

    // Convert HDR to sRGB (linear → sRGB), render directly to external RGBA texture
    let output_view = rgba_texture_view;
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("export_linear_to_srgb_bind_group"),
        layout: &gpu_resources.linear_to_srgb_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&gpu_resources.hdr_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&gpu_resources.linear_to_srgb_sampler),
            },
        ],
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("export_linear_to_srgb_encoder"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("export_linear_to_srgb_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&gpu_resources.linear_to_srgb_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }

    // Return encoder for caller to submit (ReadbackPipeline will handle submission and async readback)
    // Frame is already rendered to external RGBA texture, no GPU YUV conversion needed
    Ok(encoder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgba_to_yuv420p_white() {
        // White: R=255, G=255, B=255
        let rgba = vec![255u8, 255, 255, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Expected: Y=255 (full brightness), U=128, V=128 (neutral chroma)
        assert_eq!(y[0], 255);
        assert_eq!(u[0], 128);
        assert_eq!(v[0], 128);
    }

    #[test]
    fn test_rgba_to_yuv420p_black() {
        // Black: R=0, G=0, B=0
        let rgba = vec![0u8, 0, 0, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Expected: Y=0 (no brightness), U=128, V=128 (neutral chroma)
        assert_eq!(y[0], 0);
        assert_eq!(u[0], 128);
        assert_eq!(v[0], 128);
    }

    #[test]
    fn test_rgba_to_yuv420p_red() {
        // Red: R=255, G=0, B=0
        let rgba = vec![255u8, 0, 0, 255]; // 1 pixel
        let (y, u, v) = rgba_to_yuv420p(&rgba, 1, 1);

        // Red has:
        // - Y around 54 (low luma due to low green coefficient)
        // - U < 128 (negative blue component)
        // - V > 128 (positive red component)
        assert!(y[0] >= 50 && y[0] <= 60, "Y value: {}", y[0]);
        assert!(u[0] < 128, "U value: {}", u[0]);
        assert!(v[0] > 128, "V value: {}", v[0]);
    }

    #[test]
    fn test_rgba_to_yuv420p_dimensions() {
        // 4×4 image (16 pixels)
        let rgba = vec![0u8; 4 * 4 * 4]; // All black
        let (y, u, v) = rgba_to_yuv420p(&rgba, 4, 4);

        // Y should be full resolution: 4×4 = 16 pixels
        assert_eq!(y.len(), 16);

        // U and V should be quarter resolution: 2×2 = 4 pixels each
        assert_eq!(u.len(), 4);
        assert_eq!(v.len(), 4);
    }

    #[test]
    fn test_rgba_to_yuv420p_2x2_subsampling() {
        // Create 2×2 image with different colors in each corner
        let mut rgba = vec![0u8; 2 * 2 * 4];

        // Top-left: Red
        rgba[0] = 255;
        rgba[1] = 0;
        rgba[2] = 0;
        rgba[3] = 255;

        // Top-right: Green
        rgba[4] = 0;
        rgba[5] = 255;
        rgba[6] = 0;
        rgba[7] = 255;

        // Bottom-left: Blue
        rgba[8] = 0;
        rgba[9] = 0;
        rgba[10] = 255;
        rgba[11] = 255;

        // Bottom-right: White
        rgba[12] = 255;
        rgba[13] = 255;
        rgba[14] = 255;
        rgba[15] = 255;

        let (y, u, v) = rgba_to_yuv420p(&rgba, 2, 2);

        // Y plane should have 4 distinct values (one per pixel)
        assert_eq!(y.len(), 4);

        // U and V should have 1 value each (averaged over 2×2 block)
        assert_eq!(u.len(), 1);
        assert_eq!(v.len(), 1);

        // The averaged chroma should be close to neutral (128)
        // since we have all primary colors + white
        assert!(u[0] >= 100 && u[0] <= 156, "U value: {}", u[0]);
        assert!(v[0] >= 100 && v[0] <= 156, "V value: {}", v[0]);
    }
}
