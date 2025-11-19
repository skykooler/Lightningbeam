//! Rendering system for Lightningbeam documents
//!
//! Renders documents to Vello scenes for GPU-accelerated display.

use crate::animation::TransformProperty;
use crate::document::Document;
use crate::layer::{AnyLayer, VectorLayer};
use kurbo::{Affine, Shape};
use vello::kurbo::Rect;
use vello::peniko::Fill;
use vello::Scene;

/// Render a document to a Vello scene
pub fn render_document(document: &Document, scene: &mut Scene) {
    render_document_with_transform(document, scene, Affine::IDENTITY);
}

/// Render a document to a Vello scene with a base transform
/// The base transform is composed with all object transforms (useful for camera zoom/pan)
pub fn render_document_with_transform(document: &Document, scene: &mut Scene, base_transform: Affine) {
    // 1. Draw background
    render_background(document, scene, base_transform);

    // 2. Recursively render the root graphics object
    render_graphics_object(document, scene, base_transform);
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
fn render_graphics_object(document: &Document, scene: &mut Scene, base_transform: Affine) {
    // Render all visible layers in the root graphics object
    for layer in document.visible_layers() {
        render_layer(document, layer, scene, base_transform);
    }
}

/// Render a single layer
fn render_layer(document: &Document, layer: &AnyLayer, scene: &mut Scene, base_transform: Affine) {
    match layer {
        AnyLayer::Vector(vector_layer) => render_vector_layer(document, vector_layer, scene, base_transform),
        AnyLayer::Audio(_) => {
            // Audio layers don't render visually
        }
        AnyLayer::Video(_) => {
            // Video rendering not yet implemented
        }
    }
}

/// Render a vector layer with all its objects
fn render_vector_layer(document: &Document, layer: &VectorLayer, scene: &mut Scene, base_transform: Affine) {
    let time = document.current_time;

    // Get layer-level opacity
    let layer_opacity = layer.layer.opacity;

    // Render each object in the layer
    for object in &layer.objects {
        // Get the shape for this object
        let Some(shape) = layer.get_shape(&object.shape_id) else {
            continue;
        };

        // Evaluate animated properties
        let transform = &object.transform;
        let x = layer
            .layer
            .animation_data
            .eval(
                &crate::animation::AnimationTarget::Object {
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
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
                    id: object.id,
                    property: TransformProperty::Opacity,
                },
                time,
                transform.opacity,
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

        // Calculate final opacity (layer * object)
        let final_opacity = (layer_opacity * opacity) as f32;

        // Render fill if present
        if let Some(fill_color) = &shape.fill_color {
            // Apply opacity to color
            let alpha = ((fill_color.a as f32 / 255.0) * final_opacity * 255.0) as u8;
            let adjusted_color = crate::shape::ShapeColor::rgba(
                fill_color.r,
                fill_color.g,
                fill_color.b,
                alpha,
            );

            let fill_rule = match shape.fill_rule {
                crate::shape::FillRule::NonZero => Fill::NonZero,
                crate::shape::FillRule::EvenOdd => Fill::EvenOdd,
            };

            scene.fill(
                fill_rule,
                affine,
                adjusted_color.to_peniko(),
                None,
                &path,
            );
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
    use crate::layer::{AnyLayer, VectorLayer};
    use crate::object::Object;
    use crate::shape::{Shape, ShapeColor};
    use kurbo::{Circle, Shape as KurboShape};

    #[test]
    fn test_render_empty_document() {
        let doc = Document::new("Test");
        let mut scene = Scene::new();

        render_document(&doc, &mut scene);
        // Should render background without errors
    }

    #[test]
    fn test_render_document_with_shape() {
        let mut doc = Document::new("Test");

        // Create a simple circle shape
        let circle = Circle::new((100.0, 100.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(255, 0, 0));

        // Create an object for the shape
        let object = Object::new(shape.id);

        // Create a vector layer
        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        vector_layer.add_object(object);

        // Add to document
        doc.root.add_child(AnyLayer::Vector(vector_layer));

        // Render
        let mut scene = Scene::new();
        render_document(&doc, &mut scene);
        // Should render without errors
    }
}
