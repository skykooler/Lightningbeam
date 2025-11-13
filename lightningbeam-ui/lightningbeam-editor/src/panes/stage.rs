/// Stage pane - main animation canvas
///
/// This will eventually render the composited layers using Vello.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct StagePane {
    // TODO: Add state for camera, selection, etc.
}

impl StagePane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for StagePane {
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
            egui::Color32::from_rgb(30, 40, 50),
        );

        let text = "Stage Pane\n(TODO: Implement Vello rendering)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Stage"
    }
}
