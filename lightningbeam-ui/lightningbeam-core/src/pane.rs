/// Pane system for the layout manager
///
/// Each pane has:
/// - An icon button (top-left) for pane type selection
/// - Optional header with controls (e.g., Timeline playback controls)
/// - Content area (main pane body)

use serde::{Deserialize, Serialize};

/// Pane type enum matching the layout system
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PaneType {
    /// Main animation canvas
    Stage,
    /// Frame-based timeline (matches timelineV2 from JS, but just "timeline" in Rust)
    #[serde(rename = "timelineV2")]
    Timeline,
    /// Tool selection bar
    Toolbar,
    /// Property/info panel
    Infopanel,
    /// Layer hierarchy
    #[serde(rename = "outlineer")]
    Outliner,
    /// MIDI piano roll editor
    PianoRoll,
    /// Virtual piano keyboard for live MIDI input
    VirtualPiano,
    /// Node-based editor
    NodeEditor,
    /// Preset/asset browser
    PresetBrowser,
    /// Asset library for browsing clips
    AssetLibrary,
}

impl PaneType {
    /// Get display name for the pane type
    pub fn display_name(self) -> &'static str {
        match self {
            PaneType::Stage => "Stage",
            PaneType::Timeline => "Timeline",
            PaneType::Toolbar => "Toolbar",
            PaneType::Infopanel => "Info Panel",
            PaneType::Outliner => "Outliner",
            PaneType::PianoRoll => "Piano Roll",
            PaneType::VirtualPiano => "Virtual Piano",
            PaneType::NodeEditor => "Node Editor",
            PaneType::PresetBrowser => "Preset Browser",
            PaneType::AssetLibrary => "Asset Library",
        }
    }

    /// Get SVG icon file name for the pane type
    /// Path is relative to ~/Dev/Lightningbeam-2/src/assets/
    /// TODO: Move assets to lightningbeam-editor/assets/icons/ before release
    pub fn icon_file(self) -> &'static str {
        match self {
            PaneType::Stage => "stage.svg",
            PaneType::Timeline => "timeline.svg",
            PaneType::Toolbar => "toolbar.svg",
            PaneType::Infopanel => "infopanel.svg",
            PaneType::Outliner => "stage.svg", // TODO: needs own icon
            PaneType::PianoRoll => "piano-roll.svg",
            PaneType::VirtualPiano => "piano.svg",
            PaneType::NodeEditor => "node-editor.svg",
            PaneType::PresetBrowser => "stage.svg", // TODO: needs own icon
            PaneType::AssetLibrary => "stage.svg", // TODO: needs own icon
        }
    }

    /// Parse pane type from string name (case-insensitive)
    /// Accepts both JS names (timelineV2) and Rust names (timeline)
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "stage" => Some(PaneType::Stage),
            "timeline" | "timelinev2" => Some(PaneType::Timeline),
            "toolbar" => Some(PaneType::Toolbar),
            "infopanel" => Some(PaneType::Infopanel),
            "outlineer" | "outliner" => Some(PaneType::Outliner),
            "pianoroll" => Some(PaneType::PianoRoll),
            "virtualpiano" => Some(PaneType::VirtualPiano),
            "nodeeditor" => Some(PaneType::NodeEditor),
            "presetbrowser" => Some(PaneType::PresetBrowser),
            "assetlibrary" => Some(PaneType::AssetLibrary),
            _ => None,
        }
    }

    /// Get all available pane types
    pub fn all() -> &'static [PaneType] {
        &[
            PaneType::Stage,
            PaneType::Timeline,
            PaneType::Toolbar,
            PaneType::Infopanel,
            PaneType::Outliner,
            PaneType::NodeEditor,
            PaneType::PianoRoll,
            PaneType::VirtualPiano,
            PaneType::PresetBrowser,
            PaneType::AssetLibrary,
        ]
    }

    /// Get the string name for this pane type (used in JSON)
    pub fn to_name(self) -> &'static str {
        match self {
            PaneType::Stage => "stage",
            PaneType::Timeline => "timelineV2",  // JSON uses timelineV2
            PaneType::Toolbar => "toolbar",
            PaneType::Infopanel => "infopanel",
            PaneType::Outliner => "outlineer",  // JSON uses outlineer
            PaneType::PianoRoll => "pianoRoll",
            PaneType::VirtualPiano => "virtualPiano",
            PaneType::NodeEditor => "nodeEditor",
            PaneType::PresetBrowser => "presetBrowser",
            PaneType::AssetLibrary => "assetLibrary",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pane_type_from_name() {
        assert_eq!(PaneType::from_name("stage"), Some(PaneType::Stage));
        assert_eq!(PaneType::from_name("Stage"), Some(PaneType::Stage));
        assert_eq!(PaneType::from_name("STAGE"), Some(PaneType::Stage));
        // Accept both JS name (timelineV2) and Rust name (timeline)
        assert_eq!(PaneType::from_name("timelineV2"), Some(PaneType::Timeline));
        assert_eq!(PaneType::from_name("timeline"), Some(PaneType::Timeline));
        assert_eq!(PaneType::from_name("invalid"), None);
    }

    #[test]
    fn test_pane_type_display() {
        assert_eq!(PaneType::Stage.display_name(), "Stage");
        assert_eq!(PaneType::Timeline.display_name(), "Timeline");
    }

    #[test]
    fn test_pane_type_icons() {
        assert_eq!(PaneType::Stage.icon_file(), "stage.svg");
        assert_eq!(PaneType::Timeline.icon_file(), "timeline.svg");
        assert_eq!(PaneType::NodeEditor.icon_file(), "node-editor.svg");
    }
}
