//! Custom text field widget with IME workaround
//!
//! WORKAROUND for IBus Wayland bug (egui issue #7485):
//! https://github.com/emilk/egui/issues/7485
//!
//! IBus on Wayland only delivers one character through normal TextEdit handling.
//! This widget renders a custom text field and handles all input manually.
//!
//! TODO: Remove this workaround once the upstream issue is fixed.

use eframe::egui;

/// Convert egui Key to character for manual text input handling.
/// Uses egui's name()/symbol_or_name() where possible, with shift handling
/// for uppercase letters and shifted symbols (US keyboard layout).
fn key_to_char(key: egui::Key, shift: bool) -> Option<char> {
    let symbol = key.symbol_or_name();

    // If it's a single character, we can use it (with shift handling)
    if symbol.chars().count() == 1 {
        let c = symbol.chars().next().unwrap();

        // Handle letters - apply shift for case
        if c.is_ascii_alphabetic() {
            return Some(if shift { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() });
        }

        // Handle digits with shift -> symbols (US keyboard layout)
        if c.is_ascii_digit() && shift {
            return Some(match c {
                '0' => ')',
                '1' => '!',
                '2' => '@',
                '3' => '#',
                '4' => '$',
                '5' => '%',
                '6' => '^',
                '7' => '&',
                '8' => '*',
                '9' => '(',
                _ => c,
            });
        }

        // Handle punctuation with shift (US keyboard layout)
        if shift {
            return Some(match c {
                '-' | '−' => '_',  // Minus (egui uses special minus char)
                '=' => '+',
                '[' => '{',
                ']' => '}',
                '\\' => '|',
                ';' => ':',
                '\'' => '"',
                ',' => '<',
                '.' => '>',
                '/' => '?',
                '`' => '~',
                _ => c,
            });
        }

        return Some(c);
    }

    // Special case: Space returns "Space" not " "
    if matches!(key, egui::Key::Space) {
        return Some(' ');
    }

    None // Non-printable keys (arrows, function keys, etc.)
}

/// Response from the IME text field widget
pub struct ImeTextFieldResponse {
    /// The egui response for the text field area
    pub response: egui::Response,
    /// Whether the text was changed
    pub changed: bool,
    /// Whether Enter was pressed (for single-line fields)
    pub submitted: bool,
    /// Whether Escape was pressed
    pub cancelled: bool,
}

/// A text field widget that works around IBus Wayland IME issues.
///
/// This is a temporary workaround for egui issue #7485. Use this instead of
/// `egui::TextEdit` when you need text input to work on Wayland with IBus.
pub struct ImeTextField<'a> {
    text: &'a mut String,
    placeholder: Option<&'a str>,
    font_size: f32,
    desired_width: Option<f32>,
    request_focus: bool,
}

impl<'a> ImeTextField<'a> {
    /// Create a new text field widget
    pub fn new(text: &'a mut String) -> Self {
        Self {
            text,
            placeholder: None,
            font_size: 14.0,
            desired_width: None,
            request_focus: false,
        }
    }

    /// Set placeholder text shown when the field is empty and unfocused
    pub fn placeholder(mut self, placeholder: &'a str) -> Self {
        self.placeholder = Some(placeholder);
        self
    }

    /// Set the font size (default: 14.0)
    pub fn font_size(mut self, size: f32) -> Self {
        self.font_size = size;
        self
    }

    /// Set the desired width of the field
    pub fn desired_width(mut self, width: f32) -> Self {
        self.desired_width = Some(width);
        self
    }

    /// Request focus on this field
    pub fn request_focus(mut self) -> Self {
        self.request_focus = true;
        self
    }

    /// Show the text field widget
    pub fn show(self, ui: &mut egui::Ui) -> ImeTextFieldResponse {
        let desired_size = egui::vec2(
            self.desired_width.unwrap_or(ui.available_width()),
            self.font_size + 8.0,
        );

        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
        let id = response.id;

        // Handle click to focus
        if response.clicked() {
            ui.memory_mut(|m| m.request_focus(id));
        }

        // Handle focus request
        if self.request_focus {
            ui.memory_mut(|m| m.request_focus(id));
        }

        let has_focus = ui.memory(|m| m.has_focus(id));

        // Draw the text field background
        let bg_color = if has_focus {
            egui::Color32::from_rgb(50, 50, 55)
        } else {
            egui::Color32::from_rgb(40, 40, 45)
        };
        let stroke = if has_focus {
            egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 150, 255))
        } else {
            egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 60, 65))
        };
        ui.painter().rect(rect, 3.0, bg_color, stroke, egui::StrokeKind::Middle);

        // Draw the text or placeholder
        let text_pos = rect.min + egui::vec2(6.0, (rect.height() - self.font_size) / 2.0);
        if self.text.is_empty() && !has_focus {
            if let Some(placeholder) = self.placeholder {
                ui.painter().text(
                    text_pos,
                    egui::Align2::LEFT_TOP,
                    placeholder,
                    egui::FontId::proportional(self.font_size),
                    egui::Color32::from_gray(100),
                );
            }
        } else {
            ui.painter().text(
                text_pos,
                egui::Align2::LEFT_TOP,
                self.text.as_str(),
                egui::FontId::proportional(self.font_size),
                egui::Color32::from_gray(220),
            );

            // Draw cursor when focused
            if has_focus {
                let text_width = ui.painter().layout_no_wrap(
                    self.text.clone(),
                    egui::FontId::proportional(self.font_size),
                    egui::Color32::WHITE,
                ).rect.width();

                let cursor_x = text_pos.x + text_width + 1.0;
                let blink = (ui.input(|i| i.time) * 2.0).fract() < 0.5;
                if blink {
                    ui.painter().line_segment(
                        [
                            egui::pos2(cursor_x, rect.min.y + 4.0),
                            egui::pos2(cursor_x, rect.max.y - 4.0),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::WHITE),
                    );
                }
                ui.ctx().request_repaint(); // For cursor blinking
            }
        }

        // Handle keyboard input when focused
        let mut changed = false;
        let mut submitted = false;
        let mut cancelled = false;

        if has_focus {
            ui.input(|i| {
                for event in &i.events {
                    if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
                        // Skip if modifier keys are held (except shift)
                        if modifiers.ctrl || modifiers.alt || modifiers.command {
                            continue;
                        }

                        match key {
                            egui::Key::Backspace => {
                                if !self.text.is_empty() {
                                    self.text.pop();
                                    changed = true;
                                }
                            }
                            egui::Key::Enter => {
                                submitted = true;
                            }
                            egui::Key::Escape => {
                                cancelled = true;
                            }
                            _ => {
                                if let Some(c) = key_to_char(*key, modifiers.shift) {
                                    self.text.push(c);
                                    changed = true;
                                }
                            }
                        }
                    }
                }
            });

            // Lose focus on Escape
            if cancelled {
                ui.memory_mut(|m| m.surrender_focus(id));
            }
        }

        ImeTextFieldResponse {
            response,
            changed,
            submitted,
            cancelled,
        }
    }
}
