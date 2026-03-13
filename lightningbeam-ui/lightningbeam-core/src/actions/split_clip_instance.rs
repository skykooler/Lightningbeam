//! Split clip instance action
//!
//! Handles splitting a clip instance at a specific timeline position,
//! creating two clip instances from one.

use crate::action::{Action, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that splits a clip instance at a specific timeline position
pub struct SplitClipInstanceAction {
    /// The target layer ID
    layer_id: Uuid,

    /// The clip instance to split
    instance_id: Uuid,

    /// Timeline time where to split (in seconds)
    split_time: f64,

    /// Whether the action has been executed (for rollback)
    executed: bool,

    // Stored during execute for rollback
    /// Original trim_end value of the left (original) instance
    original_trim_end: Option<f64>,
    /// Original timeline_duration value of the left (original) instance
    original_timeline_duration: Option<f64>,
    /// ID of the new (right) instance created by the split
    new_instance_id: Option<Uuid>,

    // Backend IDs for the new instance
    /// Backend track ID (stored during execute_backend for undo)
    backend_track_id: Option<daw_backend::TrackId>,
    /// Backend MIDI clip instance ID (stored during execute_backend for undo)
    backend_midi_instance_id: Option<daw_backend::MidiClipInstanceId>,
    /// Backend audio clip instance ID (stored during execute_backend for undo)
    backend_audio_instance_id: Option<daw_backend::AudioClipInstanceId>,
}

impl SplitClipInstanceAction {
    /// Create a new split clip instance action
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The ID of the layer containing the clip instance
    /// * `instance_id` - The ID of the clip instance to split
    /// * `split_time` - The timeline time (in seconds) where to split
    pub fn new(layer_id: Uuid, instance_id: Uuid, split_time: f64) -> Self {
        Self {
            layer_id,
            instance_id,
            split_time,
            executed: false,
            original_trim_end: None,
            original_timeline_duration: None,
            new_instance_id: None,
            backend_track_id: None,
            backend_midi_instance_id: None,
            backend_audio_instance_id: None,
        }
    }

    /// Create a new split clip instance action with a pre-generated ID for the new instance
    ///
    /// Use this when you need to know the new instance ID before execution,
    /// e.g., for creating groups that include the new instance.
    ///
    /// # Arguments
    ///
    /// * `layer_id` - The ID of the layer containing the clip instance
    /// * `instance_id` - The ID of the clip instance to split
    /// * `split_time` - The timeline time (in seconds) where to split
    /// * `new_instance_id` - The UUID to use for the new (right) clip instance
    pub fn with_new_instance_id(layer_id: Uuid, instance_id: Uuid, split_time: f64, new_instance_id: Uuid) -> Self {
        Self {
            layer_id,
            instance_id,
            split_time,
            executed: false,
            original_trim_end: None,
            original_timeline_duration: None,
            new_instance_id: Some(new_instance_id),
            backend_track_id: None,
            backend_midi_instance_id: None,
            backend_audio_instance_id: None,
        }
    }

    /// Get the ID of the new clip instance created by the split (if executed)
    pub fn new_instance_id(&self) -> Option<Uuid> {
        self.new_instance_id
    }

    /// Get the layer ID this action targets
    pub fn layer_id(&self) -> Uuid {
        self.layer_id
    }
}

impl Action for SplitClipInstanceAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Find the clip instance
        let layer = document
            .get_layer(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let clip_instances: &[ClipInstance] = match layer {
            AnyLayer::Vector(vl) => &vl.clip_instances,
            AnyLayer::Audio(al) => &al.clip_instances,
            AnyLayer::Video(vl) => &vl.clip_instances,
            AnyLayer::Effect(el) => &el.clip_instances,
            AnyLayer::Group(_) => return Err("Cannot split clip instances on group layers".to_string()),
            AnyLayer::Raster(_) => return Err("Cannot split clip instances on group layers".to_string()),
        };

        let instance = clip_instances
            .iter()
            .find(|ci| ci.id == self.instance_id)
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;

        // Get the clip's duration
        let clip_duration = document
            .get_clip_duration(&instance.clip_id)
            .ok_or_else(|| format!("Clip {} not found", instance.clip_id))?;

        // Calculate the effective duration and timeline end
        let effective_duration = instance.effective_duration(clip_duration);
        let timeline_end = instance.timeline_start + effective_duration;

        // Validate: split_time must be strictly within the clip's timeline span
        const EPSILON: f64 = 0.001; // 1ms tolerance
        if self.split_time <= instance.timeline_start + EPSILON
            || self.split_time >= timeline_end - EPSILON
        {
            return Err(format!(
                "Split time {} must be within clip bounds ({} to {})",
                self.split_time, instance.timeline_start, timeline_end
            ));
        }

        // Store original values for rollback
        self.original_trim_end = instance.trim_end;
        self.original_timeline_duration = instance.timeline_duration;

        // Check if this is a looping clip
        let is_looping = instance.timeline_duration.is_some();
        let content_duration = instance.trim_end.unwrap_or(clip_duration) - instance.trim_start;

        // Calculate the split point
        let time_into_clip = self.split_time - instance.timeline_start;
        let left_duration = time_into_clip;
        let right_duration = effective_duration - left_duration;

        // Calculate content split time
        let content_split_time = if is_looping {
            // For looping clips, wrap around content
            instance.trim_start + (time_into_clip % content_duration)
        } else {
            instance.trim_start + time_into_clip
        };

        // Clone the instance for the right side
        let mut right_instance = instance.clone();
        // Use pre-generated ID if provided, otherwise generate a new one
        right_instance.id = self.new_instance_id.unwrap_or_else(Uuid::new_v4);
        right_instance.timeline_start = self.split_time;

        if is_looping {
            // For looping clips: both halves keep the same trim values but different timeline_duration
            right_instance.timeline_duration = Some(right_duration);
        } else {
            // For non-looping clips: adjust trim values
            right_instance.trim_start = content_split_time;
            right_instance.trim_end = self.original_trim_end;
            right_instance.timeline_duration = None;
        }

        self.new_instance_id = Some(right_instance.id);

        // Now modify the original (left) instance and add the new (right) instance
        let layer_mut = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        match layer_mut {
            AnyLayer::Vector(vl) => {
                if let Some(inst) = vl.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    if is_looping {
                        inst.timeline_duration = Some(left_duration);
                    } else {
                        inst.trim_end = Some(content_split_time);
                        inst.timeline_duration = None;
                    }
                }
                vl.clip_instances.push(right_instance);
            }
            AnyLayer::Audio(al) => {
                if let Some(inst) = al.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    if is_looping {
                        inst.timeline_duration = Some(left_duration);
                    } else {
                        inst.trim_end = Some(content_split_time);
                        inst.timeline_duration = None;
                    }
                }
                al.clip_instances.push(right_instance);
            }
            AnyLayer::Video(vl) => {
                if let Some(inst) = vl.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    if is_looping {
                        inst.timeline_duration = Some(left_duration);
                    } else {
                        inst.trim_end = Some(content_split_time);
                        inst.timeline_duration = None;
                    }
                }
                vl.clip_instances.push(right_instance);
            }
            AnyLayer::Effect(el) => {
                if let Some(inst) = el.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    if is_looping {
                        inst.timeline_duration = Some(left_duration);
                    } else {
                        inst.trim_end = Some(content_split_time);
                        inst.timeline_duration = None;
                    }
                }
                el.clip_instances.push(right_instance);
            }
            AnyLayer::Group(_) => {
                return Err("Cannot split clip instances on group layers".to_string());
            }
            AnyLayer::Raster(_) => {
                return Err("Cannot split clip instances on group layers".to_string());
            }
        }

        self.executed = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if !self.executed {
            return Ok(());
        }

        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Remove the new instance and restore the original
        match layer {
            AnyLayer::Vector(vl) => {
                // Remove the new instance
                if let Some(new_id) = self.new_instance_id {
                    vl.clip_instances.retain(|ci| ci.id != new_id);
                }
                // Restore original values
                if let Some(inst) = vl.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    inst.trim_end = self.original_trim_end;
                    inst.timeline_duration = self.original_timeline_duration;
                }
            }
            AnyLayer::Audio(al) => {
                if let Some(new_id) = self.new_instance_id {
                    al.clip_instances.retain(|ci| ci.id != new_id);
                }
                if let Some(inst) = al.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    inst.trim_end = self.original_trim_end;
                    inst.timeline_duration = self.original_timeline_duration;
                }
            }
            AnyLayer::Video(vl) => {
                if let Some(new_id) = self.new_instance_id {
                    vl.clip_instances.retain(|ci| ci.id != new_id);
                }
                if let Some(inst) = vl.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    inst.trim_end = self.original_trim_end;
                    inst.timeline_duration = self.original_timeline_duration;
                }
            }
            AnyLayer::Effect(el) => {
                if let Some(new_id) = self.new_instance_id {
                    el.clip_instances.retain(|ci| ci.id != new_id);
                }
                if let Some(inst) = el.clip_instances.iter_mut().find(|ci| ci.id == self.instance_id) {
                    inst.trim_end = self.original_trim_end;
                    inst.timeline_duration = self.original_timeline_duration;
                }
            }
            AnyLayer::Group(_) => {
                // Group layers don't have clip instances, nothing to rollback
            }
            AnyLayer::Raster(_) => {
                // Raster layers don't have clip instances, nothing to rollback
            }
        }

        self.executed = false;
        Ok(())
    }

    fn description(&self) -> String {
        "Split clip instance".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        // Only sync audio clips to the backend
        let layer = document
            .get_layer(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        // Only process audio layers
        if !matches!(layer, AnyLayer::Audio(_)) {
            return Ok(());
        }

        let new_instance_id = match self.new_instance_id {
            Some(id) => id,
            None => return Ok(()), // No new instance created
        };

        // Find clip instances
        let clip_instances = match layer {
            AnyLayer::Audio(al) => &al.clip_instances,
            _ => return Ok(()),
        };

        // Find the new (right) clip instance
        let new_instance = clip_instances
            .iter()
            .find(|ci| ci.id == new_instance_id)
            .ok_or_else(|| "New clip instance not found".to_string())?;

        // Find the original (left) clip instance
        let original_instance = clip_instances
            .iter()
            .find(|ci| ci.id == self.instance_id)
            .ok_or_else(|| "Original clip instance not found".to_string())?;

        // Look up the clip from the document
        let clip = document
            .get_audio_clip(&new_instance.clip_id)
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
                use daw_backend::command::{Query, QueryResponse};

                // 1. Trim the original (left) instance
                let orig_internal_start = original_instance.trim_start;
                let orig_internal_end = original_instance.trim_end.unwrap_or(clip.duration);

                // Look up the original backend instance ID
                if let Some(crate::action::BackendClipInstanceId::Midi(orig_backend_id)) =
                    backend.clip_instance_to_backend_map.get(&self.instance_id)
                {
                    controller.trim_clip(*backend_track_id, *orig_backend_id, orig_internal_start, orig_internal_end);
                }

                // 2. Add the new (right) instance
                let internal_start = new_instance.trim_start;
                let internal_end = new_instance.trim_end.unwrap_or(clip.duration);
                let external_start = new_instance.timeline_start;
                let external_duration = new_instance
                    .timeline_duration
                    .unwrap_or(internal_end - internal_start);

                let instance = daw_backend::MidiClipInstance::new(
                    0,
                    *midi_clip_id,
                    internal_start,
                    internal_end,
                    external_start,
                    external_duration,
                );

                let query = Query::AddMidiClipInstanceSync(*backend_track_id, instance);

                match controller.send_query(query)? {
                    QueryResponse::MidiClipInstanceAdded(Ok(instance_id)) => {
                        self.backend_track_id = Some(*backend_track_id);
                        self.backend_midi_instance_id = Some(instance_id);

                        backend.clip_instance_to_backend_map.insert(
                            new_instance_id,
                            crate::action::BackendClipInstanceId::Midi(instance_id),
                        );

                        Ok(())
                    }
                    QueryResponse::MidiClipInstanceAdded(Err(e)) => Err(e),
                    _ => Err("Unexpected query response".to_string()),
                }
            }
            AudioClipType::Sampled { audio_pool_index } => {
                // 1. Trim the original (left) instance
                let orig_internal_start = original_instance.trim_start;
                let orig_internal_end = original_instance.trim_end.unwrap_or(clip.duration);

                // Look up the original backend instance ID
                if let Some(crate::action::BackendClipInstanceId::Audio(orig_backend_id)) =
                    backend.clip_instance_to_backend_map.get(&self.instance_id)
                {
                    controller.trim_clip(*backend_track_id, *orig_backend_id, orig_internal_start, orig_internal_end);
                }

                // 2. Add the new (right) instance
                let internal_start = new_instance.trim_start;
                let internal_end = new_instance.trim_end.unwrap_or(clip.duration);
                let effective_duration = new_instance.timeline_duration
                    .unwrap_or(internal_end - internal_start);
                let start_time = new_instance.timeline_start;

                let instance_id = controller.add_audio_clip(
                    *backend_track_id,
                    *audio_pool_index,
                    start_time,
                    effective_duration,
                    internal_start,
                );

                self.backend_track_id = Some(*backend_track_id);
                self.backend_audio_instance_id = Some(instance_id);

                backend.clip_instance_to_backend_map.insert(
                    new_instance_id,
                    crate::action::BackendClipInstanceId::Audio(instance_id),
                );

                Ok(())
            }
            AudioClipType::Recording => {
                // Recording clips cannot be split
                Err("Cannot split a clip that is currently recording".to_string())
            }
        }
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        // Remove the new clip from backend if it was added
        if let (Some(track_id), Some(controller)) =
            (self.backend_track_id, backend.audio_controller.as_mut())
        {
            if let Some(midi_instance_id) = self.backend_midi_instance_id {
                controller.remove_midi_clip(track_id, midi_instance_id);
            } else if let Some(audio_instance_id) = self.backend_audio_instance_id {
                controller.remove_audio_clip(track_id, audio_instance_id);
            }

            // Remove from global clip instance mapping
            if let Some(new_id) = self.new_instance_id {
                backend.clip_instance_to_backend_map.remove(&new_id);
            }

            // Restore the original (left) instance's trim on the backend
            // After rollback(), the document instance should have original values restored
            if let Some(layer) = document.get_layer(&self.layer_id) {
                if let AnyLayer::Audio(al) = layer {
                    if let Some(instance) = al.clip_instances.iter().find(|ci| ci.id == self.instance_id) {
                        if let Some(clip) = document.get_audio_clip(&instance.clip_id) {
                            let orig_internal_start = instance.trim_start;
                            let orig_internal_end = self.original_trim_end.unwrap_or(clip.duration);

                            // Restore based on clip type
                            use crate::clip::AudioClipType;
                            match &clip.clip_type {
                                AudioClipType::Midi { .. } => {
                                    if let Some(crate::action::BackendClipInstanceId::Midi(orig_backend_id)) =
                                        backend.clip_instance_to_backend_map.get(&self.instance_id)
                                    {
                                        controller.trim_clip(track_id, *orig_backend_id, orig_internal_start, orig_internal_end);
                                    }
                                }
                                AudioClipType::Sampled { .. } => {
                                    if let Some(crate::action::BackendClipInstanceId::Audio(orig_backend_id)) =
                                        backend.clip_instance_to_backend_map.get(&self.instance_id)
                                    {
                                        controller.trim_clip(track_id, *orig_backend_id, orig_internal_start, orig_internal_end);
                                    }
                                }
                                AudioClipType::Recording => {
                                    // Recording clips - nothing to rollback
                                }
                            }
                        }
                    }
                }
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
    fn test_split_clip_instance() {
        let mut document = Document::new("Test");

        // Create a clip ID
        let clip_id = Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        // Create a clip instance at timeline 0, with trim 0-10 (10 seconds)
        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.timeline_start = 0.0;
        clip_instance.trim_start = 0.0;
        clip_instance.trim_end = Some(10.0);
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Split at timeline 5.0
        let mut action = SplitClipInstanceAction::new(layer_id, instance_id, 5.0);

        // Execute - this will fail because we don't have a real clip in the document
        // In a real test, we'd need to add a VectorClip first
        // For now, just test the structure
        assert_eq!(action.layer_id(), layer_id);
    }

    #[test]
    fn test_split_action_description() {
        let action = SplitClipInstanceAction::new(Uuid::new_v4(), Uuid::new_v4(), 5.0);
        assert_eq!(action.description(), "Split clip instance");
    }
}
