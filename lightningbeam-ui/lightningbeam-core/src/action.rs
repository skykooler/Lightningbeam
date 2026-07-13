//! Action system for undo/redo functionality
//!
//! This module provides a type-safe action system that ensures document
//! mutations can only happen through actions, enforced by Rust's type system.
//!
//! ## Architecture
//!
//! - `Action` trait: Defines execute() and rollback() operations
//! - `ActionExecutor`: Wraps the document and manages undo/redo stacks
//! - Document mutations are only accessible via `pub(crate)` methods
//! - External code gets read-only access via `ActionExecutor::document()`
//!
//! ## Memory Model
//!
//! The document is stored in an `Arc<Document>` for efficient cloning during
//! GPU render callbacks. When mutation is needed, `Arc::make_mut()` provides
//! copy-on-write semantics - if other Arc holders exist (e.g., in-flight render
//! callbacks), the document is cloned before mutation, preserving their snapshot.

use crate::document::Document;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Backend clip instance ID - wraps both MIDI and Audio instance IDs
#[derive(Debug, Clone, Copy)]
pub enum BackendClipInstanceId {
    Midi(daw_backend::MidiClipInstanceId),
    Audio(daw_backend::AudioClipInstanceId),
}

/// Backend context for actions that need to interact with external systems
///
/// This bundles all backend references (audio, future video) that actions
/// may need to synchronize state with external systems beyond the document.
pub struct BackendContext<'a> {
    /// Audio engine controller (optional - may not be initialized)
    pub audio_controller: Option<&'a mut daw_backend::EngineController>,

    /// Mapping from all document layer/clip/group UUIDs to backend track IDs.
    /// Covers audio layers, MIDI layers, group layers, and vector clip metatracks.
    pub layer_to_track_map: &'a HashMap<Uuid, daw_backend::TrackId>,

    /// Mapping from document clip instance UUIDs to backend clip instance IDs
    pub clip_instance_to_backend_map: &'a mut HashMap<Uuid, BackendClipInstanceId>,

    // Future: pub video_controller: Option<&'a mut VideoController>,
}

impl BackendContext<'_> {
    /// Hand a clip instance to the audio engine and record it in the instance→backend map.
    ///
    /// Take folders are resolved through the instance's `active_take`, so the backend gets whichever
    /// take is selected. Returns the backend track and instance IDs, or `None` when there's nothing
    /// to sync yet (a recording in progress, or an empty take folder).
    ///
    /// Lives here rather than in any one action because more than one action needs it: adding an
    /// instance, and switching a take folder's active take (which is a remove + re-add, there being
    /// no in-place pool-swap command). Keeping one implementation keeps the trim/duration
    /// conversions — the easy thing to get subtly wrong, since `trim_*` is SECONDS while
    /// `timeline_*` is BEATS — from drifting between copies.
    pub fn add_clip_instance(
        &mut self,
        document: &Document,
        layer_id: &Uuid,
        instance: &crate::clip::ClipInstance,
    ) -> Result<Option<(daw_backend::TrackId, BackendClipInstanceId)>, String> {
        use crate::clip::ResolvedContent;

        let clip = document
            .get_audio_clip(&instance.clip_id)
            .ok_or_else(|| format!("Audio clip {} not found", instance.clip_id))?;

        let track_id = *self
            .layer_to_track_map
            .get(layer_id)
            .ok_or_else(|| format!("Layer {} not mapped to backend track", layer_id))?;

        let resolved = clip.resolve(instance.active_take);
        let content_duration = clip.content_duration().native();
        let internal_start = instance.trim_start;
        let internal_end = instance.trim_end.unwrap_or(content_duration);
        let start_time = instance.timeline_start;

        let controller = self
            .audio_controller
            .as_mut()
            .ok_or_else(|| "Audio controller not available".to_string())?;

        let backend_id = match resolved {
            ResolvedContent::Midi { midi_clip_id } => {
                use daw_backend::command::{Query, QueryResponse};

                // MIDI trims are in the BEATS domain, so the fallback span is beats too.
                let external_duration = instance
                    .timeline_duration
                    .unwrap_or(daw_backend::Beats(internal_end - internal_start));

                let midi_instance = daw_backend::MidiClipInstance::new(
                    0, // assigned by the backend
                    midi_clip_id,
                    daw_backend::Beats(internal_start),
                    daw_backend::Beats(internal_end),
                    start_time,
                    external_duration,
                );

                match controller
                    .send_query(Query::AddMidiClipInstanceSync(track_id, midi_instance))?
                {
                    QueryResponse::MidiClipInstanceAdded(Ok(id)) => BackendClipInstanceId::Midi(id),
                    QueryResponse::MidiClipInstanceAdded(Err(e)) => return Err(e),
                    _ => return Err("Unexpected query response".to_string()),
                }
            }
            ResolvedContent::Audio { audio_pool_index } => {
                // `trim_*` and the clip's content duration are SECONDS (audio content time); the
                // backend's start/duration are BEATS.
                //
                // When `timeline_duration` is set it's already beats; otherwise the clip occupies
                // its natural content length, so convert that seconds-span to beats *at the clip's
                // start* (NOT `internal_end - internal_start`, which is seconds — that was the
                // seconds-as-beats bug that made clips stop early at anything but 60 BPM).
                let effective_duration = instance.timeline_duration.unwrap_or_else(|| {
                    let tempo_map = document.tempo_map();
                    let content_secs = daw_backend::Seconds(internal_end - internal_start);
                    tempo_map.seconds_to_beats(tempo_map.beats_to_seconds(start_time) + content_secs)
                        - start_time
                });

                let id = controller.add_audio_clip(
                    track_id,
                    audio_pool_index,
                    start_time,
                    effective_duration,
                    daw_backend::Seconds(internal_start),
                );
                BackendClipInstanceId::Audio(id)
            }
            // Nothing to sync until it has content.
            ResolvedContent::Recording => return Ok(None),
        };

        self.clip_instance_to_backend_map
            .insert(instance.id, backend_id);

        Ok(Some((track_id, backend_id)))
    }

    /// Remove a clip instance's backend clip and drop it from the instance→backend map.
    pub fn remove_clip_instance(
        &mut self,
        track_id: daw_backend::TrackId,
        backend_id: BackendClipInstanceId,
        instance_id: Uuid,
    ) {
        if let Some(controller) = self.audio_controller.as_mut() {
            match backend_id {
                BackendClipInstanceId::Midi(id) => controller.remove_midi_clip(track_id, id),
                BackendClipInstanceId::Audio(id) => controller.remove_audio_clip(track_id, id),
            }
        }
        self.clip_instance_to_backend_map.remove(&instance_id);
    }
}

/// Action trait for undo/redo operations
///
/// Each action must be able to execute (apply changes) and rollback (undo changes).
/// Actions are stored in the undo stack and can be re-executed from the redo stack.
///
/// ## Backend Integration
///
/// Actions can optionally implement backend synchronization via `execute_backend()`
/// and `rollback_backend()`. Default implementations do nothing, so actions that
/// only affect the document (vector graphics) don't need to implement these.
pub trait Action: Send {
    /// Apply this action to the document
    ///
    /// Returns Ok(()) if successful, or Err(message) if the action cannot be performed
    fn execute(&mut self, document: &mut Document) -> Result<(), String>;

    /// Undo this action (rollback changes)
    ///
    /// Returns Ok(()) if successful, or Err(message) if rollback fails
    fn rollback(&mut self, document: &mut Document) -> Result<(), String>;

    /// Get a human-readable description of this action (for UI display)
    fn description(&self) -> String;

    /// For raster actions that store dirty-rect diffs: the `(layer_id, time)` of the
    /// keyframe whose full pixels must be resident before `execute`/`rollback` can
    /// apply the diff. The editor faults the frame in (synchronously) before undo/redo
    /// so a paged-out clean frame is restored to its container state first. Non-raster
    /// actions (and full-buffer ones) return `None`.
    fn raster_resident_hint(&self) -> Option<(Uuid, f64)> {
        None
    }

    /// Execute backend operations after document changes
    ///
    /// Called AFTER execute() succeeds. If this returns an error, execute()
    /// will be automatically rolled back to maintain atomicity.
    ///
    /// # Arguments
    /// * `backend` - Backend context with audio/video controllers
    /// * `document` - Read-only document access for looking up clip data
    ///
    /// Default: No backend operations
    fn execute_backend(&mut self, _backend: &mut BackendContext, _document: &Document) -> Result<(), String> {
        Ok(())
    }

    /// Rollback backend operations during undo
    ///
    /// Called BEFORE rollback() to undo backend changes in reverse order.
    ///
    /// # Arguments
    /// * `backend` - Backend context with audio/video controllers
    /// * `document` - Read-only document access (if needed)
    ///
    /// Default: No backend operations
    fn rollback_backend(&mut self, _backend: &mut BackendContext, _document: &Document) -> Result<(), String> {
        Ok(())
    }

    /// Return MIDI cache data reflecting the state after execute/redo.
    /// Format: (clip_id, notes) where notes are (start_time, note, velocity, duration).
    /// Used to keep the frontend MIDI event cache in sync after undo/redo.
    fn midi_notes_after_execute(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        None
    }

    /// Return MIDI cache data reflecting the state after rollback/undo.
    fn midi_notes_after_rollback(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        None
    }

    /// Return full MIDI event data (CC, pitch bend, etc.) reflecting the state after execute/redo.
    /// Used to keep the frontend MIDI event cache in sync after undo/redo.
    fn midi_events_after_execute(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        None
    }

    /// Return full MIDI event data reflecting the state after rollback/undo.
    fn midi_events_after_rollback(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        None
    }

    /// Return MIDI event data for multiple clips after execute/redo (e.g. BPM change).
    /// Each element is (midi_clip_id, events). Default: empty.
    fn all_midi_events_after_execute(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        Vec::new()
    }

    /// Return MIDI event data for multiple clips after rollback/undo.
    fn all_midi_events_after_rollback(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        Vec::new()
    }
}

/// Action executor that wraps the document and manages undo/redo
///
/// This is the only way to get mutable access to the document, ensuring
/// all mutations go through the action system.
///
/// The document is stored in `Arc<Document>` for efficient sharing with
/// render callbacks. Use `document_arc()` for cheap cloning to GPU passes.
pub struct ActionExecutor {
    /// The document being edited (wrapped in Arc for cheap cloning)
    document: Arc<Document>,

    /// Stack of executed actions (for undo)
    undo_stack: Vec<Box<dyn Action>>,

    /// Stack of undone actions (for redo)
    redo_stack: Vec<Box<dyn Action>>,

    /// Maximum number of actions to keep in undo stack
    max_undo_depth: usize,

    /// Monotonically increasing counter, bumped on every `execute` call.
    /// Used to detect whether any actions were taken during a region selection.
    epoch: u64,
}

impl ActionExecutor {
    /// Create a new action executor with the given document
    pub fn new(mut document: Document) -> Self {
        // Rebuild transient lookup maps (not serialized)
        document.rebuild_layer_to_clip_map();
        Self {
            document: Arc::new(document),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_undo_depth: 100, // Default: keep last 100 actions
            epoch: 0,
        }
    }

    /// Get read-only access to the document
    ///
    /// This is the public API for reading document state.
    /// Mutations must go through execute() which requires an Action.
    pub fn document(&self) -> &Document {
        &self.document
    }

    /// Get a cheap clone of the document Arc for render callbacks
    ///
    /// Use this when passing the document to GPU render passes or other
    /// contexts that need to own a reference. Cloning Arc is just a pointer
    /// copy + atomic increment, not a deep clone.
    pub fn document_arc(&self) -> Arc<Document> {
        Arc::clone(&self.document)
    }

    /// Get mutable access to the document
    ///
    /// Uses copy-on-write semantics: if other Arc holders exist (e.g., in-flight
    /// render callbacks), the document is cloned before mutation. Otherwise,
    /// returns direct mutable access.
    ///
    /// Note: This should only be used for live previews. Permanent changes
    /// should go through the execute() method to support undo/redo.
    pub fn document_mut(&mut self) -> &mut Document {
        Arc::make_mut(&mut self.document)
    }

    /// Execute an action and add it to the undo stack
    ///
    /// This clears the redo stack since we're creating a new timeline branch.
    ///
    /// Returns Ok(()) if successful, or Err(message) if the action failed
    pub fn execute(&mut self, mut action: Box<dyn Action>) -> Result<(), String> {
        // Apply the action (uses copy-on-write if other Arc holders exist)
        action.execute(Arc::make_mut(&mut self.document))?;

        // Clear redo stack (new action invalidates redo history)
        self.redo_stack.clear();

        // Bump epoch so region selections can detect that an action occurred
        self.epoch = self.epoch.wrapping_add(1);

        // Add to undo stack
        self.undo_stack.push(action);

        // Limit undo stack size
        if self.undo_stack.len() > self.max_undo_depth {
            self.undo_stack.remove(0);
        }

        Ok(())
    }

    /// Register an action whose effect has **already been applied** to the document (and backend)
    /// outside the executor — e.g. a recording, which streams its content into the document live
    /// over time and can't be applied by a single synchronous `execute()`.
    ///
    /// Unlike `execute`, this does NOT run `execute()`/`execute_backend()` (the effect is already
    /// present). It clears the redo stack, bumps the epoch (so the document reads as modified), and
    /// pushes the action so it becomes undoable: undo runs `rollback`/`rollback_backend` to remove
    /// the content, redo runs `execute`/`execute_backend` to bring it back. The action must be
    /// constructed already in its post-execute state (see e.g. `AddClipInstanceAction::already_applied`).
    pub fn push_applied(&mut self, action: Box<dyn Action>) {
        self.redo_stack.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.undo_stack.push(action);
        if self.undo_stack.len() > self.max_undo_depth {
            self.undo_stack.remove(0);
        }
    }

    /// Undo the last action
    ///
    /// Returns Ok(true) if an action was undone, Ok(false) if undo stack is empty,
    /// or Err(message) if rollback failed
    pub fn undo(&mut self) -> Result<bool, String> {
        if let Some(mut action) = self.undo_stack.pop() {
            // Rollback the action (uses copy-on-write if other Arc holders exist)
            match action.rollback(Arc::make_mut(&mut self.document)) {
                Ok(()) => {
                    // Move to redo stack
                    self.redo_stack.push(action);
                    self.epoch = self.epoch.wrapping_add(1);
                    Ok(true)
                }
                Err(e) => {
                    // Put action back on undo stack if rollback failed
                    self.undo_stack.push(action);
                    Err(e)
                }
            }
        } else {
            Ok(false)
        }
    }

    /// Redo the last undone action
    ///
    /// Returns Ok(true) if an action was redone, Ok(false) if redo stack is empty,
    /// or Err(message) if re-execution failed
    pub fn redo(&mut self) -> Result<bool, String> {
        if let Some(mut action) = self.redo_stack.pop() {
            // Re-execute the action (uses copy-on-write if other Arc holders exist)
            match action.execute(Arc::make_mut(&mut self.document)) {
                Ok(()) => {
                    // Move back to undo stack
                    self.undo_stack.push(action);
                    self.epoch = self.epoch.wrapping_add(1);
                    Ok(true)
                }
                Err(e) => {
                    // Put action back on redo stack if re-execution failed
                    self.redo_stack.push(action);
                    Err(e)
                }
            }
        } else {
            Ok(false)
        }
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Check if redo is available
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Get the description of the next action to undo
    pub fn undo_description(&self) -> Option<String> {
        self.undo_stack.last().map(|a| a.description())
    }

    /// `(layer_id, time)` of the raster keyframe the next undo needs resident, if any.
    /// The editor faults it in before calling `undo()` so a paged-out clean frame is
    /// restored to its container state, giving the diff a correct base to apply onto.
    pub fn peek_undo_raster_hint(&self) -> Option<(Uuid, f64)> {
        self.undo_stack.last().and_then(|a| a.raster_resident_hint())
    }

    /// `(layer_id, time)` of the raster keyframe the next redo needs resident, if any.
    pub fn peek_redo_raster_hint(&self) -> Option<(Uuid, f64)> {
        self.redo_stack.last().and_then(|a| a.raster_resident_hint())
    }

    /// Get MIDI cache data from the last action on the undo stack (after redo).
    /// Returns the notes reflecting execute state.
    pub fn last_undo_midi_notes(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        self.undo_stack.last().and_then(|a| a.midi_notes_after_execute())
    }

    /// Get MIDI cache data from the last action on the redo stack (after undo).
    /// Returns the notes reflecting rollback state.
    pub fn last_redo_midi_notes(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        self.redo_stack.last().and_then(|a| a.midi_notes_after_rollback())
    }

    /// Get full MIDI event data from the last action on the undo stack (after redo).
    pub fn last_undo_midi_events(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        self.undo_stack.last().and_then(|a| a.midi_events_after_execute())
    }

    /// Get full MIDI event data from the last action on the redo stack (after undo).
    pub fn last_redo_midi_events(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        self.redo_stack.last().and_then(|a| a.midi_events_after_rollback())
    }

    /// Get multi-clip MIDI event data from the last undo stack action (after redo).
    pub fn last_undo_all_midi_events(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        self.undo_stack.last().map(|a| a.all_midi_events_after_execute()).unwrap_or_default()
    }

    /// Get multi-clip MIDI event data from the last redo stack action (after undo).
    pub fn last_redo_all_midi_events(&self) -> Vec<(u32, Vec<daw_backend::audio::midi::MidiEvent>)> {
        self.redo_stack.last().map(|a| a.all_midi_events_after_rollback()).unwrap_or_default()
    }

    /// Get the description of the next action to redo
    pub fn redo_description(&self) -> Option<String> {
        self.redo_stack.last().map(|a| a.description())
    }

    /// Get the number of actions in the undo stack
    pub fn undo_depth(&self) -> usize {
        self.undo_stack.len()
    }

    /// Get the number of actions in the redo stack
    pub fn redo_depth(&self) -> usize {
        self.redo_stack.len()
    }

    /// Return the current action epoch.
    ///
    /// The epoch is a monotonically increasing counter that is bumped every
    /// time `execute` is called. It is never decremented on undo/redo, so
    /// callers can record it at a point in time and later compare to detect
    /// whether any action was executed in the interim.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Clear all undo/redo history
    pub fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Set the maximum undo depth
    pub fn set_max_undo_depth(&mut self, depth: usize) {
        self.max_undo_depth = depth;

        // Trim undo stack if needed
        if self.undo_stack.len() > depth {
            let remove_count = self.undo_stack.len() - depth;
            self.undo_stack.drain(0..remove_count);
        }
    }

    /// Execute an action with backend synchronization
    ///
    /// This performs atomic execution: if backend operations fail, the document
    /// changes are automatically rolled back to maintain consistency.
    ///
    /// # Arguments
    /// * `action` - The action to execute
    /// * `backend` - Backend context for audio/video operations
    ///
    /// # Returns
    /// * `Ok(())` if both document and backend operations succeeded
    /// * `Err(msg)` if backend failed (document changes are rolled back)
    pub fn execute_with_backend(
        &mut self,
        mut action: Box<dyn Action>,
        backend: &mut BackendContext,
    ) -> Result<(), String> {
        // 1. Execute document changes
        action.execute(Arc::make_mut(&mut self.document))?;

        // 2. Execute backend changes (pass document for reading clip data)
        if let Err(e) = action.execute_backend(backend, &self.document) {
            // ATOMIC ROLLBACK: Backend failed → undo document
            action.rollback(Arc::make_mut(&mut self.document))?;
            return Err(e);
        }

        // 3. Push to undo stack (both succeeded)
        self.redo_stack.clear();
        self.epoch = self.epoch.wrapping_add(1);
        self.undo_stack.push(action);

        // Limit undo stack size
        if self.undo_stack.len() > self.max_undo_depth {
            self.undo_stack.remove(0);
        }

        Ok(())
    }

    /// Undo the last action with backend synchronization
    ///
    /// Rollback happens in reverse order: backend first, then document.
    ///
    /// # Arguments
    /// * `backend` - Backend context for audio/video operations
    ///
    /// # Returns
    /// * `Ok(true)` if an action was undone
    /// * `Ok(false)` if undo stack is empty
    /// * `Err(msg)` if backend rollback failed
    pub fn undo_with_backend(&mut self, backend: &mut BackendContext) -> Result<bool, String> {
        if let Some(mut action) = self.undo_stack.pop() {
            // Rollback in REVERSE order: backend first, then document
            action.rollback_backend(backend, &self.document)?;
            action.rollback(Arc::make_mut(&mut self.document))?;

            // Move to redo stack
            self.redo_stack.push(action);
            self.epoch = self.epoch.wrapping_add(1);

            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Redo the last undone action with backend synchronization
    ///
    /// Re-execution happens in normal order: document first, then backend.
    ///
    /// # Arguments
    /// * `backend` - Backend context for audio/video operations
    ///
    /// # Returns
    /// * `Ok(true)` if an action was redone
    /// * `Ok(false)` if redo stack is empty
    /// * `Err(msg)` if backend execution failed
    pub fn redo_with_backend(&mut self, backend: &mut BackendContext) -> Result<bool, String> {
        if let Some(mut action) = self.redo_stack.pop() {
            // Re-execute in same order: document first, then backend
            if let Err(e) = action.execute(Arc::make_mut(&mut self.document)) {
                // Put action back on redo stack if document execute fails
                self.redo_stack.push(action);
                return Err(e);
            }

            if let Err(e) = action.execute_backend(backend, &self.document) {
                // Rollback document if backend fails
                action.rollback(Arc::make_mut(&mut self.document))?;
                // Put action back on redo stack
                self.redo_stack.push(action);
                return Err(e);
            }

            // Move back to undo stack
            self.undo_stack.push(action);
            self.epoch = self.epoch.wrapping_add(1);

            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test action that just tracks execute/rollback calls
    struct TestAction {
        description: String,
        executed: bool,
    }

    impl TestAction {
        fn new(description: &str) -> Self {
            Self {
                description: description.to_string(),
                executed: false,
            }
        }
    }

    impl Action for TestAction {
        fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
            self.executed = true;
            Ok(())
        }

        fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
            self.executed = false;
            Ok(())
        }

        fn description(&self) -> String {
            self.description.clone()
        }
    }

    #[test]
    fn test_action_executor_basic() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);

        assert!(!executor.can_undo());
        assert!(!executor.can_redo());

        // Execute an action
        let action = Box::new(TestAction::new("Test Action"));
        executor.execute(action).unwrap();

        assert!(executor.can_undo());
        assert!(!executor.can_redo());
        assert_eq!(executor.undo_depth(), 1);

        // Undo
        assert!(executor.undo().unwrap());
        assert!(!executor.can_undo());
        assert!(executor.can_redo());
        assert_eq!(executor.redo_depth(), 1);

        // Redo
        assert!(executor.redo().unwrap());
        assert!(executor.can_undo());
        assert!(!executor.can_redo());
    }

    #[test]
    fn test_action_descriptions() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);

        executor.execute(Box::new(TestAction::new("Action 1"))).unwrap();
        executor.execute(Box::new(TestAction::new("Action 2"))).unwrap();

        assert_eq!(executor.undo_description(), Some("Action 2".to_string()));

        executor.undo().unwrap();
        assert_eq!(executor.redo_description(), Some("Action 2".to_string()));
        assert_eq!(executor.undo_description(), Some("Action 1".to_string()));
    }

    #[test]
    fn test_new_action_clears_redo() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);

        executor.execute(Box::new(TestAction::new("Action 1"))).unwrap();
        executor.execute(Box::new(TestAction::new("Action 2"))).unwrap();
        executor.undo().unwrap();

        assert!(executor.can_redo());

        // Execute new action should clear redo stack
        executor.execute(Box::new(TestAction::new("Action 3"))).unwrap();

        assert!(!executor.can_redo());
        assert_eq!(executor.undo_depth(), 2);
    }

    #[test]
    fn test_max_undo_depth() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);
        executor.set_max_undo_depth(3);

        executor.execute(Box::new(TestAction::new("Action 1"))).unwrap();
        executor.execute(Box::new(TestAction::new("Action 2"))).unwrap();
        executor.execute(Box::new(TestAction::new("Action 3"))).unwrap();
        executor.execute(Box::new(TestAction::new("Action 4"))).unwrap();

        // Should only keep last 3
        assert_eq!(executor.undo_depth(), 3);
        assert_eq!(executor.undo_description(), Some("Action 4".to_string()));
    }
}
