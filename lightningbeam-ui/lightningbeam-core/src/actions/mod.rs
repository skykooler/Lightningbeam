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
pub mod paint_bucket;
pub mod remove_effect;
pub mod set_document_properties;
pub mod set_instance_properties;
pub mod set_layer_properties;
pub mod set_shape_properties;
pub mod split_clip_instance;
pub mod transform_clip_instances;
pub mod trim_clip_instances;
pub mod create_folder;
pub mod rename_folder;
pub mod delete_folder;
pub mod move_asset_to_folder;
pub mod update_midi_notes;
pub mod loop_clip_instances;
pub mod remove_clip_instances;
pub mod set_keyframe;
pub mod group_shapes;
pub mod convert_to_movie_clip;
pub mod region_split;
pub mod toggle_group_expansion;
pub mod group_layers;
pub mod raster_stroke;
pub mod raster_fill;
pub mod move_layer;

pub use add_clip_instance::AddClipInstanceAction;
pub use add_effect::AddEffectAction;
pub use add_layer::AddLayerAction;
pub use add_shape::AddShapeAction;
pub use modify_shape_path::ModifyDcelAction;
pub use move_clip_instances::MoveClipInstancesAction;
pub use paint_bucket::PaintBucketAction;
pub use remove_effect::RemoveEffectAction;
pub use set_document_properties::SetDocumentPropertiesAction;
pub use set_instance_properties::{InstancePropertyChange, SetInstancePropertiesAction};
pub use set_layer_properties::{LayerProperty, SetLayerPropertiesAction};
pub use set_shape_properties::SetShapePropertiesAction;
pub use split_clip_instance::SplitClipInstanceAction;
pub use transform_clip_instances::TransformClipInstancesAction;
pub use trim_clip_instances::{TrimClipInstancesAction, TrimData, TrimType};
pub use create_folder::CreateFolderAction;
pub use rename_folder::RenameFolderAction;
pub use delete_folder::{DeleteFolderAction, DeleteStrategy};
pub use move_asset_to_folder::MoveAssetToFolderAction;
pub use update_midi_notes::UpdateMidiNotesAction;
pub use loop_clip_instances::LoopClipInstancesAction;
pub use remove_clip_instances::RemoveClipInstancesAction;
pub use set_keyframe::SetKeyframeAction;
pub use group_shapes::GroupAction;
pub use convert_to_movie_clip::ConvertToMovieClipAction;
pub use region_split::RegionSplitAction;
pub use toggle_group_expansion::ToggleGroupExpansionAction;
pub use group_layers::GroupLayersAction;
pub use raster_stroke::RasterStrokeAction;
pub use raster_fill::RasterFillAction;
pub use move_layer::MoveLayerAction;
