//! Append freshly-recorded takes to an existing take folder.
//!
//! Cycle-recording over a region that already holds a take folder should *add* to that folder, not
//! drop a second clip on top of it. Otherwise the takes from your second attempt are stranded in a
//! separate, overlapping clip and you can't audition them against the first.
//!
//! The recorded content already exists in the backend pools by the time this runs (the engine put it
//! there at stop), so this action only touches the document — plus the one backend clip the instance
//! plays, which has to be repointed at the newly-active take.

use crate::action::{Action, BackendClipInstanceId, BackendContext};
use crate::clip::AudioTake;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that appends takes to an instance's take list and selects the last of them.
pub struct AppendTakesAction {
    layer_id: Uuid,
    /// The instance whose take list is being extended (and whose active take changes).
    instance_id: Uuid,
    /// The takes to add, in recording order.
    new_takes: Vec<AudioTake>,

    // Stored during execute for rollback.
    old_take_count: usize,
    old_active_take: Option<usize>,
    executed: bool,
}

impl AppendTakesAction {
    pub fn new(layer_id: Uuid, instance_id: Uuid, new_takes: Vec<AudioTake>) -> Self {
        Self {
            layer_id,
            instance_id,
            new_takes,
            old_take_count: 0,
            old_active_take: None,
            executed: false,
        }
    }

    /// Swap the instance's backend clip to whatever take the document now says is active.
    ///
    /// Same remove + re-add as `SetActiveTakeAction` — there's no in-place pool-swap command.
    fn resync(&self, backend: &mut BackendContext, document: &Document) -> Result<(), String> {
        let instance = document
            .get_layer(&self.layer_id)
            .and_then(|l| match l {
                AnyLayer::Audio(al) => al.clip_instances.iter().find(|ci| ci.id == self.instance_id),
                _ => None,
            })
            .cloned()
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;

        let existing: Option<BackendClipInstanceId> = backend
            .clip_instance_to_backend_map
            .get(&self.instance_id)
            .copied();
        let track_id = backend.layer_to_track_map.get(&self.layer_id).copied();
        if let (Some(backend_id), Some(track_id)) = (existing, track_id) {
            backend.remove_clip_instance(track_id, backend_id, self.instance_id);
        }

        backend.add_clip_instance(document, &self.layer_id, &instance)?;
        Ok(())
    }
}

impl Action for AppendTakesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let AnyLayer::Audio(audio_layer) = layer else {
            return Err("Takes only exist on audio layers".to_string());
        };
        let instance = audio_layer
            .clip_instances
            .iter_mut()
            .find(|ci| ci.id == self.instance_id)
            .ok_or_else(|| format!("Clip instance {} not found", self.instance_id))?;

        // Only record the pre-state on the first execute; a redo must not overwrite it with the
        // post-state left behind by the previous run.
        if !self.executed {
            self.old_take_count = instance.takes.len();
            self.old_active_take = instance.active_take;
        }

        // Number the new takes on from what's already there. Existing names are left alone — the
        // user may well have renamed them, and renumbering would clobber that.
        let base = instance.takes.len();
        for (i, take) in self.new_takes.iter().enumerate() {
            let mut take = take.clone();
            if take.name.is_empty() {
                take.name = format!("Take {}", base + i + 1);
            }
            instance.takes.push(take);
        }

        // Land on the take just recorded, GarageBand-style.
        instance.active_take = Some(instance.takes.len() - 1);
        self.executed = true;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(AnyLayer::Audio(audio_layer)) = document.get_layer_mut(&self.layer_id) {
            if let Some(instance) = audio_layer
                .clip_instances
                .iter_mut()
                .find(|ci| ci.id == self.instance_id)
            {
                instance.takes.truncate(self.old_take_count);
                instance.active_take = self.old_active_take;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Record {} take(s)", self.new_takes.len())
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
