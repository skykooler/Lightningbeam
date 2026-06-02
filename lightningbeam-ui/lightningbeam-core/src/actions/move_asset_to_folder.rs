//! Move asset to folder action
//!
//! Handles moving an asset between folders in the asset library.

use crate::action::Action;
use crate::document::{AssetCategory, Document};
use uuid::Uuid;

/// Action that moves an asset to a different folder
pub struct MoveAssetToFolderAction {
    /// Asset category
    category: AssetCategory,

    /// Asset ID to move
    asset_id: Uuid,

    /// New folder ID (None = move to root)
    new_folder_id: Option<Uuid>,

    /// Old folder ID (stored after execution for rollback)
    old_folder_id: Option<Option<Uuid>>,
}

impl MoveAssetToFolderAction {
    /// Create a new move asset to folder action
    ///
    /// # Arguments
    ///
    /// * `category` - Which asset category the asset is in
    /// * `asset_id` - ID of the asset to move
    /// * `new_folder_id` - ID of the destination folder (None = root)
    pub fn new(
        category: AssetCategory,
        asset_id: Uuid,
        new_folder_id: Option<Uuid>,
    ) -> Self {
        Self {
            category,
            asset_id,
            new_folder_id,
            old_folder_id: None,
        }
    }
}

impl Action for MoveAssetToFolderAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        // Validate that the destination folder exists (if specified)
        if let Some(folder_id) = self.new_folder_id {
            let tree = document.get_folder_tree(self.category);
            if !tree.folders.contains_key(&folder_id) {
                return Err(format!("Destination folder {} not found", folder_id));
            }
        }

        // Find the asset and update its folder_id
        match self.category {
            AssetCategory::Vector => {
                let clip = document
                    .vector_clips
                    .get_mut(&self.asset_id)
                    .ok_or_else(|| format!("Vector clip {} not found", self.asset_id))?;

                self.old_folder_id = Some(clip.folder_id);
                clip.folder_id = self.new_folder_id;
            }
            AssetCategory::Video => {
                let clip = document
                    .video_clips
                    .get_mut(&self.asset_id)
                    .ok_or_else(|| format!("Video clip {} not found", self.asset_id))?;

                self.old_folder_id = Some(clip.folder_id);
                clip.folder_id = self.new_folder_id;
            }
            AssetCategory::Audio => {
                let clip = document
                    .audio_clips
                    .get_mut(&self.asset_id)
                    .ok_or_else(|| format!("Audio clip {} not found", self.asset_id))?;

                self.old_folder_id = Some(clip.folder_id);
                clip.folder_id = self.new_folder_id;
            }
            AssetCategory::Images => {
                let asset = document
                    .image_assets
                    .get_mut(&self.asset_id)
                    .ok_or_else(|| format!("Image asset {} not found", self.asset_id))?;

                self.old_folder_id = Some(asset.folder_id);
                asset.folder_id = self.new_folder_id;
            }
            AssetCategory::Effects => {
                let effect = document
                    .effect_definitions
                    .get_mut(&self.asset_id)
                    .ok_or_else(|| format!("Effect definition {} not found", self.asset_id))?;

                self.old_folder_id = Some(effect.folder_id);
                effect.folder_id = self.new_folder_id;
            }
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        // Restore the old folder_id
        if let Some(old_folder_id) = self.old_folder_id {
            match self.category {
                AssetCategory::Vector => {
                    if let Some(clip) = document.vector_clips.get_mut(&self.asset_id) {
                        clip.folder_id = old_folder_id;
                    }
                }
                AssetCategory::Video => {
                    if let Some(clip) = document.video_clips.get_mut(&self.asset_id) {
                        clip.folder_id = old_folder_id;
                    }
                }
                AssetCategory::Audio => {
                    if let Some(clip) = document.audio_clips.get_mut(&self.asset_id) {
                        clip.folder_id = old_folder_id;
                    }
                }
                AssetCategory::Images => {
                    if let Some(asset) = document.image_assets.get_mut(&self.asset_id) {
                        asset.folder_id = old_folder_id;
                    }
                }
                AssetCategory::Effects => {
                    if let Some(effect) = document.effect_definitions.get_mut(&self.asset_id) {
                        effect.folder_id = old_folder_id;
                    }
                }
            }
        }

        Ok(())
    }

    fn description(&self) -> String {
        if self.new_folder_id.is_some() {
            "Move asset to folder".to_string()
        } else {
            "Move asset to root".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::create_folder::CreateFolderAction;
    use crate::clip::VectorClip;

    #[test]
    fn test_move_asset_to_folder() {
        let mut document = Document::new("Test");

        // Create a folder
        let mut create_action =
            CreateFolderAction::new(AssetCategory::Vector, "My Folder", None);
        create_action.execute(&mut document).unwrap();
        let folder_id = create_action.created_folder_id().unwrap();

        // Create a clip at root
        let clip = VectorClip::new("Test Clip", 100.0, 100.0, 5.0);
        let clip_id = clip.id;
        document.vector_clips.insert(clip_id, clip);

        // Verify clip is at root
        assert_eq!(document.vector_clips[&clip_id].folder_id, None);

        // Move clip to folder
        let mut move_action =
            MoveAssetToFolderAction::new(AssetCategory::Vector, clip_id, Some(folder_id));
        move_action.execute(&mut document).unwrap();

        // Verify clip moved
        assert_eq!(
            document.vector_clips[&clip_id].folder_id,
            Some(folder_id)
        );

        // Rollback
        move_action.rollback(&mut document).unwrap();

        // Verify clip back at root
        assert_eq!(document.vector_clips[&clip_id].folder_id, None);
    }

    #[test]
    fn test_move_asset_to_root() {
        let mut document = Document::new("Test");

        // Create a folder
        let mut create_action =
            CreateFolderAction::new(AssetCategory::Video, "Videos", None);
        create_action.execute(&mut document).unwrap();
        let folder_id = create_action.created_folder_id().unwrap();

        // Create a clip in the folder
        let mut clip = crate::clip::VideoClip::new(
            "Test Video",
            "test.mp4",
            1920.0,
            1080.0,
            10.0,
            30.0,
        );
        clip.folder_id = Some(folder_id);
        let clip_id = clip.id;
        document.video_clips.insert(clip_id, clip);

        // Move clip to root
        let mut move_action = MoveAssetToFolderAction::new(AssetCategory::Video, clip_id, None);
        move_action.execute(&mut document).unwrap();

        // Verify clip at root
        assert_eq!(document.video_clips[&clip_id].folder_id, None);

        // Rollback
        move_action.rollback(&mut document).unwrap();

        // Verify clip back in folder
        assert_eq!(
            document.video_clips[&clip_id].folder_id,
            Some(folder_id)
        );
    }

    #[test]
    fn test_move_asset_between_folders() {
        let mut document = Document::new("Test");

        // Create two folders
        let mut folder1_action =
            CreateFolderAction::new(AssetCategory::Audio, "Folder 1", None);
        folder1_action.execute(&mut document).unwrap();
        let folder1_id = folder1_action.created_folder_id().unwrap();

        let mut folder2_action =
            CreateFolderAction::new(AssetCategory::Audio, "Folder 2", None);
        folder2_action.execute(&mut document).unwrap();
        let folder2_id = folder2_action.created_folder_id().unwrap();

        // Create a clip in folder 1
        let mut clip = crate::clip::AudioClip::new_sampled("Test Audio", 0, 5.0);
        clip.folder_id = Some(folder1_id);
        let clip_id = clip.id;
        document.audio_clips.insert(clip_id, clip);

        // Move to folder 2
        let mut move_action =
            MoveAssetToFolderAction::new(AssetCategory::Audio, clip_id, Some(folder2_id));
        move_action.execute(&mut document).unwrap();

        // Verify in folder 2
        assert_eq!(
            document.audio_clips[&clip_id].folder_id,
            Some(folder2_id)
        );

        // Rollback
        move_action.rollback(&mut document).unwrap();

        // Verify back in folder 1
        assert_eq!(
            document.audio_clips[&clip_id].folder_id,
            Some(folder1_id)
        );
    }

    #[test]
    fn test_move_to_nonexistent_folder() {
        let mut document = Document::new("Test");

        // Create a clip
        let clip = VectorClip::new("Test", 100.0, 100.0, 5.0);
        let clip_id = clip.id;
        document.vector_clips.insert(clip_id, clip);

        // Try to move to nonexistent folder
        let fake_folder_id = Uuid::new_v4();
        let mut action =
            MoveAssetToFolderAction::new(AssetCategory::Vector, clip_id, Some(fake_folder_id));
        let result = action.execute(&mut document);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_move_nonexistent_asset() {
        let mut document = Document::new("Test");

        let fake_asset_id = Uuid::new_v4();
        let mut action = MoveAssetToFolderAction::new(AssetCategory::Images, fake_asset_id, None);
        let result = action.execute(&mut document);

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_move_description() {
        let asset_id = Uuid::new_v4();
        let folder_id = Uuid::new_v4();

        let action1 =
            MoveAssetToFolderAction::new(AssetCategory::Effects, asset_id, Some(folder_id));
        assert_eq!(action1.description(), "Move asset to folder");

        let action2 = MoveAssetToFolderAction::new(AssetCategory::Effects, asset_id, None);
        assert_eq!(action2.description(), "Move asset to root");
    }
}
