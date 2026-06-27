//! Remove clip instances action
//!
//! Handles removing one or more clip instances from layers (for cut/delete).

use crate::action::{Action, BackendClipInstanceId, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that removes clip instances from layers
pub struct RemoveClipInstancesAction {
    /// (layer_id, instance_id) pairs to remove
    removals: Vec<(Uuid, Uuid)>,
    /// Saved instances for rollback (layer_id -> ClipInstance)
    saved: Vec<(Uuid, ClipInstance)>,
    /// Saved backend mappings for rollback (instance_id -> BackendClipInstanceId)
    saved_backend_ids: HashMap<Uuid, BackendClipInstanceId>,
}

impl RemoveClipInstancesAction {
    /// Create a new remove clip instances action
    pub fn new(removals: Vec<(Uuid, Uuid)>) -> Self {
        Self {
            removals,
            saved: Vec::new(),
            saved_backend_ids: HashMap::new(),
        }
    }
}

impl Action for RemoveClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.saved.clear();

        for (layer_id, instance_id) in &self.removals {
            let layer = document
                .get_layer_mut(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
                AnyLayer::Group(_) => continue,
                AnyLayer::Raster(_) => continue,
                AnyLayer::Text(_) => continue,
            };

            // Find and remove the instance, saving it for rollback
            if let Some(pos) = clip_instances.iter().position(|ci| ci.id == *instance_id) {
                let removed = clip_instances.remove(pos);
                self.saved.push((*layer_id, removed));
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        // Re-insert saved instances
        for (layer_id, instance) in self.saved.drain(..).rev() {
            let layer = document
                .get_layer_mut(&layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
                AnyLayer::Group(_) => continue,
                AnyLayer::Raster(_) => continue,
                AnyLayer::Text(_) => continue,
            };

            clip_instances.push(instance);
        }

        Ok(())
    }

    fn description(&self) -> String {
        let count = self.removals.len();
        if count == 1 {
            "Delete clip instance".to_string()
        } else {
            format!("Delete {} clip instances", count)
        }
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        for (layer_id, instance_id) in &self.removals {
            // Only process audio layers
            let layer = match document.get_layer(layer_id) {
                Some(l) => l,
                None => continue,
            };
            if !matches!(layer, AnyLayer::Audio(_)) {
                continue;
            }

            let track_id = match backend.layer_to_track_map.get(layer_id) {
                Some(id) => *id,
                None => continue,
            };

            // Remove from backend using stored mapping
            if let Some(backend_id) = backend.clip_instance_to_backend_map.remove(instance_id) {
                self.saved_backend_ids.insert(*instance_id, backend_id.clone());
                match backend_id {
                    BackendClipInstanceId::Midi(midi_id) => {
                        controller.remove_midi_clip(track_id, midi_id);
                    }
                    BackendClipInstanceId::Audio(audio_id) => {
                        controller.remove_audio_clip(track_id, audio_id);
                    }
                }
            }
        }

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        use crate::clip::AudioClipType;

        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        // Re-add clips that were removed from backend
        for (layer_id, instance) in &self.saved {
            let layer = match document.get_layer(layer_id) {
                Some(l) => l,
                None => continue,
            };
            if !matches!(layer, AnyLayer::Audio(_)) {
                continue;
            }

            let track_id = match backend.layer_to_track_map.get(layer_id) {
                Some(id) => *id,
                None => continue,
            };

            let clip = match document.get_audio_clip(&instance.clip_id) {
                Some(c) => c,
                None => continue,
            };

            match &clip.clip_type {
                AudioClipType::Midi { midi_clip_id } => {
                    use daw_backend::command::{Query, QueryResponse};

                    let internal_start = instance.trim_start;
                    let internal_end = instance.trim_end.unwrap_or(clip.duration);
                    let external_start = instance.timeline_start;
                    let external_duration = instance
                        .timeline_duration
                        .unwrap_or(internal_end - internal_start);

                    let midi_instance = daw_backend::MidiClipInstance::new(
                        0,
                        *midi_clip_id,
                        daw_backend::Beats(internal_start),
                        daw_backend::Beats(internal_end),
                        daw_backend::Beats(external_start),
                        daw_backend::Beats(external_duration),
                    );

                    let query = Query::AddMidiClipInstanceSync(track_id, midi_instance);
                    if let Ok(QueryResponse::MidiClipInstanceAdded(Ok(new_id))) =
                        controller.send_query(query)
                    {
                        backend.clip_instance_to_backend_map.insert(
                            instance.id,
                            BackendClipInstanceId::Midi(new_id),
                        );
                    }
                }
                AudioClipType::Sampled { audio_pool_index } => {
                    let internal_start = instance.trim_start;
                    let internal_end = instance.trim_end.unwrap_or(clip.duration);
                    let effective_duration = instance.timeline_duration
                        .unwrap_or(internal_end - internal_start);
                    let start_time = instance.timeline_start;

                    let new_id = controller.add_audio_clip(
                        track_id,
                        *audio_pool_index,
                        start_time,
                        effective_duration,
                        internal_start,
                    );
                    backend.clip_instance_to_backend_map.insert(
                        instance.id,
                        BackendClipInstanceId::Audio(new_id),
                    );
                }
                AudioClipType::Recording => {}
            }
        }

        // Clear saved backend IDs
        self.saved_backend_ids.clear();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::VectorLayer;

    #[test]
    fn test_remove_clip_instances() {
        let mut document = Document::new("Test");

        let clip_id = Uuid::new_v4();
        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut ci1 = ClipInstance::new(clip_id);
        ci1.timeline_start = 0.0;
        let id1 = ci1.id;

        let mut ci2 = ClipInstance::new(clip_id);
        ci2.timeline_start = 5.0;
        let id2 = ci2.id;

        vector_layer.clip_instances.push(ci1);
        vector_layer.clip_instances.push(ci2);

        let layer_id = document.root_mut().add_child(AnyLayer::Vector(vector_layer));

        // Remove first clip instance
        let mut action = RemoveClipInstancesAction::new(vec![(layer_id, id1)]);
        action.execute(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.clip_instances.len(), 1);
            assert_eq!(vl.clip_instances[0].id, id2);
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
            assert_eq!(vl.clip_instances.len(), 2);
        }
    }
}
