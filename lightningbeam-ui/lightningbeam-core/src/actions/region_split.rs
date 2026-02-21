//! Region split action
//!
//! Commits a temporary region-based shape split permanently.
//! Replaces original shapes with their inside and outside portions.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use crate::shape::Shape;
use uuid::Uuid;
use vello::kurbo::BezPath;

/// One shape split entry for the action
#[derive(Clone, Debug)]
struct SplitEntry {
    /// The original shape (for rollback)
    original_shape: Shape,
    /// The inside portion shape
    inside_shape: Shape,
    /// The outside portion shape
    outside_shape: Shape,
}

/// Action that commits a region split — replacing original shapes with
/// their inside and outside portions.
pub struct RegionSplitAction {
    layer_id: Uuid,
    time: f64,
    splits: Vec<SplitEntry>,
}

impl RegionSplitAction {
    /// Create a new region split action.
    ///
    /// Each tuple is (original_shape, inside_path, inside_id, outside_path, outside_id).
    pub fn new(
        layer_id: Uuid,
        time: f64,
        split_data: Vec<(Shape, BezPath, Uuid, BezPath, Uuid)>,
    ) -> Self {
        let splits = split_data
            .into_iter()
            .map(|(original, inside_path, inside_id, outside_path, outside_id)| {
                let mut inside_shape = original.clone();
                inside_shape.id = inside_id;
                inside_shape.versions[0].path = inside_path;

                let mut outside_shape = original.clone();
                outside_shape.id = outside_id;
                outside_shape.versions[0].path = outside_path;

                SplitEntry {
                    original_shape: original,
                    inside_shape,
                    outside_shape,
                }
            })
            .collect();

        Self {
            layer_id,
            time,
            splits,
        }
    }
}

impl Action for RegionSplitAction {
    fn execute(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vector_layer = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        for split in &self.splits {
            // Remove original
            vector_layer.remove_shape_from_keyframe(&split.original_shape.id, self.time);
            // Add inside and outside portions
            vector_layer.add_shape_to_keyframe(split.inside_shape.clone(), self.time);
            vector_layer.add_shape_to_keyframe(split.outside_shape.clone(), self.time);
        }

        Ok(())
    }

    fn rollback(&mut self, document: &mut Document) -> Result<(), String> {
        let layer = document
            .get_layer_mut(&self.layer_id)
            .ok_or_else(|| format!("Layer {} not found", self.layer_id))?;

        let vector_layer = match layer {
            AnyLayer::Vector(vl) => vl,
            _ => return Err("Not a vector layer".to_string()),
        };

        for split in &self.splits {
            // Remove inside and outside portions
            vector_layer.remove_shape_from_keyframe(&split.inside_shape.id, self.time);
            vector_layer.remove_shape_from_keyframe(&split.outside_shape.id, self.time);
            // Restore original
            vector_layer.add_shape_to_keyframe(split.original_shape.clone(), self.time);
        }

        Ok(())
    }

    fn description(&self) -> String {
        let count = self.splits.len();
        if count == 1 {
            "Region split shape".to_string()
        } else {
            format!("Region split {} shapes", count)
        }
    }
}
