//! Create folder action
//!
//! Handles creating a new folder in the asset library.

use crate::action::Action;
use crate::asset_folder::AssetFolder;
use crate::document::{AssetCategory, Document};
use uuid::Uuid;

/// Action that creates a new folder in an asset category
pub struct CreateFolderAction {
    /// Asset category for this folder
    category: AssetCategory,

    /// Folder name
    name: String,

    /// Parent folder ID (None = root level)
    parent_id: Option<Uuid>,

    /// ID of the created folder (set after execution)
    created_folder_id: Option<Uuid>,
}

impl CreateFolderAction {
    /// Create a new folder action
    ///
    /// # Arguments
    ///
    /// * `category` - Which asset category to create the folder in
    /// * `name` - The name for the new folder
    /// * `parent_id` - Optional parent folder ID (None = root level)
    pub fn new(
        category: AssetCategory,
        name: impl Into<String>,
        parent_id: Option<Uuid>,
    ) -> Self {
        Self {
            category,
            name: name.into(),
            parent_id,
            created_folder_id: None,
        }
    }

    /// Get the ID of the created folder (after execution)
    pub fn created_folder_id(&self) -> Option<Uuid> {
        self.created_folder_id
    }
}

impl Action for CreateFolderAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Create the folder
        let folder = AssetFolder::new(&self.name, self.parent_id);
        let folder_id = folder.id;

        // Add to the appropriate folder tree
        let tree = document.get_folder_tree_mut(self.category);
        tree.add_folder(folder);

        // Store the ID for rollback
        self.created_folder_id = Some(folder_id);

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        // Remove the created folder if it exists
        if let Some(folder_id) = self.created_folder_id {
            let tree = document.get_folder_tree_mut(self.category);
            tree.remove_folder(&folder_id);

            // Clear the stored ID
            self.created_folder_id = None;
        }

        Ok(())
    }

    fn description(&self) -> String {
        format!("Create folder '{}'", self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_folder_at_root() {
        let mut document = Document::new("Test");

        // Create and execute action
        let mut action = CreateFolderAction::new(AssetCategory::Vector, "My Folder", None);
        action.execute(&mut document).unwrap();

        // Verify folder was created
        let tree = document.get_folder_tree(AssetCategory::Vector);
        assert_eq!(tree.folders.len(), 1);

        let roots = tree.root_folders();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "My Folder");
        assert_eq!(roots[0].parent_id, None);

        // Get the created ID
        let folder_id = action.created_folder_id().unwrap();
        assert_eq!(roots[0].id, folder_id);

        // Rollback
        action.rollback(&mut document).unwrap();

        // Verify folder was removed
        let tree = document.get_folder_tree(AssetCategory::Vector);
        assert_eq!(tree.folders.len(), 0);
    }

    #[test]
    fn test_create_nested_folder() {
        let mut document = Document::new("Test");

        // Create parent folder
        let mut parent_action = CreateFolderAction::new(AssetCategory::Audio, "Parent", None);
        parent_action.execute(&mut document).unwrap();
        let parent_id = parent_action.created_folder_id().unwrap();

        // Create child folder
        let mut child_action =
            CreateFolderAction::new(AssetCategory::Audio, "Child", Some(parent_id));
        child_action.execute(&mut document).unwrap();

        // Verify structure
        let tree = document.get_folder_tree(AssetCategory::Audio);
        assert_eq!(tree.folders.len(), 2);

        let roots = tree.root_folders();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name, "Parent");

        let children = tree.children_of(&parent_id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "Child");
        assert_eq!(children[0].parent_id, Some(parent_id));

        // Rollback child
        child_action.rollback(&mut document).unwrap();
        let tree = document.get_folder_tree(AssetCategory::Audio);
        assert_eq!(tree.folders.len(), 1);
        assert_eq!(tree.children_of(&parent_id).len(), 0);
    }

    #[test]
    fn test_create_folder_description() {
        let action = CreateFolderAction::new(AssetCategory::Images, "Photos", None);
        assert_eq!(action.description(), "Create folder 'Photos'");
    }

    #[test]
    fn test_multiple_categories() {
        let mut document = Document::new("Test");

        // Create folders in different categories
        let mut vector_action = CreateFolderAction::new(AssetCategory::Vector, "Shapes", None);
        let mut video_action = CreateFolderAction::new(AssetCategory::Video, "Clips", None);

        vector_action.execute(&mut document).unwrap();
        video_action.execute(&mut document).unwrap();

        // Verify each category has its own tree
        let vector_tree = document.get_folder_tree(AssetCategory::Vector);
        assert_eq!(vector_tree.root_folders().len(), 1);
        assert_eq!(vector_tree.root_folders()[0].name, "Shapes");

        let video_tree = document.get_folder_tree(AssetCategory::Video);
        assert_eq!(video_tree.root_folders().len(), 1);
        assert_eq!(video_tree.root_folders()[0].name, "Clips");

        // Other categories should still be empty
        let audio_tree = document.get_folder_tree(AssetCategory::Audio);
        assert_eq!(audio_tree.folders.len(), 0);
    }
}
