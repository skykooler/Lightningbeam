/// Virtual Piano Pane - On-screen piano keyboard for live MIDI input
///
/// Provides a clickable/draggable piano keyboard that sends MIDI note events
/// to the currently active MIDI track via daw-backend.

use super::{NodePath, PaneRenderer, SharedPaneState};
use eframe::egui;
use std::collections::{HashMap, HashSet};

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
    /// Tracks which computer keys are held and which MIDI notes they're playing
    active_key_presses: HashMap<String, u8>,
    /// Current velocity for keyboard input (adjustable with C/V keys)
    keyboard_velocity: u8,
    /// Base keyboard mapping (key string -> MIDI note, before octave offset)
    keyboard_map: HashMap<String, u8>,
    /// Reverse mapping for displaying labels (MIDI note -> key label)
    note_to_key_map: HashMap<u8, String>,
    /// Sustain pedal state (Tab key toggles)
    sustain_active: bool,
    /// Notes being held by sustain pedal (not by active key/mouse press)
    sustained_notes: HashSet<u8>,
}

impl Default for VirtualPianoPane {
    fn default() -> Self {
        Self::new()
    }
}

impl VirtualPianoPane {
    pub fn new() -> Self {
        // Create keyboard mapping (C4-F5 range, MIDI notes 60-77)
        let mut keyboard_map = HashMap::new();
        keyboard_map.insert("a".to_string(), 60);  // C4
        keyboard_map.insert("w".to_string(), 61);  // C#4
        keyboard_map.insert("s".to_string(), 62);  // D4
        keyboard_map.insert("e".to_string(), 63);  // D#4
        keyboard_map.insert("d".to_string(), 64);  // E4
        keyboard_map.insert("f".to_string(), 65);  // F4
        keyboard_map.insert("t".to_string(), 66);  // F#4
        keyboard_map.insert("g".to_string(), 67);  // G4
        keyboard_map.insert("y".to_string(), 68);  // G#4
        keyboard_map.insert("h".to_string(), 69);  // A4
        keyboard_map.insert("u".to_string(), 70);  // A#4
        keyboard_map.insert("j".to_string(), 71);  // B4
        keyboard_map.insert("k".to_string(), 72);  // C5
        keyboard_map.insert("o".to_string(), 73);  // C#5
        keyboard_map.insert("l".to_string(), 74);  // D5
        keyboard_map.insert("p".to_string(), 75);  // D#5
        keyboard_map.insert(";".to_string(), 76);  // E5
        keyboard_map.insert("'".to_string(), 77);  // F5

        // Create reverse mapping for labels (note -> uppercase key)
        let mut note_to_key_map = HashMap::new();
        for (key, note) in &keyboard_map {
            note_to_key_map.insert(*note, key.to_uppercase());
        }

        Self {
            white_key_aspect_ratio: 6.0,
            black_key_width_ratio: 0.6,
            black_key_height_ratio: 0.62,
            pressed_notes: HashSet::new(),
            dragging_note: None,
            octave_offset: 0, // Center on C4 (MIDI note 60)
            active_key_presses: HashMap::new(),
            keyboard_velocity: 100, // Default MIDI velocity
            keyboard_map,
            note_to_key_map,
            sustain_active: false,
            sustained_notes: HashSet::new(),
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

        // Check black keys for interaction first (since they render on top and overlap white keys)
        // This prevents white keys underneath from also receiving the click
        let mut black_key_interacted = false;
        let pointer_pos = ui.input(|i| i.pointer.hover_pos());

        if let Some(pos) = pointer_pos {
            for note in visible_start..=visible_end {
                if !Self::is_black_key(note) {
                    continue;
                }

                // Calculate black key rect
                let mut white_keys_before = 0;
                for n in visible_start..note {
                    if Self::is_white_key(n) {
                        white_keys_before += 1;
                    }
                }

                let x = rect.min.x + offset_x + (white_keys_before as f32 * white_key_width) - (black_key_width / 2.0);
                let key_rect = egui::Rect::from_min_size(
                    egui::pos2(x, rect.min.y),
                    egui::vec2(black_key_width, black_key_height),
                );

                if key_rect.contains(pos) {
                    black_key_interacted = true;
                    break;
                }
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

            // Handle interaction (skip if a black key is being interacted with)
            let key_id = ui.id().with(("white_key", note));
            let response = ui.interact(key_rect, key_id, egui::Sense::click_and_drag());

            // Visual feedback for pressed keys (check both pressed_notes and current pointer state)
            let pointer_over_key = ui.input(|i| {
                i.pointer.hover_pos().map_or(false, |pos| key_rect.contains(pos))
            });
            let pointer_down = ui.input(|i| i.pointer.primary_down());
            let is_pressed = self.pressed_notes.contains(&note) ||
                             (!black_key_interacted && pointer_over_key && pointer_down);
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

            if !black_key_interacted {
                // Mouse down starts note (detect primary button pressed on this key)
                if pointer_over_key && ui.input(|i| i.pointer.primary_pressed()) {
                    // Calculate velocity based on mouse Y position
                    let mouse_y = ui.input(|i| i.pointer.hover_pos()).unwrap().y;
                    let velocity = self.calculate_velocity_from_mouse_y(mouse_y, key_rect);

                    self.send_note_on(note, velocity, shared);
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
                if pointer_over_key && pointer_down {
                    if self.dragging_note != Some(note) {
                        // Stop previous note
                        if let Some(prev_note) = self.dragging_note {
                            self.send_note_off(prev_note, shared);
                        }
                        // Start new note with velocity from mouse position
                        let mouse_y = ui.input(|i| i.pointer.hover_pos()).unwrap().y;
                        let velocity = self.calculate_velocity_from_mouse_y(mouse_y, key_rect);

                        self.send_note_on(note, velocity, shared);
                        self.dragging_note = Some(note);
                    }
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

            // Handle interaction (same as white keys)
            let key_id = ui.id().with(("black_key", note));
            let response = ui.interact(key_rect, key_id, egui::Sense::click_and_drag());

            // Visual feedback for pressed keys (check both pressed_notes and current pointer state)
            let pointer_over_key = ui.input(|i| {
                i.pointer.hover_pos().map_or(false, |pos| key_rect.contains(pos))
            });
            let pointer_down = ui.input(|i| i.pointer.primary_down());
            let is_pressed = self.pressed_notes.contains(&note) ||
                             (pointer_over_key && pointer_down);
            let color = if is_pressed {
                egui::Color32::from_rgb(50, 100, 200) // Darker blue when pressed
            } else {
                egui::Color32::BLACK
            };

            ui.painter().rect_filled(key_rect, 2.0, color);

            // Mouse down starts note
            if pointer_over_key && ui.input(|i| i.pointer.primary_pressed()) {
                // Calculate velocity based on mouse Y position
                let mouse_y = ui.input(|i| i.pointer.hover_pos()).unwrap().y;
                let velocity = self.calculate_velocity_from_mouse_y(mouse_y, key_rect);

                self.send_note_on(note, velocity, shared);
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
            if pointer_over_key && pointer_down {
                if self.dragging_note != Some(note) {
                    if let Some(prev_note) = self.dragging_note {
                        self.send_note_off(prev_note, shared);
                    }
                    // Start new note with velocity from mouse position
                    let mouse_y = ui.input(|i| i.pointer.hover_pos()).unwrap().y;
                    let velocity = self.calculate_velocity_from_mouse_y(mouse_y, key_rect);

                    self.send_note_on(note, velocity, shared);
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
                if let Some(controller_arc) = shared.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.send_midi_note_on(track_id, note, velocity);
                }
            }
        }
    }

    /// Send note-off event to daw-backend (or add to sustain if active)
    fn send_note_off(&mut self, note: u8, shared: &mut SharedPaneState) {
        // If sustain is active, move note to sustained set instead of releasing
        if self.sustain_active {
            self.sustained_notes.insert(note);
            // Keep note in pressed_notes for visual feedback
            return;
        }

        // Normal release: remove from all note sets
        self.pressed_notes.remove(&note);
        self.sustained_notes.remove(&note);

        if let Some(active_layer_id) = *shared.active_layer_id {
            if let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) {
                if let Some(controller_arc) = shared.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.send_midi_note_off(track_id, note);
                }
            }
        }
    }

    /// Release sustain pedal - stop all sustained notes that aren't currently being held
    fn release_sustain(&mut self, shared: &mut SharedPaneState) {
        self.sustain_active = false;

        // Collect currently active notes (keyboard + mouse)
        let mut currently_playing = HashSet::new();

        // Add notes from keyboard
        for &note in self.active_key_presses.values() {
            currently_playing.insert(note);
        }

        // Add note from mouse drag
        if let Some(note) = self.dragging_note {
            currently_playing.insert(note);
        }

        // Release sustained notes that aren't currently being played
        let notes_to_release: Vec<u8> = self.sustained_notes
            .iter()
            .filter(|&&note| !currently_playing.contains(&note))
            .copied()
            .collect();

        for note in notes_to_release {
            self.send_note_off(note, shared);
        }

        self.sustained_notes.clear();
    }

    /// Calculate MIDI velocity based on mouse Y position within key
    ///
    /// - Top of key: velocity 1
    /// - Top 75% of key: Linear scaling from velocity 1 to 127
    /// - Bottom 25% of key: Full velocity (127)
    ///
    /// # Arguments
    /// * `mouse_y` - Y coordinate of mouse cursor
    /// * `key_rect` - Rectangle bounds of the key
    ///
    /// # Returns
    /// MIDI velocity value clamped to range [1, 127]
    fn calculate_velocity_from_mouse_y(&self, mouse_y: f32, key_rect: egui::Rect) -> u8 {
        // Calculate relative position (0.0 at top, 1.0 at bottom)
        let key_height = key_rect.height();
        let relative_y = (mouse_y - key_rect.min.y) / key_height;
        let relative_y = relative_y.clamp(0.0, 1.0);

        // Bottom 25% of key = full velocity
        if relative_y >= 0.75 {
            return 127;
        }

        // Top 75% = linear scale from 1 to 127
        let velocity = 1.0 + (relative_y / 0.75) * 126.0;
        velocity.round().clamp(1.0, 127.0) as u8
    }

    /// Process keyboard input for virtual piano
    /// Returns true if the event was consumed
    fn handle_keyboard_input(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        // Check if we have an active MIDI layer - don't process input if not
        let has_active_midi_layer = if let Some(active_layer_id) = *shared.active_layer_id {
            shared.layer_to_track_map.contains_key(&active_layer_id)
        } else {
            false
        };

        if !has_active_midi_layer {
            return false;
        }

        let mut consumed = false;

        ui.input(|i| {
            // Handle octave shift keys (Z/X)
            if i.key_pressed(egui::Key::Z) {
                if self.octave_offset > -2 {
                    self.octave_offset -= 1;
                    consumed = true;
                }
            }
            if i.key_pressed(egui::Key::X) {
                if self.octave_offset < 2 {
                    self.octave_offset += 1;
                    consumed = true;
                }
            }

            // Handle velocity adjustment (C/V)
            if i.key_pressed(egui::Key::C) {
                self.keyboard_velocity = self.keyboard_velocity.saturating_sub(10).max(1);
                consumed = true;
            }
            if i.key_pressed(egui::Key::V) {
                self.keyboard_velocity = self.keyboard_velocity.saturating_add(10).min(127);
                consumed = true;
            }

            // Handle sustain pedal (Tab key)
            if i.key_pressed(egui::Key::Tab) {
                self.sustain_active = true;
                consumed = true;
            }
            if i.key_released(egui::Key::Tab) {
                self.release_sustain(shared);
                consumed = true;
            }

            // Process raw events for piano keys (need to track press/release separately)
            for event in &i.events {
                if let egui::Event::Key { key, pressed, repeat, .. } = event {
                    if *repeat {
                        continue; // Ignore key repeats
                    }

                    // Convert egui::Key to string representation
                    let key_str = match key {
                        egui::Key::A => "a",
                        egui::Key::S => "s",
                        egui::Key::D => "d",
                        egui::Key::F => "f",
                        egui::Key::G => "g",
                        egui::Key::H => "h",
                        egui::Key::J => "j",
                        egui::Key::K => "k",
                        egui::Key::L => "l",
                        egui::Key::W => "w",
                        egui::Key::E => "e",
                        egui::Key::T => "t",
                        egui::Key::Y => "y",
                        egui::Key::U => "u",
                        egui::Key::O => "o",
                        egui::Key::P => "p",
                        egui::Key::Semicolon => ";",
                        egui::Key::Quote => "'",
                        _ => continue,
                    };

                    if let Some(&base_note) = self.keyboard_map.get(key_str) {
                        if *pressed {
                            // Key down - start note
                            if !self.active_key_presses.contains_key(key_str) {
                                let note = (base_note as i32 + self.octave_offset as i32 * 12)
                                    .clamp(0, 127) as u8;
                                self.active_key_presses.insert(key_str.to_string(), note);
                                self.send_note_on(note, self.keyboard_velocity, shared);
                                consumed = true;
                            }
                        } else {
                            // Key up - stop note
                            if let Some(note) = self.active_key_presses.remove(key_str) {
                                self.send_note_off(note, shared);
                                consumed = true;
                            }
                        }
                    }
                }
            }
        });

        consumed
    }

    /// Release all keyboard-held notes (call when losing focus or switching tracks)
    fn release_all_keyboard_notes(&mut self, shared: &mut SharedPaneState) {
        let notes_to_release: Vec<u8> = self.active_key_presses.values().copied().collect();
        for note in notes_to_release {
            // Force release, bypassing sustain
            self.pressed_notes.remove(&note);
            if let Some(active_layer_id) = *shared.active_layer_id {
                if let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) {
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.send_midi_note_off(track_id, note);
                    }
                }
            }
        }
        self.active_key_presses.clear();

        // Also release all sustained notes
        for note in &self.sustained_notes {
            self.pressed_notes.remove(note);
            if let Some(active_layer_id) = *shared.active_layer_id {
                if let Some(&track_id) = shared.layer_to_track_map.get(&active_layer_id) {
                    if let Some(controller_arc) = shared.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.send_midi_note_off(track_id, *note);
                    }
                }
            }
        }
        self.sustained_notes.clear();
        self.sustain_active = false;
    }

    /// Render keyboard letter labels on piano keys
    fn render_key_labels(
        &self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        visible_start: u8,
        visible_end: u8,
        white_key_width: f32,
        offset_x: f32,
    ) {
        let white_key_height = rect.height();
        let black_key_width = white_key_width * self.black_key_width_ratio;
        let black_key_height = white_key_height * self.black_key_height_ratio;

        // Render labels on white keys
        for note in visible_start..=visible_end {
            if !Self::is_white_key(note) {
                continue;
            }

            // Calculate base note (subtract octave offset to get unmapped note)
            let base_note = (note as i32 - self.octave_offset as i32 * 12).clamp(0, 127) as u8;

            // Check if this note has a keyboard mapping
            if let Some(label) = self.note_to_key_map.get(&base_note) {
                // Count white keys before this note for positioning
                let mut white_keys_before = 0;
                for n in visible_start..note {
                    if Self::is_white_key(n) {
                        white_keys_before += 1;
                    }
                }

                let x = rect.min.x + offset_x + (white_keys_before as f32 * white_key_width);
                let label_pos = egui::pos2(
                    x + white_key_width / 2.0,
                    rect.min.y + rect.height() - 30.0,
                );

                // Check if key is currently pressed
                let is_pressed = self.pressed_notes.contains(&note);
                let color = if is_pressed {
                    egui::Color32::BLACK
                } else {
                    egui::Color32::from_gray(51) // #333333
                };

                ui.painter().text(
                    label_pos,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(16.0),
                    color,
                );
            }
        }

        // Render labels on black keys
        for note in visible_start..=visible_end {
            if !Self::is_black_key(note) {
                continue;
            }

            let base_note = (note as i32 - self.octave_offset as i32 * 12).clamp(0, 127) as u8;

            if let Some(label) = self.note_to_key_map.get(&base_note) {
                // Count white keys before this note for positioning
                let mut white_keys_before = 0;
                for n in visible_start..note {
                    if Self::is_white_key(n) {
                        white_keys_before += 1;
                    }
                }

                let x = rect.min.x + offset_x + (white_keys_before as f32 * white_key_width)
                    - (black_key_width / 2.0);
                let label_pos = egui::pos2(
                    x + black_key_width / 2.0,
                    rect.min.y + black_key_height - 20.0,
                );

                let is_pressed = self.pressed_notes.contains(&note);
                let color = if is_pressed {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_rgba_premultiplied(255, 255, 255, 178) // rgba(255,255,255,0.7)
                };

                ui.painter().text(
                    label_pos,
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(14.0),
                    color,
                );
            }
        }
    }
}

impl PaneRenderer for VirtualPianoPane {
    fn render_header(&mut self, ui: &mut egui::Ui, _shared: &mut SharedPaneState) -> bool {
        ui.horizontal(|ui| {
            ui.label("Octave Shift:");
            if ui.button("-").clicked() && self.octave_offset > -2 {
                self.octave_offset -= 1;
            }
            let center_note = 60 + (self.octave_offset as i32 * 12);
            let octave_name = format!("C{}", center_note / 12);
            ui.label(octave_name);
            if ui.button("+").clicked() && self.octave_offset < 2 {
                self.octave_offset += 1;
            }

            ui.separator();

            ui.label("Velocity:");
            if ui.button("-").clicked() {
                self.keyboard_velocity = self.keyboard_velocity.saturating_sub(10).max(1);
            }
            ui.label(format!("{}", self.keyboard_velocity));
            if ui.button("+").clicked() {
                self.keyboard_velocity = self.keyboard_velocity.saturating_add(10).min(127);
            }

            ui.separator();

            // Sustain pedal indicator
            ui.label("Sustain:");
            let sustain_text = if self.sustain_active {
                egui::RichText::new("ON").color(egui::Color32::from_rgb(100, 200, 100))
            } else {
                egui::RichText::new("OFF").color(egui::Color32::GRAY)
            };
            ui.label(sustain_text);

            if !self.sustained_notes.is_empty() {
                ui.label(format!("({} notes)", self.sustained_notes.len()));
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
            // Release any held notes before showing error message
            if !self.active_key_presses.is_empty() {
                self.release_all_keyboard_notes(shared);
            }

            // Show message if no active MIDI track
            ui.centered_and_justified(|ui| {
                ui.label("No MIDI track selected. Create a MIDI track to use the virtual piano.");
            });
            return;
        }

        // Request keyboard focus to prevent tool shortcuts from firing
        // This sets wants_keyboard_input() to true
        let piano_id = ui.id().with("virtual_piano_keyboard");
        ui.memory_mut(|m| m.request_focus(piano_id));

        // Handle keyboard input FIRST
        self.handle_keyboard_input(ui, shared);

        // Calculate visible range (needed for both rendering and labels)
        let (visible_start, visible_end, white_key_width, offset_x) =
            self.calculate_visible_range(rect.width(), rect.height());

        // Render the keyboard
        self.render_keyboard(ui, rect, shared);

        // Render keyboard labels on top
        self.render_key_labels(ui, rect, visible_start, visible_end, white_key_width, offset_x);
    }

    fn name(&self) -> &str {
        "Virtual Piano"
    }
}
