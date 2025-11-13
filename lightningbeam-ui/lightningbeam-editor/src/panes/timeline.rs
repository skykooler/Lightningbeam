/// Timeline pane - frame-based animation timeline
///
/// This will eventually render keyframes, layers, and playback controls.
/// For now, it's a placeholder.

use eframe::egui;
use super::{NodePath, PaneRenderer, SharedPaneState};

pub struct TimelinePane {
    // TODO: Add state for zoom, scroll, playback, etc.
}

impl TimelinePane {
    pub fn new() -> Self {
        Self {}
    }
}

impl PaneRenderer for TimelinePane {
    fn render_header(&mut self, ui: &mut egui::Ui, _shared: &mut SharedPaneState) -> bool {
        // TODO: Add playback controls (play/pause, frame counter, zoom)
        ui.horizontal(|ui| {
            ui.label("⏯");
            ui.label("Frame: 0");
            ui.label("FPS: 24");
        });
        true // Header was rendered
    }

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
            egui::Color32::from_rgb(40, 30, 50),
        );

        let text = "Timeline Pane\n(TODO: Implement frame scrubbing)";
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    fn name(&self) -> &str {
        "Timeline"
    }
}
