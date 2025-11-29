//! Integration tests for selection operations
//!
//! Tests mixed selections of shape instances and clip instances,
//! selection state management, and interaction with transforms.

use lightningbeam_core::action::Action;
use lightningbeam_core::actions::TransformClipInstancesAction;
use lightningbeam_core::clip::ClipInstance;
use lightningbeam_core::document::Document;
use lightningbeam_core::layer::{AnyLayer, VectorLayer};
use lightningbeam_core::object::{ShapeInstance, Transform};
use lightningbeam_core::selection::Selection;
use lightningbeam_core::shape::Shape;
use std::collections::HashMap;
use uuid::Uuid;
use vello::kurbo::{Circle, Rect, Shape as KurboShape};

/// Create a test document with shapes and clips
fn setup_mixed_content_document() -> (Document, Uuid, Vec<Uuid>, Vec<Uuid>) {
    let mut document = Document::new("Test Project");

    let mut layer = VectorLayer::new("Layer 1");

    // Add shapes
    let circle = Circle::new((50.0, 50.0), 25.0);
    let shape1 = Shape::new(circle.to_path(0.1));
    let _shape1_id = shape1.id;
    let instance1 = ShapeInstance::new(shape1.id);
    let instance1_id = instance1.id;
    layer.add_shape(shape1);
    layer.add_object(instance1);

    let rect = Rect::new(100.0, 100.0, 150.0, 150.0);
    let shape2 = Shape::new(rect.to_path(0.1));
    let instance2 = ShapeInstance::new(shape2.id);
    let instance2_id = instance2.id;
    layer.add_shape(shape2);
    layer.add_object(instance2);

    // Add clip instances
    let clip_id = Uuid::new_v4();
    let clip_instance1 = ClipInstance::new(clip_id);
    let clip_instance1_id = clip_instance1.id;
    layer.clip_instances.push(clip_instance1);

    let clip_instance2 = ClipInstance::new(clip_id);
    let clip_instance2_id = clip_instance2.id;
    layer.clip_instances.push(clip_instance2);

    let layer_id = document.root.add_child(AnyLayer::Vector(layer));

    let shape_ids = vec![instance1_id, instance2_id];
    let clip_ids = vec![clip_instance1_id, clip_instance2_id];

    (document, layer_id, shape_ids, clip_ids)
}

#[test]
fn test_selection_of_shape_instances() {
    let (_document, _layer_id, shape_ids, _clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();

    // Select first shape instance
    selection.add_shape_instance(shape_ids[0]);
    assert!(selection.contains_shape_instance(&shape_ids[0]));
    assert!(!selection.contains_shape_instance(&shape_ids[1]));
    assert_eq!(selection.shape_instances().len(), 1);

    // Add second shape instance
    selection.add_shape_instance(shape_ids[1]);
    assert!(selection.contains_shape_instance(&shape_ids[0]));
    assert!(selection.contains_shape_instance(&shape_ids[1]));
    assert_eq!(selection.shape_instances().len(), 2);

    // Toggle first shape instance (deselect)
    selection.toggle_shape_instance(shape_ids[0]);
    assert!(!selection.contains_shape_instance(&shape_ids[0]));
    assert!(selection.contains_shape_instance(&shape_ids[1]));
    assert_eq!(selection.shape_instances().len(), 1);
}

#[test]
fn test_selection_of_clip_instances() {
    let (_document, _layer_id, _shape_ids, clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();

    // Select clip instances
    selection.add_clip_instance(clip_ids[0]);
    assert!(selection.contains_clip_instance(&clip_ids[0]));
    assert_eq!(selection.clip_instances().len(), 1);

    selection.add_clip_instance(clip_ids[1]);
    assert!(selection.contains_clip_instance(&clip_ids[0]));
    assert!(selection.contains_clip_instance(&clip_ids[1]));
    assert_eq!(selection.clip_instances().len(), 2);

    // Toggle
    selection.toggle_clip_instance(clip_ids[0]);
    assert!(!selection.contains_clip_instance(&clip_ids[0]));
    assert!(selection.contains_clip_instance(&clip_ids[1]));
}

#[test]
fn test_mixed_selection() {
    let (_document, _layer_id, shape_ids, clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();

    // Select both shapes and clips
    selection.add_shape_instance(shape_ids[0]);
    selection.add_shape_instance(shape_ids[1]);
    selection.add_clip_instance(clip_ids[0]);
    selection.add_clip_instance(clip_ids[1]);

    assert_eq!(selection.shape_instances().len(), 2);
    assert_eq!(selection.clip_instances().len(), 2);

    // Clear only clip instances
    selection.clear_clip_instances();

    assert_eq!(selection.shape_instances().len(), 2);
    assert_eq!(selection.clip_instances().len(), 0);

    // Re-add clip
    selection.add_clip_instance(clip_ids[0]);

    // Full clear
    selection.clear();

    assert_eq!(selection.shape_instances().len(), 0);
    assert_eq!(selection.clip_instances().len(), 0);
}

#[test]
fn test_select_only_shape_instance() {
    let (_document, _layer_id, shape_ids, clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();

    // Select multiple items
    selection.add_shape_instance(shape_ids[0]);
    selection.add_shape_instance(shape_ids[1]);
    selection.add_clip_instance(clip_ids[0]);

    // Select only shape_ids[0] - this clears ALL selections first
    selection.select_only_shape_instance(shape_ids[0]);

    assert!(selection.contains_shape_instance(&shape_ids[0]));
    assert!(!selection.contains_shape_instance(&shape_ids[1]));
    // select_only_shape_instance calls clear() so clip instances are also cleared
    assert!(!selection.contains_clip_instance(&clip_ids[0]));
}

#[test]
fn test_select_only_clip_instance() {
    let (_document, _layer_id, shape_ids, clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();

    // Select multiple items
    selection.add_shape_instance(shape_ids[0]);
    selection.add_clip_instance(clip_ids[0]);
    selection.add_clip_instance(clip_ids[1]);

    // Select only clip_ids[0] - this clears ALL selections first
    selection.select_only_clip_instance(clip_ids[0]);

    assert!(selection.contains_clip_instance(&clip_ids[0]));
    assert!(!selection.contains_clip_instance(&clip_ids[1]));
    // select_only_clip_instance calls clear() so shape instances are also cleared
    assert!(!selection.contains_shape_instance(&shape_ids[0]));
}

#[test]
fn test_selection_with_transform_action() {
    let (mut document, layer_id, _shape_ids, clip_ids) = setup_mixed_content_document();

    let mut selection = Selection::new();
    selection.add_clip_instance(clip_ids[0]);

    // Transform selected clip instance
    let old_transform = Transform::new();
    let new_transform = Transform::with_position(50.0, 50.0);

    let mut transforms = HashMap::new();
    for &id in selection.clip_instances() {
        transforms.insert(id, (old_transform.clone(), new_transform.clone()));
    }

    let mut action = TransformClipInstancesAction::new(layer_id, transforms);
    action.execute(&mut document);

    // Verify transform applied
    if let Some(AnyLayer::Vector(layer)) = document.get_layer_mut(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == clip_ids[0])
            .unwrap();
        assert_eq!(instance.transform.x, 50.0);
        assert_eq!(instance.transform.y, 50.0);
    }

    // Rollback
    action.rollback(&mut document);

    if let Some(AnyLayer::Vector(layer)) = document.get_layer_mut(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == clip_ids[0])
            .unwrap();
        assert_eq!(instance.transform.x, 0.0);
        assert_eq!(instance.transform.y, 0.0);
    }
}

#[test]
fn test_selection_is_empty() {
    let selection = Selection::new();
    assert!(selection.is_empty());

    let mut selection2 = Selection::new();
    selection2.add_shape_instance(Uuid::new_v4());
    assert!(!selection2.is_empty());

    let mut selection3 = Selection::new();
    selection3.add_clip_instance(Uuid::new_v4());
    assert!(!selection3.is_empty());
}

#[test]
fn test_selection_count() {
    let mut selection = Selection::new();

    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    let clip_id = Uuid::new_v4();

    selection.add_shape_instance(id1);
    selection.add_shape_instance(id2);
    selection.add_clip_instance(clip_id);

    assert_eq!(selection.shape_instances().len(), 2);
    assert_eq!(selection.clip_instances().len(), 1);

    // Remove one
    selection.remove_shape_instance(&id1);
    assert_eq!(selection.shape_instances().len(), 1);

    // Remove clip
    selection.remove_clip_instance(&clip_id);
    assert_eq!(selection.clip_instances().len(), 0);
}

#[test]
fn test_duplicate_selection_handling() {
    let mut selection = Selection::new();
    let id = Uuid::new_v4();

    // Add same ID multiple times
    selection.add_shape_instance(id);
    selection.add_shape_instance(id);
    selection.add_shape_instance(id);

    // Should only contain one instance (dedup behavior)
    assert_eq!(selection.shape_instances().len(), 1);

    // Same for clip instances
    let clip_id = Uuid::new_v4();
    selection.add_clip_instance(clip_id);
    selection.add_clip_instance(clip_id);

    assert_eq!(selection.clip_instances().len(), 1);
}
