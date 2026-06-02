//! Rename folder action
//!
//! Handles renaming a folder in the asset library.

use crate::action::Action;
use crate::document::{AssetCategory, Document};
use uuid::Uuid;

/// Action that renames a folder
pub struct RenameFolderAction {
    /// Asset category for this folder
    category: AssetCategory,

    /// Folder ID to rename
    folder_id: Uuid,

    /// New folder name
    new_name: String,

    /// Old folder name (stored after execution for rollback)
    old_name: Option<String>,
}

impl RenameFolderAction {
    /// Create a new rename folder action
    ///
    /// # Arguments
    ///
    /// * `category` - Which asset category the folder is in
    /// * `folder_id` - ID of the folder to rename
    /// * `new_name` - The new name for the folder
    pub fn new(category: AssetCategory, folder_id: Uuid, new_name: impl Into<String>) -> Self {
        Self {
            category,
            folder_id,
            new_name: new_name.into(),
            old_name: None,
        }
    }
}

impl Action for RenameFolderAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Get the folder tree
        let tree = document.get_folder_tree_mut(self.category);

        // Get the folder
        let folder = tree
            .folders
            .get_mut(&self.folder_id)
            .ok_or_else(|| format!("Folder {} not found", self.folder_id))?;

        // Store old name for rollback
        self.old_name = Some(folder.name.clone());

        // Update the name
        folder.name = self.new_name.clone();

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        // Get the folder tree
        let tree = document.get_folder_tree_mut(self.category);

        // Get the folder
        let folder = tree
            .folders
            .get_mut(&self.folder_id)
            .ok_or_else(|| format!("Folder {} not found", self.folder_id))?;

        // Restore old name
        if let Some(old_name) = &self.old_name {
            folder.name = old_name.clone();
        }

        Ok(())
    }

    fn description(&self) -> String {
        format!("Rename folder to '{}'", self.new_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::create_folder::CreateFolderAction;

    #[test]
    fn test_rename_folder() {
        let mut document = Document::new("Test");

        // Create a folder first
        let mut create_action =
            CreateFolderAction::new(AssetCategory::Vector, "Original Name", None);
        create_action.execute(&mut document).unwrap();
        let folder_id = create_action.created_folder_id().unwrap();

        // Rename it
        let mut rename_action =
            RenameFolderAction::new(AssetCategory::Vector, folder_id, "New Name");
        rename_action.execute(&mut document).unwrap();

        // Verify name changed
        let tree = document.get_folder_tree(AssetCategory::Vector);
        let folder = &tree.folders[&folder_id];
        assert_eq!(folder.name, "New Name");

        // Rollback
        rename_action.rollback(&mut document).unwrap();

        // Verify name restored
        let tree = document.get_folder_tree(AssetCategory::Vector);
        let folder = &tree.folders[&folder_id];
        assert_eq!(folder.name, "Original Name");
    }

    #[test]
    fn test_rename_nonexistent_folder() {
        let mut document = Document::new("Test");
        let fake_id = Uuid::new_v4();

        let mut action = RenameFolderAction::new(AssetCategory::Audio, fake_id, "New Name");
        let result = action.execute(&mut document);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_rename_description() {
        let folder_id = Uuid::new_v4();
        let action = RenameFolderAction::new(AssetCategory::Images, folder_id, "Photos 2024");
        assert_eq!(action.description(), "Rename folder to 'Photos 2024'");
    }

    #[test]
    fn test_multiple_renames() {
        let mut document = Document::new("Test");

        // Create a folder
        let mut create_action = CreateFolderAction::new(AssetCategory::Effects, "V1", None);
        create_action.execute(&mut document).unwrap();
        let folder_id = create_action.created_folder_id().unwrap();

        // Rename multiple times
        let mut rename1 = RenameFolderAction::new(AssetCategory::Effects, folder_id, "V2");
        let mut rename2 = RenameFolderAction::new(AssetCategory::Effects, folder_id, "V3");
        let mut rename3 = RenameFolderAction::new(AssetCategory::Effects, folder_id, "Final");

        rename1.execute(&mut document).unwrap();
        rename2.execute(&mut document).unwrap();
        rename3.execute(&mut document).unwrap();

        // Verify final name
        let tree = document.get_folder_tree(AssetCategory::Effects);
        assert_eq!(tree.folders[&folder_id].name, "Final");

        // Rollback in reverse order
        rename3.rollback(&mut document).unwrap();
        let tree = document.get_folder_tree(AssetCategory::Effects);
        assert_eq!(tree.folders[&folder_id].name, "V3");

        rename2.rollback(&mut document).unwrap();
        let tree = document.get_folder_tree(AssetCategory::Effects);
        assert_eq!(tree.folders[&folder_id].name, "V2");

        rename1.rollback(&mut document).unwrap();
        let tree = document.get_folder_tree(AssetCategory::Effects);
        assert_eq!(tree.folders[&folder_id].name, "V1");
    }
}
