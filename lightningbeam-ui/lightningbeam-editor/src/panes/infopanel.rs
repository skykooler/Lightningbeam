/// Info Panel pane - displays properties of selected objects
///
/// This will eventually show editable properties.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct InfopanelPane {}

impl InfopanelPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for InfopanelPane {
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
            egui::Color32::from_rgb(30, 50, 40),
        );

        let text = "Info Panel\n(TODO: Implement property editor)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Info Panel"
    }
}
