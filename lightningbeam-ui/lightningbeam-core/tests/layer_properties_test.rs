//! Integration tests for layer property operations
//!
//! Tests solo, mute, lock, opacity, and visibility interactions.

use lightningbeam_core::action::Action;
use lightningbeam_core::actions::{LayerProperty, SetLayerPropertiesAction};
use lightningbeam_core::document::Document;
use lightningbeam_core::layer::{AnyLayer, LayerTrait, VectorLayer};

/// Create a test document with multiple layers
fn setup_multi_layer_document() -> (Document, Vec<uuid::Uuid>) {
    let mut document = Document::new("Test Project");

    let layer1 = VectorLayer::new("Layer 1");
    let layer2 = VectorLayer::new("Layer 2");
    let layer3 = VectorLayer::new("Layer 3");

    let id1 = document.root.add_child(AnyLayer::Vector(layer1));
    let id2 = document.root.add_child(AnyLayer::Vector(layer2));
    let id3 = document.root.add_child(AnyLayer::Vector(layer3));

    (document, vec![id1, id2, id3])
}

#[test]
fn test_solo_interaction_single_layer() {
    let (mut document, ids) = setup_multi_layer_document();
    let id1 = ids[0];

    // Solo layer 1
    let mut action = SetLayerPropertiesAction::new(id1, LayerProperty::Soloed(true));
    action.execute(&mut document);

    // Verify layer 1 is soloed, others are not
    assert_eq!(document.root.get_child(&ids[0]).unwrap().soloed(), true);
    assert_eq!(document.root.get_child(&ids[1]).unwrap().soloed(), false);
    assert_eq!(document.root.get_child(&ids[2]).unwrap().soloed(), false);

    // Only layer 1 should be "effectively visible" for rendering
    let any_soloed = document.visible_layers().any(|l| l.soloed());
    assert!(any_soloed);

    // Unsolo
    action.rollback(&mut document);

    assert_eq!(document.root.get_child(&ids[0]).unwrap().soloed(), false);
}

#[test]
fn test_solo_interaction_multiple_layers() {
    let (mut document, ids) = setup_multi_layer_document();

    // Solo layers 1 and 2
    let mut action = SetLayerPropertiesAction::new_batch(
        vec![ids[0], ids[1]],
        LayerProperty::Soloed(true),
    );
    action.execute(&mut document);

    // Verify layers 1 and 2 are soloed
    assert_eq!(document.root.get_child(&ids[0]).unwrap().soloed(), true);
    assert_eq!(document.root.get_child(&ids[1]).unwrap().soloed(), true);
    assert_eq!(document.root.get_child(&ids[2]).unwrap().soloed(), false);

    // Unsolo both
    action.rollback(&mut document);

    assert_eq!(document.root.get_child(&ids[0]).unwrap().soloed(), false);
    assert_eq!(document.root.get_child(&ids[1]).unwrap().soloed(), false);
}

#[test]
fn test_mute_and_volume_interaction() {
    let (mut document, ids) = setup_multi_layer_document();
    let id1 = ids[0];

    // Set volume to 0.5
    let mut vol_action = SetLayerPropertiesAction::new(id1, LayerProperty::Volume(0.5));
    vol_action.execute(&mut document);

    assert_eq!(document.root.get_child(&id1).unwrap().volume(), 0.5);

    // Mute the layer
    let mut mute_action = SetLayerPropertiesAction::new(id1, LayerProperty::Muted(true));
    mute_action.execute(&mut document);

    // Layer is muted but volume is still 0.5
    assert_eq!(document.root.get_child(&id1).unwrap().muted(), true);
    assert_eq!(document.root.get_child(&id1).unwrap().volume(), 0.5);

    // Unmute
    mute_action.rollback(&mut document);

    // Volume should still be 0.5
    assert_eq!(document.root.get_child(&id1).unwrap().muted(), false);
    assert_eq!(document.root.get_child(&id1).unwrap().volume(), 0.5);
}

#[test]
fn test_lock_prevents_conceptual_editing() {
    let (mut document, ids) = setup_multi_layer_document();
    let id1 = ids[0];

    // Lock layer 1
    let mut action = SetLayerPropertiesAction::new(id1, LayerProperty::Locked(true));
    action.execute(&mut document);

    assert_eq!(document.root.get_child(&id1).unwrap().locked(), true);

    // Note: The lock state is a flag that UI should check before allowing edits
    // The core library doesn't enforce this - it's the UI's responsibility

    // Unlock
    action.rollback(&mut document);
    assert_eq!(document.root.get_child(&id1).unwrap().locked(), false);
}

#[test]
fn test_opacity_cascading() {
    let (mut document, ids) = setup_multi_layer_document();
    let id1 = ids[0];

    // Set opacity to 0.5
    let mut action = SetLayerPropertiesAction::new(id1, LayerProperty::Opacity(0.5));
    action.execute(&mut document);

    assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 0.5);

    // Set to 0.0 (fully transparent)
    let mut action2 = SetLayerPropertiesAction::new(id1, LayerProperty::Opacity(0.0));
    action2.execute(&mut document);

    assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 0.0);

    // Rollback to 0.5
    action2.rollback(&mut document);
    assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 0.5);

    // Rollback to 1.0
    action.rollback(&mut document);
    assert_eq!(document.root.get_child(&id1).unwrap().opacity(), 1.0);
}

#[test]
fn test_visibility_and_solo_interaction() {
    let (mut document, ids) = setup_multi_layer_document();

    // Hide layer 1
    let mut hide_action = SetLayerPropertiesAction::new(ids[0], LayerProperty::Visible(false));
    hide_action.execute(&mut document);

    // Solo layer 1 (while hidden)
    let mut solo_action = SetLayerPropertiesAction::new(ids[0], LayerProperty::Soloed(true));
    solo_action.execute(&mut document);

    // Layer 1 is hidden and soloed
    assert_eq!(document.root.get_child(&ids[0]).unwrap().visible(), false);
    assert_eq!(document.root.get_child(&ids[0]).unwrap().soloed(), true);

    // visible_layers() should NOT include hidden layers
    let visible_count = document.visible_layers().count();
    assert_eq!(visible_count, 2); // Only layers 2 and 3

    // Check if any visible layer is soloed (should be false since layer 1 is hidden)
    let any_visible_soloed = document.visible_layers().any(|l| l.soloed());
    assert_eq!(any_visible_soloed, false);
}

#[test]
fn test_batch_property_changes() {
    let (mut document, ids) = setup_multi_layer_document();

    // Lock all layers
    let mut lock_action = SetLayerPropertiesAction::new_batch(
        ids.clone(),
        LayerProperty::Locked(true),
    );
    lock_action.execute(&mut document);

    for id in &ids {
        assert_eq!(document.root.get_child(id).unwrap().locked(), true);
    }

    // Set opacity on all layers
    let mut opacity_action = SetLayerPropertiesAction::new_batch(
        ids.clone(),
        LayerProperty::Opacity(0.75),
    );
    opacity_action.execute(&mut document);

    for id in &ids {
        assert_eq!(document.root.get_child(id).unwrap().opacity(), 0.75);
    }

    // Rollback opacity
    opacity_action.rollback(&mut document);

    for id in &ids {
        assert_eq!(document.root.get_child(id).unwrap().opacity(), 1.0);
    }

    // Layers should still be locked
    for id in &ids {
        assert_eq!(document.root.get_child(id).unwrap().locked(), true);
    }
}

#[test]
fn test_property_undo_redo_sequence() {
    let (mut document, ids) = setup_multi_layer_document();
    let id1 = ids[0];

    // Sequence of changes
    let mut actions: Vec<SetLayerPropertiesAction> = vec![
        SetLayerPropertiesAction::new(id1, LayerProperty::Opacity(0.8)),
        SetLayerPropertiesAction::new(id1, LayerProperty::Locked(true)),
        SetLayerPropertiesAction::new(id1, LayerProperty::Muted(true)),
    ];

    // Execute all
    for action in &mut actions {
        action.execute(&mut document);
    }

    // Verify final state
    let layer = document.root.get_child(&id1).unwrap();
    assert_eq!(layer.opacity(), 0.8);
    assert_eq!(layer.locked(), true);
    assert_eq!(layer.muted(), true);

    // Undo in reverse order
    for action in actions.iter_mut().rev() {
        action.rollback(&mut document);
    }

    // Verify initial state
    let layer = document.root.get_child(&id1).unwrap();
    assert_eq!(layer.opacity(), 1.0);
    assert_eq!(layer.locked(), false);
    assert_eq!(layer.muted(), false);
}
