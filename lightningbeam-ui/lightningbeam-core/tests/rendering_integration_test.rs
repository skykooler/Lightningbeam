//! Integration tests for rendering scenarios
//!
//! Tests complex rendering scenarios including solo, mute, opacity cascading,
//! and clip instance rendering.

use lightningbeam_core::clip::{ClipInstance, VectorClip};
use lightningbeam_core::document::Document;
use lightningbeam_core::layer::{AnyLayer, LayerTrait, VectorLayer};
use lightningbeam_core::object::ShapeInstance;
use lightningbeam_core::renderer::{render_document, render_document_with_transform};
use lightningbeam_core::shape::{Shape, ShapeColor};
use vello::kurbo::{Affine, Circle, Shape as KurboShape};
use vello::Scene;

/// Create a test document with multiple layers containing shapes
fn setup_rendering_document() -> (Document, Vec<uuid::Uuid>) {
    let mut document = Document::new("Test Project");
    document.width = 800.0;
    document.height = 600.0;

    // Layer 1 with a red circle
    let mut layer1 = VectorLayer::new("Red Layer");
    let circle1 = Circle::new((100.0, 100.0), 50.0);
    let shape1 = Shape::new(circle1.to_path(0.1)).with_fill(ShapeColor::rgb(255, 0, 0));
    let instance1 = ShapeInstance::new(shape1.id);
    layer1.add_shape(shape1);
    layer1.add_object(instance1);

    // Layer 2 with a green circle
    let mut layer2 = VectorLayer::new("Green Layer");
    let circle2 = Circle::new((200.0, 200.0), 50.0);
    let shape2 = Shape::new(circle2.to_path(0.1)).with_fill(ShapeColor::rgb(0, 255, 0));
    let instance2 = ShapeInstance::new(shape2.id);
    layer2.add_shape(shape2);
    layer2.add_object(instance2);

    // Layer 3 with a blue circle
    let mut layer3 = VectorLayer::new("Blue Layer");
    let circle3 = Circle::new((300.0, 300.0), 50.0);
    let shape3 = Shape::new(circle3.to_path(0.1)).with_fill(ShapeColor::rgb(0, 0, 255));
    let instance3 = ShapeInstance::new(shape3.id);
    layer3.add_shape(shape3);
    layer3.add_object(instance3);

    let id1 = document.root.add_child(AnyLayer::Vector(layer1));
    let id2 = document.root.add_child(AnyLayer::Vector(layer2));
    let id3 = document.root.add_child(AnyLayer::Vector(layer3));

    (document, vec![id1, id2, id3])
}

#[test]
fn test_render_empty_document() {
    let document = Document::new("Empty");
    let mut scene = Scene::new();

    // Should not panic
    render_document(&document, &mut scene);
}

#[test]
fn test_render_document_with_shapes() {
    let (document, _ids) = setup_rendering_document();
    let mut scene = Scene::new();

    // Should render all 3 layers without error
    render_document(&document, &mut scene);
}

#[test]
fn test_render_with_transform() {
    let (document, _ids) = setup_rendering_document();
    let mut scene = Scene::new();

    // Render with zoom and pan
    let transform = Affine::translate((100.0, 50.0)) * Affine::scale(2.0);
    render_document_with_transform(&document, &mut scene, transform);
}

#[test]
fn test_render_solo_single_layer() {
    let (mut document, ids) = setup_rendering_document();

    // Solo layer 2 (green)
    if let Some(layer) = document.root.get_child_mut(&ids[1]) {
        layer.set_soloed(true);
    }

    // Count visible layers for rendering
    let any_soloed = document.visible_layers().any(|l| l.soloed());
    assert!(any_soloed);

    let layers_to_render: Vec<_> = document
        .visible_layers()
        .filter(|l| l.soloed())
        .collect();
    assert_eq!(layers_to_render.len(), 1);

    // Render should work
    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_solo_multiple_layers() {
    let (mut document, ids) = setup_rendering_document();

    // Solo layers 1 and 3
    if let Some(layer) = document.root.get_child_mut(&ids[0]) {
        layer.set_soloed(true);
    }
    if let Some(layer) = document.root.get_child_mut(&ids[2]) {
        layer.set_soloed(true);
    }

    // Two layers should render
    let layers_to_render: Vec<_> = document
        .visible_layers()
        .filter(|l| l.soloed())
        .collect();
    assert_eq!(layers_to_render.len(), 2);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_hidden_layer_not_rendered() {
    let (mut document, ids) = setup_rendering_document();

    // Hide layer 2
    if let Some(layer) = document.root.get_child_mut(&ids[1]) {
        layer.set_visible(false);
    }

    // Only 2 visible layers
    assert_eq!(document.visible_layers().count(), 2);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_with_layer_opacity() {
    let (mut document, ids) = setup_rendering_document();

    // Set different opacities
    if let Some(layer) = document.root.get_child_mut(&ids[0]) {
        layer.set_opacity(0.5);
    }
    if let Some(layer) = document.root.get_child_mut(&ids[1]) {
        layer.set_opacity(0.25);
    }
    if let Some(layer) = document.root.get_child_mut(&ids[2]) {
        layer.set_opacity(1.0);
    }

    // Verify opacities
    assert_eq!(document.root.get_child(&ids[0]).unwrap().opacity(), 0.5);
    assert_eq!(document.root.get_child(&ids[1]).unwrap().opacity(), 0.25);
    assert_eq!(document.root.get_child(&ids[2]).unwrap().opacity(), 1.0);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_with_clip_instances() {
    let mut document = Document::new("Test");

    // Create a vector clip
    let mut clip_layer = VectorLayer::new("Clip Content");
    let circle = Circle::new((50.0, 50.0), 25.0);
    let shape = Shape::new(circle.to_path(0.1)).with_fill(ShapeColor::rgb(255, 255, 0));
    let instance = ShapeInstance::new(shape.id);
    clip_layer.add_shape(shape);
    clip_layer.add_object(instance);

    let mut vector_clip = VectorClip::new("Yellow Circle Clip", 5.0, 100.0, 100.0);
    vector_clip.layers.roots.push(lightningbeam_core::layer_tree::LayerNode::new(
        AnyLayer::Vector(clip_layer),
    ));

    let clip_id = vector_clip.id;
    document.vector_clips.insert(clip_id, vector_clip);

    // Create a layer with a clip instance
    let mut layer = VectorLayer::new("Main Layer");
    let mut clip_instance = ClipInstance::new(clip_id);
    clip_instance.timeline_start = 0.0;
    clip_instance.transform.x = 100.0;
    clip_instance.transform.y = 100.0;
    layer.clip_instances.push(clip_instance);

    document.root.add_child(AnyLayer::Vector(layer));

    // Set time within clip range
    document.set_time(2.0);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_clip_instance_outside_time_range() {
    let mut document = Document::new("Test");

    // Create a vector clip
    let vector_clip = VectorClip::new("Test Clip", 5.0, 100.0, 100.0);
    let clip_id = vector_clip.id;
    document.vector_clips.insert(clip_id, vector_clip);

    // Create clip instance starting at time 10.0
    let mut layer = VectorLayer::new("Main Layer");
    let mut clip_instance = ClipInstance::new(clip_id);
    clip_instance.timeline_start = 10.0;
    layer.clip_instances.push(clip_instance);

    document.root.add_child(AnyLayer::Vector(layer));

    // Set time before clip starts
    document.set_time(5.0);

    // Clip shouldn't render (it hasn't started yet)
    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_all_layers_hidden() {
    let (mut document, ids) = setup_rendering_document();

    // Hide all layers
    for id in &ids {
        if let Some(layer) = document.root.get_child_mut(id) {
            layer.set_visible(false);
        }
    }

    // No visible layers
    assert_eq!(document.visible_layers().count(), 0);

    // Should still render (just background)
    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_solo_hidden_layer_interaction() {
    let (mut document, ids) = setup_rendering_document();

    // Hide and solo layer 1
    if let Some(layer) = document.root.get_child_mut(&ids[0]) {
        layer.set_visible(false);
        layer.set_soloed(true);
    }

    // Layer 1 is hidden, so not in visible_layers()
    // The solo flag on a hidden layer doesn't affect rendering
    let visible_soloed: Vec<_> = document
        .visible_layers()
        .filter(|l| l.soloed())
        .collect();

    // No visible layer is soloed
    assert_eq!(visible_soloed.len(), 0);

    // All 2 visible layers should render (layers 2 and 3)
    assert_eq!(document.visible_layers().count(), 2);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_background_color() {
    let mut document = Document::new("Test");
    document.background_color = ShapeColor::rgb(128, 128, 128);

    let mut scene = Scene::new();
    render_document(&document, &mut scene);
}

#[test]
fn test_render_at_different_times() {
    let (mut document, _ids) = setup_rendering_document();

    // Render at different times
    for time in [0.0, 0.5, 1.0, 2.5, 5.0, 10.0] {
        document.set_time(time);
        let mut scene = Scene::new();
        render_document(&document, &mut scene);
    }
}
