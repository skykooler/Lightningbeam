/// Preset Browser pane - asset and preset library
///
/// This will eventually show a file browser for presets.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct PresetBrowserPane {}

impl PresetBrowserPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for PresetBrowserPane {
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
            egui::Color32::from_rgb(50, 45, 30),
        );

        let text = "Preset Browser\n(TODO: Implement file browser)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Preset Browser"
    }
}
