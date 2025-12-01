//! Add clip instance action
//!
//! Handles adding a clip instance to a layer.

use crate::action::{Action, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that adds a clip instance to a layer
pub struct AddClipInstanceAction {
    /// The target layer ID
    layer_id: Uuid,

    /// The clip instance to add
    clip_instance: ClipInstance,

    /// Whether the action has been executed (for rollback)
    executed: bool,

    /// Backend track ID (stored during execute_backend for undo)
    backend_track_id: Option<daw_backend::TrackId>,

    /// Backend MIDI clip instance ID (stored during execute_backend for undo)
    backend_midi_instance_id: Option<daw_backend::MidiClipInstanceId>,

    /// Backend audio clip instance ID (stored during execute_backend for undo)
    backend_audio_instance_id: Option<daw_backend::AudioClipInstanceId>,
}

impl AddClipInstanceAction {
    /// Create a new add clip instance action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The ID of the layer to add the clip instance to
    /// * `clip_instance` - The clip instance to add
    pub fn new(layer_id: Uuid, clip_instance: ClipInstance) -> Self {
        Self {
            layer_id,
            clip_instance,
            executed: false,
            backend_track_id: None,
            backend_midi_instance_id: None,
            backend_audio_instance_id: None,
        }
    }

    /// Get the ID of the clip instance that will be/was added
    pub fn clip_instance_id(&self) -> Uuid {
        self.clip_instance.id
    }

    /// Get the layer ID this action targets
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }
}

impl Action for AddClipInstanceAction {
    fn execute(&mut self, document: &mut Document) {
        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            match layer {
                AnyLayer::Vector(vector_layer) => {
                    vector_layer.clip_instances.push(self.clip_instance.clone());
                }
                AnyLayer::Audio(audio_layer) => {
                    audio_layer.clip_instances.push(self.clip_instance.clone());
                }
                AnyLayer::Video(video_layer) => {
                    video_layer.clip_instances.push(self.clip_instance.clone());
                }
            }
            self.executed = true;
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        if !self.executed {
            return;
        }

        let instance_id = self.clip_instance.id;

        if let Some(layer) = document.get_layer_mut(&self.layer_id) {
            match layer {
                AnyLayer::Vector(vector_layer) => {
                    vector_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
                AnyLayer::Audio(audio_layer) => {
                    audio_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
                AnyLayer::Video(video_layer) => {
                    video_layer
                        .clip_instances
                        .retain(|ci| ci.id != instance_id);
                }
            }
            self.executed = false;
        }
    }

    fn description(&self) -> String {
        "Add clip instance".to_string()
    }

    fn execute_backend(&mut self, backend: &mut BackendContext, document: &Document) -> Result<(), String> {
        // Only sync audio clips to the backend
        // Look up the clip from the document
        let clip = document
            .get_audio_clip(&self.clip_instance.clip_id)
            .ok_or_else(|| "Audio clip not found".to_string())?;

        // Look up backend track ID from layer mapping
        let backend_track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not mapped to backend track", self.layer_id))?;

        // Get audio controller
        let controller = backend
            .audio_controller
            .as_mut()
            .ok_or_else(|| "Audio controller not available".to_string())?;

        // Handle different clip types
        use crate::clip::AudioClipType;
        match &clip.clip_type {
            AudioClipType::Midi { midi_clip_id } => {
                // Create a MIDI clip instance referencing the existing clip in the backend pool
                // No need to add to pool again - it was added during MIDI import
                use daw_backend::command::{Query, QueryResponse};

                // Calculate internal start/end from trim parameters
                let internal_start = self.clip_instance.trim_start;
                let internal_end = self.clip_instance.trim_end.unwrap_or(clip.duration);
                let external_start = self.clip_instance.timeline_start;

                // Calculate external duration (for looping if timeline_duration is set)
                let external_duration = self.clip_instance.timeline_duration
                    .unwrap_or(internal_end - internal_start);

                // Create MidiClipInstance
                let instance = daw_backend::MidiClipInstance::new(
                    0, // Instance ID will be assigned by backend
                    *midi_clip_id,
                    internal_start,
                    internal_end,
                    external_start,
                    external_duration,
                );

                // Send query to add instance and get instance ID
                let query = Query::AddMidiClipInstanceSync(*backend_track_id, instance);

                match controller.send_query(query)? {
                    QueryResponse::MidiClipInstanceAdded(Ok(instance_id)) => {
                        self.backend_track_id = Some(*backend_track_id);
                        self.backend_midi_instance_id = Some(instance_id);
                        Ok(())
                    }
                    QueryResponse::MidiClipInstanceAdded(Err(e)) => Err(e),
                    _ => Err("Unexpected query response".to_string()),
                }
            }
            AudioClipType::Sampled { audio_pool_index } => {
                // For sampled audio, send AddAudioClipSync query
                use daw_backend::command::{Query, QueryResponse};

                let duration = clip.duration;
                let start_time = self.clip_instance.timeline_start;
                let offset = self.clip_instance.trim_start;

                let query =
                    Query::AddAudioClipSync(*backend_track_id, *audio_pool_index, start_time, duration, offset);

                match controller.send_query(query)? {
                    QueryResponse::AudioClipInstanceAdded(Ok(instance_id)) => {
                        self.backend_track_id = Some(*backend_track_id);
                        self.backend_audio_instance_id = Some(instance_id);
                        Ok(())
                    }
                    QueryResponse::AudioClipInstanceAdded(Err(e)) => Err(e),
                    _ => Err("Unexpected query response".to_string()),
                }
            }
        }
    }

    fn rollback_backend(&mut self, backend: &mut BackendContext, _document: &Document) -> Result<(), String> {
        // Remove clip from backend if it was added
        if let (Some(track_id), Some(controller)) =
            (self.backend_track_id, backend.audio_controller.as_mut())
        {
            if let Some(midi_instance_id) = self.backend_midi_instance_id {
                controller.remove_midi_clip(track_id, midi_instance_id);
            } else if let Some(audio_instance_id) = self.backend_audio_instance_id {
                controller.remove_audio_clip(track_id, audio_instance_id);
            }

            // Clear stored IDs
            self.backend_track_id = None;
            self.backend_midi_instance_id = None;
            self.backend_audio_instance_id = None;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;

    #[test]
    fn test_add_clip_instance_to_vector_layer() {
        let mut document = Document::new("Test");

        // Add a layer
        let layer = VectorLayer::new("Test Layer");
        let layer_id = layer.layer.id;
        document.root_mut().add_child(AnyLayer::Vector(layer));

        // Create a clip instance (using a fake clip_id since we're just testing the action)
        let clip_id = Uuid::new_v4();
        let clip_instance = ClipInstance::new(clip_id);
        let instance_id = clip_instance.id;

        // Execute action
        let mut action = AddClipInstanceAction::new(layer_id, clip_instance);
        action.execute(&mut document);

        // Verify clip instance was added
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer(&layer_id) {
            assert_eq!(vector_layer.clip_instances.len(), 1);
            assert_eq!(vector_layer.clip_instances[0].id, instance_id);
        } else {
            panic!("Layer not found");
        }

        // Rollback
        action.rollback(&mut document);

        // Verify clip instance was removed
        if let Some(AnyLayer::Vector(vector_layer)) = document.get_layer(&layer_id) {
            assert_eq!(vector_layer.clip_instances.len(), 0);
        } else {
            panic!("Layer not found");
        }
    }

    #[test]
    fn test_add_clip_instance_description() {
        let action = AddClipInstanceAction::new(Uuid::new_v4(), ClipInstance::new(Uuid::new_v4()));
        assert_eq!(action.description(), "Add clip instance");
    }
}
