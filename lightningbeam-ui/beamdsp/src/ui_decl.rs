use serde::{Deserialize, Serialize};

/// Declarative UI layout for a script node, rendered in bottom_ui()
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiDeclaration {
    pub elements: Vec<UiElement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UiElement {
    /// Render a parameter slider/knob
    Param(String),
    /// Render a sample picker dropdown
    Sample(String),
    /// Collapsible group with label
    Group {
        label: String,
        children: Vec<UiElement>,
    },
    /// Drawable canvas area (phase 2)
    Canvas {
        width: f32,
        height: f32,
    },
    /// Vertical spacer
    Spacer(f32),
}
