//! Set document properties action
//!
//! Handles changing document-level properties (width, height, duration, framerate)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;

/// Individual property change for a document
#[derive(Clone, Debug)]
pub enum DocumentPropertyChange {
    Width(f64),
    Height(f64),
    Duration(f64),
    Framerate(f64),
}

impl DocumentPropertyChange {
    /// Extract the f64 value from any variant
    fn value(&self) -> f64 {
        match self {
            DocumentPropertyChange::Width(v) => *v,
            DocumentPropertyChange::Height(v) => *v,
            DocumentPropertyChange::Duration(v) => *v,
            DocumentPropertyChange::Framerate(v) => *v,
        }
    }
}

/// Action that sets a property on the document
pub struct SetDocumentPropertiesAction {
    /// The new property value
    property: DocumentPropertyChange,
    /// The old value for undo
    old_value: Option<f64>,
}

impl SetDocumentPropertiesAction {
    /// Create a new action to set width
    pub fn set_width(width: f64) -> Self {
        Self {
            property: DocumentPropertyChange::Width(width),
            old_value: None,
        }
    }

    /// Create a new action to set height
    pub fn set_height(height: f64) -> Self {
        Self {
            property: DocumentPropertyChange::Height(height),
            old_value: None,
        }
    }

    /// Create a new action to set duration
    pub fn set_duration(duration: f64) -> Self {
        Self {
            property: DocumentPropertyChange::Duration(duration),
            old_value: None,
        }
    }

    /// Create a new action to set framerate
    pub fn set_framerate(framerate: f64) -> Self {
        Self {
            property: DocumentPropertyChange::Framerate(framerate),
            old_value: None,
        }
    }

    fn get_current_value(&self, document: &Document) -> f64 {
        match &self.property {
            DocumentPropertyChange::Width(_) => document.width,
            DocumentPropertyChange::Height(_) => document.height,
            DocumentPropertyChange::Duration(_) => document.duration,
            DocumentPropertyChange::Framerate(_) => document.framerate,
        }
    }

    fn apply_value(&self, document: &mut Document, value: f64) {
        match &self.property {
            DocumentPropertyChange::Width(_) => document.width = value,
            DocumentPropertyChange::Height(_) => document.height = value,
            DocumentPropertyChange::Duration(_) => document.duration = value,
            DocumentPropertyChange::Framerate(_) => document.framerate = value,
        }
    }
}

impl Action for SetDocumentPropertiesAction {
    fn execute(&mut self, document: &mut Document) {
        // Store old value if not already stored
        if self.old_value.is_none() {
            self.old_value = Some(self.get_current_value(document));
        }

        // Apply new value
        let new_value = self.property.value();
        self.apply_value(document, new_value);
    }

    fn rollback(&mut self, document: &mut Document) {
        if let Some(old_value) = self.old_value {
            self.apply_value(document, old_value);
        }
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            DocumentPropertyChange::Width(_) => "canvas width",
            DocumentPropertyChange::Height(_) => "canvas height",
            DocumentPropertyChange::Duration(_) => "duration",
            DocumentPropertyChange::Framerate(_) => "framerate",
        };
        format!("Set {}", property_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_width() {
        let mut document = Document::new("Test");
        document.width = 1920.0;

        let mut action = SetDocumentPropertiesAction::set_width(1280.0);
        action.execute(&mut document);
        assert_eq!(document.width, 1280.0);

        action.rollback(&mut document);
        assert_eq!(document.width, 1920.0);
    }

    #[test]
    fn test_set_height() {
        let mut document = Document::new("Test");
        document.height = 1080.0;

        let mut action = SetDocumentPropertiesAction::set_height(720.0);
        action.execute(&mut document);
        assert_eq!(document.height, 720.0);

        action.rollback(&mut document);
        assert_eq!(document.height, 1080.0);
    }

    #[test]
    fn test_set_duration() {
        let mut document = Document::new("Test");
        document.duration = 10.0;

        let mut action = SetDocumentPropertiesAction::set_duration(30.0);
        action.execute(&mut document);
        assert_eq!(document.duration, 30.0);

        action.rollback(&mut document);
        assert_eq!(document.duration, 10.0);
    }

    #[test]
    fn test_set_framerate() {
        let mut document = Document::new("Test");
        document.framerate = 30.0;

        let mut action = SetDocumentPropertiesAction::set_framerate(60.0);
        action.execute(&mut document);
        assert_eq!(document.framerate, 60.0);

        action.rollback(&mut document);
        assert_eq!(document.framerate, 30.0);
    }

    #[test]
    fn test_description() {
        let action = SetDocumentPropertiesAction::set_width(100.0);
        assert_eq!(action.description(), "Set canvas width");

        let action = SetDocumentPropertiesAction::set_duration(30.0);
        assert_eq!(action.description(), "Set duration");
    }
}
