//! Change BPM action
//!
//! Atomically changes the document BPM and rescales all clip instance positions and
//! MIDI event timestamps so that beat positions are preserved (Measures mode behaviour).

use crate::action::{Action, BackendContext};
use crate::clip::ClipInstance;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
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

#[derive(Clone)]
struct MidiClipSnapshot {
    layer_id: Uuid,
    midi_clip_id: u32,
    clip_id: Uuid,
    old_clip_duration: f64,
    new_clip_duration: f64,
    old_events: Vec<daw_backend::audio::midi::MidiEvent>,
    new_events: Vec<daw_backend::audio::midi::MidiEvent>,
}

/// Action that atomically changes BPM and rescales all clip/note positions to preserve beats
pub struct ChangeBpmAction {
    old_bpm: f64,
    new_bpm: f64,
    time_sig: (u32, u32),
    clip_snapshots: Vec<ClipTimingSnapshot>,
    midi_snapshots: Vec<MidiClipSnapshot>,
}

impl ChangeBpmAction {
    /// Build the action, computing new positions for all clip instances and MIDI events.
    ///
    /// `midi_event_cache` maps backend MIDI clip ID → current event list.
    pub fn new(
        old_bpm: f64,
        new_bpm: f64,
        document: &Document,
        midi_event_cache: &HashMap<u32, Vec<daw_backend::audio::midi::MidiEvent>>,
    ) -> Self {
        let fps = document.framerate;
        let time_sig = (
            document.time_signature.numerator,
            document.time_signature.denominator,
        );

        let mut clip_snapshots: Vec<ClipTimingSnapshot> = Vec::new();
        let mut midi_snapshots: Vec<MidiClipSnapshot> = Vec::new();

        // Collect MIDI clip IDs we've already snapshotted (avoid duplicates)
        let mut seen_midi_clips: std::collections::HashSet<u32> = std::collections::HashSet::new();

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

                // Compute new fields: beats are canonical, recompute seconds + frames.
                // Guard: if timeline_start_beats was never populated (clips added without
                // sync_from_seconds), derive beats from seconds before applying.
                let mut new_ci = ci.clone();
                if new_ci.timeline_start_beats == 0.0 && new_ci.timeline_start.abs() > 1e-9 {
                    new_ci.sync_from_seconds(old_bpm, fps);
                }
                new_ci.apply_beats(new_bpm, fps);
                let new_fields = TimingFields::from_instance(&new_ci);

                clip_snapshots.push(ClipTimingSnapshot {
                    layer_id,
                    instance_id: ci.id,
                    old_fields,
                    new_fields,
                });

                // If this is a MIDI clip on an audio layer, collect MIDI events + rescale duration.
                // Always snapshot the clip (even if empty) so clip.duration is rescaled.
                if let AnyLayer::Audio(_) = layer {
                    if let Some(audio_clip) = document.get_audio_clip(&ci.clip_id) {
                        use crate::clip::AudioClipType;
                        if let AudioClipType::Midi { midi_clip_id } = &audio_clip.clip_type {
                            let midi_id = *midi_clip_id;
                            if !seen_midi_clips.contains(&midi_id) {
                                seen_midi_clips.insert(midi_id);

                                let old_clip_duration = audio_clip.duration;
                                let new_clip_duration = old_clip_duration * old_bpm / new_bpm;

                                // Use cached events if present; empty vec for clips with no events yet.
                                let old_events = midi_event_cache.get(&midi_id).cloned().unwrap_or_default();
                                let new_events: Vec<_> = old_events.iter().map(|ev| {
                                    let mut e = ev.clone();
                                    // Ensure beats are populated before using them as canonical.
                                    // Events created before triple-rep (e.g. from recording)
                                    // have timestamp_beats == 0.0 — sync from seconds first.
                                    if e.timestamp_beats == 0.0 && e.timestamp.abs() > 1e-9 {
                                        e.sync_from_seconds(old_bpm, fps);
                                    }
                                    e.apply_beats(new_bpm, fps);
                                    e
                                }).collect();

                                midi_snapshots.push(MidiClipSnapshot {
                                    layer_id,
                                    midi_clip_id: midi_id,
                                    clip_id: ci.clip_id,
                                    old_clip_duration,
                                    new_clip_duration,
                                    old_events,
                                    new_events,
                                });
                            }
                        }
                    }
                }
            }
        }

        Self {
            old_bpm,
            new_bpm,
            time_sig,
            clip_snapshots,
            midi_snapshots,
        }
    }

    /// Return the new MIDI event lists for each affected clip (for immediate cache update).
    pub fn new_midi_events(&self) -> impl Iterator<Item = (u32, &Vec<daw_backend::audio::midi::MidiEvent>)> {
        self.midi_snapshots.iter().map(|s| (s.midi_clip_id, &s.new_events))
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

    fn apply_midi_durations(document: &mut Document, snapshots: &[MidiClipSnapshot], use_new: bool) {
        for snap in snapshots {
            if let Some(clip) = document.get_audio_clip_mut(&snap.clip_id) {
                clip.duration = if use_new { snap.new_clip_duration } else { snap.old_clip_duration };
            }
        }
    }
}

impl Action for ChangeBpmAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        document.bpm = self.new_bpm;
        Self::apply_clips(document, &self.clip_snapshots, true);
        Self::apply_midi_durations(document, &self.midi_snapshots, true);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        document.bpm = self.old_bpm;
        Self::apply_clips(document, &self.clip_snapshots, false);
        Self::apply_midi_durations(document, &self.midi_snapshots, false);
        Ok(())
    }

    fn description(&self) -> String {
        "Change BPM".to_string()
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

        // Update tempo
        controller.set_tempo(self.new_bpm as f32, self.time_sig);

        // Update MIDI clip events and positions
        for snap in &self.midi_snapshots {
            let track_id = match backend.layer_to_track_map.get(&snap.layer_id) {
                Some(&id) => id,
                None => continue,
            };
            controller.update_midi_clip_events(track_id, snap.midi_clip_id, snap.new_events.clone());
        }

        // Move clip instances in the backend
        for snap in &self.clip_snapshots {
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
                None => {} // Vector/video clips — no backend move needed
            }
        }

        // Sync beat/frame representations and rescale MIDI clip durations in the backend
        let fps = document.framerate;
        let midi_durations: Vec<(u32, f64)> = self.midi_snapshots.iter()
            .map(|s| (s.midi_clip_id, s.new_clip_duration))
            .collect();
        controller.apply_bpm_change(self.new_bpm, fps, midi_durations);

        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        controller.set_tempo(self.old_bpm as f32, self.time_sig);

        for snap in &self.midi_snapshots {
            let track_id = match backend.layer_to_track_map.get(&snap.layer_id) {
                Some(&id) => id,
                None => continue,
            };
            controller.update_midi_clip_events(track_id, snap.midi_clip_id, snap.old_events.clone());
        }

        for snap in &self.clip_snapshots {
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

        // Sync beat/frame representations and restore MIDI clip durations in the backend
        let fps = document.framerate;
        let midi_durations: Vec<(u32, f64)> = self.midi_snapshots.iter()
            .map(|s| (s.midi_clip_id, s.old_clip_duration))
            .collect();
        controller.apply_bpm_change(self.old_bpm, fps, midi_durations);

        Ok(())
    }

    fn all_midi_events_after_execute(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        self.midi_snapshots.iter()
            .map(|s| (s.midi_clip_id, s.new_events.clone()))
            .collect()
    }

    fn all_midi_events_after_rollback(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        self.midi_snapshots.iter()
            .map(|s| (s.midi_clip_id, s.old_events.clone()))
            .collect()
    }
}
