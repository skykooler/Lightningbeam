//! Change FPS action
//!
//! Atomically changes the document framerate.
//! All clip positions are stored in **beats** so no position rescaling is
//! required — only the framerate field changes.

use crate::action::{Action, BackendContext};
use crate::document::Document;

/// Action that changes the document framerate.
pub struct ChangeFpsAction {
    old_fps: f64,
    new_fps: f64,
}

impl ChangeFpsAction {
    /// Build the action from old and new framerates.
    pub fn new(old_fps: f64, new_fps: f64, _document: &Document) -> Self {
        Self { old_fps, new_fps }
    }
}

impl Action for ChangeFpsAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        document.framerate = self.new_fps;
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        document.framerate = self.old_fps;
        Ok(())
    }

    fn description(&self) -> String {
        "Change FPS".to_string()
    }

    fn execute_backend(
        &mut self,
        _backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        // FPS does not affect audio scheduling — nothing to do in the backend.
        Ok(())
    }

    fn rollback_backend(
        &mut self,
        _backend: &mut BackendContext,
        _document: &Document,
    ) -> Result<(), String> {
        Ok(())
    }
}
