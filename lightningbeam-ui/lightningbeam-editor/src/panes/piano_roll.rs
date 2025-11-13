/// Piano Roll pane - MIDI editor
///
/// This will eventually render a piano roll with Vello.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct PianoRollPane {}

impl PianoRollPane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for PianoRollPane {
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
            egui::Color32::from_rgb(55, 35, 45),
        );

        let text = "Piano Roll\n(TODO: Implement MIDI editor)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Piano Roll"
    }
}
