//! Create-text-clip action
//!
//! Used by the text tool when the active layer is a **vector** layer: it creates a
//! VectorClip containing a single text layer, registers it, and places a clip
//! instance in the parent vector layer. The editor then enters that clip so the
//! text is directly editable. All of this is one undoable step so undo never
//! leaves an orphan clip.

use crate::action::Action;
use crate::clip::{ClipInstance, VectorClip};
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::text_layer::TextLayer;
use uuid::Uuid;

/// Minimum clip duration (seconds) when the document has no duration set yet.
const FALLBACK_DURATION: f64 = 10.0;

pub struct CreateTextClipAction {
    /// The vector layer to place the clip instance in.
    parent_layer_id: Uuid,
    /// The text layer to embed (its id is stable across redo).
    text_layer: TextLayer,
    /// Instance position in the parent layer's space.
    position: (f64, f64),

    // Assigned on first execute; reused on redo so ids are stable.
    clip_id: Option<Uuid>,
    instance_id: Option<Uuid>,
    executed: bool,
}

impl CreateTextClipAction {
    pub fn new(parent_layer_id: Uuid, text_layer: TextLayer, position: (f64, f64)) -> Self {
        Self {
            parent_layer_id,
            text_layer,
            position,
            clip_id: None,
            instance_id: None,
            executed: false,
        }
    }

    /// Preset the clip + instance ids (so the caller knows them up-front for
    /// entering the clip immediately after execute). Ids stay stable across redo.
    pub fn with_ids(mut self, clip_id: Uuid, instance_id: Uuid) -> Self {
        self.clip_id = Some(clip_id);
        self.instance_id = Some(instance_id);
        self
    }

    /// The vector clip id (after execute).
    pub fn clip_id(&self) -> Option<Uuid> {
        self.clip_id
    }

    /// The clip instance id placed in the parent layer (after execute).
    pub fn instance_id(&self) -> Option<Uuid> {
        self.instance_id
    }

    /// The embedded text layer's id.
    pub fn text_layer_id(&self) -> Uuid {
        self.text_layer.layer.id
    }
}

impl Action for CreateTextClipAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let clip_id = self.clip_id.unwrap_or_else(Uuid::new_v4);
        self.clip_id = Some(clip_id);
        let instance_id = self.instance_id.unwrap_or_else(Uuid::new_v4);
        self.instance_id = Some(instance_id);

        let duration = if document.duration > 0.0 { document.duration } else { FALLBACK_DURATION };

        // Build the clip with the text layer as its single root layer.
        let mut clip = VectorClip::with_id(
            clip_id,
            "Text",
            self.text_layer.box_width.max(1.0),
            self.text_layer.box_height.max(1.0),
            duration,
        );
        // A movie clip (not keyframe-gated) so the text persists across the timeline.
        clip.is_group = false;
        clip.layers.add_root(AnyLayer::Text(self.text_layer.clone()));
        // Registers the clip and its layers in layer_to_clip_map for O(1) lookup.
        document.add_vector_clip(clip);

        // Place an instance of the clip in the parent vector layer.
        let layer = document
            .get_layer_mut(&self.parent_layer_id)
            .ok_or_else(|| format!("Parent layer {} not found", self.parent_layer_id))?;
        let AnyLayer::Vector(vector_layer) = layer else {
            return Err("Text clip can only be created inside a vector layer".to_string());
        };
        let mut instance = ClipInstance::with_id(instance_id, clip_id);
        instance.transform.x = self.position.0;
        instance.transform.y = self.position.1;
        vector_layer.clip_instances.push(instance);

        self.executed = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if !self.executed {
            return Ok(());
        }
        // Remove the clip instance from the parent layer.
        if let (Some(instance_id), Some(AnyLayer::Vector(vector_layer))) =
            (self.instance_id, document.get_layer_mut(&self.parent_layer_id))
        {
            vector_layer.clip_instances.retain(|ci| ci.id != instance_id);
        }
        // Remove the clip and its layer_to_clip_map registrations.
        if let Some(clip_id) = self.clip_id {
            if let Some(clip) = document.vector_clips.remove(&clip_id) {
                for node in &clip.layers.roots {
                    document.layer_to_clip_map.remove(&node.data.id());
                }
            }
        }
        self.executed = false;
        Ok(())
    }

    fn description(&self) -> String {
        "Add text".to_string()
    }
}
