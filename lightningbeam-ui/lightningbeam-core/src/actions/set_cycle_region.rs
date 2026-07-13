//! Set the transport cycle (loop) region.
//!
//! The cycle region is document state (it's saved in the `.beam`), so changing it goes through the
//! action system like any other edit: it's undoable and it marks the document modified.
//!
//! The region is stored in **beats** so it stays put musically across tempo changes. Callers commit
//! one action per gesture (e.g. on drag release, or a toggle click) rather than one per frame —
//! the timeline previews the drag from its own local state, exactly like a clip drag does.

use crate::action::{Action, BackendContext};
use crate::document::Document;
use daw_backend::Beats;

/// Action that sets the cycle region and/or whether the transport loops over it.
#[derive(Clone)]
pub struct SetCycleRegionAction {
    old_region: Option<(Beats, Beats)>,
    old_enabled: bool,
    new_region: Option<(Beats, Beats)>,
    new_enabled: bool,
}

impl SetCycleRegionAction {
    /// Build from the document's current state and the desired new region/enabled flag.
    pub fn new(
        document: &Document,
        new_region: Option<(Beats, Beats)>,
        new_enabled: bool,
    ) -> Self {
        Self {
            old_region: document.cycle_region,
            old_enabled: document.cycle_enabled,
            new_region,
            new_enabled,
        }
    }

    /// Toggle looping on/off, leaving the region itself alone.
    pub fn toggle_enabled(document: &Document) -> Self {
        Self::new(document, document.cycle_region, !document.cycle_enabled)
    }

    /// True if this action would not actually change anything (lets callers skip a no-op undo entry).
    pub fn is_noop(&self) -> bool {
        self.old_region == self.new_region && self.old_enabled == self.new_enabled
    }
}

impl Action for SetCycleRegionAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        document.cycle_region = self.new_region;
        document.cycle_enabled = self.new_enabled;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        document.cycle_region = self.old_region;
        document.cycle_enabled = self.old_enabled;
        Ok(())
    }

    fn description(&self) -> String {
        "Set cycle region".to_string()
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
        controller.set_loop_region(self.new_region);
        controller.set_loop_enabled(self.new_enabled);
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
        controller.set_loop_region(self.old_region);
        controller.set_loop_enabled(self.old_enabled);
        Ok(())
    }
}
