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
enum PitchBendZone {
    Start,   // First 30% of note: ramp from bend → 0
    Middle,  // Middle 40%: bell curve 0 → bend → 0
    End,     // Last 30%: ramp from 0 → bend
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
enum SnapValue {
    #[default] None,
    Whole, Half, Quarter, Eighth, Sixteenth, ThirtySecond,
    QuarterTriplet, EighthTriplet, SixteenthTriplet, ThirtySecondTriplet,
    EighthSwingLight, SixteenthSwingLight,
    EighthSwingHeavy, SixteenthSwingHeavy,
}

impl SnapValue {
    fn label(self) -> &'static str {
        match self {
            Self::None              => "None",
            Self::Whole             => "1/1",
            Self::Half              => "1/2",
            Self::Quarter           => "1/4",
            Self::Eighth            => "1/8",
            Self::Sixteenth         => "1/16",
            Self::ThirtySecond      => "1/32",
            Self::QuarterTriplet    => "1/4T",
            Self::EighthTriplet     => "1/8T",
            Self::SixteenthTriplet  => "1/16T",
            Self::ThirtySecondTriplet => "1/32T",
            Self::EighthSwingLight    => "1/8 swing light",
            Self::SixteenthSwingLight => "1/16 swing light",
            Self::EighthSwingHeavy    => "1/8 swing heavy",
            Self::SixteenthSwingHeavy => "1/16 swing heavy",
        }
    }

    fn all() -> &'static [SnapValue] {
        &[
            Self::None, Self::Whole, Self::Half, Self::Quarter,
            Self::Eighth, Self::Sixteenth, Self::ThirtySecond,
            Self::QuarterTriplet, Self::EighthTriplet,
            Self::SixteenthTriplet, Self::ThirtySecondTriplet,
            Self::EighthSwingLight, Self::SixteenthSwingLight,
            Self::EighthSwingHeavy, Self::SixteenthSwingHeavy,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum DragMode {
    MoveNotes { start_time_offset: f64, start_note_offset: i32 },
    ResizeNote { note_index: usize, original_duration: f64 },
    CreateNote,
    SelectRect,
    /// Alt-drag pitch bend editing on a note
    PitchBend {
        note_index: usize,
        zone: PitchBendZone,
        note_pitch: u8,
        note_channel: u8,
        note_start: f64,
        note_duration: f64,
        origin_y: f32,
        current_semitones: f32,
    },
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
    channel: u8,
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

    // Spectrogram gamma (power curve for colormap)
    spectrogram_gamma: f32,

    // Header slider values — persist across frames during drag
    header_vel: f32,
    header_mod: f32,

    // Instrument pitch bend range in semitones (queried from backend when layer changes)
    pitch_bend_range: f32,
    // Layer ID for which pitch_bend_range was last queried
    pitch_bend_range_layer: Option<uuid::Uuid>,

    // Snap / quantize
    snap_value: SnapValue,
    last_snap_selection: HashSet<usize>,
    snap_user_changed: bool, // set in render_header, consumed before handle_input
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
            spectrogram_gamma: 0.8,
            header_vel: 100.0,
            header_mod: 0.0,
            pitch_bend_range: 2.0,
            pitch_bend_range_layer: None,
            snap_value: SnapValue::None,
            last_snap_selection: HashSet::new(),
            snap_user_changed: false,
        }
    }

    // ── Coordinate helpers ───────────────────────────────────────────────

    fn time_to_x(&self, time: f64, grid_rect: Rect) -> f32 {
        grid_rect.min.x + ((time - self.viewport_start_time) * self.pixels_per_second as f64) as f32
    }

    fn x_to_time(&self, x: f32, grid_rect: Rect) -> f64 {
        self.viewport_start_time + ((x - grid_rect.min.x) / self.pixels_per_second) as f64
    }

    fn apply_zoom_at_point(&mut self, zoom_delta: f32, mouse_x: f32, grid_rect: Rect) {
        let time_at_mouse = self.x_to_time(mouse_x, grid_rect);
        self.pixels_per_second = (self.pixels_per_second * (1.0 + zoom_delta)).clamp(20.0, 2000.0);
        let new_mouse_x = self.time_to_x(time_at_mouse, grid_rect);
        let time_delta = (new_mouse_x - mouse_x) / self.pixels_per_second;
        self.viewport_start_time = (self.viewport_start_time + time_delta as f64).max(0.0);
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

    // ── Note resolution ──────────────────────────────────────────────────

    fn resolve_notes(events: &[daw_backend::audio::midi::MidiEvent]) -> Vec<ResolvedNote> {
        let mut active: HashMap<u8, (f64, u8, u8)> = HashMap::new(); // note -> (start_time, velocity, channel)
        let mut notes = Vec::new();

        for event in events {
            let channel = event.status & 0x0F;
            if event.is_note_on() {
                active.insert(event.data1, (event.timestamp.beats_to_f64(), event.data2, channel));
            } else if event.is_note_off() {
                if let Some((start, vel, ch)) = active.remove(&event.data1) {
                    let duration = (event.timestamp.beats_to_f64() - start).max(MIN_NOTE_DURATION);
                    notes.push(ResolvedNote {
                        note: event.data1,
                        channel: ch,
                        start_time: start,
                        duration,
                        velocity: vel,
                    });
                }
            }
        }

        // Handle unterminated notes
        for (&note_number, &(start, vel, ch)) in &active {
            notes.push(ResolvedNote {
                note: note_number,
                channel: ch,
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

    fn ruler_interval(&self, tempo_map: &daw_backend::TempoMap, time_sig: &lightningbeam_core::document::TimeSignature) -> f64 {
        let min_pixel_gap = 80.0;
        let min_seconds = (min_pixel_gap / self.pixels_per_second) as f64;

        // Use beat-aligned intervals
        let beat_dur = lightningbeam_core::beat_time::beat_duration(0.0, tempo_map);
        let measure_dur = lightningbeam_core::beat_time::measure_duration(0.0, tempo_map, time_sig);
        let beat_intervals = [
            beat_dur / 4.0, beat_dur / 2.0, beat_dur, beat_dur * 2.0,
            measure_dur, measure_dur * 2.0, measure_dur * 4.0,
        ];
        for &interval in &beat_intervals {
            if interval >= min_seconds {
                return interval;
            }
        }
        measure_dur * 4.0
    }

    // ── MIDI mode rendering ──────────────────────────────────────────────

} // end impl PianoRollPane (snap helpers follow as free functions)

fn snap_to_value(t: f64, snap: SnapValue, tempo_map: &daw_backend::TempoMap) -> f64 {
    let beat = lightningbeam_core::beat_time::beat_duration(0.0, tempo_map);
    match snap {
        SnapValue::None                => t,
        SnapValue::Whole               => round_to_grid(t, beat * 4.0),
        SnapValue::Half                => round_to_grid(t, beat * 2.0),
        SnapValue::Quarter             => round_to_grid(t, beat),
        SnapValue::Eighth              => round_to_grid(t, beat * 0.5),
        SnapValue::Sixteenth           => round_to_grid(t, beat * 0.25),
        SnapValue::ThirtySecond        => round_to_grid(t, beat * 0.125),
        SnapValue::QuarterTriplet      => round_to_grid(t, beat * 2.0 / 3.0),
        SnapValue::EighthTriplet       => round_to_grid(t, beat / 3.0),
        SnapValue::SixteenthTriplet    => round_to_grid(t, beat / 6.0),
        SnapValue::ThirtySecondTriplet => round_to_grid(t, beat / 12.0),
        SnapValue::EighthSwingLight    => snap_swing(t, beat,       2.0 / 3.0),
        SnapValue::SixteenthSwingLight => snap_swing(t, beat * 0.5, 2.0 / 3.0),
        SnapValue::EighthSwingHeavy    => snap_swing(t, beat,       3.0 / 4.0),
        SnapValue::SixteenthSwingHeavy => snap_swing(t, beat * 0.5, 3.0 / 4.0),
    }
}

fn round_to_grid(t: f64, interval: f64) -> f64 {
    (t / interval).round() * interval
}

fn snap_swing(t: f64, cell: f64, ratio: f64) -> f64 {
    let cell_n = (t / cell).floor() as i64;
    let cell_start = cell_n as f64 * cell;
    let cands = [cell_start, cell_start + ratio * cell, cell_start + cell];
    *cands.iter().min_by(|&&a, &&b| (a - t).abs().partial_cmp(&(b - t).abs()).unwrap()).unwrap()
}

fn detect_snap(notes: &[&ResolvedNote], tempo_map: &daw_backend::TempoMap) -> SnapValue {
    const EPS: f64 = 0.002;
    if notes.is_empty() { return SnapValue::None; }
    let order = [
        SnapValue::Whole, SnapValue::Half, SnapValue::Quarter,
        SnapValue::EighthSwingHeavy, SnapValue::EighthSwingLight, SnapValue::Eighth,
        SnapValue::SixteenthSwingHeavy, SnapValue::SixteenthSwingLight,
        SnapValue::QuarterTriplet, SnapValue::Sixteenth,
        SnapValue::EighthTriplet, SnapValue::ThirtySecond,
        SnapValue::SixteenthTriplet, SnapValue::ThirtySecondTriplet,
    ];
    for &sv in &order {
        if notes.iter().all(|n| (snap_to_value(n.start_time, sv, tempo_map) - n.start_time).abs() < EPS) {
            return sv;
        }
    }
    SnapValue::None
}

impl PianoRollPane {

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

        // Query pitch bend range from backend when the layer changes
        if self.pitch_bend_range_layer != Some(layer_id) {
            if let Some(track_id) = shared.layer_to_track_map.get(&layer_id) {
                if let Some(ctrl) = shared.audio_controller.as_ref() {
                    if let Ok(mut c) = ctrl.lock() {
                        self.pitch_bend_range = c.query_pitch_bend_range(*track_id);
                    }
                }
            }
            self.pitch_bend_range_layer = Some(layer_id);
        }

        let document = shared.action_executor.document();

        // Collect clip data using the engine snapshot (source of truth), which reflects
        // recorded clips immediately. Falls back to document if snapshot is empty/absent.
        let mut clip_data: Vec<(u32, f64, f64, f64, Uuid)> = Vec::new(); // (midi_clip_id, timeline_start, trim_start, duration, instance_id)

        let snapshot_clips: Option<Vec<daw_backend::audio::midi::MidiClipInstance>> =
            shared.clip_snapshot.as_ref().and_then(|arc| {
                let snap = arc.read().ok()?;
                let track_id = shared.layer_to_track_map.get(&layer_id)?;
                snap.midi.get(track_id).cloned()
            });

        if let Some(midi_instances) = snapshot_clips.filter(|v| !v.is_empty()) {
            // Use snapshot data (engine is source of truth)
            for mc in &midi_instances {
                if let Some((clip_doc_id, _)) = document.audio_clip_by_midi_clip_id(mc.clip_id) {
                    let clip_doc_id = clip_doc_id; // doc-side AudioClip uuid
                    let duration = mc.external_duration;
                    let instance_uuid = Uuid::nil(); // no doc-side instance uuid yet
                    clip_data.push((mc.clip_id, mc.external_start.beats_to_f64(), mc.internal_start.beats_to_f64(), duration.beats_to_f64(), instance_uuid));
                    let _ = clip_doc_id; // used above for the if-let pattern
                }
            }
        } else {
            // Fall back to document (handles recording-in-progress and pre-snapshot clips)
            if let Some(AnyLayer::Audio(audio_layer)) = document.get_layer(&layer_id) {
                for instance in &audio_layer.clip_instances {
                    if let Some(clip) = document.audio_clips.get(&instance.clip_id) {
                        if let AudioClipType::Midi { midi_clip_id } = clip.clip_type {
                            let duration = instance.effective_duration(clip.duration, document.tempo_map());
                            clip_data.push((midi_clip_id, instance.timeline_start, instance.trim_start, duration, instance.id));
                        }
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

        // Apply quantize if the user changed the snap dropdown (must happen before handle_input
        // which may clear the selection when the ComboBox click propagates to the grid).
        if self.snap_user_changed {
            self.snap_user_changed = false;
            if self.snap_value != SnapValue::None && !self.selected_note_indices.is_empty() {
                if let Some(clip_id) = self.selected_clip_id {
                    let tempo_map = shared.action_executor.document().tempo_map().clone();
                    self.quantize_selected_notes(clip_id, &tempo_map, shared);
                }
            }
        }

        // Handle input before rendering
        self.handle_input(ui, grid_rect, keyboard_rect, shared, &clip_data);

        // Auto-scroll during playback: pin playhead to center of viewport
        if *shared.is_playing && self.auto_scroll_enabled && !self.user_scrolled_since_play {
            self.viewport_start_time = *shared.playback_time - (grid_rect.width() * 0.5 / self.pixels_per_second) as f64;
            self.viewport_start_time = self.viewport_start_time.max(0.0);
        }

        // Reset user_scrolled when playback stops
        if !*shared.is_playing {
            self.user_scrolled_since_play = false;
        }

        let painter = ui.painter_at(rect);

        // Background
        let bg = shared.theme.bg_color(&["#piano-roll", ".pane-content"], ui.ctx(), Color32::from_rgb(30, 30, 35));
        painter.rect_filled(rect, 0.0, bg);

        // Render grid (clipped to grid area)
        let grid_painter = ui.painter_at(grid_rect);
        let (grid_tempo_map, grid_time_sig) = {
            let doc = shared.action_executor.document();
            (doc.tempo_map().clone(), doc.time_signature.clone())
        };
        self.render_grid(&grid_painter, grid_rect, &grid_tempo_map, &grid_time_sig);

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
                self.render_notes(&grid_painter, grid_rect, &resolved, events, timeline_start, trim_start, duration, opacity, is_selected, midi_clip_id);
            }
        }

        // Render temp note being created
        if let Some(ref temp) = self.creating_note {
            if let Some(selected_clip) = clip_data.iter().find(|c| Some(c.0) == self.selected_clip_id) {
                let timeline_start = selected_clip.1;
                let trim_start = selected_clip.2;
                let x = self.time_to_x(timeline_start + (temp.start_time - trim_start), grid_rect);
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

    fn render_grid(&self, painter: &egui::Painter, grid_rect: Rect,
                   tempo_map: &daw_backend::TempoMap, time_sig: &lightningbeam_core::document::TimeSignature) {
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

        // Vertical lines (beat-aligned time grid)
        let interval = self.ruler_interval(tempo_map, time_sig);
        let beat_dur = lightningbeam_core::beat_time::beat_duration(0.0, tempo_map);
        let measure_dur = lightningbeam_core::beat_time::measure_duration(0.0, tempo_map, time_sig);

        let start = (self.viewport_start_time / interval).floor() as i64;
        let end_time = self.viewport_start_time + (grid_rect.width() / self.pixels_per_second) as f64;
        let end = (end_time / interval).ceil() as i64;

        for i in start..=end {
            let time = i as f64 * interval;
            let x = self.time_to_x(time, grid_rect);
            if x < grid_rect.min.x || x > grid_rect.max.x {
                continue;
            }

            // Determine tick importance: measure boundary > beat > subdivision
            let is_measure = (time / measure_dur).fract().abs() < 1e-9 || (time / measure_dur).fract() > 1.0 - 1e-9;
            let is_beat = (time / beat_dur).fract().abs() < 1e-9 || (time / beat_dur).fract() > 1.0 - 1e-9;
            let alpha = if is_measure { 60 } else if is_beat { 35 } else { 20 };

            painter.line_segment(
                [pos2(x, grid_rect.min.y), pos2(x, grid_rect.max.y)],
                Stroke::new(1.0, Color32::from_white_alpha(alpha)),
            );

            // Labels at measure boundaries
            if is_measure && x > grid_rect.min.x + 20.0 {
                let pos = lightningbeam_core::beat_time::time_to_measure(time, tempo_map, time_sig);
                painter.text(
                    pos2(x + 2.0, grid_rect.min.y + 2.0),
                    Align2::LEFT_TOP,
                    format!("{}", pos.measure),
                    FontId::proportional(9.0),
                    Color32::from_white_alpha(80),
                );
            } else if is_beat && !is_measure && x > grid_rect.min.x + 20.0
                && beat_dur as f32 * self.pixels_per_second > 40.0 {
                let pos = lightningbeam_core::beat_time::time_to_measure(time, tempo_map, time_sig);
                painter.text(
                    pos2(x + 2.0, grid_rect.min.y + 2.0),
                    Align2::LEFT_TOP,
                    format!("{}.{}", pos.measure, pos.beat),
                    FontId::proportional(9.0),
                    Color32::from_white_alpha(50),
                );
            }
        }
    }

    /// Find the peak pitch bend value (in semitones) for a note in the event list.
    /// Returns 0.0 if no pitch bend events are present in the note's time range.
    fn find_peak_pitch_bend_semitones(
        events: &[daw_backend::audio::midi::MidiEvent],
        note_start: f64,
        note_end: f64,
        channel: u8,
        pitch_bend_range: f32,
    ) -> f32 {
        let mut peak = 0.0f32;
        for ev in events {
            if ev.timestamp.beats_to_f64() > note_end + 0.01 { break; }
            if ev.timestamp.beats_to_f64() >= note_start - 0.01
                && (ev.status & 0xF0) == 0xE0
                && (ev.status & 0x0F) == channel
            {
                let raw = ((ev.data2 as i16) << 7) | (ev.data1 as i16);
                let normalized = (raw - 8192) as f32 / 8192.0;
                let semitones = normalized * pitch_bend_range;
                if semitones.abs() > peak.abs() {
                    peak = semitones;
                }
            }
        }
        peak
    }

    /// Determine which zone of a note was clicked based on the X position within the note rect.
    fn pitch_bend_zone_from_x(click_x: f32, note_left: f32, note_right: f32) -> PitchBendZone {
        let t = (click_x - note_left) / (note_right - note_left).max(1.0);
        if t < 0.3 {
            PitchBendZone::Start
        } else if t < 0.7 {
            PitchBendZone::Middle
        } else {
            PitchBendZone::End
        }
    }

    /// Find the lowest available MIDI channel (1–15) not already used by any note
    /// overlapping [note_start, note_end], excluding the note being assigned itself.
    /// Returns the note's current channel unchanged if it is already uniquely assigned (non-zero).
    fn find_or_assign_channel(
        events: &[daw_backend::audio::midi::MidiEvent],
        note_start: f64,
        note_end: f64,
        note_pitch: u8,
        current_channel: u8,
    ) -> u8 {
        use std::collections::HashMap;
        let mut used = [false; 16];
        // Walk events to find which channels have notes overlapping the target range.
        // key = (pitch, channel), value = note_start_time
        let mut active: HashMap<(u8, u8), f64> = HashMap::new();
        for ev in events {
            let ch = ev.status & 0x0F;
            let msg = ev.status & 0xF0;
            if msg == 0x90 && ev.data2 > 0 {
                active.insert((ev.data1, ch), ev.timestamp.beats_to_f64());
            } else if msg == 0x80 || (msg == 0x90 && ev.data2 == 0) {
                if let Some(start) = active.remove(&(ev.data1, ch)) {
                    // Overlaps target range and is NOT the note we're assigning
                    if start < note_end && ev.timestamp.beats_to_f64() > note_start
                        && !(ev.data1 == note_pitch && ch == current_channel)
                    {
                        used[ch as usize] = true;
                    }
                }
            }
        }
        // Mark still-active (no note-off seen) notes
        for ((pitch, ch), start) in &active {
            if *start < note_end && !(*pitch == note_pitch && *ch == current_channel) {
                used[*ch as usize] = true;
            }
        }
        // Keep current channel if already uniquely assigned and non-zero
        if current_channel != 0 && !used[current_channel as usize] {
            return current_channel;
        }
        // Find lowest free channel in 1..15
        for ch in 1u8..16 {
            if !used[ch as usize] { return ch; }
        }
        current_channel // fallback (>15 simultaneous notes)
    }

    /// Find the CC1 (modulation) value for a note in the event list.
    /// Searches for a CC1 event at or just before the note's start time on the same channel.
    fn find_cc1_for_note(events: &[daw_backend::audio::midi::MidiEvent], note_start: f64, note_end: f64, channel: u8) -> u8 {
        let mut cc1 = 0u8;
        for ev in events {
            if ev.timestamp.beats_to_f64() > note_end { break; }
            if (ev.status & 0xF0) == 0xB0 && (ev.status & 0x0F) == channel && ev.data1 == 1 {
                if ev.timestamp.beats_to_f64() <= note_start {
                    cc1 = ev.data2;
                }
            }
        }
        cc1
    }

    fn render_notes(
        &self,
        painter: &egui::Painter,
        grid_rect: Rect,
        notes: &[ResolvedNote],
        events: &[daw_backend::audio::midi::MidiEvent],
        clip_timeline_start: f64,
        trim_start: f64,
        clip_duration: f64,
        opacity: f32,
        is_selected_clip: bool,
        clip_id: u32,
    ) {
        for (i, note) in notes.iter().enumerate() {
            // Skip notes entirely outside the visible trim window
            if note.start_time + note.duration <= trim_start {
                continue;
            }
            if note.start_time >= trim_start + clip_duration {
                continue;
            }

            let global_time = clip_timeline_start + (note.start_time - trim_start);

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

                // Modulation (CC1) bar: 3px column on left edge of note, fills from bottom
                let cc1 = Self::find_cc1_for_note(events, note.start_time, note.start_time + note.duration, note.channel);
                if cc1 > 0 {
                    let bar_width = 3.0_f32.min(clipped.width());
                    let bar_height = (cc1 as f32 / 127.0) * clipped.height();
                    let bar_rect = Rect::from_min_size(
                        pos2(clipped.min.x, clipped.max.y - bar_height),
                        vec2(bar_width, bar_height),
                    );
                    let bar_alpha = (128.0 * opacity) as u8;
                    painter.rect_filled(bar_rect, 0.0, Color32::from_rgba_unmultiplied(255, 255, 255, bar_alpha));
                }

                // Pitch bend ghost overlay — contour-following filled band
                // Build a curve of semitone values sampled across the note width.
                // For live drag: existing bend + new zone contribution (additive).
                // For persisted: sample actual events.
                const N_SAMPLES: usize = 24;
                let bend_curve: Option<[f32; N_SAMPLES + 1]> =
                    if let Some(DragMode::PitchBend { note_index: drag_idx, current_semitones, zone, note_channel: drag_ch, .. }) = self.drag_mode {
                        if drag_idx == i && is_selected_clip && Some(clip_id) == self.selected_clip_id {
                            let mut curve = [0.0f32; N_SAMPLES + 1];
                            let pi = std::f32::consts::PI;
                            for s in 0..=N_SAMPLES {
                                let t = s as f32 / N_SAMPLES as f32;
                                // Sample existing bend at this time position
                                let ts = note.start_time + t as f64 * note.duration;
                                let mut existing_norm = 0.0f32;
                                for ev in events {
                                    if ev.timestamp.beats_to_f64() > ts { break; }
                                    if (ev.status & 0xF0) == 0xE0 && (ev.status & 0x0F) == drag_ch {
                                        let raw = ((ev.data2 as i16) << 7) | (ev.data1 as i16);
                                        existing_norm = (raw - 8192) as f32 / 8192.0;
                                    }
                                }
                                let existing_semi = existing_norm * self.pitch_bend_range;
                                // New zone contribution
                                let zone_semi = match zone {
                                    PitchBendZone::Start  => current_semitones * (1.0 + (pi * t).cos()) * 0.5,
                                    PitchBendZone::Middle => current_semitones * (pi * t).sin(),
                                    PitchBendZone::End    => current_semitones * (1.0 - (pi * t).cos()) * 0.5,
                                };
                                curve[s] = existing_semi + zone_semi;
                            }
                            // Only show ghost if there's any meaningful bend at all
                            if curve.iter().any(|v| v.abs() >= 0.05) {
                                Some(curve)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                // For persisted notes (no live drag), sample actual pitch bend events
                let bend_curve = bend_curve.or_else(|| {
                    let peak = Self::find_peak_pitch_bend_semitones(
                        events, note.start_time, note.start_time + note.duration,
                        note.channel, self.pitch_bend_range);
                    if peak.abs() < 0.05 { return None; }
                    let mut curve = [0.0f32; N_SAMPLES + 1];
                    for s in 0..=N_SAMPLES {
                        let t = s as f64 / N_SAMPLES as f64;
                        let ts = note.start_time + t * note.duration;
                        // Find last pitch bend event at or before ts
                        let mut bend_norm = 0.0f32;
                        for ev in events {
                            if ev.timestamp.beats_to_f64() > ts { break; }
                            if (ev.status & 0xF0) == 0xE0 && (ev.status & 0x0F) == note.channel {
                                let raw = ((ev.data2 as i16) << 7) | (ev.data1 as i16);
                                bend_norm = (raw - 8192) as f32 / 8192.0;
                            }
                        }
                        curve[s] = bend_norm * self.pitch_bend_range;
                    }
                    Some(curve)
                });

                if let Some(curve) = bend_curve {
                    // Draw a stroked curve relative to the note's centerline.
                    let note_center_y = y + h * 0.5;
                    // Brighten toward white for visibility
                    let brighten = |c: u8| -> u8 { (c as u16 + (255 - c as u16) * 3 / 4) as u8 };
                    let stroke_color = Color32::from_rgba_unmultiplied(
                        brighten(r), brighten(g), brighten(b), (220.0 * opacity) as u8,
                    );

                    let points: Vec<egui::Pos2> = (0..=N_SAMPLES).map(|s| {
                        let t = s as f32 / N_SAMPLES as f32;
                        let px = (x + t * w).clamp(grid_rect.min.x, grid_rect.max.x);
                        let bend_px = (curve[s] * self.note_height)
                            .clamp(-(grid_rect.height()), grid_rect.height());
                        let py = (note_center_y - bend_px).clamp(grid_rect.min.y, grid_rect.max.y);
                        pos2(px, py)
                    }).collect();
                    painter.add(egui::Shape::line(points, egui::Stroke::new(3.0, stroke_color)));
                }
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

    fn render_dot_grid(&self, painter: &egui::Painter, grid_rect: Rect,
                       tempo_map: &daw_backend::TempoMap, time_sig: &lightningbeam_core::document::TimeSignature) {
        // Collect visible time grid positions
        let interval = self.ruler_interval(tempo_map, time_sig);
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
        let alt_held = ui.input(|i| i.modifiers.alt);
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
            let mut zoom_handled = false;

            // Check raw mouse wheel events to distinguish mouse wheel from trackpad
            let raw_wheel = ui.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::MouseWheel { unit, delta, modifiers } = e {
                        Some((*unit, *delta, *modifiers))
                    } else {
                        None
                    }
                })
            });

            if let Some((unit, delta, modifiers)) = raw_wheel {
                match unit {
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                        // Mouse wheel: always zoom horizontally
                        let zoom_delta = delta.y * 0.005;
                        self.apply_zoom_at_point(zoom_delta, hover_pos.x, grid_rect);
                        self.user_scrolled_since_play = true;
                        zoom_handled = true;
                    }
                    egui::MouseWheelUnit::Point => {
                        if ctrl_held || modifiers.ctrl {
                            // Trackpad + Ctrl: zoom
                            let zoom_delta = delta.y * 0.005;
                            self.apply_zoom_at_point(zoom_delta, hover_pos.x, grid_rect);
                            self.user_scrolled_since_play = true;
                            zoom_handled = true;
                        }
                    }
                }
            }

            // Trackpad panning (smooth scroll without Ctrl)
            if !zoom_handled {
                let scroll = ui.input(|i| i.smooth_scroll_delta);
                if scroll.x.abs() > 0.0 {
                    self.viewport_start_time -= (scroll.x / self.pixels_per_second) as f64;
                    self.viewport_start_time = self.viewport_start_time.max(0.0);
                    self.user_scrolled_since_play = true;
                }
                if scroll.y.abs() > 0.0 {
                    self.scroll_y -= scroll.y;
                    let max_scroll = (MAX_NOTE - MIN_NOTE + 1) as f32 * self.note_height - grid_rect.height();
                    self.scroll_y = self.scroll_y.clamp(0.0, max_scroll.max(0.0));
                }
            }
        }

        // Delete key
        let delete_pressed = ui.input(|i| shared.keymap.action_pressed_with_backspace(crate::keymap::AppAction::PianoRollDelete, i));
        if delete_pressed && !self.selected_note_indices.is_empty() {
            if let Some(clip_id) = self.selected_clip_id {
                self.delete_selected_notes(clip_id, shared, clip_data);
            }
        }

        // Copy/Cut/Paste — winit converts Ctrl+C/X/V to Event::Copy/Cut/Paste
        let (has_copy, has_cut, has_paste) = ui.input(|i| {
            let mut copy = false;
            let mut cut = false;
            let mut paste = false;
            for event in &i.events {
                match event {
                    egui::Event::Copy => copy = true,
                    egui::Event::Cut => cut = true,
                    egui::Event::Paste(_) => paste = true,
                    _ => {}
                }
            }
            (copy, cut, paste)
        });

        if has_copy && !self.selected_note_indices.is_empty() {
            if let Some(clip_id) = self.selected_clip_id {
                self.copy_selected_notes(clip_id, shared);
                *shared.clipboard_consumed = true;
            }
        }

        if has_cut && !self.selected_note_indices.is_empty() {
            if let Some(clip_id) = self.selected_clip_id {
                self.copy_selected_notes(clip_id, shared);
                self.delete_selected_notes(clip_id, shared, clip_data);
                *shared.clipboard_consumed = true;
            }
        }

        if has_paste {
            if let Some(clip_id) = self.selected_clip_id {
                // Only consume if clipboard has MIDI notes
                if shared.clipboard_manager.has_content() {
                    if let Some(lightningbeam_core::clipboard::ClipboardContent::MidiNotes { .. }) = shared.clipboard_manager.paste() {
                        self.paste_notes(clip_id, shared, clip_data);
                        *shared.clipboard_consumed = true;
                    }
                }
            }
        }

        // Immediate press detection (fires on the actual press frame, before egui's drag threshold).
        // This ensures note preview and hit testing use the real press position.
        // Skip when any popup (e.g. ComboBox dropdown) is open so clicks there don't pass through.
        let pointer_just_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary))
            && !ui.ctx().is_popup_open();
        if pointer_just_pressed {
            if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                if full_rect.contains(pos) {
                    let in_grid = pos.x >= grid_rect.min.x;
                    if in_grid {
                        self.on_grid_press(pos, grid_rect, shift_held, ctrl_held, alt_held, now, shared, clip_data);
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
        if matches!(self.drag_mode, Some(DragMode::PitchBend { .. })) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        } else if let Some(hover_pos) = response.hover_pos() {
            if hover_pos.x >= grid_rect.min.x {
                if shift_held {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if alt_held && self.hit_test_note(hover_pos, grid_rect, shared, clip_data).is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
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
        alt_held: bool,
        now: f64,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let time = self.x_to_time(pos.x, grid_rect);
        let note = self.y_to_note(pos.y, grid_rect);
        self.drag_start_screen = Some(pos);
        self.drag_start_time = time;
        self.drag_start_note = note;

        // Alt+click on a note: start pitch bend drag
        if alt_held {
            if let Some(note_idx) = self.hit_test_note(pos, grid_rect, shared, clip_data) {
                if let Some(clip_id) = self.selected_clip_id {
                    if let Some(events) = shared.midi_event_cache.get(&clip_id) {
                        let resolved = Self::resolve_notes(events);
                        if note_idx < resolved.len() {
                            let n = &resolved[note_idx];
                            // Determine zone from X position within note rect
                            let note_x = self.time_to_x(n.start_time, grid_rect);
                            let note_w = (n.duration as f32 * self.pixels_per_second).max(2.0);
                            let zone = Self::pitch_bend_zone_from_x(pos.x, note_x, note_x + note_w);
                            self.drag_mode = Some(DragMode::PitchBend {
                                note_index: note_idx,
                                zone,
                                note_pitch: n.note,
                                note_channel: n.channel,
                                note_start: n.start_time,
                                note_duration: n.duration,
                                origin_y: pos.y,
                                current_semitones: 0.0, // additive delta; existing bend shown separately
                            });
                            return;
                        }
                    }
                }
            }
        }

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
            self.update_focus(shared);

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
                let trim_start = selected_clip.2;
                let tempo_map = shared.action_executor.document().tempo_map();
                let clip_local_time = snap_to_value(
                    (time - clip_start).max(0.0) + trim_start,
                    self.snap_value, tempo_map,
                );
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
            // Start selection rectangle and seek playhead to clicked time
            self.selected_note_indices.clear();
            self.update_focus(shared);
            self.selection_rect = Some((pos, pos));
            self.drag_mode = Some(DragMode::SelectRect);

            let tempo_map = shared.action_executor.document().tempo_map();
            let seek_time = snap_to_value(time.max(0.0), self.snap_value, tempo_map);
            *shared.playback_time = seek_time;
            if let Some(ctrl) = shared.audio_controller.as_ref() {
                if let Ok(mut c) = ctrl.lock() { c.seek(seek_time); }
            }
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
                        let trim_start = selected_clip.2;
                        let clip_local_time = (time - clip_start).max(0.0) + trim_start;
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
            Some(DragMode::PitchBend { .. }) => {
                // Handled below (needs mutable access to self.drag_mode and self.pitch_bend_range)
            }
            None => {}
        }

        // Pitch bend drag: update current_semitones based on Y movement
        if let Some(DragMode::PitchBend { ref mut current_semitones, ref mut origin_y, .. }) = self.drag_mode {
            let range = self.pitch_bend_range;
            let delta_semitones = (*origin_y - pos.y) / self.note_height;
            *current_semitones = (*current_semitones + delta_semitones).clamp(-range, range);
            *origin_y = pos.y;
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
                self.update_focus(shared);
            }
            Some(DragMode::PitchBend { note_pitch, note_channel, note_start, note_duration, zone, current_semitones, .. }) => {
                // Only commit if the drag added a meaningful new contribution
                if current_semitones.abs() >= 0.05 {
                    if let Some(clip_id) = self.selected_clip_id {
                        let range = self.pitch_bend_range;
                        let old_events = shared.midi_event_cache.get(&clip_id).cloned().unwrap_or_default();
                        let mut new_events = old_events.clone();

                        // Assign a unique channel to this note so bend only affects it
                        let target_channel = Self::find_or_assign_channel(
                            &new_events, note_start, note_start + note_duration,
                            note_pitch, note_channel,
                        );

                        // Re-stamp note-on/off for this specific note if channel changed
                        if target_channel != note_channel {
                            for ev in &mut new_events {
                                let msg = ev.status & 0xF0;
                                let ch = ev.status & 0x0F;
                                if (msg == 0x90 || msg == 0x80) && ev.data1 == note_pitch && ch == note_channel {
                                    ev.status = (ev.status & 0xF0) | target_channel;
                                }
                            }
                        }

                        // Sample existing bend (normalised -1..1) at each step, then add the
                        // new zone contribution additively and write back as combined events.
                        let num_steps: usize = 128;
                        let pi = std::f32::consts::PI;
                        let existing_norm: Vec<f32> = (0..=num_steps).map(|i| {
                            let t = i as f64 / num_steps as f64;
                            let ts = note_start + t * note_duration;
                            let mut bend = 0.0f32;
                            for ev in &new_events {
                                if ev.timestamp.beats_to_f64() > ts { break; }
                                if (ev.status & 0xF0) == 0xE0 && (ev.status & 0x0F) == target_channel {
                                    let raw = ((ev.data2 as i16) << 7) | (ev.data1 as i16);
                                    bend = (raw - 8192) as f32 / 8192.0;
                                }
                            }
                            bend
                        }).collect();

                        // Remove old bend events in range before writing combined
                        new_events.retain(|ev| {
                            let is_bend = (ev.status & 0xF0) == 0xE0 && (ev.status & 0x0F) == target_channel;
                            let ts = ev.timestamp.beats_to_f64();
                            let in_range = ts >= note_start - 0.001 && ts <= note_start + note_duration + 0.01;
                            !(is_bend && in_range)
                        });

                        let encode_bend = |normalized: f32| -> (u8, u8) {
                            let v = (normalized * 8191.0 + 8192.0).clamp(0.0, 16383.0) as i16;
                            ((v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8)
                        };
                        for i in 0..=num_steps {
                            let t = i as f32 / num_steps as f32;
                            let zone_norm = match zone {
                                PitchBendZone::Start  => current_semitones / range * (1.0 + (pi * t).cos()) * 0.5,
                                PitchBendZone::Middle => current_semitones / range * (pi * t).sin(),
                                PitchBendZone::End    => current_semitones / range * (1.0 - (pi * t).cos()) * 0.5,
                            };
                            let combined = (existing_norm[i] + zone_norm).clamp(-1.0, 1.0);
                            let (lsb, msb) = encode_bend(combined);
                            let ts = note_start + i as f64 / num_steps as f64 * note_duration;
                            new_events.push(daw_backend::audio::midi::MidiEvent::new(daw_backend::Beats(ts), 0xE0 | target_channel, lsb, msb));
                        }
                        // For End zone: reset just after note ends so it doesn't bleed into next note
                        if zone == PitchBendZone::End {
                            let (lsb, msb) = encode_bend(0.0);
                            new_events.push(daw_backend::audio::midi::MidiEvent::new(daw_backend::Beats(note_start + note_duration + 0.005), 0xE0 | target_channel, lsb, msb));
                        }

                        new_events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap_or(std::cmp::Ordering::Equal));
                        self.push_events_action("Set pitch bend", clip_id, old_events, new_events.clone(), shared);
                        shared.midi_event_cache.insert(clip_id, new_events);
                    }
                }
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
        let trim_start = clip_info.2;
        let clip_duration = clip_info.3;

        for (i, note) in resolved.iter().enumerate().rev() {
            // Skip notes outside trim window
            if note.start_time + note.duration <= trim_start || note.start_time >= trim_start + clip_duration {
                continue;
            }
            let x = self.time_to_x(timeline_start + (note.start_time - trim_start), grid_rect);
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
        let trim_start = clip_info.2;
        let clip_duration = clip_info.3;

        for (i, note) in resolved.iter().enumerate().rev() {
            // Skip notes outside trim window
            if note.start_time + note.duration <= trim_start || note.start_time >= trim_start + clip_duration {
                continue;
            }
            let x = self.time_to_x(timeline_start + (note.start_time - trim_start), grid_rect);
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

    fn update_focus(&self, shared: &mut SharedPaneState) {
        if self.selected_note_indices.is_empty() {
            *shared.focus = lightningbeam_core::selection::FocusSelection::None;
        } else if let (Some(layer_id), Some(midi_clip_id)) = (*shared.active_layer_id, self.selected_clip_id) {
            *shared.focus = lightningbeam_core::selection::FocusSelection::Notes {
                layer_id,
                midi_clip_id,
                indices: self.selected_note_indices.iter().copied().collect(),
            };
        }
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
        let trim_start = clip_info.2;
        let clip_duration = clip_info.3;

        for (i, note) in resolved.iter().enumerate() {
            // Skip notes outside trim window
            if note.start_time + note.duration <= trim_start || note.start_time >= trim_start + clip_duration {
                continue;
            }
            let x = self.time_to_x(timeline_start + (note.start_time - trim_start), grid_rect);
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
        let mut events: Vec<daw_backend::audio::midi::MidiEvent> = Vec::with_capacity(resolved.len() * 2);
        for n in resolved {
            events.push(daw_backend::audio::midi::MidiEvent::note_on(daw_backend::Beats(n.start_time), 0, n.note, n.velocity));
            events.push(daw_backend::audio::midi::MidiEvent::note_off(daw_backend::Beats(n.start_time + n.duration), 0, n.note, 0));
        }
        events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
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
            channel: 0,
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

        let tempo_map = shared.action_executor.document().tempo_map();
        let mut new_resolved = resolved.clone();
        for &idx in &self.selected_note_indices {
            if idx < new_resolved.len() {
                let raw_time = (new_resolved[idx].start_time + dt).max(0.0);
                new_resolved[idx].start_time = snap_to_value(raw_time, self.snap_value, tempo_map);
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

    fn copy_selected_notes(&self, clip_id: u32, shared: &mut SharedPaneState) {
        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let resolved = Self::resolve_notes(events);

        // Collect selected notes
        let selected: Vec<&ResolvedNote> = self.selected_note_indices.iter()
            .filter_map(|&i| resolved.get(i))
            .collect();

        if selected.is_empty() {
            return;
        }

        // Find earliest start time as base offset
        let min_time = selected.iter()
            .map(|n| n.start_time)
            .fold(f64::INFINITY, f64::min);

        // Store as relative times
        let notes: Vec<(f64, u8, u8, f64)> = selected.iter()
            .map(|n| (n.start_time - min_time, n.note, n.velocity, n.duration))
            .collect();

        shared.clipboard_manager.copy(
            lightningbeam_core::clipboard::ClipboardContent::MidiNotes { notes }
        );
    }

    fn paste_notes(
        &mut self,
        clip_id: u32,
        shared: &mut SharedPaneState,
        clip_data: &[(u32, f64, f64, f64, Uuid)],
    ) {
        let notes_to_paste = match shared.clipboard_manager.paste() {
            Some(lightningbeam_core::clipboard::ClipboardContent::MidiNotes { notes }) => notes,
            _ => return,
        };

        if notes_to_paste.is_empty() {
            return;
        }

        // Get clip info for trim offset
        let clip_info = match clip_data.iter().find(|c| c.0 == clip_id) {
            Some(c) => c,
            None => return,
        };
        let clip_start = clip_info.1;
        let trim_start = clip_info.2;

        // Place pasted notes at current playhead position (clip-local time)
        let paste_time = (*shared.playback_time - clip_start).max(0.0) + trim_start;

        let events = match shared.midi_event_cache.get(&clip_id) {
            Some(e) => e,
            None => return,
        };
        let mut resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);

        let paste_start_index = resolved.len();
        for &(rel_time, note, velocity, duration) in &notes_to_paste {
            resolved.push(ResolvedNote {
                note,
                channel: 0,
                start_time: paste_time + rel_time,
                duration,
                velocity,
            });
        }
        let new_notes = Self::notes_to_backend_format(&resolved);

        Self::update_cache_from_resolved(clip_id, &resolved, shared);
        self.push_update_action("Paste notes", clip_id, old_notes, new_notes, shared, clip_data);

        // Select the pasted notes
        self.selected_note_indices.clear();
        for i in paste_start_index..resolved.len() {
            self.selected_note_indices.insert(i);
        }
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

    fn quantize_selected_notes(&mut self, clip_id: u32, tempo_map: &daw_backend::TempoMap, shared: &mut SharedPaneState) {
        let events = match shared.midi_event_cache.get(&clip_id) { Some(e) => e, None => return };
        let resolved = Self::resolve_notes(events);
        let old_notes = Self::notes_to_backend_format(&resolved);
        let mut new_resolved = resolved.clone();
        for &idx in &self.selected_note_indices {
            if idx < new_resolved.len() {
                new_resolved[idx].start_time =
                    snap_to_value(new_resolved[idx].start_time, self.snap_value, tempo_map).max(0.0);
            }
        }
        let new_notes = Self::notes_to_backend_format(&new_resolved);
        Self::update_cache_from_resolved(clip_id, &new_resolved, shared);
        self.push_update_action("Quantize notes", clip_id, old_notes, new_notes, shared, &[]);
        self.cached_clip_id = None;
    }

    fn push_events_action(
        &self,
        description: &str,
        clip_id: u32,
        old_events: Vec<daw_backend::audio::midi::MidiEvent>,
        new_events: Vec<daw_backend::audio::midi::MidiEvent>,
        shared: &mut SharedPaneState,
    ) {
        let layer_id = match *shared.active_layer_id {
            Some(id) => id,
            None => return,
        };
        let action = lightningbeam_core::actions::UpdateMidiEventsAction {
            layer_id,
            midi_clip_id: clip_id,
            old_events,
            new_events,
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
        let spec_bg = shared.theme.bg_color(&["#piano-roll", ".pane-content"], ui.ctx(), Color32::from_rgb(20, 20, 25));
        painter.rect_filled(rect, 0.0, spec_bg);

        // Dot grid background (visible where the spectrogram doesn't draw)
        let grid_painter = ui.painter_at(view_rect);
        {
            let (dot_tempo_map, dot_ts) = {
                let doc = shared.action_executor.document();
                (doc.tempo_map().clone(), doc.time_signature.clone())
            };
            self.render_dot_grid(&grid_painter, view_rect, &dot_tempo_map, &dot_ts);
        }

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

        // Render CQT spectrogram for each sampled clip on this layer
        for &(pool_index, timeline_start, trim_start, _duration, sample_rate) in &clip_infos {
            // Get audio duration from the raw audio cache
            let audio_duration = if let Some((samples, sr, ch)) = shared.raw_audio_cache.get(&pool_index) {
                samples.len() as f64 / (*sr as f64 * *ch as f64)
            } else {
                continue;
            };

            if view_rect.width() > 0.0 && view_rect.height() > 0.0 {
                // Calculate visible CQT column range for streaming
                let viewport_end_time = self.viewport_start_time + (view_rect.width() / self.pixels_per_second) as f64;
                let vis_audio_start = (self.viewport_start_time - timeline_start + trim_start).max(0.0);
                let vis_audio_end = (viewport_end_time - timeline_start + trim_start).min(audio_duration);
                let vis_col_start = (vis_audio_start * sample_rate as f64 / 512.0).floor() as i64;
                let vis_col_end = (vis_audio_end * sample_rate as f64 / 512.0).ceil() as i64 + 1;

                // Calculate stride: how many CQT columns per pixel
                // When zoomed out, multiple CQT columns map to one pixel — compute every Nth
                let cols_per_pixel = sample_rate as f32 / (512.0 * self.pixels_per_second);
                let cqt_stride = (cols_per_pixel.ceil() as u32).max(1);

                let callback = crate::cqt_gpu::CqtCallback {
                    pool_index,
                    params: crate::cqt_gpu::CqtRenderParams {
                        clip_rect: [view_rect.min.x, view_rect.min.y, view_rect.max.x, view_rect.max.y],
                        viewport_start_time: self.viewport_start_time as f32,
                        pixels_per_second: self.pixels_per_second,
                        audio_duration: audio_duration as f32,
                        sample_rate: sample_rate as f32,
                        clip_start_time: timeline_start as f32,
                        trim_start: trim_start as f32,
                        freq_bins: 174.0,
                        bins_per_octave: 24.0,
                        hop_size: 512.0,
                        scroll_y: self.scroll_y,
                        note_height: self.note_height,
                        min_note: MIN_NOTE as f32,
                        max_note: MAX_NOTE as f32,
                        gamma: self.spectrogram_gamma,
                        cache_capacity: 0.0, // filled by prepare()
                        cache_start_column: 0.0,
                        cache_valid_start: 0.0,
                        cache_valid_end: 0.0,
                        column_stride: 0.0, // filled by prepare()
                        _pad: 0.0,
                    },
                    target_format: shared.target_format,
                    sample_rate,
                    visible_col_start: vis_col_start,
                    visible_col_end: vis_col_end,
                    stride: cqt_stride,
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
            let ctrl_held = ui.input(|i| i.modifiers.ctrl);
            let mut zoom_handled = false;

            let raw_wheel = ui.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::MouseWheel { unit, delta, modifiers } = e {
                        Some((*unit, *delta, *modifiers))
                    } else {
                        None
                    }
                })
            });

            if let Some((unit, delta, modifiers)) = raw_wheel {
                match unit {
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => {
                        let zoom_delta = delta.y * 0.005;
                        self.apply_zoom_at_point(zoom_delta, hover_pos.x, view_rect);
                        self.user_scrolled_since_play = true;
                        zoom_handled = true;
                    }
                    egui::MouseWheelUnit::Point => {
                        if ctrl_held || modifiers.ctrl {
                            let zoom_delta = delta.y * 0.005;
                            self.apply_zoom_at_point(zoom_delta, hover_pos.x, view_rect);
                            self.user_scrolled_since_play = true;
                            zoom_handled = true;
                        }
                    }
                }
            }

            if !zoom_handled {
                let scroll = ui.input(|i| i.smooth_scroll_delta);
                if scroll.x.abs() > 0.0 {
                    self.viewport_start_time -= (scroll.x / self.pixels_per_second) as f64;
                    self.viewport_start_time = self.viewport_start_time.max(0.0);
                    self.user_scrolled_since_play = true;
                }
                if scroll.y.abs() > 0.0 {
                    self.scroll_y -= scroll.y;
                    let max_scroll = (MAX_NOTE - MIN_NOTE + 1) as f32 * self.note_height - view_rect.height();
                    self.scroll_y = self.scroll_y.clamp(0.0, max_scroll.max(0.0));
                }
            }
        }

        // Playhead
        let playhead_painter = ui.painter_at(view_rect);
        self.render_playhead(&playhead_painter, view_rect, *shared.playback_time);

        // Keyboard on top (same as MIDI mode)
        self.render_keyboard(&painter, keyboard_rect);

        // Auto-scroll during playback: pin playhead to center of viewport
        if *shared.is_playing && self.auto_scroll_enabled && !self.user_scrolled_since_play {
            self.viewport_start_time = *shared.playback_time - (view_rect.width() * 0.5 / self.pixels_per_second) as f64;
            self.viewport_start_time = self.viewport_start_time.max(0.0);
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
            let header_text = shared.theme.text_color(&["#piano-roll", ".pane-header"], ui.ctx(), Color32::from_gray(180));
            let header_secondary = shared.theme.text_color(&["#piano-roll", ".text-secondary"], ui.ctx(), Color32::from_gray(140));
            let header_accent = shared.theme.text_color(&["#piano-roll", ".status-success"], ui.ctx(), Color32::from_rgb(143, 252, 143));

            // Pane title
            ui.label(
                egui::RichText::new("Piano Roll")
                    .color(header_text)
                    .size(11.0),
            );
            ui.separator();

            // Zoom
            ui.label(
                egui::RichText::new(format!("{:.0}px/s", self.pixels_per_second))
                    .color(header_secondary)
                    .size(10.0),
            );

            // Selected notes count
            if !self.selected_note_indices.is_empty() {
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("{} selected", self.selected_note_indices.len()))
                        .color(header_accent)
                        .size(10.0),
                );
            }

            // Velocity + modulation sliders for selected note(s)
            if !self.selected_note_indices.is_empty() {
                if let Some(clip_id) = self.selected_clip_id {
                    if let Some(events) = shared.midi_event_cache.get(&clip_id).cloned() {
                        let resolved = Self::resolve_notes(&events);
                        // Pick the first selected note as the representative value
                        let first_idx = self.selected_note_indices.iter().copied().next();
                        if let Some(idx) = first_idx {
                            if idx < resolved.len() {
                                let n = &resolved[idx];

                                // ── Velocity ──────────────────────────────
                                ui.separator();
                                ui.label(egui::RichText::new("Vel").color(header_secondary).size(10.0));
                                let vel_resp = ui.add(
                                    egui::DragValue::new(&mut self.header_vel)
                                        .range(1.0..=127.0)
                                        .max_decimals(0)
                                        .speed(1.0),
                                );
                                // Commit before syncing so header_vel isn't overwritten first
                                if vel_resp.drag_stopped() || vel_resp.lost_focus() {
                                    let new_vel = self.header_vel.round().clamp(1.0, 127.0) as u8;
                                    if new_vel != n.velocity {
                                        let old_notes = Self::notes_to_backend_format(&resolved);
                                        let mut new_resolved = resolved.clone();
                                        for &i in &self.selected_note_indices {
                                            if i < new_resolved.len() {
                                                new_resolved[i].velocity = new_vel;
                                            }
                                        }
                                        let new_notes = Self::notes_to_backend_format(&new_resolved);
                                        self.push_update_action("Set velocity", clip_id, old_notes, new_notes, shared, &[]);
                                        // Patch the event cache immediately so next frame sees the new velocity
                                        if let Some(cached) = shared.midi_event_cache.get_mut(&clip_id) {
                                            for &i in &self.selected_note_indices {
                                                if i >= resolved.len() { continue; }
                                                let sn = &resolved[i];
                                                for ev in cached.iter_mut() {
                                                    if ev.is_note_on() && ev.data1 == sn.note
                                                        && (ev.status & 0x0F) == sn.channel
                                                        && (ev.timestamp.beats_to_f64() - sn.start_time).abs() < 1e-6
                                                    {
                                                        ev.data2 = new_vel;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                // Sync from note only when idle (not on commit frames)
                                if !vel_resp.dragged() && !vel_resp.has_focus() && !vel_resp.drag_stopped() && !vel_resp.lost_focus() {
                                    self.header_vel = n.velocity as f32;
                                }

                                // ── Modulation (CC1) ──────────────────────
                                ui.separator();
                                ui.label(egui::RichText::new("Mod").color(header_secondary).size(10.0));
                                let current_cc1 = Self::find_cc1_for_note(&events, n.start_time, n.start_time + n.duration, n.channel);
                                let mod_resp = ui.add(
                                    egui::DragValue::new(&mut self.header_mod)
                                        .range(0.0..=127.0)
                                        .max_decimals(0)
                                        .speed(1.0),
                                );
                                // Commit before syncing
                                if mod_resp.drag_stopped() || mod_resp.lost_focus() {
                                    let new_cc1 = self.header_mod.round().clamp(0.0, 127.0) as u8;
                                    if new_cc1 != current_cc1 {
                                        let old_events = events.clone();
                                        let mut new_events = events.clone();
                                        for &i in &self.selected_note_indices {
                                            if i >= resolved.len() { continue; }
                                            let sn = &resolved[i];
                                            new_events.retain(|ev| {
                                                let is_cc1 = (ev.status & 0xF0) == 0xB0
                                                    && (ev.status & 0x0F) == sn.channel
                                                    && ev.data1 == 1;
                                                let at_start = (ev.timestamp.beats_to_f64() - sn.start_time).abs() < 0.001;
                                                !(is_cc1 && at_start)
                                            });
                                            if new_cc1 > 0 {
                                                new_events.push(daw_backend::audio::midi::MidiEvent::new(
                                                    daw_backend::Beats(sn.start_time), 0xB0 | sn.channel, 1, new_cc1,
                                                ));
                                            }
                                        }
                                        new_events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap_or(std::cmp::Ordering::Equal));
                                        self.push_events_action("Set modulation", clip_id, old_events, new_events.clone(), shared);
                                        shared.midi_event_cache.insert(clip_id, new_events);
                                    }
                                }
                                // Sync from note only when idle
                                if !mod_resp.dragged() && !mod_resp.has_focus() && !mod_resp.drag_stopped() && !mod_resp.lost_focus() {
                                    self.header_mod = current_cc1 as f32;
                                }
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
                        .color(header_secondary)
                        .size(10.0),
                );
                ui.add(
                    egui::DragValue::new(&mut self.spectrogram_gamma)
                        .speed(0.05)
                        .range(0.5..=10.0)
                        .max_decimals(1),
                );
            }

            // Snap-to dropdown — only in Measures mode
            let doc = shared.action_executor.document();
            let is_measures = doc.timeline_mode == lightningbeam_core::document::TimelineMode::Measures;
            let tempo_map = doc.tempo_map();
            let _ = doc;

            if is_measures {
                // Auto-detect grid when selection changes
                if self.selected_note_indices != self.last_snap_selection {
                    if !self.selected_note_indices.is_empty() {
                        if let Some(clip_id) = self.selected_clip_id {
                            if let Some(events) = shared.midi_event_cache.get(&clip_id) {
                                let resolved = Self::resolve_notes(events);
                                let sel: Vec<&ResolvedNote> = self.selected_note_indices.iter()
                                    .filter_map(|&i| resolved.get(i))
                                    .collect();
                                self.snap_value = detect_snap(&sel, tempo_map);
                            }
                        }
                    }
                    self.last_snap_selection = self.selected_note_indices.clone();
                }

                ui.separator();
                ui.label(egui::RichText::new("Snap to:").color(header_secondary).size(10.0));
                let old_snap = self.snap_value;
                egui::ComboBox::from_id_salt("piano_roll_snap")
                    .selected_text(self.snap_value.label())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for &sv in SnapValue::all() {
                            ui.selectable_value(&mut self.snap_value, sv, sv.label());
                        }
                    });

                if self.snap_value != old_snap {
                    self.snap_user_changed = true;
                }
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
