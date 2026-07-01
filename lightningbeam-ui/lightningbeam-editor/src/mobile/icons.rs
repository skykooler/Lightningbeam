//! Lucide icon font (ISC license — see `assets/fonts/LICENSE-lucide.txt`).
#![allow(dead_code)] // a forward-looking icon palette; not all are wired up yet
//!
//! The font is embedded and registered as a named egui font family ("lucide"). Icons are drawn as
//! text using [`font`] for the `FontId`, with the codepoint constants below (Lucide's private-use
//! area). Run [`install`] once at startup.

use std::sync::Arc;

use eframe::egui;

/// The egui font-family name the Lucide glyphs live under.
pub const FAMILY: &str = "lucide";

/// Embed + register the Lucide icon font. Call once with the egui context at startup.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "lucide".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/fonts/lucide.ttf"
        ))),
    );
    fonts
        .families
        .insert(egui::FontFamily::Name(FAMILY.into()), vec!["lucide".to_owned()]);
    ctx.set_fonts(fonts);
}

/// A `FontId` in the Lucide family at `size`.
pub fn font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(FAMILY.into()))
}

// --- Codepoints (Lucide PUA). Add more as the mobile UI needs them. ---
pub const MAXIMIZE: &str = "\u{e112}";
pub const MINIMIZE: &str = "\u{e11a}";
pub const ARROW_LEFT_RIGHT: &str = "\u{e24a}";
pub const GRIP_HORIZONTAL: &str = "\u{e0ea}";
pub const CHEVRONS_UP: &str = "\u{e074}";
pub const PLAY: &str = "\u{e13c}";
pub const PAUSE: &str = "\u{e12e}";
pub const SETTINGS: &str = "\u{e154}";
pub const SEARCH: &str = "\u{e151}";
pub const PLUS: &str = "\u{e13d}";
pub const X: &str = "\u{e1b2}";
pub const MENU: &str = "\u{e115}";
// Intent-picker icons.
pub const BRUSH: &str = "\u{e1d3}";
pub const FILM: &str = "\u{e0d0}";
pub const MUSIC: &str = "\u{e122}";
pub const MIC: &str = "\u{e118}";
pub const CLAPPERBOARD: &str = "\u{e29b}";
pub const SQUARE_DASHED: &str = "\u{e1cb}";
pub const FOLDER_OPEN: &str = "\u{e247}";
// Tool icons (omnibutton).
pub const MOUSE_POINTER_2: &str = "\u{e1c3}";
pub const PENCIL: &str = "\u{e1f9}";
pub const MOVE: &str = "\u{e121}";
pub const VECTOR_SQUARE: &str = "\u{e67c}";
pub const SQUARE: &str = "\u{e167}";
pub const CIRCLE: &str = "\u{e076}";
pub const MINUS: &str = "\u{e11c}";
pub const HEXAGON: &str = "\u{e0f3}";
pub const PAINT_BUCKET: &str = "\u{e2e6}";
pub const PIPETTE: &str = "\u{e13b}";
pub const TYPE: &str = "\u{e198}";
pub const ERASER: &str = "\u{e28f}";
pub const PEN_TOOL: &str = "\u{e131}";
pub const BLEND: &str = "\u{e59c}";
pub const WAND_SPARKLES: &str = "\u{e357}";
pub const LASSO_SELECT: &str = "\u{e1cf}";
pub const SCISSORS: &str = "\u{e14e}";
// Create-menu icons.
pub const AUDIO_WAVEFORM: &str = "\u{e55b}";
pub const PIANO: &str = "\u{e561}";
pub const FOLDER: &str = "\u{e0d7}";
pub const FILE_PLUS: &str = "\u{e0c9}";
pub const COPY: &str = "\u{e09e}";
pub const LAYERS: &str = "\u{e529}";
