//! Loop clip instances action
//!
//! Handles extending clip instances beyond their content duration to enable looping,
//! by setting timeline_duration on the ClipInstance.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that changes the loop duration of clip instances
pub struct LoopClipInstancesAction {
    /// Map of layer IDs to vectors of (instance_id, old_timeline_duration, new_timeline_duration)
    layer_loops: HashMap<Uuid, Vec<(Uuid, Option<f64>, Option<f64>)>>,
}

impl LoopClipInstancesAction {
    pub fn new(layer_loops: HashMap<Uuid, Vec<(Uuid, Option<f64>, Option<f64>)>>) -> Self {
        Self { layer_loops }
    }
}

impl Action for LoopClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        for (layer_id, loops) in &self.layer_loops {
            let layer = document.get_layer_mut(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
            };

            for (instance_id, _old, new) in loops {
                if let Some(instance) = clip_instances.iter_mut().find(|ci| ci.id == *instance_id) {
                    instance.timeline_duration = *new;
                }
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        for (layer_id, loops) in &self.layer_loops {
            let layer = document.get_layer_mut(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
            };

            for (instance_id, old, _new) in loops {
                if let Some(instance) = clip_instances.iter_mut().find(|ci| ci.id == *instance_id) {
                    instance.timeline_duration = *old;
                }
            }
        }
        Ok(())
    }

    fn execute_backend(&mut self, backend: &mut crate::action::BackendContext, document: &Document) -> Result<(), String> {
        self.sync_backend(backend, document, false)
    }

    fn rollback_backend(&mut self, backend: &mut crate::action::BackendContext, document: &Document) -> Result<(), String> {
        self.sync_backend(backend, document, true)
    }

    fn description(&self) -> String {
        "Loop clip".to_string()
    }
}

impl LoopClipInstancesAction {
    fn sync_backend(&self, backend: &mut crate::action::BackendContext, document: &Document, rollback: bool) -> Result<(), String> {
        use crate::clip::AudioClipType;

        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        for (layer_id, loops) in &self.layer_loops {
            let layer = document.get_layer(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            if !matches!(layer, AnyLayer::Audio(_)) {
                continue;
            }

            let track_id = backend.layer_to_track_map.get(layer_id)
                .ok_or_else(|| format!("Layer {} not mapped to backend track", layer_id))?;

            let clip_instances = match layer {
                AnyLayer::Audio(al) => &al.clip_instances,
                _ => continue,
            };

            for (instance_id, old, new) in loops {
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                // Determine which duration to send: on rollback use old, otherwise use new (current)
                let target_duration = if rollback { old } else { new };

                // If timeline_duration is None, the external duration equals the content window
                let content_window = {
                    let trim_end = instance.trim_end.unwrap_or(clip.duration);
                    (trim_end - instance.trim_start).max(0.0)
                };
                let external_duration = target_duration.unwrap_or(content_window);

                match &clip.clip_type {
                    AudioClipType::Midi { midi_clip_id } => {
                        controller.extend_clip(*track_id, *midi_clip_id, external_duration);
                    }
                    AudioClipType::Sampled { .. } => {
                        let backend_instance_id = backend.clip_instance_to_backend_map.get(instance_id)
                            .ok_or_else(|| format!("Clip instance {} not mapped to backend", instance_id))?;

                        match backend_instance_id {
                            crate::action::BackendClipInstanceId::Audio(audio_id) => {
                                controller.extend_clip(*track_id, *audio_id, external_duration);
                            }
                            _ => return Err("Expected audio instance ID for sampled clip".to_string()),
                        }
                    }
                    AudioClipType::Recording => {}
                }
            }
        }

        Ok(())
    }
}
