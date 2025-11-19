//! Action implementations for document editing
//!
//! This module contains all the concrete action types that can be executed
//! through the action system.

pub mod add_shape;
pub mod move_objects;
pub mod paint_bucket;
pub mod transform_objects;

pub use add_shape::AddShapeAction;
pub use move_objects::MoveObjectsAction;
pub use paint_bucket::PaintBucketAction;
pub use transform_objects::TransformObjectsAction;
