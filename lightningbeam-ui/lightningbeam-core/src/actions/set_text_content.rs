//! Set-text-content action
//!
//! Replaces a text layer's [`TextContent`] (text string, font size, color, family,
//! alignment) as one undoable step. Used both by in-place editing (text changes)
//! and the info panel (style changes).

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::text_layer::TextContent;
use uuid::Uuid;

pub struct SetTextContentAction {
    layer_id: Uuid,
    new: TextContent,
    old: Option<TextContent>,
}

impl SetTextContentAction {
    pub fn new(layer_id: Uuid, new: TextContent) -> Self {
        Self { layer_id, new, old: None }
    }

    /// Construct with an explicit `old` value. Used by in-place editing, which mutates
    /// the document live for preview and then records one undoable step capturing the
    /// content as it was *before* editing began.
    pub fn with_old(layer_id: Uuid, old: TextContent, new: TextContent) -> Self {
        Self { layer_id, new, old: Some(old) }
    }
}

impl Action for SetTextContentAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        let AnyLayer::Text(text_layer) = layer else {
            return Err("SetTextContentAction target is not a text layer".to_string());
        };
        if self.old.is_none() {
            self.old = Some(text_layer.content.clone());
        }
        text_layer.content = self.new.clone();
        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let Some(old) = self.old.clone() else { return Ok(()) };
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;
        if let AnyLayer::Text(text_layer) = layer {
            text_layer.content = old;
        }
        Ok(())
    }

    fn description(&self) -> String {
        "Edit text".to_string()
    }
}
