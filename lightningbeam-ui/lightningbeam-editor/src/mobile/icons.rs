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
pub const ARROW_RIGHT_FROM_LINE: &str = "\u{e458}"; // output port
pub const ARROW_RIGHT_TO_LINE: &str = "\u{e459}"; // input port
pub const GRIP_HORIZONTAL: &str = "\u{e0ea}";
pub const CHEVRONS_UP: &str = "\u{e074}";
pub const PLAY: &str = "\u{e13c}";
pub const PAUSE: &str = "\u{e12e}";
pub const REPEAT: &str = "\u{e146}"; // cycle / loop region toggle
pub const TRASH: &str = "\u{e18d}"; // delete a take
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
pub const ELLIPSIS: &str = "\u{e0b6}";
// Undo / redo (stage header).
pub const UNDO_2: &str = "\u{e2a1}";
pub const REDO_2: &str = "\u{e2a0}";
// Tool cursors (see custom_cursor.rs).
pub const STAMP: &str = "\u{e3bb}";
pub const SPRAY_CAN: &str = "\u{e495}";
pub const BANDAGE: &str = "\u{e61d}";
pub const DROPLET: &str = "\u{e0b4}";
pub const TEXT_CURSOR: &str = "\u{e264}";
pub const CROSSHAIR: &str = "\u{e0ac}";
pub const POINTER: &str = "\u{e1e8}";
pub const CONTRAST: &str = "\u{e09d}";
pub const WIND: &str = "\u{e1b0}";
pub const SUN_MOON: &str = "\u{e2b2}";
pub const SPLINE: &str = "\u{e38b}";
pub const DROPLETS: &str = "\u{e0b5}";
pub const CIRCLE_DASHED: &str = "\u{e4b0}";
pub const PALETTE: &str = "\u{e1dd}";
pub const SHAPES: &str = "\u{e4b3}";
pub const SCALING: &str = "\u{e2ec}";
pub const FRAME: &str = "\u{e291}";
// Timeline layer-row toggles.
pub const VOLUME_2: &str = "\u{e1ab}"; // unmuted
pub const VOLUME_X: &str = "\u{e1ac}"; // muted
pub const HEADPHONES: &str = "\u{e0f1}"; // solo
pub const LOCK: &str = "\u{e10b}";
pub const LOCK_OPEN: &str = "\u{e10c}";
pub const EYE: &str = "\u{e0ba}";
pub const EYE_OFF: &str = "\u{e0bb}";
pub const VIDEO: &str = "\u{e1a5}"; // camera enabled
pub const VIDEO_OFF: &str = "\u{e1a6}"; // camera disabled

/// Lucide glyph for tools that have no bundled SVG icon. `None` means the tool has a real SVG
/// icon in `src/assets/` and the caller should use that instead.
pub fn tool_glyph(tool: lightningbeam_core::tool::Tool) -> Option<&'static str> {
    use lightningbeam_core::tool::Tool;
    Some(match tool {
        Tool::Pencil => PENCIL,
        Tool::Pen => PEN_TOOL,
        Tool::Airbrush => SPRAY_CAN,
        Tool::CloneStamp => STAMP,
        Tool::HealingBrush => BANDAGE,
        Tool::PatternStamp => SHAPES,
        Tool::DodgeBurn => SUN_MOON,
        Tool::Sponge => DROPLETS,
        Tool::BlurSharpen => CONTRAST,
        Tool::Gradient => BLEND,
        Tool::CustomShape => HEXAGON,
        Tool::SelectEllipse => CIRCLE_DASHED,
        Tool::MagicWand => WAND_SPARKLES,
        Tool::QuickSelect => BRUSH,
        Tool::Warp => SCALING,
        Tool::Liquify => DROPLET,
        Tool::SelectLasso => LASSO_SELECT,
        _ => return None,
    })
}
