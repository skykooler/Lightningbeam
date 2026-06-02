//! Set shape instance properties action — STUB: needs DCEL rewrite

use crate::action::Action;
use crate::document::Document;
use uuid::Uuid;

/// Individual property change for a shape instance
#[derive(Clone, Debug)]
pub enum InstancePropertyChange {
    X(f64),
    Y(f64),
    Rotation(f64),
    ScaleX(f64),
    ScaleY(f64),
    SkewX(f64),
    SkewY(f64),
    Opacity(f64),
}

impl InstancePropertyChange {
    pub fn value(&self) -> f64 {
        match self {
            InstancePropertyChange::X(v) => *v,
            InstancePropertyChange::Y(v) => *v,
            InstancePropertyChange::Rotation(v) => *v,
            InstancePropertyChange::ScaleX(v) => *v,
            InstancePropertyChange::ScaleY(v) => *v,
            InstancePropertyChange::SkewX(v) => *v,
            InstancePropertyChange::SkewY(v) => *v,
            InstancePropertyChange::Opacity(v) => *v,
        }
    }
}

/// Action that sets a property on one or more shapes in a keyframe
/// TODO: Replace with DCEL-based property changes
pub struct SetInstancePropertiesAction {
    layer_id: Uuid,
    time: f64,
    shape_changes: Vec<(Uuid, Option<f64>)>,
    property: InstancePropertyChange,
}

impl SetInstancePropertiesAction {
    pub fn new(layer_id: Uuid, time: f64, shape_id: Uuid, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            time,
            shape_changes: vec![(shape_id, None)],
            property,
        }
    }

    pub fn new_batch(layer_id: Uuid, time: f64, shape_ids: Vec<Uuid>, property: InstancePropertyChange) -> Self {
        Self {
            layer_id,
            time,
            shape_changes: shape_ids.into_iter().map(|id| (id, None)).collect(),
            property,
        }
    }
}

impl Action for SetInstancePropertiesAction {
    fn execute(&mut self, _document: &mut Document) -> Result<(), String> {
        let _ = (&self.layer_id, self.time, &self.shape_changes, &self.property);
        Ok(())
    }

    fn rollback(&mut self, _document: &mut Document) -> Result<(), String> {
        Ok(())
    }

    fn description(&self) -> String {
        let property_name = match &self.property {
            InstancePropertyChange::X(_) => "X position",
            InstancePropertyChange::Y(_) => "Y position",
            InstancePropertyChange::Rotation(_) => "rotation",
            InstancePropertyChange::ScaleX(_) => "scale X",
            InstancePropertyChange::ScaleY(_) => "scale Y",
            InstancePropertyChange::SkewX(_) => "skew X",
            InstancePropertyChange::SkewY(_) => "skew Y",
            InstancePropertyChange::Opacity(_) => "opacity",
        };

        if self.shape_changes.len() == 1 {
            format!("Set {}", property_name)
        } else {
            format!("Set {} on {} shapes", property_name, self.shape_changes.len())
        }
    }
}
