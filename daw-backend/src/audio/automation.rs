/// Automation system for parameter modulation over time
use serde::{Deserialize, Serialize};

/// Unique identifier for automation lanes
pub type AutomationLaneId = u32;

/// Unique identifier for parameters that can be automated
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ParameterId {
    /// Track volume
    TrackVolume,
    /// Track pan
    TrackPan,
    /// Effect parameter (effect_index, param_id)
    EffectParameter(usize, u32),
    /// Metatrack time stretch
    TimeStretch,
    /// Metatrack offset
    TimeOffset,
}

/// Type of interpolation curve between automation points
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CurveType {
    /// Linear interpolation (straight line)
    Linear,
    /// Exponential curve (smooth acceleration)
    Exponential,
    /// S-curve (ease in/out)
    SCurve,
    /// Step (no interpolation, jump to next value)
    Step,
}

/// A single automation point
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AutomationPoint {
    /// Time in seconds
    pub time: f64,
    /// Parameter value (normalized 0.0 to 1.0, or actual value depending on parameter)
    pub value: f32,
    /// Curve type to next point
    pub curve: CurveType,
}

impl AutomationPoint {
    /// Create a new automation point
    pub fn new(time: f64, value: f32, curve: CurveType) -> Self {
        Self { time, value, curve }
    }
}

/// An automation lane for a specific parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLane {
    /// Unique identifier for this lane
    pub id: AutomationLaneId,
    /// Which parameter this lane controls
    pub parameter_id: ParameterId,
    /// Sorted list of automation points
    points: Vec<AutomationPoint>,
    /// Whether this lane is enabled
    pub enabled: bool,
}

impl AutomationLane {
    /// Create a new automation lane
    pub fn new(id: AutomationLaneId, parameter_id: ParameterId) -> Self {
        Self {
            id,
            parameter_id,
            points: Vec::new(),
            enabled: true,
        }
    }

    /// Add an automation point, maintaining sorted order
    pub fn add_point(&mut self, point: AutomationPoint) {
        // Find insertion position to maintain sorted order
        let pos = self.points.binary_search_by(|p| {
            p.time.partial_cmp(&point.time).unwrap_or(std::cmp::Ordering::Equal)
        });

        match pos {
            Ok(idx) => {
                // Replace existing point at same time
                self.points[idx] = point;
            }
            Err(idx) => {
                // Insert at correct position
                self.points.insert(idx, point);
            }
        }
    }

    /// Remove point at specific time
    pub fn remove_point_at_time(&mut self, time: f64, tolerance: f64) -> bool {
        if let Some(idx) = self.points.iter().position(|p| (p.time - time).abs() < tolerance) {
            self.points.remove(idx);
            true
        } else {
            false
        }
    }

    /// Remove all points
    pub fn clear(&mut self) {
        self.points.clear();
    }

    /// Get all points
    pub fn points(&self) -> &[AutomationPoint] {
        &self.points
    }

    /// Get value at a specific time with interpolation
    pub fn evaluate(&self, time: f64) -> Option<f32> {
        if !self.enabled || self.points.is_empty() {
            return None;
        }

        // Before first point
        if time <= self.points[0].time {
            return Some(self.points[0].value);
        }

        // After last point
        if time >= self.points[self.points.len() - 1].time {
            return Some(self.points[self.points.len() - 1].value);
        }

        // Find surrounding points
        for i in 0..self.points.len() - 1 {
            let p1 = &self.points[i];
            let p2 = &self.points[i + 1];

            if time >= p1.time && time <= p2.time {
                return Some(interpolate(p1, p2, time));
            }
        }

        None
    }

    /// Get number of points
    pub fn point_count(&self) -> usize {
        self.points.len()
    }
}

/// Interpolate between two automation points based on curve type
fn interpolate(p1: &AutomationPoint, p2: &AutomationPoint, time: f64) -> f32 {
    // Calculate normalized position between points (0.0 to 1.0)
    let t = if p2.time == p1.time {
        0.0
    } else {
        ((time - p1.time) / (p2.time - p1.time)) as f32
    };

    // Apply curve
    let curved_t = match p1.curve {
        CurveType::Linear => t,
        CurveType::Exponential => {
            // Exponential curve: y = x^2
            t * t
        }
        CurveType::SCurve => {
            // Smooth S-curve using smoothstep
            smoothstep(t)
        }
        CurveType::Step => {
            // Step: hold value until next point
            return p1.value;
        }
    };

    // Linear interpolation with curved t
    p1.value + (p2.value - p1.value) * curved_t
}

/// Smoothstep function for S-curve interpolation
/// Returns a smooth curve from 0 to 1
#[inline]
fn smoothstep(t: f32) -> f32 {
    // Clamp to [0, 1]
    let t = t.clamp(0.0, 1.0);
    // 3t^2 - 2t^3
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_points_sorted() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(2.0, 0.5, CurveType::Linear));
        lane.add_point(AutomationPoint::new(1.0, 0.3, CurveType::Linear));
        lane.add_point(AutomationPoint::new(3.0, 0.8, CurveType::Linear));

        assert_eq!(lane.points().len(), 3);
        assert_eq!(lane.points()[0].time, 1.0);
        assert_eq!(lane.points()[1].time, 2.0);
        assert_eq!(lane.points()[2].time, 3.0);
    }

    #[test]
    fn test_replace_point_at_same_time() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(1.0, 0.3, CurveType::Linear));
        lane.add_point(AutomationPoint::new(1.0, 0.5, CurveType::Linear));

        assert_eq!(lane.points().len(), 1);
        assert_eq!(lane.points()[0].value, 0.5);
    }

    #[test]
    fn test_linear_interpolation() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(0.0, 0.0, CurveType::Linear));
        lane.add_point(AutomationPoint::new(1.0, 1.0, CurveType::Linear));

        assert_eq!(lane.evaluate(0.0), Some(0.0));
        assert_eq!(lane.evaluate(0.5), Some(0.5));
        assert_eq!(lane.evaluate(1.0), Some(1.0));
    }

    #[test]
    fn test_step_interpolation() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(0.0, 0.5, CurveType::Step));
        lane.add_point(AutomationPoint::new(1.0, 1.0, CurveType::Step));

        assert_eq!(lane.evaluate(0.0), Some(0.5));
        assert_eq!(lane.evaluate(0.5), Some(0.5));
        assert_eq!(lane.evaluate(0.99), Some(0.5));
        assert_eq!(lane.evaluate(1.0), Some(1.0));
    }

    #[test]
    fn test_evaluate_outside_range() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(1.0, 0.5, CurveType::Linear));
        lane.add_point(AutomationPoint::new(2.0, 1.0, CurveType::Linear));

        // Before first point
        assert_eq!(lane.evaluate(0.0), Some(0.5));
        // After last point
        assert_eq!(lane.evaluate(3.0), Some(1.0));
    }

    #[test]
    fn test_disabled_lane() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(0.0, 0.5, CurveType::Linear));
        lane.enabled = false;

        assert_eq!(lane.evaluate(0.0), None);
    }

    #[test]
    fn test_remove_point() {
        let mut lane = AutomationLane::new(0, ParameterId::TrackVolume);

        lane.add_point(AutomationPoint::new(1.0, 0.5, CurveType::Linear));
        lane.add_point(AutomationPoint::new(2.0, 0.8, CurveType::Linear));

        assert!(lane.remove_point_at_time(1.0, 0.001));
        assert_eq!(lane.points().len(), 1);
        assert_eq!(lane.points()[0].time, 2.0);
    }
}
