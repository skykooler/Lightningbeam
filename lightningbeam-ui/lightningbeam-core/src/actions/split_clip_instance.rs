//! Split clip instance action
//!
//! Handles splitting a clip instance at a specific timeline position,
//! creating two clip instances from one.

use crate::action::{Action, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use daw_backend::ContentTime;
use uuid::Uuid;

/// Action that splits a clip instance at a specific timeline position
pub struct SplitClipInstanceAction {
    /// The target layer ID
    layer_id: Uuid,

    /// The clip instance to split
    instance_id: Uuid,

    /// Timeline position where to split (in beats)
    split_time: daw_backend::Beats,

    /// Whether the action has been executed (for rollback)
    executed: bool,

    // Stored during execute for rollback
    /// Original trim_end value of the left (original) instance
    original_trim_end: Option<ContentTime>,
    /// Original timeline_duration value of the left (original) instance (beats)
    original_timeline_duration: Option<daw_backend::Beats>,
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
    /// * `split_time` - The timeline position (in beats) where to split
    pub fn new(layer_id: Uuid, instance_id: Uuid, split_time: daw_backend::Beats) -> Self {
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
    /// * `split_time` - The timeline position (in beats) where to split
    /// * `new_instance_id` - The UUID to use for the new (right) clip instance
    pub fn with_new_instance_id(layer_id: Uuid, instance_id: Uuid, split_time: daw_backend::Beats, new_instance_id: Uuid) -> Self {
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
            AnyLayer::Text(_) => return Err("Cannot split clip instances on group layers".to_string()),
        };

        let instance = clip_instances
            .iter()
            .find(|ci| ci.id == self.instance_id)
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;

        // The clip's content duration in its OWN domain — seconds for audio/video/vector, beats for
        // MIDI. All the content math below is trim-domain, so it has to be done in whichever domain
        // this clip uses; a seconds duration would silently be added to a MIDI clip's beats trim.
        let trim_duration = document
            .clip_trim_duration(&instance.clip_id)
            .ok_or_else(|| format!("Clip {} not found", instance.clip_id))?;

        // Calculate the effective duration and timeline end (both in beats)
        let effective_duration = instance.effective_duration(trim_duration, document.tempo_map());
        let timeline_end = instance.timeline_start + effective_duration;

        // Validate: split_time must be strictly within the clip's timeline span
        let epsilon = daw_backend::Beats(0.001); // ~1ms tolerance
        if self.split_time <= instance.timeline_start + epsilon
            || self.split_time >= timeline_end - epsilon
        {
            return Err(format!(
                "Split time {} must be within clip bounds ({} to {})",
                self.split_time, instance.timeline_start, timeline_end
            ));
        }

        // Store original values for rollback
        self.original_trim_end = instance.trim_end;
        self.original_timeline_duration = instance.timeline_duration;

        let is_looping = instance.timeline_duration.is_some();
        let content_duration = ContentTime(instance.content_window(trim_duration).native());

        // Timeline split point (beats).
        let time_into_clip = self.split_time - instance.timeline_start;
        let left_duration = time_into_clip;
        let right_duration = effective_duration - left_duration;

        // How far the split lands into the clip's *content*, expressed in the content domain: beats
        // content takes the beats delta directly, wall-clock content takes the seconds delta.
        let tempo_map = document.tempo_map();
        let time_into_content = ContentTime(match trim_duration {
            crate::clip::ClipDuration::Beats(_) => time_into_clip.beats_to_f64(),
            crate::clip::ClipDuration::Seconds(_) => (tempo_map.beats_to_seconds(self.split_time)
                - tempo_map.beats_to_seconds(instance.timeline_start))
            .seconds_to_f64(),
        });

        // Calculate the content split point (content domain).
        let content_split_time = if is_looping && content_duration > ContentTime::ZERO {
            // For looping clips, wrap around content
            instance.trim_start + (time_into_content % content_duration)
        } else {
            instance.trim_start + time_into_content
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
            AnyLayer::Raster(_) | AnyLayer::Text(_) => {
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
            AnyLayer::Raster(_) | AnyLayer::Text(_) => {
                // Raster/text layers don't have clip instances, nothing to rollback
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

        use crate::clip::ResolvedContent;
        if matches!(original_instance.resolve(clip), ResolvedContent::Recording) {
            return Err("Cannot split a clip that is currently recording".to_string());
        }

        // A split is: shorten the left half's backend clip, then add the right half as a new one.
        //
        // 1. Trim the left (original) instance. `trim_range` tags the bounds with the clip's own
        //    content domain, so a MIDI clip's beats trims can't be sent as seconds.
        let left_trim = clip.trim_range(
            original_instance.trim_start,
            original_instance
                .trim_end
                .unwrap_or(ContentTime(clip.content_duration().native())),
        );
        let new_instance = new_instance.clone();

        let backend_track_id = *backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not mapped to backend track", self.layer_id))?;
        let left_backend_id = backend
            .clip_instance_to_backend_map
            .get(&self.instance_id)
            .copied();

        {
            let controller = backend
                .audio_controller
                .as_mut()
                .ok_or_else(|| "Audio controller not available".to_string())?;
            match left_backend_id {
                Some(crate::action::BackendClipInstanceId::Midi(id))
                | Some(crate::action::BackendClipInstanceId::Audio(id)) => {
                    controller.trim_clip(backend_track_id, id, left_trim);
                }
                None => {}
            }
        }

        // 2. Add the right (new) instance via the shared helper — same one AddClipInstanceAction
        //    uses, so the trim/duration conversions live in exactly one place.
        if let Some((track_id, backend_id)) =
            backend.add_clip_instance(document, &self.layer_id, &new_instance)?
        {
            self.backend_track_id = Some(track_id);
            match backend_id {
                crate::action::BackendClipInstanceId::Midi(id) => {
                    self.backend_midi_instance_id = Some(id)
                }
                crate::action::BackendClipInstanceId::Audio(id) => {
                    self.backend_audio_instance_id = Some(id)
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
                            let orig_internal_end = self
                                .original_trim_end
                                .unwrap_or(ContentTime(clip.content_duration().native()));

                            // Restore based on clip type
                            use crate::clip::ResolvedContent;
                            match &instance.resolve(clip) {
                                ResolvedContent::Midi { .. } => {
                                    if let Some(crate::action::BackendClipInstanceId::Midi(orig_backend_id)) =
                                        backend.clip_instance_to_backend_map.get(&self.instance_id)
                                    {
                                        controller.trim_clip(track_id, *orig_backend_id, clip.trim_range(orig_internal_start, orig_internal_end));
                                    }
                                }
                                ResolvedContent::Audio { .. } => {
                                    if let Some(crate::action::BackendClipInstanceId::Audio(orig_backend_id)) =
                                        backend.clip_instance_to_backend_map.get(&self.instance_id)
                                    {
                                        controller.trim_clip(track_id, *orig_backend_id, clip.trim_range(orig_internal_start, orig_internal_end));
                                    }
                                }
                                ResolvedContent::Recording => {
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
        clip_instance.timeline_start = daw_backend::Beats::ZERO;
        clip_instance.trim_start = ContentTime::ZERO;
        clip_instance.trim_end = Some(ContentTime(10.0));
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Split at timeline 5.0
        let mut action = SplitClipInstanceAction::new(layer_id, instance_id, daw_backend::Beats(5.0));

        // Execute - this will fail because we don't have a real clip in the document
        // In a real test, we'd need to add a VectorClip first
        // For now, just test the structure
        assert_eq!(action.layer_id(), layer_id);
    }

    #[test]
    fn test_split_action_description() {
        let action = SplitClipInstanceAction::new(Uuid::new_v4(), Uuid::new_v4(), daw_backend::Beats(5.0));
        assert_eq!(action.description(), "Split clip instance");
    }

    #[test]
    fn splitting_a_midi_clip_stays_in_the_beats_domain() {
        // Regression: `trim_start`/`trim_end` are domain-polymorphic — SECONDS for audio/video/
        // vector, but BEATS for MIDI (the backend takes MIDI trims as `Beats`). Split used to map
        // the split point into the clip's content in seconds unconditionally, so on a MIDI clip it
        // added a seconds delta to a beats offset. At anything but 60 BPM the right half started at
        // the wrong place in the content.
        //
        // At 120 BPM, beat 4 is 2 SECONDS in. The right half must trim to beat 4, not "4 seconds"
        // (= beat 8) and not 2 (the seconds value).
        let mut document = Document::new("Test");
        document.set_bpm(120.0);

        // 8-beat MIDI clip at the timeline origin.
        let clip = crate::clip::AudioClip::new_midi("Midi", 1, daw_backend::Beats(8.0));
        let clip_id = document.add_audio_clip(clip);

        let mut audio_layer = crate::layer::AudioLayer::new("Layer 1");
        let mut instance = ClipInstance::new(clip_id);
        instance.timeline_start = daw_backend::Beats::ZERO;
        instance.trim_start = ContentTime::ZERO;
        instance.trim_end = Some(ContentTime(8.0)); // beats
        let instance_id = instance.id;
        audio_layer.clip_instances.push(instance);
        let layer_id = document.root.add_child(AnyLayer::Audio(audio_layer));

        let mut action = SplitClipInstanceAction::new(layer_id, instance_id, daw_backend::Beats(4.0));
        action.execute(&mut document).expect("split");
        let new_id = action.new_instance_id().expect("right instance");

        let AnyLayer::Audio(al) = document.get_layer(&layer_id).unwrap() else { panic!() };
        let right = al.clip_instances.iter().find(|ci| ci.id == new_id).unwrap();
        let left = al.clip_instances.iter().find(|ci| ci.id == instance_id).unwrap();

        assert_eq!(right.trim_start, ContentTime(4.0), "right half must start 4 BEATS into the content");
        assert_eq!(left.trim_end, Some(ContentTime(4.0)), "left half must end 4 BEATS into the content");
    }
}
