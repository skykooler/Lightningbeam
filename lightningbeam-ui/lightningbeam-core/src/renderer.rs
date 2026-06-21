//! Rendering system for Lightningbeam documents
//!
//! Renders documents to Vello scenes for GPU-accelerated display.
//!
//! This module supports two rendering modes:
//! 1. **Legacy mode**: All layers rendered to a single Scene (simple, fast)
//! 2. **Compositing mode**: Each layer rendered to its own Scene for HDR compositing
//!
//! The compositing mode enables proper per-layer opacity, blend modes, and effects.

use crate::animation::TransformProperty;
use crate::clip::{ClipInstance, ImageAsset};
use crate::document::Document;
use crate::gpu::BlendMode;
use crate::layer::{AnyLayer, LayerTrait, VectorLayer};
use kurbo::Affine;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use vello::kurbo::Rect;
use vello::peniko::{Blob, Fill, ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
use vello::Scene;

/// Cache for decoded image data to avoid re-decoding every frame
/// Decoded-image cache, bounded by a byte budget with usage-LRU eviction (Phase 4
/// asset paging). The decoded RGBA (~`w·h·4` per image) is the heavy, evictable cost;
/// a miss re-decodes from `asset.data`. Recency is bumped on every access, so images
/// actually rendered each frame stay resident and unused ones age out under pressure.
pub struct ImageCache {
    cache: HashMap<Uuid, Arc<ImageBrush>>,
    /// CPU path: tiny-skia pixmaps decoded from the same assets (premultiplied RGBA8)
    cpu_cache: HashMap<Uuid, Arc<tiny_skia::Pixmap>>,
    /// Recency order (least-recent first) of resident asset ids.
    lru: Vec<Uuid>,
    /// Decoded bytes per resident asset (counted once; GPU/CPU are ~equal and a render
    /// session uses one path) and the running total.
    sizes: HashMap<Uuid, usize>,
    bytes: usize,
    /// `.beam` container path for lazily loading compressed `ImageAsset` bytes on a
    /// decode miss (Tier 1 paging) when `asset.data` isn't resident.
    container_path: Option<std::path::PathBuf>,
}

impl ImageCache {
    /// Max decoded-image bytes kept resident before LRU eviction.
    const BUDGET: usize = 256 * 1024 * 1024;

    /// Create a new empty image cache
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            cpu_cache: HashMap::new(),
            lru: Vec::new(),
            sizes: HashMap::new(),
            bytes: 0,
            container_path: None,
        }
    }

    /// Set the `.beam` container path used to lazily load image bytes that aren't
    /// resident in `asset.data` (Tier 1 paging). Cheap to call each frame.
    pub fn set_container_path(&mut self, path: Option<std::path::PathBuf>) {
        self.container_path = path;
    }

    /// Resolve an asset's compressed bytes: prefer the resident `asset.data` (imported
    /// this session, or an old base64 project), else page from the container.
    fn resolve_bytes<'a>(&self, asset: &'a ImageAsset) -> Option<std::borrow::Cow<'a, [u8]>> {
        if let Some(d) = &asset.data {
            return Some(std::borrow::Cow::Borrowed(d.as_slice()));
        }
        let path = self.container_path.as_ref()?;
        crate::beam_archive::read_packed_media_readonly(path, asset.id)
            .ok()
            .flatten()
            .map(std::borrow::Cow::Owned)
    }

    /// Mark `id` (size `size` bytes) as most-recently-used; evict LRU entries over budget.
    fn touch(&mut self, id: Uuid, size: usize) {
        if !self.sizes.contains_key(&id) {
            self.sizes.insert(id, size);
            self.bytes += size;
        }
        if let Some(pos) = self.lru.iter().position(|x| *x == id) {
            self.lru.remove(pos);
        }
        self.lru.push(id);
        // Keep at least the just-touched entry resident.
        while self.bytes > Self::BUDGET && self.lru.len() > 1 {
            let old = self.lru.remove(0);
            self.cache.remove(&old);
            self.cpu_cache.remove(&old);
            if let Some(sz) = self.sizes.remove(&old) {
                self.bytes -= sz;
            }
        }
    }

    /// Get or decode an image, caching the result
    pub fn get_or_decode(&mut self, asset: &ImageAsset) -> Option<Arc<ImageBrush>> {
        let size = (asset.width as usize) * (asset.height as usize) * 4;
        if let Some(cached) = self.cache.get(&asset.id).map(Arc::clone) {
            self.touch(asset.id, size);
            return Some(cached);
        }

        // Decode and cache (bytes from asset.data or paged from the container).
        let bytes = self.resolve_bytes(asset)?;
        let image = decode_image_brush(&bytes)?;
        let arc_image = Arc::new(image);
        self.cache.insert(asset.id, Arc::clone(&arc_image));
        self.touch(asset.id, size);
        Some(arc_image)
    }

    /// Get or decode an image as a premultiplied tiny-skia Pixmap (CPU render path).
    pub fn get_or_decode_cpu(&mut self, asset: &ImageAsset) -> Option<Arc<tiny_skia::Pixmap>> {
        let size = (asset.width as usize) * (asset.height as usize) * 4;
        if let Some(cached) = self.cpu_cache.get(&asset.id).map(Arc::clone) {
            self.touch(asset.id, size);
            return Some(cached);
        }

        let bytes = self.resolve_bytes(asset)?;
        let pixmap = decode_image_to_pixmap(&bytes)?;
        let arc = Arc::new(pixmap);
        self.cpu_cache.insert(asset.id, Arc::clone(&arc));
        self.touch(asset.id, size);
        Some(arc)
    }

    /// Clear cache entry when an image asset is deleted or modified
    pub fn invalidate(&mut self, id: &Uuid) {
        self.cache.remove(id);
        self.cpu_cache.remove(id);
        if let Some(pos) = self.lru.iter().position(|x| x == id) {
            self.lru.remove(pos);
        }
        if let Some(sz) = self.sizes.remove(id) {
            self.bytes -= sz;
        }
    }

    /// Clear all cached images
    pub fn clear(&mut self) {
        self.cache.clear();
        self.cpu_cache.clear();
        self.lru.clear();
        self.sizes.clear();
        self.bytes = 0;
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode image bytes to a premultiplied tiny-skia Pixmap (CPU render path).
fn decode_image_to_pixmap(data: &[u8]) -> Option<tiny_skia::Pixmap> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (iw, ih) = rgba.dimensions();
    let mut pixmap = tiny_skia::Pixmap::new(iw, ih)?;
    for (dst, src) in pixmap.pixels_mut().iter_mut().zip(rgba.pixels()) {
        let [r, g, b, a] = src.0;
        // Convert straight alpha (image crate output) to premultiplied (tiny-skia internal format)
        let af = a as f32 / 255.0;
        let pr = (r as f32 * af).round() as u8;
        let pg = (g as f32 * af).round() as u8;
        let pb = (b as f32 * af).round() as u8;
        // from_rgba only fails when channel > alpha; premultiplied values are always ≤ alpha
        *dst = tiny_skia::PremultipliedColorU8::from_rgba(pr, pg, pb, a).unwrap();
    }
    Some(pixmap)
}

/// Decode image bytes to a peniko ImageBrush (GPU render path).
fn decode_image_brush(data: &[u8]) -> Option<ImageBrush> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let (iw, ih) = rgba.dimensions();

    let image_data = ImageData {
        data: Blob::from(rgba.into_raw()),
        format: ImageFormat::Rgba8,
        width: iw,
        height: ih,
        alpha_type: ImageAlphaType::Alpha,
    };
    Some(ImageBrush::new(image_data))
}

// ============================================================================
// Per-Layer Rendering for HDR Compositing Pipeline
// ============================================================================

/// A single decoded video frame ready for GPU upload, with its document-space transform.
pub struct VideoRenderInstance {
    /// sRGB RGBA8 pixel data (straight alpha — as decoded by ffmpeg).
    pub rgba_data: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    /// Affine transform that maps from video-pixel space to document space.
    /// Composed from the clip's animated position/rotation/scale properties.
    pub transform: Affine,
    /// Final opacity [0,1] after cascading layer and instance opacity.
    pub opacity: f32,
}

/// Type of rendered layer for compositor handling
pub enum RenderedLayerType {
    /// Vector / group layer — Vello scene in `RenderedLayer::scene` is used.
    Vector,
    /// Raster keyframe — bypass Vello; compositor uploads pixels via GPU texture cache.
    Raster {
        kf_id: Uuid,
        width: u32,
        height: u32,
        /// True when `raw_pixels` changed since the last upload; forces a cache re-upload.
        dirty: bool,
        /// Accumulated parent-clip affine (IDENTITY for top-level layers).
        /// Compositor composes this with the camera into the blit matrix.
        transform: Affine,
    },
    /// Video layer — bypass Vello; each active clip instance carries decoded frame data.
    Video {
        instances: Vec<VideoRenderInstance>,
    },
    /// Floating raster selection — blitted immediately above its parent layer.
    Float {
        canvas_id: Uuid,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        /// Accumulated parent-clip affine (IDENTITY for top-level layers).
        transform: Affine,
        /// CPU pixel data (sRGB-premultiplied RGBA8). Arc so the per-frame clone is O(1).
        /// Used by the export compositor; the live compositor reads the GPU canvas directly.
        pixels: std::sync::Arc<Vec<u8>>,
    },
    /// Effect layer — applied as a post-process pass on the HDR accumulator.
    Effect {
        effect_instances: Vec<ClipInstance>,
    },
}

/// Metadata for a rendered layer, used for compositing
pub struct RenderedLayer {
    /// The layer's unique identifier
    pub layer_id: Uuid,
    /// Vello scene — only populated for `RenderedLayerType::Vector` in GPU mode.
    pub scene: Scene,
    /// CPU-rendered pixmap — `Some` for `RenderedLayerType::Vector` in CPU mode, `None` otherwise.
    /// When `Some`, `scene` is empty; the pixmap is uploaded directly to the GPU texture.
    pub cpu_pixmap: Option<tiny_skia::Pixmap>,
    /// Layer opacity (0.0 to 1.0)
    pub opacity: f32,
    /// Blend mode for compositing
    pub blend_mode: BlendMode,
    /// Whether this layer has any visible content
    pub has_content: bool,
    /// Layer variant — determines how the compositor renders this entry.
    pub layer_type: RenderedLayerType,
}

impl RenderedLayer {
    /// Create a new vector layer with default settings.
    pub fn new(layer_id: Uuid) -> Self {
        Self {
            layer_id,
            scene: Scene::new(),
            cpu_pixmap: None,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            has_content: false,
            layer_type: RenderedLayerType::Vector,
        }
    }

    /// Create a vector layer with specific opacity and blend mode.
    pub fn with_settings(layer_id: Uuid, opacity: f32, blend_mode: BlendMode) -> Self {
        Self {
            layer_id,
            scene: Scene::new(),
            cpu_pixmap: None,
            opacity,
            blend_mode,
            has_content: false,
            layer_type: RenderedLayerType::Vector,
        }
    }

    /// Create an effect layer with active effect instances.
    pub fn effect_layer(layer_id: Uuid, opacity: f32, effect_instances: Vec<ClipInstance>) -> Self {
        let has_content = !effect_instances.is_empty();
        Self {
            layer_id,
            scene: Scene::new(),
            cpu_pixmap: None,
            opacity,
            blend_mode: BlendMode::Normal,
            has_content,
            layer_type: RenderedLayerType::Effect { effect_instances },
        }
    }
}

/// Result of rendering a document for compositing
pub struct CompositeRenderResult {
    /// Background scene — GPU mode only; empty in CPU mode.
    pub background: Scene,
    /// CPU-rendered background pixmap — `Some` in CPU mode, `None` in GPU mode.
    pub background_cpu: Option<tiny_skia::Pixmap>,
    /// Rendered layers in bottom-to-top order
    pub layers: Vec<RenderedLayer>,
    /// Document dimensions
    pub width: f64,
    pub height: f64,
}

/// Render a document for the HDR compositing pipeline
///
/// Unlike `render_document_with_transform`, this function renders each visible
/// layer to its own Scene, enabling proper per-layer opacity, blend modes,
/// and effects in the GPU compositor.
///
/// Layers are returned in bottom-to-top order for compositing.
pub fn render_document_for_compositing(
    document: &Document,
    base_transform: Affine,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
    floating_selection: Option<&crate::selection::RasterFloatingSelection>,
    draw_checkerboard: bool,
) -> CompositeRenderResult {
    let time = document.current_time;

    // Render background to its own scene
    let mut background = Scene::new();
    render_background(document, &mut background, base_transform, draw_checkerboard);

    // Check if any layers are soloed
    let any_soloed = document.visible_layers().any(|layer| layer.soloed());

    // Collect layers to render
    let layers_to_render: Vec<_> = document
        .visible_layers()
        .filter(|layer| {
            if any_soloed {
                layer.soloed()
            } else {
                true
            }
        })
        .collect();

    // Render each layer to its own scene
    let mut rendered_layers = Vec::with_capacity(layers_to_render.len());

    for layer in layers_to_render {
        let rendered = render_layer_isolated(
            document,
            time,
            layer,
            base_transform,
            image_cache,
            video_manager,
            camera_frame,
        );
        rendered_layers.push(rendered);
    }

    // Insert the floating raster selection immediately above its parent layer.
    // This ensures it composites at the correct z-position in both edit and export.
    if let Some(float_sel) = floating_selection {
        if let Some(pos) = rendered_layers.iter().position(|l| l.layer_id == float_sel.layer_id) {
            // Inherit the parent layer's transform so the float follows it into
            // any transformed clip context.
            let parent_transform = match &rendered_layers[pos].layer_type {
                RenderedLayerType::Raster { transform, .. } => *transform,
                _ => Affine::IDENTITY,
            };
            let float_entry = RenderedLayer {
                layer_id: Uuid::nil(), // sentinel — not a real document layer
                scene: Scene::new(),
                cpu_pixmap: None,
                opacity: 1.0,
                blend_mode: crate::gpu::BlendMode::Normal,
                has_content: !float_sel.pixels.is_empty(),
                layer_type: RenderedLayerType::Float {
                    canvas_id: float_sel.canvas_id,
                    x: float_sel.x,
                    y: float_sel.y,
                    width: float_sel.width,
                    height: float_sel.height,
                    transform: parent_transform,
                    pixels: std::sync::Arc::clone(&float_sel.pixels),
                },
            };
            rendered_layers.insert(pos + 1, float_entry);
        }
    }

    CompositeRenderResult {
        background,
        background_cpu: None,
        layers: rendered_layers,
        width: document.width,
        height: document.height,
    }
}

/// Render a single layer to its own isolated Scene
///
/// The layer is rendered with full opacity in its scene; the actual opacity
/// will be applied during compositing. This enables proper alpha blending
/// for nested clips and complex layer hierarchies.
pub fn render_layer_isolated(
    document: &Document,
    time: f64,
    layer: &AnyLayer,
    base_transform: Affine,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
) -> RenderedLayer {
    let layer_id = layer.id();
    let opacity = layer.opacity() as f32;

    // TODO: When we add blend mode support to layers, read it here
    let blend_mode = BlendMode::Normal;

    let mut rendered = RenderedLayer::with_settings(layer_id, opacity, blend_mode);

    // Render layer content with full opacity (1.0) - opacity applied during compositing
    match layer {
        AnyLayer::Vector(vector_layer) => {
            render_vector_layer_to_scene(
                document,
                time,
                vector_layer,
                &mut rendered.scene,
                base_transform,
                1.0, // Full opacity - layer opacity handled in compositing
                image_cache,
                video_manager,
            );
            rendered.has_content = vector_layer.graph_at_time(time)
                .map_or(false, |graph| !graph.edges.iter().all(|e| e.deleted) || !graph.fills.iter().all(|f| f.deleted))
                || !vector_layer.clip_instances.is_empty();
        }
        AnyLayer::Audio(_) => {
            // Audio layers don't render visually
            rendered.has_content = false;
        }
        AnyLayer::Video(video_layer) => {
            use crate::animation::TransformProperty;
            let layer_opacity = layer.opacity();
            let mut video_mgr = video_manager.lock().unwrap();
            let mut instances = Vec::new();

            let tempo_map = document.tempo_map();
            for clip_instance in &video_layer.clip_instances {
                let Some(video_clip) = document.video_clips.get(&clip_instance.clip_id) else { continue };
                let Some(clip_time) = clip_instance.remap_time(time, video_clip.duration, tempo_map) else { continue };
                let Some(frame) = video_mgr.get_frame(&clip_instance.clip_id, clip_time) else { continue };

                // Evaluate animated transform properties.
                let anim = &video_layer.layer.animation_data;
                let id = clip_instance.id;
                let t = &clip_instance.transform;
                let x        = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::X        }, time, t.x);
                let y        = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::Y        }, time, t.y);
                let rotation = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::Rotation }, time, t.rotation);
                let scale_x  = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::ScaleX  }, time, t.scale_x);
                let scale_y  = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::ScaleY  }, time, t.scale_y);
                let skew_x   = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::SkewX   }, time, t.skew_x);
                let skew_y   = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::SkewY   }, time, t.skew_y);
                let inst_opacity = anim.eval(&crate::animation::AnimationTarget::Object { id, property: TransformProperty::Opacity }, time, clip_instance.opacity);

                let cx = video_clip.width  / 2.0;
                let cy = video_clip.height / 2.0;
                let skew_transform = if skew_x != 0.0 || skew_y != 0.0 {
                    let sx = if skew_x != 0.0 { Affine::new([1.0, 0.0, skew_x.to_radians().tan(), 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
                    let sy = if skew_y != 0.0 { Affine::new([1.0, skew_y.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
                    Affine::translate((cx, cy)) * sx * sy * Affine::translate((-cx, -cy))
                } else { Affine::IDENTITY };

                let clip_transform = Affine::translate((x, y))
                    * Affine::rotate(rotation.to_radians())
                    * Affine::scale_non_uniform(scale_x, scale_y)
                    * skew_transform;

                // The decoded frame is scaled down to fit the document (decoder caps
                // at the canvas size), so its pixel size is smaller than the clip's
                // native dimensions. The instance is blitted treating the texture as
                // `frame.width × frame.height`, while `clip_transform` is expressed in
                // the clip's native space — so scale frame-px → clip-native-px first,
                // else the frame renders small in a corner with its edges streaked.
                let frame_to_clip = if frame.width > 0 && frame.height > 0 {
                    Affine::scale_non_uniform(
                        video_clip.width / frame.width as f64,
                        video_clip.height / frame.height as f64,
                    )
                } else {
                    Affine::IDENTITY
                };
                instances.push(VideoRenderInstance {
                    rgba_data: frame.rgba_data.clone(),
                    width: frame.width,
                    height: frame.height,
                    transform: base_transform * clip_transform * frame_to_clip,
                    opacity: (layer_opacity * inst_opacity) as f32,
                });
            }

            // Camera / webcam frame.
            if instances.is_empty() && video_layer.camera_enabled {
                if let Some(frame) = camera_frame {
                    let vw = frame.width as f64;
                    let vh = frame.height as f64;
                    let scale = (document.width / vw).min(document.height / vh);
                    let ox = (document.width  - vw * scale) / 2.0;
                    let oy = (document.height - vh * scale) / 2.0;
                    let cam_transform = base_transform
                        * Affine::translate((ox, oy))
                        * Affine::scale(scale);
                    instances.push(VideoRenderInstance {
                        rgba_data: frame.rgba_data.clone(),
                        width: frame.width,
                        height: frame.height,
                        transform: cam_transform,
                        opacity: layer_opacity as f32,
                    });
                }
            }

            rendered.has_content = !instances.is_empty();
            rendered.layer_type = RenderedLayerType::Video { instances };
        }
        AnyLayer::Effect(effect_layer) => {
            // Effect layers are processed during compositing, not rendered to scene
            // Return early with a dedicated effect layer type
            let tempo_map = document.tempo_map();
            let active_effects: Vec<ClipInstance> = effect_layer
                .active_clip_instances_at(time, tempo_map)
                .into_iter()
                .cloned()
                .collect();
            return RenderedLayer::effect_layer(layer_id, opacity, active_effects);
        }
        AnyLayer::Group(group_layer) => {
            // Render each child layer's content into the group's scene
            for child in &group_layer.children {
                render_layer(
                    document, time, child, &mut rendered.scene, base_transform,
                    1.0, // Full opacity - layer opacity handled in compositing
                    image_cache, video_manager, camera_frame,
                );
            }
            rendered.has_content = !group_layer.children.is_empty();
        }
        AnyLayer::Raster(raster_layer) => {
            if let Some(kf) = raster_layer.keyframe_at(time) {
                rendered.has_content = kf.has_pixels();
                rendered.layer_type = RenderedLayerType::Raster {
                    kf_id: kf.id,
                    width: kf.width,
                    height: kf.height,
                    dirty: kf.texture_dirty,
                    transform: base_transform,
                };
            }
        }
    }

    rendered
}

/// Render a vector layer to an isolated scene (for compositing pipeline)
fn render_vector_layer_to_scene(
    document: &Document,
    time: f64,
    layer: &VectorLayer,
    scene: &mut Scene,
    base_transform: Affine,
    parent_opacity: f64,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
) {
    render_vector_layer(
        document,
        time,
        layer,
        scene,
        base_transform,
        parent_opacity,
        image_cache,
        video_manager,
    );
}

/// Render a raster layer's active keyframe to a Vello scene using an ImageBrush.
///
/// Uses `raw_pixels` directly — no PNG decode needed.
fn render_raster_layer_to_scene(
    layer: &crate::raster_layer::RasterLayer,
    time: f64,
    scene: &mut Scene,
    base_transform: Affine,
) {
    let Some(kf) = layer.keyframe_at(time) else { return };
    if kf.raw_pixels.is_empty() {
        return;
    }

    let image_data = ImageData {
        data: Blob::from(kf.raw_pixels.clone()),
        format: ImageFormat::Rgba8,
        width: kf.width,
        height: kf.height,
        // raw_pixels stores sRGB-encoded premultiplied RGBA (channels are
        // gamma-encoded, alpha is linear).  Premultiplied tells Vello to
        // decode the sRGB channels without premultiplying again.
        alpha_type: ImageAlphaType::AlphaPremultiplied,
    };
    let brush = ImageBrush::new(image_data).with_quality(ImageQuality::Low);
    let canvas_rect = Rect::new(0.0, 0.0, kf.width as f64, kf.height as f64);
    scene.fill(Fill::NonZero, base_transform, &brush, None, &canvas_rect);
}

// ============================================================================
// Legacy Single-Scene Rendering (kept for backwards compatibility)
// ============================================================================

/// Render a document to a Vello scene
pub fn render_document(
    document: &Document,
    scene: &mut Scene,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
) {
    render_document_with_transform(document, scene, Affine::IDENTITY, image_cache, video_manager);
}

/// Render a document to a Vello scene with a base transform
/// The base transform is composed with all object transforms (useful for camera zoom/pan)
pub fn render_document_with_transform(
    document: &Document,
    scene: &mut Scene,
    base_transform: Affine,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
) {
    // 1. Draw background (with checkerboard for transparent backgrounds — UI path)
    render_background(document, scene, base_transform, true);

    // 2. Recursively render the root graphics object at current time
    let time = document.current_time;

    // Check if any layers are soloed
    let any_soloed = document.visible_layers().any(|layer| layer.soloed());

    for layer in document.visible_layers() {
        if any_soloed {
            if layer.soloed() {
                render_layer(document, time, layer, scene, base_transform, 1.0, image_cache, video_manager, None);
            }
        } else {
            render_layer(document, time, layer, scene, base_transform, 1.0, image_cache, video_manager, None);
        }
    }
}

/// Draw the document background
fn render_background(document: &Document, scene: &mut Scene, base_transform: Affine, draw_checkerboard: bool) {
    let background_rect = Rect::new(0.0, 0.0, document.width, document.height);
    let bg = &document.background_color;

    // Draw checkerboard behind transparent backgrounds (UI-only; skip in export)
    if draw_checkerboard && bg.a < 255 {
        use vello::peniko::{Blob, Extend, ImageAlphaType, ImageData, ImageQuality};
        // 2x2 pixel checkerboard pattern: light/dark alternating
        let light: [u8; 4] = [204, 204, 204, 255];
        let dark: [u8; 4] = [170, 170, 170, 255];
        let pixels: Vec<u8> = [light, dark, dark, light].concat();
        let image_data = ImageData {
            data: Blob::from(pixels),
            format: ImageFormat::Rgba8,
            width: 2,
            height: 2,
            alpha_type: ImageAlphaType::AlphaPremultiplied,
        };
        let brush = ImageBrush::new(image_data)
            .with_extend(Extend::Repeat)
            .with_quality(ImageQuality::Low);
        // Scale each pixel to 16x16 document units
        let brush_transform = Affine::scale(16.0);
        scene.fill(
            Fill::NonZero,
            base_transform,
            &brush,
            Some(brush_transform),
            &background_rect,
        );
    }

    // Draw the background color on top (alpha-blended)
    let background_color = bg.to_peniko();
    scene.fill(
        Fill::NonZero,
        base_transform,
        background_color,
        None,
        &background_rect,
    );
}


/// Render a single layer
fn render_layer(
    document: &Document,
    time: f64,
    layer: &AnyLayer,
    scene: &mut Scene,
    base_transform: Affine,
    parent_opacity: f64,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
) {
    match layer {
        AnyLayer::Vector(vector_layer) => {
            render_vector_layer(document, time, vector_layer, scene, base_transform, parent_opacity, image_cache, video_manager)
        }
        AnyLayer::Audio(_) => {
            // Audio layers don't render visually
        }
        AnyLayer::Video(video_layer) => {
            let mut video_mgr = video_manager.lock().unwrap();
            let layer_camera_frame = if video_layer.camera_enabled { camera_frame } else { None };
            render_video_layer(document, time, video_layer, scene, base_transform, parent_opacity, &mut video_mgr, layer_camera_frame);
        }
        AnyLayer::Effect(_) => {
            // Effect layers are processed during GPU compositing, not rendered to scene
        }
        AnyLayer::Group(group_layer) => {
            // Render each child layer in the group
            for child in &group_layer.children {
                render_layer(document, time, child, scene, base_transform, parent_opacity, image_cache, video_manager, camera_frame);
            }
        }
        AnyLayer::Raster(raster_layer) => {
            render_raster_layer_to_scene(raster_layer, time, scene, base_transform);
        }
    }
}

/// Render a single clip instance by ID to a scene.
/// Used for re-rendering the "focused" clip on top of a dimmed scene when editing inside a clip.
pub fn render_single_clip_instance(
    document: &Document,
    scene: &mut Scene,
    base_transform: Affine,
    layer_id: &uuid::Uuid,
    instance_id: &uuid::Uuid,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
) {
    let time = document.current_time;

    // Find the layer containing this instance
    let Some(layer) = document.get_layer(layer_id) else { return };
    let AnyLayer::Vector(vector_layer) = layer else { return };

    let layer_opacity = vector_layer.layer.opacity;

    // Find the specific clip instance
    let Some(clip_instance) = vector_layer.clip_instances.iter().find(|ci| &ci.id == instance_id) else { return };

    // Compute group_end_time if needed
    let group_end_time = document.vector_clips.get(&clip_instance.clip_id)
        .filter(|vc| vc.is_group)
        .map(|_| {
            let frame_duration = 1.0 / document.framerate;
            vector_layer.group_visibility_end(&clip_instance.id, clip_instance.timeline_start, frame_duration)
        });

    render_clip_instance(
        document, time, clip_instance, layer_opacity, scene, base_transform,
        &vector_layer.layer.animation_data, image_cache, video_manager, group_end_time,
    );
}

/// Render a clip instance (recursive rendering for nested compositions)
fn render_clip_instance(
    document: &Document,
    time: f64,
    clip_instance: &crate::clip::ClipInstance,
    parent_opacity: f64,
    scene: &mut Scene,
    base_transform: Affine,
    animation_data: &crate::animation::AnimationData,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    group_end_time: Option<f64>,
) {
    // Try to find the clip in the document's clip libraries
    // For now, only handle VectorClips (VideoClip and AudioClip rendering not yet implemented)
    let Some(vector_clip) = document.vector_clips.get(&clip_instance.clip_id) else {
        return; // Clip not found or not a vector clip
    };

    // Remap timeline time to clip's internal time
    let tempo_map = document.tempo_map();
    let clip_time = if vector_clip.is_group {
        // Groups are static — visible from timeline_start to the next keyframe boundary.
        // timeline_start is in beats; group_end_time is in seconds (render time).
        let start_secs = tempo_map.transform(clip_instance.timeline_start);
        let end = group_end_time.unwrap_or(start_secs);
        if time < start_secs || time >= end {
            return;
        }
        0.0
    } else {
        let clip_dur = document.get_clip_duration(&vector_clip.id).unwrap_or(vector_clip.duration);
        let Some(t) = clip_instance.remap_time(time, clip_dur, tempo_map) else {
            return; // Clip instance not active at this time
        };
        t
    };

    // Evaluate animated transform properties
    let transform = &clip_instance.transform;
    let x = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::X,
        },
        time,
        transform.x,
    );
    let y = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::Y,
        },
        time,
        transform.y,
    );
    let rotation = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::Rotation,
        },
        time,
        transform.rotation,
    );
    let scale_x = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::ScaleX,
        },
        time,
        transform.scale_x,
    );
    let scale_y = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::ScaleY,
        },
        time,
        transform.scale_y,
    );
    let skew_x = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::SkewX,
        },
        time,
        transform.skew_x,
    );
    let skew_y = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::SkewY,
        },
        time,
        transform.skew_y,
    );

    // Build transform matrix (similar to shape instances)
    // For clip instances, we don't have a path to calculate center from,
    // so we use the clip's center point (width/2, height/2)
    let center_x = vector_clip.width / 2.0;
    let center_y = vector_clip.height / 2.0;

    // Build skew transforms (applied around clip center)
    let skew_transform = if skew_x != 0.0 || skew_y != 0.0 {
        let skew_x_affine = if skew_x != 0.0 {
            let tan_skew = skew_x.to_radians().tan();
            Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };

        let skew_y_affine = if skew_y != 0.0 {
            let tan_skew = skew_y.to_radians().tan();
            Affine::new([1.0, tan_skew, 0.0, 1.0, 0.0, 0.0])
        } else {
            Affine::IDENTITY
        };

        // Skew around center: translate to origin, skew, translate back
        Affine::translate((center_x, center_y))
            * skew_x_affine
            * skew_y_affine
            * Affine::translate((-center_x, -center_y))
    } else {
        Affine::IDENTITY
    };

    let clip_transform = Affine::translate((x, y))
        * Affine::rotate(rotation.to_radians())
        * Affine::scale_non_uniform(scale_x, scale_y)
        * skew_transform;
    let instance_transform = base_transform * clip_transform;

    // Evaluate animated opacity
    let opacity = animation_data.eval(
        &crate::animation::AnimationTarget::Object {
            id: clip_instance.id,
            property: TransformProperty::Opacity,
        },
        time,
        clip_instance.opacity,
    );

    // Cascade opacity: parent_opacity × animated opacity
    let clip_opacity = parent_opacity * opacity;

    // Recursively render all root layers in the clip at the remapped time
    for layer_node in vector_clip.layers.iter() {
        // Skip invisible layers for performance
        if !layer_node.data.visible() {
            continue;
        }
        render_layer(document, clip_time, &layer_node.data, scene, instance_transform, clip_opacity, image_cache, video_manager, None);
    }
}

/// Render a video layer with all its clip instances
fn render_video_layer(
    document: &Document,
    time: f64,
    layer: &crate::layer::VideoLayer,
    scene: &mut Scene,
    base_transform: Affine,
    parent_opacity: f64,
    video_manager: &mut crate::video::VideoManager,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
) {
    use crate::animation::TransformProperty;

    // Cascade opacity: parent_opacity × layer.opacity
    let layer_opacity = parent_opacity * layer.layer.opacity;

    // Track whether any clip was rendered at the current time
    let mut clip_rendered = false;

    // Render each video clip instance
    for clip_instance in &layer.clip_instances {
        // Get the video clip from the document
        let Some(video_clip) = document.video_clips.get(&clip_instance.clip_id) else {
            continue; // Clip not found
        };

        // Remap timeline time to clip's internal time
        let tempo_map = document.tempo_map();
        let Some(clip_time) = clip_instance.remap_time(time, video_clip.duration, tempo_map) else {
            continue; // Clip instance not active at this time
        };

        // Get video frame from VideoManager
        let Some(frame) = video_manager.get_frame(&clip_instance.clip_id, clip_time) else {
            continue; // Frame not available
        };

        // Evaluate animated transform properties
        let transform = &clip_instance.transform;
        let x = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::X,
            },
            time,
            transform.x,
        );
        let y = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::Y,
            },
            time,
            transform.y,
        );
        let rotation = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::Rotation,
            },
            time,
            transform.rotation,
        );
        let scale_x = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::ScaleX,
            },
            time,
            transform.scale_x,
        );
        let scale_y = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::ScaleY,
            },
            time,
            transform.scale_y,
        );
        let skew_x = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::SkewX,
            },
            time,
            transform.skew_x,
        );
        let skew_y = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::SkewY,
            },
            time,
            transform.skew_y,
        );

        // Build skew transform (applied around center)
        let center_x = video_clip.width / 2.0;
        let center_y = video_clip.height / 2.0;

        let skew_transform = if skew_x != 0.0 || skew_y != 0.0 {
            let skew_x_affine = if skew_x != 0.0 {
                let tan_skew = skew_x.to_radians().tan();
                Affine::new([1.0, 0.0, tan_skew, 1.0, 0.0, 0.0])
            } else {
                Affine::IDENTITY
            };

            let skew_y_affine = if skew_y != 0.0 {
                let tan_skew = skew_y.to_radians().tan();
                Affine::new([1.0, tan_skew, 0.0, 1.0, 0.0, 0.0])
            } else {
                Affine::IDENTITY
            };

            // Skew around center
            Affine::translate((center_x, center_y))
                * skew_x_affine
                * skew_y_affine
                * Affine::translate((-center_x, -center_y))
        } else {
            Affine::IDENTITY
        };

        let clip_transform = Affine::translate((x, y))
            * Affine::rotate(rotation.to_radians())
            * Affine::scale_non_uniform(scale_x, scale_y)
            * skew_transform;
        let instance_transform = base_transform * clip_transform;

        // Evaluate animated opacity
        let opacity = layer.layer.animation_data.eval(
            &crate::animation::AnimationTarget::Object {
                id: clip_instance.id,
                property: TransformProperty::Opacity,
            },
            time,
            clip_instance.opacity,
        );

        // Cascade opacity: layer_opacity × animated opacity
        let final_opacity = (layer_opacity * opacity) as f32;

        // Create peniko ImageBrush from video frame data (zero-copy via Arc clone)
        // Coerce Arc<Vec<u8>> to Arc<dyn AsRef<[u8]> + Send + Sync>
        let blob_data: Arc<dyn AsRef<[u8]> + Send + Sync> = frame.rgba_data.clone();
        let image_data = ImageData {
            data: Blob::new(blob_data),
            format: ImageFormat::Rgba8,
            width: frame.width,
            height: frame.height,
            alpha_type: ImageAlphaType::Alpha,
        };
        let image = ImageBrush::new(image_data);

        // Apply opacity
        let image_with_alpha = image.with_alpha(final_opacity);

        // Create rectangle path for the video frame
        let video_rect = Rect::new(0.0, 0.0, video_clip.width, video_clip.height);

        // The decoded frame is scaled down to fit the document (the decoder caps at
        // the canvas size to bound memory), so its pixel dimensions are smaller than
        // the clip's native display size. Scale the image brush from frame-pixel
        // space to the clip rect; without this the image is drawn 1:1 in a corner
        // and its edge pixels pad the rest (small frame with "stretched corners").
        let brush_transform = if frame.width > 0 && frame.height > 0 {
            Affine::scale_non_uniform(
                video_clip.width / frame.width as f64,
                video_clip.height / frame.height as f64,
            )
        } else {
            Affine::IDENTITY
        };

        // Render video frame as image fill
        scene.fill(
            Fill::NonZero,
            instance_transform,
            &image_with_alpha,
            Some(brush_transform),
            &video_rect,
        );
        clip_rendered = true;
    }

    // If no clip was rendered at this time and camera is enabled, show live preview
    if !clip_rendered && layer.camera_enabled {
        if let Some(frame) = camera_frame {
            let final_opacity = layer_opacity as f32;

            let blob_data: Arc<dyn AsRef<[u8]> + Send + Sync> = frame.rgba_data.clone();
            let image_data = ImageData {
                data: Blob::new(blob_data),
                format: ImageFormat::Rgba8,
                width: frame.width,
                height: frame.height,
                alpha_type: ImageAlphaType::Alpha,
            };
            let image = ImageBrush::new(image_data);
            let image_with_alpha = image.with_alpha(final_opacity);
            let frame_rect = Rect::new(0.0, 0.0, frame.width as f64, frame.height as f64);

            // Scale-to-fit and center in document (same as imported video clips)
            let video_w = frame.width as f64;
            let video_h = frame.height as f64;
            let scale_x = document.width / video_w;
            let scale_y = document.height / video_h;
            let uniform_scale = scale_x.min(scale_y);
            let scaled_w = video_w * uniform_scale;
            let scaled_h = video_h * uniform_scale;
            let offset_x = (document.width - scaled_w) / 2.0;
            let offset_y = (document.height - scaled_h) / 2.0;

            let preview_transform = base_transform
                * Affine::translate((offset_x, offset_y))
                * Affine::scale(uniform_scale);

            scene.fill(
                Fill::NonZero,
                preview_transform,
                &image_with_alpha,
                None,
                &frame_rect,
            );
        }
    }
}

/// Compute start/end canvas points for a linear gradient across a bounding box.
///
/// The axis is centred on the bbox midpoint and oriented at `angle_deg` degrees
/// (0 = left→right, 90 = top→bottom).  The axis extends ± half the bbox diagonal
/// so the gradient covers the entire shape regardless of angle.
fn gradient_bbox_endpoints(angle_deg: f32, bbox: kurbo::Rect) -> (kurbo::Point, kurbo::Point) {
    let cx = bbox.center().x;
    let cy = bbox.center().y;
    let dx = bbox.width();
    let dy = bbox.height();
    // Use half the diagonal so the full gradient fits at any angle.
    let half_len = (dx * dx + dy * dy).sqrt() * 0.5;
    let rad = (angle_deg as f64).to_radians();
    let (sin, cos) = (rad.sin(), rad.cos());
    let start = kurbo::Point::new(cx - cos * half_len, cy - sin * half_len);
    let end   = kurbo::Point::new(cx + cos * half_len, cy + sin * half_len);
    (start, end)
}

/// Render a VectorGraph to a Vello scene.
///
/// Walks fills and edges for strokes.
pub fn render_vector_graph(
    graph: &crate::vector_graph::VectorGraph,
    scene: &mut Scene,
    base_transform: Affine,
    layer_opacity: f64,
    document: &Document,
    image_cache: &mut ImageCache,
) {
    let opacity_f32 = layer_opacity as f32;

    // 1. Render fills
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue; // Skip deleted fills
        }
        if fill.color.is_none() && fill.image_fill.is_none() && fill.gradient_fill.is_none() {
            continue; // No fill to render
        }

        let fill_id = crate::vector_graph::FillId(i as u32);
        let path = graph.fill_to_bezpath(fill_id);
        let fill_rule: Fill = fill.fill_rule.into();

        let mut filled = false;

        // Image fill
        if let Some(image_asset_id) = fill.image_fill {
            if let Some(image_asset) = document.get_image_asset(&image_asset_id) {
                if let Some(image) = image_cache.get_or_decode(image_asset) {
                    let image_with_alpha = (*image).clone().with_alpha(opacity_f32);
                    // Map the image (native pixel space, origin 0,0) onto the fill's
                    // bounding box, so it sits where the shape is and scales to fit
                    // (1:1 for an image-sized rectangle).
                    let bbox = vello::kurbo::Shape::bounding_box(&path);
                    let iw = (image_asset.width.max(1)) as f64;
                    let ih = (image_asset.height.max(1)) as f64;
                    let brush_transform = Affine::translate((bbox.x0, bbox.y0))
                        * Affine::scale_non_uniform(bbox.width() / iw, bbox.height() / ih);
                    scene.fill(fill_rule, base_transform, &image_with_alpha, Some(brush_transform), &path);
                    filled = true;
                }
            }
        }

        // Gradient fill (takes priority over solid colour fill)
        if !filled {
            if let Some(ref grad) = fill.gradient_fill {
                use kurbo::Rect;
                use crate::gradient::GradientType;
                let bbox: Rect = vello::kurbo::Shape::bounding_box(&path);
                let (start, end) = match (grad.start_world, grad.end_world) {
                    (Some((sx, sy)), Some((ex, ey))) => match grad.kind {
                        GradientType::Linear => {
                            (kurbo::Point::new(sx, sy), kurbo::Point::new(ex, ey))
                        }
                        GradientType::Radial => {
                            // start_world = center, end_world = edge point.
                            // to_peniko_brush uses midpoint(start, end) as center,
                            // so reflect the edge through the center to get the
                            // opposing diameter endpoint.
                            let opp = kurbo::Point::new(2.0 * sx - ex, 2.0 * sy - ey);
                            (opp, kurbo::Point::new(ex, ey))
                        }
                    },
                    _ => gradient_bbox_endpoints(grad.angle, bbox),
                };
                let brush = grad.to_peniko_brush(start, end, opacity_f32);
                scene.fill(fill_rule, base_transform, &brush, None, &path);
                filled = true;
            }
        }

        // Solid colour fill
        if !filled {
            if let Some(fill_color) = &fill.color {
                let alpha = ((fill_color.a as f32 / 255.0) * opacity_f32 * 255.0) as u8;
                let adjusted = crate::shape::ShapeColor::rgba(
                    fill_color.r,
                    fill_color.g,
                    fill_color.b,
                    alpha,
                );
                scene.fill(fill_rule, base_transform, adjusted.to_peniko(), None, &path);
            }
        }
    }

    // 2. Render edges (strokes)
    for edge in &graph.edges {
        if edge.deleted {
            continue;
        }
        if let (Some(stroke_color), Some(stroke_style)) = (&edge.stroke_color, &edge.stroke_style) {
            let alpha = ((stroke_color.a as f32 / 255.0) * opacity_f32 * 255.0) as u8;
            let adjusted = crate::shape::ShapeColor::rgba(
                stroke_color.r,
                stroke_color.g,
                stroke_color.b,
                alpha,
            );

            let mut path = kurbo::BezPath::new();
            path.move_to(edge.curve.p0);
            path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);

            scene.stroke(
                &stroke_style.to_stroke(),
                base_transform,
                adjusted.to_peniko(),
                None,
                &path,
            );
        }
    }
}

fn render_vector_layer(
    document: &Document,
    time: f64,
    layer: &VectorLayer,
    scene: &mut Scene,
    base_transform: Affine,
    parent_opacity: f64,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
) {
    // Cascade opacity: parent_opacity × layer.opacity
    let layer_opacity = parent_opacity * layer.layer.opacity;

    // Render clip instances first (they appear under shape instances)
    for clip_instance in &layer.clip_instances {
        // For groups, compute the visibility end from keyframe data
        let group_end_time = document.vector_clips.get(&clip_instance.clip_id)
            .filter(|vc| vc.is_group)
            .map(|_| {
                let frame_duration = 1.0 / document.framerate;
                layer.group_visibility_end(&clip_instance.id, clip_instance.timeline_start, frame_duration)
            });
        render_clip_instance(document, time, clip_instance, layer_opacity, scene, base_transform, &layer.layer.animation_data, image_cache, video_manager, group_end_time);
    }

    // Render VectorGraph from active keyframe
    if let Some(graph) = layer.graph_at_time(time) {
        render_vector_graph(graph, scene, base_transform, layer_opacity, document, image_cache);
    }
}

// ============================================================================
// CPU Render Path (tiny-skia)
// ============================================================================
//
// When Vello's CPU renderer is too slow (fixed per-call overhead), we render
// vector layers to `tiny_skia::Pixmap` and upload via `queue.write_texture`.
// The GPU compositor pipeline (sRGB→linear, blend modes) is unchanged.

/// Convert a kurbo `Affine` to a tiny-skia `Transform`.
///
/// kurbo `as_coeffs()` → `[a, b, c, d, e, f]` where the matrix is:
/// ```text
/// | a  c  e |
/// | b  d  f |
/// | 0  0  1 |
/// ```
/// tiny-skia `from_row(sx, ky, kx, sy, tx, ty)` fills the same layout.
fn affine_to_ts(affine: Affine) -> tiny_skia::Transform {
    let [a, b, c, d, e, f] = affine.as_coeffs();
    tiny_skia::Transform::from_row(a as f32, b as f32, c as f32, d as f32, e as f32, f as f32)
}

/// Convert a kurbo `BezPath` to a tiny-skia `Path`. Returns `None` if the path
/// produces no segments (tiny-skia requires at least one segment).
fn bezpath_to_ts(path: &kurbo::BezPath) -> Option<tiny_skia::Path> {
    use kurbo::PathEl;
    let mut pb = tiny_skia::PathBuilder::new();
    for el in path.iter() {
        match el {
            PathEl::MoveTo(p) => pb.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => pb.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p1, p2) => {
                pb.quad_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32)
            }
            PathEl::CurveTo(p1, p2, p3) => pb.cubic_to(
                p1.x as f32, p1.y as f32,
                p2.x as f32, p2.y as f32,
                p3.x as f32, p3.y as f32,
            ),
            PathEl::ClosePath => pb.close(),
        }
    }
    pb.finish()
}

/// Build a tiny-skia `Paint` with a solid colour and optional opacity.
fn solid_paint(r: u8, g: u8, b: u8, a: u8, opacity: f32) -> tiny_skia::Paint<'static> {
    let alpha = ((a as f32 / 255.0) * opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(r, g, b, alpha);
    paint.anti_alias = true;
    paint
}

/// Build a tiny-skia `Paint` with a gradient shader.
fn gradient_paint<'a>(
    grad: &crate::gradient::ShapeGradient,
    start: kurbo::Point,
    end: kurbo::Point,
    opacity: f32,
) -> Option<tiny_skia::Paint<'a>> {
    use crate::gradient::GradientType;
    use tiny_skia::{Color, GradientStop, SpreadMode};

    let spread_mode = match grad.extend {
        crate::gradient::GradientExtend::Pad => SpreadMode::Pad,
        crate::gradient::GradientExtend::Reflect => SpreadMode::Reflect,
        crate::gradient::GradientExtend::Repeat => SpreadMode::Repeat,
    };

    let stops: Vec<GradientStop> = grad.stops.iter().map(|s| {
        let a = ((s.color.a as f32 / 255.0) * opacity * 255.0).round().clamp(0.0, 255.0) as u8;
        GradientStop::new(s.position, Color::from_rgba8(s.color.r, s.color.g, s.color.b, a))
    }).collect();

    let shader = match grad.kind {
        GradientType::Linear => {
            tiny_skia::LinearGradient::new(
                tiny_skia::Point { x: start.x as f32, y: start.y as f32 },
                tiny_skia::Point { x: end.x as f32, y: end.y as f32 },
                stops,
                spread_mode,
                tiny_skia::Transform::identity(),
            )?
        }
        GradientType::Radial => {
            let mid = kurbo::Point::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let radius = ((dx * dx + dy * dy).sqrt() * 0.5) as f32;
            tiny_skia::RadialGradient::new(
                tiny_skia::Point { x: mid.x as f32, y: mid.y as f32 },
                tiny_skia::Point { x: mid.x as f32, y: mid.y as f32 },
                radius,
                stops,
                spread_mode,
                tiny_skia::Transform::identity(),
            )?
        }
    };

    let mut paint = tiny_skia::Paint::default();
    paint.shader = shader;
    paint.anti_alias = true;
    Some(paint)
}

/// Render the document background to a CPU pixmap.
fn render_background_cpu(
    document: &Document,
    pixmap: &mut tiny_skia::PixmapMut<'_>,
    base_transform: Affine,
    draw_checkerboard: bool,
) {
    let ts_transform = affine_to_ts(base_transform);
    let bg_rect = tiny_skia::Rect::from_xywh(
        0.0, 0.0, document.width as f32, document.height as f32,
    );
    let Some(bg_rect) = bg_rect else { return };

    let bg = &document.background_color;

    // Draw checkerboard behind transparent backgrounds
    if draw_checkerboard && bg.a < 255 {
        // Build a 32×32 checkerboard pixmap (16×16 px light/dark squares)
        // in document space — each square = 16 document units.
        if let Some(mut checker) = tiny_skia::Pixmap::new(32, 32) {
            let light = tiny_skia::Color::from_rgba8(204, 204, 204, 255);
            let dark  = tiny_skia::Color::from_rgba8(170, 170, 170, 255);
            for py in 0u32..32 {
                for px in 0u32..32 {
                    let is_light = ((px / 16) + (py / 16)) % 2 == 0;
                    let color = if is_light { light } else { dark };
                    checker.pixels_mut()[(py * 32 + px) as usize] =
                        tiny_skia::PremultipliedColorU8::from_rgba(
                            (color.red()   * 255.0) as u8,
                            (color.green() * 255.0) as u8,
                            (color.blue()  * 255.0) as u8,
                            (color.alpha() * 255.0) as u8,
                        ).unwrap();
                }
            }
            let pattern = tiny_skia::Pattern::new(
                checker.as_ref(),
                tiny_skia::SpreadMode::Repeat,
                tiny_skia::FilterQuality::Nearest,
                1.0,
                tiny_skia::Transform::identity(),
            );
            let mut paint = tiny_skia::Paint::default();
            paint.shader = pattern;
            pixmap.fill_rect(bg_rect, &paint, ts_transform, None);
        }
    }

    // Draw the background colour
    let alpha = bg.a;
    let paint = solid_paint(bg.r, bg.g, bg.b, alpha, 1.0);
    pixmap.fill_rect(bg_rect, &paint, ts_transform, None);
}

/// Render a VectorGraph to a CPU pixmap.
fn render_vector_graph_cpu(
    graph: &crate::vector_graph::VectorGraph,
    pixmap: &mut tiny_skia::PixmapMut<'_>,
    transform: tiny_skia::Transform,
    opacity: f32,
    document: &Document,
    image_cache: &mut ImageCache,
) {
    // 1. Fills
    for (i, fill) in graph.fills.iter().enumerate() {
        if fill.deleted {
            continue;
        }
        if fill.color.is_none() && fill.image_fill.is_none() && fill.gradient_fill.is_none() {
            continue;
        }

        let fill_id = crate::vector_graph::FillId(i as u32);
        let path = graph.fill_to_bezpath(fill_id);
        let Some(ts_path) = bezpath_to_ts(&path) else { continue };

        let fill_type = match fill.fill_rule {
            crate::shape::FillRule::NonZero => tiny_skia::FillRule::Winding,
            crate::shape::FillRule::EvenOdd => tiny_skia::FillRule::EvenOdd,
        };

        let mut filled = false;

        // Gradient fill (takes priority over solid)
        if let Some(ref grad) = fill.gradient_fill {
            let bbox: kurbo::Rect = vello::kurbo::Shape::bounding_box(&path);
            let (start, end) = match (grad.start_world, grad.end_world) {
                (Some((sx, sy)), Some((ex, ey))) => match grad.kind {
                    crate::gradient::GradientType::Linear => {
                        (kurbo::Point::new(sx, sy), kurbo::Point::new(ex, ey))
                    }
                    crate::gradient::GradientType::Radial => {
                        let opp = kurbo::Point::new(2.0 * sx - ex, 2.0 * sy - ey);
                        (opp, kurbo::Point::new(ex, ey))
                    }
                },
                _ => gradient_bbox_endpoints(grad.angle, bbox),
            };
            if let Some(paint) = gradient_paint(grad, start, end, opacity) {
                pixmap.fill_path(&ts_path, &paint, fill_type, transform, None);
                filled = true;
            }
        }

        // Image fill — decode to Pixmap and use as a Pattern shader
        if let Some(image_asset_id) = fill.image_fill {
            if let Some(asset) = document.get_image_asset(&image_asset_id) {
                if let Some(img_pixmap) = image_cache.get_or_decode_cpu(asset) {
                    // Map the image's native pixel space onto the fill's bounding box.
                    let bbox: kurbo::Rect = vello::kurbo::Shape::bounding_box(&path);
                    let iw = (asset.width.max(1)) as f32;
                    let ih = (asset.height.max(1)) as f32;
                    let sx = (bbox.width() as f32) / iw;
                    let sy = (bbox.height() as f32) / ih;
                    let pat_tf = tiny_skia::Transform::from_row(
                        sx, 0.0, 0.0, sy, bbox.x0 as f32, bbox.y0 as f32,
                    );
                    let pattern = tiny_skia::Pattern::new(
                        tiny_skia::Pixmap::as_ref(&img_pixmap),
                        tiny_skia::SpreadMode::Pad,
                        tiny_skia::FilterQuality::Bilinear,
                        opacity,
                        pat_tf,
                    );
                    let mut paint = tiny_skia::Paint::default();
                    paint.shader = pattern;
                    paint.anti_alias = true;
                    pixmap.fill_path(&ts_path, &paint, fill_type, transform, None);
                    filled = true;
                }
            }
        }

        // Solid colour fill
        if !filled {
            if let Some(fc) = &fill.color {
                let paint = solid_paint(fc.r, fc.g, fc.b, fc.a, opacity);
                pixmap.fill_path(&ts_path, &paint, fill_type, transform, None);
            }
        }
    }

    // 2. Edges (strokes)
    for edge in &graph.edges {
        if edge.deleted {
            continue;
        }
        if let (Some(stroke_color), Some(stroke_style)) = (&edge.stroke_color, &edge.stroke_style) {
            let mut path = kurbo::BezPath::new();
            path.move_to(edge.curve.p0);
            path.curve_to(edge.curve.p1, edge.curve.p2, edge.curve.p3);
            let Some(ts_path) = bezpath_to_ts(&path) else { continue };

            let paint = solid_paint(stroke_color.r, stroke_color.g, stroke_color.b, stroke_color.a, opacity);
            let stroke = tiny_skia::Stroke {
                width: stroke_style.width as f32,
                line_cap: match stroke_style.cap {
                    crate::shape::Cap::Butt   => tiny_skia::LineCap::Butt,
                    crate::shape::Cap::Round  => tiny_skia::LineCap::Round,
                    crate::shape::Cap::Square => tiny_skia::LineCap::Square,
                },
                line_join: match stroke_style.join {
                    crate::shape::Join::Miter => tiny_skia::LineJoin::Miter,
                    crate::shape::Join::Round => tiny_skia::LineJoin::Round,
                    crate::shape::Join::Bevel => tiny_skia::LineJoin::Bevel,
                },
                miter_limit: stroke_style.miter_limit as f32,
                ..Default::default()
            };
            pixmap.stroke_path(&ts_path, &paint, &stroke, transform, None);
        }
    }
}

/// Render a vector layer to a CPU pixmap.
fn render_vector_layer_cpu(
    document: &Document,
    time: f64,
    layer: &crate::layer::VectorLayer,
    pixmap: &mut tiny_skia::PixmapMut<'_>,
    base_transform: Affine,
    parent_opacity: f64,
    image_cache: &mut ImageCache,
) {
    let layer_opacity = parent_opacity * layer.layer.opacity;

    for clip_instance in &layer.clip_instances {
        let group_end_time = document.vector_clips.get(&clip_instance.clip_id)
            .filter(|vc| vc.is_group)
            .map(|_| {
                let frame_duration = 1.0 / document.framerate;
                layer.group_visibility_end(&clip_instance.id, clip_instance.timeline_start, frame_duration)
            });
        render_clip_instance_cpu(
            document, time, clip_instance, layer_opacity, pixmap, base_transform,
            &layer.layer.animation_data, image_cache, group_end_time,
        );
    }

    if let Some(graph) = layer.graph_at_time(time) {
        render_vector_graph_cpu(graph, pixmap, affine_to_ts(base_transform), layer_opacity as f32, document, image_cache);
    }
}

/// Render a clip instance (and its nested layers) to a CPU pixmap.
fn render_clip_instance_cpu(
    document: &Document,
    time: f64,
    clip_instance: &crate::clip::ClipInstance,
    parent_opacity: f64,
    pixmap: &mut tiny_skia::PixmapMut<'_>,
    base_transform: Affine,
    animation_data: &crate::animation::AnimationData,
    image_cache: &mut ImageCache,
    group_end_time: Option<f64>,
) {
    let Some(vector_clip) = document.vector_clips.get(&clip_instance.clip_id) else { return };

    let tempo_map = document.tempo_map();
    let clip_time = if vector_clip.is_group {
        let start_secs = tempo_map.transform(clip_instance.timeline_start);
        let end = group_end_time.unwrap_or(start_secs);
        if time < start_secs || time >= end { return; }
        0.0
    } else {
        let clip_dur = document.get_clip_duration(&vector_clip.id).unwrap_or(vector_clip.duration);
        let Some(t) = clip_instance.remap_time(time, clip_dur, tempo_map) else { return };
        t
    };

    let transform = &clip_instance.transform;
    let x = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::X }, time, transform.x);
    let y = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::Y }, time, transform.y);
    let rotation = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::Rotation }, time, transform.rotation);
    let scale_x = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::ScaleX }, time, transform.scale_x);
    let scale_y = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::ScaleY }, time, transform.scale_y);
    let skew_x = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::SkewX }, time, transform.skew_x);
    let skew_y = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::SkewY }, time, transform.skew_y);
    let opacity = animation_data.eval(&crate::animation::AnimationTarget::Object { id: clip_instance.id, property: TransformProperty::Opacity }, time, clip_instance.opacity);

    let center_x = vector_clip.width / 2.0;
    let center_y = vector_clip.height / 2.0;
    let skew_transform = if skew_x != 0.0 || skew_y != 0.0 {
        let sx = if skew_x != 0.0 { Affine::new([1.0, 0.0, skew_x.to_radians().tan(), 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
        let sy = if skew_y != 0.0 { Affine::new([1.0, skew_y.to_radians().tan(), 0.0, 1.0, 0.0, 0.0]) } else { Affine::IDENTITY };
        Affine::translate((center_x, center_y)) * sx * sy * Affine::translate((-center_x, -center_y))
    } else { Affine::IDENTITY };

    let clip_transform = Affine::translate((x, y)) * Affine::rotate(rotation.to_radians()) * Affine::scale_non_uniform(scale_x, scale_y) * skew_transform;
    let instance_transform = base_transform * clip_transform;
    let clip_opacity = parent_opacity * opacity;

    for layer_node in vector_clip.layers.iter() {
        if !layer_node.data.visible() { continue; }
        render_vector_content_cpu(document, clip_time, &layer_node.data, pixmap, instance_transform, clip_opacity, image_cache);
    }
}

/// Render only vector/group content from a layer to a CPU pixmap.
/// Video, Audio, Effect, and Raster variants are intentionally skipped —
/// they are handled by the compositor via other paths.
fn render_vector_content_cpu(
    document: &Document,
    time: f64,
    layer: &AnyLayer,
    pixmap: &mut tiny_skia::PixmapMut<'_>,
    base_transform: Affine,
    parent_opacity: f64,
    image_cache: &mut ImageCache,
) {
    match layer {
        AnyLayer::Vector(vector_layer) => {
            render_vector_layer_cpu(document, time, vector_layer, pixmap, base_transform, parent_opacity, image_cache);
        }
        AnyLayer::Group(group_layer) => {
            for child in &group_layer.children {
                render_vector_content_cpu(document, time, child, pixmap, base_transform, parent_opacity, image_cache);
            }
        }
        AnyLayer::Audio(_) | AnyLayer::Video(_) | AnyLayer::Effect(_) | AnyLayer::Raster(_) => {}
    }
}

/// Render a single layer to its own isolated CPU pixmap.
fn render_layer_isolated_cpu(
    document: &Document,
    time: f64,
    layer: &AnyLayer,
    base_transform: Affine,
    width: u32,
    height: u32,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
) -> RenderedLayer {
    // Reuse the GPU path for non-vector layer types (they don't use the Vello scene anyway)
    let mut rendered = render_layer_isolated(document, time, layer, base_transform, image_cache, video_manager, camera_frame);

    // For vector layers, replace the empty scene with a CPU pixmap
    if matches!(rendered.layer_type, RenderedLayerType::Vector) {
        let opacity = layer.opacity() as f64;
        if let Some(mut pixmap) = tiny_skia::Pixmap::new(width.max(1), height.max(1)) {
            {
                let mut pm = pixmap.as_mut();
                render_vector_content_cpu(document, time, layer, &mut pm, base_transform, opacity, image_cache);
            }
            rendered.has_content = true;
            rendered.cpu_pixmap = Some(pixmap);
        }
    }

    rendered
}

/// Render a document for compositing using the CPU (tiny-skia) path.
///
/// Produces the same `CompositeRenderResult` shape as `render_document_for_compositing`,
/// but vector layers are rendered to `Pixmap`s instead of Vello `Scene`s.
/// `viewport_width` / `viewport_height` set the pixmap dimensions (should match
/// the wgpu render buffer size).
pub fn render_document_for_compositing_cpu(
    document: &Document,
    base_transform: Affine,
    viewport_width: u32,
    viewport_height: u32,
    image_cache: &mut ImageCache,
    video_manager: &std::sync::Arc<std::sync::Mutex<crate::video::VideoManager>>,
    camera_frame: Option<&crate::webcam::CaptureFrame>,
    floating_selection: Option<&crate::selection::RasterFloatingSelection>,
    draw_checkerboard: bool,
) -> CompositeRenderResult {
    let time = document.current_time;
    let w = viewport_width.max(1);
    let h = viewport_height.max(1);

    // Render background
    let background_cpu = tiny_skia::Pixmap::new(w, h).map(|mut pixmap| {
        render_background_cpu(document, &mut pixmap.as_mut(), base_transform, draw_checkerboard);
        pixmap
    });

    // Solo check
    let any_soloed = document.visible_layers().any(|layer| layer.soloed());

    let layers_to_render: Vec<_> = document
        .visible_layers()
        .filter(|layer| if any_soloed { layer.soloed() } else { true })
        .collect();

    let mut rendered_layers = Vec::with_capacity(layers_to_render.len());
    for layer in layers_to_render {
        let rendered = render_layer_isolated_cpu(
            document, time, layer, base_transform, w, h,
            image_cache, video_manager, camera_frame,
        );
        rendered_layers.push(rendered);
    }

    // Insert floating raster selection at the correct z-position (same logic as GPU path)
    if let Some(float_sel) = floating_selection {
        if let Some(pos) = rendered_layers.iter().position(|l| l.layer_id == float_sel.layer_id) {
            let parent_transform = match &rendered_layers[pos].layer_type {
                RenderedLayerType::Raster { transform, .. } => *transform,
                _ => Affine::IDENTITY,
            };
            let float_entry = RenderedLayer {
                layer_id: Uuid::nil(),
                scene: Scene::new(),
                cpu_pixmap: None,
                opacity: 1.0,
                blend_mode: crate::gpu::BlendMode::Normal,
                has_content: !float_sel.pixels.is_empty(),
                layer_type: RenderedLayerType::Float {
                    canvas_id: float_sel.canvas_id,
                    x: float_sel.x,
                    y: float_sel.y,
                    width: float_sel.width,
                    height: float_sel.height,
                    transform: parent_transform,
                    pixels: std::sync::Arc::clone(&float_sel.pixels),
                },
            };
            rendered_layers.insert(pos + 1, float_entry);
        }
    }

    CompositeRenderResult {
        background: Scene::new(),
        background_cpu,
        layers: rendered_layers,
        width: document.width,
        height: document.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::layer::{AnyLayer, LayerTrait, VectorLayer};
    use crate::shape::{Shape, ShapeColor};
    use vello::kurbo::{Circle, Shape as KurboShape};

    // Note: render_document tests require video_manager and are omitted here.
    // The solo/visibility logic is tested via helpers.

    /// Helper to check if any layer is soloed in document
    fn has_soloed_layer(doc: &Document) -> bool {
        doc.visible_layers().any(|layer| layer.soloed())
    }

    /// Helper to count visible layers for rendering (respecting solo)
    fn count_layers_to_render(doc: &Document) -> usize {
        let any_soloed = has_soloed_layer(doc);
        doc.visible_layers()
            .filter(|layer| {
                if any_soloed {
                    layer.soloed()
                } else {
                    true
                }
            })
            .count()
    }

    #[test]
    fn test_no_solo_all_layers_render() {
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        assert_eq!(has_soloed_layer(&doc), false);
        assert_eq!(count_layers_to_render(&doc), 2);
    }

    #[test]
    fn test_one_layer_soloed() {
        let mut doc = Document::new("Test");

        let mut layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");

        layer1.layer.soloed = true;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        assert_eq!(has_soloed_layer(&doc), true);
        assert_eq!(count_layers_to_render(&doc), 1);
    }

    #[test]
    fn test_hidden_layer_not_rendered() {
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");
        layer2.layer.visible = false;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        assert_eq!(doc.visible_layers().count(), 1);
    }

    #[test]
    fn test_unsolo_returns_to_normal() {
        let mut doc = Document::new("Test");

        let mut layer1 = VectorLayer::new("Layer 1");

        layer1.layer.soloed = true;

        let id1 = doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(VectorLayer::new("Layer 2")));

        assert_eq!(count_layers_to_render(&doc), 1);

        if let Some(layer) = doc.root.get_child_mut(&id1) {
            layer.set_soloed(false);
        }

        assert_eq!(has_soloed_layer(&doc), false);
        assert_eq!(count_layers_to_render(&doc), 2);
    }
}
