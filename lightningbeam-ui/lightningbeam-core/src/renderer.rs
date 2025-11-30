//! Rendering system for Lightningbeam documents
//!
//! Renders documents to Vello scenes for GPU-accelerated display.

use crate::animation::TransformProperty;
use crate::clip::ImageAsset;
use crate::document::Document;
use crate::layer::{AnyLayer, LayerTrait, VectorLayer};
use crate::object::ShapeInstance;
use kurbo::{Affine, Shape};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;
use vello::kurbo::Rect;
use vello::peniko::{Blob, Fill, Image, ImageFormat};
use vello::Scene;

/// Cache for decoded image data to avoid re-decoding every frame
pub struct ImageCache {
    cache: HashMap<Uuid, Arc<Image>>,
}

impl ImageCache {
    /// Create a new empty image cache
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Get or decode an image, caching the result
    pub fn get_or_decode(&mut self, asset: &ImageAsset) -> Option<Arc<Image>> {
        if let Some(cached) = self.cache.get(&asset.id) {
            return Some(Arc::clone(cached));
        }

        // Decode and cache
        let image = decode_image_asset(asset)?;
        let arc_image = Arc::new(image);
        self.cache.insert(asset.id, Arc::clone(&arc_image));
        Some(arc_image)
    }

    /// Clear cache entry when an image asset is deleted or modified
    pub fn invalidate(&mut self, id: &Uuid) {
        self.cache.remove(id);
    }

    /// Clear all cached images
    pub fn clear(&mut self) {
        self.cache.clear();
    }
}

impl Default for ImageCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode an image asset to peniko Image
fn decode_image_asset(asset: &ImageAsset) -> Option<Image> {
    // Get the raw file data
    let data = asset.data.as_ref()?;

    // Decode using the image crate
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();

    // Create peniko Image
    Some(Image::new(
        Blob::from(rgba.into_raw()),
        ImageFormat::Rgba8,
        asset.width,
        asset.height,
    ))
}

/// Render a document to a Vello scene
pub fn render_document(document: &Document, scene: &mut Scene, image_cache: &mut ImageCache) {
    render_document_with_transform(document, scene, Affine::IDENTITY, image_cache);
}

/// Render a document to a Vello scene with a base transform
/// The base transform is composed with all object transforms (useful for camera zoom/pan)
pub fn render_document_with_transform(document: &Document, scene: &mut Scene, base_transform: Affine, image_cache: &mut ImageCache) {
    // 1. Draw background
    render_background(document, scene, base_transform);

    // 2. Recursively render the root graphics object at current time
    let time = document.current_time;
    render_graphics_object(document, time, scene, base_transform, image_cache);
}

/// Draw the document background
fn render_background(document: &Document, scene: &mut Scene, base_transform: Affine) {
    let background_rect = Rect::new(0.0, 0.0, document.width, document.height);

    // Convert our ShapeColor to vello's peniko Color
    let background_color = document.background_color.to_peniko();

    scene.fill(
        Fill::NonZero,
        base_transform,
        background_color,
        None,
        &background_rect,
    );
}

/// Recursively render the root graphics object and its children
fn render_graphics_object(document: &Document, time: f64, scene: &mut Scene, base_transform: Affine, image_cache: &mut ImageCache) {
    // Check if any layers are soloed
    let any_soloed = document.visible_layers().any(|layer| layer.soloed());

    // Render layers based on solo state
    // If any layer is soloed, only render soloed layers
    // Otherwise, render all visible layers
    // Start with full opacity (1.0)
    for layer in document.visible_layers() {
        if any_soloed {
            // Only render soloed layers when solo is active
            if layer.soloed() {
                render_layer(document, time, layer, scene, base_transform, 1.0, image_cache);
            }
        } else {
            // Render all visible layers when no solo is active
            render_layer(document, time, layer, scene, base_transform, 1.0, image_cache);
        }
    }
}

/// Render a single layer
fn render_layer(document: &Document, time: f64, layer: &AnyLayer, scene: &mut Scene, base_transform: Affine, parent_opacity: f64, image_cache: &mut ImageCache) {
    match layer {
        AnyLayer::Vector(vector_layer) => render_vector_layer(document, time, vector_layer, scene, base_transform, parent_opacity, image_cache),
        AnyLayer::Audio(_) => {
            // Audio layers don't render visually
        }
        AnyLayer::Video(_) => {
            // Video rendering not yet implemented
        }
    }
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
) {
    // Try to find the clip in the document's clip libraries
    // For now, only handle VectorClips (VideoClip and AudioClip rendering not yet implemented)
    let Some(vector_clip) = document.vector_clips.get(&clip_instance.clip_id) else {
        return; // Clip not found or not a vector clip
    };

    // Remap timeline time to clip's internal time
    let Some(clip_time) = clip_instance.remap_time(time, vector_clip.duration) else {
        return; // Clip instance not active at this time
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
        render_layer(document, clip_time, &layer_node.data, scene, instance_transform, clip_opacity, image_cache);
    }
}

/// Render a vector layer with all its clip instances and shape instances
fn render_vector_layer(document: &Document, time: f64, layer: &VectorLayer, scene: &mut Scene, base_transform: Affine, parent_opacity: f64, image_cache: &mut ImageCache) {

    // Cascade opacity: parent_opacity × layer.opacity
    let layer_opacity = parent_opacity * layer.layer.opacity;

    // Render clip instances first (they appear under shape instances)
    for clip_instance in &layer.clip_instances {
        render_clip_instance(document, time, clip_instance, layer_opacity, scene, base_transform, &layer.layer.animation_data, image_cache);
    }

    // Render each shape instance in the layer
    for shape_instance in &layer.shape_instances {
        // Get the shape for this instance
        let Some(shape) = layer.get_shape(&shape_instance.shape_id) else {
            continue;
        };

        // Evaluate animated properties
        let transform = &shape_instance.transform;
        let x = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::X,
                },
                time,
                transform.x,
            );
        let y = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::Y,
                },
                time,
                transform.y,
            );
        let rotation = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::Rotation,
                },
                time,
                transform.rotation,
            );
        let scale_x = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::ScaleX,
                },
                time,
                transform.scale_x,
            );
        let scale_y = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::ScaleY,
                },
                time,
                transform.scale_y,
            );
        let skew_x = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::SkewX,
                },
                time,
                transform.skew_x,
            );
        let skew_y = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::SkewY,
                },
                time,
                transform.skew_y,
            );
        let opacity = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: shape_instance.id,
                    property: TransformProperty::Opacity,
                },
                time,
                shape_instance.opacity,
            );

        // Check if shape has morphing animation
        let shape_index = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Shape {
                    id: shape.id,
                    property: crate::animation::ShapeProperty::ShapeIndex,
                },
                time,
                0.0,
            );

        // Get the morphed path
        let path = shape.get_morphed_path(shape_index);

        // Build transform matrix (compose with base transform for camera)
        // Get shape center for skewing around center
        let shape_bbox = shape.path().bounding_box();
        let center_x = (shape_bbox.x0 + shape_bbox.x1) / 2.0;
        let center_y = (shape_bbox.y0 + shape_bbox.y1) / 2.0;

        // Build skew transforms (applied around shape center)
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

        let object_transform = Affine::translate((x, y))
            * Affine::rotate(rotation.to_radians())
            * Affine::scale_non_uniform(scale_x, scale_y)
            * skew_transform;
        let affine = base_transform * object_transform;

        // Calculate final opacity (cascaded from parent → layer → shape instance)
        // layer_opacity already includes parent_opacity from render_vector_layer
        let final_opacity = (layer_opacity * opacity) as f32;

        // Determine fill rule
        let fill_rule = match shape.fill_rule {
            crate::shape::FillRule::NonZero => Fill::NonZero,
            crate::shape::FillRule::EvenOdd => Fill::EvenOdd,
        };

        // Render fill - prefer image fill over color fill
        let mut filled = false;

        // Check for image fill first
        if let Some(image_asset_id) = shape.image_fill {
            if let Some(image_asset) = document.get_image_asset(&image_asset_id) {
                if let Some(image) = image_cache.get_or_decode(image_asset) {
                    // Apply opacity to image (clone is cheap - Image uses Arc<Blob> internally)
                    let image_with_alpha = (*image).clone().with_alpha(final_opacity);

                    // The image is rendered as a fill for the shape path
                    // Since the shape path is a rectangle matching the image dimensions,
                    // the image should fill the shape perfectly
                    scene.fill(fill_rule, affine, &image_with_alpha, None, &path);
                    filled = true;
                }
            }
        }

        // Fall back to color fill if no image fill (or image failed to load)
        if !filled {
            if let Some(fill_color) = &shape.fill_color {
                // Apply opacity to color
                let alpha = ((fill_color.a as f32 / 255.0) * final_opacity * 255.0) as u8;
                let adjusted_color = crate::shape::ShapeColor::rgba(
                    fill_color.r,
                    fill_color.g,
                    fill_color.b,
                    alpha,
                );

                scene.fill(
                    fill_rule,
                    affine,
                    adjusted_color.to_peniko(),
                    None,
                    &path,
                );
            }
        }

        // Render stroke if present
        if let (Some(stroke_color), Some(stroke_style)) = (&shape.stroke_color, &shape.stroke_style)
        {
            // Apply opacity to color
            let alpha = ((stroke_color.a as f32 / 255.0) * final_opacity * 255.0) as u8;
            let adjusted_color = crate::shape::ShapeColor::rgba(
                stroke_color.r,
                stroke_color.g,
                stroke_color.b,
                alpha,
            );

            scene.stroke(
                &stroke_style.to_stroke(),
                affine,
                adjusted_color.to_peniko(),
                None,
                &path,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::layer::{AnyLayer, LayerTrait, VectorLayer};
    use crate::object::ShapeInstance;
    use crate::shape::{Shape, ShapeColor};
    use kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_render_empty_document() {
        let doc = Document::new("Test");
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();

        render_document(&doc, &mut scene, &mut image_cache);
        // Should render background without errors
    }

    #[test]
    fn test_render_document_with_shape() {
        let mut doc = Document::new("Test");

        // Create a simple circle shape
        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));

        // Create a shape instance for the shape
        let shape_instance = ShapeInstance::new(shape.id);

        // Create a vector layer
        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        vector_layer.add_object(shape_instance);

        // Add to document
        doc.root.add_child(AnyLayer::Vector(vector_layer));

        // Render
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
        // Should render without errors
    }

    // === Solo Rendering Tests ===

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

        // Add two visible layers, neither soloed
        let layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Both should be rendered
        assert_eq!(has_soloed_layer(&doc), false);
        assert_eq!(count_layers_to_render(&doc), 2);

        // Render should work without errors
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_one_layer_soloed() {
        let mut doc = Document::new("Test");

        // Add two layers
        let mut layer1 = VectorLayer::new("Layer 1");
        let layer2 = VectorLayer::new("Layer 2");

        // Solo layer 1
        layer1.layer.soloed = true;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Only soloed layer should be rendered
        assert_eq!(has_soloed_layer(&doc), true);
        assert_eq!(count_layers_to_render(&doc), 1);

        // Verify the soloed layer is the one that would render
        let any_soloed = has_soloed_layer(&doc);
        let soloed_count: usize = doc.visible_layers()
            .filter(|l| any_soloed && l.soloed())
            .count();
        assert_eq!(soloed_count, 1);

        // Render should work
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_multiple_layers_soloed() {
        let mut doc = Document::new("Test");

        // Add three layers
        let mut layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");
        let layer3 = VectorLayer::new("Layer 3");

        // Solo layers 1 and 2
        layer1.layer.soloed = true;
        layer2.layer.soloed = true;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));
        doc.root.add_child(AnyLayer::Vector(layer3));

        // Only soloed layers (1 and 2) should render
        assert_eq!(has_soloed_layer(&doc), true);
        assert_eq!(count_layers_to_render(&doc), 2);

        // Render
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_hidden_layer_not_rendered() {
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        // Hide layer 2
        layer2.layer.visible = false;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Only visible layer (1) should be considered
        assert_eq!(doc.visible_layers().count(), 1);

        // Render
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_hidden_but_soloed_layer() {
        // A hidden layer that is soloed shouldn't render
        // because visible_layers() filters out hidden layers first
        let mut doc = Document::new("Test");

        let layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        // Layer 2: soloed but hidden
        layer2.layer.soloed = true;
        layer2.layer.visible = false;

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // visible_layers only returns layer 1 (layer 2 is hidden)
        // Since layer 1 isn't soloed and no visible layers are soloed,
        // all visible layers render
        let any_soloed = has_soloed_layer(&doc);
        assert_eq!(any_soloed, false); // No *visible* layer is soloed

        // Both visible layers render (only 1 is visible)
        assert_eq!(count_layers_to_render(&doc), 1);

        // Render
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_solo_with_layer_opacity() {
        let mut doc = Document::new("Test");

        // Create layers with different opacities
        let mut layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        layer1.layer.opacity = 0.5;
        layer1.layer.soloed = true;

        layer2.layer.opacity = 0.8;

        // Add circle shapes for visible rendering
        let circle = Circle::new((50.0, 50.0), 20.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));
        let shape_instance = ShapeInstance::new(shape.id);
        layer1.add_shape(shape.clone());
        layer1.add_object(shape_instance);

        let shape2 = Shape::new(circle.to_path(0.1)).with_fill(ShapeColor::rgb(0, 255, 0));
        let shape_instance2 = ShapeInstance::new(shape2.id);
        layer2.add_shape(shape2);
        layer2.add_object(shape_instance2);

        doc.root.add_child(AnyLayer::Vector(layer1));
        doc.root.add_child(AnyLayer::Vector(layer2));

        // Only layer 1 (soloed with 0.5 opacity) should render
        assert_eq!(has_soloed_layer(&doc), true);
        assert_eq!(count_layers_to_render(&doc), 1);

        // Render
        let mut scene = Scene::new();
        let mut image_cache = ImageCache::new();
        render_document(&doc, &mut scene, &mut image_cache);
    }

    #[test]
    fn test_unsolo_returns_to_normal() {
        let mut doc = Document::new("Test");

        let mut layer1 = VectorLayer::new("Layer 1");
        let mut layer2 = VectorLayer::new("Layer 2");

        // First, solo layer 1
        layer1.layer.soloed = true;

        let id1 = doc.root.add_child(AnyLayer::Vector(layer1));
        let id2 = doc.root.add_child(AnyLayer::Vector(layer2));

        // Only 1 layer renders when soloed
        assert_eq!(count_layers_to_render(&doc), 1);

        // Now unsolo layer 1
        if let Some(layer) = doc.root.get_child_mut(&id1) {
            layer.set_soloed(false);
        }

        // Now both should render again
        assert_eq!(has_soloed_layer(&doc), false);
        assert_eq!(count_layers_to_render(&doc), 2);
    }
}
