/// Piano Roll pane — MIDI editor and audio spectrogram viewer
///
/// When a MIDI layer is selected, shows a full piano roll editor with note
/// creation, movement, resize, selection, and deletion.
/// When a sampled audio layer is selected, shows a GPU-rendered spectrogram.

use eframe::egui;
use egui::{pos2, vec2, Align2, Color32, FontId, Rect, Stroke, StrokeKind};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use lightningbeam_core::clip::AudioClipType;
use lightningbeam_core::layer::{AnyLayer, AudioLayerType};

use super::{NodePath, PaneRenderer, SharedPaneState};

// ── Constants ────────────────────────────────────────────────────────────────

const KEYBOARD_WIDTH: f32 = 60.0;
const DEFAULT_NOTE_HEIGHT: f32 = 16.0;
const MIN_NOTE: u8 = 21;  // A0
const MAX_NOTE: u8 = 108; // C8
const DEFAULT_PPS: f32 = 100.0; // pixels per second
const NOTE_RESIZE_ZONE: f32 = 8.0; // pixels from right edge to trigger resize
const MIN_NOTE_DURATION: f64 = 0.05; // 50ms minimum note length
const DEFAULT_VELOCITY: u8 = 100;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    MoveNotes { start_time_offset: f64, start_note_offset: i32 },
    ResizeNote { note_index: usize, original_duration: f64 },
    CreateNote,
    SelectRect,
}

#[derive(Debug, Clone)]
struct TempNote {
    note: u8,
    start_time: f64,
    duration: f64,
    velocity: u8,
}

/// A MIDI note resolved from event pairs (note-on + note-off)
#[derive(Debug, Clone)]
struct ResolvedNote {
    note: u8,
    start_time: f64,
    duration: f64,
    velocity: u8,
}

// ── PianoRollPane ────────────────────────────────────────────────────────────

pub struct PianoRollPane {
    // Time axis
    pixels_per_second: f32,
    viewport_start_time: f64,

    // Vertical axis
    note_height: f32,
    scroll_y: f32,
    initial_scroll_set: bool,

    // Interaction
    drag_mode: Option<DragMode>,
    drag_start_screen: Option<egui::Pos2>,
    drag_start_time: f64,
    drag_start_note: u8,
    creating_note: Option<TempNote>,
    selection_rect: Option<(egui::Pos2, egui::Pos2)>,
    selected_note_indices: HashSet<usize>,
    drag_note_offsets: Option<(f64, i32)>, // (time_delta, note_delta) for live preview

    // Clip selection
    selected_clip_id: Option<u32>,

    // Note preview
    preview_note: Option<u8>,       // current preview pitch (stays set after auto-release for re-strike check)
    preview_note_sounding: bool,    // true while MIDI note-on is active (false after auto-release)
    preview_base_note: Option<u8>,  // original pitch before drag offset
    preview_velocity: u8,
    preview_duration: Option<f64>,  // auto-release after this many seconds (None = hold until mouse-up)
    preview_start_time: f64,

    // Auto-scroll
    auto_scroll_enabled: bool,
    user_scrolled_since_play: bool,

    // Resolved note cache — tracks when to invalidate
    cached_clip_id: Option<u32>,

    // Spectrogram cache — keyed by audio pool index
    // Stores pre-computed SpectrogramUpload data ready for GPU
    spectrogram_computed: HashMap<usize, crate::spectrogram_gpu::SpectrogramUpload>,

    // Spectrogram gamma (power curve for colormap)
    spectrogram_gamma: f32,
}

impl PianoRollPane {
    pub fn new() -> Self {
        Self {
            pixels_per_second: DEFAULT_PPS,
            viewport_start_time: 0.0,
            note_height: DEFAULT_NOTE_HEIGHT,
            scroll_y: 0.0,
            initial_scroll_set: false,
            drag_mode: None,
            drag_start_screen: None,
            drag_start_time: 0.0,
            drag_start_note: 60,
            creating_note: None,
            selection_rect: None,
            selected_note_indices: HashSet::new(),
            drag_note_offsets: None,
            selected_clip_id: None,
            preview_note: None,
            preview_note_sounding: false,
            preview_base_note: None,
            preview_velocity: DEFAULT_VELOCITY,
            preview_duration: None,
            preview_start_time: 0.0,
            auto_scroll_enabled: true,
            user_scrolled_since_play: false,
            cached_clip_id: None,
            spectrogram_computed: HashMap::new(),
            spectrogram_gamma: 5.0,
        }
    }

    // ── Coordinate helpers ───────────────────────────────────────────────

    fn time_to_x(&self, time: f64, grid_rect: Rect) -> f32 {
        grid_rect.min.x + ((time - self.viewport_start_time) * self.pixels_per_second as f64) as f32
    }

    fn x_to_time(&self, x: f32, grid_rect: Rect) -> f64 {
        self.viewport_start_time + ((x - grid_rect.min.x) / self.pixels_per_second) as f64
    }

    fn note_to_y(&self, note: u8, rect: Rect) -> f32 {
        let note_index = (MAX_NOTE - note) as f32;
        rect.min.y + note_index * self.note_height - self.scroll_y
    }

    fn y_to_note(&self, y: f32, rect: Rect) -> u8 {
        let note_index = ((y - rect.min.y + self.scroll_y) / self.note_height) as i32;
        (MAX_NOTE as i32 - note_index).clamp(MIN_NOTE as i32, MAX_NOTE as i32) as u8
    }

    fn is_black_key(note: u8) -> bool {
        matches!(note % 12, 1 | 3 | 6 | 8 | 10)
    }

    fn note_name(note: u8) -> String {
        let names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        let octave = (note / 12) as i32 - 1;
        format!("{}{}", names[note as usize % 12], octave)
    }

    // ── Note resolution ──────────────────────────────────────────────────

    fn resolve_notes(events: &[(f64, u8, u8, bool)]) -> Vec<ResolvedNote> {
        let mut active: HashMap<u8, (f64, u8)> = HashMap::new(); // note -> (start_time, velocity)
        let mut notes = Vec::new();

        for &(timestamp, note_number, velocity, is_note_on) in events {
            if is_note_on {
                active.insert(note_number, (timestamp, velocity));
            } else if let Some((start, vel)) = active.remove(&note_number) {
                let duration = (timestamp - start).max(MIN_NOTE_DURATION);
                notes.push(ResolvedNote {
                    note: note_number,
                    start_time: start,
                    duration,
                    velocity: vel,
                });
            }
        }

        // Handle unterminated notes
        for (&note_number, &(start, vel)) in &active {
            notes.push(ResolvedNote {
                note: note_number,
                start_time: start,
                duration: 0.5, // default duration for unterminated
                velocity: vel,
            });
        }

        notes.sort_by(|a, b| a.start_time.partial_cmp(&b.start_time).unwrap());
        notes
    }

    /// Convert resolved notes back to the backend format (start_time, note, velocity, duration)
    fn notes_to_backend_format(notes: &[ResolvedNote]) -> Vec<(f64, u8, u8, f64)> {
        notes.iter().map(|n| (n.start_time, n.note, n.velocity, n.duration)).collect()
    }

    // ── Ruler interval calculation ───────────────────────────────────────

    fn ruler_interval(&self) -> f64 {
        let min_pixel_gap = 80.0;
        let min_seconds = min_pixel_gap / self.pixels_per_second;
        let intervals = [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0];
        for &interval in &intervals {
            if interval >= min_seconds as f64 {
                return interval;
            }
        }
        60.0
    }

    // ── MIDI mode rendering ──────────────────────────────────────────────

    fn render_midi_mode(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        shared: &mut SharedPaneState,
    ) {
        let keyboard_rect = Rect::from_min_size(rect.min, vec2(KEYBOARD_WIDTH, rect.height()));
        let grid_rect = Rect::from_min_max(
            pos2(rect.min.x + KEYBOARD_WIDTH, rect.min.y),
            rect.max,
        );

        // Set initial scroll to center around C4 (MIDI 60)
        if !self.initial_scroll_set {
            let c4_y = (MAX_NOTE - 60) as f32 * self.note_height;
            self.scroll_y = c4_y - rect.height() / 2.0;
            self.initial_scroll_set = true;
        }

        // Get active layer info
        let layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let document = shared.action_executor.document();

        // Collect clip data we need before borrowing shared mutably
        let mut clip_data: Vec<(u32, f64, f64, f64, Uuid)> = Vec::new(); // (midi_clip_id, timeline_start, trim_start, duration, instance_id)
        if let Some(AnyLayer::Audio(audio_layer)) = document.get_layer(&layer_id) {
            for instance in &audio_layer.clip_instances {
                if let Some(clip) = document.audio_clips.get(&instance.clip_id) {
                    if let AudioClipType::Midi { midi_clip_id } = clip.clip_type {
                        let duration = instance.timeline_duration.unwrap_or(clip.duration);
                        clip_data.push((midi_clip_id, instance.timeline_start, instance.trim_start, duration, instance.id));
                    }
                }
            }
        }

        // Auto-select first clip if none selected
        if self.selected_clip_id.is_none() {
            if let Some(&(clip_id, ..)) = clip_data.first() {
                self.selected_clip_id = Some(clip_id);
            }
        }

        // Handle input before rendering
        self.handle_input(ui, grid_rect, keyboard_rect, shared, &clip_data);

        // Auto-scroll during playback
        if *shared.is_playing && self.auto_scroll_enabled && !self.user_scrolled_since_play {
            let playhead_x = self.time_to_x(*shared.playback_time, grid_rect);
            let margin = grid_rect.width() * 0.2;
            if playhead_x > grid_rect.max.x - margin || playhead_x < grid_rect.min.x + margin {
                self.viewport_start_time = *shared.playback_time - (grid_rect.width() * 0.4 / self.pixels_per_second) as f64;
                self.viewport_start_time = self.viewport_start_time.max(0.0);
            }
        }

        // Reset user_scrolled when playback stops
        if !*shared.is_playing {
            self.user_scrolled_since_play = false;
        }

        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(30, 30, 35));

        // Render grid (clipped to grid area)
        let grid_painter = ui.painter_at(grid_rect);
        self.render_grid(&grid_painter, grid_rect);

        // Render clip boundaries and notes
        for &(midi_clip_id, timeline_start, trim_start, duration, _instance_id) in &clip_data {
            let is_selected = self.selected_clip_id == Some(midi_clip_id);
            let opacity = if is_selected { 1.0 } else { 0.3 };

            // Clip boundary
            let clip_x_start = self.time_to_x(timeline_start, grid_rect);
            let clip_x_end = self.time_to_x(timeline_start + duration, grid_rect);

            if clip_x_end >= grid_rect.min.x && clip_x_start <= grid_rect.max.x {
                // Clip background tint
                let clip_bg = Rect::from_min_max(
                    pos2(clip_x_start.max(grid_rect.min.x), grid_rect.min.y),
                    pos2(clip_x_end.min(grid_rect.max.x), grid_rect.max.y),
                );
                grid_painter.rect_filled(clip_bg, 0.0, Color32::from_rgba_unmultiplied(40, 80, 40, (30.0 * opacity) as u8));

                // Clip boundary lines
                let boundary_color = Color32::from_rgba_unmultiplied(100, 200, 100, (150.0 * opacity) as u8);
                if clip_x_start >= grid_rect.min.x {
                    grid_painter.line_segment(
                        [pos2(clip_x_start, grid_rect.min.y), pos2(clip_x_start, grid_rect.max.y)],
                        Stroke::new(1.0, boundary_color),
                    );
                }
                if clip_x_end <= grid_rect.max.x {
                    grid_painter.line_segment(
                        [pos2(clip_x_end, grid_rect.min.y), pos2(clip_x_end, grid_rect.max.y)],
                        Stroke::new(1.0, boundary_color),
                    );
                }
            }

            // Render notes
            if let Some(events) = shared.midi_event_cache.get(&midi_clip_id) {
                let resolved = Self::resolve_notes(events);
                self.render_notes(&grid_painter, grid_rect, &resolved, timeline_start, trim_start, opacity, is_selected);
            }
        }

        // Render temp note being created
        if let Some(ref temp) = self.creating_note {
            if let Some(selected_clip) = clip_data.iter().find(|c| Some(c.0) == self.selected_clip_id) {
                let timeline_start = selected_clip.1;
                let x = self.time_to_x(timeline_start + temp.start_time, grid_rect);
                let y = self.note_to_y(temp.note, grid_rect);
                let w = (temp.duration as f32 * self.pixels_per_second).max(2.0);
                let note_rect = Rect::from_min_size(pos2(x, y), vec2(w, self.note_height - 2.0));

                grid_painter.rect_filled(note_rect, 1.0, Color32::from_rgba_unmultiplied(180, 255, 180, 180));
                grid_painter.rect_stroke(note_rect, 1.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 200)), StrokeKind::Middle);
            }
        }

        // Render selection rectangle
        if let Some((start, end)) = self.selection_rect {
            let sel_rect = Rect::from_two_pos(start, end);
            let clipped = sel_rect.intersect(grid_rect);
            if clipped.is_positive() {
                grid_painter.rect_filled(clipped, 0.0, Color32::from_rgba_unmultiplied(100, 150, 255, 40));
                grid_painter.rect_stroke(clipped, 0.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(100, 150, 255, 150)), StrokeKind::Middle);
            }
        }

        // Render playhead
        self.render_playhead(&grid_painter, grid_rect, *shared.playback_time);

        // Render keyboard on top (so it overlaps grid content at boundary)
        self.render_keyboard(&painter, keyboard_rect);
    }

    fn render_keyboard(&self, painter: &egui::Painter, rect: Rect) {
        // Background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(40, 40, 45));

        for note in MIN_NOTE..=MAX_NOTE {
            let y = self.note_to_y(note, rect);
            let h = self.note_height - 1.0;

            // Skip off-screen
            if y + h < rect.min.y || y > rect.max.y {
                continue;
            }

            let is_black = Self::is_black_key(note);
            let key_width = if is_black {
                KEYBOARD_WIDTH * 0.65
            } else {
                KEYBOARD_WIDTH - 2.0
            };

            let color = if is_black {
                Color32::from_rgb(51, 51, 56)
            } else {
                Color32::from_rgb(220, 220, 225)
            };

            let key_rect = Rect::from_min_size(
                pos2(rect.min.x + 1.0, y),
                vec2(key_width, h),
            );

            // Clip to keyboard area
            let clipped = key_rect.intersect(rect);
            if clipped.is_positive() {
                painter.rect_filled(clipped, 1.0, color);
            }

            // C note labels
            if note % 12 == 0 {
                let octave = (note / 12) as i32 - 1;
                let text_y = (y + self.note_height / 2.0).clamp(rect.min.y, rect.max.y);
                painter.text(
                    pos2(rect.max.x - 4.0, text_y),
                    Align2::RIGHT_CENTER,
                    format!("C{}", octave),
                    FontId::proportional(9.0),
                    Color32::from_gray(100),
                );
            }
        }

        // Right border
        painter.line_segment(
            [pos2(rect.max.x, rect.min.y), pos2(rect.max.x, rect.max.y)],
            Stroke::new(1.0, Color32::from_gray(60)),
        );
    }

    fn render_grid(&self, painter: &egui::Painter, grid_rect: Rect) {
        // Horizontal lines (note separators)
        for note in MIN_NOTE..=MAX_NOTE {
            let y = self.note_to_y(note, grid_rect);
            if y < grid_rect.min.y - 1.0 || y > grid_rect.max.y + 1.0 {
                continue;
            }

            // Black key rows get a slightly different background
            if Self::is_black_key(note) {
                let row_rect = Rect::from_min_size(
                    pos2(grid_rect.min.x, y),
                    vec2(grid_rect.width(), self.note_height),
                ).intersect(grid_rect);
                if row_rect.is_positive() {
                    painter.rect_filled(row_rect, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 15));
                }
            }

            let alpha = if note % 12 == 0 { 60 } else { 20 };
            painter.line_segment(
                [pos2(grid_rect.min.x, y), pos2(grid_rect.max.x, y)],
                Stroke::new(1.0, Color32::from_white_alpha(alpha)),
            );
        }

        // Vertical lines (time grid)
        let interval = self.ruler_interval();
        let start = (self.viewport_start_time / interval).floor() as i64;
        let end_time = self.viewport_start_time + (grid_rect.width() / self.pixels_per_second) as f64;
        let end = (end_time / interval).ceil() as i64;

        for i in start..=end {
            let time = i as f64 * interval;
            let x = self.time_to_x(time, grid_rect);
            if x < grid_rect.min.x || x > grid_rect.max.x {
                continue;
            }

            let is_major = (i % 4 == 0) || interval >= 1.0;
            let alpha = if is_major { 50 } else { 20 };
            painter.line_segment(
                [pos2(x, grid_rect.min.y), pos2(x, grid_rect.max.y)],
                Stroke::new(1.0, Color32::from_white_alpha(alpha)),
            );

            // Time labels at major lines
            if is_major && x > grid_rect.min.x + 20.0 {
                let label = if time >= 60.0 {
                    format!("{}:{:05.2}", (time / 60.0) as u32, time % 60.0)
                } else {
                    format!("{:.2}s", time)
                };
                painter.text(
                    pos2(x + 2.0, grid_rect.min.y + 2.0),
                    Align2::LEFT_TOP,
                    label,
                    FontId::proportional(9.0),
                    Color32::from_white_alpha(80),
                );
            }
        }
    }

    fn render_notes(
        &self,
        painter: &egui::Painter,
        grid_rect: Rect,
        notes: &[ResolvedNote],
        clip_timeline_start: f64,
        _trim_start: f64,
        opacity: f32,
        is_selected_clip: bool,
    ) {
        for (i, note) in notes.iter().enumerate() {
            let global_time = clip_timeline_start + note.start_time;

            // Apply drag offset for selected notes during move
            let (display_time, display_note) = if is_selected_clip
                && self.selected_note_indices.contains(&i)
                && matches!(self.drag_mode, Some(DragMode::MoveNotes { .. }))
            {
                if let Some((dt, dn)) = self.drag_note_offsets {
                    (global_time + dt, (note.note as i32 + dn).clamp(0, 127) as u8)
                } else {
                    (global_time, note.note)
                }
            } else {
                (global_time, note.note)
            };

            // Apply resize for the specific note during resize drag
            let display_duration = if is_selected_clip
                && matches!(self.drag_mode, Some(DragMode::ResizeNote { note_index, .. }) if note_index == i)
            {
                if let Some((dt, _)) = self.drag_note_offsets {
                    (note.duration + dt).max(MIN_NOTE_DURATION)
                } else {
                    note.duration
                }
            } else {
                note.duration
            };

            let x = self.time_to_x(display_time, grid_rect);
            let y = self.note_to_y(display_note, grid_rect);
            let w = (display_duration as f32 * self.pixels_per_second).max(2.0);
            let h = self.note_height - 2.0;

            // Skip off-screen
            if x + w < grid_rect.min.x || x > grid_rect.max.x {
                continue;
            }
            if y + h < grid_rect.min.y || y > grid_rect.max.y {
                continue;
            }

            // Velocity-based brightness
            let brightness = 0.35 + (note.velocity as f32 / 127.0) * 0.65;

            let is_selected = is_selected_clip && self.selected_note_indices.contains(&i);
            let (r, g, b) = if is_selected {
                ((143.0 * brightness) as u8, (252.0 * brightness) as u8, (143.0 * brightness) as u8)
            } else {
                ((111.0 * brightness) as u8, (220.0 * brightness) as u8, (111.0 * brightness) as u8)
            };

            let alpha = (opacity * 255.0) as u8;
            let color = Color32::from_rgba_unmultiplied(r, g, b, alpha);

            let note_rect = Rect::from_min_size(pos2(x, y), vec2(w, h));
            let clipped = note_rect.intersect(grid_rect);
            if clipped.is_positive() {
                painter.rect_filled(clipped, 1.0, color);
                painter.rect_stroke(clipped, 1.0, Stroke::new(1.0, Color32::from_rgba_unmultiplied(0, 0, 0, (76.0 * opacity) as u8)), StrokeKind::Middle);
            }
        }
    }

    fn render_playhead(&self, painter: &egui::Painter, grid_rect: Rect, playback_time: f64) {
        let x = self.time_to_x(playback_time, grid_rect);
        if x < grid_rect.min.x || x > grid_rect.max.x {
            return;
        }
        painter.line_segment(
            [pos2(x, grid_rect.min.y), pos2(x, grid_rect.max.y)],
            Stroke::new(2.0, Color32::from_rgb(255, 100, 100)),
        );
    }

    fn render_dot_grid(&self, painter: &egui::Painter, grid_rect: Rect) {
        // Collect visible time grid positions
        let interval = self.ruler_interval();
        let start = (self.viewport_start_time / interval).floor() as i64;
        let end_time = self.viewport_start_time + (grid_rect.width() / self.pixels_per_second) as f64;
        let end = (end_time / interval).ceil() as i64;

        let time_xs: Vec<f32> = (start..=end)
            .filter_map(|i| {
                let x = self.time_to_x(i as f64 * interval, grid_rect);
                if x >= grid_rect.min.x && x <= grid_rect.max.x {
                    Some(x)
                } else {
                    None
                }
            })
            .collect();

        // Draw dots at grid intersections (note boundary x time line)
        for note in MIN_NOTE..=MAX_NOTE {
            let y = self.note_to_y(note, grid_rect);
            if y < grid_rect.min.y - 1.0 || y > grid_rect.max.y + 1.0 {
                continue;
            }

            let is_c = note % 12 == 0;
            let alpha = if is_c { 50 } else { 20 };
            let radius = if is_c { 1.5 } else { 1.0 };
            let color = Color32::from_white_alpha(alpha);

            for &x in &time_xs {
                painter.circle_filled(pos2(x, y), radius, color);
            }
        }
    }

    // ── Input handling ───────────────────────────────────────────────────

    fn handle_input(
        &mut self,
        ui: &mut egui::Ui,
        grid_rect: Rect,
        keyboard_rect: Rect,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)], // (midi_clip_id, timeline_start, trim_start, duration, instance_id)
    ) {
        let full_rect = Rect::from_min_max(keyboard_rect.min, grid_rect.max);
        let response = ui.allocate_rect(full_rect, egui::Sense::click_and_drag());
        let shift_held = ui.input(|i| i.modifiers.shift);
        let ctrl_held = ui.input(|i| i.modifiers.ctrl);
        let now = ui.input(|i| i.time);

        // Auto-release preview note after its duration expires.
        // Sends note_off but keeps preview_note set so the re-strike check
        // won't re-trigger at the same pitch.
        if let (Some(note), Some(dur)) = (self.preview_note, self.preview_duration) {
            if self.preview_note_sounding && now - self.preview_start_time >= dur {
                if let Some(layer_id) = *shared.active_layer_id {
                    if let Some(&track_id) = shared.layer_to_track_map.get(&layer_id) {
                        if let Some(controller_arc) = shared.audio_controller.as_ref() {
                            let mut controller = controller_arc.lock().unwrap();
                            controller.send_midi_note_off(track_id, note);
                        }
                    }
                }
                self.preview_note_sounding = false;
            }
        }

        // Scroll/zoom handling
        if let Some(hover_pos) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);

            if ctrl_held {
                // Zoom
                if scroll.y != 0.0 {
                    let zoom_factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let time_at_cursor = self.x_to_time(hover_pos.x, grid_rect);
                    self.pixels_per_second = (self.pixels_per_second * zoom_factor as f32).clamp(20.0, 2000.0);
                    // Keep cursor at same time position
                    self.viewport_start_time = time_at_cursor - ((hover_pos.x - grid_rect.min.x) / self.pixels_per_second) as f64;
                    self.user_scrolled_since_play = true;
                }
            } else if shift_held || scroll.x.abs() > 0.0 {
                // Horizontal scroll
                let dx = if scroll.x.abs() > 0.0 { scroll.x } else { scroll.y };
                self.viewport_start_time -= (dx / self.pixels_per_second) as f64;
                self.viewport_start_time = self.viewport_start_time.max(0.0);
                self.user_scrolled_since_play = true;
            } else {
                // Vertical scroll
                self.scroll_y -= scroll.y;
                let max_scroll = (MAX_NOTE - MIN_NOTE + 1) as f32 * self.note_height - grid_rect.height();
                self.scroll_y = self.scroll_y.clamp(0.0, max_scroll.max(0.0));
            }
        }

        // Delete key
        let delete_pressed = ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace));
        if delete_pressed && !self.selected_note_indices.is_empty() {
            if let Some(clip_id) = self.selected_clip_id {
                self.delete_selected_notes(clip_id, shared, clip_data);
            }
        }

        // Immediate press detection (fires on the actual press frame, before egui's drag threshold).
        // This ensures note preview and hit testing use the real press position.
        let pointer_just_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
        if pointer_just_pressed {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if full_rect.contains(pos) {
                    let in_grid = pos.x >= grid_rect.min.x;
                    if in_grid {
                        self.on_grid_press(pos, grid_rect, shift_held, ctrl_held, now, shared, clip_data);
                    } else {
                        // Keyboard click - preview note (hold until mouse-up)
                        let note = self.y_to_note(pos.y, keyboard_rect);
                        self.preview_note_on(note, DEFAULT_VELOCITY, None, now, shared);
                    }
                }
            }
        }

        // Ongoing drag (uses egui's movement threshold)
        if let Some(pos) = response.interact_pointer_pos() {
            if response.dragged() {
                self.on_grid_drag(pos, grid_rect, now, shared, clip_data);
            }
        }

        // Release — either drag ended or click completed (no drag)
        if response.drag_stopped() || response.clicked() {
            self.on_grid_release(grid_rect, shared, clip_data);
        }

        // Update cursor
        if let Some(hover_pos) = response.hover_pos() {
            if hover_pos.x >= grid_rect.min.x {
                if shift_held {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if self.hit_test_note_edge(hover_pos, grid_rect, shared, clip_data).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                } else if self.hit_test_note(hover_pos, grid_rect, shared, clip_data).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            }
        }

        // Request continuous repaint during playback or drag
        if *shared.is_playing || self.drag_mode.is_some() {
            ui.ctx().request_repaint();
        }
    }

    fn on_grid_press(
        &mut self,
        pos: egui::Pos2,
        grid_rect: Rect,
        shift_held: bool,
        ctrl_held: bool,
        now: f64,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let time = self.x_to_time(pos.x, grid_rect);
        let note = self.y_to_note(pos.y, grid_rect);
        self.drag_start_screen = Some(pos);
        self.drag_start_time = time;
        self.drag_start_note = note;

        // Check if clicking on a note edge (resize)
        if let Some(note_idx) = self.hit_test_note_edge(pos, grid_rect, shared, clip_data) {
            if let Some(clip_id) = self.selected_clip_id {
                if let Some(events) = shared.midi_event_cache.get(&clip_id) {
                    let resolved = Self::resolve_notes(events);
                    if note_idx < resolved.len() {
                        self.drag_mode = Some(DragMode::ResizeNote {
                            note_index: note_idx,
                            original_duration: resolved[note_idx].duration,
                        });
                        self.drag_note_offsets = Some((0.0, 0));
                        return;
                    }
                }
            }
        }

        // Check if clicking on a note (select/move)
        if let Some(note_idx) = self.hit_test_note(pos, grid_rect, shared, clip_data) {
            if !ctrl_held && !self.selected_note_indices.contains(&note_idx) {
                // New selection (replace unless Ctrl held)
                self.selected_note_indices.clear();
            }
            self.selected_note_indices.insert(note_idx);

            self.drag_mode = Some(DragMode::MoveNotes {
                start_time_offset: 0.0,
                start_note_offset: 0,
            });
            self.drag_note_offsets = Some((0.0, 0));

            // Preview the note (hold for its duration or until mouse-up)
            if let Some(clip_id) = self.selected_clip_id {
                if let Some(events) = shared.midi_event_cache.get(&clip_id) {
                    let resolved = Self::resolve_notes(events);
                    if note_idx < resolved.len() {
                        let n = &resolved[note_idx];
                        self.preview_base_note = Some(n.note);
                        self.preview_note_on(n.note, n.velocity, Some(n.duration), now, shared);
                    }
                }
            }
            return;
        }

        // Empty space — check which clip we're in
        for &(midi_clip_id, timeline_start, _trim_start, duration, _) in clip_data {
            if time >= timeline_start && time <= timeline_start + duration {
                if self.selected_clip_id != Some(midi_clip_id) {
                    self.selected_clip_id = Some(midi_clip_id);
                    self.selected_note_indices.clear();
                    self.cached_clip_id = None;
                    return;
                }
            }
        }

        if shift_held {
            // Create new note
            if let Some(selected_clip) = clip_data.iter().find(|c| Some(c.0) == self.selected_clip_id) {
                let clip_start = selected_clip.1;
                let clip_local_time = (time - clip_start).max(0.0);
                self.creating_note = Some(TempNote {
                    note,
                    start_time: clip_local_time,
                    duration: MIN_NOTE_DURATION,
                    velocity: DEFAULT_VELOCITY,
                });
                self.drag_mode = Some(DragMode::CreateNote);
                self.preview_note_on(note, DEFAULT_VELOCITY, None, now, shared);
            }
        } else {
            // Start selection rectangle
            self.selected_note_indices.clear();
            self.selection_rect = Some((pos, pos));
            self.drag_mode = Some(DragMode::SelectRect);
        }
    }

    fn on_grid_drag(
        &mut self,
        pos: egui::Pos2,
        grid_rect: Rect,
        now: f64,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let time = self.x_to_time(pos.x, grid_rect);
        let note = self.y_to_note(pos.y, grid_rect);

        match self.drag_mode {
            Some(DragMode::CreateNote) => {
                if let Some(ref mut temp) = self.creating_note {
                    if let Some(selected_clip) = clip_data.iter().find(|c| Some(c.0) == self.selected_clip_id) {
                        let clip_start = selected_clip.1;
                        let clip_local_time = (time - clip_start).max(0.0);
                        temp.duration = (clip_local_time - temp.start_time).max(MIN_NOTE_DURATION);
                    }
                }
            }
            Some(DragMode::MoveNotes { .. }) => {
                let dt = time - self.drag_start_time;
                let dn = note as i32 - self.drag_start_note as i32;
                self.drag_note_offsets = Some((dt, dn));

                // Re-strike preview when pitch changes during drag
                if let Some(base_note) = self.preview_base_note {
                    let effective_pitch = (base_note as i32 + dn).clamp(0, 127) as u8;
                    if self.preview_note != Some(effective_pitch) {
                        let vel = self.preview_velocity;
                        let dur = self.preview_duration;
                        self.preview_note_on(effective_pitch, vel, dur, now, shared);
                    }
                }
            }
            Some(DragMode::ResizeNote { .. }) => {
                let dt = time - self.drag_start_time;
                self.drag_note_offsets = Some((dt, 0));
            }
            Some(DragMode::SelectRect) => {
                if let Some((start, _)) = self.selection_rect {
                    self.selection_rect = Some((start, pos));
                    // Update selected notes based on rectangle
                    self.update_selection_from_rect(grid_rect, shared, clip_data);
                }
            }
            None => {}
        }
    }

    fn on_grid_release(
        &mut self,
        grid_rect: Rect,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let _ = grid_rect; // used for future snapping
        match self.drag_mode.take() {
            Some(DragMode::CreateNote) => {
                if let Some(temp) = self.creating_note.take() {
                    if let Some(clip_id) = self.selected_clip_id {
                        self.commit_create_note(clip_id, temp, shared, clip_data);
                    }
                }
            }
            Some(DragMode::MoveNotes { .. }) => {
                if let Some((dt, dn)) = self.drag_note_offsets.take() {
                    if dt.abs() > 0.001 || dn != 0 {
                        if let Some(clip_id) = self.selected_clip_id {
                            self.commit_move_notes(clip_id, dt, dn, shared, clip_data);
                        }
                    }
                }
            }
            Some(DragMode::ResizeNote { note_index, .. }) => {
                if let Some((dt, _)) = self.drag_note_offsets.take() {
                    if dt.abs() > 0.001 {
                        if let Some(clip_id) = self.selected_clip_id {
                            self.commit_resize_note(clip_id, note_index, dt, shared, clip_data);
                        }
                    }
                }
            }
            Some(DragMode::SelectRect) => {
                self.selection_rect = None;
            }
            None => {}
        }

        self.drag_note_offsets = None;
        self.preview_note_off(shared);
        self.preview_base_note = None;
        self.preview_duration = None;
    }

    // ── Hit testing ──────────────────────────────────────────────────────

    fn hit_test_note(
        &self,
        pos: egui::Pos2,
        grid_rect: Rect,
        shared: &SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) -> Option<usize> {
        let clip_id = self.selected_clip_id?;
        let events = shared.midi_event_cache.get(&clip_id)?;
        let resolved = Self::resolve_notes(events);
        let clip_info = clip_data.iter().find(|c| c.0 == clip_id)?;
        let timeline_start = clip_info.1;

        for (i, note) in resolved.iter().enumerate().rev() {
            let x = self.time_to_x(timeline_start + note.start_time, grid_rect);
            let y = self.note_to_y(note.note, grid_rect);
            let w = (note.duration as f32 * self.pixels_per_second).max(2.0);
            let note_rect = Rect::from_min_size(pos2(x, y), vec2(w, self.note_height - 2.0));

            if note_rect.contains(pos) {
                return Some(i);
            }
        }
        None
    }

    fn hit_test_note_edge(
        &self,
        pos: egui::Pos2,
        grid_rect: Rect,
        shared: &SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) -> Option<usize> {
        let clip_id = self.selected_clip_id?;
        let events = shared.midi_event_cache.get(&clip_id)?;
        let resolved = Self::resolve_notes(events);
        let clip_info = clip_data.iter().find(|c| c.0 == clip_id)?;
        let timeline_start = clip_info.1;

        for (i, note) in resolved.iter().enumerate().rev() {
            let x = self.time_to_x(timeline_start + note.start_time, grid_rect);
            let y = self.note_to_y(note.note, grid_rect);
            let w = (note.duration as f32 * self.pixels_per_second).max(2.0);
            let note_rect = Rect::from_min_size(pos2(x, y), vec2(w, self.note_height - 2.0));

            if note_rect.contains(pos) {
                let edge_x = note_rect.max.x;
                if (pos.x - edge_x).abs() < NOTE_RESIZE_ZONE {
                    return Some(i);
                }
            }
        }
        None
    }

    fn update_selection_from_rect(
        &mut self,
        grid_rect: Rect,
        shared: &SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let (start, end) = match self.selection_rect {
            Some(se) => se,
            None => return,
        };
        let sel_rect = Rect::from_two_pos(start, end);

        self.selected_note_indices.clear();

        let clip_id = match self.selected_clip_id {
            Some(id) => id,
            None => return,
        };
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let resolved = Self::resolve_notes(events);
        let clip_info = match clip_data.iter().find(|c| c.0 == clip_id) {
            Some(c) => c,
            None => return,
        };
        let timeline_start = clip_info.1;

        for (i, note) in resolved.iter().enumerate() {
            let x = self.time_to_x(timeline_start + note.start_time, grid_rect);
            let y = self.note_to_y(note.note, grid_rect);
            let w = (note.duration as f32 * self.pixels_per_second).max(2.0);
            let note_rect = Rect::from_min_size(pos2(x, y), vec2(w, self.note_height - 2.0));

            if sel_rect.intersects(note_rect) {
                self.selected_note_indices.insert(i);
            }
        }
    }

    // ── Note operations (commit to action system) ────────────────────────

    /// Update midi_event_cache immediately so notes render at their new positions
    /// without waiting for the backend round-trip.
    ///
    /// DESYNC RISK: This updates the cache before the action executes on the backend.
    /// If the action later fails during execute_with_backend(), the cache will be out
    /// of sync with the backend state. This is acceptable because MIDI note edits are
    /// simple operations unlikely to fail, and undo/redo rebuilds cache from the action's
    /// stored note data to restore consistency.
    fn update_cache_from_resolved(clip_id: u32, resolved: &[ResolvedNote], shared: &mut SharedPaneState) {
        let mut events: Vec<(f64, u8, u8, bool)> = Vec::with_capacity(resolved.len() * 2);
        for n in resolved {
            events.push((n.start_time, n.note, n.velocity, true));
            events.push((n.start_time + n.duration, n.note, n.velocity, false));
        }
        events.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        shared.midi_event_cache.insert(clip_id, events);
    }

    fn commit_create_note(
        &mut self,
        clip_id: u32,
        temp: TempNote,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let mut resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);

        resolved.push(ResolvedNote {
            note: temp.note,
            start_time: temp.start_time,
            duration: temp.duration,
            velocity: temp.velocity,
        });
        let new_notes = Self::notes_to_backend_format(&resolved);

        Self::update_cache_from_resolved(clip_id, &resolved, shared);
        self.push_update_action("Add note", clip_id, old_notes, new_notes, shared, clip_data);
        self.cached_clip_id = None;
    }

    fn commit_move_notes(
        &mut self,
        clip_id: u32,
        dt: f64,
        dn: i32,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);

        let mut new_resolved = resolved.clone();
        for &idx in &self.selected_note_indices {
            if idx < new_resolved.len() {
                new_resolved[idx].start_time = (new_resolved[idx].start_time + dt).max(0.0);
                new_resolved[idx].note = (new_resolved[idx].note as i32 + dn).clamp(0, 127) as u8;
            }
        }
        let new_notes = Self::notes_to_backend_format(&new_resolved);

        Self::update_cache_from_resolved(clip_id, &new_resolved, shared);
        self.push_update_action("Move notes", clip_id, old_notes, new_notes, shared, clip_data);
        self.cached_clip_id = None;
    }

    fn commit_resize_note(
        &mut self,
        clip_id: u32,
        note_index: usize,
        dt: f64,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);

        let mut new_resolved = resolved.clone();
        if note_index < new_resolved.len() {
            new_resolved[note_index].duration = (new_resolved[note_index].duration + dt).max(MIN_NOTE_DURATION);
        }
        let new_notes = Self::notes_to_backend_format(&new_resolved);

        Self::update_cache_from_resolved(clip_id, &new_resolved, shared);
        self.push_update_action("Resize note", clip_id, old_notes, new_notes, shared, clip_data);
        self.cached_clip_id = None;
    }

    fn delete_selected_notes(
        &mut self,
        clip_id: u32,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);

        let new_resolved: Vec<ResolvedNote> = resolved
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.selected_note_indices.contains(i))
            .map(|(_, n)| n.clone())
            .collect();
        let new_notes = Self::notes_to_backend_format(&new_resolved);

        Self::update_cache_from_resolved(clip_id, &new_resolved, shared);
        self.push_update_action("Delete notes", clip_id, old_notes, new_notes, shared, clip_data);
        self.selected_note_indices.clear();
        self.cached_clip_id = None;
    }

    fn push_update_action(
        &self,
        description: &str,
        clip_id: u32,
        old_notes: Vec<(f64, u8, u8, f64)>,
        new_notes: Vec<(f64, u8, u8, f64)>,
        shared: &mut SharedPaneState,
        _clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        // Find the layer_id for this clip
        let layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let action = lightningbeam_core::actions::UpdateMidiNotesAction {
            layer_id,
            midi_clip_id: clip_id,
            old_notes,
            new_notes,
            description_text: description.to_string(),
        };
        shared.pending_actions.push(Box::new(action));
    }

    // ── Note preview ─────────────────────────────────────────────────────

    fn preview_note_on(&mut self, note: u8, velocity: u8, duration: Option<f64>, time: f64, shared: &mut SharedPaneState) {
        self.preview_note_off(shared);

        if let Some(layer_id) = *shared.active_layer_id {
            if let Some(&track_id) = shared.layer_to_track_map.get(&layer_id) {
                if let Some(controller_arc) = shared.audio_controller.as_ref() {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.send_midi_note_on(track_id, note, velocity);
                    self.preview_note = Some(note);
                    self.preview_note_sounding = true;
                    self.preview_velocity = velocity;
                    self.preview_duration = duration;
                    self.preview_start_time = time;
                }
            }
        }
    }

    fn preview_note_off(&mut self, shared: &mut SharedPaneState) {
        if let Some(note) = self.preview_note.take() {
            if self.preview_note_sounding {
                if let Some(layer_id) = *shared.active_layer_id {
                    if let Some(&track_id) = shared.layer_to_track_map.get(&layer_id) {
                        if let Some(controller_arc) = shared.audio_controller.as_ref() {
                            let mut controller = controller_arc.lock().unwrap();
                            controller.send_midi_note_off(track_id, note);
                        }
                    }
                }
                self.preview_note_sounding = false;
            }
        }
        // Don't clear preview_base_note or preview_duration here —
        // they're needed for re-striking during drag. Cleared in on_grid_release.
    }

    // ── Spectrogram mode ─────────────────────────────────────────────────

    fn render_spectrogram_mode(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        shared: &mut SharedPaneState,
    ) {
        let keyboard_rect = Rect::from_min_size(rect.min, vec2(KEYBOARD_WIDTH, rect.height()));
        let view_rect = Rect::from_min_max(
            pos2(rect.min.x + KEYBOARD_WIDTH, rect.min.y),
            rect.max,
        );

        // Set initial scroll to center around C4 (MIDI 60) — same as MIDI mode
        if !self.initial_scroll_set {
            let c4_y = (MAX_NOTE - 60) as f32 * self.note_height;
            self.scroll_y = c4_y - rect.height() / 2.0;
            self.initial_scroll_set = true;
        }

        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(20, 20, 25));

        // Dot grid background (visible where the spectrogram doesn't draw)
        let grid_painter = ui.painter_at(view_rect);
        self.render_dot_grid(&grid_painter, view_rect);

        // Find audio pool index for the active layer's clips
        let layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };

        let document = shared.action_executor.document();
        let mut clip_infos: Vec<(usize, f64, f64, f64, u32)> = Vec::new(); // (pool_index, timeline_start, trim_start, duration, sample_rate)
        if let Some(AnyLayer::Audio(audio_layer)) = document.get_layer(&layer_id) {
            for instance in &audio_layer.clip_instances {
                if let Some(clip) = document.audio_clips.get(&instance.clip_id) {
                    if let AudioClipType::Sampled { audio_pool_index } = clip.clip_type {
                        let duration = instance.timeline_duration.unwrap_or(clip.duration);
                        // Get sample rate from raw_audio_cache
                        if let Some((_samples, sr, _ch)) = shared.raw_audio_cache.get(&audio_pool_index) {
                            clip_infos.push((audio_pool_index, instance.timeline_start, instance.trim_start, duration, *sr));
                        }
                    }
                }
            }
        }

        let screen_size = ui.ctx().input(|i| i.content_rect().size());

        // Render spectrogram for each sampled clip on this layer
        for &(pool_index, timeline_start, trim_start, _duration, sample_rate) in &clip_infos {
            // Compute spectrogram if not cached
            let needs_compute = !self.spectrogram_computed.contains_key(&pool_index);
            let pending_upload = if needs_compute {
                if let Some((samples, sr, ch)) = shared.raw_audio_cache.get(&pool_index) {
                    let spec_data = crate::spectrogram_compute::compute_spectrogram(
                        samples, *sr, *ch, 2048, 512,
                    );
                    if spec_data.time_bins > 0 {
                        let upload = crate::spectrogram_gpu::SpectrogramUpload {
                            magnitudes: spec_data.magnitudes,
                            time_bins: spec_data.time_bins as u32,
                            freq_bins: spec_data.freq_bins as u32,
                            sample_rate: spec_data.sample_rate,
                            hop_size: spec_data.hop_size as u32,
                            fft_size: spec_data.fft_size as u32,
                            duration: spec_data.duration as f32,
                        };
                        // Store a marker so we don't recompute
                        self.spectrogram_computed.insert(pool_index, crate::spectrogram_gpu::SpectrogramUpload {
                            magnitudes: Vec::new(), // We don't need to keep the data around
                            time_bins: upload.time_bins,
                            freq_bins: upload.freq_bins,
                            sample_rate: upload.sample_rate,
                            hop_size: upload.hop_size,
                            fft_size: upload.fft_size,
                            duration: upload.duration,
                        });
                        Some(upload)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Get cached spectrogram metadata for params
            let spec_meta = self.spectrogram_computed.get(&pool_index);
            let (time_bins, freq_bins, hop_size, fft_size, audio_duration) = match spec_meta {
                Some(m) => (m.time_bins as f32, m.freq_bins as f32, m.hop_size as f32, m.fft_size as f32, m.duration),
                None => continue,
            };

            if view_rect.width() > 0.0 && view_rect.height() > 0.0 {
                let callback = crate::spectrogram_gpu::SpectrogramCallback {
                    pool_index,
                    params: crate::spectrogram_gpu::SpectrogramParams {
                        clip_rect: [view_rect.min.x, view_rect.min.y, view_rect.max.x, view_rect.max.y],
                        viewport_start_time: self.viewport_start_time as f32,
                        pixels_per_second: self.pixels_per_second,
                        audio_duration,
                        sample_rate: sample_rate as f32,
                        clip_start_time: timeline_start as f32,
                        trim_start: trim_start as f32,
                        time_bins,
                        freq_bins,
                        hop_size,
                        fft_size,
                        scroll_y: self.scroll_y,
                        note_height: self.note_height,
                        screen_size: [screen_size.x, screen_size.y],
                        min_note: MIN_NOTE as f32,
                        max_note: MAX_NOTE as f32,
                        gamma: self.spectrogram_gamma,
                        _pad: [0.0; 3],
                    },
                    target_format: shared.target_format,
                    pending_upload,
                };

                ui.painter().add(egui_wgpu::Callback::new_paint_callback(
                    view_rect,
                    callback,
                ));
            }
        }

        // Handle scroll/zoom
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        if let Some(hover_pos) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta);
            let ctrl_held = ui.input(|i| i.modifiers.ctrl);
            let shift_held = ui.input(|i| i.modifiers.shift);

            if ctrl_held && scroll.y != 0.0 {
                // Zoom
                let zoom_factor = if scroll.y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                let time_at_cursor = self.x_to_time(hover_pos.x, view_rect);
                self.pixels_per_second = (self.pixels_per_second * zoom_factor as f32).clamp(20.0, 2000.0);
                self.viewport_start_time = time_at_cursor - ((hover_pos.x - view_rect.min.x) / self.pixels_per_second) as f64;
                self.user_scrolled_since_play = true;
            } else if shift_held || scroll.x.abs() > 0.0 {
                // Horizontal scroll
                let dx = if scroll.x.abs() > 0.0 { scroll.x } else { scroll.y };
                self.viewport_start_time -= (dx / self.pixels_per_second) as f64;
                self.viewport_start_time = self.viewport_start_time.max(0.0);
                self.user_scrolled_since_play = true;
            } else {
                // Vertical scroll (same as MIDI mode)
                self.scroll_y -= scroll.y;
                let max_scroll = (MAX_NOTE - MIN_NOTE + 1) as f32 * self.note_height - view_rect.height();
                self.scroll_y = self.scroll_y.clamp(0.0, max_scroll.max(0.0));
            }
        }

        // Playhead
        let playhead_painter = ui.painter_at(view_rect);
        self.render_playhead(&playhead_painter, view_rect, *shared.playback_time);

        // Keyboard on top (same as MIDI mode)
        self.render_keyboard(&painter, keyboard_rect);

        // Auto-scroll during playback
        if *shared.is_playing && self.auto_scroll_enabled && !self.user_scrolled_since_play {
            let playhead_x = self.time_to_x(*shared.playback_time, view_rect);
            let margin = view_rect.width() * 0.2;
            if playhead_x > view_rect.max.x - margin || playhead_x < view_rect.min.x + margin {
                self.viewport_start_time = *shared.playback_time - (view_rect.width() * 0.4 / self.pixels_per_second) as f64;
                self.viewport_start_time = self.viewport_start_time.max(0.0);
            }
        }

        if !*shared.is_playing {
            self.user_scrolled_since_play = false;
        }

        if *shared.is_playing {
            ui.ctx().request_repaint();
        }
    }

    fn render_empty_state(&self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_rgb(30, 30, 35));
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "Select a MIDI or audio layer to view",
            FontId::proportional(14.0),
            Color32::from_gray(100),
        );
    }
}

impl PaneRenderer for PianoRollPane {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        ui.horizontal(|ui| {
            // Pane title
            ui.label(
                egui::RichText::new("Piano Roll")
                    .color(Color32::from_gray(180))
                    .size(11.0),
            );
            ui.separator();

            // Zoom
            ui.label(
                egui::RichText::new(format!("{:.0}px/s", self.pixels_per_second))
                    .color(Color32::from_gray(140))
                    .size(10.0),
            );

            // Selected notes count
            if !self.selected_note_indices.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("{} selected", self.selected_note_indices.len()))
                        .color(Color32::from_rgb(143, 252, 143))
                        .size(10.0),
                );
            }

            // Velocity display for selected notes
            if self.selected_note_indices.len() == 1 {
                if let Some(clip_id) = self.selected_clip_id {
                    if let Some(events) = shared.midi_event_cache.get(&clip_id) {
                        let resolved = Self::resolve_notes(events);
                        if let Some(&idx) = self.selected_note_indices.iter().next() {
                            if idx < resolved.len() {
                                ui.separator();
                                let n = &resolved[idx];
                                ui.label(
                                    egui::RichText::new(format!("{} vel:{}", Self::note_name(n.note), n.velocity))
                                        .color(Color32::from_gray(140))
                                        .size(10.0),
                                );
                            }
                        }
                    }
                }
            }

            // Spectrogram gamma slider (only in spectrogram mode)
            let is_spectrogram = shared.active_layer_id.and_then(|id| {
                let document = shared.action_executor.document();
                match document.get_layer(&id)? {
                    AnyLayer::Audio(audio) => Some(matches!(audio.audio_layer_type, AudioLayerType::Sampled)),
                    _ => None,
                }
            }).unwrap_or(false);

            if is_spectrogram {
                ui.separator();
                ui.label(
                    egui::RichText::new("Gamma")
                        .color(Color32::from_gray(140))
                        .size(10.0),
                );
                ui.add(
                    egui::DragValue::new(&mut self.spectrogram_gamma)
                        .speed(0.05)
                        .range(0.5..=10.0)
                        .max_decimals(1),
                );
            }
        });
        true
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Determine mode based on active layer type
        let layer_id = *shared.active_layer_id;

        let mode = layer_id.and_then(|id| {
            let document = shared.action_executor.document();
            match document.get_layer(&id)? {
                AnyLayer::Audio(audio) => Some(audio.audio_layer_type.clone()),
                _ => None,
            }
        });

        match mode {
            Some(AudioLayerType::Midi) => self.render_midi_mode(ui, rect, shared),
            Some(AudioLayerType::Sampled) => self.render_spectrogram_mode(ui, rect, shared),
            None => self.render_empty_state(ui, rect),
        }
    }

    fn name(&self) -> &str {
        "Piano Roll"
    }
}
