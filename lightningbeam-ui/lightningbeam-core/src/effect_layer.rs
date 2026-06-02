//! Effect layer type for Lightningbeam
//!
//! An EffectLayer applies visual effects to the composition below it.
//! Effect instances are stored as `ClipInstance` objects where `clip_id`
//! references an `EffectDefinition`.

use crate::clip::ClipInstance;
use crate::layer::{Layer, LayerTrait, LayerType};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Layer type that applies visual effects to the composition
///
/// Effect instances are represented as `ClipInstance` objects.
/// The `clip_id` field references an `EffectDefinition` rather than a clip.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EffectLayer {
    /// Base layer properties
    pub layer: Layer,
    /// Effect instances (as ClipInstances referencing EffectDefinitions)
    pub clip_instances: Vec<ClipInstance>,
}

impl LayerTrait for EffectLayer {
    fn id(&self) -> Uuid {
        self.layer.id
    }

    fn name(&self) -> &str {
        &self.layer.name
    }

    fn set_name(&mut self, name: String) {
        self.layer.name = name;
    }

    fn has_custom_name(&self) -> bool {
        self.layer.has_custom_name
    }

    fn set_has_custom_name(&mut self, custom: bool) {
        self.layer.has_custom_name = custom;
    }

    fn visible(&self) -> bool {
        self.layer.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.layer.visible = visible;
    }

    fn opacity(&self) -> f64 {
        self.layer.opacity
    }

    fn set_opacity(&mut self, opacity: f64) {
        self.layer.opacity = opacity;
    }

    fn volume(&self) -> f64 {
        self.layer.volume
    }

    fn set_volume(&mut self, volume: f64) {
        self.layer.volume = volume;
    }

    fn muted(&self) -> bool {
        self.layer.muted
    }

    fn set_muted(&mut self, muted: bool) {
        self.layer.muted = muted;
    }

    fn soloed(&self) -> bool {
        self.layer.soloed
    }

    fn set_soloed(&mut self, soloed: bool) {
        self.layer.soloed = soloed;
    }

    fn locked(&self) -> bool {
        self.layer.locked
    }

    fn set_locked(&mut self, locked: bool) {
        self.layer.locked = locked;
    }
}

impl EffectLayer {
    /// Create a new effect layer
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            layer: Layer::new(LayerType::Effect, name),
            clip_instances: Vec::new(),
        }
    }

    /// Create with a specific ID
    pub fn with_id(id: Uuid, name: impl Into<String>) -> Self {
        Self {
            layer: Layer::with_id(id, LayerType::Effect, name),
            clip_instances: Vec::new(),
        }
    }

    /// Add a clip instance (effect) to this layer
    pub fn add_clip_instance(&mut self, instance: ClipInstance) -> Uuid {
        let id = instance.id;
        self.clip_instances.push(instance);
        id
    }

    /// Insert a clip instance at a specific index
    pub fn insert_clip_instance(&mut self, index: usize, instance: ClipInstance) -> Uuid {
        let id = instance.id;
        let index = index.min(self.clip_instances.len());
        self.clip_instances.insert(index, instance);
        id
    }

    /// Remove a clip instance by ID
    pub fn remove_clip_instance(&mut self, id: &Uuid) -> Option<ClipInstance> {
        if let Some(index) = self.clip_instances.iter().position(|e| &e.id == id) {
            Some(self.clip_instances.remove(index))
        } else {
            None
        }
    }

    /// Get a clip instance by ID
    pub fn get_clip_instance(&self, id: &Uuid) -> Option<&ClipInstance> {
        self.clip_instances.iter().find(|e| &e.id == id)
    }

    /// Get a mutable clip instance by ID
    pub fn get_clip_instance_mut(&mut self, id: &Uuid) -> Option<&mut ClipInstance> {
        self.clip_instances.iter_mut().find(|e| &e.id == id)
    }

    /// Get all clip instances (effects) that are active at a given time.
    ///
    /// `time_secs` is the playback time in seconds; `bpm` is used to convert
    /// `timeline_start` (beats) to seconds for comparison.
    pub fn active_clip_instances_at(&self, time_secs: f64, tempo_map: &crate::tempo_map::TempoMap) -> Vec<&ClipInstance> {
        use crate::effect::EFFECT_DURATION;
        let time_beats = tempo_map.inverse_transform(time_secs);
        self.clip_instances
            .iter()
            .filter(|e| {
                let end = e.timeline_start + e.effective_duration(EFFECT_DURATION, tempo_map);
                time_beats >= e.timeline_start && time_beats < end
            })
            .collect()
    }

    /// Get the index of a clip instance
    pub fn clip_instance_index(&self, id: &Uuid) -> Option<usize> {
        self.clip_instances.iter().position(|e| &e.id == id)
    }

    /// Move a clip instance to a new position in the layer
    pub fn move_clip_instance(&mut self, id: &Uuid, new_index: usize) -> bool {
        if let Some(current_index) = self.clip_instance_index(id) {
            let instance = self.clip_instances.remove(current_index);
            let new_index = new_index.min(self.clip_instances.len());
            self.clip_instances.insert(new_index, instance);
            true
        } else {
            false
        }
    }

    /// Reorder clip instances by providing a list of IDs in desired order
    pub fn reorder_clip_instances(&mut self, order: &[Uuid]) {
        let mut new_order = Vec::with_capacity(self.clip_instances.len());

        // Add instances in the specified order
        for id in order {
            if let Some(index) = self.clip_instances.iter().position(|e| &e.id == id) {
                new_order.push(self.clip_instances.remove(index));
            }
        }

        // Append any instances not in the order list
        new_order.append(&mut self.clip_instances);
        self.clip_instances = new_order;
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::{EffectCategory, EffectDefinition, EffectParameterDef};

    fn create_test_effect_def() -> EffectDefinition {
        EffectDefinition::new(
            "Test Effect",
            EffectCategory::Color,
            "// shader code",
            vec![EffectParameterDef::float_range("intensity", "Intensity", 1.0, 0.0, 2.0)],
        )
    }

    #[test]
    fn test_effect_layer_creation() {
        let layer = EffectLayer::new("Effects");
        assert_eq!(layer.name(), "Effects");
        assert_eq!(layer.clip_instances.len(), 0);
    }

    #[test]
    fn test_add_effect() {
        let mut layer = EffectLayer::new("Effects");
        let def = create_test_effect_def();
        let effect = def.create_instance(0.0, 10.0);
        let effect_id = effect.id;

        let id = layer.add_clip_instance(effect);
        assert_eq!(id, effect_id);
        assert_eq!(layer.clip_instances.len(), 1);
        assert!(layer.get_clip_instance(&effect_id).is_some());
    }

    #[test]
    fn test_active_effects() {
        let mut layer = EffectLayer::new("Effects");
        let def = create_test_effect_def();

        // Effect 1: active from 0 to 5
        let effect1 = def.create_instance(0.0, 5.0);
        layer.add_clip_instance(effect1);

        // Effect 2: active from 3 to 10
        let effect2 = def.create_instance(3.0, 7.0); // 3.0 + 7.0 = 10.0 end
        layer.add_clip_instance(effect2);

        // At time 2: only effect1 active
        assert_eq!(layer.active_clip_instances_at(2.0, 60.0).len(), 1);

        // At time 4: both effects active
        assert_eq!(layer.active_clip_instances_at(4.0, 60.0).len(), 2);

        // At time 7: only effect2 active
        assert_eq!(layer.active_clip_instances_at(7.0, 60.0).len(), 1);
    }

    #[test]
    fn test_effect_reordering() {
        let mut layer = EffectLayer::new("Effects");
        let def = create_test_effect_def();

        let effect1 = def.create_instance(0.0, 10.0);
        let id1 = effect1.id;
        layer.add_clip_instance(effect1);

        let effect2 = def.create_instance(0.0, 10.0);
        let id2 = effect2.id;
        layer.add_clip_instance(effect2);

        // Initially: [id1, id2]
        assert_eq!(layer.clip_instance_index(&id1), Some(0));
        assert_eq!(layer.clip_instance_index(&id2), Some(1));

        // Move id1 to index 1: [id2, id1]
        layer.move_clip_instance(&id1, 1);
        assert_eq!(layer.clip_instance_index(&id1), Some(1));
        assert_eq!(layer.clip_instance_index(&id2), Some(0));
    }
}
