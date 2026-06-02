//! Toggle group layer expansion state (collapsed/expanded in timeline)

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use uuid::Uuid;

/// Action that toggles a group layer's expanded/collapsed state
pub struct ToggleGroupExpansionAction {
    group_id: Uuid,
    new_expanded: bool,
    old_expanded: Option<bool>,
}

impl ToggleGroupExpansionAction {
    pub fn new(group_id: Uuid, expanded: bool) -> Self {
        Self {
            group_id,
            new_expanded: expanded,
            old_expanded: None,
        }
    }
}

impl Action for ToggleGroupExpansionAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(AnyLayer::Group(g)) = document
            .root
            .children
            .iter_mut()
            .find(|l| l.id() == self.group_id)
        {
            self.old_expanded = Some(g.expanded);
            g.expanded = self.new_expanded;
            Ok(())
        } else {
            Err(format!("Group layer {} not found", self.group_id))
        }
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        if let Some(old) = self.old_expanded {
            if let Some(AnyLayer::Group(g)) = document
                .root
                .children
                .iter_mut()
                .find(|l| l.id() == self.group_id)
            {
                g.expanded = old;
            }
        }
        Ok(())
    }

    fn description(&self) -> String {
        if self.new_expanded {
            "Expand group".to_string()
        } else {
            "Collapse group".to_string()
        }
    }
}
