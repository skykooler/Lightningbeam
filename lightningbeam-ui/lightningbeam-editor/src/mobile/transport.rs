//! The transport "floor": the always-present bottom bar with play/pause, the timecode, and a
//! project scrub. Wired to the audio controller exactly like `TimelinePane`'s header.

use eframe::egui;

use super::{icons, Palette};
use crate::panes::SharedPaneState;

pub fn render(ui: &mut egui::Ui, rect: egui::Rect, shared: &mut SharedPaneState, pal: &Palette) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, pal.surface);
    painter.hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, pal.line));

    let cy = rect.center().y;

    // --- Play / pause button (circle on the left) ---
    let btn_r = 18.0;
    let btn_center = egui::pos2(rect.left() + 20.0 + btn_r, cy);
    let btn_rect = egui::Rect::from_center_size(btn_center, egui::vec2(btn_r * 2.0, btn_r * 2.0));
    let btn_resp = ui.interact(btn_rect, ui.id().with("mobile_transport_play"), egui::Sense::click());
    painter.circle_filled(btn_center, btn_r, pal.accent);
    let glyph = if *shared.is_playing { icons::PAUSE } else { icons::PLAY };
    painter.text(
        btn_center,
        egui::Align2::CENTER_CENTER,
        glyph,
        icons::font(16.0),
        pal.on_accent,
    );
    if btn_resp.clicked() {
        *shared.is_playing = !*shared.is_playing;
        if let Some(controller_arc) = shared.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            if *shared.is_playing {
                controller.seek(*shared.playback_time);
                controller.play();
            } else {
                controller.pause();
            }
        }
    }

    // --- Playhead readout: format follows the project's TimelineMode (set at creation by type). ---
    let t = *shared.playback_time;
    let tc = {
        use lightningbeam_core::document::TimelineMode;
        let doc = shared.action_executor.document();
        match doc.timeline_mode {
            TimelineMode::Measures => {
                let pos = lightningbeam_core::beat_time::time_to_measure(t, doc.tempo_map(), &doc.time_signature);
                format!("{}.{}.{:02}", pos.measure, pos.beat, pos.tick / 10)
            }
            TimelineMode::Frames => format_timecode(t, doc.framerate.max(1.0)),
            TimelineMode::Seconds => {
                let total = t.max(0.0);
                let m = (total / 60.0).floor() as u32;
                format!("{:02}:{:06.3}", m, total % 60.0)
            }
        }
    };
    let tc_left = btn_rect.right() + 12.0;
    painter.text(
        egui::pos2(tc_left, cy),
        egui::Align2::LEFT_CENTER,
        &tc,
        egui::FontId::monospace(13.0),
        pal.text,
    );
    let tc_width = 78.0;

    // --- Project scrub (fills the remaining width) ---
    let duration = shared.action_executor.document().duration.max(1.0);
    let scrub_left = tc_left + tc_width;
    let scrub_rect = egui::Rect::from_min_max(
        egui::pos2(scrub_left, cy - 3.0),
        egui::pos2(rect.right() - 14.0, cy + 3.0),
    );
    let scrub_resp = ui.interact(
        scrub_rect.expand2(egui::vec2(0.0, 12.0)),
        ui.id().with("mobile_transport_scrub"),
        egui::Sense::click_and_drag(),
    );
    painter.rect_filled(scrub_rect, 3.0, pal.line);
    let frac = (*shared.playback_time / duration).clamp(0.0, 1.0) as f32;
    let filled = egui::Rect::from_min_max(
        scrub_rect.min,
        egui::pos2(scrub_rect.left() + scrub_rect.width() * frac, scrub_rect.bottom()),
    );
    painter.rect_filled(filled, 3.0, pal.accent.gamma_multiply(0.5));
    let head_x = scrub_rect.left() + scrub_rect.width() * frac;
    painter.vline(
        head_x,
        (scrub_rect.top() - 4.0)..=(scrub_rect.bottom() + 4.0),
        egui::Stroke::new(2.0, pal.accent),
    );

    if (scrub_resp.dragged() || scrub_resp.clicked()) && scrub_rect.width() > 0.0 {
        if let Some(pos) = scrub_resp.interact_pointer_pos() {
            let f = ((pos.x - scrub_rect.left()) / scrub_rect.width()).clamp(0.0, 1.0) as f64;
            let new_time = f * duration;
            *shared.playback_time = new_time;
            if let Some(controller_arc) = shared.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                controller.seek(new_time);
            }
        }
    }
}

fn format_timecode(seconds: f64, fps: f64) -> String {
    let total = seconds.max(0.0);
    let minutes = (total / 60.0).floor() as u32;
    let secs = (total % 60.0).floor() as u32;
    let frames = ((total - total.floor()) * fps).floor() as u32;
    format!("{:02}:{:02}:{:02}", minutes, secs, frames)
}
