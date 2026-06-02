//! Asset library folder organization
//!
//! Provides hierarchical folder structure for organizing assets in the library.
//! Each asset category (Vector, Video, Audio, Images, Effects) has its own
//! independent folder tree with unlimited nesting depth.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Metadata for an asset library folder
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AssetFolder {
    /// Unique identifier
    pub id: Uuid,

    /// Folder name
    pub name: String,

    /// Parent folder ID (None for root-level folders)
    pub parent_id: Option<Uuid>,
}

impl AssetFolder {
    /// Create a new folder
    pub fn new(name: impl Into<String>, parent_id: Option<Uuid>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            parent_id,
        }
    }
}

/// Folder tree for a specific asset category
///
/// Uses a flat HashMap for efficient O(1) lookup, with parent_id references
/// for hierarchy. This matches the Document's asset storage pattern.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AssetFolderTree {
    /// All folders in this category, keyed by ID
    pub folders: HashMap<Uuid, AssetFolder>,
}

impl AssetFolderTree {
    /// Create a new empty folder tree
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a folder to the tree
    ///
    /// Returns the folder's ID for convenience
    pub fn add_folder(&mut self, folder: AssetFolder) -> Uuid {
        let id = folder.id;
        self.folders.insert(id, folder);
        id
    }

    /// Remove a folder and all its children recursively
    ///
    /// Returns the removed folder and all descendants for undo support
    pub fn remove_folder(&mut self, folder_id: &Uuid) -> Vec<AssetFolder> {
        let mut removed = Vec::new();

        // Collect all descendant IDs using breadth-first traversal
        let mut to_remove = vec![*folder_id];
        let mut i = 0;
        while i < to_remove.len() {
            let current_id = to_remove[i];

            // Find children of current folder
            for (child_id, child) in &self.folders {
                if child.parent_id == Some(current_id) {
                    to_remove.push(*child_id);
                }
            }
            i += 1;
        }

        // Remove all collected folders
        for id in to_remove {
            if let Some(folder) = self.folders.remove(&id) {
                removed.push(folder);
            }
        }

        removed
    }

    /// Get all root folders (folders with no parent)
    ///
    /// Returns folders sorted alphabetically by name (case-insensitive)
    pub fn root_folders(&self) -> Vec<&AssetFolder> {
        let mut roots: Vec<_> = self.folders.values()
            .filter(|f| f.parent_id.is_none())
            .collect();
        roots.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        roots
    }

    /// Get children of a specific folder
    ///
    /// Returns folders sorted alphabetically by name (case-insensitive)
    pub fn children_of(&self, parent_id: &Uuid) -> Vec<&AssetFolder> {
        let mut children: Vec<_> = self.folders.values()
            .filter(|f| f.parent_id == Some(*parent_id))
            .collect();
        children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        children
    }

    /// Get the full path from root to a folder
    ///
    /// Returns a vector of folder IDs starting from root and ending at the specified folder
    pub fn path_to_folder(&self, folder_id: &Uuid) -> Vec<Uuid> {
        let mut path = Vec::new();
        let mut current_id = Some(*folder_id);

        // Walk up the tree from folder to root
        while let Some(id) = current_id {
            path.insert(0, id);
            current_id = self.folders.get(&id).and_then(|f| f.parent_id);
        }

        path
    }

    /// Check if folder_a is a descendant of folder_b (or the same folder)
    ///
    /// Used to prevent circular references when moving folders
    pub fn is_descendant_of(&self, folder_a: &Uuid, folder_b: &Uuid) -> bool {
        if folder_a == folder_b {
            return true;
        }

        let mut current_id = self.folders.get(folder_a).and_then(|f| f.parent_id);

        // Walk up from folder_a, looking for folder_b
        while let Some(id) = current_id {
            if id == *folder_b {
                return true;
            }
            current_id = self.folders.get(&id).and_then(|f| f.parent_id);
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_folder() {
        let folder = AssetFolder::new("Test Folder", None);
        assert_eq!(folder.name, "Test Folder");
        assert_eq!(folder.parent_id, None);
    }

    #[test]
    fn test_add_and_get_root_folders() {
        let mut tree = AssetFolderTree::new();

        let folder1 = AssetFolder::new("Folder B", None);
        let folder2 = AssetFolder::new("Folder A", None);

        tree.add_folder(folder1);
        tree.add_folder(folder2);

        let roots = tree.root_folders();
        assert_eq!(roots.len(), 2);
        // Should be sorted alphabetically
        assert_eq!(roots[0].name, "Folder A");
        assert_eq!(roots[1].name, "Folder B");
    }

    #[test]
    fn test_children_of() {
        let mut tree = AssetFolderTree::new();

        let parent = AssetFolder::new("Parent", None);
        let parent_id = tree.add_folder(parent);

        let child1 = AssetFolder::new("Child B", Some(parent_id));
        let child2 = AssetFolder::new("Child A", Some(parent_id));

        tree.add_folder(child1);
        tree.add_folder(child2);

        let children = tree.children_of(&parent_id);
        assert_eq!(children.len(), 2);
        // Should be sorted alphabetically
        assert_eq!(children[0].name, "Child A");
        assert_eq!(children[1].name, "Child B");
    }

    #[test]
    fn test_path_to_folder() {
        let mut tree = AssetFolderTree::new();

        let root = AssetFolder::new("Root", None);
        let root_id = tree.add_folder(root);

        let child = AssetFolder::new("Child", Some(root_id));
        let child_id = tree.add_folder(child);

        let grandchild = AssetFolder::new("Grandchild", Some(child_id));
        let grandchild_id = tree.add_folder(grandchild);

        let path = tree.path_to_folder(&grandchild_id);
        assert_eq!(path, vec![root_id, child_id, grandchild_id]);
    }

    #[test]
    fn test_is_descendant_of() {
        let mut tree = AssetFolderTree::new();

        let root = AssetFolder::new("Root", None);
        let root_id = tree.add_folder(root);

        let child = AssetFolder::new("Child", Some(root_id));
        let child_id = tree.add_folder(child);

        let grandchild = AssetFolder::new("Grandchild", Some(child_id));
        let grandchild_id = tree.add_folder(grandchild);

        // Grandchild is descendant of child
        assert!(tree.is_descendant_of(&grandchild_id, &child_id));

        // Grandchild is descendant of root
        assert!(tree.is_descendant_of(&grandchild_id, &root_id));

        // Child is not descendant of grandchild
        assert!(!tree.is_descendant_of(&child_id, &grandchild_id));

        // Folder is descendant of itself
        assert!(tree.is_descendant_of(&child_id, &child_id));
    }

    #[test]
    fn test_remove_folder_recursive() {
        let mut tree = AssetFolderTree::new();

        let root = AssetFolder::new("Root", None);
        let root_id = tree.add_folder(root);

        let child1 = AssetFolder::new("Child1", Some(root_id));
        let child1_id = tree.add_folder(child1);

        let child2 = AssetFolder::new("Child2", Some(root_id));
        tree.add_folder(child2);

        let grandchild = AssetFolder::new("Grandchild", Some(child1_id));
        tree.add_folder(grandchild);

        // Remove root folder (should remove all descendants)
        let removed = tree.remove_folder(&root_id);
        assert_eq!(removed.len(), 4); // root + 2 children + 1 grandchild
        assert_eq!(tree.folders.len(), 0);
    }
}
