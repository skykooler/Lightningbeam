//! Action implementations for document editing
//!
//! This module contains all the concrete action types that can be executed
//! through the action system.

pub mod add_clip_instance;
pub mod add_effect;
pub mod add_layer;
pub mod add_shape;
pub mod modify_shape_path;
pub mod move_clip_instances;
pub mod move_objects;
pub mod paint_bucket;
pub mod remove_effect;
pub mod set_document_properties;
pub mod set_instance_properties;
pub mod set_layer_properties;
pub mod set_shape_properties;
pub mod split_clip_instance;
pub mod transform_clip_instances;
pub mod transform_objects;
pub mod trim_clip_instances;
pub mod create_folder;
pub mod rename_folder;
pub mod delete_folder;
pub mod move_asset_to_folder;

pub use add_clip_instance::AddClipInstanceAction;
pub use add_effect::AddEffectAction;
pub use add_layer::AddLayerAction;
pub use add_shape::AddShapeAction;
pub use modify_shape_path::ModifyShapePathAction;
pub use move_clip_instances::MoveClipInstancesAction;
pub use move_objects::MoveShapeInstancesAction;
pub use paint_bucket::PaintBucketAction;
pub use remove_effect::RemoveEffectAction;
pub use set_document_properties::SetDocumentPropertiesAction;
pub use set_instance_properties::{InstancePropertyChange, SetInstancePropertiesAction};
pub use set_layer_properties::{LayerProperty, SetLayerPropertiesAction};
pub use set_shape_properties::{SetShapePropertiesAction, ShapePropertyChange};
pub use split_clip_instance::SplitClipInstanceAction;
pub use transform_clip_instances::TransformClipInstancesAction;
pub use transform_objects::TransformShapeInstancesAction;
pub use trim_clip_instances::{TrimClipInstancesAction, TrimData, TrimType};
pub use create_folder::CreateFolderAction;
pub use rename_folder::RenameFolderAction;
pub use delete_folder::{DeleteFolderAction, DeleteStrategy};
pub use move_asset_to_folder::MoveAssetToFolderAction;
