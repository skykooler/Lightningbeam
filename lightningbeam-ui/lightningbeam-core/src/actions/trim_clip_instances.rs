//! Trim clip instances action
//!
//! Handles trimming one or more clip instances by adjusting trim_start and/or trim_end.

use crate::action::Action;
use crate::document::Document;
use crate::layer::AnyLayer;
use std::collections::HashMap;
use uuid::Uuid;

/// Type of trim operation
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrimType {
    /// Trim from the start (adjust trim_start and timeline_start)
    TrimLeft,
    /// Trim from the end (adjust trim_end)
    TrimRight,
}

/// Action that trims clip instances
pub struct TrimClipInstancesAction {
    /// Map of layer IDs to vectors of (clip_instance_id, trim_type, old_values, new_values)
    /// For TrimLeft: (old_trim_start, old_timeline_start, new_trim_start, new_timeline_start)
    /// For TrimRight: (old_trim_end, new_trim_end) - stored as Option<f64>
    layer_trims: HashMap<Uuid, Vec<(Uuid, TrimType, TrimData, TrimData)>>,
}

/// Trim data that can represent either left or right trim values
#[derive(Debug, Clone)]
pub struct TrimData {
    /// For TrimLeft: trim_start value
    /// For TrimRight: trim_end value (Option because it can be None)
    pub trim_value: Option<f64>,
    /// For TrimLeft: timeline_start value (where the clip appears on timeline)
    /// For TrimRight: unused (None)
    pub timeline_start: Option<f64>,
}

impl TrimData {
    /// Create TrimData for left trim
    pub fn left(trim_start: f64, timeline_start: f64) -> Self {
        Self {
            trim_value: Some(trim_start),
            timeline_start: Some(timeline_start),
        }
    }

    /// Create TrimData for right trim
    pub fn right(trim_end: Option<f64>) -> Self {
        Self {
            trim_value: trim_end,
            timeline_start: None,
        }
    }
}

impl TrimClipInstancesAction {
    /// Create a new trim clip instances action
    pub fn new(layer_trims: HashMap<Uuid, Vec<(Uuid, TrimType, TrimData, TrimData)>>) -> Self {
        Self { layer_trims }
    }
}

impl Action for TrimClipInstancesAction {
    fn execute(&mut self, document: &mut Document) {
        for (layer_id, trims) in &self.layer_trims {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Apply trims
            for (clip_id, trim_type, _old, new) in trims {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    match trim_type {
                        TrimType::TrimLeft => {
                            if let (Some(new_trim), Some(new_timeline)) =
                                (new.trim_value, new.timeline_start)
                            {
                                clip_instance.trim_start = new_trim;
                                clip_instance.timeline_start = new_timeline;
                            }
                        }
                        TrimType::TrimRight => {
                            clip_instance.trim_end = new.trim_value;
                        }
                    }
                }
            }
        }
    }

    fn rollback(&mut self, document: &mut Document) {
        for (layer_id, trims) in &self.layer_trims {
            let layer = match document.get_layer_mut(layer_id) {
                Some(l) => l,
                None => continue,
            };

            // Get mutable reference to clip_instances for this layer type
            let clip_instances = match layer {
                AnyLayer::Vector(vl) => &mut vl.clip_instances,
                AnyLayer::Audio(al) => &mut al.clip_instances,
                AnyLayer::Video(vl) => &mut vl.clip_instances,
            };

            // Restore original trim values
            for (clip_id, trim_type, old, _new) in trims {
                if let Some(clip_instance) = clip_instances.iter_mut().find(|ci| ci.id == *clip_id)
                {
                    match trim_type {
                        TrimType::TrimLeft => {
                            if let (Some(old_trim), Some(old_timeline)) =
                                (old.trim_value, old.timeline_start)
                            {
                                clip_instance.trim_start = old_trim;
                                clip_instance.timeline_start = old_timeline;
                            }
                        }
                        TrimType::TrimRight => {
                            clip_instance.trim_end = old.trim_value;
                        }
                    }
                }
            }
        }
    }

    fn description(&self) -> String {
        let total_count: usize = self.layer_trims.values().map(|v| v.len()).sum();
        if total_count == 1 {
            "Trim clip instance".to_string()
        } else {
            format!("Trim {} clip instances", total_count)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::ClipInstance;
    use crate::layer::VectorLayer;

    #[test]
    fn test_trim_left_action() {
        let mut document = Document::new("Test");

        // Create a clip ID (ClipInstance references clip by ID)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.timeline_start = 0.0;
        clip_instance.trim_start = 0.0;
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create trim action: trim 2 seconds from left
        let mut layer_trims = HashMap::new();
        layer_trims.insert(
            layer_id,
            vec![(
                instance_id,
                TrimType::TrimLeft,
                TrimData::left(0.0, 0.0),
                TrimData::left(2.0, 2.0),
            )],
        );

        let mut action = TrimClipInstancesAction::new(layer_trims);

        // Execute
        action.execute(&mut document);

        // Verify trim applied
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_start, 2.0);
            assert_eq!(instance.timeline_start, 2.0);
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_start, 0.0);
            assert_eq!(instance.timeline_start, 0.0);
        }
    }

    #[test]
    fn test_trim_right_action() {
        let mut document = Document::new("Test");

        // Create a clip ID (ClipInstance references clip by ID)
        let clip_id = uuid::Uuid::new_v4();

        let mut vector_layer = VectorLayer::new("Layer 1");

        let mut clip_instance = ClipInstance::new(clip_id);
        clip_instance.trim_end = None; // Full duration
        let instance_id = clip_instance.id;
        vector_layer.clip_instances.push(clip_instance);

        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Create trim action: trim to 8 seconds from right
        let mut layer_trims = HashMap::new();
        layer_trims.insert(
            layer_id,
            vec![(
                instance_id,
                TrimType::TrimRight,
                TrimData::right(None),
                TrimData::right(Some(8.0)),
            )],
        );

        let mut action = TrimClipInstancesAction::new(layer_trims);

        // Execute
        action.execute(&mut document);

        // Verify trim applied
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_end, Some(8.0));
        }

        // Rollback
        action.rollback(&mut document);

        // Verify restored
        if let Some(AnyLayer::Vector(layer)) = document.get_layer(&layer_id) {
            let instance = layer
                .clip_instances
                .iter()
                .find(|ci| ci.id == instance_id)
                .unwrap();
            assert_eq!(instance.trim_end, None);
        }
    }
}
