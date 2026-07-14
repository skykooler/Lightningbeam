//! Loop clip instances action
//!
//! Handles extending clip instances beyond their content duration to enable looping,
//! by setting timeline_duration and/or loop_before on the ClipInstance.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Per-instance loop change: (instance_id, old_timeline_duration, new_timeline_duration, old_loop_before, new_loop_before).
/// All durations/offsets are in beats.
pub type LoopEntry = (Uuid, Option<daw_backend::Beats>, Option<daw_backend::Beats>, Option<daw_backend::Beats>, Option<daw_backend::Beats>);

/// Action that changes the loop duration of clip instances
pub struct LoopClipInstancesAction {
    /// Map of layer IDs to vectors of loop entries
    layer_loops: HashMap<Uuid, Vec<LoopEntry>>,
}

impl LoopClipInstancesAction {
    pub fn new(layer_loops: HashMap<Uuid, Vec<LoopEntry>>) -> Self {
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
                AnyLayer::Group(_) => continue,
                AnyLayer::Raster(_) => continue,
                AnyLayer::Text(_) => continue,
            };

            for (instance_id, _old_dur, new_dur, _old_lb, new_lb) in loops {
                if let Some(instance) = clip_instances.iter_mut().find(|ci| ci.id == *instance_id) {
                    instance.timeline_duration = *new_dur;
                    instance.loop_before = *new_lb;
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
                AnyLayer::Group(_) => continue,
                AnyLayer::Raster(_) => continue,
                AnyLayer::Text(_) => continue,
            };

            for (instance_id, old_dur, _new_dur, old_lb, _new_lb) in loops {
                if let Some(instance) = clip_instances.iter_mut().find(|ci| ci.id == *instance_id) {
                    instance.timeline_duration = *old_dur;
                    instance.loop_before = *old_lb;
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
        use crate::clip::ResolvedContent;

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

            for (instance_id, old_dur, new_dur, old_lb, new_lb) in loops {
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                let (target_duration, target_loop_before) = if rollback {
                    (old_dur, old_lb)
                } else {
                    (new_dur, new_lb)
                };

                // Natural content length as a beats span (the fallback when no explicit
                // timeline_duration is set). Resolved in the clip's own domain, so MIDI's beats
                // content carries over directly rather than being read as seconds.
                let content_window_beats = instance.effective_duration_beats(
                    clip.content_duration(),
                    document.tempo_map(),
                );
                let right_duration = target_duration.unwrap_or(content_window_beats);
                let left_duration = target_loop_before.unwrap_or(daw_backend::Beats::ZERO);
                let external_duration = left_duration + right_duration;
                let external_start = instance.timeline_start - left_duration;

                let get_backend_clip_id = |inst_id: &Uuid| -> Result<u32, String> {
                    match &clip.resolve(instance.active_take) {
                        ResolvedContent::Midi { midi_clip_id } => Ok(*midi_clip_id),
                        ResolvedContent::Audio { .. } => {
                            let backend_id = backend.clip_instance_to_backend_map.get(inst_id)
                                .ok_or_else(|| format!("Clip instance {} not mapped to backend", inst_id))?;
                            match backend_id {
                                crate::action::BackendClipInstanceId::Audio(audio_id) => Ok(*audio_id),
                                _ => Err("Expected audio instance ID for sampled clip".to_string()),
                            }
                        }
                        ResolvedContent::Recording => Err("Cannot sync recording clip".to_string()),
                    }
                };

                if let Ok(backend_clip_id) = get_backend_clip_id(instance_id) {
                    controller.move_clip(*track_id, backend_clip_id, external_start);
                    controller.extend_clip(*track_id, backend_clip_id, external_duration);
                }
            }
        }

        Ok(())
    }
}
