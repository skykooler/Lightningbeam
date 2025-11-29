//! Integration tests for clip workflow operations
//!
//! Tests end-to-end clip operations including creation, transformation,
//! timeline placement, and undo/redo.

use lightningbeam_core::action::Action;
use lightningbeam_core::actions::{
    MoveClipInstancesAction, TransformClipInstancesAction, TrimClipInstancesAction,
    TrimData, TrimType,
};
use lightningbeam_core::clip::{ClipInstance, VectorClip};
use lightningbeam_core::document::Document;
use lightningbeam_core::layer::{AnyLayer, VectorLayer};
use lightningbeam_core::object::Transform;
use std::collections::HashMap;
use uuid::Uuid;

/// Create a test document with a vector layer containing a clip instance
fn setup_test_document() -> (Document, Uuid, Uuid, Uuid) {
    let mut document = Document::new("Test Project");

    // Create a vector clip
    let vector_clip = VectorClip::new("Test Clip", 10.0, 1920.0, 1080.0);
    let clip_id = vector_clip.id;
    document.vector_clips.insert(clip_id, vector_clip);

    // Create a vector layer with a clip instance
    let mut layer = VectorLayer::new("Layer 1");
    let mut clip_instance = ClipInstance::new(clip_id);
    clip_instance.timeline_start = 0.0;
    clip_instance.transform = Transform::with_position(100.0, 100.0);
    let instance_id = clip_instance.id;
    layer.clip_instances.push(clip_instance);

    let layer_id = document.root.add_child(AnyLayer::Vector(layer));

    (document, layer_id, clip_id, instance_id)
}

#[test]
fn test_clip_instance_creation_workflow() {
    let (document, layer_id, clip_id, instance_id) = setup_test_document();

    // Verify clip is in document
    assert!(document.vector_clips.contains_key(&clip_id));

    // Verify clip instance is on layer
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id);
        assert!(instance.is_some());

        let instance = instance.unwrap();
        assert_eq!(instance.clip_id, clip_id);
        assert_eq!(instance.timeline_start, 0.0);
        assert_eq!(instance.transform.x, 100.0);
        assert_eq!(instance.transform.y, 100.0);
    } else {
        panic!("Layer not found");
    }
}

#[test]
fn test_move_clip_instance_workflow() {
    let (mut document, layer_id, _clip_id, instance_id) = setup_test_document();

    // Create move action: move from 0.0 to 5.0 seconds
    let mut layer_moves = HashMap::new();
    layer_moves.insert(layer_id, vec![(instance_id, 0.0, 5.0)]);

    let mut action = MoveClipInstancesAction::new(layer_moves);

    // Execute
    action.execute(&mut document);

    // Verify position changed
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.timeline_start, 5.0);
    }

    // Rollback (undo)
    action.rollback(&mut document);

    // Verify position restored
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.timeline_start, 0.0);
    }

    // Re-execute (redo)
    action.execute(&mut document);

    // Verify position changed again
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.timeline_start, 5.0);
    }
}

#[test]
fn test_transform_clip_instance_workflow() {
    let (mut document, layer_id, _clip_id, instance_id) = setup_test_document();

    // Create transform action: move, rotate, scale
    let old_transform = Transform::with_position(100.0, 100.0);
    let mut new_transform = Transform::with_position(200.0, 150.0);
    new_transform.rotation = 45.0;
    new_transform.scale_x = 1.5;
    new_transform.scale_y = 1.5;

    let mut transforms = HashMap::new();
    transforms.insert(instance_id, (old_transform, new_transform));

    let mut action = TransformClipInstancesAction::new(layer_id, transforms);

    // Execute
    action.execute(&mut document);

    // Verify transform changed
    if let Some(AnyLayer::Vector(layer)) = document.get_layer_mut(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.transform.x, 200.0);
        assert_eq!(instance.transform.y, 150.0);
        assert_eq!(instance.transform.rotation, 45.0);
        assert_eq!(instance.transform.scale_x, 1.5);
        assert_eq!(instance.transform.scale_y, 1.5);
    }

    // Rollback
    action.rollback(&mut document);

    // Verify transform restored
    if let Some(AnyLayer::Vector(layer)) = document.get_layer_mut(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.transform.x, 100.0);
        assert_eq!(instance.transform.y, 100.0);
        assert_eq!(instance.transform.rotation, 0.0);
        assert_eq!(instance.transform.scale_x, 1.0);
    }
}

#[test]
fn test_trim_clip_instance_workflow() {
    let (mut document, layer_id, _clip_id, instance_id) = setup_test_document();

    // Create trim action: trim 2 seconds from left
    let mut layer_trims = HashMap::new();
    layer_trims.insert(
        layer_id,
        vec![(
            instance_id,
            TrimType::TrimLeft,
            TrimData::left(0.0, 0.0),
            TrimData::left(2.0, 2.0),
        )],
    );

    let mut action = TrimClipInstancesAction::new(layer_trims);

    // Execute
    action.execute(&mut document);

    // Verify trim applied
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.trim_start, 2.0);
        assert_eq!(instance.timeline_start, 2.0);
    }

    // Rollback
    action.rollback(&mut document);

    // Verify trim restored
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        let instance = layer
            .clip_instances
            .iter()
            .find(|ci| ci.id == instance_id)
            .unwrap();
        assert_eq!(instance.trim_start, 0.0);
        assert_eq!(instance.timeline_start, 0.0);
    }
}

#[test]
fn test_multiple_clip_instances_workflow() {
    let mut document = Document::new("Test Project");

    // Create a vector clip
    let vector_clip = VectorClip::new("Test Clip", 10.0, 1920.0, 1080.0);
    let clip_id = vector_clip.id;
    document.vector_clips.insert(clip_id, vector_clip);

    // Create layer with multiple clip instances
    let mut layer = VectorLayer::new("Layer 1");

    let mut instance1 = ClipInstance::new(clip_id);
    instance1.timeline_start = 0.0;
    let id1 = instance1.id;

    let mut instance2 = ClipInstance::new(clip_id);
    instance2.timeline_start = 5.0;
    let id2 = instance2.id;

    let mut instance3 = ClipInstance::new(clip_id);
    instance3.timeline_start = 10.0;
    let id3 = instance3.id;

    layer.clip_instances.push(instance1);
    layer.clip_instances.push(instance2);
    layer.clip_instances.push(instance3);

    let layer_id = document.root.add_child(AnyLayer::Vector(layer));

    // Move all three instances
    let mut layer_moves = HashMap::new();
    layer_moves.insert(
        layer_id,
        vec![
            (id1, 0.0, 1.0),
            (id2, 5.0, 6.0),
            (id3, 10.0, 11.0),
        ],
    );

    let mut action = MoveClipInstancesAction::new(layer_moves);
    action.execute(&mut document);

    // Verify all moved
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id1).unwrap().timeline_start,
            1.0
        );
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id2).unwrap().timeline_start,
            6.0
        );
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id3).unwrap().timeline_start,
            11.0
        );
    }

    // Rollback all
    action.rollback(&mut document);

    // Verify all restored
    if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id1).unwrap().timeline_start,
            0.0
        );
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id2).unwrap().timeline_start,
            5.0
        );
        assert_eq!(
            layer.clip_instances.iter().find(|ci| ci.id == id3).unwrap().timeline_start,
            10.0
        );
    }
}

#[test]
fn test_clip_time_remapping() {
    let mut document = Document::new("Test Project");

    // Create a 10 second clip
    let vector_clip = VectorClip::new("Test Clip", 10.0, 1920.0, 1080.0);
    let clip_id = vector_clip.id;
    let clip_duration = vector_clip.duration;
    document.vector_clips.insert(clip_id, vector_clip);

    // Create instance at timeline 5.0 with trim_start of 2.0
    let mut layer = VectorLayer::new("Layer 1");
    let mut instance = ClipInstance::new(clip_id);
    instance.timeline_start = 5.0;
    instance.trim_start = 2.0;
    instance.trim_end = Some(8.0); // Clip plays from 2.0 to 8.0 internal time
    layer.clip_instances.push(instance.clone());

    document.root.add_child(AnyLayer::Vector(layer));

    // Test time remapping
    // At timeline time 5.0, clip internal time should be 2.0 (trim_start)
    let clip_time = instance.remap_time(5.0, clip_duration);
    assert_eq!(clip_time, Some(2.0));

    // At timeline time 6.0, clip internal time should be 3.0
    let clip_time = instance.remap_time(6.0, clip_duration);
    assert_eq!(clip_time, Some(3.0));

    // At timeline time 10.999, clip internal time should be just under 8.0
    // The clip plays from timeline 5.0 to 11.0 (exclusive end)
    // At timeline 10.999: relative_time = 5.999, content_time = 5.999
    // Since content_window = 6.0, we get: trim_start + 5.999 = 7.999
    let clip_time = instance.remap_time(10.999, clip_duration);
    assert!(clip_time.is_some());
    let time = clip_time.unwrap();
    assert!(time > 7.9 && time < 8.0, "Expected ~7.999, got {}", time);

    // At timeline time 11.0 (exact end), clip should be past its end (None)
    // because the range is [timeline_start, timeline_start + effective_duration)
    let clip_time = instance.remap_time(11.0, clip_duration);
    assert_eq!(clip_time, None);

    // At timeline time 4.0, clip hasn't started yet (None)
    let clip_time = instance.remap_time(4.0, clip_duration);
    assert_eq!(clip_time, None);
}
