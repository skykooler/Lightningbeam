/// Virtual Piano Pane - On-screen piano keyboard for live MIDI input
///
/// Provides a clickable/draggable piano keyboard that sends MIDI note events
/// to the currently active MIDI track via daw-backend.

use super::{NodePath, PaneRenderer, SharedPaneState};
use eframe::egui;
use std::collections::HashSet;

/// Virtual piano pane state
pub struct VirtualPianoPane {
    /// White key aspect ratio (height:width) - matches JS version
    white_key_aspect_ratio: f32,
    /// Black key width ratio relative to white keys
    black_key_width_ratio: f32,
    /// Black key height ratio relative to white keys
    black_key_height_ratio: f32,
    /// Currently pressed notes (for visual feedback)
    pressed_notes: HashSet<u8>,
    /// Note being held by mouse drag (to prevent retriggering)
    dragging_note: Option<u8>,
    /// Octave offset for keyboard mapping (default: 0 = C4)
    octave_offset: i8,
}

impl Default for VirtualPianoPane {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualPianoPane {
    pub fn new() -> Self {
        Self {
            white_key_aspect_ratio: 6.0,
            black_key_width_ratio: 0.6,
            black_key_height_ratio: 0.62,
            pressed_notes: HashSet::new(),
            dragging_note: None,
            octave_offset: 0, // Center on C4 (MIDI note 60)
        }
    }

    /// Check if a MIDI note is a black key
    fn is_black_key(note: u8) -> bool {
        matches!(note % 12, 1 | 3 | 6 | 8 | 10) // C#, D#, F#, G#, A#
    }

    /// Check if a MIDI note is a white key
    fn is_white_key(note: u8) -> bool {
        !Self::is_black_key(note)
    }

    /// Calculate visible note range and white key width based on pane dimensions
    /// Returns (visible_start_note, visible_end_note, white_key_width, offset_x)
    fn calculate_visible_range(&self, width: f32, height: f32) -> (u8, u8, f32, f32) {
        // Calculate white key width based on height to maintain aspect ratio
        let white_key_width = height / self.white_key_aspect_ratio;

        // Calculate how many white keys can fit in the pane
        let white_keys_fit = (width / white_key_width).ceil() as i32;

        // Keyboard-mapped range is C4 (60) to C5 (72), shifted by octave offset
        // This contains 8 white keys: C, D, E, F, G, A, B, C
        let keyboard_center = 60 + (self.octave_offset as i32 * 12); // C4 + octave shift
        let keyboard_white_keys = 8;

        if white_keys_fit <= keyboard_white_keys {
            // Not enough space to show all keyboard keys, just center what we have
            let visible_start_note = keyboard_center as u8;
            let visible_end_note = (keyboard_center + 12) as u8; // One octave up
            let total_white_key_width = keyboard_white_keys as f32 * white_key_width;
            let offset_x = (width - total_white_key_width) / 2.0;
            return (visible_start_note, visible_end_note, white_key_width, offset_x);
        }

        // Calculate how many extra white keys we have space for
        let extra_white_keys = white_keys_fit - keyboard_white_keys;
        let left_extra = extra_white_keys / 2;
        let right_extra = extra_white_keys - left_extra;

        // Extend left from keyboard center
        let mut start_note = keyboard_center;
        let mut white_count = 0;
        while white_count < left_extra && start_note > 0 {
            start_note -= 1;
            if Self::is_white_key(start_note as u8) {
                white_count += 1;
            }
        }

        // Extend right from keyboard end (C5 = 72)
        let mut end_note = keyboard_center + 12; // C5
        white_count = 0;
        while white_count < right_extra && end_note < 127 {
            end_note += 1;
            if Self::is_white_key(end_note as u8) {
                white_count += 1;
            }
        }

        // No offset - keys start from left edge and fill to the right
        (start_note as u8, end_note as u8, white_key_width, 0.0)
    }

    /// Render the piano keyboard
    fn render_keyboard(&mut self, ui: &mut egui::Ui, rect: egui::Rect, shared: &mut SharedPaneState) {
        // Calculate visible range and key dimensions based on pane size
        let (visible_start, visible_end, white_key_width, offset_x) =
            self.calculate_visible_range(rect.width(), rect.height());

        let white_key_height = rect.height();
        let black_key_width = white_key_width * self.black_key_width_ratio;
        let black_key_height = white_key_height * self.black_key_height_ratio;

        // Count white keys before each note for positioning
        let mut white_key_positions: std::collections::HashMap<u8, f32> = std::collections::HashMap::new();
        let mut white_count = 0;
        for note in visible_start..=visible_end {
            if Self::is_white_key(note) {
                white_key_positions.insert(note, white_count as f32);
                white_count += 1;
            }
        }

        // Draw white keys first (so black keys render on top)
        for note in visible_start..=visible_end {
            if !Self::is_white_key(note) {
                continue;
            }

            let white_pos = white_key_positions[&note];
            let x = rect.min.x + offset_x + (white_pos * white_key_width);
            let key_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.min.y),
                egui::vec2(white_key_width - 1.0, white_key_height),
            );

            // Visual feedback for pressed keys
            let is_pressed = self.pressed_notes.contains(&note);
            let color = if is_pressed {
                egui::Color32::from_rgb(100, 150, 255) // Blue when pressed
            } else {
                egui::Color32::WHITE
            };

            ui.painter().rect_filled(key_rect, 2.0, color);
            ui.painter().rect_stroke(
                key_rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::BLACK),
                egui::StrokeKind::Middle,
            );

            // Handle interaction
            let key_id = ui.id().with(("white_key", note));
            let response = ui.interact(key_rect, key_id, egui::Sense::click_and_drag());

            // Check if pointer is currently over this key (works during drag too)
            let pointer_over_key = ui.input(|i| {
                i.pointer.hover_pos().map_or(false, |pos| key_rect.contains(pos))
            });

            // Mouse down starts note (detect primary button pressed on this key)
            if pointer_over_key && ui.input(|i| i.pointer.primary_pressed()) {
                self.send_note_on(note, 100, shared);
                self.dragging_note = Some(note);
            }

            // Mouse up stops note (detect primary button released)
            if ui.input(|i| i.pointer.primary_released()) {
                if self.dragging_note == Some(note) {
                    self.send_note_off(note, shared);
                    self.dragging_note = None;
                }
            }

            // Dragging over a new key (pointer is down and over a different key)
            if pointer_over_key && ui.input(|i| i.pointer.primary_down()) {
                if self.dragging_note != Some(note) {
                    // Stop previous note
                    if let Some(prev_note) = self.dragging_note {
                        self.send_note_off(prev_note, shared);
                    }
                    // Start new note
                    self.send_note_on(note, 100, shared);
                    self.dragging_note = Some(note);
                }
            }
        }

        // Draw black keys on top
        for note in visible_start..=visible_end {
            if !Self::is_black_key(note) {
                continue;
            }

            // Find the white key immediately before this black key
            let mut white_keys_before = 0;
            for n in visible_start..note {
                if Self::is_white_key(n) {
                    white_keys_before += 1;
                }
            }

            // Position black key at the right edge of the preceding white key
            let x = rect.min.x + offset_x + (white_keys_before as f32 * white_key_width) - (black_key_width / 2.0);
            let key_rect = egui::Rect::from_min_size(
                egui::pos2(x, rect.min.y),
                egui::vec2(black_key_width, black_key_height),
            );

            let is_pressed = self.pressed_notes.contains(&note);
            let color = if is_pressed {
                egui::Color32::from_rgb(50, 100, 200) // Darker blue when pressed
            } else {
                egui::Color32::BLACK
            };

            ui.painter().rect_filled(key_rect, 2.0, color);

            // Handle interaction (same as white keys)
            let key_id = ui.id().with(("black_key", note));
            let response = ui.interact(key_rect, key_id, egui::Sense::click_and_drag());

            // Check if pointer is currently over this key (works during drag too)
            let pointer_over_key = ui.input(|i| {
                i.pointer.hover_pos().map_or(false, |pos| key_rect.contains(pos))
            });

            // Mouse down starts note
            if pointer_over_key && ui.input(|i| i.pointer.primary_pressed()) {
                self.send_note_on(note, 100, shared);
                self.dragging_note = Some(note);
            }

            // Mouse up stops note
            if ui.input(|i| i.pointer.primary_released()) {
                if self.dragging_note == Some(note) {
                    self.send_note_off(note, shared);
                    self.dragging_note = None;
                }
            }

            // Dragging over a new key
            if pointer_over_key && ui.input(|i| i.pointer.primary_down()) {
                if self.dragging_note != Some(note) {
                    if let Some(prev_note) = self.dragging_note {
                        self.send_note_off(prev_note, shared);
                    }
                    self.send_note_on(note, 100, shared);
                    self.dragging_note = Some(note);
                }
            }
        }
    }

    /// Send note-on event to daw-backend
    fn send_note_on(&mut self, note: u8, velocity: u8, shared: &mut SharedPaneState) {
        self.pressed_notes.insert(note);

        // Get active MIDI layer from shared state
        if let Some(active_layer_id) = *shared.active_layer_id {
            // Look up daw-backend track ID from layer ID
            if let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) {
                if let Some(ref mut controller) = shared.audio_controller {
                    controller.send_midi_note_on(track_id, note, velocity);
                }
            }
        }
    }

    /// Send note-off event to daw-backend
    fn send_note_off(&mut self, note: u8, shared: &mut SharedPaneState) {
        self.pressed_notes.remove(&note);

        if let Some(active_layer_id) = *shared.active_layer_id {
            if let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) {
                if let Some(ref mut controller) = shared.audio_controller {
                    controller.send_midi_note_off(track_id, note);
                }
            }
        }
    }
}

impl PaneRenderer for VirtualPianoPane {
    fn render_header(&mut self, ui: &mut egui::Ui, _shared: &mut SharedPaneState) -> bool {
        ui.horizontal(|ui| {
            ui.label("Octave Shift:");
            if ui.button("-").clicked() && self.octave_offset > -3 {
                self.octave_offset -= 1;
            }
            let center_note = 60 + (self.octave_offset as i32 * 12);
            let octave_name = format!("C{}", center_note / 12);
            ui.label(octave_name);
            if ui.button("+").clicked() && self.octave_offset < 3 {
                self.octave_offset += 1;
            }
        });

        true // We rendered a header
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        _path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        // Check if there's an active MIDI layer
        let has_active_midi_layer = if let Some(active_layer_id) = *shared.active_layer_id {
            shared.layer_to_track_map.contains_key(&active_layer_id)
        } else {
            false
        };

        if !has_active_midi_layer {
            // Show message if no active MIDI track
            ui.centered_and_justified(|ui| {
                ui.label("No MIDI track selected. Create a MIDI track to use the virtual piano.");
            });
            return;
        }

        // Render the keyboard
        self.render_keyboard(ui, rect, shared);
    }

    fn name(&self) -> &str {
        "Virtual Piano"
    }
}
