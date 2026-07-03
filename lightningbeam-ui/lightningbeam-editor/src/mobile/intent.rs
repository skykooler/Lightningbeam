//! Mobile new-file intent picker — the phone equivalent of the desktop start screen.
//!
//! Mirrors the desktop "new project" flow: each intent calls
//! [`EditorApp::create_new_project_with_focus`] (which builds the document, picks a default layer,
//! and switches to editor mode) and then sets the initial mobile stack window so the relevant panes
//! are in view.

use eframe::egui;

use super::{icons, Palette};
use crate::EditorApp;

/// One intent card.
struct Intent {
    label: &'static str,
    icon: &'static str,
    accent: egui::Color32,
    /// Argument to `create_new_project_with_focus` (0=Animation, 1=Video, 2=Music, 5=Painting).
    focus: usize,
    /// Initial mobile stack window: (window_top, window_count).
    window: (usize, usize),
    /// Initial pane weights (relative heights) for the window.
    weights: [f32; 3],
}

fn intents(pal: &Palette) -> [Intent; 6] {
    let [coral, cyan, amber, pink, violet] = pal.accents;
    let even = [1.0, 1.0, 1.0];
    // Compose/Record: compressed Timeline ribbon on top; the tall Piano Roll (which now hosts the
    // instrument header + keyboard as one surface) fills the rest.
    let music = [0.4, 1.6, 1.0];
    [
        // Stage indices (see super::STACK): Stage=2, Timeline=3, PianoRoll=4, VirtualPiano=5.
        Intent { label: "Draw", icon: icons::BRUSH, accent: coral, focus: 5, window: (2, 1), weights: even },
        Intent { label: "Animate", icon: icons::FILM, accent: cyan, focus: 0, window: (2, 2), weights: even },
        Intent { label: "Compose", icon: icons::MUSIC, accent: amber, focus: 2, window: (3, 2), weights: music },
        Intent { label: "Record", icon: icons::MIC, accent: pink, focus: 2, window: (3, 2), weights: music },
        Intent { label: "Edit video", icon: icons::CLAPPERBOARD, accent: violet, focus: 1, window: (2, 2), weights: even },
        Intent { label: "Blank", icon: icons::SQUARE_DASHED, accent: pal.text_dim, focus: 0, window: (2, 2), weights: even },
    ]
}

pub fn render(app: &mut EditorApp, ctx: &egui::Context) {
    let pal = Palette::from_theme(&app.theme, ctx);
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(pal.bg))
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
                pal.text,
            );

            let landscape = rect.width() > rect.height();
            let content_top = rect.top() + 62.0;
            let content_bottom = rect.bottom() - margin;
            let gap = 10.0;

            // Portrait: 3 rows × 2 cols of intent cards up top, recent list below.
            // Landscape: 2 rows × 3 cols on the left, recent list to the right.
            let (cols, rows) = if landscape { (3usize, 2usize) } else { (2usize, 3usize) };
            let (grid_rect, recent_rect) = if landscape {
                let split = left + (right - left) * 0.62;
                (
                    egui::Rect::from_min_max(egui::pos2(left, content_top), egui::pos2(split - gap, content_bottom)),
                    egui::Rect::from_min_max(egui::pos2(split + gap, content_top), egui::pos2(right, content_bottom)),
                )
            } else {
                let grid_h = (content_bottom - content_top) * 0.66;
                (
                    egui::Rect::from_min_max(egui::pos2(left, content_top), egui::pos2(right, content_top + grid_h)),
                    egui::Rect::from_min_max(egui::pos2(left, content_top + grid_h + 14.0), egui::pos2(right, content_bottom)),
                )
            };

            // Floor at 0 so a very small window can't produce negative/inverted card rects.
            let col_w = ((grid_rect.width() - (cols as f32 - 1.0) * gap) / cols as f32).max(0.0);
            let card_h = ((grid_rect.height() - (rows as f32 - 1.0) * gap) / rows as f32).max(0.0);
            for (i, intent) in intents(&pal).iter().enumerate() {
                let col = (i % cols) as f32;
                let row = (i / cols) as f32;
                let cx = grid_rect.left() + col * (col_w + gap);
                let cy = grid_rect.top() + row * (card_h + gap);
                let card = egui::Rect::from_min_size(egui::pos2(cx, cy), egui::vec2(col_w, card_h));

                let resp = ui.interact(card, ui.id().with(("mobile_intent", i)), egui::Sense::click());
                let p = ui.painter();
                p.rect_filled(card, 12.0, if resp.hovered() { pal.surface_alt } else { pal.surface });
                p.rect_stroke(card, 12.0, egui::Stroke::new(1.0, pal.line), egui::StrokeKind::Inside);
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
                    pal.text,
                );

                if resp.clicked() {
                    app.create_new_project_with_focus(intent.focus);
                    app.mobile_state.window_top = intent.window.0;
                    app.mobile_state.window_count = intent.window.1;
                    app.mobile_state.weights = intent.weights;
                }
            }

            // Recent projects list (below the grid in portrait, beside it in landscape).
            let rleft = recent_rect.left();
            let rright = recent_rect.right();
            ui.painter().text(
                egui::pos2(rleft, recent_rect.top()),
                egui::Align2::LEFT_TOP,
                "Recent",
                egui::FontId::proportional(13.0),
                pal.text_dim,
            );
            let list_top = recent_rect.top() + 22.0;
            let recents = app.config.get_recent_files();
            if recents.is_empty() {
                ui.painter().text(
                    egui::pos2(rleft, list_top + 8.0),
                    egui::Align2::LEFT_TOP,
                    "No recent projects",
                    egui::FontId::proportional(12.0),
                    pal.text_dim,
                );
            } else {
                let row_h = 38.0;
                let row_gap = 6.0;
                let avail = recent_rect.bottom() - list_top;
                let max_rows = ((avail + row_gap) / (row_h + row_gap)).floor().max(0.0) as usize;
                let mut chosen: Option<std::path::PathBuf> = None;
                for (j, path) in recents.iter().take(max_rows).enumerate() {
                    let ry = list_top + j as f32 * (row_h + row_gap);
                    let row_rect =
                        egui::Rect::from_min_max(egui::pos2(rleft, ry), egui::pos2(rright, ry + row_h));
                    let resp =
                        ui.interact(row_rect, ui.id().with(("mobile_recent", j)), egui::Sense::click());
                    let p = ui.painter();
                    p.rect_filled(row_rect, 8.0, if resp.hovered() { pal.surface_alt } else { pal.surface });
                    p.rect_stroke(row_rect, 8.0, egui::Stroke::new(1.0, pal.line), egui::StrokeKind::Inside);
                    p.text(
                        egui::pos2(row_rect.left() + 12.0, row_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        icons::FOLDER_OPEN,
                        icons::font(15.0),
                        pal.text_dim,
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
                        pal.text,
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
