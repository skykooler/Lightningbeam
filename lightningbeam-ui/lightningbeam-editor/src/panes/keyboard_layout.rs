//! Shared horizontal keyboard geometry (pitch → x), used by both the Virtual Piano and the mobile
//! portrait "Synthesia" Piano Roll so their columns line up with the keys. It is **width-driven**
//! (key width from the pane width, not its height) and supports a smooth horizontal **pan** (pixels)
//! so the roll and keyboard scroll together and stay aligned.

/// Number of white keys strictly before each pitch-class within its octave.
const WHITES_BEFORE: [i32; 12] = [0, 1, 1, 2, 2, 3, 4, 4, 5, 5, 6, 6];
/// White pitch-classes, indexed by white-key-within-octave.
const WHITE_PC: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
/// Target on-screen white-key width (px); the visible white-key count approximates it.
const TARGET_WHITE_KEY_W: f32 = 40.0;

/// Absolute white-key index of a note, counting white keys from MIDI 0.
fn awi(note: u8) -> i32 {
    (note as i32 / 12) * 7 + WHITES_BEFORE[(note % 12) as usize]
}

/// The white note at a given absolute white-key index.
fn white_note_from_awi(a: i32) -> u8 {
    let oct = a.div_euclid(7);
    let rem = a.rem_euclid(7) as usize;
    (oct * 12 + WHITE_PC[rem]).clamp(0, 127) as u8
}

/// A horizontal piano key layout for a pane width, octave center, and horizontal pan.
#[derive(Clone, Copy)]
pub struct KeyboardLayout {
    pub white_key_width: f32,
    pub black_key_width: f32,
    /// `note_x(white n) == base_x + awi(n) * white_key_width`.
    base_x: f32,
    rect_left: f32,
    rect_width: f32,
}

impl KeyboardLayout {
    pub fn is_black_key(note: u8) -> bool {
        matches!(note % 12, 1 | 3 | 6 | 8 | 10)
    }

    /// Build a layout. `pan_x` shifts everything right (drag-right reveals lower keys); at `pan_x == 0`
    /// the octave-center key sits in the middle of the pane.
    pub fn from_width(origin_x: f32, width: f32, octave: i8, pan_x: f32) -> Self {
        let vis = ((width / TARGET_WHITE_KEY_W).round() as u32).clamp(7, 24) as f32;
        let white_key_width = width / vis;
        let black_key_width = white_key_width * 0.6;
        let center = (60 + octave as i32 * 12).clamp(0, 127) as u8;
        let base_x = origin_x + width / 2.0 - (awi(center) as f32 + 0.5) * white_key_width + pan_x;
        Self { white_key_width, black_key_width, base_x, rect_left: origin_x, rect_width: width }
    }

    /// Snap a pan offset to the nearest whole key (for release-snap).
    pub fn snap_pan(&self, pan_x: f32) -> f32 {
        (pan_x / self.white_key_width).round() * self.white_key_width
    }

    /// Left x of a key's rect (black keys straddle the white boundary).
    pub fn note_x(&self, note: u8) -> f32 {
        let x = self.base_x + awi(note) as f32 * self.white_key_width;
        if Self::is_black_key(note) {
            x - self.black_key_width / 2.0
        } else {
            x
        }
    }

    pub fn note_width(&self, note: u8) -> f32 {
        if Self::is_black_key(note) {
            self.black_key_width
        } else {
            self.white_key_width
        }
    }

    pub fn note_center_x(&self, note: u8) -> f32 {
        self.note_x(note) + self.note_width(note) / 2.0
    }

    /// Leftmost visible white key (with one key of margin).
    pub fn first_visible_white(&self) -> u8 {
        let a = ((self.rect_left - self.base_x) / self.white_key_width).floor() as i32 - 1;
        white_note_from_awi(a.max(0))
    }

    /// Rightmost visible note (with one key of margin).
    pub fn last_visible_note(&self) -> u8 {
        let a = ((self.rect_left + self.rect_width - self.base_x) / self.white_key_width).ceil() as i32 + 1;
        white_note_from_awi(a).min(127)
    }

    pub fn visible_notes(&self) -> std::ops::RangeInclusive<u8> {
        self.first_visible_white()..=self.last_visible_note()
    }

    /// Which key is at screen x (black keys, drawn on top, take precedence).
    pub fn x_to_note(&self, x: f32) -> u8 {
        for note in self.visible_notes() {
            if Self::is_black_key(note) {
                let c = self.note_center_x(note);
                if (x - c).abs() <= self.black_key_width / 2.0 {
                    return note;
                }
            }
        }
        let a = ((x - self.base_x) / self.white_key_width).floor() as i32;
        white_note_from_awi(a.max(0)).min(127)
    }
}
