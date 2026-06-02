use crate::action::Action;
use crate::document::Document;
use uuid::Uuid;

/// Action to replace all MIDI events in a clip (CC, pitch bend, notes, etc.) with undo/redo.
///
/// Used when editing per-note CC or pitch bend from the piano roll. Stores full
/// `MidiEvent` lists rather than the simplified note-tuple format of `UpdateMidiNotesAction`.
pub struct UpdateMidiEventsAction {
    /// Layer containing the MIDI clip
    pub layer_id: Uuid,
    /// Backend MIDI clip ID
    pub midi_clip_id: u32,
    /// Full event list before the edit
    pub old_events: Vec<daw_backend::audio::midi::MidiEvent>,
    /// Full event list after the edit
    pub new_events: Vec<daw_backend::audio::midi::MidiEvent>,
    /// Human-readable description
    pub description_text: String,
}

impl Action for UpdateMidiEventsAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
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
        controller.update_midi_clip_events(*track_id, self.midi_clip_id, self.new_events.clone());
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
        controller.update_midi_clip_events(*track_id, self.midi_clip_id, self.old_events.clone());
        Ok(())
    }

    fn midi_events_after_execute(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        Some((self.midi_clip_id, &self.new_events))
    }

    fn midi_events_after_rollback(&self) -> Option<(u32, &[daw_backend::audio::midi::MidiEvent])> {
        Some((self.midi_clip_id, &self.old_events))
    }
}
