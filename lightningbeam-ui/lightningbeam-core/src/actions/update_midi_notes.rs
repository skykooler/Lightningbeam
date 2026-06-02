use crate::action::Action;
use crate::document::Document;
use uuid::Uuid;

/// Action to update MIDI notes in a clip (supports undo/redo)
///
/// Stores the before and after note states. MIDI note data lives in the backend,
/// so execute/rollback are no-ops on the document — all changes go through
/// execute_backend/rollback_backend.
pub struct UpdateMidiNotesAction {
    /// Layer containing the MIDI clip
    pub layer_id: Uuid,
    /// Backend MIDI clip ID
    pub midi_clip_id: u32,
    /// Notes before the edit: (start_time, note, velocity, duration)
    pub old_notes: Vec<(f64, u8, u8, f64)>,
    /// Notes after the edit: (start_time, note, velocity, duration)
    pub new_notes: Vec<(f64, u8, u8, f64)>,
    /// Human-readable description
    pub description_text: String,
}

impl Action for UpdateMidiNotesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        // MIDI note data lives in the backend, not the document
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        self.description_text.clone()
    }

    fn execute_backend(
        &mut self,
        backend: &mut crate::action::BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not mapped to backend track", self.layer_id))?;

        controller.update_midi_clip_notes(*track_id, self.midi_clip_id, self.new_notes.clone());
        Ok(())
    }

    fn rollback_backend(
        &mut self,
        backend: &mut crate::action::BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };

        let track_id = backend
            .layer_to_track_map
            .get(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not mapped to backend track", self.layer_id))?;

        controller.update_midi_clip_notes(*track_id, self.midi_clip_id, self.old_notes.clone());
        Ok(())
    }

    fn midi_notes_after_execute(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        Some((self.midi_clip_id, &self.new_notes))
    }

    fn midi_notes_after_rollback(&self) -> Option<(u32, &[(f64, u8, u8, f64)])> {
        Some((self.midi_clip_id, &self.old_notes))
    }
}
