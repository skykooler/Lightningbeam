//! Take management: delete and rename the takes on a clip instance.
//!
//! Takes live on the INSTANCE, so both of these are naturally scoped to the one the user clicked —
//! deleting a take from one half of a comped split leaves the other half's list alone.

use crate::action::{Action, BackendClipInstanceId, BackendContext};
use crate::clip::{AudioTake, ClipInstance};
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// The instance a take action targets, looked up mutably.
fn instance_mut<'a>(
    document: &'a mut Document,
    layer_id: &Uuid,
    instance_id: &Uuid,
) -> Result<&'a mut ClipInstance, String> {
    let layer = document
        .get_layer_mut(layer_id)
        .ok_or_else(|| format!("Layer {} not found", layer_id))?;
    let AnyLayer::Audio(audio_layer) = layer else {
        return Err("Takes only exist on audio layers".to_string());
    };
    audio_layer
        .clip_instances
        .iter_mut()
        .find(|ci| ci.id == *instance_id)
        .ok_or_else(|| format!("Clip instance {} not found", instance_id))
}

/// Swap an instance's backend clip to whatever take the document now says is active.
///
/// The same remove + re-add as `SetActiveTakeAction` — there's no in-place pool-swap command.
fn resync(
    backend: &mut BackendContext,
    document: &Document,
    layer_id: &Uuid,
    instance_id: &Uuid,
) -> Result<(), String> {
    let instance = document
        .get_layer(layer_id)
        .and_then(|l| match l {
            AnyLayer::Audio(al) => al.clip_instances.iter().find(|ci| ci.id == *instance_id),
            _ => None,
        })
        .cloned()
        .ok_or_else(|| format!("Clip instance {} not found", instance_id))?;

    let existing: Option<BackendClipInstanceId> = backend
        .clip_instance_to_backend_map
        .get(instance_id)
        .copied();
    let track_id = backend.layer_to_track_map.get(layer_id).copied();
    if let (Some(backend_id), Some(track_id)) = (existing, track_id) {
        backend.remove_clip_instance(track_id, backend_id, *instance_id);
    }

    backend.add_clip_instance(document, layer_id, &instance)?;
    Ok(())
}

/// Remove a take from an instance's take list.
///
/// The take's recorded audio/MIDI stays in the backend pool — undo has to be able to put it back,
/// and other instances (the other half of a split, say) may still be playing it.
pub struct DeleteTakeAction {
    layer_id: Uuid,
    instance_id: Uuid,
    take_index: usize,

    // Stored during execute for rollback.
    removed: Option<AudioTake>,
    old_active_take: Option<usize>,
}

impl DeleteTakeAction {
    pub fn new(layer_id: Uuid, instance_id: Uuid, take_index: usize) -> Self {
        Self {
            layer_id,
            instance_id,
            take_index,
            removed: None,
            old_active_take: None,
        }
    }
}

impl Action for DeleteTakeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;

        if self.take_index >= instance.takes.len() {
            return Err(format!("Take {} does not exist", self.take_index + 1));
        }
        // An instance with no takes at all would fall back to the clip's own content, which for a
        // cycle recording is a take we may just have deleted. Refuse rather than strand it.
        if instance.takes.len() == 1 {
            return Err("Can't delete the only take".to_string());
        }

        self.old_active_take = instance.active_take;
        self.removed = Some(instance.takes.remove(self.take_index));

        // Everything above the removed take shifts down one, so the selection has to move with it.
        // Deleting the *active* take lands on the one that took its place (or the new last take, if
        // it was at the end) — that keeps the clip sounding rather than silently picking take 1.
        let active = instance.active_take.unwrap_or(0);
        instance.active_take = Some(if active > self.take_index {
            active - 1
        } else if active == self.take_index {
            self.take_index.min(instance.takes.len() - 1)
        } else {
            active
        });

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(take) = self.removed.take() else {
            return Ok(());
        };
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;
        let at = self.take_index.min(instance.takes.len());
        instance.takes.insert(at, take);
        instance.active_take = self.old_active_take;
        Ok(())
    }

    fn description(&self) -> String {
        format!("Delete take {}", self.take_index + 1)
    }

    fn execute_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        resync(backend, document, &self.layer_id, &self.instance_id)
    }

    fn rollback_backend(
        &mut self,
        backend: &mut BackendContext,
        document: &Document,
    ) -> Result<(), String> {
        resync(backend, document, &self.layer_id, &self.instance_id)
    }
}

/// Throw away every take except the one that's playing.
///
/// The tidy-up once you've picked your keeper. Scoped to this instance, so on a comped split it
/// prunes the half you clicked and leaves the other half's alternatives intact.
pub struct DeleteUnusedTakesAction {
    layer_id: Uuid,
    instance_id: Uuid,

    // Stored during execute for rollback.
    old_takes: Vec<AudioTake>,
    old_active_take: Option<usize>,
}

impl DeleteUnusedTakesAction {
    pub fn new(layer_id: Uuid, instance_id: Uuid) -> Self {
        Self {
            layer_id,
            instance_id,
            old_takes: Vec::new(),
            old_active_take: None,
        }
    }
}

impl Action for DeleteUnusedTakesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;
        if instance.takes.len() < 2 {
            return Err("Nothing to delete".to_string());
        }

        let keep = instance.active_take_index();
        self.old_takes = instance.takes.clone();
        self.old_active_take = instance.active_take;

        let kept = instance.takes.remove(keep);
        instance.takes.clear();
        instance.takes.push(kept);
        instance.active_take = Some(0);
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;
        instance.takes = std::mem::take(&mut self.old_takes);
        instance.active_take = self.old_active_take;
        Ok(())
    }

    fn description(&self) -> String {
        "Delete unused takes".to_string()
    }

    // The take that plays doesn't change, so the backend clip is already correct.
}

/// Rename a take. Document-only — which take *plays* doesn't change, so the backend is untouched.
pub struct RenameTakeAction {
    layer_id: Uuid,
    instance_id: Uuid,
    take_index: usize,
    new_name: String,
    old_name: String,
}

impl RenameTakeAction {
    pub fn new(layer_id: Uuid, instance_id: Uuid, take_index: usize, new_name: String) -> Self {
        Self {
            layer_id,
            instance_id,
            take_index,
            new_name,
            old_name: String::new(),
        }
    }
}

impl Action for RenameTakeAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;
        let take = instance
            .takes
            .get_mut(self.take_index)
            .ok_or_else(|| format!("Take {} does not exist", self.take_index + 1))?;
        self.old_name = std::mem::replace(&mut take.name, self.new_name.clone());
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let instance = instance_mut(document, &self.layer_id, &self.instance_id)?;
        if let Some(take) = instance.takes.get_mut(self.take_index) {
            take.name = self.old_name.clone();
        }
        Ok(())
    }

    fn description(&self) -> String {
        format!("Rename take to \"{}\"", self.new_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::TakeContent;
    use crate::layer::AudioLayer;

    /// A document with one audio layer holding one instance with 4 takes (pools 10..13).
    fn doc_with_takes() -> (Document, Uuid, Uuid) {
        let mut document = Document::new("Test");
        let clip = crate::clip::AudioClip::new_sampled("Cycle rec", 10, 2.0);
        let clip_id = document.add_audio_clip(clip);

        let mut instance = ClipInstance::new(clip_id);
        instance.takes = (10..14)
            .map(|pool| AudioTake {
                name: format!("Take {}", pool - 9),
                content: TakeContent::Audio { audio_pool_index: pool },
            })
            .collect();
        let instance_id = instance.id;

        let mut layer = AudioLayer::new("Layer");
        layer.clip_instances.push(instance);
        let layer_id = document.root.add_child(AnyLayer::Audio(layer));
        (document, layer_id, instance_id)
    }

    fn takes_of(document: &Document, layer_id: &Uuid, instance_id: &Uuid) -> ClipInstance {
        let AnyLayer::Audio(al) = document.get_layer(layer_id).unwrap() else { panic!() };
        al.clip_instances.iter().find(|ci| ci.id == *instance_id).unwrap().clone()
    }

    #[test]
    fn deleting_a_take_below_the_active_one_shifts_the_selection_down() {
        // Everything above the removed take shifts down one, so a selection above it has to move
        // with it — otherwise the instance silently starts playing a different take.
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].active_take = Some(3); // playing pool 13
        }

        DeleteTakeAction::new(layer_id, instance_id, 1)
            .execute(&mut document)
            .expect("delete");

        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.takes.len(), 3);
        assert_eq!(inst.active_take, Some(2), "index shifted down with the take");
        assert_eq!(
            inst.takes[inst.active_take_index()].content,
            TakeContent::Audio { audio_pool_index: 13 },
            "still playing the same take it was",
        );
    }

    #[test]
    fn deleting_the_active_take_lands_on_its_replacement() {
        // Deleting what you're listening to should hand you the take that took its place, not
        // silently jump you back to take 1.
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].active_take = Some(1); // playing pool 11
        }

        DeleteTakeAction::new(layer_id, instance_id, 1)
            .execute(&mut document)
            .expect("delete");

        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.active_take, Some(1));
        assert_eq!(
            inst.takes[1].content,
            TakeContent::Audio { audio_pool_index: 12 },
            "the take that slid into the deleted one's place",
        );
    }

    #[test]
    fn deleting_the_last_take_in_the_list_steps_back() {
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].active_take = Some(3);
        }

        DeleteTakeAction::new(layer_id, instance_id, 3)
            .execute(&mut document)
            .expect("delete");

        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.active_take, Some(2), "there is no take 4 to land on");
    }

    #[test]
    fn the_only_take_cannot_be_deleted() {
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].takes.truncate(1);
        }
        assert!(DeleteTakeAction::new(layer_id, instance_id, 0)
            .execute(&mut document)
            .is_err());
    }

    #[test]
    fn undoing_a_delete_puts_the_take_back_where_it_was() {
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].active_take = Some(2);
        }

        let mut action = DeleteTakeAction::new(layer_id, instance_id, 1);
        action.execute(&mut document).expect("delete");
        action.rollback(&mut document).expect("undo");

        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.takes.len(), 4);
        assert_eq!(
            inst.takes[1].content,
            TakeContent::Audio { audio_pool_index: 11 },
            "restored at its original index",
        );
        assert_eq!(inst.active_take, Some(2), "and the selection with it");
    }

    #[test]
    fn deleting_unused_takes_keeps_the_one_thats_playing() {
        let (mut document, layer_id, instance_id) = doc_with_takes();
        {
            let AnyLayer::Audio(al) = document.get_layer_mut(&layer_id).unwrap() else { panic!() };
            al.clip_instances[0].active_take = Some(2); // pool 12 — the keeper
        }

        let mut action = DeleteUnusedTakesAction::new(layer_id, instance_id);
        action.execute(&mut document).expect("prune");

        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.takes.len(), 1);
        assert_eq!(inst.active_take, Some(0));
        assert_eq!(
            inst.takes[0].content,
            TakeContent::Audio { audio_pool_index: 12 },
            "the take that was playing survives, and nothing else",
        );

        action.rollback(&mut document).expect("undo");
        let inst = takes_of(&document, &layer_id, &instance_id);
        assert_eq!(inst.takes.len(), 4);
        assert_eq!(inst.active_take, Some(2), "back to what was playing before");
    }

    #[test]
    fn renaming_a_take_round_trips() {
        let (mut document, layer_id, instance_id) = doc_with_takes();
        let mut action =
            RenameTakeAction::new(layer_id, instance_id, 2, "The good one".to_string());

        action.execute(&mut document).expect("rename");
        assert_eq!(takes_of(&document, &layer_id, &instance_id).takes[2].name, "The good one");

        action.rollback(&mut document).expect("undo");
        assert_eq!(takes_of(&document, &layer_id, &instance_id).takes[2].name, "Take 3");
    }
}
