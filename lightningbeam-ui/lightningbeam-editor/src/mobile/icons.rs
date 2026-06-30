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
