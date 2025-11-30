//! Action implementations for document editing
//!
//! This module contains all the concrete action types that can be executed
//! through the action system.

pub mod add_clip_instance;
pub mod add_layer;
pub mod add_shape;
pub mod move_clip_instances;
pub mod move_objects;
pub mod paint_bucket;
pub mod set_layer_properties;
pub mod transform_clip_instances;
pub mod transform_objects;
pub mod trim_clip_instances;

pub use add_clip_instance::AddClipInstanceAction;
pub use add_layer::AddLayerAction;
pub use add_shape::AddShapeAction;
pub use move_clip_instances::MoveClipInstancesAction;
pub use move_objects::MoveShapeInstancesAction;
pub use paint_bucket::PaintBucketAction;
pub use set_layer_properties::{LayerProperty, SetLayerPropertiesAction};
pub use transform_clip_instances::TransformClipInstancesAction;
pub use transform_objects::TransformShapeInstancesAction;
pub use trim_clip_instances::{TrimClipInstancesAction, TrimData, TrimType};
