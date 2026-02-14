//! Move clip instances action
//!
//! Handles moving one or more clip instances along the timeline.

use crate::action::Action;
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Action that moves clip instances to new timeline positions
pub struct MoveClipInstancesAction {
    /// Map of layer IDs to vectors of (clip_instance_id, old_timeline_start, new_timeline_start)
    layer_moves: HashMap<Uuid, Vec<(Uuid, f64, f64)>>,
}

impl MoveClipInstancesAction {
    /// Create a new move clip instances action
    ///
    /// # Arguments
    ///
    /// * `layer_moves` - Map of layer IDs to vectors of (clip_instance_id, old_timeline_start, new_timeline_start)
    pub fn new(layer_moves: HashMap<Uuid, Vec<(Uuid, f64, f64)>>) -> Self {
        Self { layer_moves }
    }
}

impl Action for MoveClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Expand moves to include grouped instances
        let mut expanded_moves = self.layer_moves.clone();
        let mut already_processed = std::collections::HashSet::new();

        for (_layer_id, moves) in &self.layer_moves {
            for (instance_id, old_start, new_start) in moves {
                // Skip if already processed
                if already_processed.contains(instance_id) {
                    continue;
                }
                already_processed.insert(*instance_id);

                // Check if this instance is in a group
                if let Some(group) = document.find_group_for_instance(instance_id) {
                    let offset = new_start - old_start;

                    // Add all group members to the move list
                    for (member_layer_id, member_instance_id) in group.get_members() {
                        if member_instance_id != instance_id && !already_processed.contains(member_instance_id) {
                            already_processed.insert(*member_instance_id);

                            // Find member's current position
                            if let Some(layer) = document.get_layer(member_layer_id) {
                                let clip_instances: &[ClipInstance] = match layer {
                                    AnyLayer::Vector(vl) => &vl.clip_instances,
                                    AnyLayer::Audio(al) => &al.clip_instances,
                                    AnyLayer::Video(vl) => &vl.clip_instances,
                                    AnyLayer::Effect(el) => &el.clip_instances,
                                };

                                if let Some(instance) = clip_instances.iter().find(|ci| ci.id == *member_instance_id) {
                                    let member_old = instance.timeline_start;
                                    let member_new = member_old + offset;

                                    expanded_moves.entry(*member_layer_id)
                                        .or_insert_with(Vec::new)
                                        .push((*member_instance_id, member_old, member_new));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Auto-adjust moves to avoid overlaps
        let mut adjusted_moves: HashMap<Uuid, Vec<(Uuid, f64, f64)>> = HashMap::new();

        for (layer_id, moves) in &expanded_moves {
            let layer = document.get_layer(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Vector layers don't need adjustment
            if matches!(layer, AnyLayer::Vector(_)) {
                adjusted_moves.insert(*layer_id, moves.clone());
                continue;
            }

            let mut adjusted_layer_moves = Vec::new();

            for (instance_id, old_start, new_start) in moves {
                // Get the instance to calculate its duration
                let clip_instances: &[ClipInstance] = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    AnyLayer::Video(vl) => &vl.clip_instances,
                    AnyLayer::Vector(vl) => &vl.clip_instances,
                    AnyLayer::Effect(el) => &el.clip_instances,
                };

                let instance = clip_instances.iter()
                    .find(|ci| &ci.id == instance_id)
                    .ok_or_else(|| format!("Instance {} not found", instance_id))?;

                let clip_duration = document.get_clip_duration(&instance.clip_id)
                    .ok_or_else(|| format!("Clip {} not found", instance.clip_id))?;

                let trim_start = instance.trim_start;
                let trim_end = instance.trim_end.unwrap_or(clip_duration);
                let effective_duration = trim_end - trim_start;

                // Find nearest valid position, excluding this instance from overlap checks
                let adjusted_start = document.find_nearest_valid_position(
                    layer_id,
                    *new_start,
                    effective_duration,
                    Some(instance_id),
                );

                if let Some(valid_start) = adjusted_start {
                    adjusted_layer_moves.push((*instance_id, *old_start, valid_start));
                } else {
                    return Err(format!(
                        "Cannot move clip: no valid position found on layer"
                    ));
                }
            }

            adjusted_moves.insert(*layer_id, adjusted_layer_moves);
        }

        // Store adjusted moves for rollback
        self.layer_moves = adjusted_moves.clone();

        // Apply all adjusted moves
        for (layer_id, moves) in &adjusted_moves {
            let layer = document.get_layer_mut(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
            };

            // Update timeline_start for each clip instance
            for (clip_id, _old, new) in moves {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    clip_instance.timeline_start = *new;
                }
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        for (layer_id, moves) in &self.layer_moves {
            let layer = document.get_layer_mut(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
            };

            // Restore original timeline_start for each clip instance
            for (clip_id, old, _new) in moves {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    clip_instance.timeline_start = *old;
                }
            }
        }

        Ok(())
    }

    fn description(&self) -> String {
        let total_count: usize = self.layer_moves.values().map(|v| v.len()).sum();
        if total_count == 1 {
            "Move clip instance".to_string()
        } else {
            format!("Move {} clip instances", total_count)
        }
    }

    fn execute_backend(&mut self, backend: &mut crate::action::BackendContext, document: &Document) -> Result<(), String> {
        use crate::layer::AnyLayer;
        use crate::clip::AudioClipType;

        // Get audio controller
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()), // No audio system, skip backend sync
        };

        // Process each layer's moves
        for (layer_id, moves) in &self.layer_moves {
            // Get the layer to determine its type
            let layer = document.get_layer(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Only process audio layers
            if !matches!(layer, AnyLayer::Audio(_)) {
                continue;
            }

            // Look up backend track ID
            let track_id = backend.layer_to_track_map.get(layer_id)
                .ok_or_else(|| format!("Layer {} not mapped to backend track", layer_id))?;

            // Process each clip instance move
            for (instance_id, _old_start, new_start) in moves {
                // Get clip instances from the layer
                let clip_instances = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    _ => continue,
                };

                // Find the clip instance
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                // Look up the clip to determine its type
                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                // Handle move based on clip type
                match &clip.clip_type {
                    AudioClipType::Midi { midi_clip_id } => {
                        // For MIDI: move_clip expects the pool clip ID
                        controller.move_clip(*track_id, *midi_clip_id, *new_start);
                    }
                    AudioClipType::Sampled { .. } => {
                        // For sampled audio: move_clip expects the instance ID
                        let backend_instance_id = backend.clip_instance_to_backend_map.get(instance_id)
                            .ok_or_else(|| format!("Clip instance {} not mapped to backend", instance_id))?;

                        match backend_instance_id {
                            crate::action::BackendClipInstanceId::Audio(audio_id) => {
                                controller.move_clip(*track_id, *audio_id, *new_start);
                            }
                            _ => return Err("Expected audio instance ID for sampled clip".to_string()),
                        }
                    }
                    AudioClipType::Recording => {
                        // Recording clips cannot be moved - skip
                    }
                }
            }
        }

        Ok(())
    }

    fn rollback_backend(&mut self, backend: &mut crate::action::BackendContext, document: &Document) -> Result<(), String> {
        use crate::layer::AnyLayer;
        use crate::clip::AudioClipType;

        // Get audio controller
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()), // No audio system, skip backend sync
        };

        // Process each layer's moves (restore old positions)
        for (layer_id, moves) in &self.layer_moves {
            // Get the layer to determine its type
            let layer = document.get_layer(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Only process audio layers
            if !matches!(layer, AnyLayer::Audio(_)) {
                continue;
            }

            // Look up backend track ID
            let track_id = backend.layer_to_track_map.get(layer_id)
                .ok_or_else(|| format!("Layer {} not mapped to backend track", layer_id))?;

            // Process each clip instance move (restore old position)
            for (instance_id, old_start, _new_start) in moves {
                // Get clip instances from the layer
                let clip_instances = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    _ => continue,
                };

                // Find the clip instance
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                // Look up the clip to determine its type
                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                // Handle move based on clip type (restore old position)
                match &clip.clip_type {
                    AudioClipType::Midi { midi_clip_id } => {
                        // For MIDI: move_clip expects the pool clip ID
                        controller.move_clip(*track_id, *midi_clip_id, *old_start);
                    }
                    AudioClipType::Sampled { .. } => {
                        // For sampled audio: move_clip expects the instance ID
                        let backend_instance_id = backend.clip_instance_to_backend_map.get(instance_id)
                            .ok_or_else(|| format!("Clip instance {} not mapped to backend", instance_id))?;

                        match backend_instance_id {
                            crate::action::BackendClipInstanceId::Audio(audio_id) => {
                                controller.move_clip(*track_id, *audio_id, *old_start);
                            }
                            _ => return Err("Expected audio instance ID for sampled clip".to_string()),
                        }
                    }
                    AudioClipType::Recording => {
                        // Recording clips cannot be moved - skip
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::ClipInstance;
    use crate::layer::VectorLayer;

    #[test]
    fn test_move_clip_instances_action() {
        // Create a document with a test clip instance
        let mut document = Document::new("Test");

        // Create a clip ID (no Clip definition needed for ClipInstance)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.timeline_start = 1.0; // Start at 1 second
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create move action: move from 1.0 to 5.0 seconds
        let mut layer_moves = HashMap::new();
        layer_moves.insert(layer_id, vec![(instance_id, 1.0, 5.0)]);

        let mut action = MoveClipInstancesAction::new(layer_moves);

        // Execute
        action.execute(&mut document).unwrap();

        // Verify position changed
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.timeline_start, 5.0);
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        // Verify position restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.timeline_start, 1.0);
        }
    }
}
