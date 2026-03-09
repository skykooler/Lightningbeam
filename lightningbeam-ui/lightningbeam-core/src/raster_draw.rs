//! CPU-side raster drawing primitives for geometric shapes on raster layers.
//!
//! All coordinates are in canvas pixels (f32).  The pixel buffer is RGBA u8,
//! 4 bytes per pixel, row-major, top-left origin.

/// RGBA color as `[R, G, B, A]` bytes.
pub type Rgba = [u8; 4];

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Alpha-composite `color` (RGBA) onto `pixels[idx..idx+4]` with an extra
/// `coverage` factor (0.0 = transparent, 1.0 = full color alpha).
#[inline]
fn blend_at(pixels: &mut [u8], idx: usize, color: Rgba, coverage: f32) {
    let a = (color[3] as f32 / 255.0) * coverage;
    if a <= 0.0 { return; }
    let inv = 1.0 - a;
    pixels[idx]     = (color[0] as f32 * a + pixels[idx]     as f32 * inv) as u8;
    pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv) as u8;
    pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv) as u8;
    pixels[idx + 3] = ((a + pixels[idx + 3] as f32 / 255.0 * inv) * 255.0).min(255.0) as u8;
}

/// Write a pixel at integer canvas coordinates, clipped to canvas bounds.
#[inline]
fn put(pixels: &mut [u8], w: u32, h: u32, x: i32, y: i32, color: Rgba, coverage: f32) {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { return; }
    let idx = (y as u32 * w + x as u32) as usize * 4;
    blend_at(pixels, idx, color, coverage);
}

/// Draw an anti-aliased filled disk at (`cx`, `cy`) with the given `radius`.
fn draw_disk(pixels: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, radius: f32, color: Rgba) {
    let r = (radius + 1.0) as i32;
    let ix = cx as i32;
    let iy = cy as i32;
    for dy in -r..=r {
        for dx in -r..=r {
            let px = ix + dx;
            let py = iy + dy;
            let dist = ((px as f32 - cx).powi(2) + (py as f32 - cy).powi(2)).sqrt();
            let cov = (radius + 0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                put(pixels, w, h, px, py, color, cov);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Draw a thick line from (`x0`, `y0`) to (`x1`, `y1`) by stamping
/// anti-aliased disks of radius `thickness / 2` at every half-pixel step.
pub fn draw_line(
    pixels: &mut [u8], w: u32, h: u32,
    x0: f32, y0: f32, x1: f32, y1: f32,
    color: Rgba, thickness: f32,
) {
    let radius = (thickness / 2.0).max(0.5);
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        draw_disk(pixels, w, h, x0, y0, radius, color);
        return;
    }
    let steps = ((len * 2.0).ceil() as i32).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        draw_disk(pixels, w, h, x0 + dx * t, y0 + dy * t, radius, color);
    }
}

/// Draw a rectangle with corners (`x0`, `y0`) and (`x1`, `y1`).
///
/// `stroke` draws the four edges; `fill` fills the interior.  Either may be
/// `None` to skip that part.
pub fn draw_rect(
    pixels: &mut [u8], w: u32, h: u32,
    x0: f32, y0: f32, x1: f32, y1: f32,
    stroke: Option<Rgba>, fill: Option<Rgba>, thickness: f32,
) {
    let (lx, rx) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (ty, by) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };

    if let Some(fc) = fill {
        let px0 = lx.ceil() as i32;
        let py0 = ty.ceil() as i32;
        let px1 = rx.floor() as i32;
        let py1 = by.floor() as i32;
        for py in py0..=py1 {
            for px in px0..=px1 {
                put(pixels, w, h, px, py, fc, 1.0);
            }
        }
    }

    if let Some(sc) = stroke {
        draw_line(pixels, w, h, lx, ty, rx, ty, sc, thickness); // top
        draw_line(pixels, w, h, rx, ty, rx, by, sc, thickness); // right
        draw_line(pixels, w, h, rx, by, lx, by, sc, thickness); // bottom
        draw_line(pixels, w, h, lx, by, lx, ty, sc, thickness); // left
    }
}

/// Draw an ellipse centred at (`cx`, `cy`) with semi-axes `rx` and `ry`.
///
/// `stroke` draws the outline; `fill` fills the interior via scanline.
pub fn draw_ellipse(
    pixels: &mut [u8], w: u32, h: u32,
    cx: f32, cy: f32, rx: f32, ry: f32,
    stroke: Option<Rgba>, fill: Option<Rgba>, thickness: f32,
) {
    if rx <= 0.0 || ry <= 0.0 { return; }

    if let Some(fc) = fill {
        let py0 = (cy - ry).ceil() as i32;
        let py1 = (cy + ry).floor() as i32;
        for py in py0..=py1 {
            let dy = py as f32 - cy;
            let t = 1.0 - (dy / ry).powi(2);
            if t <= 0.0 { continue; }
            let x_ext = rx * t.sqrt();
            let px0 = (cx - x_ext).ceil() as i32;
            let px1 = (cx + x_ext).floor() as i32;
            for px in px0..=px1 {
                put(pixels, w, h, px, py, fc, 1.0);
            }
        }
    }

    if let Some(sc) = stroke {
        let radius = (thickness / 2.0).max(0.5);
        // Ramanujan's perimeter approximation for step count.
        let perim = std::f32::consts::PI
            * (3.0 * (rx + ry) - ((3.0 * rx + ry) * (rx + 3.0 * ry)).sqrt());
        let steps = ((perim * 2.0).ceil() as i32).max(16);
        for i in 0..steps {
            let t = i as f32 / steps as f32 * std::f32::consts::TAU;
            draw_disk(pixels, w, h, cx + rx * t.cos(), cy + ry * t.sin(), radius, sc);
        }
    }
}

/// Draw a closed polygon given world-space `vertices` (at least 2).
///
/// `stroke` draws the outline; `fill` fills the interior via scanline.
pub fn draw_polygon(
    pixels: &mut [u8], w: u32, h: u32,
    vertices: &[(f32, f32)],
    stroke: Option<Rgba>, fill: Option<Rgba>, thickness: f32,
) {
    let n = vertices.len();
    if n < 2 { return; }

    if let Some(fc) = fill {
        let min_y = vertices.iter().map(|v| v.1).fold(f32::MAX, f32::min).ceil() as i32;
        let max_y = vertices.iter().map(|v| v.1).fold(f32::MIN, f32::max).floor() as i32;
        let mut xs: Vec<f32> = Vec::with_capacity(n);
        for py in min_y..=max_y {
            xs.clear();
            let scan_y = py as f32 + 0.5;
            for i in 0..n {
                let (x0, y0) = vertices[i];
                let (x1, y1) = vertices[(i + 1) % n];
                if (y0 <= scan_y && scan_y < y1) || (y1 <= scan_y && scan_y < y0) {
                    xs.push(x0 + (scan_y - y0) / (y1 - y0) * (x1 - x0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mut j = 0;
            while j + 1 < xs.len() {
                let px0 = xs[j].ceil() as i32;
                let px1 = xs[j + 1].floor() as i32;
                for px in px0..=px1 {
                    put(pixels, w, h, px, py, fc, 1.0);
                }
                j += 2;
            }
        }
    }

    if let Some(sc) = stroke {
        for i in 0..n {
            let (x0, y0) = vertices[i];
            let (x1, y1) = vertices[(i + 1) % n];
            draw_line(pixels, w, h, x0, y0, x1, y1, sc, thickness);
        }
    }
}
