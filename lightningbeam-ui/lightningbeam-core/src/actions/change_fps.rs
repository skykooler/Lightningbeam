//! Change FPS action
//!
//! Atomically changes the document framerate and rescales all clip instance positions
//! so that frame positions are preserved (Frames mode behaviour).

use crate::action::{Action, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Snapshot of all timing fields on a `ClipInstance`
#[derive(Clone)]
struct TimingFields {
    timeline_start: f64,
    timeline_start_beats: f64,
    timeline_start_frames: f64,
    trim_start: f64,
    trim_start_beats: f64,
    trim_start_frames: f64,
    trim_end: Option<f64>,
    trim_end_beats: Option<f64>,
    trim_end_frames: Option<f64>,
    timeline_duration: Option<f64>,
    timeline_duration_beats: Option<f64>,
    timeline_duration_frames: Option<f64>,
}

impl TimingFields {
    fn from_instance(ci: &ClipInstance) -> Self {
        Self {
            timeline_start: ci.timeline_start,
            timeline_start_beats: ci.timeline_start_beats,
            timeline_start_frames: ci.timeline_start_frames,
            trim_start: ci.trim_start,
            trim_start_beats: ci.trim_start_beats,
            trim_start_frames: ci.trim_start_frames,
            trim_end: ci.trim_end,
            trim_end_beats: ci.trim_end_beats,
            trim_end_frames: ci.trim_end_frames,
            timeline_duration: ci.timeline_duration,
            timeline_duration_beats: ci.timeline_duration_beats,
            timeline_duration_frames: ci.timeline_duration_frames,
        }
    }

    fn apply_to(&self, ci: &mut ClipInstance) {
        ci.timeline_start = self.timeline_start;
        ci.timeline_start_beats = self.timeline_start_beats;
        ci.timeline_start_frames = self.timeline_start_frames;
        ci.trim_start = self.trim_start;
        ci.trim_start_beats = self.trim_start_beats;
        ci.trim_start_frames = self.trim_start_frames;
        ci.trim_end = self.trim_end;
        ci.trim_end_beats = self.trim_end_beats;
        ci.trim_end_frames = self.trim_end_frames;
        ci.timeline_duration = self.timeline_duration;
        ci.timeline_duration_beats = self.timeline_duration_beats;
        ci.timeline_duration_frames = self.timeline_duration_frames;
    }
}

#[derive(Clone)]
struct ClipTimingSnapshot {
    layer_id: Uuid,
    instance_id: Uuid,
    old_fields: TimingFields,
    new_fields: TimingFields,
}

/// Action that atomically changes framerate and rescales all clip positions to preserve frames
pub struct ChangeFpsAction {
    old_fps: f64,
    new_fps: f64,
    clip_snapshots: Vec<ClipTimingSnapshot>,
}

impl ChangeFpsAction {
    /// Build the action, computing new positions for all clip instances.
    pub fn new(old_fps: f64, new_fps: f64, document: &Document) -> Self {
        let bpm = document.bpm;

        let mut clip_snapshots: Vec<ClipTimingSnapshot> = Vec::new();

        for layer in document.all_layers() {
            let layer_id = layer.id();

            let clip_instances: &[ClipInstance] = match layer {
                AnyLayer::Vector(vl) => &vl.clip_instances,
                AnyLayer::Audio(al) => &al.clip_instances,
                AnyLayer::Video(vl) => &vl.clip_instances,
                AnyLayer::Effect(el) => &el.clip_instances,
                AnyLayer::Group(_) | AnyLayer::Raster(_) => continue,
            };

            for ci in clip_instances {
                let old_fields = TimingFields::from_instance(ci);

                // Compute new fields: frames are canonical, recompute seconds + beats
                let mut new_ci = ci.clone();
                new_ci.apply_frames(new_fps, bpm);
                let new_fields = TimingFields::from_instance(&new_ci);

                clip_snapshots.push(ClipTimingSnapshot {
                    layer_id,
                    instance_id: ci.id,
                    old_fields,
                    new_fields,
                });
            }
        }

        Self {
            old_fps,
            new_fps,
            clip_snapshots,
        }
    }

    fn apply_clips(document: &mut Document, snapshots: &[ClipTimingSnapshot], use_new: bool) {
        for snap in snapshots {
            let fields = if use_new { &snap.new_fields } else { &snap.old_fields };

            let layer = match document.get_layer_mut(&snap.layer_id) {
                Some(l) => l,
                None => continue,
            };

            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
                AnyLayer::Effect(el) => &mut el.clip_instances,
                AnyLayer::Group(_) | AnyLayer::Raster(_) => continue,
            };

            if let Some(ci) = clip_instances.iter_mut().find(|ci| ci.id == snap.instance_id) {
                fields.apply_to(ci);
            }
        }
    }
}

impl Action for ChangeFpsAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        document.framerate = self.new_fps;
        Self::apply_clips(document, &self.clip_snapshots, true);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        document.framerate = self.old_fps;
        Self::apply_clips(document, &self.clip_snapshots, false);
        Ok(())
    }

    fn description(&self) -> String {
        "Change FPS".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // FPS change does not affect audio timing — only move clips that changed position
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        for snap in &self.clip_snapshots {
            if (snap.new_fields.timeline_start - snap.old_fields.timeline_start).abs() < 1e-9 {
                continue; // No movement, skip
            }
            let track_id = match backend.layer_to_track_map.get(&snap.layer_id) {
                Some(&id) => id,
                None => continue,
            };
            let backend_id = backend.clip_instance_to_backend_map.get(&snap.instance_id);
            match backend_id {
                Some(crate::action::BackendClipInstanceId::Audio(audio_id)) => {
                    controller.move_clip(track_id, *audio_id, snap.new_fields.timeline_start);
                }
                Some(crate::action::BackendClipInstanceId::Midi(midi_id)) => {
                    controller.move_clip(track_id, *midi_id, snap.new_fields.timeline_start);
                }
                None => {}
            }
        }

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        for snap in &self.clip_snapshots {
            if (snap.new_fields.timeline_start - snap.old_fields.timeline_start).abs() < 1e-9 {
                continue;
            }
            let track_id = match backend.layer_to_track_map.get(&snap.layer_id) {
                Some(&id) => id,
                None => continue,
            };
            let backend_id = backend.clip_instance_to_backend_map.get(&snap.instance_id);
            match backend_id {
                Some(crate::action::BackendClipInstanceId::Audio(audio_id)) => {
                    controller.move_clip(track_id, *audio_id, snap.old_fields.timeline_start);
                }
                Some(crate::action::BackendClipInstanceId::Midi(midi_id)) => {
                    controller.move_clip(track_id, *midi_id, snap.old_fields.timeline_start);
                }
                None => {}
            }
        }

        Ok(())
    }
}
