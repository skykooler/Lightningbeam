//! Dirty-rect diff for raster undo/redo.
//!
//! Brush strokes and fills used to store the *entire* before/after RGBA frame in the
//! undo stack (~8 MB each at 1080p → up to ~1.6 GB at the 100-action cap). A
//! [`RasterDiff`] instead stores only the changed bounding box's pixels before and
//! after, which for a typical brush dab is a few tens of KB.
//!
//! Applying a diff overwrites just the bbox of the keyframe's `raw_pixels`, so the
//! buffer **must be resident** (full length `w*h*4`) when `apply_*` runs. The editor
//! guarantees this by faulting the target frame in before undo/redo (a clean evicted
//! frame's container bytes equal its current logical state, so the restored base is
//! correct). If the base is somehow not resident we skip rather than corrupt.

/// Normalize a buffer to full length `n`; an empty/short buffer becomes transparent.
fn normalize(buf: &[u8], n: usize) -> std::borrow::Cow<[u8]> {
    if buf.len() == n {
        std::borrow::Cow::Borrowed(buf)
    } else {
        std::borrow::Cow::Owned(vec![0u8; n])
    }
}

/// A minimal before/after record of the region a raster edit changed.
#[derive(Clone, Debug)]
pub struct RasterDiff {
    full_width: u32,
    full_height: u32,
    /// Changed bounding box `(x, y, w, h)`; `None` when before == after (no-op).
    bbox: Option<(u32, u32, u32, u32)>,
    /// bbox-sized RGBA (`w*h*4`) of the region before the edit.
    before_region: Vec<u8>,
    /// bbox-sized RGBA (`w*h*4`) of the region after the edit.
    after_region: Vec<u8>,
}

impl RasterDiff {
    /// Build a diff from full before/after buffers. `after` is expected to be the
    /// resident post-edit buffer (`width*height*4`); `before` may be empty (a blank
    /// keyframe's first stroke), treated as fully transparent.
    pub fn compute(before: &[u8], after: &[u8], width: u32, height: u32) -> Self {
        let n = width as usize * height as usize * 4;
        // Normalize both sides to full length; empty/short ⇒ transparent.
        let before_full = normalize(before, n);
        let after_full = normalize(after, n);

        // Find the tight bbox of differing pixels (compare 4-byte texels).
        let (w, h) = (width as usize, height as usize);
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut any = false;
        for y in 0..h {
            let row = y * w * 4;
            for x in 0..w {
                let i = row + x * 4;
                if before_full[i..i + 4] != after_full[i..i + 4] {
                    any = true;
                    if x < min_x { min_x = x; }
                    if x > max_x { max_x = x; }
                    if y < min_y { min_y = y; }
                    if y > max_y { max_y = y; }
                }
            }
        }

        if !any {
            return Self { full_width: width, full_height: height, bbox: None,
                          before_region: Vec::new(), after_region: Vec::new() };
        }

        let bw = max_x - min_x + 1;
        let bh = max_y - min_y + 1;
        let crop = |full: &[u8]| -> Vec<u8> {
            let mut out = Vec::with_capacity(bw * bh * 4);
            for row in 0..bh {
                let src = ((min_y + row) * w + min_x) * 4;
                out.extend_from_slice(&full[src..src + bw * 4]);
            }
            out
        };

        Self {
            full_width: width,
            full_height: height,
            bbox: Some((min_x as u32, min_y as u32, bw as u32, bh as u32)),
            before_region: crop(&before_full),
            after_region: crop(&after_full),
        }
    }

    /// Approximate retained size in bytes (the two cropped regions).
    pub fn byte_size(&self) -> usize {
        self.before_region.len() + self.after_region.len()
    }

    /// Restore the pre-edit pixels into `raw` (undo).
    pub fn apply_before(&self, raw: &mut Vec<u8>) {
        self.apply(&self.before_region, raw);
    }

    /// Re-apply the post-edit pixels into `raw` (redo).
    pub fn apply_after(&self, raw: &mut Vec<u8>) {
        self.apply(&self.after_region, raw);
    }

    fn apply(&self, region: &[u8], raw: &mut Vec<u8>) {
        let n = self.full_width as usize * self.full_height as usize * 4;
        let (x, y, bw, bh) = match self.bbox {
            Some(b) => b,
            None => return, // no change
        };
        if raw.len() != n {
            // Base not resident: the editor should have faulted it in. Skip rather
            // than resize-and-corrupt — the frame will re-page to its container state.
            eprintln!(
                "⚠️ [RASTER_DIFF] base not resident ({} != {}); skipping undo/redo apply",
                raw.len(), n
            );
            return;
        }
        let (x, y, bw, bh) = (x as usize, y as usize, bw as usize, bh as usize);
        let fw = self.full_width as usize;
        for row in 0..bh {
            let dst = ((y + row) * fw + x) * 4;
            let src = row * bw * 4;
            raw[dst..dst + bw * 4].copy_from_slice(&region[src..src + bw * 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.iter().copied().cycle().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn roundtrip_reproduces_buffers_exactly() {
        let (w, h) = (8, 6);
        let before = solid(w, h, [10, 20, 30, 255]);
        let mut after = before.clone();
        // Change a 2x2 region at (3,2).
        for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let i = (((2 + dy) * w + (3 + dx)) * 4) as usize;
            after[i..i + 4].copy_from_slice(&[200, 100, 50, 255]);
        }
        let diff = RasterDiff::compute(&before, &after, w, h);
        assert_eq!(diff.bbox, Some((3, 2, 2, 2)));

        let mut buf = after.clone();
        diff.apply_before(&mut buf);
        assert_eq!(buf, before, "undo must reproduce the pre-edit buffer exactly");
        diff.apply_after(&mut buf);
        assert_eq!(buf, after, "redo must reproduce the post-edit buffer exactly");
    }

    #[test]
    fn blank_before_first_stroke() {
        let (w, h) = (4, 4);
        let before: Vec<u8> = Vec::new(); // blank keyframe
        let mut after = vec![0u8; (w * h * 4) as usize];
        let i = ((1 * w + 1) * 4) as usize;
        after[i..i + 4].copy_from_slice(&[255, 0, 0, 255]);
        let diff = RasterDiff::compute(&before, &after, w, h);
        assert_eq!(diff.bbox, Some((1, 1, 1, 1)));

        // Undo onto the resident post-stroke buffer → fully transparent (blank-equiv).
        let mut buf = after.clone();
        diff.apply_before(&mut buf);
        assert_eq!(buf, vec![0u8; (w * h * 4) as usize]);
    }

    #[test]
    fn no_change_is_noop() {
        let (w, h) = (4, 4);
        let buf = solid(w, h, [1, 2, 3, 4]);
        let diff = RasterDiff::compute(&buf, &buf, w, h);
        assert_eq!(diff.bbox, None);
        assert_eq!(diff.byte_size(), 0);
        let mut b = buf.clone();
        diff.apply_before(&mut b);
        assert_eq!(b, buf);
    }

    #[test]
    fn not_resident_base_is_skipped_not_corrupted() {
        let (w, h) = (4, 4);
        let before = solid(w, h, [9, 9, 9, 255]);
        let after = solid(w, h, [1, 2, 3, 255]);
        let diff = RasterDiff::compute(&before, &after, w, h);
        let mut empty: Vec<u8> = Vec::new();
        diff.apply_before(&mut empty); // base not resident
        assert!(empty.is_empty(), "must not resize/corrupt a non-resident base");
    }
}
