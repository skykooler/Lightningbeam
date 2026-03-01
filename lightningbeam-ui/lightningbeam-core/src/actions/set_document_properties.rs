//! Set document properties action
//!
//! Handles changing document-level properties (width, height, duration, framerate)
//! with undo/redo support.

use crate::action::Action;
use crate::document::Document;
use crate::shape::ShapeColor;

/// Individual property change for a document
#[derive(Clone, Debug)]
pub enum DocumentPropertyChange {
    Width(f64),
    Height(f64),
    Duration(f64),
    Framerate(f64),
    BackgroundColor(ShapeColor),
}

/// Stored old value for undo (either f64 or color)
#[derive(Clone, Debug)]
enum OldValue {
    F64(f64),
    Color(ShapeColor),
}

/// Action that sets a property on the document
pub struct SetDocumentPropertiesAction {
    /// The new property value
    property: DocumentPropertyChange,
    /// The old value for undo
    old_value: Option<OldValue>,
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

    /// Create a new action to set background color
    pub fn set_background_color(color: ShapeColor) -> Self {
        Self {
            property: DocumentPropertyChange::BackgroundColor(color),
            old_value: None,
        }
    }
}

impl Action for SetDocumentPropertiesAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        if self.old_value.is_none() {
            self.old_value = Some(match &self.property {
                DocumentPropertyChange::Width(_) => OldValue::F64(document.width),
                DocumentPropertyChange::Height(_) => OldValue::F64(document.height),
                DocumentPropertyChange::Duration(_) => OldValue::F64(document.duration),
                DocumentPropertyChange::Framerate(_) => OldValue::F64(document.framerate),
                DocumentPropertyChange::BackgroundColor(_) => OldValue::Color(document.background_color),
            });
        }

        match &self.property {
            DocumentPropertyChange::Width(v) => document.width = *v,
            DocumentPropertyChange::Height(v) => document.height = *v,
            DocumentPropertyChange::Duration(v) => document.duration = *v,
            DocumentPropertyChange::Framerate(v) => document.framerate = *v,
            DocumentPropertyChange::BackgroundColor(c) => document.background_color = *c,
        }
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        match &self.old_value {
            Some(OldValue::F64(v)) => {
                let v = *v;
                match &self.property {
                    DocumentPropertyChange::Width(_) => document.width = v,
                    DocumentPropertyChange::Height(_) => document.height = v,
                    DocumentPropertyChange::Duration(_) => document.duration = v,
                    DocumentPropertyChange::Framerate(_) => document.framerate = v,
                    DocumentPropertyChange::BackgroundColor(_) => {}
                }
            }
            Some(OldValue::Color(c)) => {
                document.background_color = *c;
            }
            None => {}
        }
        Ok(())
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            DocumentPropertyChange::Width(_) => "canvas width",
            DocumentPropertyChange::Height(_) => "canvas height",
            DocumentPropertyChange::Duration(_) => "duration",
            DocumentPropertyChange::Framerate(_) => "framerate",
            DocumentPropertyChange::BackgroundColor(_) => "background color",
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
        action.execute(&mut document).unwrap();
        assert_eq!(document.width, 1280.0);

        action.rollback(&mut document).unwrap();
        assert_eq!(document.width, 1920.0);
    }

    #[test]
    fn test_set_height() {
        let mut document = Document::new("Test");
        document.height = 1080.0;

        let mut action = SetDocumentPropertiesAction::set_height(720.0);
        action.execute(&mut document).unwrap();
        assert_eq!(document.height, 720.0);

        action.rollback(&mut document).unwrap();
        assert_eq!(document.height, 1080.0);
    }

    #[test]
    fn test_set_duration() {
        let mut document = Document::new("Test");
        document.duration = 10.0;

        let mut action = SetDocumentPropertiesAction::set_duration(30.0);
        action.execute(&mut document).unwrap();
        assert_eq!(document.duration, 30.0);

        action.rollback(&mut document).unwrap();
        assert_eq!(document.duration, 10.0);
    }

    #[test]
    fn test_set_framerate() {
        let mut document = Document::new("Test");
        document.framerate = 30.0;

        let mut action = SetDocumentPropertiesAction::set_framerate(60.0);
        action.execute(&mut document).unwrap();
        assert_eq!(document.framerate, 60.0);

        action.rollback(&mut document).unwrap();
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
