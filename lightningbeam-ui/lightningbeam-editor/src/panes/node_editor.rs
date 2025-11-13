/// Node Editor pane - node-based visual programming
///
/// This will eventually render a node graph with Vello.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct NodeEditorPane {}

impl NodeEditorPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for NodeEditorPane {
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        _shared: &mut SharedPaneState,
    ) {
        // Placeholder rendering
        ui.painter().rect_filled(
            rect,
            0.0,
            egui::Color32::from_rgb(30, 45, 50),
        );

        let text = "Node Editor\n(TODO: Implement node graph)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Node Editor"
    }
}
