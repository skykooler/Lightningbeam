//! Font registry + text layout for text layers.
//!
//! Wraps a thread-local parley [`FontContext`]/[`LayoutContext`]. Three fonts are
//! bundled (serif, sans-serif, monospaced) so text renders deterministically even
//! offline; system fonts are also enumerated (parley's fontique source) and
//! document-embedded fonts can be registered at load time (see `file_io`).
//!
//! The same `linebender_resource_handle` crate (0.1.1) backs both vello's
//! `peniko::FontData` and parley's `FontData`, so a glyph run's `run.font()` can be
//! handed straight to `vello::Scene::draw_glyphs` with no conversion.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontFamilyName, Layout, LayoutContext,
    PositionedLayoutItem, StyleProperty,
};
use vello::peniko::Blob;

use crate::text_layer::{TextAlign, TextContent};

// ── Bundled fonts (SIL OFL; vendored under assets/fonts) ─────────────────────
static BUNDLED_SANS: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf");
static BUNDLED_SERIF: &[u8] = include_bytes!("../assets/fonts/LiberationSerif-Regular.ttf");
static BUNDLED_MONO: &[u8] = include_bytes!("../assets/fonts/LiberationMono-Regular.ttf");

/// Parley requires a brush type, but glyph color is applied by vello at draw time,
/// so we use a zero-sized placeholder.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct NoBrush;

struct FontStore {
    fcx: FontContext,
    lcx: LayoutContext<NoBrush>,
    /// Registered family names of the three bundled fonts (sans, serif, mono).
    bundled: Vec<String>,
    /// The default family (bundled sans-serif), used when `font_family` is empty.
    default_family: String,
}

impl FontStore {
    fn new() -> Self {
        let mut fcx = FontContext::new();
        let mut bundled = Vec::new();
        for bytes in [BUNDLED_SANS, BUNDLED_SERIF, BUNDLED_MONO] {
            let blob = Blob::new(Arc::new(bytes) as Arc<dyn AsRef<[u8]> + Send + Sync>);
            for (family_id, _) in fcx.collection.register_fonts(blob, None) {
                if let Some(name) = fcx.collection.family_name(family_id) {
                    let name = name.to_string();
                    if !bundled.contains(&name) {
                        bundled.push(name);
                    }
                }
            }
        }
        let default_family = bundled.first().cloned().unwrap_or_else(|| "sans-serif".to_string());
        Self { fcx, lcx: LayoutContext::new(), bundled, default_family }
    }
}

thread_local! {
    static STORE: RefCell<FontStore> = RefCell::new(FontStore::new());
}

/// The default family name (bundled sans-serif), used when a text layer's
/// `font_family` is empty.
pub fn default_family() -> String {
    STORE.with(|s| s.borrow().default_family.clone())
}

/// Whether `family` is one of the three bundled families (which therefore must
/// never be embedded into a `.beam`).
pub fn is_bundled(family: &str) -> bool {
    STORE.with(|s| s.borrow().bundled.iter().any(|f| f == family))
}

/// True if a shorter word-prefix of `name` is itself a family in `set` — i.e. `name`
/// is a variant of a base family (e.g. "Noto Sans Arabic" when "Noto Sans" exists).
/// Used to collapse the many script/region-specific fonts (mostly Noto) into their
/// base family in the picker; parley's fallback still resolves the right script font
/// automatically when rendering non-Latin text.
fn is_variant_of_listed_base(name: &str, set: &std::collections::HashSet<&str>) -> bool {
    let words: Vec<&str> = name.split(' ').collect();
    for k in 1..words.len() {
        if set.contains(words[..k].join(" ").as_str()) {
            return true;
        }
    }
    false
}

/// Available family names for the info-panel picker: the three bundled families first,
/// then a consolidated, alphabetical list of system families — script/region variants
/// whose base family is present are dropped (monospace variants are kept).
pub fn families() -> Vec<String> {
    STORE.with(|s| {
        let s = &mut *s.borrow_mut();

        let mut system: Vec<String> =
            s.fcx.collection.family_names().map(|n| n.to_string()).collect();
        system.sort();
        system.dedup();
        let set: std::collections::HashSet<&str> = system.iter().map(|x| x.as_str()).collect();

        let mut out = s.bundled.clone();
        for name in &system {
            if out.iter().any(|b| b == name) {
                continue;
            }
            // Keep base families and monospace variants; drop script/region variants
            // whose base family is already in the list.
            if name.contains("Mono") || !is_variant_of_listed_base(name, &set) {
                out.push(name.clone());
            }
        }
        out
    })
}

/// Register a document-embedded font (raw TTF/OTF bytes) into the runtime font
/// collection so text layers referencing its family resolve to it. Returns the
/// registered family names.
pub fn register_embedded(bytes: Vec<u8>) -> Vec<String> {
    STORE.with(|s| {
        let s = &mut *s.borrow_mut();
        let blob = Blob::new(Arc::new(bytes) as Arc<dyn AsRef<[u8]> + Send + Sync>);
        let mut names = Vec::new();
        for (family_id, _) in s.fcx.collection.register_fonts(blob, None) {
            if let Some(name) = s.fcx.collection.family_name(family_id) {
                names.push(name.to_string());
            }
        }
        names
    })
}

/// Whether `family` currently resolves to a registered family (bundled, system,
/// or embedded). Used to flag "missing font" on load.
pub fn family_available(family: &str) -> bool {
    if family.is_empty() {
        return true;
    }
    STORE.with(|s| {
        let s = &mut *s.borrow_mut();
        s.fcx.collection.family_id(family).is_some()
    })
}

/// The raw bytes of the font file that `family` resolves to (for embedding into a
/// `.beam`). Returns `None` if the family resolves to no glyphs. Lays out a single
/// glyph and reads the resolved run's font blob (the same `linebender_resource_handle`
/// blob vello uses).
pub fn family_font_bytes(family: &str) -> Option<Vec<u8>> {
    let content = TextContent {
        text: "A".to_string(),
        font_family: family.to_string(),
        ..TextContent::default()
    };
    with_layout(&content, 10_000.0, |layout| {
        for line in layout.lines() {
            for item in line.items() {
                if let PositionedLayoutItem::GlyphRun(run) = item {
                    return Some(run.run().font().data.data().to_vec());
                }
            }
        }
        None
    })
}

// ── Background font preloading ───────────────────────────────────────────────
//
// Loading a family's bytes (`family_font_bytes`) is the expensive part (font file IO +
// shaping). Doing it for the whole picker on the UI thread causes hitches, so a
// background thread (started at app launch) loads every picker family's bytes ahead of
// time into a shared map. The UI then only has to *register* them with its renderer
// (cheap), and synchronously loads the few stragglers only if needed before they're ready.

struct PreloadState {
    loaded: HashMap<String, Vec<u8>>,
    started: bool,
    done: bool,
}

fn preload() -> &'static Mutex<PreloadState> {
    static P: OnceLock<Mutex<PreloadState>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(PreloadState {
        loaded: HashMap::new(),
        started: false,
        done: false,
    }))
}

/// Start (once) a background thread that loads every picker font's bytes. Idempotent;
/// safe to call every frame or from app startup.
pub fn start_preload() {
    {
        let mut p = preload().lock().unwrap();
        if p.started {
            return;
        }
        p.started = true;
    }
    let _ = std::thread::Builder::new()
        .name("font-preload".into())
        .spawn(|| {
            // This thread gets its own thread-local FontContext (enumerates system fonts
            // off the UI thread). Bytes are plain `Vec<u8>` (Send) handed back via the map.
            for fam in families() {
                if let Some(bytes) = family_font_bytes(&fam) {
                    preload().lock().unwrap().loaded.insert(fam, bytes);
                }
            }
            preload().lock().unwrap().done = true;
        });
}

/// Remove and return a preloaded font's bytes if the background thread has them ready.
pub fn take_preloaded(family: &str) -> Option<Vec<u8>> {
    preload().lock().unwrap().loaded.remove(family)
}

/// Whether the background preloader has finished loading every family.
pub fn preload_done() -> bool {
    preload().lock().unwrap().done
}

/// Caret rectangle (x0, y0, x1, y1, in layout space relative to the box origin) for
/// `byte_index` into `content.text`, wrapped to `max_width`.
pub fn caret_geometry(content: &TextContent, max_width: f32, byte_index: usize) -> Option<(f64, f64, f64, f64)> {
    let caret_w = (content.font_size as f32 * 0.06).max(1.0);
    with_layout(content, max_width, |layout| {
        let cur = parley::Cursor::from_byte_index(layout, byte_index, parley::Affinity::Downstream);
        let bb = cur.geometry(layout, caret_w);
        Some((bb.x0, bb.y0, bb.x1, bb.y1))
    })
}

/// Selection highlight rectangles (each x0, y0, x1, y1 in layout space) for the byte
/// range `[start, end)` into `content.text`, wrapped to `max_width`.
pub fn selection_geometry(content: &TextContent, max_width: f32, start: usize, end: usize) -> Vec<(f64, f64, f64, f64)> {
    if start == end {
        return Vec::new();
    }
    with_layout(content, max_width, |layout| {
        let anchor = parley::Cursor::from_byte_index(layout, start, parley::Affinity::Downstream);
        let focus = parley::Cursor::from_byte_index(layout, end, parley::Affinity::Downstream);
        let sel = parley::Selection::new(anchor, focus);
        sel.geometry(layout)
            .into_iter()
            .map(|(bb, _)| (bb.x0, bb.y0, bb.x1, bb.y1))
            .collect()
    })
}

fn alignment_of(a: TextAlign) -> Alignment {
    match a {
        TextAlign::Left => Alignment::Left,
        TextAlign::Center => Alignment::Center,
        TextAlign::Right => Alignment::Right,
        TextAlign::Justify => Alignment::Justify,
    }
}

/// Build a parley layout for `content` wrapped to `max_width` (document units),
/// then invoke `f` with it. The layout is rebuilt on each call (v1: no cache).
pub fn with_layout<R>(
    content: &TextContent,
    max_width: f32,
    f: impl FnOnce(&Layout<NoBrush>) -> R,
) -> R {
    STORE.with(|s| {
        let s = &mut *s.borrow_mut();
        let default_family = s.default_family.clone();
        let FontStore { fcx, lcx, .. } = s;

        let mut builder = lcx.ranged_builder(fcx, &content.text, 1.0, true);
        builder.push_default(StyleProperty::FontSize(content.font_size as f32));

        let family_name = if content.font_family.is_empty() {
            default_family
        } else {
            content.font_family.clone()
        };
        let family = FontFamily::Single(FontFamilyName::Named(Cow::Owned(family_name)));
        builder.push_default(StyleProperty::FontFamily(family));

        let mut layout = builder.build(&content.text);
        layout.break_all_lines(Some(max_width));
        layout.align(alignment_of(content.align), AlignmentOptions::default());
        f(&layout)
    })
}
