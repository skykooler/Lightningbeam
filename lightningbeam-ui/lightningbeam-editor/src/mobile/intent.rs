//! Mobile new-file intent picker — the phone equivalent of the desktop start screen.
//!
//! Mirrors the desktop "new project" flow: each intent calls
//! [`EditorApp::create_new_project_with_focus`] (which builds the document, picks a default layer,
//! and switches to editor mode) and then sets the initial mobile stack window so the relevant panes
//! are in view.

use eframe::egui;

use super::icons;
use crate::EditorApp;

const C_DEVICE: egui::Color32 = egui::Color32::from_rgb(0x14, 0x16, 0x1b);
const C_CARD: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x24, 0x2c);
const C_CARD_HOT: egui::Color32 = egui::Color32::from_rgb(0x27, 0x2d, 0x37);
const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);
const C_BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xea, 0xee, 0xf3);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x95, 0xa1);

/// One intent card.
struct Intent {
    label: &'static str,
    icon: &'static str,
    accent: egui::Color32,
    /// Argument to `create_new_project_with_focus` (0=Animation, 1=Video, 2=Music, 5=Painting).
    focus: usize,
    /// Initial mobile stack window: (window_top, window_count).
    window: (usize, usize),
}

fn intents() -> [Intent; 6] {
    let coral = egui::Color32::from_rgb(0xe8, 0x82, 0x6b);
    let cyan = egui::Color32::from_rgb(0x54, 0xc3, 0xe8);
    let amber = egui::Color32::from_rgb(0xf4, 0xa3, 0x40);
    let pink = egui::Color32::from_rgb(0xc7, 0x5b, 0x8a);
    let violet = egui::Color32::from_rgb(0x6a, 0x46, 0x90);
    [
        // Stage indices (see super::STACK): Stage=2, Timeline=3, PianoRoll=4, VirtualPiano=5.
        Intent { label: "Draw", icon: icons::BRUSH, accent: coral, focus: 5, window: (2, 1) },
        Intent { label: "Animate", icon: icons::FILM, accent: cyan, focus: 0, window: (2, 2) },
        Intent { label: "Compose", icon: icons::MUSIC, accent: amber, focus: 2, window: (3, 3) },
        Intent { label: "Record", icon: icons::MIC, accent: pink, focus: 2, window: (3, 3) },
        Intent { label: "Edit video", icon: icons::CLAPPERBOARD, accent: violet, focus: 1, window: (2, 2) },
        Intent { label: "Blank", icon: icons::SQUARE_DASHED, accent: C_DIM, focus: 0, window: (2, 2) },
    ]
}

pub fn render(app: &mut EditorApp, ctx: &egui::Context) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(C_DEVICE))
        .show(ctx, |ui| {
            let rect = ui.available_rect_before_wrap();
            let margin = 16.0;
            let left = rect.left() + margin;
            let right = rect.right() - margin;

            // Header.
            ui.painter().text(
                egui::pos2(left, rect.top() + 22.0),
                egui::Align2::LEFT_TOP,
                "Start something",
                egui::FontId::proportional(22.0),
                C_BRIGHT,
            );

            // Vertical budget: ~2/3 for the intent grid, ~1/3 for the recent list.
            let content_top = rect.top() + 62.0;
            let content_h = (rect.bottom() - margin) - content_top;
            let gap = 10.0;
            let grid_h = content_h * 0.66;

            // 2×3 grid of intent cards filling the grid budget.
            let col_w = (right - left - gap) / 2.0;
            let card_h = (grid_h - 2.0 * gap) / 3.0;
            for (i, intent) in intents().iter().enumerate() {
                let col = (i % 2) as f32;
                let row = (i / 2) as f32;
                let cx = left + col * (col_w + gap);
                let cy = content_top + row * (card_h + gap);
                let card = egui::Rect::from_min_size(egui::pos2(cx, cy), egui::vec2(col_w, card_h));

                let resp = ui.interact(card, ui.id().with(("mobile_intent", i)), egui::Sense::click());
                let p = ui.painter();
                p.rect_filled(card, 12.0, if resp.hovered() { C_CARD_HOT } else { C_CARD });
                p.rect_stroke(card, 12.0, egui::Stroke::new(1.0, C_LINE), egui::StrokeKind::Inside);
                p.text(
                    egui::pos2(card.center().x, card.top() + card.height() * 0.40),
                    egui::Align2::CENTER_CENTER,
                    intent.icon,
                    icons::font(30.0),
                    intent.accent,
                );
                p.text(
                    egui::pos2(card.center().x, card.bottom() - 18.0),
                    egui::Align2::CENTER_CENTER,
                    intent.label,
                    egui::FontId::proportional(15.0),
                    C_BRIGHT,
                );

                if resp.clicked() {
                    app.create_new_project_with_focus(intent.focus);
                    app.mobile_state.window_top = intent.window.0;
                    app.mobile_state.window_count = intent.window.1;
                    app.mobile_state.weights = [1.0, 1.0, 1.0];
                }
            }

            // Recent projects list in the bottom third.
            let recent_top = content_top + grid_h + 14.0;
            ui.painter().text(
                egui::pos2(left, recent_top),
                egui::Align2::LEFT_TOP,
                "Recent",
                egui::FontId::proportional(13.0),
                C_DIM,
            );
            let list_top = recent_top + 22.0;
            let recents = app.config.get_recent_files();
            if recents.is_empty() {
                ui.painter().text(
                    egui::pos2(left, list_top + 8.0),
                    egui::Align2::LEFT_TOP,
                    "No recent projects",
                    egui::FontId::proportional(12.0),
                    C_DIM,
                );
            } else {
                let row_h = 38.0;
                let row_gap = 6.0;
                let avail = rect.bottom() - margin - list_top;
                let max_rows = ((avail + row_gap) / (row_h + row_gap)).floor().max(0.0) as usize;
                let mut chosen: Option<std::path::PathBuf> = None;
                for (j, path) in recents.iter().take(max_rows).enumerate() {
                    let ry = list_top + j as f32 * (row_h + row_gap);
                    let row_rect =
                        egui::Rect::from_min_max(egui::pos2(left, ry), egui::pos2(right, ry + row_h));
                    let resp =
                        ui.interact(row_rect, ui.id().with(("mobile_recent", j)), egui::Sense::click());
                    let p = ui.painter();
                    p.rect_filled(row_rect, 8.0, if resp.hovered() { C_CARD_HOT } else { C_CARD });
                    p.rect_stroke(row_rect, 8.0, egui::Stroke::new(1.0, C_LINE), egui::StrokeKind::Inside);
                    p.text(
                        egui::pos2(row_rect.left() + 12.0, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        icons::FOLDER_OPEN,
                        icons::font(15.0),
                        C_DIM,
                    );
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| "Untitled".to_string());
                    p.text(
                        egui::pos2(row_rect.left() + 36.0, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        egui::FontId::proportional(13.0),
                        C_BRIGHT,
                    );
                    if resp.clicked() {
                        chosen = Some(path.clone());
                    }
                }
                if let Some(path) = chosen {
                    app.load_from_file(path);
                    app.app_mode = crate::AppMode::Editor;
                }
            }
        });
}
