use serde::{Deserialize, Serialize};

/// Complete layout definition matching JS schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutDefinition {
    pub name: String,
    pub description: String,
    pub layout: LayoutNode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom: Option<bool>,
}

/// Recursive layout tree node
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum LayoutNode {
    Pane {
        name: String,
    },
    #[serde(rename = "horizontal-grid")]
    HorizontalGrid {
        percent: f32,
        children: [Box<LayoutNode>; 2],
    },
    #[serde(rename = "vertical-grid")]
    VerticalGrid {
        percent: f32,
        children: [Box<LayoutNode>; 2],
    },
}

/// Pane types available in the editor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneType {
    Stage,
    Timeline,
    TimelineV2,
    Toolbar,
    Infopanel,
    Outliner,
    Piano,
    PianoRoll,
    NodeEditor,
    PresetBrowser,
}

impl PaneType {
    /// Convert from camelCase name (from JSON)
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "stage" => Some(Self::Stage),
            "timeline" => Some(Self::Timeline),
            "timelineV2" => Some(Self::TimelineV2),
            "toolbar" => Some(Self::Toolbar),
            "infopanel" => Some(Self::Infopanel),
            "outliner" | "outlineer" => Some(Self::Outliner), // Handle typo in JS
            "piano" => Some(Self::Piano),
            "pianoRoll" => Some(Self::PianoRoll),
            "nodeEditor" => Some(Self::NodeEditor),
            "presetBrowser" => Some(Self::PresetBrowser),
            _ => None,
        }
    }

    /// Convert to camelCase name (for JSON)
    pub fn to_name(&self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Timeline => "timeline",
            Self::TimelineV2 => "timelineV2",
            Self::Toolbar => "toolbar",
            Self::Infopanel => "infopanel",
            Self::Outliner => "outliner",
            Self::Piano => "piano",
            Self::PianoRoll => "pianoRoll",
            Self::NodeEditor => "nodeEditor",
            Self::PresetBrowser => "presetBrowser",
        }
    }

    /// Convert to kebab-case for display
    pub fn to_kebab_case(&self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Timeline => "timeline",
            Self::TimelineV2 => "timeline-v2",
            Self::Toolbar => "toolbar",
            Self::Infopanel => "infopanel",
            Self::Outliner => "outliner",
            Self::Piano => "piano",
            Self::PianoRoll => "piano-roll",
            Self::NodeEditor => "node-editor",
            Self::PresetBrowser => "preset-browser",
        }
    }

    /// Get display name for UI
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Timeline => "Timeline",
            Self::TimelineV2 => "Timeline V2",
            Self::Toolbar => "Toolbar",
            Self::Infopanel => "Info Panel",
            Self::Outliner => "Outliner",
            Self::Piano => "Piano",
            Self::PianoRoll => "Piano Roll",
            Self::NodeEditor => "Node Editor",
            Self::PresetBrowser => "Preset Browser",
        }
    }

    /// Get all pane types
    pub fn all() -> &'static [Self] {
        &[
            Self::Stage,
            Self::Timeline,
            Self::TimelineV2,
            Self::Toolbar,
            Self::Infopanel,
            Self::Outliner,
            Self::Piano,
            Self::PianoRoll,
            Self::NodeEditor,
            Self::PresetBrowser,
        ]
    }
}
