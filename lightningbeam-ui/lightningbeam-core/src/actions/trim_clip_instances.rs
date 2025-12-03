//! Trim clip instances action
//!
//! Handles trimming one or more clip instances by adjusting trim_start and/or trim_end.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Type of trim operation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrimType {
    /// Trim from the start (adjust trim_start and timeline_start)
    TrimLeft,
    /// Trim from the end (adjust trim_end)
    TrimRight,
}

/// Action that trims clip instances
pub struct TrimClipInstancesAction {
    /// Map of layer IDs to vectors of (clip_instance_id, trim_type, old_values, new_values)
    /// For TrimLeft: (old_trim_start, old_timeline_start, new_trim_start, new_timeline_start)
    /// For TrimRight: (old_trim_end, new_trim_end) - stored as Option<f64>
    layer_trims: HashMap<Uuid, Vec<(Uuid, TrimType, TrimData, TrimData)>>,
}

/// Trim data that can represent either left or right trim values
#[derive(Debug, Clone)]
pub struct TrimData {
    /// For TrimLeft: trim_start value
    /// For TrimRight: trim_end value (Option because it can be None)
    pub trim_value: Option<f64>,
    /// For TrimLeft: timeline_start value (where the clip appears on timeline)
    /// For TrimRight: unused (None)
    pub timeline_start: Option<f64>,
}

impl TrimData {
    /// Create TrimData for left trim
    pub fn left(trim_start: f64, timeline_start: f64) -> Self {
        Self {
            trim_value: Some(trim_start),
            timeline_start: Some(timeline_start),
        }
    }

    /// Create TrimData for right trim
    pub fn right(trim_end: Option<f64>) -> Self {
        Self {
            trim_value: trim_end,
            timeline_start: None,
        }
    }
}

impl TrimClipInstancesAction {
    /// Create a new trim clip instances action
    pub fn new(layer_trims: HashMap<Uuid, Vec<(Uuid, TrimType, TrimData, TrimData)>>) -> Self {
        Self { layer_trims }
    }
}

impl Action for TrimClipInstancesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Expand trims to include grouped instances
        let mut expanded_trims = self.layer_trims.clone();
        let mut already_processed = std::collections::HashSet::new();

        for (layer_id, trims) in &self.layer_trims {
            for (instance_id, trim_type, old, new) in trims {
                // Skip if already processed
                if already_processed.contains(instance_id) {
                    continue;
                }
                already_processed.insert(*instance_id);

                // Check if this instance is in a group
                if let Some(group) = document.find_group_for_instance(instance_id) {
                    // Calculate offset based on trim type
                    match trim_type {
                        TrimType::TrimLeft => {
                            if let (Some(old_trim), Some(new_trim), Some(old_timeline), Some(new_timeline)) =
                                (old.trim_value, new.trim_value, old.timeline_start, new.timeline_start)
                            {
                                let trim_offset = new_trim - old_trim;
                                let timeline_offset = new_timeline - old_timeline;

                                // Add all group members to the trim list
                                for (member_layer_id, member_instance_id) in group.get_members() {
                                    if member_instance_id != instance_id && !already_processed.contains(member_instance_id) {
                                        already_processed.insert(*member_instance_id);

                                        // Find member's current values
                                        if let Some(layer) = document.get_layer(member_layer_id) {
                                            let clip_instances = match layer {
                                                AnyLayer::Vector(vl) => &vl.clip_instances,
                                                AnyLayer::Audio(al) => &al.clip_instances,
                                                AnyLayer::Video(vl) => &vl.clip_instances,
                                            };

                                            if let Some(instance) = clip_instances.iter().find(|ci| ci.id == *member_instance_id) {
                                                let member_old_trim = instance.trim_start;
                                                let member_old_timeline = instance.timeline_start;
                                                let member_new_trim = member_old_trim + trim_offset;
                                                let member_new_timeline = member_old_timeline + timeline_offset;

                                                expanded_trims.entry(*member_layer_id)
                                                    .or_insert_with(Vec::new)
                                                    .push((
                                                        *member_instance_id,
                                                        TrimType::TrimLeft,
                                                        TrimData::left(member_old_trim, member_old_timeline),
                                                        TrimData::left(member_new_trim, member_new_timeline),
                                                    ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        TrimType::TrimRight => {
                            // Add all group members to the trim list
                            for (member_layer_id, member_instance_id) in group.get_members() {
                                if member_instance_id != instance_id && !already_processed.contains(member_instance_id) {
                                    already_processed.insert(*member_instance_id);

                                    // Find member's current trim_end
                                    if let Some(layer) = document.get_layer(member_layer_id) {
                                        let clip_instances = match layer {
                                            AnyLayer::Vector(vl) => &vl.clip_instances,
                                            AnyLayer::Audio(al) => &al.clip_instances,
                                            AnyLayer::Video(vl) => &vl.clip_instances,
                                        };

                                        if let Some(instance) = clip_instances.iter().find(|ci| ci.id == *member_instance_id) {
                                            let member_old_trim_end = instance.trim_end;
                                            let member_new_trim_end = new.trim_value;

                                            expanded_trims.entry(*member_layer_id)
                                                .or_insert_with(Vec::new)
                                                .push((
                                                    *member_instance_id,
                                                    TrimType::TrimRight,
                                                    TrimData::right(member_old_trim_end),
                                                    TrimData::right(member_new_trim_end),
                                                ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Auto-clamp trims to avoid overlaps when extending clips
        let mut clamped_trims: HashMap<Uuid, Vec<(Uuid, TrimType, TrimData, TrimData)>> = HashMap::new();

        for (layer_id, trims) in &expanded_trims {
            let layer = document.get_layer(layer_id)
                .ok_or_else(|| format!("Layer {} not found", layer_id))?;

            // Only validate for audio/video layers
            let should_validate = matches!(layer, AnyLayer::Audio(_) | AnyLayer::Video(_));

            let mut clamped_layer_trims = Vec::new();

            for (instance_id, trim_type, old, new) in trims {
                let clip_instances = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    AnyLayer::Video(vl) => &vl.clip_instances,
                    AnyLayer::Vector(vl) => &vl.clip_instances,
                };

                let instance = clip_instances.iter()
                    .find(|ci| &ci.id == instance_id)
                    .ok_or_else(|| format!("Instance {} not found", instance_id))?;

                let clip_duration = document.get_clip_duration(&instance.clip_id)
                    .ok_or_else(|| format!("Clip {} not found", instance.clip_id))?;

                let mut clamped_new = new.clone();

                match trim_type {
                    TrimType::TrimLeft => {
                        if let (Some(old_trim), Some(new_trim), Some(old_timeline), Some(new_timeline)) =
                            (old.trim_value, new.trim_value, old.timeline_start, new.timeline_start)
                        {
                            // If extending to the left (new_trim < old_trim)
                            if should_validate && new_trim < old_trim {
                                // Find the maximum we can extend left
                                let max_extend = document.find_max_trim_extend_left(
                                    layer_id,
                                    instance_id,
                                    instance.timeline_start,
                                );

                                // Calculate how much we want to extend
                                let desired_extend = old_trim - new_trim;

                                // Clamp to max allowed
                                let actual_extend = desired_extend.min(max_extend);
                                let clamped_trim_start = old_trim - actual_extend;
                                let clamped_timeline_start = old_timeline - actual_extend;

                                clamped_new = TrimData::left(clamped_trim_start, clamped_timeline_start);
                            }
                        }
                    }
                    TrimType::TrimRight => {
                        let old_trim_end = old.trim_value.unwrap_or(clip_duration);
                        let new_trim_end = new.trim_value.unwrap_or(clip_duration);

                        // If extending to the right (new_trim_end > old_trim_end)
                        if should_validate && new_trim_end > old_trim_end {
                            // Calculate current effective duration
                            let current_effective_duration = old_trim_end - instance.trim_start;

                            // Find the maximum we can extend right
                            let max_extend = document.find_max_trim_extend_right(
                                layer_id,
                                instance_id,
                                instance.timeline_start,
                                current_effective_duration,
                            );

                            // Calculate how much we want to extend
                            let desired_extend = new_trim_end - old_trim_end;

                            // Clamp to max allowed
                            let actual_extend = desired_extend.min(max_extend);
                            let clamped_trim_end = old_trim_end + actual_extend;

                            // Don't exceed clip duration
                            let final_trim_end = clamped_trim_end.min(clip_duration);

                            clamped_new = TrimData::right(Some(final_trim_end));
                        }
                    }
                }

                clamped_layer_trims.push((*instance_id, *trim_type, old.clone(), clamped_new));
            }

            clamped_trims.insert(*layer_id, clamped_layer_trims);
        }

        // Store clamped trims for rollback
        self.layer_trims = clamped_trims.clone();

        // Apply all clamped trims
        for (layer_id, trims) in &clamped_trims {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Apply trims
            for (clip_id, trim_type, _old, new) in trims {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    match trim_type {
                        TrimType::TrimLeft => {
                            if let (Some(new_trim), Some(new_timeline)) =
                                (new.trim_value, new.timeline_start)
                            {
                                clip_instance.trim_start = new_trim;
                                clip_instance.timeline_start = new_timeline;
                            }
                        }
                        TrimType::TrimRight => {
                            clip_instance.trim_end = new.trim_value;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        for (layer_id, trims) in &self.layer_trims {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Restore original trim values
            for (clip_id, trim_type, old, _new) in trims {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    match trim_type {
                        TrimType::TrimLeft => {
                            if let (Some(old_trim), Some(old_timeline)) =
                                (old.trim_value, old.timeline_start)
                            {
                                clip_instance.trim_start = old_trim;
                                clip_instance.timeline_start = old_timeline;
                            }
                        }
                        TrimType::TrimRight => {
                            clip_instance.trim_end = old.trim_value;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        let total_count: usize = self.layer_trims.values().map(|v| v.len()).sum();
        if total_count == 1 {
            "Trim clip instance".to_string()
        } else {
            format!("Trim {} clip instances", total_count)
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

        // Process each layer's trims
        for (layer_id, trims) in &self.layer_trims {
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

            // Process each clip instance trim
            for (instance_id, trim_type, _old, new) in trims {
                // Get clip instances from the layer
                let clip_instances = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    _ => continue,
                };

                // Find the clip instance (post-execute, so it has new trim values)
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                // Look up the clip to determine its type and duration
                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                // Calculate new internal_start and internal_end for backend
                // Note: instance already has the new trim values after execute()
                let internal_start = instance.trim_start;
                let internal_end = instance.trim_end.unwrap_or(clip.duration);

                // Handle trim based on clip type
                match &clip.clip_type {
                    AudioClipType::Midi { midi_clip_id } => {
                        // For MIDI: trim_clip expects the pool clip ID
                        controller.trim_clip(*track_id, *midi_clip_id, internal_start, internal_end);
                    }
                    AudioClipType::Sampled { .. } => {
                        // For sampled audio: trim_clip expects the instance ID
                        let backend_instance_id = backend.clip_instance_to_backend_map.get(instance_id)
                            .ok_or_else(|| format!("Clip instance {} not mapped to backend", instance_id))?;

                        match backend_instance_id {
                            crate::action::BackendClipInstanceId::Audio(audio_id) => {
                                controller.trim_clip(*track_id, *audio_id, internal_start, internal_end);
                            }
                            _ => return Err("Expected audio instance ID for sampled clip".to_string()),
                        }
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

        // Process each layer's trims (restore old trim values)
        for (layer_id, trims) in &self.layer_trims {
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

            // Process each clip instance trim (restore old values)
            for (instance_id, trim_type, old, _new) in trims {
                // Get clip instances from the layer
                let clip_instances = match layer {
                    AnyLayer::Audio(al) => &al.clip_instances,
                    _ => continue,
                };

                // Find the clip instance
                let instance = clip_instances.iter()
                    .find(|ci| ci.id == *instance_id)
                    .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

                // Look up the clip to determine its type and duration
                let clip = document.get_audio_clip(&instance.clip_id)
                    .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

                // Calculate old internal_start and internal_end for backend
                let internal_start = match trim_type {
                    TrimType::TrimLeft => old.trim_value.unwrap_or(0.0),
                    TrimType::TrimRight => instance.trim_start, // trim_start wasn't changed
                };
                let internal_end = match trim_type {
                    TrimType::TrimLeft => instance.trim_end.unwrap_or(clip.duration), // trim_end wasn't changed
                    TrimType::TrimRight => old.trim_value.unwrap_or(clip.duration),
                };

                // Handle trim based on clip type
                match &clip.clip_type {
                    AudioClipType::Midi { midi_clip_id } => {
                        // For MIDI: trim_clip expects the pool clip ID
                        controller.trim_clip(*track_id, *midi_clip_id, internal_start, internal_end);
                    }
                    AudioClipType::Sampled { .. } => {
                        // For sampled audio: trim_clip expects the instance ID
                        let backend_instance_id = backend.clip_instance_to_backend_map.get(instance_id)
                            .ok_or_else(|| format!("Clip instance {} not mapped to backend", instance_id))?;

                        match backend_instance_id {
                            crate::action::BackendClipInstanceId::Audio(audio_id) => {
                                controller.trim_clip(*track_id, *audio_id, internal_start, internal_end);
                            }
                            _ => return Err("Expected audio instance ID for sampled clip".to_string()),
                        }
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
    fn test_trim_left_action() {
        let mut document = Document::new("Test");

        // Create a clip ID (ClipInstance references clip by ID)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.timeline_start = 0.0;
        clip_instance.trim_start = 0.0;
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

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
        action.execute(&mut document).unwrap();

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
        action.rollback(&mut document).unwrap();

        // Verify restored
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
    fn test_trim_right_action() {
        let mut document = Document::new("Test");

        // Create a clip ID (ClipInstance references clip by ID)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.trim_end = None; // Full duration
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create trim action: trim to 8 seconds from right
        let mut layer_trims = HashMap::new();
        layer_trims.insert(
            layer_id,
            vec![(
                instance_id,
                TrimType::TrimRight,
                TrimData::right(None),
                TrimData::right(Some(8.0)),
            )],
        );

        let mut action = TrimClipInstancesAction::new(layer_trims);

        // Execute
        action.execute(&mut document).unwrap();

        // Verify trim applied
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_end, Some(8.0));
        }

        // Rollback
        action.rollback(&mut document).unwrap();

        // Verify restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_end, None);
        }
    }
}
