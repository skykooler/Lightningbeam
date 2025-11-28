//! Hierarchical layer tree
//!
//! Provides a tree structure for organizing layers in a hierarchical manner.
//! Layers can be nested within other layers for organizational purposes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Node in the layer tree
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerNode<T> {
    /// The layer data
    pub data: T,

    /// Child layers
    pub children: Vec<LayerNode<T>>,
}

impl<T> LayerNode<T> {
    /// Create a new layer node
    pub fn new(data: T) -> Self {
        Self {
            data,
            children: Vec::new(),
        }
    }

    /// Add a child layer
    pub fn add_child(&mut self, child: LayerNode<T>) {
        self.children.push(child);
    }

    /// Remove a child layer by index
    pub fn remove_child(&mut self, index: usize) -> Option<LayerNode<T>> {
        if index < self.children.len() {
            Some(self.children.remove(index))
        } else {
            None
        }
    }

    /// Get a reference to a child
    pub fn get_child(&self, index: usize) -> Option<&LayerNode<T>> {
        self.children.get(index)
    }

    /// Get a mutable reference to a child
    pub fn get_child_mut(&mut self, index: usize) -> Option<&mut LayerNode<T>> {
        self.children.get_mut(index)
    }

    /// Get number of children
    pub fn child_count(&self) -> usize {
        self.children.len()
    }
}

/// Layer tree root
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerTree<T> {
    /// Root layers (no parent)
    pub roots: Vec<LayerNode<T>>,
}

impl<T> LayerTree<T> {
    /// Create a new empty layer tree
    pub fn new() -> Self {
        Self { roots: Vec::new() }
    }

    /// Add a root layer and return its index
    pub fn add_root(&mut self, data: T) -> usize {
        let node = LayerNode::new(data);
        let index = self.roots.len();
        self.roots.push(node);
        index
    }

    /// Remove a root layer by index
    pub fn remove_root(&mut self, index: usize) -> Option<LayerNode<T>> {
        if index < self.roots.len() {
            Some(self.roots.remove(index))
        } else {
            None
        }
    }

    /// Get a reference to a root layer
    pub fn get_root(&self, index: usize) -> Option<&LayerNode<T>> {
        self.roots.get(index)
    }

    /// Get a mutable reference to a root layer
    pub fn get_root_mut(&mut self, index: usize) -> Option<&mut LayerNode<T>> {
        self.roots.get_mut(index)
    }

    /// Get number of root layers
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    /// Iterate over all root layers
    pub fn iter(&self) -> impl Iterator<Item = &LayerNode<T>> {
        self.roots.iter()
    }

    /// Iterate over all root layers mutably
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut LayerNode<T>> {
        self.roots.iter_mut()
    }
}

impl<T> Default for LayerTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layer_tree_creation() {
        let tree: LayerTree<i32> = LayerTree::new();
        assert_eq!(tree.root_count(), 0);
    }

    #[test]
    fn test_add_root_layers() {
        let mut tree = LayerTree::new();
        tree.add_root(1);
        tree.add_root(2);
        tree.add_root(3);

        assert_eq!(tree.root_count(), 3);
        assert_eq!(tree.get_root(0).unwrap().data, 1);
        assert_eq!(tree.get_root(1).unwrap().data, 2);
        assert_eq!(tree.get_root(2).unwrap().data, 3);
    }

    #[test]
    fn test_nested_layers() {
        let mut tree = LayerTree::new();
        let root_idx = tree.add_root("Root");

        let root = tree.get_root_mut(root_idx).unwrap();
        root.add_child(LayerNode::new("Child 1"));
        root.add_child(LayerNode::new("Child 2"));

        assert_eq!(root.child_count(), 2);
        assert_eq!(root.get_child(0).unwrap().data, "Child 1");
        assert_eq!(root.get_child(1).unwrap().data, "Child 2");
    }

    #[test]
    fn test_remove_root() {
        let mut tree = LayerTree::new();
        tree.add_root(1);
        tree.add_root(2);
        tree.add_root(3);

        let removed = tree.remove_root(1);
        assert_eq!(removed.unwrap().data, 2);
        assert_eq!(tree.root_count(), 2);
        assert_eq!(tree.get_root(0).unwrap().data, 1);
        assert_eq!(tree.get_root(1).unwrap().data, 3);
    }

    #[test]
    fn test_remove_child() {
        let mut tree = LayerTree::new();
        let root_idx = tree.add_root("Root");

        let root = tree.get_root_mut(root_idx).unwrap();
        root.add_child(LayerNode::new("Child 1"));
        root.add_child(LayerNode::new("Child 2"));
        root.add_child(LayerNode::new("Child 3"));

        let removed = root.remove_child(1);
        assert_eq!(removed.unwrap().data, "Child 2");
        assert_eq!(root.child_count(), 2);
        assert_eq!(root.get_child(0).unwrap().data, "Child 1");
        assert_eq!(root.get_child(1).unwrap().data, "Child 3");
    }
}
