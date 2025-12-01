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

/// Backend context for actions that need to interact with external systems
///
/// This bundles all backend references (audio, future video) that actions
/// may need to synchronize state with external systems beyond the document.
pub struct BackendContext<'a> {
    /// Audio engine controller (optional - may not be initialized)
    pub audio_controller: Option<&'a mut daw_backend::EngineController>,

    /// Mapping from document layer UUIDs to backend track IDs
    pub layer_to_track_map: &'a HashMap<Uuid, daw_backend::TrackId>,

    // Future: pub video_controller: Option<&'a mut VideoController>,
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
    fn execute(&mut self, document: &mut Document);

    /// Undo this action (rollback changes)
    fn rollback(&mut self, document: &mut Document);

    /// Get a human-readable description of this action (for UI display)
    fn description(&self) -> String;

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
}

impl ActionExecutor {
    /// Create a new action executor with the given document
    pub fn new(document: Document) -> Self {
        Self {
            document: Arc::new(document),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            max_undo_depth: 100, // Default: keep last 100 actions
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
    pub fn execute(&mut self, mut action: Box<dyn Action>) {
        // Apply the action (uses copy-on-write if other Arc holders exist)
        action.execute(Arc::make_mut(&mut self.document));

        // Clear redo stack (new action invalidates redo history)
        self.redo_stack.clear();

        // Add to undo stack
        self.undo_stack.push(action);

        // Limit undo stack size
        if self.undo_stack.len() > self.max_undo_depth {
            self.undo_stack.remove(0);
        }
    }

    /// Undo the last action
    ///
    /// Returns true if an action was undone, false if undo stack is empty.
    pub fn undo(&mut self) -> bool {
        if let Some(mut action) = self.undo_stack.pop() {
            // Rollback the action (uses copy-on-write if other Arc holders exist)
            action.rollback(Arc::make_mut(&mut self.document));

            // Move to redo stack
            self.redo_stack.push(action);

            true
        } else {
            false
        }
    }

    /// Redo the last undone action
    ///
    /// Returns true if an action was redone, false if redo stack is empty.
    pub fn redo(&mut self) -> bool {
        if let Some(mut action) = self.redo_stack.pop() {
            // Re-execute the action (uses copy-on-write if other Arc holders exist)
            action.execute(Arc::make_mut(&mut self.document));

            // Move back to undo stack
            self.undo_stack.push(action);

            true
        } else {
            false
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
        action.execute(Arc::make_mut(&mut self.document));

        // 2. Execute backend changes (pass document for reading clip data)
        if let Err(e) = action.execute_backend(backend, &self.document) {
            // ATOMIC ROLLBACK: Backend failed → undo document
            action.rollback(Arc::make_mut(&mut self.document));
            return Err(e);
        }

        // 3. Push to undo stack (both succeeded)
        self.redo_stack.clear();
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
            action.rollback(Arc::make_mut(&mut self.document));

            // Move to redo stack
            self.redo_stack.push(action);

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
            action.execute(Arc::make_mut(&mut self.document));

            if let Err(e) = action.execute_backend(backend, &self.document) {
                // Rollback document if backend fails
                action.rollback(Arc::make_mut(&mut self.document));
                // Put action back on redo stack
                self.redo_stack.push(action);
                return Err(e);
            }

            // Move back to undo stack
            self.undo_stack.push(action);

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
        fn execute(&mut self, _document: &mut Document) {
            self.executed = true;
        }

        fn rollback(&mut self, _document: &mut Document) {
            self.executed = false;
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
        executor.execute(action);

        assert!(executor.can_undo());
        assert!(!executor.can_redo());
        assert_eq!(executor.undo_depth(), 1);

        // Undo
        assert!(executor.undo());
        assert!(!executor.can_undo());
        assert!(executor.can_redo());
        assert_eq!(executor.redo_depth(), 1);

        // Redo
        assert!(executor.redo());
        assert!(executor.can_undo());
        assert!(!executor.can_redo());
    }

    #[test]
    fn test_action_descriptions() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);

        executor.execute(Box::new(TestAction::new("Action 1")));
        executor.execute(Box::new(TestAction::new("Action 2")));

        assert_eq!(executor.undo_description(), Some("Action 2".to_string()));

        executor.undo();
        assert_eq!(executor.redo_description(), Some("Action 2".to_string()));
        assert_eq!(executor.undo_description(), Some("Action 1".to_string()));
    }

    #[test]
    fn test_new_action_clears_redo() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);

        executor.execute(Box::new(TestAction::new("Action 1")));
        executor.execute(Box::new(TestAction::new("Action 2")));
        executor.undo();

        assert!(executor.can_redo());

        // Execute new action should clear redo stack
        executor.execute(Box::new(TestAction::new("Action 3")));

        assert!(!executor.can_redo());
        assert_eq!(executor.undo_depth(), 2);
    }

    #[test]
    fn test_max_undo_depth() {
        let document = Document::new("Test");
        let mut executor = ActionExecutor::new(document);
        executor.set_max_undo_depth(3);

        executor.execute(Box::new(TestAction::new("Action 1")));
        executor.execute(Box::new(TestAction::new("Action 2")));
        executor.execute(Box::new(TestAction::new("Action 3")));
        executor.execute(Box::new(TestAction::new("Action 4")));

        // Should only keep last 3
        assert_eq!(executor.undo_depth(), 3);
        assert_eq!(executor.undo_description(), Some("Action 4".to_string()));
    }
}
