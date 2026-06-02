//! Gap handling modes for paint bucket fill
//!
//! When curves don't precisely intersect but come within tolerance distance,
//! we need to decide how to bridge the gap. This module defines the available
//! strategies.

/// Mode for handling gaps between curves during paint bucket fill
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapHandlingMode {
    /// Modify curves to connect at the midpoint of closest approach
    ///
    /// When two curves come within tolerance distance but don't exactly intersect,
    /// this mode will:
    /// 1. Find the closest approach point between the curves
    /// 2. Calculate the midpoint between the two closest points
    /// 3. Split both curves at their respective t parameters
    /// 4. Snap the endpoints to the midpoint
    ///
    /// This creates a precise intersection by modifying the curve geometry.
    /// The modification is temporary (only for the fill operation) and doesn't
    /// affect the original shapes.
    SnapAndSplit,

    /// Insert a line segment to bridge the gap
    ///
    /// When two curves come within tolerance distance but don't exactly intersect,
    /// this mode will:
    /// 1. Find the closest approach point between the curves
    /// 2. Insert a straight line segment from the end of one curve to the start of the next
    ///
    /// This preserves the original curve geometry but adds artificial connecting segments.
    /// Bridge segments are included in the final filled path.
    BridgeSegment,
}

impl Default for GapHandlingMode {
    fn default() -> Self {
        // Default to bridge segments as it's less invasive
        GapHandlingMode::BridgeSegment
    }
}

impl GapHandlingMode {
    /// Get a human-readable description of this mode
    pub fn description(&self) -> &'static str {
        match self {
            GapHandlingMode::SnapAndSplit => {
                "Snap curves to midpoint and split at intersection"
            }
            GapHandlingMode::BridgeSegment => {
                "Insert line segments to bridge gaps between curves"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mode() {
        assert_eq!(GapHandlingMode::default(), GapHandlingMode::BridgeSegment);
    }

    #[test]
    fn test_description() {
        let snap = GapHandlingMode::SnapAndSplit;
        let bridge = GapHandlingMode::BridgeSegment;

        assert!(!snap.description().is_empty());
        assert!(!bridge.description().is_empty());
        assert_ne!(snap.description(), bridge.description());
    }

    #[test]
    fn test_equality() {
        let mode1 = GapHandlingMode::SnapAndSplit;
        let mode2 = GapHandlingMode::SnapAndSplit;
        let mode3 = GapHandlingMode::BridgeSegment;

        assert_eq!(mode1, mode2);
        assert_ne!(mode1, mode3);
    }
}
