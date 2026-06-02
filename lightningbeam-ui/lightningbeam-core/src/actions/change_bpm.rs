//! Change BPM / Tempo action
//!
//! Atomically swaps the document `TempoMap`, sending the new map to the engine.
//! All clip and note positions are stored in **beats** so no position rescaling
//! is required — only the TempoMap entry changes.

use crate::action::{Action, BackendContext};
use crate::document::Document;
use crate::tempo_map::TempoMap;

/// Action that changes the global BPM by replacing the tempo map.
#[derive(Clone)]
pub struct ChangeBpmAction {
    old_map: TempoMap,
    new_map: TempoMap,
}

impl ChangeBpmAction {
    /// Build the action from the current document state and a desired new BPM.
    pub fn new(new_bpm: f64, document: &Document) -> Self {
        let old_map = document.tempo_map().clone();
        let mut new_map = old_map.clone();
        new_map.set_global_bpm(new_bpm);
        Self { old_map, new_map }
    }

    /// Build from explicit old/new maps (used when the map already changed in-place,
    /// e.g., after live drag preview; the caller provides start and end states).
    pub fn from_maps(old_map: TempoMap, new_map: TempoMap) -> Self {
        Self { old_map, new_map }
    }

    pub fn new_bpm(&self) -> f64 { self.new_map.global_bpm() }
    pub fn old_bpm(&self) -> f64 { self.old_map.global_bpm() }
}

impl Action for ChangeBpmAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        *document.tempo_map_mut() = self.new_map.clone();
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        *document.tempo_map_mut() = self.old_map.clone();
        Ok(())
    }

    fn description(&self) -> String {
        "Change BPM".to_string()
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        let controller = match backend.audio_controller.as_mut() {
            Some(c) => c,
            None => return Ok(()),
        };
        controller.set_tempo_map(self.new_map.clone());
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
        controller.set_tempo_map(self.old_map.clone());
        Ok(())
    }
}
