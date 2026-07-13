//! Choose which take of a take-folder clip an instance plays.
//!
//! The take list lives on the *clip* but the selection lives on the *instance*, so two instances of
//! the same folder can play different takes. Splitting a take-folder instance clones it, which is
//! what makes comping work: set take 1 on the left half and take 3 on the right, and you've comped.
//!
//! There's no in-place pool-swap command in the backend, so switching a take means removing the
//! instance's backend clip and re-adding it against the new take's audio/MIDI resource. Both halves
//! of that go through `BackendContext`, which is also what `AddClipInstanceAction` uses.

use crate::action::{Action, BackendClipInstanceId, BackendContext};
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that points a clip instance at a different take of its take folder.
#[derive(Clone)]
pub struct SetActiveTakeAction {
    layer_id: Uuid,
    instance_id: Uuid,
    new_take: Option<usize>,
    old_take: Option<usize>,
    /// The backend track/clip the instance was on before we swapped, so rollback can undo it.
    backend_track_id: Option<daw_backend::TrackId>,
}

impl SetActiveTakeAction {
    pub fn new(layer_id: Uuid, instance_id: Uuid, new_take: usize, old_take: Option<usize>) -> Self {
        Self {
            layer_id,
            instance_id,
            new_take: Some(new_take),
            old_take,
            backend_track_id: None,
        }
    }

    /// Point the instance at `take`, mutating the document. Shared by execute and rollback.
    fn apply(&self, document: &mut Document, take: Option<usize>) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let AnyLayer::Audio(audio_layer) = layer else {
            return Err("Take folders only exist on audio layers".to_string());
        };
        let instance = audio_layer
            .clip_instances
            .iter_mut()
            .find(|ci| ci.id == self.instance_id)
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;
        instance.active_take = take;
        Ok(())
    }

    /// Swap the instance's backend clip to whatever take the document now says is active.
    ///
    /// Called after the document has already been mutated, so re-adding just re-resolves the
    /// instance — `BackendContext::add_clip_instance` reads `active_take` itself.
    fn resync(&mut self, backend: &mut BackendContext, document: &Document) -> Result<(), String> {
        let instance = document
            .get_layer(&self.layer_id)
            .and_then(|l| match l {
                AnyLayer::Audio(al) => al.clip_instances.iter().find(|ci| ci.id == self.instance_id),
                _ => None,
            })
            .cloned()
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;

        // Drop the old backend clip first. Its track comes from the map we're about to overwrite,
        // so read it before add_clip_instance replaces the entry.
        let existing: Option<BackendClipInstanceId> = backend
            .clip_instance_to_backend_map
            .get(&self.instance_id)
            .copied();
        let track_id = backend.layer_to_track_map.get(&self.layer_id).copied();
        if let (Some(backend_id), Some(track_id)) = (existing, track_id) {
            backend.remove_clip_instance(track_id, backend_id, self.instance_id);
        }

        let added = backend.add_clip_instance(document, &self.layer_id, &instance)?;
        self.backend_track_id = added.map(|(track_id, _)| track_id);
        Ok(())
    }
}

impl Action for SetActiveTakeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        self.apply(document, self.new_take)
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        self.apply(document, self.old_take)
    }

    fn description(&self) -> String {
        match self.new_take {
            Some(i) => format!("Select take {}", i + 1),
            None => "Select take".to_string(),
        }
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        self.resync(backend, document)
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        self.resync(backend, document)
    }
}
