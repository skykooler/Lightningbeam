//! Custom cursor system
//!
//! Provides SVG-based custom cursors beyond egui's built-in system cursors.
//! When a custom cursor is active, the system cursor is hidden and the SVG
//! cursor image is drawn at the pointer position.

use eframe::egui;
use egui::TextureHandle;
use lightningbeam_core::tool::Tool;
use std::collections::HashMap;

/// Custom cursor identifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CustomCursor {
    // Stage tool cursors
    Select,
    Draw,
    Transform,
    Rectangle,
    Ellipse,
    PaintBucket,
    Eyedropper,
    Line,
    Polygon,
    BezierEdit,
    Text,
    // Timeline cursors
    LoopExtend,
}

impl CustomCursor {
    /// Convert a Tool enum to the corresponding custom cursor
    pub fn from_tool(tool: Tool) -> Self {
        match tool {
            Tool::Select => CustomCursor::Select,
            Tool::Draw => CustomCursor::Draw,
            Tool::Transform => CustomCursor::Transform,
            Tool::Rectangle => CustomCursor::Rectangle,
            Tool::Ellipse => CustomCursor::Ellipse,
            Tool::PaintBucket => CustomCursor::PaintBucket,
            Tool::Eyedropper => CustomCursor::Eyedropper,
            Tool::Line => CustomCursor::Line,
            Tool::Polygon => CustomCursor::Polygon,
            Tool::BezierEdit => CustomCursor::BezierEdit,
            Tool::Text => CustomCursor::Text,
            Tool::RegionSelect => CustomCursor::Select, // Reuse select cursor for now
            Tool::Split => CustomCursor::Select, // Reuse select cursor for now
            Tool::Erase => CustomCursor::Draw, // Reuse draw cursor for raster erase
            Tool::Smudge => CustomCursor::Draw, // Reuse draw cursor for raster smudge
        }
    }

    /// Hotspot offset — the "click point" relative to the image top-left
    pub fn hotspot(&self) -> egui::Vec2 {
        match self {
            // Select cursor: pointer tip at top-left
            CustomCursor::Select => egui::vec2(3.0, 1.0),
            // Drawing tools: tip at bottom-left
            CustomCursor::Draw => egui::vec2(1.0, 23.0),
            // Transform: center
            CustomCursor::Transform => egui::vec2(12.0, 12.0),
            // Shape tools: crosshair at center
            CustomCursor::Rectangle
            | CustomCursor::Ellipse
            | CustomCursor::Line
            | CustomCursor::Polygon => egui::vec2(12.0, 12.0),
            // Paint bucket: tip at bottom-left
            CustomCursor::PaintBucket => egui::vec2(2.0, 21.0),
            // Eyedropper: tip at bottom
            CustomCursor::Eyedropper => egui::vec2(4.0, 22.0),
            // Bezier edit: tip at top-left
            CustomCursor::BezierEdit => egui::vec2(3.0, 1.0),
            // Text: I-beam center
            CustomCursor::Text => egui::vec2(12.0, 12.0),
            // Loop extend: center of circular arrow
            CustomCursor::LoopExtend => egui::vec2(12.0, 12.0),
        }
    }

    /// Get the embedded SVG data for this cursor
    fn svg_data(&self) -> &'static [u8] {
        match self {
            CustomCursor::Select => include_bytes!("../../../src/assets/select.svg"),
            CustomCursor::Draw => include_bytes!("../../../src/assets/draw.svg"),
            CustomCursor::Transform => include_bytes!("../../../src/assets/transform.svg"),
            CustomCursor::Rectangle => include_bytes!("../../../src/assets/rectangle.svg"),
            CustomCursor::Ellipse => include_bytes!("../../../src/assets/ellipse.svg"),
            CustomCursor::PaintBucket => include_bytes!("../../../src/assets/paint_bucket.svg"),
            CustomCursor::Eyedropper => include_bytes!("../../../src/assets/eyedropper.svg"),
            CustomCursor::Line => include_bytes!("../../../src/assets/line.svg"),
            CustomCursor::Polygon => include_bytes!("../../../src/assets/polygon.svg"),
            CustomCursor::BezierEdit => include_bytes!("../../../src/assets/bezier_edit.svg"),
            CustomCursor::Text => include_bytes!("../../../src/assets/text.svg"),
            CustomCursor::LoopExtend => include_bytes!("../../../src/assets/arrow-counterclockwise.svg"),
        }
    }
}

/// Cache of rasterized cursor textures (black fill + white outline version)
pub struct CursorCache {
    /// Black cursor for the main image
    textures: HashMap<CustomCursor, TextureHandle>,
    /// White cursor for the outline
    outline_textures: HashMap<CustomCursor, TextureHandle>,
}

impl CursorCache {
    pub fn new() -> Self {
        Self {
            textures: HashMap::new(),
            outline_textures: HashMap::new(),
        }
    }

    /// Get or lazily load the black (fill) cursor texture
    pub fn get_or_load(&mut self, cursor: CustomCursor, ctx: &egui::Context) -> &TextureHandle {
        self.textures.entry(cursor).or_insert_with(|| {
            let svg_data = cursor.svg_data();
            let svg_string = String::from_utf8_lossy(svg_data);
            let svg_with_color = svg_string.replace("currentColor", "#000000");
            rasterize_cursor_svg(svg_with_color.as_bytes(), &format!("cursor_{:?}", cursor), CURSOR_SIZE, ctx)
                .expect("Failed to rasterize cursor SVG")
        })
    }

    /// Get or lazily load the white (outline) cursor texture
    pub fn get_or_load_outline(&mut self, cursor: CustomCursor, ctx: &egui::Context) -> &TextureHandle {
        self.outline_textures.entry(cursor).or_insert_with(|| {
            let svg_data = cursor.svg_data();
            let svg_string = String::from_utf8_lossy(svg_data);
            // Replace all colors with white for the outline
            let svg_white = svg_string
                .replace("currentColor", "#ffffff")
                .replace("#000000", "#ffffff")
                .replace("#000", "#ffffff");
            rasterize_cursor_svg(svg_white.as_bytes(), &format!("cursor_{:?}_outline", cursor), CURSOR_SIZE, ctx)
                .expect("Failed to rasterize cursor SVG outline")
        })
    }
}

const CURSOR_SIZE: u32 = 24;
const OUTLINE_OFFSET: f32 = 1.0;

/// Rasterize an SVG into an egui texture (same approach as main.rs rasterize_svg)
fn rasterize_cursor_svg(
    svg_data: &[u8],
    name: &str,
    render_size: u32,
    ctx: &egui::Context,
) -> Option<TextureHandle> {
    let tree = resvg::usvg::Tree::from_data(svg_data, &resvg::usvg::Options::default()).ok()?;
    let pixmap_size = tree.size().to_int_size();
    let scale_x = render_size as f32 / pixmap_size.width() as f32;
    let scale_y = render_size as f32 / pixmap_size.height() as f32;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_size, render_size)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale_x, scale_y),
        &mut pixmap.as_mut(),
    );
    let rgba_data = pixmap.data().to_vec();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        [render_size as usize, render_size as usize],
        &rgba_data,
    );
    Some(ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR))
}

// --- Per-frame cursor slot using egui context data ---

/// Key for storing the active custom cursor in egui's per-frame data
#[derive(Clone, Copy)]
struct ActiveCustomCursor(CustomCursor);

/// Set the custom cursor for this frame. Call from any pane during rendering.
/// This hides the system cursor and draws the SVG cursor at pointer position.
pub fn set(ctx: &egui::Context, cursor: CustomCursor) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::new("active_custom_cursor"), ActiveCustomCursor(cursor)));
}

/// Render the custom cursor overlay. Call at the end of the main update loop.
pub fn render_overlay(ctx: &egui::Context, cache: &mut CursorCache) {
    // Take and remove the cursor so it doesn't persist to the next frame
    let id = egui::Id::new("active_custom_cursor");
    let cursor = ctx.data_mut(|d| {
        let val = d.get_temp::<ActiveCustomCursor>(id);
        d.remove::<ActiveCustomCursor>(id);
        val
    });

    if let Some(ActiveCustomCursor(cursor)) = cursor {
        // If a system cursor was explicitly set (resize handles, text inputs, etc.),
        // let it take priority over the custom cursor
        let system_cursor = ctx.output(|o| o.cursor_icon);
        if system_cursor != egui::CursorIcon::Default {
            return;
        }

        // Hide the system cursor
        ctx.set_cursor_icon(egui::CursorIcon::None);

        if let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) {
            let hotspot = cursor.hotspot();
            let size = egui::vec2(CURSOR_SIZE as f32, CURSOR_SIZE as f32);
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            let painter = ctx.debug_painter();

            // Draw white outline: render white version offset in 8 directions
            let outline_tex = cache.get_or_load_outline(cursor, ctx);
            let outline_id = outline_tex.id();
            for &(dx, dy) in &[
                (-OUTLINE_OFFSET, 0.0), (OUTLINE_OFFSET, 0.0),
                (0.0, -OUTLINE_OFFSET), (0.0, OUTLINE_OFFSET),
                (-OUTLINE_OFFSET, -OUTLINE_OFFSET), (OUTLINE_OFFSET, -OUTLINE_OFFSET),
                (-OUTLINE_OFFSET, OUTLINE_OFFSET), (OUTLINE_OFFSET, OUTLINE_OFFSET),
            ] {
                let offset_rect = egui::Rect::from_min_size(
                    pos - hotspot + egui::vec2(dx, dy),
                    size,
                );
                painter.image(outline_id, offset_rect, uv, egui::Color32::WHITE);
            }

            // Draw black fill on top
            let fill_tex = cache.get_or_load(cursor, ctx);
            let cursor_rect = egui::Rect::from_min_size(pos - hotspot, size);
            painter.image(fill_tex.id(), cursor_rect, uv, egui::Color32::WHITE);
        }
    }
}
