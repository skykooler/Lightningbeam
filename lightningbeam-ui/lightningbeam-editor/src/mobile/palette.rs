//! The mobile UI's semantic color palette, sourced from the shared theme CSS variables so it
//! matches the main UI and responds to light/dark/user themes. Built once per frame from the active
//! [`Theme`](crate::theme::Theme) and passed (by `Copy`) into the mobile render functions, replacing
//! the per-file hardcoded color constants.

use eframe::egui::{Color32, Context};

use crate::theme::Theme;

#[derive(Clone, Copy)]
pub struct Palette {
    /// App/device background behind everything.
    pub bg: Color32,
    /// Panels, cards, bands, sheets.
    pub surface: Color32,
    /// Raised / hovered surface.
    pub surface_alt: Color32,
    /// Band headers, the top bar.
    pub header: Color32,
    /// Borders and dividers.
    pub line: Color32,
    /// Primary text / icons.
    pub text: Color32,
    /// Secondary (dim) text / icons.
    pub text_dim: Color32,
    /// Selection / active accent.
    pub accent: Color32,
    /// Text / icons drawn on top of `accent`.
    pub on_accent: Color32,
    /// Modal backdrop (translucent).
    pub scrim: Color32,
    /// Decorative category accents (intent picker, chips): coral, cyan, amber, pink, violet.
    pub accents: [Color32; 5],
}

impl Palette {
    pub fn from_theme(theme: &Theme, ctx: &Context) -> Self {
        let c = |name: &str, fb: Color32| theme.var(name, ctx).unwrap_or(fb);
        Self {
            bg: c("bg-app", Color32::from_rgb(0x2a, 0x2a, 0x2a)),
            surface: c("bg-panel", Color32::from_rgb(0x22, 0x22, 0x22)),
            surface_alt: c("bg-surface-raised", Color32::from_rgb(0x3f, 0x3f, 0x3f)),
            header: c("bg-header", Color32::from_rgb(0x35, 0x35, 0x35)),
            line: c("border-default", Color32::from_rgb(0x44, 0x44, 0x44)),
            text: c("text-primary", Color32::from_rgb(0xf6, 0xf6, 0xf6)),
            text_dim: c("text-secondary", Color32::from_rgb(0xaa, 0xaa, 0xaa)),
            accent: c("accent", Color32::from_rgb(0x39, 0x6c, 0xd8)),
            on_accent: c("text-on-accent", Color32::WHITE),
            scrim: c("scrim", Color32::from_rgba_unmultiplied(0x10, 0x14, 0x1a, 0xb0)),
            accents: [
                c("accent-coral", Color32::from_rgb(0xe8, 0x82, 0x6b)),
                c("accent-cyan", Color32::from_rgb(0x54, 0xc3, 0xe8)),
                c("accent-amber", Color32::from_rgb(0xf4, 0xa3, 0x40)),
                c("accent-pink", Color32::from_rgb(0xc7, 0x5b, 0x8a)),
                c("accent-violet", Color32::from_rgb(0x8a, 0x6e, 0xc0)),
            ],
        }
    }
}
