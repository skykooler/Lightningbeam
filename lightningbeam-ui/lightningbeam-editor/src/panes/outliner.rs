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
        shared: &mut SharedPaneState,
    ) {
        // Placeholder rendering
        let bg = shared.theme.bg_color(&["#outliner", ".pane-content"], ui.ctx(), egui::Color32::from_rgb(40, 50, 30));
        ui.painter().rect_filled(rect, 0.0, bg);

        let text = "Outliner\n(TODO: Implement layer tree)";
        let text_color = shared.theme.text_color(&["#outliner", ".text-secondary"], ui.ctx(), egui::Color32::from_gray(150));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            text_color,
        );
    }

    fn name(&self) -> &str {
        "Outliner"
    }
}
