//! Custom cursor system
//!
//! Tool cursors are drawn from the bundled Lucide icon font. A tool falls into one of three
//! kinds, because "draw the tool's icon at the pointer" only works for icons that actually have
//! a point:
//!
//! * [`CursorKind::System`] — the OS cursor already says it better than we can (the arrow for
//!   Select, the move cross for Transform). We use the real OS cursor.
//! * [`CursorKind::Precise`] — the glyph has an unambiguous tip (pencil, brush, pipette), so it
//!   *is* the cursor: the tip lands exactly on the click point.
//! * [`CursorKind::Badge`] — the glyph has no focus point (stamp, bandage, spray can). Using it
//!   alone would leave you guessing where you're clicking, so we pair it with a crosshair: the
//!   crosshair marks the click point and the glyph hangs off its bottom-right.
//!
//! Rather than *painting* the cursor into the egui scene, we rasterize it and hand it to the
//! windowing system as a real OS cursor (`egui::CursorImage` → winit `CustomCursor`). A painted
//! cursor is composited with our frame and therefore always trails the pointer by a frame or
//! more; an OS cursor is composited by the window system and doesn't lag at all.
//!
//! The glyphs are rasterized out of egui's own font atlas using the same placement maths as
//! `epaint`'s text tessellator, so a glyph lands exactly where `painter.text` would have put it.

use eframe::egui;
use lightningbeam_core::tool::Tool;
use std::collections::HashMap;
use std::sync::Arc;

use crate::mobile::icons;

/// What kind of cursor a tool gets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CursorKind {
    /// Use a native OS cursor; build nothing ourselves.
    System(egui::CursorIcon),
    /// The glyph is the cursor. `hotspot` is the click point in Lucide's 24×24 icon grid.
    Precise {
        glyph: &'static str,
        hotspot: egui::Vec2,
    },
    /// A crosshair marks the click point; the glyph sits to its bottom-right.
    Badge { glyph: &'static str },
}

/// Hotspot for a glyph whose tip is at the bottom-left (pencil, brush, pen, pipette).
const TIP_BOTTOM_LEFT: egui::Vec2 = egui::vec2(3.0, 21.0);
/// Hotspot for a glyph centred on the click point.
const TIP_CENTER: egui::Vec2 = egui::vec2(12.0, 12.0);

/// Which cursor a stage tool uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomCursor {
    Tool(Tool),
    /// Timeline: dragging the end of the loop region.
    LoopExtend,
}

impl CustomCursor {
    pub fn from_tool(tool: Tool) -> Self {
        CustomCursor::Tool(tool)
    }

    pub fn kind(&self) -> CursorKind {
        use egui::CursorIcon as Sys;

        let tool = match self {
            CustomCursor::LoopExtend => {
                return CursorKind::Precise {
                    glyph: icons::REPEAT,
                    hotspot: TIP_CENTER,
                }
            }
            CustomCursor::Tool(t) => *t,
        };

        let precise = |glyph, hotspot| CursorKind::Precise { glyph, hotspot };
        let badge = |glyph| CursorKind::Badge { glyph };

        match tool {
            // The OS knows these better than we do.
            Tool::Select => CursorKind::System(Sys::Default),
            Tool::Transform => CursorKind::System(Sys::Move),

            // Glyphs with a real tip: the icon is the cursor.
            Tool::Draw => precise(icons::BRUSH, TIP_BOTTOM_LEFT),
            Tool::Pencil => precise(icons::PENCIL, TIP_BOTTOM_LEFT),
            Tool::Pen => precise(icons::PEN_TOOL, TIP_BOTTOM_LEFT),
            Tool::Eyedropper => precise(icons::PIPETTE, TIP_BOTTOM_LEFT),
            Tool::PaintBucket => precise(icons::PAINT_BUCKET, TIP_BOTTOM_LEFT),
            Tool::Text => precise(icons::TEXT_CURSOR, TIP_CENTER),

            // Shape tools: crosshair for precision, glyph to say which shape.
            Tool::Rectangle => badge(icons::SQUARE),
            Tool::Ellipse => badge(icons::CIRCLE),
            Tool::Line => badge(icons::MINUS),
            Tool::Polygon => badge(icons::HEXAGON),
            Tool::CustomShape => badge(icons::SHAPES),

            // Selection tools.
            Tool::RegionSelect => badge(icons::SQUARE_DASHED),
            Tool::SelectEllipse => badge(icons::CIRCLE_DASHED),
            Tool::SelectLasso => badge(icons::LASSO_SELECT),
            Tool::MagicWand => badge(icons::WAND_SPARKLES),
            Tool::QuickSelect => badge(icons::BRUSH),
            Tool::Split => badge(icons::SCISSORS),

            // Raster tools whose icons have no focus point. These also draw a brush-size ring on
            // the stage, which is the real precision cue.
            Tool::Erase => badge(icons::ERASER),
            Tool::Airbrush => badge(icons::SPRAY_CAN),
            Tool::Smudge => badge(icons::POINTER),
            Tool::CloneStamp => badge(icons::STAMP),
            Tool::PatternStamp => badge(icons::SHAPES),
            Tool::HealingBrush => badge(icons::BANDAGE),
            Tool::DodgeBurn => badge(icons::SUN_MOON),
            Tool::Sponge => badge(icons::DROPLETS),
            Tool::BlurSharpen => badge(icons::CONTRAST),
            Tool::Gradient => badge(icons::BLEND),
            Tool::Warp => badge(icons::SCALING),
            Tool::Liquify => badge(icons::DROPLET),

            // Vertex editing: crosshair to place the point, glyph to say we're in bezier mode.
            Tool::BezierEdit => badge(icons::SPLINE),
        }
    }
}

// ---------------------------------------------------------------------------
// Geometry (logical points; scaled by pixels_per_point when rasterized)
// ---------------------------------------------------------------------------

/// Rendered size of a cursor glyph.
const GLYPH_SIZE: f32 = 18.0;
/// Lucide icons are authored on a 24×24 grid; hotspots are given in those units.
const ICON_GRID: f32 = 24.0;
/// Where a badge glyph's top-left sits relative to the crosshair centre.
const BADGE_OFFSET: egui::Vec2 = egui::vec2(6.0, 6.0);
/// Half-length of a crosshair arm.
const CROSSHAIR_ARM: f32 = 7.0;
/// Gap between the crosshair centre and the start of each arm, so the click point stays visible.
const CROSSHAIR_GAP: f32 = 2.0;
/// Margin around the artwork, leaving room for the outline.
const MARGIN: f32 = 1.5;

// ---------------------------------------------------------------------------
// Rasterization
// ---------------------------------------------------------------------------

/// Cache of rasterized cursor images, keyed by cursor and DPI scale.
#[derive(Default)]
pub struct CursorCache {
    images: HashMap<(CustomCursor, u32), Option<egui::CursorImage>>,
    next_id: u64,
}

impl CursorCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn get_or_build(
        &mut self,
        ctx: &egui::Context,
        cursor: CustomCursor,
        ppp: f32,
    ) -> Option<egui::CursorImage> {
        let key = (cursor, ppp.to_bits());
        if let Some(cached) = self.images.get(&key) {
            return cached.clone();
        }

        let id = self.next_id;
        self.next_id += 1;

        let built = build_cursor_image(ctx, cursor.kind(), ppp, id);
        self.images.insert(key, built.clone());
        built
    }
}

/// An alpha coverage mask being composed, in physical pixels.
struct Mask {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Mask {
    fn new(w: usize, h: usize) -> Self {
        Self { w, h, a: vec![0.0; w * h] }
    }

    fn add(&mut self, x: usize, y: usize, coverage: f32) {
        if x < self.w && y < self.h {
            let p = &mut self.a[y * self.w + x];
            *p = (*p + coverage).min(1.0);
        }
    }

    fn get(&self, x: isize, y: isize) -> f32 {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            0.0
        } else {
            self.a[y as usize * self.w + x as usize]
        }
    }

    /// Fill an axis-aligned rect (physical px, may be fractional) with full coverage.
    fn fill_rect(&mut self, rect: egui::Rect) {
        let x0 = rect.min.x.floor().max(0.0) as usize;
        let y0 = rect.min.y.floor().max(0.0) as usize;
        let x1 = (rect.max.x.ceil() as usize).min(self.w);
        let y1 = (rect.max.y.ceil() as usize).min(self.h);
        for y in y0..y1 {
            for x in x0..x1 {
                self.add(x, y, 1.0);
            }
        }
    }
}

/// Rasterize a cursor into a premultiplied-RGBA image: black artwork with a white outline, so it
/// reads against both light and dark backgrounds.
fn build_cursor_image(
    ctx: &egui::Context,
    kind: CursorKind,
    ppp: f32,
    id: u64,
) -> Option<egui::CursorImage> {
    let (glyph, glyph_origin, hotspot, crosshair) = match kind {
        CursorKind::System(_) => return None,
        CursorKind::Precise { glyph, hotspot } => {
            let scale = GLYPH_SIZE / ICON_GRID;
            (
                glyph,
                egui::vec2(MARGIN, MARGIN),
                egui::pos2(MARGIN, MARGIN) + hotspot * scale,
                false,
            )
        }
        CursorKind::Badge { glyph } => {
            // The crosshair centre is the click point; place it a margin + arm in from the corner.
            let centre = egui::pos2(MARGIN + CROSSHAIR_ARM, MARGIN + CROSSHAIR_ARM);
            (glyph, centre.to_vec2() + BADGE_OFFSET, centre, true)
        }
    };

    // Lay the glyph out exactly as `painter.text(.., Align2::LEFT_TOP, ..)` would, so we inherit
    // epaint's placement rather than re-deriving it from font metrics.
    let galley = ctx.fonts_mut(|f| {
        f.layout_no_wrap(
            glyph.to_owned(),
            icons::font(GLYPH_SIZE),
            egui::Color32::WHITE,
        )
    });

    // Canvas must cover the glyph, and the crosshair too when there is one.
    let mut content = egui::Rect::from_min_size(glyph_origin.to_pos2(), galley.size());
    if crosshair {
        content = content.union(egui::Rect::from_center_size(
            hotspot,
            egui::Vec2::splat(2.0 * CROSSHAIR_ARM),
        ));
    }
    let canvas = content.expand(MARGIN);

    let w = (canvas.width() * ppp).ceil() as usize;
    let h = (canvas.height() * ppp).ceil() as usize;
    if w == 0 || h == 0 || w > 2048 || h > 2048 {
        return None; // winit caps cursors at 2048px.
    }

    let mut mask = Mask::new(w, h);
    // Everything is positioned relative to the canvas origin.
    let to_px = |p: egui::Pos2| ((p - canvas.min.to_vec2()).to_vec2() * ppp).to_pos2();

    // --- Crosshair ---
    if crosshair {
        let c = to_px(hotspot);
        let thickness = ppp.max(1.0); // one physical pixel, at least
        let arm = CROSSHAIR_ARM * ppp;
        let gap = CROSSHAIR_GAP * ppp;
        // Four arms, leaving a gap at the centre so the exact click point stays visible.
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)] {
            let from = egui::pos2(c.x + dx * gap, c.y + dy * gap);
            let to = egui::pos2(c.x + dx * arm, c.y + dy * arm);
            let rect = egui::Rect::from_two_pos(from, to).expand2(if dx == 0.0 {
                egui::vec2(thickness / 2.0, 0.0)
            } else {
                egui::vec2(0.0, thickness / 2.0)
            });
            mask.fill_rect(rect);
        }
    }

    // --- Glyph, copied out of egui's font atlas ---
    let atlas = ctx.fonts_mut(|f| f.image());
    let atlas_w = atlas.size[0];
    for row in &galley.rows {
        for g in &row.glyphs {
            let uv = g.uv_rect;
            if uv.is_nothing() {
                continue;
            }
            // Same maths as epaint's text tessellator: the glyph's top-left in points is
            // `glyph.pos + uv_rect.offset`, relative to the galley's top-left.
            let left_top = to_px((glyph_origin + (g.pos + uv.offset).to_vec2()).to_pos2());
            let src_w = (uv.max[0] - uv.min[0]) as usize;
            let src_h = (uv.max[1] - uv.min[1]) as usize;

            for sy in 0..src_h {
                for sx in 0..src_w {
                    let src_i = (uv.min[1] as usize + sy) * atlas_w + (uv.min[0] as usize + sx);
                    let Some(px) = atlas.pixels.get(src_i) else { continue };
                    let coverage = px.a() as f32 / 255.0;
                    if coverage <= 0.0 {
                        continue;
                    }
                    let dx = (left_top.x.round() as isize) + sx as isize;
                    let dy = (left_top.y.round() as isize) + sy as isize;
                    if dx >= 0 && dy >= 0 {
                        mask.add(dx as usize, dy as usize, coverage);
                    }
                }
            }
        }
    }

    // --- Compose: black fill over a white outline (dilate the mask by one pixel) ---
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let fill = mask.get(x as isize, y as isize);

            let mut outline: f32 = 0.0;
            for oy in -1..=1_isize {
                for ox in -1..=1_isize {
                    outline = outline.max(mask.get(x as isize + ox, y as isize + oy));
                }
            }

            // White outline underneath, black fill on top; premultiplied.
            let white = outline * (1.0 - fill);
            let alpha = fill + outline * (1.0 - fill);
            let c = (white * 255.0).round().clamp(0.0, 255.0) as u8;

            let i = (y * w + x) * 4;
            rgba[i] = c;
            rgba[i + 1] = c;
            rgba[i + 2] = c;
            rgba[i + 3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    let hot = to_px(hotspot);
    Some(egui::CursorImage {
        id,
        rgba: Arc::new(rgba),
        size: (w as u16, h as u16),
        hotspot: (
            hot.x.round().clamp(0.0, w as f32 - 1.0) as u16,
            hot.y.round().clamp(0.0, h as f32 - 1.0) as u16,
        ),
    })
}

// --- Per-frame cursor slot using egui context data ---

#[derive(Clone, Copy)]
struct ActiveCustomCursor(CustomCursor);

/// Set the custom cursor for this frame. Call from any pane during rendering.
pub fn set(ctx: &egui::Context, cursor: CustomCursor) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("active_custom_cursor"), ActiveCustomCursor(cursor)));
}

/// Hand the active cursor to the windowing system. Call at the end of the main update loop.
pub fn render_overlay(ctx: &egui::Context, cache: &mut CursorCache) {
    // Take and remove the cursor so it doesn't persist to the next frame
    let id = egui::Id::new("active_custom_cursor");
    let cursor = ctx.data_mut(|d| {
        let val = d.get_temp::<ActiveCustomCursor>(id);
        d.remove::<ActiveCustomCursor>(id);
        val
    });

    let Some(ActiveCustomCursor(cursor)) = cursor else { return };

    // If a widget explicitly asked for a system cursor (resize handles, text inputs, ...), let it
    // win — it knows something about the hover target that we don't.
    if ctx.output(|o| o.cursor_icon) != egui::CursorIcon::Default {
        return;
    }

    match cursor.kind() {
        CursorKind::System(icon) => ctx.set_cursor_icon(icon),
        _ => {
            let ppp = ctx.pixels_per_point();
            if let Some(image) = cache.get_or_build(ctx, cursor, ppp) {
                ctx.set_cursor_image(Some(image));
            } else {
                // Rasterization failed — better a crosshair than an invisible cursor.
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            }
        }
    }
}
