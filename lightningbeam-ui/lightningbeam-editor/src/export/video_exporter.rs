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

/// The document→export-pixels transform for a given fit mode. Stretch distorts to fill; Letterbox
/// scales uniformly to fit (centered, black bars); Crop scales uniformly to fill (centered, trims).
pub fn export_base_transform(
    doc_w: f64,
    doc_h: f64,
    out_w: f64,
    out_h: f64,
    fit: lightningbeam_core::export::ExportFitMode,
) -> vello::kurbo::Affine {
    use lightningbeam_core::export::ExportFitMode;
    use vello::kurbo::Affine;
    if doc_w <= 0.0 || doc_h <= 0.0 {
        return Affine::IDENTITY;
    }
    let (sx, sy) = (out_w / doc_w, out_h / doc_h);
    match fit {
        ExportFitMode::Stretch => Affine::scale_non_uniform(sx, sy),
        ExportFitMode::Letterbox | ExportFitMode::Crop => {
            let s = if matches!(fit, ExportFitMode::Letterbox) { sx.min(sy) } else { sx.max(sy) };
            Affine::translate(((out_w - doc_w * s) / 2.0, (out_h - doc_h * s) / 2.0)) * Affine::scale(s)
        }
    }
}

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
    /// Variant with highlight rolloff (document HDR output mode = Highlight rolloff).
    pub linear_to_srgb_pipeline_rolloff: wgpu::RenderPipeline,
    /// Bind group layout for linear to sRGB blit
    pub linear_to_srgb_bind_group_layout: wgpu::BindGroupLayout,
    /// Sampler for linear to sRGB conversion
    pub linear_to_srgb_sampler: wgpu::Sampler,
    /// Canvas blit pipeline for raster/video/float layers (bypasses Vello).
    pub canvas_blit: crate::gpu_brush::CanvasBlitPipeline,
    /// NV12→linear blit for hardware-decoded video frames (export on the shared device).
    pub nv12_blit: crate::nv12_blit::Nv12BlitPipeline,
    /// Per-keyframe GPU texture cache for raster layers during export.
    pub raster_cache: std::collections::HashMap<uuid::Uuid, crate::gpu_brush::CanvasPair>,
    /// Cached HDR accumulator state after the (static) background is composited in. The document
    /// background doesn't change across an export, so it's rendered once and restored with a cheap
    /// texture copy each frame instead of a full Vello render + 2 passes/submits. `None` until the
    /// first frame; invalidated on resize.
    cached_bg_hdr: Option<wgpu::Texture>,
    /// HDR encode pipeline (linear→PQ/HLG BT.2020 → 10-bit YUV). Lazily built on the first HDR frame.
    hdr_pipeline: Option<super::hdr_frame::HdrFramePipeline>,
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST, // restore cached background each frame
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

        // Highlight-rolloff variant: identical but the `fs_main_rolloff` entry point.
        let linear_to_srgb_pipeline_rolloff = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("linear_to_srgb_pipeline_rolloff"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main_rolloff"),
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
        let nv12_blit = crate::nv12_blit::Nv12BlitPipeline::new(device);

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
            linear_to_srgb_pipeline_rolloff,
            linear_to_srgb_bind_group_layout,
            linear_to_srgb_sampler,
            canvas_blit,
            nv12_blit,
            raster_cache: std::collections::HashMap::new(),
            cached_bg_hdr: None,
            hdr_pipeline: None,
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.hdr_texture_view = self.hdr_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.cached_bg_hdr = None; // dimensions changed — rebuild the background cache
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

// Highlight rolloff: identity below the knee, smooth C1 rolloff [knee,∞)→[knee,1) above (recovers
// super-white HDR detail). SDR below the knee is untouched. Mirrors panes/shaders/linear_to_srgb.wgsl.
fn highlight_rolloff_ch(x: f32) -> f32 {
    let knee = 0.8;
    if x <= knee {
        return x;
    }
    let headroom = 1.0 - knee;
    return knee + headroom * (1.0 - exp(-(x - knee) / headroom));
}

// Variant of fs_main with highlight rolloff (document HDR output mode = Highlight rolloff).
@fragment
fn fs_main_rolloff(in: VertexOutput) -> @location(0) vec4<f32> {
    let src = textureSample(source_tex, source_sampler, in.uv);
    let a = src.a;
    let straight = select(src.rgb / a, vec3<f32>(0.0), a <= 0.0);
    let rolled = vec3<f32>(
        highlight_rolloff_ch(straight.r),
        highlight_rolloff_ch(straight.g),
        highlight_rolloff_ch(straight.b),
    );
    let srgb = linear_to_srgb(rolled);
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
    hdr: lightningbeam_core::export::HdrExportMode,
    full_range: bool,
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
    // ProRes needs 10-bit 4:2:2; HDR needs 10-bit 4:2:0 BT.2020; other SDR is 8-bit 4:2:0.
    let is_prores = codec_id == ffmpeg::codec::Id::PRORES;
    if hdr.is_hdr() {
        encoder.set_format(ffmpeg::format::Pixel::YUV420P10LE);
    } else if is_prores {
        encoder.set_format(ffmpeg::format::Pixel::YUV422P10LE);
    } else {
        encoder.set_format(ffmpeg::format::Pixel::YUV420P);
    }
    encoder.set_time_base(ffmpeg::Rational(1, (framerate * 1000.0) as i32));
    encoder.set_frame_rate(Some(ffmpeg::Rational(framerate as i32, 1)));
    encoder.set_bit_rate((bitrate_kbps * 1000) as usize);
    encoder.set_gop(framerate as u32); // 1 second GOP

    // Tag the color metadata so players interpret the YUV correctly.
    // SDR: our RGB→YUV uses the BT.709 matrix with FULL-range (0–255) luma and no transfer applied
    // to the already-sRGB-encoded RGB, so tag full-range BT.709 to avoid level/hue shifts.
    // HDR: BT.2020 non-constant-luminance matrix, LIMITED range (standard for HDR10/HLG), with the
    // PQ or HLG transfer; the 10-bit YUV is produced from PQ/HLG-encoded BT.2020 RGB.
    let mut color_opts = ffmpeg::Dictionary::new();
    if hdr.is_hdr() {
        encoder.set_colorspace(ffmpeg::color::Space::BT2020NCL);
        encoder.set_color_range(ffmpeg::color::Range::MPEG); // limited
        color_opts.set("color_primaries", "bt2020");
        color_opts.set("color_trc", hdr.transfer_name());
        // HEVC 10-bit profile (the only HDR-capable codec we wire up).
        color_opts.set("profile", "main10");
    } else {
        encoder.set_colorspace(ffmpeg::color::Space::BT709);
        // Range must match what the YUV converters (gpu_yuv / cpu_yuv) actually produce.
        encoder.set_color_range(if full_range {
            ffmpeg::color::Range::JPEG // full (PC, 0–255)
        } else {
            ffmpeg::color::Range::MPEG // limited (TV, 16–235)
        });
        color_opts.set("color_primaries", "bt709");
        color_opts.set("color_trc", "bt709");
        if is_prores {
            // prores_ks profile: 3 = HQ (4:2:2 10-bit). Matches the YUV422P10LE frames we feed.
            color_opts.set("profile", "3");
        }
    }

    println!("📐 Video dimensions: {}×{} (aligned to {}×{}){}",
             width, height, aligned_width, aligned_height,
             if hdr.is_hdr() { " [HDR 10-bit BT.2020]" } else { "" });

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

    let prof = render_profile_enabled();
    let t_c0 = std::time::Instant::now();

    // --- Background (cached) ---
    // The document background is static across an export, so render it through Vello exactly once
    // (into the accumulator) and snapshot the result; every later frame restores it with a single
    // GPU texture copy instead of a Vello render + sRGB-convert + composite (+2 submits).
    let bg_cached = matches!(
        &gpu_resources.cached_bg_hdr,
        Some(t) if t.width() == width && t.height() == height
    );
    let copy_size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    if bg_cached {
        // Restore the cached background into the accumulator.
        let cached = gpu_resources.cached_bg_hdr.as_ref().unwrap();
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_bg_restore") });
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: cached, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &gpu_resources.hdr_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            copy_size,
        );
        queue.submit(Some(enc.finish()));
    } else {
        // First frame (or after a resize): full background render into the accumulator.
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

        // Snapshot the composited background for reuse on subsequent frames.
        let cached = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("export_cached_bg_hdr"),
            size: copy_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("export_bg_snapshot") });
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &gpu_resources.hdr_texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &cached, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            copy_size,
        );
        queue.submit(Some(enc.finish()));
        gpu_resources.cached_bg_hdr = Some(cached);
    }
    let t_bg = std::time::Instant::now();

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
                    if inst.gpu.is_none() && inst.rgba_data.is_empty() { continue; }
                    let hdr_layer_handle = gpu_resources.buffer_pool.acquire(device, hdr_spec);
                    if let Some(hdr_layer_view) = gpu_resources.buffer_pool.get_view(hdr_layer_handle) {
                        let bt = crate::gpu_brush::BlitTransform::new(inst.transform, inst.width, inst.height, width, height);
                        if let Some(gpu) = &inst.gpu {
                            // Hardware-decoded NV12 plane textures → linear, no CPU upload.
                            let y_view = gpu.y.create_view(&Default::default());
                            let uv_view = gpu.uv.create_view(&Default::default());
                            gpu_resources.nv12_blit.blit(
                                device, queue, &y_view, &uv_view, hdr_layer_view, &bt,
                                gpu.full_range, gpu.coeffs, gpu.transfer, gpu.primaries,
                            );
                        } else {
                            // Upload raw sRGB straight-alpha bytes into an sRGB texture; the GPU
                            // decodes to linear on sample (no per-pixel CPU conversion). Blit with
                            // blit_straight so the shader doesn't unpremultiply.
                            let tex = upload_transient_texture(device, queue, &inst.rgba_data, inst.width, inst.height, wgpu::TextureFormat::Rgba8UnormSrgb, Some("export_video_frame_tex"));
                            let tex_view = tex.create_view(&Default::default());
                            gpu_resources.canvas_blit.blit_straight(device, queue, &tex_view, hdr_layer_view, &bt, None);
                        }
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
                    let tex = upload_transient_texture(device, queue, &linear, *fw, *fh, wgpu::TextureFormat::Rgba8Unorm, Some("export_float_tex"));
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
                    let tempo_map = document.tempo_map();
                    let effect_end_beats = effect_instance.timeline_start
                        + effect_instance.effective_duration(daw_backend::Seconds(lightningbeam_core::effect::EFFECT_DURATION), tempo_map);
                    let effect_inst = lightningbeam_core::effect::EffectInstance::new(
                        effect_def,
                        tempo_map.beats_to_seconds(effect_instance.timeline_start).seconds_to_f64(),
                        tempo_map.beats_to_seconds(effect_end_beats).seconds_to_f64(),
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

    if prof {
        record_composite_profile(t_bg.duration_since(t_c0), t_bg.elapsed());
    }

    gpu_resources.buffer_pool.next_frame();
    Ok(())
}

/// Split of `composite_document_to_hdr`: static-background re-render vs. the layer loop
/// (video upload + blits). Prints a running average every 200 frames under LB_RENDER_PROFILE.
fn record_composite_profile(background: std::time::Duration, layers: std::time::Duration) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static BG_US: AtomicU64 = AtomicU64::new(0);
    static LAYERS_US: AtomicU64 = AtomicU64::new(0);
    static N: AtomicU64 = AtomicU64::new(0);
    BG_US.fetch_add(background.as_micros() as u64, Ordering::Relaxed);
    LAYERS_US.fetch_add(layers.as_micros() as u64, Ordering::Relaxed);
    let n = N.fetch_add(1, Ordering::Relaxed) + 1;
    if n % 200 == 0 {
        println!(
            "📊 [COMPOSITE PROFILE] {n} frames avg: background-render {:.2}ms | layers(video upload+blit) {:.2}ms",
            BG_US.load(Ordering::Relaxed) as f64 / n as f64 / 1000.0,
            LAYERS_US.load(Ordering::Relaxed) as f64 / n as f64 / 1000.0,
        );
    }
}

/// Upload `pixels` to a transient GPU texture (TEXTURE_BINDING | COPY_DST) in the
/// given format. Use `Rgba8UnormSrgb` to upload raw sRGB bytes and let the GPU
/// decode to linear on sample (no CPU conversion).
fn upload_transient_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pixels: &[u8],
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: Option<&'static str>,
) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label,
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
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

/// Render frame to GPU RGBA texture (non-blocking, for async pipeline)
///
/// Renders to an external RGBA texture view
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

/// Render one frame as 10-bit HDR YUV420P10LE planes (BT.2020 + PQ/HLG). Synchronous: composites,
/// runs the linear→PQ/HLG GPU pass, reads it back, and CPU-converts to 10-bit YUV. Used by the
/// HDR export path instead of the async readback pipeline.
#[allow(clippy::too_many_arguments)]
pub fn render_frame_to_yuv10_hdr(
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
    hdr_mode: lightningbeam_core::export::HdrExportMode,
    fit: lightningbeam_core::export::ExportFitMode,
    raster_store: Option<&lightningbeam_core::raster_store::RasterStore>,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), String> {
    document.current_time = timestamp;
    fault_in_raster_for_frame(document, raster_store);

    let base_transform = export_base_transform(document.width, document.height, width as f64, height as f64, fit);

    // HDR export composites on the shared device, so it can consume hardware-decoded GPU frames.
    if let Ok(mut vm) = video_manager.lock() {
        vm.set_render_hardware_ok(true);
    }

    let composite_result = render_document_for_compositing(
        document, base_transform, image_cache, video_manager, None, None, false,
    );
    composite_document_to_hdr(&composite_result, document, device, queue, renderer, gpu_resources, width, height, false)?;

    if gpu_resources.hdr_pipeline.is_none() {
        gpu_resources.hdr_pipeline = Some(super::hdr_frame::HdrFramePipeline::new(device, width, height));
    }
    let planes = gpu_resources
        .hdr_pipeline
        .as_ref()
        .unwrap()
        .render_to_yuv10(device, queue, &gpu_resources.hdr_texture_view, hdr_mode);
    Ok(planes)
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
    // True when compositing on the shared device (software/image export) → may consume
    // hardware-decoded GPU frames; false for the zero-copy path on its own device.
    hardware_ok: bool,
    fit: lightningbeam_core::export::ExportFitMode,
) -> Result<wgpu::CommandEncoder, String> {
    // One-shot profiling of the render-bucket split (LB_RENDER_PROFILE=1): how much of the
    // per-frame CPU "render" is document build (incl. video decode) vs. composite-command
    // recording (incl. the frame texture upload) vs. the sRGB pass. Prints a running average.
    let prof = render_profile_enabled();
    let t0 = std::time::Instant::now();

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
    let base_transform = export_base_transform(document.width, document.height, width as f64, height as f64, fit);

    // GPU frames are usable only on the shared device (software/image export); the zero-copy path
    // runs on its own device and must download to CPU.
    if let Ok(mut vm) = video_manager.lock() {
        vm.set_render_hardware_ok(hardware_ok);
    }

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
    let t_build = std::time::Instant::now();

    composite_document_to_hdr(&composite_result, document, device, queue, renderer, gpu_resources, width, height, allow_transparency)?;
    let t_composite = std::time::Instant::now();

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

        let final_pipeline = match document.hdr_output_mode {
            lightningbeam_core::document::HdrOutputMode::HighlightRolloff => &gpu_resources.linear_to_srgb_pipeline_rolloff,
            lightningbeam_core::document::HdrOutputMode::Clip => &gpu_resources.linear_to_srgb_pipeline,
        };
        render_pass.set_pipeline(final_pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..4, 0..1);
    }

    if prof {
        record_render_profile(
            t_build.duration_since(t0),
            t_composite.duration_since(t_build),
            t_composite.elapsed(),
        );
    }

    // Return encoder for caller to submit (ReadbackPipeline will handle submission and async readback)
    // Frame is already rendered to external RGBA texture, no GPU YUV conversion needed
    Ok(encoder)
}

/// `LB_RENDER_PROFILE` gate, checked once.
fn render_profile_enabled() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("LB_RENDER_PROFILE").is_ok())
}

/// Accumulate the per-frame render split and print a running average every 200 frames.
/// `build` = document build incl. video decode; `composite` = composite-command recording
/// incl. the frame texture upload; `srgb` = the linear→sRGB pass.
fn record_render_profile(build: std::time::Duration, composite: std::time::Duration, srgb: std::time::Duration) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static BUILD_US: AtomicU64 = AtomicU64::new(0);
    static COMPOSITE_US: AtomicU64 = AtomicU64::new(0);
    static SRGB_US: AtomicU64 = AtomicU64::new(0);
    static N: AtomicU64 = AtomicU64::new(0);
    BUILD_US.fetch_add(build.as_micros() as u64, Ordering::Relaxed);
    COMPOSITE_US.fetch_add(composite.as_micros() as u64, Ordering::Relaxed);
    SRGB_US.fetch_add(srgb.as_micros() as u64, Ordering::Relaxed);
    let n = N.fetch_add(1, Ordering::Relaxed) + 1;
    if n % 200 == 0 {
        let (b, c, s) = (BUILD_US.load(Ordering::Relaxed), COMPOSITE_US.load(Ordering::Relaxed), SRGB_US.load(Ordering::Relaxed));
        println!(
            "📊 [RENDER PROFILE] {n} frames avg: build(+decode) {:.2}ms | composite(+upload) {:.2}ms | srgb {:.2}ms",
            b as f64 / n as f64 / 1000.0,
            c as f64 / n as f64 / 1000.0,
            s as f64 / n as f64 / 1000.0,
        );
    }
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

    /// ProRes must actually open with the 10-bit 4:2:2 format we now feed it. Before the fix the
    /// SDR path handed prores_ks 8-bit YUV420P and `open` failed every time — so this opening
    /// successfully is the regression guard for "ProRes export always errored".
    #[test]
    fn prores_encoder_opens_with_yuv422p10() {
        ffmpeg::init().unwrap();
        // Skip cleanly if this ffmpeg build lacks a ProRes encoder (rather than false-fail).
        if ffmpeg::encoder::find(ffmpeg::codec::Id::PRORES).is_none()
            && ffmpeg::encoder::find_by_name("prores_ks").is_none()
        {
            eprintln!("prores encoder not present in this ffmpeg build; skipping");
            return;
        }
        let r = setup_video_encoder(
            ffmpeg::codec::Id::PRORES,
            640, 480, 30.0, 20_000,
            lightningbeam_core::export::HdrExportMode::Sdr,
            false,
        );
        assert!(r.is_ok(), "ProRes encoder failed to open: {:?}", r.err());
        let (encoder, _codec) = r.unwrap();
        assert_eq!(encoder.format(), ffmpeg::format::Pixel::YUV422P10LE);
    }

    // NOTE: `rgba_to_yuv420p` rounds dimensions up to multiples of 16 (H.264
    // macroblock alignment), so its plane lengths are the aligned sizes, not the
    // tight input dimensions. The former `test_rgba_to_yuv420p_dimensions` and
    // `_2x2_subsampling` tests asserted tight sizes and were removed when that
    // alignment was added. (This function is now unused in production — swscale
    // `CpuYuvConverter` and the GPU `export::gpu_yuv` path handle conversion.)
}
