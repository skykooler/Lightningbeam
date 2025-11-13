/// Outliner pane - layer hierarchy view
///
/// This will eventually show a tree view of layers.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct OutlinerPane {
    // TODO: Add tree expansion state
}

impl OutlinerPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for OutlinerPane {
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
            egui::Color32::from_rgb(40, 50, 30),
        );

        let text = "Outliner\n(TODO: Implement layer tree)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Outliner"
    }
}
