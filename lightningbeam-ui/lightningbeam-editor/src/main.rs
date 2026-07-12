#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use daw_backend::{Beats, Seconds};
use lightningbeam_core::clip::ClipDuration;
use lightningbeam_core::layer::{AnyLayer, AudioLayer};
use lightningbeam_core::layout::{LayoutDefinition, LayoutNode};
use lightningbeam_core::pane::PaneType;
use lightningbeam_core::tool::Tool;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use clap::Parser;
use uuid::Uuid;

mod mobile;
mod panes;
use panes::{PaneInstance, PaneRenderer};

mod tools;

mod widgets;

mod menu;
use menu::{MenuAction, MenuSystem};

mod theme;
mod theme_render;
use theme::{Theme, ThemeMode};

mod waveform_gpu;
mod cqt_gpu;
mod gpu_brush;

mod raster_tool;

mod config;
use config::AppConfig;

mod keymap;
use keymap::KeymapManager;

mod default_instrument;

mod export;

mod preferences;

mod notifications;

mod effect_thumbnails;
use effect_thumbnails::EffectThumbnailGenerator;

mod custom_cursor;
mod tablet;
mod debug_overlay;
mod gpu_timer;
#[cfg(target_os = "linux")]
mod hw_video;
mod nv12_blit;

#[cfg(debug_assertions)]
mod test_mode;

mod sample_import;
mod sample_import_dialog;
mod svg_import;

mod curve_editor;

/// Lightningbeam Editor - Animation and video editing software
#[derive(Parser, Debug)]
#[command(name = "Lightningbeam Editor")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Use light theme
    #[arg(long, conflicts_with = "dark")]
    light: bool,

    /// Use dark theme
    #[arg(long, conflicts_with = "light")]
    dark: bool,

    /// Force Vello to use its CPU renderer instead of the GPU.
    /// Useful for testing the CPU fallback path or working around GPU driver issues.
    #[arg(long)]
    cpu_renderer: bool,
}

/// The default eframe wgpu device setup (wgpu picks the adapter/device). Used on non-Linux, when
/// the shared VAAPI device is unavailable, or when disabled via `LB_NO_SHARED_DEVICE`.
fn lb_default_wgpu_setup() -> egui_wgpu::WgpuSetup {
    egui_wgpu::WgpuSetup::CreateNew(egui_wgpu::WgpuSetupCreateNew {
        device_descriptor: std::sync::Arc::new(|adapter| {
            let features = adapter.features();
            // Request SHADER_F16 if available — needed on Mesa/llvmpipe for vello's
            // unpack2x16float (enables the SHADER_F16_IN_F32 downlevel capability).
            // TIMESTAMP_QUERY(+INSIDE_ENCODERS) drives the F3 GPU composite timer
            // (gpu_timer.rs); both are optional and no-op when unsupported.
            let optional_features = wgpu::Features::SHADER_F16
                | wgpu::Features::TIMESTAMP_QUERY
                | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;

            let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };

            wgpu::DeviceDescriptor {
                label: Some("lightningbeam wgpu device"),
                required_features: features & optional_features,
                required_limits: wgpu::Limits {
                    max_texture_dimension_2d: 8192,
                    ..base_limits
                },
                ..Default::default()
            }
        }),
        ..Default::default()
    })
}

fn main() -> eframe::Result {
    println!("🚀 Starting Lightningbeam Editor...");

    // Configure rayon thread pool to use fewer threads, leaving cores free for video playback
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let waveform_threads = (num_cpus.saturating_sub(2)).max(2); // Leave 2 cores free, minimum 2 threads
    rayon::ThreadPoolBuilder::new()
        .num_threads(waveform_threads)
        .thread_name(|i| format!("waveform-{}", i))
        .build_global()
        .expect("Failed to build rayon thread pool");
    println!("✅ Configured waveform generation to use {} threads (leaving {} cores for video)",
        waveform_threads, num_cpus - waveform_threads);

    // Parse command line arguments
    let args = Args::parse();

    if args.cpu_renderer {
        panes::stage::FORCE_CPU_RENDERER.store(true, std::sync::atomic::Ordering::Relaxed);
        println!("⚠️  CPU renderer forced via --cpu-renderer");
    }

    // Load config to get theme preference
    let config = AppConfig::load();

    // Determine theme mode: command-line args override config
    let theme_mode = if args.light {
        ThemeMode::Light
    } else if args.dark {
        ThemeMode::Dark
    } else {
        // Use theme from config
        ThemeMode::from_string(&config.theme_mode)
    };

    // Load theme
    let mut theme = Theme::load_default().expect("Failed to load theme");
    theme.set_mode(theme_mode);
    println!("✅ Loaded theme with {} selectors (mode: {:?})", theme.len(), theme_mode);

    // Debug: print theme info
    theme.debug_print();

    // Load layouts from JSON
    let layouts = load_layouts();
    println!("✅ Loaded {} layouts", layouts.len());
    for layout in &layouts {
        println!("   - {}: {}", layout.name, layout.description);
    }

    // Initialize native menus for macOS (app-wide, doesn't need window)
    #[cfg(target_os = "macos")]
    {
        if let Ok(menu_system) = MenuSystem::new() {
            menu_system.init_for_macos();
            println!("✅ Native macOS menus initialized");
        }
    }

    // Load window icon
    let icon_data = include_bytes!("../assets/icons/256x256.png");
    let icon_image = match image::load_from_memory(icon_data) {
        Ok(img) => {
            let rgba = img.to_rgba8();
            let (width, height) = (rgba.width(), rgba.height());
            println!("✅ Loaded window icon: {}x{}", width, height);
            Some(egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            })
        }
        Err(e) => {
            eprintln!("❌ Failed to load window icon: {}", e);
            None
        }
    };

    // When developing the mobile UI on desktop (LB_MOBILE_UI), open a phone-aspect window so
    // the shell can be exercised at a realistic size. Gating the shell itself is done at
    // render time on the flag, not on window aspect.
    let initial_size = if mobile::is_mobile_landscape_env() {
        [874.0, 402.0] // iPhone-ish landscape (LB_MOBILE_UI=2)
    } else if mobile::is_mobile_env() {
        [402.0, 874.0] // iPhone-ish portrait
    } else {
        [1920.0, 1080.0]
    };

    let mut viewport_builder = egui::ViewportBuilder::default()
        .with_inner_size(initial_size)
        .with_min_inner_size([360.0, 300.0]) // keep layouts (esp. the mobile shell) above degenerate sizes
        .with_title("Lightningbeam Editor")
        .with_app_id("lightningbeam-editor"); // Set app_id for Wayland

    if let Some(icon) = icon_image {
        viewport_builder = viewport_builder.with_icon(icon);
    }

    // Prefer the shared VAAPI-capable wgpu device on Linux: eframe + the compositor + hardware
    // video decode all run on one device, so decoded DMA-BUF frames are usable by the preview
    // compositor (decode/encode can't share textures across devices). Falls back to wgpu's normal
    // device (software video decode) when unavailable, on other platforms, or via LB_NO_SHARED_DEVICE.
    // The DrmDevice's wgpu handles are cloned into eframe (Arc-backed, keep the device alive); the
    // raw VkDevice persists with them.
    #[allow(unused_mut)]
    let mut wgpu_setup = lb_default_wgpu_setup();
    // Whether the shared VAAPI-capable device is in use — only then can decoded DMA-BUF frames be
    // imported + composited, so hardware decode is injected into the VideoManager only if true.
    #[allow(unused_assignments, unused_mut)]
    let mut shared_device_active = false;
    #[cfg(target_os = "linux")]
    if std::env::var("LB_NO_SHARED_DEVICE").is_ok() {
        println!("🖥  Shared device disabled via LB_NO_SHARED_DEVICE; default wgpu device (software video decode)");
    } else {
        match gpu_video_encoder::vk_device::create_windowed() {
            Ok(drm) => {
                println!("🖥  Using shared VAAPI-capable wgpu device (hardware video decode enabled)");
                wgpu_setup = egui_wgpu::WgpuSetup::Existing(egui_wgpu::WgpuSetupExisting {
                    instance: drm.instance.clone(),
                    adapter: drm.adapter.clone(),
                    device: drm.device.clone(),
                    queue: drm.queue.clone(),
                });
                shared_device_active = true;
            }
            Err(e) => {
                println!("🖥  Shared device unavailable ({e}); default wgpu device (software video decode)");
            }
        }
    }

    let options = eframe::NativeOptions {
        viewport: viewport_builder,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup,
            ..Default::default()
        },
        ..Default::default()
    };

    // Test mode: install panic hook for crash capture (debug builds only)
    #[cfg(debug_assertions)]
    let test_mode_panic_snapshot: std::sync::Arc<std::sync::Mutex<Option<lightningbeam_core::test_mode::TestCase>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    #[cfg(debug_assertions)]
    let test_mode_pending_event: std::sync::Arc<std::sync::Mutex<Option<lightningbeam_core::test_mode::TestEvent>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    #[cfg(debug_assertions)]
    let test_mode_is_replaying: std::sync::Arc<std::sync::atomic::AtomicBool> =
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    #[cfg(debug_assertions)]
    let test_mode_pending_geometry: std::sync::Arc<std::sync::Mutex<Option<serde_json::Value>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    #[cfg(debug_assertions)]
    let test_mode_panic_snapshot_for_app = test_mode_panic_snapshot.clone();
    #[cfg(debug_assertions)]
    let test_mode_pending_event_for_app = test_mode_pending_event.clone();
    #[cfg(debug_assertions)]
    let test_mode_is_replaying_for_app = test_mode_is_replaying.clone();
    #[cfg(debug_assertions)]
    let test_mode_pending_geometry_for_app = test_mode_pending_geometry.clone();

    #[cfg(debug_assertions)]
    {
        let panic_snapshot = test_mode_panic_snapshot.clone();
        let pending_event = test_mode_pending_event.clone();
        let is_replaying = test_mode_is_replaying.clone();
        let pending_geometry = test_mode_pending_geometry.clone();
        let test_dir = directories::ProjectDirs::from("", "", "lightningbeam")
            .map(|dirs| dirs.data_dir().join("test_cases"))
            .unwrap_or_else(|| std::path::PathBuf::from("test_cases"));

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                format!("{}", info)
            };
            let backtrace = format!("{}", std::backtrace::Backtrace::force_capture());
            test_mode::TestModeState::record_panic(&panic_snapshot, &pending_event, &is_replaying, &pending_geometry, msg, backtrace, &test_dir);
            default_hook(info);
        }));
    }

    eframe::run_native(
        "Lightningbeam Editor",
        options,
        Box::new(move |cc| {
            #[cfg(debug_assertions)]
            #[allow(unused_mut)]
            let mut app = EditorApp::new(cc, layouts, theme, test_mode_panic_snapshot_for_app, test_mode_pending_event_for_app, test_mode_is_replaying_for_app, test_mode_pending_geometry_for_app);
            #[cfg(not(debug_assertions))]
            #[allow(unused_mut)]
            let mut app = EditorApp::new(cc, layouts, theme);
            // Wire hardware video decode into the VideoManager now that the shared device exists, and
            // stash the shared device handles so the zero-copy export encoder can run on it too.
            #[cfg(target_os = "linux")]
            if shared_device_active {
                if let Some(rs) = cc.wgpu_render_state.as_ref() {
                    hw_video::install(&app.video_manager, &rs.device, &rs.adapter);
                    app.shared_device = Some((rs.device.clone(), rs.queue.clone(), rs.adapter.clone()));
                }
            }
            Ok(Box::new(app))
        }),
    )
}

fn load_layouts() -> Vec<LayoutDefinition> {
    let json = include_str!("../assets/layouts.json");
    serde_json::from_str(json).expect("Failed to parse layouts.json")
}

/// Path to a node in the layout tree (indices of children)
type NodePath = Vec<usize>;

#[derive(Default)]
struct DragState {
    is_dragging: bool,
    node_path: NodePath,
    is_horizontal: bool,
}

/// Action to perform on the layout tree
enum LayoutAction {
    SplitHorizontal(NodePath, f32), // path, percent
    SplitVertical(NodePath, f32),   // path, percent
    RemoveSplit(NodePath),
    EnterSplitPreviewHorizontal,
    EnterSplitPreviewVertical,
}

#[derive(Default)]
enum SplitPreviewMode {
    #[default]
    None,
    Active {
        is_horizontal: bool,
        hovered_pane: Option<NodePath>,
        split_percent: f32,
    },
}

/// Rasterize an embedded SVG and upload it as an egui texture
pub(crate) fn rasterize_svg(svg_data: &[u8], name: &str, render_size: u32, ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let tree = resvg::usvg::Tree::from_data(svg_data, &resvg::usvg::Options::default()).ok()?;
    let pixmap_size = tree.size().to_int_size();
    let scale_x = render_size as f32 / pixmap_size.width() as f32;
    let scale_y = render_size as f32 / pixmap_size.height() as f32;
    let scale = scale_x.min(scale_y);

    let final_size = resvg::usvg::Size::from_wh(
        pixmap_size.width() as f32 * scale,
        pixmap_size.height() as f32 * scale,
    ).unwrap_or(resvg::usvg::Size::from_wh(render_size as f32, render_size as f32).unwrap());

    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        final_size.width() as u32,
        final_size.height() as u32,
    )?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba_data = pixmap.data();
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba_data);
    Some(ctx.load_texture(name, color_image, egui::TextureOptions::LINEAR))
}

/// Embedded pane icon SVGs
mod pane_icons {
    pub static STAGE: &[u8] = include_bytes!("../../../src/assets/stage.svg");
    pub static TIMELINE: &[u8] = include_bytes!("../../../src/assets/timeline.svg");
    pub static TOOLBAR: &[u8] = include_bytes!("../../../src/assets/toolbar.svg");
    pub static INFOPANEL: &[u8] = include_bytes!("../../../src/assets/infopanel.svg");
    pub static PIANO_ROLL: &[u8] = include_bytes!("../../../src/assets/piano-roll.svg");
    pub static PIANO: &[u8] = include_bytes!("../../../src/assets/piano.svg");
    pub static NODE_EDITOR: &[u8] = include_bytes!("../../../src/assets/node-editor.svg");
}

/// Embedded tool icon SVGs
mod tool_icons {
    pub static SELECT: &[u8] = include_bytes!("../../../src/assets/select.svg");
    pub static DRAW: &[u8] = include_bytes!("../../../src/assets/draw.svg");
    pub static TRANSFORM: &[u8] = include_bytes!("../../../src/assets/transform.svg");
    pub static RECTANGLE: &[u8] = include_bytes!("../../../src/assets/rectangle.svg");
    pub static ELLIPSE: &[u8] = include_bytes!("../../../src/assets/ellipse.svg");
    pub static PAINT_BUCKET: &[u8] = include_bytes!("../../../src/assets/paint_bucket.svg");
    pub static EYEDROPPER: &[u8] = include_bytes!("../../../src/assets/eyedropper.svg");
    pub static LINE: &[u8] = include_bytes!("../../../src/assets/line.svg");
    pub static POLYGON: &[u8] = include_bytes!("../../../src/assets/polygon.svg");
    pub static BEZIER_EDIT: &[u8] = include_bytes!("../../../src/assets/bezier_edit.svg");
    pub static TEXT: &[u8] = include_bytes!("../../../src/assets/text.svg");
    pub static SPLIT: &[u8] = include_bytes!("../../../src/assets/split.svg");
    pub static ERASE: &[u8] = include_bytes!("../../../src/assets/erase.svg");
    pub static SMUDGE: &[u8] = include_bytes!("../../../src/assets/smudge.svg");
    pub static LASSO: &[u8] = include_bytes!("../../../src/assets/lasso.svg");
    pub static TODO: &[u8] = include_bytes!("../../../src/assets/todo.svg");
}

/// Embedded focus icon SVGs
mod focus_icons {
    pub static ANIMATION: &[u8] = include_bytes!("../../../src/assets/focus-animation.svg");
    pub static MUSIC: &[u8] = include_bytes!("../../../src/assets/focus-music.svg");
    pub static VIDEO: &[u8] = include_bytes!("../../../src/assets/focus-video.svg");
    pub static PAINTING: &[u8] = include_bytes!("../../../src/assets/focus-painting.svg");
}

/// Icon cache for pane type icons
struct IconCache {
    icons: HashMap<PaneType, egui::TextureHandle>,
}

impl IconCache {
    fn new() -> Self {
        Self {
            icons: HashMap::new(),
        }
    }

    fn get_or_load(&mut self, pane_type: PaneType, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if !self.icons.contains_key(&pane_type) {
            let svg_data = match pane_type {
                PaneType::Stage | PaneType::Outliner | PaneType::PresetBrowser | PaneType::AssetLibrary => pane_icons::STAGE,
                PaneType::Timeline => pane_icons::TIMELINE,
                PaneType::Toolbar => pane_icons::TOOLBAR,
                PaneType::Infopanel => pane_icons::INFOPANEL,
                PaneType::PianoRoll => pane_icons::PIANO_ROLL,
                PaneType::VirtualPiano => pane_icons::PIANO,
                PaneType::NodeEditor | PaneType::ScriptEditor => pane_icons::NODE_EDITOR,
            };
            if let Some(texture) = rasterize_svg(svg_data, pane_type.icon_file(), 64, ctx) {
                self.icons.insert(pane_type, texture);
            }
        }
        self.icons.get(&pane_type)
    }
}

/// Icon cache for tool icons
struct ToolIconCache {
    icons: HashMap<Tool, egui::TextureHandle>,
}

impl ToolIconCache {
    fn new() -> Self {
        Self {
            icons: HashMap::new(),
        }
    }

    fn get_or_load(&mut self, tool: Tool, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if !self.icons.contains_key(&tool) {
            let svg_data = match tool {
                Tool::Select => tool_icons::SELECT,
                Tool::Draw => tool_icons::DRAW,
                Tool::Transform => tool_icons::TRANSFORM,
                Tool::Rectangle => tool_icons::RECTANGLE,
                Tool::Ellipse => tool_icons::ELLIPSE,
                Tool::PaintBucket => tool_icons::PAINT_BUCKET,
                Tool::Eyedropper => tool_icons::EYEDROPPER,
                Tool::Line => tool_icons::LINE,
                Tool::Polygon => tool_icons::POLYGON,
                Tool::BezierEdit => tool_icons::BEZIER_EDIT,
                Tool::Text => tool_icons::TEXT,
                Tool::RegionSelect => tool_icons::SELECT,
                Tool::Split => tool_icons::SPLIT,
                Tool::Erase => tool_icons::ERASE,
                Tool::Smudge => tool_icons::SMUDGE,
                Tool::SelectLasso => tool_icons::LASSO,
                // Not yet implemented — use placeholder icon
                Tool::Pencil
                | Tool::Pen
                | Tool::Airbrush
                | Tool::CloneStamp
                | Tool::HealingBrush
                | Tool::PatternStamp
                | Tool::DodgeBurn
                | Tool::Sponge
                | Tool::BlurSharpen
                | Tool::Gradient
                | Tool::CustomShape
                | Tool::SelectEllipse
                | Tool::MagicWand
                | Tool::QuickSelect
                | Tool::Warp
                | Tool::Liquify => tool_icons::TODO,
            };
            if let Some(texture) = rasterize_svg(svg_data, tool.icon_file(), 180, ctx) {
                self.icons.insert(tool, texture);
            }
        }
        self.icons.get(&tool)
    }
}

/// Icon cache for focus card icons (start screen)
struct FocusIconCache {
    icons: HashMap<FocusIcon, egui::TextureHandle>,
}

impl FocusIconCache {
    fn new() -> Self {
        Self {
            icons: HashMap::new(),
        }
    }

    fn get_or_load(&mut self, icon: FocusIcon, icon_color: egui::Color32, display_size: f32, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if !self.icons.contains_key(&icon) {
            let (svg_bytes, svg_filename) = match icon {
                FocusIcon::Animation => (focus_icons::ANIMATION, "focus-animation.svg"),
                FocusIcon::Music => (focus_icons::MUSIC, "focus-music.svg"),
                FocusIcon::Video => (focus_icons::VIDEO, "focus-video.svg"),
                FocusIcon::Painting => (focus_icons::PAINTING, "focus-painting.svg"),
            };

            // Replace currentColor with the actual color
            let svg_data = String::from_utf8_lossy(svg_bytes);
            let color_hex = format!(
                "#{:02x}{:02x}{:02x}",
                icon_color.r(), icon_color.g(), icon_color.b()
            );
            let svg_with_color = svg_data.replace("currentColor", &color_hex);

            let render_size = (display_size * ctx.pixels_per_point()).ceil() as u32;
            if let Some(texture) = rasterize_svg(svg_with_color.as_bytes(), svg_filename, render_size, ctx) {
                self.icons.insert(icon, texture);
            }
        }
        self.icons.get(&icon)
    }
}

/// Serialize a clip's thumbnails to an `LBTN` blob (version 2): a `complete` flag
/// then each frame PNG-encoded and packed with its timestamp. Thumbnails are 128px
/// wide; height is implied by the PNG. `complete` records whether the generating
/// pass finished — a partial pack is persisted and resumed on load. Frames that
/// fail to encode are skipped (the count reflects what's written).
fn encode_thumbnail_blob(thumbs: &[(f64, std::sync::Arc<Vec<u8>>)], complete: bool) -> Vec<u8> {
    let mut entries: Vec<(f64, Vec<u8>)> = Vec::with_capacity(thumbs.len());
    for (ts, rgba) in thumbs {
        let width = 128u32;
        let height = (rgba.len() / (width as usize * 4)) as u32;
        if height == 0 {
            continue;
        }
        let img = lightningbeam_core::brush_engine::image_from_raw((**rgba).clone(), width, height);
        if let Ok(png) = lightningbeam_core::brush_engine::encode_png(&img) {
            entries.push((*ts, png));
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"LBTN");
    out.extend_from_slice(&2u32.to_le_bytes()); // version
    out.push(if complete { 1 } else { 0 });
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for (ts, png) in &entries {
        out.extend_from_slice(&ts.to_le_bytes());
        out.extend_from_slice(&(png.len() as u32).to_le_bytes());
        out.extend_from_slice(png);
    }
    out
}

/// Read just the `complete` flag from an `LBTN` blob (cheap header read, no PNG
/// decode). Unknown/old versions are treated as incomplete (→ regenerate).
fn thumbnail_blob_is_complete(blob: &[u8]) -> bool {
    blob.len() >= 9
        && &blob[0..4] == b"LBTN"
        && u32::from_le_bytes(blob[4..8].try_into().unwrap()) == 2
        && blob[8] == 1
}

/// Decode an `LBTN` blob into `(timestamp, rgba)` thumbnails (128px wide). Returns
/// empty for unknown/old versions.
fn decode_thumbnail_blob(blob: &[u8]) -> Vec<(f64, std::sync::Arc<Vec<u8>>)> {
    let mut out = Vec::new();
    if blob.len() < 13 || &blob[0..4] != b"LBTN" {
        return out;
    }
    if u32::from_le_bytes(blob[4..8].try_into().unwrap()) != 2 {
        return out;
    }
    // blob[8] = complete flag (read separately via thumbnail_blob_is_complete)
    let count = u32::from_le_bytes(blob[9..13].try_into().unwrap()) as usize;
    let mut pos = 13;
    for _ in 0..count {
        if pos + 12 > blob.len() {
            break;
        }
        let ts = f64::from_le_bytes(blob[pos..pos + 8].try_into().unwrap());
        pos += 8;
        let png_len = u32::from_le_bytes(blob[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + png_len > blob.len() {
            break;
        }
        if let Ok(img) = lightningbeam_core::brush_engine::decode_png(&blob[pos..pos + png_len]) {
            out.push((ts, std::sync::Arc::new(img.into_raw())));
        }
        pos += png_len;
    }
    out
}

/// Command sent to file operations worker thread
enum FileCommand {
    Save {
        path: std::path::PathBuf,
        document: lightningbeam_core::document::Document,
        layer_to_track_map: std::collections::HashMap<uuid::Uuid, u32>,
        /// How to store large media on this save (from the user's preference).
        large_media_mode: lightningbeam_core::file_io::LargeMediaMode,
        /// Serialized waveform-pyramid blobs per audio pool index, persisted into
        /// the container so a later load needn't re-decode the source media.
        waveform_blobs: std::collections::HashMap<usize, Vec<u8>>,
        /// Raw video thumbnails by clip id (cheap Arc-clone snapshot). The worker
        /// PNG-encodes them off the UI thread, then persists them in the container.
        thumbnail_snapshot: std::collections::HashMap<uuid::Uuid, Vec<(f64, std::sync::Arc<Vec<u8>>)>>,
        /// Clips whose thumbnail generation has finished — partial sets are still
        /// persisted (and resumed on load), but flagged incomplete.
        complete_thumbnail_clips: std::collections::HashSet<uuid::Uuid>,
        progress_tx: std::sync::mpsc::Sender<FileProgress>,
    },
    Load {
        path: std::path::PathBuf,
        progress_tx: std::sync::mpsc::Sender<FileProgress>,
    },
}

/// Progress updates from file operations worker
#[allow(dead_code)] // EncodingAudio/DecodingAudio planned for granular progress reporting
enum FileProgress {
    SerializingAudioPool,
    EncodingAudio { current: usize, total: usize },
    WritingZip,
    LoadingProject,
    DecodingAudio { current: usize, total: usize },
    Complete(Result<lightningbeam_core::file_io::LoadedProject, String>), // For loading
    Error(String),
    Done,
}

/// Active file operation state
enum FileOperation {
    Saving {
        path: std::path::PathBuf,
        progress_rx: std::sync::mpsc::Receiver<FileProgress>,
    },
    Loading {
        path: std::path::PathBuf,
        progress_rx: std::sync::mpsc::Receiver<FileProgress>,
    },
}

/// How often (seconds) the background autosave runs when the document is dirty.
const AUTOSAVE_INTERVAL_SECS: f64 = 45.0;

/// Background crash-recovery autosave state (see the `autosave` field on `EditorApp`).
struct AutosaveState {
    /// Per-session recovery container path (in the app data dir). `None` disables autosave
    /// (e.g. the data dir couldn't be resolved/created).
    recovery_path: Option<std::path::PathBuf>,
    /// `ActionExecutor::epoch()` captured at the last dispatched autosave (or manual save). The
    /// document is "dirty" when the current epoch differs, or `pending_event` is set.
    baseline_epoch: u64,
    /// Set by non-action changes that still need capturing (import, finished recording).
    pending_event: bool,
    /// True while a recovery write is in flight on the worker (don't dispatch another).
    in_flight: bool,
    /// Wall-clock of the last dispatched autosave, for interval throttling.
    last_time: Option<std::time::Instant>,
    /// Progress channel for the in-flight recovery write.
    progress_rx: Option<std::sync::mpsc::Receiver<FileProgress>>,
    /// Recovery files left over from previous sessions that didn't exit cleanly (newest first),
    /// discovered at startup. Presented to the user as a "recover unsaved work?" prompt.
    leftover_recoveries: Vec<std::path::PathBuf>,
    /// Seconds between autosaves while dirty. Defaults to `AUTOSAVE_INTERVAL_SECS`; override with
    /// `LB_AUTOSAVE_SECS` (e.g. `LB_AUTOSAVE_SECS=5`) to make manual testing practical.
    interval_secs: f64,
}

impl AutosaveState {
    fn new() -> Self {
        Self::gc_old_recovered();
        let recovery_path = Self::make_session_path();
        eprintln!("💾 [AUTOSAVE] recovery file for this session: {:?}", recovery_path);
        let leftover_recoveries = Self::find_leftovers(recovery_path.as_deref());
        if !leftover_recoveries.is_empty() {
            eprintln!("💾 [AUTOSAVE] found {} leftover recovery file(s) from a previous session",
                leftover_recoveries.len());
        }
        Self {
            recovery_path,
            baseline_epoch: 0,
            pending_event: false,
            in_flight: false,
            last_time: None,
            progress_rx: None,
            leftover_recoveries,
            interval_secs: std::env::var("LB_AUTOSAVE_SECS")
                .ok()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|s| *s > 0.0)
                .unwrap_or(AUTOSAVE_INTERVAL_SECS),
        }
    }

    /// Session recovery `.beam` files present at startup (excluding this session's own path).
    /// Their existence means a prior session didn't reach `on_exit` — i.e. it crashed or was
    /// killed — so they hold unsaved work. Sorted newest-first by modification time.
    fn find_leftovers(exclude: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
        let Some(dir) = Self::recovery_dir() else { return Vec::new() };
        let mut files: Vec<(std::path::PathBuf, std::time::SystemTime)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|x| x.to_str()) == Some("beam")
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("session-"))
                    && Some(p.as_path()) != exclude
            })
            .filter_map(|p| {
                let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
                Some((p, mtime))
            })
            .collect();
        files.sort_by(|a, b| b.1.cmp(&a.1)); // newest first
        files.into_iter().map(|(p, _)| p).collect()
    }

    /// Delete `recovered-*` files (already-recovered work the user relocated via Save As) older than
    /// a week, so the recovery dir doesn't grow without bound. Best-effort.
    fn gc_old_recovered() {
        let Some(dir) = Self::recovery_dir() else { return };
        let Some(cutoff) =
            std::time::SystemTime::now().checked_sub(std::time::Duration::from_secs(7 * 24 * 3600))
        else {
            return;
        };
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = entry.path();
            let is_recovered = p.extension().and_then(|x| x.to_str()) == Some("beam")
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("recovered-"));
            if !is_recovered {
                continue;
            }
            if let Ok(mtime) = std::fs::metadata(&p).and_then(|m| m.modified()) {
                if mtime < cutoff {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }

    /// The recovery directory (`<data_dir>/recovery`), created if needed.
    fn recovery_dir() -> Option<std::path::PathBuf> {
        let proj = directories::ProjectDirs::from("", "", "lightningbeam")?;
        let dir = proj.data_dir().join("recovery");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// A fresh per-session recovery file path (`recovery/session-<uuid>.beam`).
    fn make_session_path() -> Option<std::path::PathBuf> {
        Some(Self::recovery_dir()?.join(format!("session-{}.beam", uuid::Uuid::new_v4())))
    }

    /// Whether `path` is one of our recovery files (`session-*` / `recovered-*` in the recovery
    /// dir). Saving one of these should behave as Save As so the work gets a real home instead of
    /// being written back into the app data dir.
    fn is_recovery_path(path: &std::path::Path) -> bool {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("session-") || n.starts_with("recovered-"))
    }
}

/// Something to do after the unsaved-changes prompt resolves: the action deferred behind "Save
/// changes?" (Save runs it after saving; Don't Save runs it immediately; Cancel drops it). Covers
/// both file switches and quitting so they share one prompt + one save-then-continue path.
#[derive(Debug, Clone)]
enum PendingAction {
    /// New File (return to the start screen).
    NewFile,
    /// Open a specific `.beam` (covers both Open… and Open Recent — the path is already resolved).
    Open(std::path::PathBuf),
    /// Quit (close the window).
    Quit,
}

/// Information about an imported asset (for auto-placement)
#[derive(Debug, Clone)]
#[allow(dead_code)] // name/duration populated for future import UX features
struct ImportedAssetInfo {
    clip_id: uuid::Uuid,
    clip_type: panes::DragClipType,
    name: String,
    dimensions: Option<(f64, f64)>,
    duration: f64,
    linked_audio_clip_id: Option<uuid::Uuid>,
}

/// Worker thread for file operations (save/load)
struct FileOperationsWorker {
    command_rx: std::sync::mpsc::Receiver<FileCommand>,
    audio_controller: std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>,
}

impl FileOperationsWorker {
    /// Create a new worker and spawn it on a background thread
    fn spawn(audio_controller: std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>)
        -> std::sync::mpsc::Sender<FileCommand>
    {
        let (command_tx, command_rx) = std::sync::mpsc::channel();

        let worker = FileOperationsWorker {
            command_rx,
            audio_controller,
        };

        std::thread::spawn(move || {
            worker.run();
        });

        command_tx
    }

    /// Main worker loop - processes file commands
    fn run(self) {
        while let Ok(command) = self.command_rx.recv() {
            match command {
                FileCommand::Save { path, document, layer_to_track_map, large_media_mode, waveform_blobs, thumbnail_snapshot, complete_thumbnail_clips, progress_tx } => {
                    self.handle_save(path, document, &layer_to_track_map, large_media_mode, waveform_blobs, thumbnail_snapshot, complete_thumbnail_clips, progress_tx);
                }
                FileCommand::Load { path, progress_tx } => {
                    self.handle_load(path, progress_tx);
                }
            }
        }
    }

    /// Handle save command
    fn handle_save(
        &self,
        path: std::path::PathBuf,
        document: lightningbeam_core::document::Document,
        layer_to_track_map: &std::collections::HashMap<uuid::Uuid, u32>,
        large_media_mode: lightningbeam_core::file_io::LargeMediaMode,
        waveform_blobs: std::collections::HashMap<usize, Vec<u8>>,
        thumbnail_snapshot: std::collections::HashMap<uuid::Uuid, Vec<(f64, std::sync::Arc<Vec<u8>>)>>,
        complete_thumbnail_clips: std::collections::HashSet<uuid::Uuid>,
        progress_tx: std::sync::mpsc::Sender<FileProgress>,
    ) {
        use lightningbeam_core::file_io::{save_beam, SaveSettings};

        // PNG-encode the thumbnail snapshot into per-clip LBTN blobs (off the UI
        // thread), flagging each pack complete-or-partial so the load can resume.
        let thumbnail_blobs: std::collections::HashMap<uuid::Uuid, Vec<u8>> = thumbnail_snapshot
            .iter()
            .filter(|(_, thumbs)| !thumbs.is_empty())
            .map(|(clip_id, thumbs)| {
                let complete = complete_thumbnail_clips.contains(clip_id);
                (*clip_id, encode_thumbnail_blob(thumbs, complete))
            })
            .collect();

        let save_start = std::time::Instant::now();
        eprintln!("📊 [SAVE] Starting save operation...");

        // Step 1: Serialize audio pool
        let _ = progress_tx.send(FileProgress::SerializingAudioPool);
        let step1_start = std::time::Instant::now();

        let mut audio_pool_entries = {
            let mut controller = self.audio_controller.lock().unwrap();
            match controller.serialize_audio_pool(&path) {
                Ok(entries) => entries,
                Err(e) => {
                    let _ = progress_tx.send(FileProgress::Error(format!("Failed to serialize audio pool: {}", e)));
                    return;
                }
            }
        };
        // Attach precomputed waveform pyramids so save_beam can persist them
        // (keyed by pool index inside the container).
        for entry in &mut audio_pool_entries {
            if let Some(blob) = waveform_blobs.get(&entry.pool_index) {
                entry.waveform_blob = Some(blob.clone());
            }
        }
        eprintln!("📊 [SAVE] Step 1: Serialize audio pool took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);

        // Step 2: Get project
        let step2_start = std::time::Instant::now();
        let mut audio_project = {
            let mut controller = self.audio_controller.lock().unwrap();
            match controller.get_project() {
                Ok(p) => p,
                Err(e) => {
                    let _ = progress_tx.send(FileProgress::Error(format!("Failed to get project: {}", e)));
                    return;
                }
            }
        };
        eprintln!("📊 [SAVE] Step 2: Get audio project took {:.2}ms", step2_start.elapsed().as_secs_f64() * 1000.0);

        // Step 3: Save to file
        let _ = progress_tx.send(FileProgress::WritingZip);
        let step3_start = std::time::Instant::now();

        let settings = SaveSettings {
            large_media_mode,
        };
        match save_beam(&path, &document, &mut audio_project, audio_pool_entries, layer_to_track_map, &thumbnail_blobs, &settings) {
            Ok(()) => {
                eprintln!("📊 [SAVE] Step 3: save_beam() took {:.2}ms", step3_start.elapsed().as_secs_f64() * 1000.0);
                eprintln!("📊 [SAVE] ✅ Total save time: {:.2}ms", save_start.elapsed().as_secs_f64() * 1000.0);
                println!("✅ Saved to: {}", path.display());
                let _ = progress_tx.send(FileProgress::Done);
            }
            Err(e) => {
                let _ = progress_tx.send(FileProgress::Error(format!("Save failed: {}", e)));
            }
        }
    }

    /// Handle load command
    fn handle_load(
        &self,
        path: std::path::PathBuf,
        progress_tx: std::sync::mpsc::Sender<FileProgress>,
    ) {
        use lightningbeam_core::file_io::load_beam;

        let load_start = std::time::Instant::now();
        eprintln!("📊 [LOAD] Starting load operation...");

        // Step 1: Load from file
        let _ = progress_tx.send(FileProgress::LoadingProject);
        let step1_start = std::time::Instant::now();

        let loaded_project = match load_beam(&path) {
            Ok(p) => p,
            Err(e) => {
                let _ = progress_tx.send(FileProgress::Error(format!("Load failed: {}", e)));
                return;
            }
        };
        eprintln!("📊 [LOAD] Step 1: load_beam() took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);

        // Check for missing files
        if !loaded_project.missing_files.is_empty() {
            eprintln!("⚠️ {} missing files", loaded_project.missing_files.len());
            for missing in &loaded_project.missing_files {
                eprintln!("   - {}", missing.original_path.display());
            }
        }

        eprintln!("📊 [LOAD] ✅ Total load time: {:.2}ms", load_start.elapsed().as_secs_f64() * 1000.0);

        // Send the loaded project back to UI thread for processing
        let _ = progress_tx.send(FileProgress::Complete(Ok(loaded_project)));
    }
}

/// Result from background audio extraction thread
#[derive(Debug)]
enum AudioExtractionResult {
    Success {
        video_clip_id: Uuid,
        audio_clip: lightningbeam_core::clip::AudioClip,
        pool_index: usize,
        video_name: String,
        channels: u32,
        sample_rate: u32,
    },
    Error {
        video_clip_id: Uuid,
        error: String,
    },
}

/// Application mode - controls whether to show start screen or editor
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    /// Show the start screen (recent projects, new project options)
    StartScreen,
    /// Show the main editor interface
    Editor,
}

/// Icons for the focus cards on the start screen
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum FocusIcon {
    Animation,
    Music,
    Video,
    Painting,
}

/// Recording arm mode - determines how tracks are armed for recording
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum RecordingArmMode {
    /// Armed state follows active track (simple single-track workflow)
    #[default]
    Auto,
    /// User explicitly arms tracks (multi-track recording workflow)
    #[allow(dead_code)]
    Manual,
}

/// Entry in the editing context stack — tracks which clip is being edited
#[derive(Clone)]
struct EditingContextEntry {
    /// The VectorClip ID being edited
    clip_id: Uuid,
    /// The ClipInstance ID through which we entered
    instance_id: Uuid,
    /// The layer ID that contains the instance in the parent context
    parent_layer_id: Uuid,
    /// Saved playback time from the parent context (restored on exit)
    saved_playback_time: f64,
    /// Saved active layer ID from the parent context
    saved_active_layer_id: Option<Uuid>,
}

/// Editing context stack — tracks which clip (or root) is being edited.
/// Empty stack = editing the document root.
#[derive(Clone, Default)]
struct EditingContext {
    stack: Vec<EditingContextEntry>,
}

impl EditingContext {
    fn current_clip_id(&self) -> Option<Uuid> {
        self.stack.last().map(|e| e.clip_id)
    }

    fn current_instance_id(&self) -> Option<Uuid> {
        self.stack.last().map(|e| e.instance_id)
    }

    fn current_parent_layer_id(&self) -> Option<Uuid> {
        self.stack.last().map(|e| e.parent_layer_id)
    }

    fn push(&mut self, entry: EditingContextEntry) {
        self.stack.push(entry);
    }

    fn pop(&mut self) -> Option<EditingContextEntry> {
        self.stack.pop()
    }

    /// The clip_id path from outermost to the current level (for breadcrumbs).
    fn clip_path(&self) -> Vec<Uuid> {
        self.stack.iter().map(|e| e.clip_id).collect()
    }

    /// Pop down to `depth` entries, returning the entry at `depth` (whose saved state should be
    /// restored). Returns None if already at or above that depth.
    fn exit_to_depth(&mut self, depth: usize) -> Option<EditingContextEntry> {
        if depth >= self.stack.len() {
            return None;
        }
        let restore = self.stack[depth].clone();
        self.stack.truncate(depth);
        Some(restore)
    }
}

struct EditorApp {
    layouts: Vec<LayoutDefinition>,
    current_layout_index: usize,
    current_layout: LayoutNode, // Mutable copy for editing
    drag_state: DragState,
    hovered_divider: Option<(NodePath, bool)>, // (path, is_horizontal)
    selected_pane: Option<NodePath>, // Currently selected pane for editing
    split_preview_mode: SplitPreviewMode,
    // === Mobile / phone UI (developed on desktop behind LB_MOBILE_UI) ===
    /// True if the mobile shell is requested via the env var (read once at startup).
    mobile_ui: bool,
    /// Runtime override of the mobile flag (None = follow `mobile_ui`).
    mobile_ui_override: Option<bool>,
    /// Persistent state for the mobile shell (active surface, ribbon tier).
    mobile_state: mobile::MobileState,
    /// Active mobile long-press context menu (set by a pane, rendered by the shell).
    mobile_context_menu: Option<panes::MobileContextMenu>,
    /// Shared keyboard octave offset (C4-relative) for the mobile Virtual Piano + Piano Roll.
    keyboard_octave: i8,
    /// Shared horizontal keyboard pan (px) for the mobile keyboard + roll.
    keyboard_pan_x: f32,
    /// Mobile: request to open the instrument Preset Browser (set by the music pane header).
    open_instrument_browser: bool,
    /// Mobile: request to toggle recording (set by the music pane REC button).
    pending_record_toggle: bool,
    icon_cache: IconCache,
    tool_icon_cache: ToolIconCache,
    focus_icon_cache: FocusIconCache, // Focus card icons (start screen)
    selected_tool: Tool, // Currently selected drawing tool
    fill_color: egui::Color32, // Fill color for drawing
    stroke_color: egui::Color32, // Stroke color for drawing
    active_color_mode: panes::ColorMode, // Which color (fill/stroke) was last interacted with
    pane_instances: HashMap<NodePath, PaneInstance>, // Pane instances per path
    menu_system: Option<MenuSystem>, // Native menu system for event checking
    pending_view_action: Option<MenuAction>, // Pending view action (zoom, recenter) to be handled by hovered pane
    theme: Theme, // Theme system for colors and dimensions
    action_executor: lightningbeam_core::action::ActionExecutor, // Action system for undo/redo
    active_layer_id: Option<Uuid>, // Currently active layer for editing
    selection: lightningbeam_core::selection::Selection, // Current selection state
    focus: lightningbeam_core::selection::FocusSelection, // Document-level focus tracking
    editing_context: EditingContext, // Which clip (or root) we're editing
    tool_state: lightningbeam_core::tool::ToolState, // Current tool interaction state
    // Draw tool configuration
    draw_simplify_mode: lightningbeam_core::tool::SimplifyMode, // Current simplification mode for draw tool
    rdp_tolerance: f64, // RDP simplification tolerance (default: 10.0)
    schneider_max_error: f64, // Schneider curve fitting max error (default: 30.0)
    /// All per-tool raster paint settings (brush, eraser, smudge, clone, pattern, dodge/burn, sponge).
    raster_settings: tools::RasterToolSettings,
    /// GPU-rendered brush preview pixel buffers, shared with VelloCallback::prepare().
    brush_preview_pixels: std::sync::Arc<std::sync::Mutex<Vec<(u32, u32, Vec<u8>)>>>,
    // Audio engine integration
    #[allow(dead_code)] // Must be kept alive to maintain audio output
    audio_stream: Option<cpal::Stream>,
    audio_controller: Option<std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>>,
    /// Holds `input_tx` and device info needed to open the microphone stream on
    /// demand (when the user selects an audio input track).
    audio_input: Option<daw_backend::InputStreamOpener>,
    /// Active microphone/line-in stream; kept alive while an audio input track is selected.
    #[allow(dead_code)]
    audio_input_stream: Option<cpal::Stream>,
    audio_buffer_size: u32,
    audio_event_rx: Option<rtrb::Consumer<daw_backend::AudioEvent>>,
    last_input_monitoring: bool,
    /// Count of in-flight graph preset loads — keeps the repaint loop alive
    /// until the audio thread sends GraphPresetLoaded events for all of them
    pending_graph_loads: std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Set by raster select tools when a new interaction requires committing the floating selection
    commit_raster_floating_if_any: bool,
    /// Set by MenuAction::Group when focus is Nodes — consumed by node graph pane next frame
    pending_node_group: bool,
    /// Set by MenuAction::Ungroup when focus is Nodes — consumed by node graph pane next frame
    pending_node_ungroup: bool,
    #[allow(dead_code)] // Stored for future export/recording configuration
    audio_sample_rate: u32,
    #[allow(dead_code)]
    audio_channels: u32,
    // Video decoding and management
    video_manager: std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>, // Shared video manager
    /// The shared VAAPI-capable wgpu device (device, queue, adapter), `Some` only when active. Lets
    /// the zero-copy export encoder run on it (GPU-resident decode→composite→encode).
    shared_device: Option<(wgpu::Device, wgpu::Queue, wgpu::Adapter)>,
    // Webcam capture state
    webcam: Option<lightningbeam_core::webcam::WebcamCapture>,
    /// Latest polled webcam frame (updated each frame for preview)
    webcam_frame: Option<lightningbeam_core::webcam::CaptureFrame>,
    /// Pending webcam recording command (set by timeline, processed in update)
    webcam_record_command: Option<panes::WebcamRecordCommand>,
    // Track ID mapping (Document layer UUIDs <-> daw-backend TrackIds)
    layer_to_track_map: HashMap<Uuid, daw_backend::TrackId>,
    track_to_layer_map: HashMap<daw_backend::TrackId, Uuid>,
    /// Generation counter - incremented on project load to force UI components to reload
    project_generation: u64,
    /// Incremented whenever node graph topology changes (add/remove node or connection)
    graph_topology_generation: u64,
    // Clip instance ID mapping (Document clip instance UUIDs <-> backend clip instance IDs)
    clip_instance_to_backend_map: HashMap<Uuid, lightningbeam_core::action::BackendClipInstanceId>,
    // Playback state (global for all panes)
    playback_time: f64, // Current playback position in seconds (persistent - save with document)
    is_playing: bool,   // Whether playback is currently active (transient - don't save)
    // Recording state
    #[allow(dead_code)] // Infrastructure for Manual recording mode
    recording_arm_mode: RecordingArmMode,
    #[allow(dead_code)]
    armed_layers: HashSet<Uuid>,
    is_recording: bool,                   // Whether recording is currently active
    metronome_enabled: bool,              // Whether metronome clicks during recording
    count_in_enabled: bool,              // Whether count-in fires before recording
    recording_clips: HashMap<Uuid, u32>,  // layer_id -> backend clip_id during recording
    recording_start_time: f64,            // Playback time when recording started
    recording_layer_ids: Vec<Uuid>,       // Layers being recorded to (for creating clips)
    // Asset drag-and-drop state
    dragging_asset: Option<panes::DraggingAsset>, // Asset being dragged from Asset Library
    // Clipboard
    clipboard_manager: lightningbeam_core::clipboard::ClipboardManager,
    // Script editor inter-pane communication
    effect_to_load: Option<Uuid>, // Effect ID to load into shader editor (set by asset library)
    script_to_edit: Option<Uuid>, // Script ID to open in editor (set by node graph)
    script_saved: Option<Uuid>, // Script ID just saved (triggers auto-recompile)
    // Effect thumbnail invalidation queue (persists across frames until processed)
    effect_thumbnails_to_invalidate: Vec<Uuid>,
    // Import dialog state
    last_import_filter: ImportFilter, // Last used import filter (remembered across imports)
    // Tool-specific options (displayed in infopanel)
    stroke_width: f64,               // Stroke width for drawing tools (default: 3.0)
    fill_enabled: bool,              // Whether to fill shapes (default: true)
    snap_enabled: bool,              // Whether to snap to geometry (default: true)
    paint_bucket_gap_tolerance: f64, // Fill gap tolerance for paint bucket (default: 5.0)
    polygon_sides: u32,              // Number of sides for polygon tool (default: 5)
    /// Undo depth before the current region-select session's first cut (see
    /// `resolve_pending_region_cut`). `None` when no region selection is pending.
    pending_region_cut_base: Option<usize>,
    region_select_mode: lightningbeam_core::tool::RegionSelectMode,
    lasso_mode: lightningbeam_core::tool::LassoMode,

    // VU meter levels
    input_level: f32,
    output_level: (f32, f32),
    track_levels: HashMap<daw_backend::TrackId, f32>,

    /// Cache for MIDI event data (keyed by backend midi_clip_id).
    /// Stores full raw MidiEvents (note on/off, CC, pitch bend, etc.)
    midi_event_cache: HashMap<u32, Vec<daw_backend::audio::midi::MidiEvent>>,
    /// Cache for audio file durations to avoid repeated queries
    /// Format: pool_index -> duration in seconds
    audio_duration_cache: HashMap<usize, f64>,
    /// Track which audio pool indices got new raw audio data this frame (for thumbnail invalidation)
    audio_pools_with_new_waveforms: HashSet<usize>,
    /// Raw audio sample cache for GPU waveform rendering
    /// Format: pool_index -> (samples, sample_rate, channels)
    raw_audio_cache: HashMap<usize, (Arc<Vec<f32>>, u32, u32)>,
    /// Pool indices that need GPU texture upload (set when raw audio arrives, cleared after upload)
    waveform_gpu_dirty: HashSet<usize>,
    /// Pool indices whose `raw_audio_cache` entry holds a packed min/max floor
    /// (4 f32 per texel: Lmin,Lmax,Rmin,Rmax) rather than raw per-sample data.
    /// Maps pool_index -> `B` (floor frames-per-texel), so the timeline render can
    /// derive the floor's effective rate `sr/B` with full float precision. Populated
    /// for streamed video-audio, which has no in-RAM samples to draw per-sample.
    waveform_minmax_pools: HashMap<usize, u32>,
    /// Serialized waveform-pyramid blobs (LBWF bytes) per pool, kept so a save can
    /// persist them into the `.beam` container without re-decoding. Populated on
    /// generation and on load. See `daw_backend::audio::waveform_pyramid::to_bytes`.
    waveform_pyramid_blobs: HashMap<usize, Arc<Vec<u8>>>,
    /// Receives generated waveforms from a background thread:
    /// (pool_index, packed_floor_texels, source_sample_rate, channels, B, pyramid_blob).
    waveform_result_rx: std::sync::mpsc::Receiver<(usize, Vec<f32>, u32, u32, u32, Vec<u8>)>,
    /// Sender handed to background waveform-pyramid generation threads.
    waveform_result_tx: std::sync::mpsc::Sender<(usize, Vec<f32>, u32, u32, u32, Vec<u8>)>,
    /// Consumer for recording audio mirror (streams recorded samples to UI for live waveform)
    recording_mirror_rx: Option<rtrb::Consumer<f32>>,
    /// Current file path (None if not yet saved)
    current_file_path: Option<std::path::PathBuf>,
    /// Onion-skinning view settings (toggle + frame counts + opacity).
    onion_skin: crate::panes::OnionSkinSettings,
    /// On-demand loader for raster keyframe pixels from the project `.beam` container.
    raster_store: lightningbeam_core::raster_store::RasterStore,
    /// Miss-sink: raster keyframe ids the canvas wanted but whose pixels weren't
    /// resident. Drained and faulted in at the top of the next `update()`.
    raster_fault_requests:
        std::sync::Arc<std::sync::Mutex<std::collections::HashSet<uuid::Uuid>>>,
    /// Async raster page-in: background decode results `(kf_id, pixels-or-None)`.
    /// `None` = the keyframe genuinely isn't in the container (blank).
    raster_load_result_tx: std::sync::mpsc::Sender<(uuid::Uuid, Option<Vec<u8>>)>,
    raster_load_result_rx: std::sync::mpsc::Receiver<(uuid::Uuid, Option<Vec<u8>>)>,
    /// Keyframe ids whose page-in is currently running on a background thread
    /// (prevents duplicate loads while the canvas keeps re-requesting them).
    raster_loads_inflight: std::collections::HashSet<uuid::Uuid>,
    /// Fault-in-recency LRU of resident raster keyframe ids. Most-recently
    /// paged-in is at the back; eviction pops the front when over budget
    /// (`RASTER_RESIDENT_MAX`). The currently-shown frame is always the most
    /// recent fault-in, so it's never evicted.
    raster_resident_lru: std::collections::VecDeque<uuid::Uuid>,
    /// Application configuration (recent files, etc.)
    config: AppConfig,
    /// Remappable keyboard shortcut manager
    keymap: KeymapManager,

    /// File operations worker command sender
    file_command_tx: std::sync::mpsc::Sender<FileCommand>,
    /// Current file operation in progress (if any)
    file_operation: Option<FileOperation>,

    /// Crash-recovery autosave state. The recovery container is a per-session `.beam` in the app's
    /// data dir; it's written in the background (reusing the file worker) and deleted on a clean
    /// exit. A leftover file on next launch signals an unclean shutdown → offer to restore.
    autosave: AutosaveState,

    /// `ActionExecutor::epoch()` at the last manual save (or new/load) — the document is "modified"
    /// (has unsaved changes vs. the user's file) when the current epoch differs, or `media_modified`
    /// is set. Distinct from the autosave baseline, which also moves on every background autosave.
    saved_epoch: u64,
    /// Non-action changes since the last save (imports, finished recordings) that `epoch` doesn't
    /// capture. Cleared on manual save / new / load.
    media_modified: bool,
    /// The pending action waiting on the "save changes?" prompt — a file switch (New/Open/Open
    /// Recent) or a quit, requested while the document had unsaved work.
    unsaved_prompt: Option<PendingAction>,
    /// The action to run once an in-flight save (triggered from that prompt via "Save") finishes.
    after_save: Option<PendingAction>,
    /// The user agreed to quit — let the next window-close request through instead of intercepting it.
    confirmed_close: bool,

    /// Audio extraction channel for background thread communication
    audio_extraction_tx: std::sync::mpsc::Sender<AudioExtractionResult>,
    audio_extraction_rx: std::sync::mpsc::Receiver<AudioExtractionResult>,

    /// Export dialog state
    export_dialog: export::dialog::ExportDialog,
    /// Export progress dialog
    export_progress_dialog: export::dialog::ExportProgressDialog,
    /// When `Some(name)`, show the "large media: pack or reference?" prompt for the
    /// named just-imported file. Set on the first large import while the
    /// `large_media_default` preference is still `Ask`.
    large_media_prompt: Option<String>,
    /// Preferences dialog
    preferences_dialog: preferences::dialog::PreferencesDialog,
    /// Export orchestrator for background exports
    export_orchestrator: Option<export::ExportOrchestrator>,
    /// Vello renderer + image cache reused across all frames of an in-progress export
    /// (built once on the first export pump, dropped when export ends) — building a new
    /// renderer per frame was the dominant export cost.
    export_renderer: Option<vello::Renderer>,
    export_image_cache: Option<lightningbeam_core::renderer::ImageCache>,
    /// GPU-rendered effect thumbnail generator
    effect_thumbnail_generator: Option<EffectThumbnailGenerator>,

    /// Custom cursor cache for SVG cursors
    cursor_cache: custom_cursor::CursorCache,
    /// Cross-platform graphics tablet (pen/stylus) input state
    tablet: tablet::TabletInput,
    /// Debug test mode (F5) — input recording, panic capture & visual replay
    #[cfg(debug_assertions)]
    test_mode: test_mode::TestModeState,

    /// Debug overlay (F3) state
    debug_overlay_visible: bool,
    debug_stats_collector: debug_overlay::DebugStatsCollector,
    gpu_info: Option<wgpu::AdapterInfo>,
    /// Surface texture format for GPU rendering (Rgba8Unorm or Bgra8Unorm depending on platform)
    target_format: wgpu::TextureFormat,
    /// Current application mode (start screen vs editor)
    app_mode: AppMode,
    /// Pending auto-reopen file path (set on startup if reopen_last_session is enabled)
    pending_auto_reopen: Option<std::path::PathBuf>,
}

/// Import filter types for the file dialog
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ImportFilter {
    #[default]
    All,
    Images,
    Audio,
    Video,
    Midi,
}

impl EditorApp {
    /// Maximum number of raster keyframes whose `raw_pixels` stay resident in
    /// RAM. Older clean keyframes beyond this are evicted (re-page on revisit);
    /// the most-recently-shown frame is always protected.
    const RASTER_RESIDENT_MAX: usize = 12;

    fn new(
        cc: &eframe::CreationContext,
        layouts: Vec<LayoutDefinition>,
        theme: Theme,
        #[cfg(debug_assertions)] panic_snapshot: std::sync::Arc<std::sync::Mutex<Option<lightningbeam_core::test_mode::TestCase>>>,
        #[cfg(debug_assertions)] pending_event: std::sync::Arc<std::sync::Mutex<Option<lightningbeam_core::test_mode::TestEvent>>>,
        #[cfg(debug_assertions)] is_replaying: std::sync::Arc<std::sync::atomic::AtomicBool>,
        #[cfg(debug_assertions)] pending_geometry: std::sync::Arc<std::sync::Mutex<Option<serde_json::Value>>>,
    ) -> Self {
        let current_layout = layouts[0].layout.clone();

        // Register the bundled Lucide icon font (used by the mobile UI; harmless on desktop).
        mobile::icons::install(&cc.egui_ctx);

        // Disable egui's "Unaligned" debug overlay (on by default in debug builds)
        #[cfg(debug_assertions)]
        cc.egui_ctx.style_mut(|style| style.debug.show_unaligned = false);

        // Disable egui's built-in Ctrl+Plus/Minus zoom — we handle zoom ourselves.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        // Load application config
        let mut config = AppConfig::load();
        // One-time cleanup: earlier builds added a Recovered file to Recent on restore. Drop any
        // recovery-dir paths that leaked in (and re-save if we removed any) so they don't show in
        // Recent or get auto-reopened below.
        let recent_before = config.recent_files.len();
        config
            .recent_files
            .retain(|p| !AutosaveState::is_recovery_path(p));
        if config.recent_files.len() != recent_before {
            config.save();
        }

        // Check if we should auto-reopen last session
        let pending_auto_reopen = if config.reopen_last_session {
            config.get_recent_files().into_iter().next()
        } else {
            None
        };

        // Initialize native menu system
        let mut menu_system = MenuSystem::new().ok();

        // Populate recent files menu
        if let Some(ref mut menu_sys) = menu_system {
            let recent_files = config.get_recent_files();
            menu_sys.update_recent_files(&recent_files);
        }

        // Create default document with a simple test scene
        let mut document = lightningbeam_core::document::Document::with_size("Untitled Animation", 1920.0, 1080.0)
            .with_duration(10.0)
            .with_framerate(60.0);

        // Add a test layer with a simple shape to visualize
        use lightningbeam_core::layer::{AnyLayer, VectorLayer};
        use lightningbeam_core::object::ShapeInstance;
        use lightningbeam_core::shape::{Shape, ShapeColor};
        use vello::kurbo::{Circle, Shape as KurboShape};

        // Create circle centered at origin, position via instance transform
        let circle = Circle::new((0.0, 0.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(100, 150, 250));
        let object = ShapeInstance::new(shape.id).with_position(200.0, 150.0);

        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        vector_layer.add_object(object);
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Wrap document in ActionExecutor
        let action_executor = lightningbeam_core::action::ActionExecutor::new(document);

        // Initialize audio system and destructure it for sharing
        let (audio_stream, audio_controller, audio_event_rx, audio_sample_rate, audio_channels, file_command_tx, recording_mirror_rx, audio_input) =
            match daw_backend::AudioSystem::new(None, config.audio_buffer_size) {
                Ok(mut audio_system) => {
                    println!("✅ Audio engine initialized successfully");

                    // Extract components
                    let mirror_rx = audio_system.take_recording_mirror_rx();
                    // take_input_opener pulls out input_tx + sample_rate/channels into
                    // a self-contained struct that can open the stream on demand.
                    let input_opener = audio_system.take_input_opener();
                    let stream = audio_system.stream;
                    let sample_rate = audio_system.sample_rate;
                    let channels = audio_system.channels;
                    let event_rx = audio_system.event_rx;

                    // Wrap controller in Arc<Mutex<>> for sharing with worker thread
                    let controller = std::sync::Arc::new(std::sync::Mutex::new(audio_system.controller));

                    // Spawn file operations worker
                    let file_command_tx = FileOperationsWorker::spawn(controller.clone());

                    (Some(stream), Some(controller), event_rx, sample_rate, channels, file_command_tx, mirror_rx, input_opener)
                }
                Err(e) => {
                    eprintln!("❌ Failed to initialize audio engine: {}", e);
                    eprintln!("   Playback will be disabled");

                    // Create a dummy channel for file operations (won't be used)
                    let (tx, _rx) = std::sync::mpsc::channel();
                    (None, None, None, 48000, 2, tx, None, None)
                }
            };

        // Create audio extraction channel for background thread communication
        let (audio_extraction_tx, audio_extraction_rx) = std::sync::mpsc::channel();

        // Extract GPU info for debug overlay
        let gpu_info = cc.wgpu_render_state.as_ref().map(|rs| rs.adapter.get_info());

        // Get surface format (defaults to Rgba8Unorm if render_state not available)
        let target_format = cc.wgpu_render_state.as_ref()
            .map(|rs| rs.target_format)
            .unwrap_or(wgpu::TextureFormat::Rgba8Unorm);

        let (waveform_result_tx, waveform_result_rx) = std::sync::mpsc::channel();
        let (raster_load_result_tx, raster_load_result_rx) = std::sync::mpsc::channel();

        Self {
            layouts,
            current_layout_index: 0,
            current_layout,
            drag_state: DragState::default(),
            hovered_divider: None,
            selected_pane: None,
            split_preview_mode: SplitPreviewMode::default(),
            mobile_ui: mobile::is_mobile_env(),
            mobile_ui_override: None,
            mobile_state: mobile::MobileState::default(),
            mobile_context_menu: None,
            keyboard_octave: 0,
            keyboard_pan_x: 0.0,
            open_instrument_browser: false,
            pending_record_toggle: false,
            icon_cache: IconCache::new(),
            tool_icon_cache: ToolIconCache::new(),
            focus_icon_cache: FocusIconCache::new(),
            selected_tool: Tool::Select, // Default tool
            fill_color: egui::Color32::from_rgb(100, 100, 255), // Default blue fill
            stroke_color: egui::Color32::from_rgb(0, 0, 0), // Default black stroke
            active_color_mode: panes::ColorMode::default(), // Default to fill color
            pane_instances: HashMap::new(), // Initialize empty, panes created on-demand
            menu_system,
            pending_view_action: None,
            theme,
            action_executor,
            active_layer_id: Some(layer_id),
            selection: lightningbeam_core::selection::Selection::new(),
            focus: lightningbeam_core::selection::FocusSelection::None,
            editing_context: EditingContext::default(),
            tool_state: lightningbeam_core::tool::ToolState::default(),
            draw_simplify_mode: lightningbeam_core::tool::SimplifyMode::Smooth, // Default to smooth curves
            rdp_tolerance: 10.0, // Default RDP tolerance
            schneider_max_error: 30.0, // Default Schneider max error
            raster_settings: tools::RasterToolSettings::default(),
            brush_preview_pixels: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            audio_stream,
            audio_controller,
            audio_event_rx,
            last_input_monitoring: false,
            audio_input,
            audio_input_stream: None,
            audio_buffer_size: config.audio_buffer_size,
            pending_graph_loads: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            commit_raster_floating_if_any: false,
            pending_node_group: false,
            pending_node_ungroup: false,
            audio_sample_rate,
            audio_channels,
            video_manager: std::sync::Arc::new(std::sync::Mutex::new(
                lightningbeam_core::video::VideoManager::new()
            )),
            shared_device: None,
            webcam: None,
            webcam_frame: None,
            webcam_record_command: None,
            layer_to_track_map: HashMap::new(),
            track_to_layer_map: HashMap::new(),
            project_generation: 0,
            graph_topology_generation: 0,
            clip_instance_to_backend_map: HashMap::new(),
            playback_time: 0.0, // Start at beginning
            is_playing: false,  // Start paused
            recording_arm_mode: RecordingArmMode::default(), // Auto mode by default
            armed_layers: HashSet::new(),     // No layers explicitly armed
            is_recording: false,              // Not recording initially
            metronome_enabled: false,         // Metronome off by default
            count_in_enabled: false,         // Count-in off by default
            recording_clips: HashMap::new(),  // No active recording clips
            recording_start_time: 0.0,        // Will be set when recording starts
            recording_layer_ids: Vec::new(),  // Will be populated when recording starts
            dragging_asset: None, // No asset being dragged initially
            clipboard_manager: lightningbeam_core::clipboard::ClipboardManager::new(),
            effect_to_load: None,
            script_to_edit: None,
            script_saved: None,
            effect_thumbnails_to_invalidate: Vec::new(),
            last_import_filter: ImportFilter::default(), // Default to "All Supported"
            stroke_width: 3.0,               // Default stroke width
            fill_enabled: true,              // Default to filling shapes
            snap_enabled: true,              // Default to snapping
            paint_bucket_gap_tolerance: 5.0, // Default gap tolerance
            polygon_sides: 5,                // Default to pentagon
            pending_region_cut_base: None,
            region_select_mode: lightningbeam_core::tool::RegionSelectMode::default(),
            lasso_mode: lightningbeam_core::tool::LassoMode::default(),
            input_level: 0.0,
            output_level: (0.0, 0.0),
            track_levels: HashMap::new(),
            midi_event_cache: HashMap::new(), // Initialize empty MIDI event cache
            audio_duration_cache: HashMap::new(), // Initialize empty audio duration cache
            audio_pools_with_new_waveforms: HashSet::new(), // Track pool indices with new raw audio
            raw_audio_cache: HashMap::new(),
            waveform_gpu_dirty: HashSet::new(),
            waveform_minmax_pools: HashMap::new(),
            waveform_pyramid_blobs: HashMap::new(),
            waveform_result_rx,
            waveform_result_tx,
            recording_mirror_rx,
            current_file_path: None, // No file loaded initially
            onion_skin: crate::panes::OnionSkinSettings::default(),
            raster_store: lightningbeam_core::raster_store::RasterStore::new(None),
            raster_fault_requests: Default::default(),
            raster_load_result_tx,
            raster_load_result_rx,
            raster_loads_inflight: Default::default(),
            raster_resident_lru: Default::default(),
            keymap: KeymapManager::new(&config.keybindings),
            config,
            file_command_tx,
            file_operation: None, // No file operation in progress initially
            autosave: AutosaveState::new(),
            saved_epoch: 0,
            media_modified: false,
            unsaved_prompt: None,
            after_save: None,
            confirmed_close: false,
            audio_extraction_tx,
            audio_extraction_rx,
            export_dialog: export::dialog::ExportDialog::default(),
            export_progress_dialog: export::dialog::ExportProgressDialog::default(),
            large_media_prompt: None,
            preferences_dialog: preferences::dialog::PreferencesDialog::default(),
            export_orchestrator: None,
            export_renderer: None,
            export_image_cache: None,
            effect_thumbnail_generator: None, // Initialized when GPU available

            // Debug test mode (F5)
            #[cfg(debug_assertions)]
            test_mode: test_mode::TestModeState::new(panic_snapshot, pending_event, is_replaying, pending_geometry),

            // Debug overlay (F3)
            cursor_cache: custom_cursor::CursorCache::new(),
            tablet: tablet::TabletInput::new(cc),
            debug_overlay_visible: false,
            debug_stats_collector: debug_overlay::DebugStatsCollector::new(),
            gpu_info,
            target_format,
            // Start screen vs editor mode
            app_mode: AppMode::StartScreen, // Always start with start screen (auto-reopen handled separately)
            pending_auto_reopen,
        }
    }

    /// Render the start screen with recent projects and new project options
    fn render_start_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_rect_before_wrap();

            // Calculate content dimensions
            let card_size: f32 = 180.0;
            let card_spacing: f32 = 24.0;
            let left_width: f32 = 350.0;
            let right_width = (card_size * 3.0) + (card_spacing * 2.0);
            let content_width = (left_width + right_width + 80.0).min(available.width() - 100.0);
            let content_height: f32 = 450.0; // Approximate height of content

            // Center content both horizontally and vertically
            let content_rect = egui::Rect::from_center_size(
                available.center(),
                egui::vec2(content_width, content_height),
            );

            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.vertical_centered(|ui| {
                    // Title
                    ui.heading(egui::RichText::new("Welcome to Lightningbeam!")
                        .size(42.0)
                        .strong());

                    ui.add_space(50.0);

                    // Main content area - two columns side by side
                    let recent_files = self.config.get_recent_files();

                    // Use columns for proper two-column layout
                    ui.columns(2, |columns| {
                        // Left column: Recent projects (stacked vertically)
                        columns[0].vertical(|ui| {
                            // Reopen last session section
                            ui.label(egui::RichText::new("Reopen last session")
                                .size(18.0)
                                .strong());
                            ui.add_space(16.0);

                            if let Some(last_file) = recent_files.first() {
                                let file_name = last_file.file_name()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_else(|| "Unknown".to_string());
                                if self.render_file_item(ui, &file_name, left_width) {
                                    self.load_from_file(last_file.clone());
                                    self.app_mode = AppMode::Editor;
                                }
                            } else {
                                ui.label(egui::RichText::new("No recent session")
                                    .color(egui::Color32::from_gray(120)));
                            }

                            ui.add_space(32.0);

                            // Recent projects section
                            ui.label(egui::RichText::new("Recent projects")
                                .size(18.0)
                                .strong());
                            ui.add_space(16.0);

                            if recent_files.len() > 1 {
                                for file in recent_files.iter().skip(1).take(5) {
                                    let file_name = file.file_name()
                                        .map(|s| s.to_string_lossy().to_string())
                                        .unwrap_or_else(|| "Unknown".to_string());
                                    if self.render_file_item(ui, &file_name, left_width) {
                                        self.load_from_file(file.clone());
                                        self.app_mode = AppMode::Editor;
                                    }
                                    ui.add_space(8.0);
                                }
                            } else {
                                ui.label(egui::RichText::new("No other recent projects")
                                    .color(egui::Color32::from_gray(120)));
                            }
                        });

                        // Right column: Create new project
                        columns[1].vertical_centered(|ui| {
                            ui.label(egui::RichText::new("Create a new project")
                                .size(18.0)
                                .strong());
                            ui.add_space(24.0);

                            // Focus cards in a horizontal row
                            ui.horizontal(|ui| {
                                // Animation
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(card_size, card_size + 40.0),
                                    egui::Sense::click(),
                                );
                                self.render_focus_card_with_icon(ui, rect, response.hovered(), "Animation", FocusIcon::Animation);
                                if response.clicked() {
                                    self.create_new_project_with_focus(0);
                                }

                                ui.add_space(card_spacing);

                                // Music
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(card_size, card_size + 40.0),
                                    egui::Sense::click(),
                                );
                                self.render_focus_card_with_icon(ui, rect, response.hovered(), "Music", FocusIcon::Music);
                                if response.clicked() {
                                    self.create_new_project_with_focus(2);
                                }

                                ui.add_space(card_spacing);

                                // Video editing
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(card_size, card_size + 40.0),
                                    egui::Sense::click(),
                                );
                                self.render_focus_card_with_icon(ui, rect, response.hovered(), "Video editing", FocusIcon::Video);
                                if response.clicked() {
                                    self.create_new_project_with_focus(1);
                                }

                                ui.add_space(card_spacing);

                                // Painting
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(card_size, card_size + 40.0),
                                    egui::Sense::click(),
                                );
                                self.render_focus_card_with_icon(ui, rect, response.hovered(), "Painting", FocusIcon::Painting);
                                if response.clicked() {
                                    self.create_new_project_with_focus(5);
                                }
                            });
                        });
                    });
                });
            });
        });
    }

    /// Render a clickable file item (for recent projects list)
    fn render_file_item(&self, ui: &mut egui::Ui, name: &str, width: f32) -> bool {
        let height = 36.0;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());

        let bg_color = if response.hovered() {
            egui::Color32::from_rgb(70, 75, 85)
        } else {
            egui::Color32::from_rgb(55, 60, 70)
        };

        let painter = ui.painter();
        painter.rect_filled(rect, 4.0, bg_color);

        painter.text(
            rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::proportional(14.0),
            egui::Color32::from_gray(220),
        );

        response.clicked()
    }

    /// Render a focus card with icon for project creation
    fn render_focus_card_with_icon(&mut self, ui: &egui::Ui, rect: egui::Rect, hovered: bool, title: &str, icon: FocusIcon) {
        let bg_color = if hovered {
            egui::Color32::from_rgb(55, 60, 70)
        } else {
            egui::Color32::from_rgb(45, 50, 58)
        };

        let border_color = egui::Color32::from_rgb(80, 85, 95);

        let painter = ui.painter();

        // Card background
        painter.rect_filled(rect, 8.0, bg_color);
        painter.rect_stroke(rect, 8.0, egui::Stroke::new(1.5, border_color), egui::StrokeKind::Inside);

        // Icon area - render SVG texture
        let icon_color = egui::Color32::from_gray(200);
        let title_area_height = 40.0;
        let icon_display_size = rect.width() - 16.0;
        let icon_center = egui::pos2(rect.center().x, rect.min.y + (rect.height() - title_area_height) * 0.5);

        // Get or load the SVG icon texture
        let ctx = ui.ctx().clone();
        if let Some(texture) = self.focus_icon_cache.get_or_load(icon, icon_color, icon_display_size, &ctx) {
            let texture_size = texture.size_vec2();
            let scale = icon_display_size / texture_size.x.max(texture_size.y);
            let scaled_size = texture_size * scale;

            let icon_rect = egui::Rect::from_center_size(icon_center, scaled_size);
            painter.image(
                texture.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Title at bottom
        painter.text(
            rect.center_bottom() - egui::vec2(0.0, 20.0),
            egui::Align2::CENTER_CENTER,
            title,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
    }

    /// Render a focus card for project creation (legacy, kept for compatibility)
    #[allow(dead_code)]
    fn render_focus_card(&self, ui: &mut egui::Ui, rect: egui::Rect, hovered: bool, title: &str, description: &str, _layout_index: usize) {
        let bg_color = if hovered {
            egui::Color32::from_rgb(60, 70, 90)
        } else {
            egui::Color32::from_rgb(45, 50, 60)
        };

        let painter = ui.painter();

        // Background with rounded corners
        painter.rect_filled(rect, 8.0, bg_color);

        // Border
        if hovered {
            painter.rect_stroke(
                rect,
                8.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 140, 200)),
                egui::StrokeKind::Inside,
            );
        }

        // Title
        painter.text(
            rect.center_top() + egui::vec2(0.0, 40.0),
            egui::Align2::CENTER_CENTER,
            title,
            egui::FontId::proportional(18.0),
            egui::Color32::WHITE,
        );

        // Description
        painter.text(
            rect.center_top() + egui::vec2(0.0, 70.0),
            egui::Align2::CENTER_CENTER,
            description,
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(180),
        );
    }

    /// Tear down the audio backend for the currently-open project.
    ///
    /// Sends `Command::Reset` (fully rebuilds the backend `Project`, audio/buffer pools, and ID
    /// counters) and clears the app-side track maps + backend-derived caches that pointed at the old
    /// tracks. Must be called before building a new document's tracks, otherwise the previous file's
    /// tracks/instruments stay resident in the backend and keep getting mixed (orphaned voices).
    ///
    /// Ordering is safe: the audio thread drains all `command_tx` commands before any `query_tx`
    /// queries each callback, so a `reset()` pushed here always runs before the `create_*_track_sync`
    /// queries that rebuild the project.
    fn reset_audio_backend(&mut self) {
        if let Some(ref controller_arc) = self.audio_controller {
            controller_arc.lock().unwrap().reset();
        }
        self.layer_to_track_map.clear();
        self.track_to_layer_map.clear();
        self.clip_instance_to_backend_map.clear();
        self.midi_event_cache.clear();
        self.audio_duration_cache.clear();
        self.raw_audio_cache.clear();
        self.waveform_gpu_dirty.clear();
        self.waveform_minmax_pools.clear();
        self.waveform_pyramid_blobs.clear();
    }

    /// Create a new project with the specified focus/layout
    fn create_new_project_with_focus(&mut self, layout_index: usize) {
        use lightningbeam_core::layer::{AnyLayer, AudioLayer, VectorLayer, VideoLayer};

        // Drop the previous project's backend tracks/instruments before building the new one, so a
        // "new file" while a project is open doesn't leave orphaned tracks resident in the backend.
        self.reset_audio_backend();

        // Create a new blank document
        let mut document = lightningbeam_core::document::Document::with_size(
            "Untitled",
            self.config.file_width as f64,
            self.config.file_height as f64,
        )
        .with_duration(60.0) // 1 minute default
        .with_framerate(self.config.framerate as f64);

        // Add default layer based on focus type
        // Layout indices: 0 = Animation, 1 = Video editing, 2 = Music, 5 = Drawing/Painting
        let layer_id = match layout_index {
            0 => {
                // Animation focus -> VectorLayer
                let layer = VectorLayer::new("Layer 1");
                document.root.add_child(AnyLayer::Vector(layer))
            }
            1 => {
                // Video editing focus -> VideoLayer + black background
                document.background_color = lightningbeam_core::shape::ShapeColor::rgb(0, 0, 0);
                let layer = VideoLayer::new("Video 1");
                document.root.add_child(AnyLayer::Video(layer))
            }
            2 => {
                // Music focus -> MIDI AudioLayer
                let layer = AudioLayer::new_midi("MIDI 1");
                document.root.add_child(AnyLayer::Audio(layer))
            }
            5 => {
                // Painting focus -> RasterLayer
                use lightningbeam_core::raster_layer::RasterLayer;
                let mut layer = RasterLayer::new("Raster 1");
                layer.ensure_keyframe_at(self.playback_time, document.width as u32, document.height as u32);
                document.root.add_child(AnyLayer::Raster(layer))
            }
            _ => {
                // Fallback to VectorLayer
                let layer = VectorLayer::new("Layer 1");
                document.root.add_child(AnyLayer::Vector(layer))
            }
        };

        // Set default timeline mode based on activity
        document.timeline_mode = match layout_index {
            2 => lightningbeam_core::document::TimelineMode::Measures, // Music
            1 => lightningbeam_core::document::TimelineMode::Seconds,  // Video
            _ => lightningbeam_core::document::TimelineMode::Frames,   // Animation, Painting, etc.
        };

        // Reset action executor with new document
        self.action_executor = lightningbeam_core::action::ActionExecutor::new(document);
        // Fresh document → nothing new to recover; rebase the autosave epoch.
        self.reset_autosave_baseline();

        // Apply the layout
        if layout_index < self.layouts.len() {
            self.current_layout_index = layout_index;
            self.current_layout = self.layouts[layout_index].layout.clone();
            self.pane_instances.clear(); // Clear old pane instances
        }

        // Clear file path (new unsaved document)
        self.current_file_path = None;

        // Reset selection and set active layer to the newly created one
        self.selection = lightningbeam_core::selection::Selection::new();
        self.focus = lightningbeam_core::selection::FocusSelection::None;
        self.active_layer_id = Some(layer_id);

        // For Music focus, sync the MIDI layer with daw-backend
        if layout_index == 2 {
            self.sync_audio_layers_to_backend();
        }

        // Switch to editor mode
        self.app_mode = AppMode::Editor;
    }

    /// Synchronize all existing MIDI layers in the document with daw-backend tracks
    ///
    /// This function should be called:
    /// - After loading a document from file
    /// - After creating a new document with pre-existing MIDI layers
    ///
    /// For each audio layer (MIDI or Sampled):
    /// 1. Creates a daw-backend track (MIDI or Audio)
    /// 2. For MIDI: Loads the default instrument
    /// 3. Stores the bidirectional mapping
    /// 4. Syncs any existing clips on the layer
    fn sync_audio_layers_to_backend(&mut self) {
        use lightningbeam_core::layer::{AnyLayer, AudioLayerType};

        // Ensure the master layer has a backend group track.
        let master_layer_id = self.action_executor.document().master_layer.layer.id;
        if !self.layer_to_track_map.contains_key(&master_layer_id) {
            if let Some(ref controller_arc) = self.audio_controller {
                let track_id_result = {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.create_group_track_sync("[Master]".to_string(), None)
                };
                if let Ok(track_id) = track_id_result {
                    self.layer_to_track_map.insert(master_layer_id, track_id);
                    println!("✅ Created master bus track (TrackId: {})", track_id);
                }
            }
        }

        // Collect audio layers from root and inside vector clips
        // Each entry: (layer_id, layer_name, audio_type, parent_clip_id)
        let mut audio_layers_to_sync: Vec<(uuid::Uuid, String, AudioLayerType, Option<uuid::Uuid>)> = Vec::new();

        // Root layers
        for layer in &self.action_executor.document().root.children {
            if let AnyLayer::Audio(audio_layer) = layer {
                let layer_id = audio_layer.layer.id;
                if !self.layer_to_track_map.contains_key(&layer_id) {
                    audio_layers_to_sync.push((
                        layer_id,
                        audio_layer.layer.name.clone(),
                        audio_layer.audio_layer_type,
                        None,
                    ));
                }
            }
        }

        // Layers inside group layers (recursive)
        fn collect_audio_from_groups(
            layers: &[lightningbeam_core::layer::AnyLayer],
            parent_group_id: Option<uuid::Uuid>,
            existing: &std::collections::HashMap<uuid::Uuid, daw_backend::TrackId>,
            out: &mut Vec<(uuid::Uuid, String, AudioLayerType, Option<uuid::Uuid>)>,
        ) {
            for layer in layers {
                if let AnyLayer::Group(group) = layer {
                    let gid = group.layer.id;
                    collect_audio_from_groups(&group.children, Some(gid), existing, out);
                } else if let AnyLayer::Audio(audio_layer) = layer {
                    if parent_group_id.is_some() && !existing.contains_key(&audio_layer.layer.id) {
                        out.push((
                            audio_layer.layer.id,
                            audio_layer.layer.name.clone(),
                            audio_layer.audio_layer_type,
                            parent_group_id,
                        ));
                    }
                }
            }
        }
        collect_audio_from_groups(
            &self.action_executor.document().root.children,
            None,
            &self.layer_to_track_map,
            &mut audio_layers_to_sync,
        );

        // Layers inside vector clips
        for (&clip_id, clip) in &self.action_executor.document().vector_clips {
            for layer in clip.layers.root_data() {
                if let AnyLayer::Audio(audio_layer) = layer {
                    let layer_id = audio_layer.layer.id;
                    if !self.layer_to_track_map.contains_key(&layer_id) {
                        audio_layers_to_sync.push((
                            layer_id,
                            audio_layer.layer.name.clone(),
                            audio_layer.audio_layer_type,
                            Some(clip_id),
                        ));
                    }
                }
            }
        }

        // Now create backend tracks for each
        for (layer_id, layer_name, audio_type, parent_id) in audio_layers_to_sync {
            // If inside a clip or group, ensure a metatrack exists
            let parent_track = parent_id.and_then(|pid| self.ensure_metatrack_for_parent(pid));

            match audio_type {
                AudioLayerType::Midi => {
                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        match controller.create_midi_track_sync(layer_name.clone(), parent_track) {
                            Ok(track_id) => {
                                self.layer_to_track_map.insert(layer_id, track_id);
                                self.track_to_layer_map.insert(track_id, layer_id);

                                if let Err(e) = default_instrument::load_default_instrument(&mut *controller, track_id) {
                                    eprintln!("⚠️  Failed to load default instrument for {}: {}", layer_name, e);
                                } else {
                                    println!("✅ Synced MIDI layer '{}' to backend (TrackId: {}, parent: {:?})", layer_name, track_id, parent_track);
                                }
                            }
                            Err(e) => {
                                eprintln!("⚠️  Failed to create daw-backend track for MIDI layer '{}': {}", layer_name, e);
                            }
                        }
                    }
                }
                AudioLayerType::Sampled => {
                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        match controller.create_audio_track_sync(layer_name.clone(), parent_track) {
                            Ok(track_id) => {
                                self.layer_to_track_map.insert(layer_id, track_id);
                                self.track_to_layer_map.insert(track_id, layer_id);
                                println!("✅ Synced Audio layer '{}' to backend (TrackId: {}, parent: {:?})", layer_name, track_id, parent_track);
                            }
                            Err(e) => {
                                eprintln!("⚠️  Failed to create daw-backend audio track for '{}': {}", layer_name, e);
                            }
                        }
                    }
                }
            }
        }

        // After all tracks are created, push subtrack mixing graph commands for group metatracks
        self.update_metatrack_subtrack_graphs();
    }

    /// Collect all group layers (depth-first) and push their subtrack graph to the backend.
    /// Also creates metatracks for any groups that don't have one yet (e.g., empty groups).
    /// Called at the end of sync_audio_layers_to_backend, after all child tracks exist.
    fn update_metatrack_subtrack_graphs(&mut self) {
        use lightningbeam_core::layer::AnyLayer;

        // Collect (group_layer_id, group_name, children snapshot) for every group.
        // Snapshot the name too so we can create metatracks for groups not yet in the map.
        let mut group_snapshots: Vec<(uuid::Uuid, String, Vec<AnyLayer>)> = Vec::new();

        fn collect_groups(layers: &[AnyLayer], out: &mut Vec<(uuid::Uuid, String, Vec<AnyLayer>)>) {
            for layer in layers {
                if let AnyLayer::Group(g) = layer {
                    out.push((g.layer.id, g.layer.name.clone(), g.children.clone()));
                    collect_groups(&g.children, out);
                }
            }
        }
        collect_groups(
            &self.action_executor.document().root.children,
            &mut group_snapshots,
        );

        // Ensure metatracks exist for ALL groups, not just those with audio children.
        // This allows the node graph pane to open for empty group layers too.
        if let Some(ref controller_arc) = self.audio_controller {
            for (group_id, group_name, _) in &group_snapshots {
                if !self.layer_to_track_map.contains_key(group_id) {
                    let track_id_result = {
                        let mut controller = controller_arc.lock().unwrap();
                        controller.create_group_track_sync(format!("[{}]", group_name), None)
                    };
                    match track_id_result {
                        Ok(track_id) => {
                            self.layer_to_track_map.insert(*group_id, track_id);
                            println!("✅ Created metatrack for group '{}' (TrackId: {})", group_name, track_id);
                        }
                        Err(e) => {
                            eprintln!("⚠️  Failed to create metatrack for group '{}': {}", group_name, e);
                        }
                    }
                }
            }
        }

        // Push subtrack graph commands for every group that now has a metatrack.
        if let Some(ref controller_arc) = self.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            for (group_id, _, children) in &group_snapshots {
                if let Some(&metatrack_id) = self.layer_to_track_map.get(group_id) {
                    let subtracks = self.build_subtrack_list_for_group(children);
                    controller.set_metatrack_subtrack_graph(metatrack_id, subtracks);
                }
            }
        }

        // Push subtrack graph for the master bus (all root-level tracks as its inputs).
        let master_layer_id = self.action_executor.document().master_layer.layer.id;
        if let Some(&master_track_id) = self.layer_to_track_map.get(&master_layer_id) {
            let root_children = self.action_executor.document().root.children.clone();
            let master_subtracks = self.build_subtrack_list_for_group(&root_children);
            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                controller.set_metatrack_subtrack_graph(master_track_id, master_subtracks);
            }
        }
    }

    /// Build the ordered subtrack list for a group layer's direct children.
    /// Audio child layers → looked up via layer_to_track_map.
    /// Nested group children → looked up via layer_to_track_map.
    fn build_subtrack_list_for_group(
        &self,
        children: &[lightningbeam_core::layer::AnyLayer],
    ) -> Vec<(daw_backend::TrackId, String)> {
        use lightningbeam_core::layer::AnyLayer;
        let mut subtracks = Vec::new();
        // Iterate in reverse so the top timeline layer maps to port 0 (top of node).
        // Timeline layers are stored top-to-bottom (index 0 = topmost), and node ports
        // are displayed top-to-bottom, so reversing aligns them.
        for child in children.iter().rev() {
            match child {
                AnyLayer::Audio(audio_layer) => {
                    if let Some(&track_id) = self.layer_to_track_map.get(&audio_layer.layer.id) {
                        subtracks.push((track_id, audio_layer.layer.name.clone()));
                    }
                }
                AnyLayer::Group(group_layer) => {
                    if let Some(&meta_id) = self.layer_to_track_map.get(&group_layer.layer.id) {
                        subtracks.push((meta_id, group_layer.layer.name.clone()));
                    }
                }
                _ => {}
            }
        }
        subtracks
    }

    /// Ensure a backend metatrack exists for a parent container (VectorClip or GroupLayer).
    /// Checks if the ID belongs to a GroupLayer first, then falls back to VectorClip.
    fn ensure_metatrack_for_parent(&mut self, parent_id: Uuid) -> Option<daw_backend::TrackId> {
        // Return existing metatrack if already mapped
        if let Some(&track_id) = self.layer_to_track_map.get(&parent_id) {
            return Some(track_id);
        }

        // Check if it's a GroupLayer
        let is_group = self.action_executor.document().root.children.iter()
            .any(|l| matches!(l, lightningbeam_core::layer::AnyLayer::Group(g) if g.layer.id == parent_id));

        if is_group {
            return self.ensure_metatrack_for_group(parent_id);
        }

        // Fall back to VectorClip
        self.ensure_metatrack_for_clip(parent_id)
    }

    /// Ensure a backend metatrack (group track) exists for a GroupLayer.
    fn ensure_metatrack_for_group(&mut self, group_layer_id: Uuid) -> Option<daw_backend::TrackId> {
        if let Some(&track_id) = self.layer_to_track_map.get(&group_layer_id) {
            return Some(track_id);
        }

        let group_name = self.action_executor.document().root.children.iter()
            .find(|l| l.id() == group_layer_id)
            .map(|l| l.name().to_string())
            .unwrap_or_else(|| "Group".to_string());

        if let Some(ref controller_arc) = self.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            match controller.create_group_track_sync(format!("[{}]", group_name), None) {
                Ok(track_id) => {
                    self.layer_to_track_map.insert(group_layer_id, track_id);
                    println!("✅ Created metatrack for group '{}' (TrackId: {})", group_name, track_id);
                    return Some(track_id);
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to create metatrack for group '{}': {}", group_name, e);
                }
            }
        }
        None
    }

    /// Ensure a backend metatrack (group track) exists for a movie clip.
    /// Returns the metatrack's TrackId, creating one if needed.
    fn ensure_metatrack_for_clip(&mut self, clip_id: Uuid) -> Option<daw_backend::TrackId> {
        // Return existing metatrack if already mapped
        if let Some(&track_id) = self.layer_to_track_map.get(&clip_id) {
            return Some(track_id);
        }

        // Create a new metatrack in the backend
        let clip_name = self.action_executor.document().vector_clips
            .get(&clip_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("Clip {}", clip_id));

        if let Some(ref controller_arc) = self.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            match controller.create_group_track_sync(format!("[{}]", clip_name), None) {
                Ok(track_id) => {
                    self.layer_to_track_map.insert(clip_id, track_id);
                    println!("✅ Created metatrack for clip '{}' (TrackId: {})", clip_name, track_id);
                    return Some(track_id);
                }
                Err(e) => {
                    eprintln!("⚠️  Failed to create metatrack for clip '{}': {}", clip_name, e);
                }
            }
        }
        None
    }

    /// Split clip instances at the current playhead position
    ///
    /// Only splits clips on the active layer, plus any clips linked to them
    /// via instance groups. For video clips with linked audio, the newly
    /// created instances are added to a new group to maintain the link.
    fn split_clips_at_playhead(&mut self) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::SplitClipInstanceAction;
        use lightningbeam_core::instance_group::InstanceGroup;
        use std::collections::HashSet;

        // Split position as a beats timeline position (playback_time is seconds).
        let split_time = self.action_executor.document().tempo_map().seconds_to_beats(Seconds(self.playback_time));
        let active_layer_id = match self.active_layer_id {
            Some(id) => id,
            None => return, // No active layer, nothing to split
        };

        let document = self.action_executor.document();

        // Helper to find clips that span the playhead in a specific layer
        fn find_splittable_clips(
            clip_instances: &[lightningbeam_core::clip::ClipInstance],
            split_time: Beats,
            document: &lightningbeam_core::document::Document,
        ) -> Vec<uuid::Uuid> {
            let mut result = Vec::new();
            for instance in clip_instances {
                if let Some(clip_duration) = document.get_clip_duration(&instance.clip_id) {
                    let effective_duration = instance.effective_duration(clip_duration, document.tempo_map());
                    let timeline_end = instance.timeline_start + effective_duration;

                    let epsilon = Beats(0.001);
                    if split_time > instance.timeline_start + epsilon
                        && split_time < timeline_end - epsilon
                    {
                        result.push(instance.id);
                    }
                }
            }
            result
        }

        // First, find clips on the active layer that span the playhead
        let mut clips_to_split: Vec<(uuid::Uuid, uuid::Uuid)> = Vec::new();
        let mut processed_instances: HashSet<uuid::Uuid> = HashSet::new();

        // Get clips on active layer
        if let Some(layer) = document.get_layer(&active_layer_id) {
            let active_layer_clips = match layer {
                AnyLayer::Vector(vl) => find_splittable_clips(&vl.clip_instances, split_time, document),
                AnyLayer::Audio(al) => find_splittable_clips(&al.clip_instances, split_time, document),
                AnyLayer::Video(vl) => find_splittable_clips(&vl.clip_instances, split_time, document),
                AnyLayer::Effect(el) => find_splittable_clips(&el.clip_instances, split_time, document),
                AnyLayer::Group(_) | AnyLayer::Raster(_) | AnyLayer::Text(_) => vec![],
            };

            for instance_id in active_layer_clips {
                clips_to_split.push((active_layer_id, instance_id));
                processed_instances.insert(instance_id);

                // Check if this instance is in a group - if so, add all group members
                if let Some(group) = document.find_group_for_instance(&instance_id) {
                    for (member_layer_id, member_instance_id) in group.get_members() {
                        if !processed_instances.contains(member_instance_id) {
                            // Verify this member also spans the playhead
                            if let Some(member_layer) = document.get_layer(member_layer_id) {
                                let member_splittable = match member_layer {
                                    AnyLayer::Vector(vl) => find_splittable_clips(&vl.clip_instances, split_time, document),
                                    AnyLayer::Audio(al) => find_splittable_clips(&al.clip_instances, split_time, document),
                                    AnyLayer::Video(vl) => find_splittable_clips(&vl.clip_instances, split_time, document),
                                    AnyLayer::Effect(el) => find_splittable_clips(&el.clip_instances, split_time, document),
                                    AnyLayer::Group(_) | AnyLayer::Raster(_) | AnyLayer::Text(_) => vec![],
                                };
                                if member_splittable.contains(member_instance_id) {
                                    clips_to_split.push((*member_layer_id, *member_instance_id));
                                    processed_instances.insert(*member_instance_id);
                                }
                            }
                        }
                    }
                }
            }
        }

        if clips_to_split.is_empty() {
            return;
        }

        // Track original instance IDs and which group they belong to (if any)
        // Also pre-generate new instance IDs so we can create groups before executing
        let mut split_info: Vec<(uuid::Uuid, uuid::Uuid, uuid::Uuid, Option<uuid::Uuid>)> = Vec::new();
        // Format: (layer_id, original_instance_id, new_instance_id, original_group_id)

        for (layer_id, instance_id) in &clips_to_split {
            let group_id = document.find_group_for_instance(instance_id).map(|g| g.id);
            let new_instance_id = uuid::Uuid::new_v4();
            split_info.push((*layer_id, *instance_id, new_instance_id, group_id));
        }

        // Execute split actions with pre-generated new instance IDs
        for (layer_id, instance_id, new_instance_id, _) in &split_info {
            let action = SplitClipInstanceAction::with_new_instance_id(
                *layer_id,
                *instance_id,
                split_time,
                *new_instance_id,
            );

            // Execute with backend synchronization
            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: &self.layer_to_track_map,
                    clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                };

                if let Err(e) = self.action_executor.execute_with_backend(Box::new(action), &mut backend_context) {
                    eprintln!("Split action failed: {}", e);
                    continue;
                }
            } else {
                let boxed_action: Box<dyn lightningbeam_core::action::Action> = Box::new(action);
                if let Err(e) = self.action_executor.execute(boxed_action) {
                    eprintln!("Split action failed: {}", e);
                    continue;
                }
            }
        }

        // Now create groups for the newly created instances to maintain linking
        // Group new instances by their original group membership
        let mut groups_to_create: std::collections::HashMap<uuid::Uuid, Vec<(uuid::Uuid, uuid::Uuid)>> = std::collections::HashMap::new();

        for (layer_id, _, new_instance_id, original_group_id) in &split_info {
            if let Some(group_id) = original_group_id {
                groups_to_create
                    .entry(*group_id)
                    .or_insert_with(Vec::new)
                    .push((*layer_id, *new_instance_id));
            }
        }

        // Create new groups for the split instances
        let document = self.action_executor.document_mut();
        for (_, members) in groups_to_create {
            if members.len() > 1 {
                let mut new_group = InstanceGroup::new();
                for (layer_id, instance_id) in members {
                    new_group.add_member(layer_id, instance_id);
                }
                document.add_instance_group(new_group);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Raster pixel helpers
    // -----------------------------------------------------------------------

    /// Extract the pixels covered by `sel` from `raw_pixels`.
    /// Returns (pixels, width, height) in sRGB-premul RGBA format.
    /// For a Lasso selection pixels outside the polygon are zeroed (alpha=0).
    fn extract_raster_selection(
        raw_pixels: &[u8],
        canvas_w: u32,
        canvas_h: u32,
        sel: &lightningbeam_core::selection::RasterSelection,
    ) -> (Vec<u8>, u32, u32) {
        use lightningbeam_core::selection::RasterSelection;
        let (x0, y0, x1, y1) = sel.bounding_rect();
        let x0 = x0.max(0) as u32;
        let y0 = y0.max(0) as u32;
        let x1 = (x1 as u32).min(canvas_w);
        let y1 = (y1 as u32).min(canvas_h);
        let w = x1.saturating_sub(x0);
        let h = y1.saturating_sub(y0);
        let mut out = vec![0u8; (w * h * 4) as usize];
        for row in 0..h {
            for col in 0..w {
                let cx = x0 + col;
                let cy = y0 + row;
                let inside = match sel {
                    RasterSelection::Rect(..) => true,
                    RasterSelection::Lasso(_) | RasterSelection::Mask { .. } =>
                        sel.contains_pixel(cx as i32, cy as i32),
                };
                if inside {
                    let src = ((cy * canvas_w + cx) * 4) as usize;
                    let dst = ((row * w + col) * 4) as usize;
                    out[dst..dst + 4].copy_from_slice(&raw_pixels[src..src + 4]);
                }
            }
        }
        (out, w, h)
    }

    /// Erase pixels covered by `sel` in `raw_pixels` (set alpha=0, rgb=0).
    fn erase_raster_selection(
        raw_pixels: &mut [u8],
        canvas_w: u32,
        canvas_h: u32,
        sel: &lightningbeam_core::selection::RasterSelection,
    ) {
        let (x0, y0, x1, y1) = sel.bounding_rect();
        let x0 = x0.max(0) as u32;
        let y0 = y0.max(0) as u32;
        let x1 = (x1 as u32).min(canvas_w);
        let y1 = (y1 as u32).min(canvas_h);
        for cy in y0..y1 {
            for cx in x0..x1 {
                if sel.contains_pixel(cx as i32, cy as i32) {
                    let idx = ((cy * canvas_w + cx) * 4) as usize;
                    raw_pixels[idx..idx + 4].fill(0);
                }
            }
        }
    }

    /// Commit a floating raster selection: composite it into the keyframe's
    /// `raw_pixels` and record a `RasterStrokeAction` for undo.
    /// Clears `selection.raster_floating` and `selection.raster_selection`.
    /// No-op if there is no floating selection.
    fn commit_raster_floating(&mut self) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterStrokeAction;

        let Some(float) = self.selection.raster_floating.take() else { return };
        let sel = self.selection.raster_selection.take();

        let document = self.action_executor.document_mut();
        let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&float.layer_id) else { return };
        let Some(kf) = rl.keyframe_at_mut(float.time) else { return };

        // Ensure the canvas is allocated (empty Vec = blank transparent canvas).
        let expected = (kf.width * kf.height * 4) as usize;
        if kf.raw_pixels.len() != expected {
            kf.raw_pixels.resize(expected, 0);
        }

        // Porter-Duff "src over dst" for sRGB-encoded premultiplied pixels,
        // masked by the selection C when present.
        for row in 0..float.height {
            let dy = float.y + row as i32;
            if dy < 0 || dy >= kf.height as i32 { continue; }
            for col in 0..float.width {
                let dx = float.x + col as i32;
                if dx < 0 || dx >= kf.width as i32 { continue; }
                // Apply selection mask C (if selection exists, only composite where inside)
                if let Some(ref s) = sel {
                    if !s.contains_pixel(dx, dy) { continue; }
                }
                let si = ((row * float.width + col) * 4) as usize;
                let di = ((dy as u32 * kf.width + dx as u32) * 4) as usize;
                let sa = float.pixels[si + 3] as u32;
                if sa == 0 { continue; }
                let da = kf.raw_pixels[di + 3] as u32;
                let out_a = sa + da * (255 - sa) / 255;
                kf.raw_pixels[di + 3] = out_a as u8;
                if out_a > 0 {
                    for c in 0..3 {
                        let v = float.pixels[si + c] as u32 * 255
                            + kf.raw_pixels[di + c] as u32 * (255 - sa);
                        kf.raw_pixels[di + c] = (v / 255).min(255) as u8;
                    }
                }
            }
        }

        let canvas_after = kf.raw_pixels.clone();
        let w = kf.width;
        let h = kf.height;

        let action = RasterStrokeAction::new(
            float.layer_id, float.time,
            std::sync::Arc::try_unwrap(float.canvas_before).unwrap_or_else(|a| (*a).clone()),
            canvas_after,
            w, h,
        );
        if let Err(e) = self.action_executor.execute(Box::new(action)) {
            eprintln!("commit_raster_floating: {}", e);
        }
    }

    /// Cancel a floating raster selection: restore the canvas from the
    /// pre-cut/paste snapshot.  No undo entry is created.
    /// Fault in the raster keyframe that a pending undo/redo needs resident, so its
    /// dirty-rect diff applies onto the correct full base. A clean evicted frame's
    /// container bytes equal its current logical state, so this restores the right
    /// base. Synchronous — undo/redo is a discrete user action, not a hot path.
    fn ensure_raster_resident_for_undo(&mut self, hint: Option<(uuid::Uuid, f64)>) {
        use lightningbeam_core::layer::AnyLayer;
        let Some((layer_id, time)) = hint else { return };
        if !self.raster_store.has_path() { return; }
        // Identify the (paged-out) keyframe the action will touch.
        let target = {
            let doc = self.action_executor.document();
            match doc.get_layer(&layer_id) {
                Some(AnyLayer::Raster(rl)) => rl
                    .keyframe_at(time)
                    .filter(|kf| kf.raw_pixels.is_empty() && kf.needs_fault_in)
                    .map(|kf| kf.id),
                _ => None,
            }
        };
        if let Some(kf_id) = target {
            if let Some(pixels) = self.raster_store.load_pixels(kf_id) {
                let doc = self.action_executor.document_mut();
                if let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&layer_id) {
                    if let Some(kf) = rl.keyframes.iter_mut().find(|k| k.id == kf_id) {
                        kf.raw_pixels = pixels;
                        kf.texture_dirty = true;
                        kf.needs_fault_in = false;
                    }
                }
                // Track these resident pixels in the LRU so they count toward
                // RASTER_RESIDENT_MAX and can be evicted later; without this, a frame
                // faulted in for undo/redo that ends up clean would stay resident forever,
                // letting resident RAM grow past the cap.
                self.raster_resident_lru.retain(|id| *id != kf_id);
                self.raster_resident_lru.push_back(kf_id);
            }
        }
    }

    fn cancel_raster_floating(&mut self) {
        use lightningbeam_core::layer::AnyLayer;

        let Some(float) = self.selection.raster_floating.take() else { return };
        self.selection.raster_selection = None;

        let document = self.action_executor.document_mut();
        let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&float.layer_id) else { return };
        let Some(kf) = rl.keyframe_at_mut(float.time) else { return };
        kf.raw_pixels = std::sync::Arc::try_unwrap(float.canvas_before).unwrap_or_else(|a| (*a).clone());
        // Restoring the pre-lift snapshot rewrites raw_pixels in memory; that
        // snapshot may itself contain un-persisted edits, and the lift had marked
        // the kf dirty. Keep it pinned (dirty) so eviction can't lose it.
        kf.dirty = true;
        kf.texture_dirty = true;
    }

    /// Drop (discard) the floating selection keeping the hole punched in the
    /// canvas.  Records a `RasterStrokeAction` for undo.  Used by cut (Ctrl+X).
    fn drop_raster_float(&mut self) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterStrokeAction;

        let Some(float) = self.selection.raster_floating.take() else { return };
        self.selection.raster_selection = None;

        let doc = self.action_executor.document_mut();
        let Some(AnyLayer::Raster(rl)) = doc.get_layer_mut(&float.layer_id) else { return };
        let Some(kf) = rl.keyframe_at_mut(float.time) else { return };
        // raw_pixels already has the hole; record the undo action.
        let canvas_after = kf.raw_pixels.clone();
        let (w, h) = (kf.width, kf.height);
        let action = RasterStrokeAction::new(
            float.layer_id, float.time,
            std::sync::Arc::try_unwrap(float.canvas_before).unwrap_or_else(|a| (*a).clone()),
            canvas_after,
            w, h,
        );
        if let Err(e) = self.action_executor.execute(Box::new(action)) {
            eprintln!("drop_raster_float: {e}");
        }
    }

    /// Copy the current selection to the clipboard
    fn clipboard_copy_selection(&mut self) {
        use lightningbeam_core::clipboard::{ClipboardContent, ClipboardLayerType};
        use lightningbeam_core::layer::AnyLayer;

        // Raster selection takes priority when on a raster layer.
        // If a floating selection exists (auto-lifted pixels), read directly from
        // the float so we get exactly the lifted pixels.
        if let Some(layer_id) = self.active_layer_id {
            let document = self.action_executor.document();
            if matches!(document.get_layer(&layer_id), Some(AnyLayer::Raster(_))) {
                if let Some(float) = &self.selection.raster_floating {
                    self.clipboard_manager.copy(ClipboardContent::RasterPixels {
                        pixels: (*float.pixels).clone(),
                        width: float.width,
                        height: float.height,
                    });
                    return;
                } else if let Some(raster_sel) = self.selection.raster_selection.as_ref() {
                    if let Some(AnyLayer::Raster(rl)) = document.get_layer(&layer_id) {
                        if let Some(kf) = rl.keyframe_at(self.playback_time) {
                            let (pixels, w, h) = Self::extract_raster_selection(
                                &kf.raw_pixels, kf.width, kf.height, raster_sel,
                            );
                            self.clipboard_manager.copy(ClipboardContent::RasterPixels {
                                pixels, width: w, height: h,
                            });
                        }
                    }
                    return;
                }
            }
        }

        // Check what's selected: clip instances take priority, then shapes
        if !self.selection.clip_instances().is_empty() {
            let active_layer_id = match self.active_layer_id {
                Some(id) => id,
                None => return,
            };

            let document = self.action_executor.document();
            let layer = match document.get_layer(&active_layer_id) {
                Some(l) => l,
                None => return,
            };

            let layer_type = ClipboardLayerType::from_layer(layer);

            let clip_slice: &[lightningbeam_core::clip::ClipInstance] = match layer {
                AnyLayer::Vector(vl) => &vl.clip_instances,
                AnyLayer::Audio(al) => &al.clip_instances,
                AnyLayer::Video(vl) => &vl.clip_instances,
                AnyLayer::Effect(el) => &el.clip_instances,
                AnyLayer::Group(_) | AnyLayer::Raster(_) | AnyLayer::Text(_) => &[],
            };
            let instances: Vec<_> = clip_slice
            .iter()
            .filter(|ci| self.selection.contains_clip_instance(&ci.id))
            .cloned()
            .collect();

            if instances.is_empty() {
                return;
            }

            // Gather referenced clip definitions
            let mut audio_clips = Vec::new();
            let mut video_clips = Vec::new();
            let mut vector_clips = Vec::new();
            let image_assets = Vec::new();
            let mut seen_clip_ids = std::collections::HashSet::new();

            for inst in &instances {
                if !seen_clip_ids.insert(inst.clip_id) {
                    continue;
                }
                if let Some(clip) = document.get_audio_clip(&inst.clip_id) {
                    audio_clips.push((inst.clip_id, clip.clone()));
                } else if let Some(clip) = document.get_video_clip(&inst.clip_id) {
                    video_clips.push((inst.clip_id, clip.clone()));
                } else if let Some(clip) = document.get_vector_clip(&inst.clip_id) {
                    vector_clips.push((inst.clip_id, clip.clone()));
                }
            }

            // Gather image assets referenced by vector clips
            // (Future: walk vector clip layers for image fill references)

            let content = ClipboardContent::ClipInstances {
                layer_type,
                instances,
                audio_clips,
                video_clips,
                vector_clips,
                image_assets,
            };

            self.clipboard_manager.copy(content);
        } else if self.selection.has_geometry_selection() {
            let subgraph = if let Some(dcel) = self.selection.vector_subgraph.take() {
                // Region selection: the sub-DCEL was pre-extracted on commit.
                dcel
            } else {
                // Select tool: extract faces adjacent to the selected edges.
                // TODO: VectorGraph copy — extract_faces_for_edges requires Dcel;
                // port to VectorGraph when clipboard is migrated.
                return;
            };

            let dcel_json = serde_json::to_string(&subgraph).unwrap_or_default();
            // TODO: svg_export needs to be ported to VectorGraph
            let svg_xml = String::new();
            self.clipboard_manager.copy(
                lightningbeam_core::clipboard::ClipboardContent::VectorGeometry {
                    dcel_json,
                    svg_xml,
                },
            );
            // Restore the subgraph so a subsequent cut can also delete.
            self.selection.vector_subgraph = Some(subgraph);
        }
    }

    /// Delete the current selection (for cut and delete operations)
    fn clipboard_delete_selection(&mut self) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::RasterStrokeAction;

        // Raster: if a floating selection exists (auto-lifted), just drop it
        // (keeps the hole).  Otherwise commit any float then erase the marquee region.
        if let Some(layer_id) = self.active_layer_id {
            let document = self.action_executor.document();
            if matches!(document.get_layer(&layer_id), Some(AnyLayer::Raster(_))) {
                if self.selection.raster_floating.is_some() {
                    self.drop_raster_float();
                    return;
                }
            }
        }

        if let (Some(layer_id), Some(raster_sel)) = (
            self.active_layer_id,
            self.selection.raster_selection.clone(),
        ) {
            let document = self.action_executor.document();
            if matches!(document.get_layer(&layer_id), Some(AnyLayer::Raster(_))) {
                self.commit_raster_floating();

                let document = self.action_executor.document_mut();
                if let Some(AnyLayer::Raster(rl)) = document.get_layer_mut(&layer_id) {
                    if let Some(kf) = rl.keyframe_at_mut(self.playback_time) {
                        let canvas_before = kf.raw_pixels.clone();
                        Self::erase_raster_selection(
                            &mut kf.raw_pixels, kf.width, kf.height, &raster_sel,
                        );
                        let canvas_after = kf.raw_pixels.clone();
                        let w = kf.width;
                        let h = kf.height;
                        let action = RasterStrokeAction::new(
                            layer_id, self.playback_time,
                            canvas_before, canvas_after, w, h,
                        );
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Raster erase failed: {}", e);
                        }
                    }
                }
                self.selection.raster_selection = None;
                return;
            }
        }

        if !self.selection.clip_instances().is_empty() {
            let active_layer_id = match self.active_layer_id {
                Some(id) => id,
                None => return,
            };

            // Build removals list
            let removals: Vec<(Uuid, Uuid)> = self
                .selection
                .clip_instances()
                .iter()
                .map(|&id| (active_layer_id, id))
                .collect();

            if removals.is_empty() {
                return;
            }

            let action = lightningbeam_core::actions::RemoveClipInstancesAction::new(removals);

            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: &self.layer_to_track_map,
                    clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                };
                if let Err(e) = self
                    .action_executor
                    .execute_with_backend(Box::new(action), &mut backend_context)
                {
                    eprintln!("Delete clip instances failed: {}", e);
                }
            } else {
                if let Err(e) = self.action_executor.execute(Box::new(action)) {
                    eprintln!("Delete clip instances failed: {}", e);
                }
            }

            self.selection.clear_clip_instances();
            self.focus = lightningbeam_core::selection::FocusSelection::None;
        } else if self.selection.has_geometry_selection() {
            let active_layer_id = match self.active_layer_id {
                Some(id) => id,
                None => return,
            };

            // Delete the selected geometry (region/marquee/click all populate the same sets).
            // Selecting a fill also selects its boundary edges, so removing edges alone would leave
            // the fill's face orphaned — free the fills too so the whole shape is removed.
            let edge_ids: Vec<lightningbeam_core::vector_graph::EdgeId> =
                self.selection.selected_edges().iter().copied().collect();
            let fill_ids: Vec<lightningbeam_core::vector_graph::FillId> =
                self.selection.selected_fills().iter().copied().collect();

            if !edge_ids.is_empty() || !fill_ids.is_empty() {
                let document = self.action_executor.document();
                if let Some(lightningbeam_core::layer::AnyLayer::Vector(vector_layer)) = document.get_layer(&active_layer_id) {
                    if let Some(graph_before) = vector_layer.graph_at_time(self.playback_time) {
                        let mut graph_after = graph_before.clone();
                        // Free fills first so their boundary edges become unreferenced and are GC'd.
                        for fill_id in &fill_ids {
                            if !graph_after.fill(*fill_id).deleted {
                                graph_after.free_fill(*fill_id);
                            }
                        }
                        for edge_id in &edge_ids {
                            if !graph_after.edge(*edge_id).deleted {
                                graph_after.remove_edge(*edge_id);
                            }
                        }
                        graph_after.gc_isolated_vertices();

                        let action = lightningbeam_core::actions::ModifyGraphAction::new(
                            active_layer_id,
                            self.playback_time,
                            graph_before.clone(),
                            graph_after,
                            "Delete",
                        );

                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Delete geometry failed: {}", e);
                        }
                    }
                }
            }

            self.selection.clear_geometry_selection();
            self.focus = lightningbeam_core::selection::FocusSelection::None;
        }
    }

    /// Paste from clipboard
    fn clipboard_paste(&mut self) {
        use lightningbeam_core::clipboard::ClipboardContent;
        use lightningbeam_core::layer::AnyLayer;

        // Resolve content from all sources:
        //   1. Internal cache (ClipboardContent, any type)
        //   2. System clipboard JSON (LIGHTNINGBEAM_CLIPBOARD: prefix)
        //   3. System clipboard image — only attempted when the active layer is raster,
        //      since non-raster layers have no way to consume raw pixel data
        let active_is_raster = self.active_layer_id
            .and_then(|id| self.action_executor.document().get_layer(&id))
            .map_or(false, |l| matches!(l, AnyLayer::Raster(_)));

        let content = self.clipboard_manager.paste().or_else(|| {
            if active_is_raster {
                self.clipboard_manager.try_get_raster_image()
                    .map(|(pixels, width, height)| ClipboardContent::RasterPixels { pixels, width, height })
            } else {
                None
            }
        });
        let Some(content) = content else { return };

        // Regenerate IDs for the paste (no-op for RasterPixels)
        let (new_content, _id_map) = content.with_regenerated_ids();

        match new_content {
            ClipboardContent::ClipInstances {
                layer_type,
                mut instances,
                audio_clips,
                video_clips,
                vector_clips,
                image_assets,
            } => {
                let active_layer_id = match self.active_layer_id {
                    Some(id) => id,
                    None => return,
                };

                // Verify layer compatibility
                {
                    let document = self.action_executor.document();
                    let layer = match document.get_layer(&active_layer_id) {
                        Some(l) => l,
                        None => return,
                    };
                    if !layer_type.is_compatible(layer) {
                        eprintln!("Cannot paste: incompatible layer type");
                        return;
                    }
                }

                // Add clip definitions to document (they have new IDs from regeneration)
                {
                    let document = self.action_executor.document_mut();
                    for (_id, clip) in &audio_clips {
                        document.audio_clips.insert(clip.id, clip.clone());
                    }
                    for (_id, clip) in &video_clips {
                        document.video_clips.insert(clip.id, clip.clone());
                    }
                    for (_id, clip) in &vector_clips {
                        document.vector_clips.insert(clip.id, clip.clone());
                    }
                    for (_id, asset) in &image_assets {
                        document.image_assets.insert(asset.id, asset.clone());
                    }
                }

                // Position instances at playhead, preserving relative offsets
                if !instances.is_empty() {
                    let min_start = instances
                        .iter()
                        .map(|i| i.timeline_start)
                        .fold(Beats(f64::INFINITY), |a, b| a.min(b));
                    let playhead_beats = self.action_executor.document().tempo_map().seconds_to_beats(Seconds(self.playback_time));
                    let offset = playhead_beats - min_start;
                    for inst in &mut instances {
                        inst.timeline_start = (inst.timeline_start + offset).max(Beats::ZERO);
                    }
                }

                // Add each instance via action (handles overlap avoidance)
                let new_ids: Vec<Uuid> = instances.iter().map(|i| i.id).collect();

                for instance in instances {
                    let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                        active_layer_id,
                        instance,
                    );

                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        let mut backend_context = lightningbeam_core::action::BackendContext {
                            audio_controller: Some(&mut *controller),
                            layer_to_track_map: &self.layer_to_track_map,
                            clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                        };
                        if let Err(e) = self
                            .action_executor
                            .execute_with_backend(Box::new(action), &mut backend_context)
                        {
                            eprintln!("Paste clip failed: {}", e);
                        }
                    } else {
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Paste clip failed: {}", e);
                        }
                    }
                }

                // Select pasted clips
                self.selection.clear_clip_instances();
                for id in new_ids {
                    self.selection.add_clip_instance(id);
                }
            }
            ClipboardContent::VectorGeometry { .. } => {
                // TODO: VectorGraph paste — import_from requires Dcel;
                // port when clipboard is migrated from Dcel to VectorGraph.
                eprintln!("Paste vector geometry: not yet ported to VectorGraph");
            }
            ClipboardContent::Layers { .. } => {
                // TODO: insert copied layers as siblings at the current selection point.
            }
            ClipboardContent::AudioNodes { .. } => {
                // TODO: add nodes to the target layer's audio graph with new IDs and
                // sync to the DAW backend.
            }
            ClipboardContent::MidiNotes { .. } => {
                // MIDI notes are pasted directly in the piano roll pane, not here
            }
            ClipboardContent::RasterPixels { pixels, width, height } => {
                let Some(layer_id) = self.active_layer_id else { return };

                // Commit any pre-existing floating selection FIRST so that
                // canvas_before captures the fully-composited state (not the
                // pre-commit state, which would corrupt the undo snapshot).
                self.commit_raster_floating();

                // Re-borrow the document after commit to get post-commit state.
                let document = self.action_executor.document();
                let layer = document.get_layer(&layer_id);
                let Some(AnyLayer::Raster(rl)) = layer else { return };
                let Some(kf) = rl.keyframe_at(self.playback_time) else { return };

                // Paste position: top-left of the current raster selection if any,
                // otherwise the canvas origin.
                let (paste_x, paste_y) = self.selection.raster_selection
                    .as_ref()
                    .map(|s| { let (x0, y0, _, _) = s.bounding_rect(); (x0, y0) })
                    .unwrap_or((0, 0));

                // Snapshot canvas AFTER commit for correct undo on commit / restore on cancel.
                let canvas_before = kf.raw_pixels.clone();
                let canvas_w = kf.width;
                let canvas_h = kf.height;
                let _ = kf; // release immutable borrow before taking mutable

                use lightningbeam_core::selection::{RasterFloatingSelection, RasterSelection};
                self.selection.raster_floating = Some(RasterFloatingSelection {
                    pixels: std::sync::Arc::new(pixels),
                    width,
                    height,
                    x: paste_x,
                    y: paste_y,
                    layer_id,
                    time: self.playback_time,
                    canvas_before: std::sync::Arc::new(canvas_before),
                    canvas_id: uuid::Uuid::new_v4(),
                });
                // Update the marquee to show the floating selection bounds.
                self.selection.raster_selection = Some(RasterSelection::Rect(
                    paste_x,
                    paste_y,
                    paste_x + width as i32,
                    paste_y + height as i32,
                ));
                let _ = (canvas_w, canvas_h); // used only to satisfy borrow checker above
            }
        }
    }

    /// Send the selected clip/group instances to the back or front of their layer's stacking order.
    fn reorder_selected_clips(&mut self, to_front: bool) {
        use lightningbeam_core::layer::AnyLayer;
        let Some(layer_id) = self.active_layer_id else {
            return;
        };
        let ids: Vec<uuid::Uuid> = {
            let document = self.action_executor.document();
            let Some(layer) = document.get_layer(&layer_id) else {
                return;
            };
            let instances: &[lightningbeam_core::clip::ClipInstance] = match layer {
                AnyLayer::Vector(l) => &l.clip_instances,
                AnyLayer::Audio(l) => &l.clip_instances,
                AnyLayer::Video(l) => &l.clip_instances,
                AnyLayer::Effect(l) => &l.clip_instances,
                _ => &[],
            };
            instances
                .iter()
                .filter(|ci| self.selection.contains_clip_instance(&ci.id))
                .map(|ci| ci.id)
                .collect()
        };
        if ids.is_empty() {
            return;
        }
        let action = lightningbeam_core::actions::ReorderClipInstancesAction::new(layer_id, ids, to_front);
        if let Err(e) = self.action_executor.execute(Box::new(action)) {
            eprintln!("Failed to reorder clip instances: {e}");
        }
    }

    /// Duplicate the selected clip instances on the active layer.
    /// Each duplicate is placed immediately after the original clip.
    fn duplicate_selected_clips(&mut self) {
        use lightningbeam_core::layer::AnyLayer;
        use lightningbeam_core::actions::AddClipInstanceAction;

        let active_layer_id = match self.active_layer_id {
            Some(id) => id,
            None => return,
        };

        // Gather all data from document in a scoped block so the borrow is released
        let (_clips_to_duplicate, midi_clip_replacements, duplicates, cache_copies) = {
            let document = self.action_executor.document();
            let selection = &self.selection;

            // Find selected clip instances on the active layer
            let clips_to_duplicate: Vec<lightningbeam_core::clip::ClipInstance> = {
                let layer = match document.get_layer(&active_layer_id) {
                    Some(l) => l,
                    None => return,
                };
                let instances: &[lightningbeam_core::clip::ClipInstance] = match layer {
                    AnyLayer::Vector(vl) => &vl.clip_instances,
                    AnyLayer::Audio(al) => &al.clip_instances,
                    AnyLayer::Video(vl) => &vl.clip_instances,
                    AnyLayer::Effect(el) => &el.clip_instances,
                    AnyLayer::Group(_) | AnyLayer::Raster(_) | AnyLayer::Text(_) => &[],
                };
                instances.iter()
                    .filter(|ci| selection.contains_clip_instance(&ci.id))
                    .cloned()
                    .collect()
            };

            if clips_to_duplicate.is_empty() {
                return;
            }

            // For MIDI clips, duplicate the backend clip to get independent note data.
            let mut midi_clip_replacements: std::collections::HashMap<uuid::Uuid, (uuid::Uuid, lightningbeam_core::clip::AudioClip)> = std::collections::HashMap::new();
            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                for original in &clips_to_duplicate {
                    if let Some(clip) = document.audio_clips.get(&original.clip_id) {
                        if let lightningbeam_core::clip::AudioClipType::Midi { midi_clip_id } = clip.clip_type {
                            let query = daw_backend::command::types::Query::DuplicateMidiClipSync(midi_clip_id);
                            if let Ok(daw_backend::command::types::QueryResponse::MidiClipDuplicated(Ok(new_midi_id))) = controller.send_query(query) {
                                let new_clip_def_id = uuid::Uuid::new_v4();
                                let mut new_clip = clip.clone();
                                new_clip.id = new_clip_def_id;
                                new_clip.clip_type = lightningbeam_core::clip::AudioClipType::Midi { midi_clip_id: new_midi_id };
                                new_clip.name = format!("{} (copy)", clip.name);
                                midi_clip_replacements.insert(original.clip_id, (new_clip_def_id, new_clip));
                            }
                        }
                    }
                }
            }

            // Build duplicate instances
            let duplicates: Vec<lightningbeam_core::clip::ClipInstance> = clips_to_duplicate.iter().map(|original| {
                let mut duplicate = original.clone();
                duplicate.id = uuid::Uuid::new_v4();
                let clip_duration = document.get_clip_duration(&original.clip_id).unwrap_or(Seconds(1.0));
                let effective_duration = original.effective_duration(clip_duration, document.tempo_map());
                duplicate.timeline_start = original.timeline_start + effective_duration;
                if let Some((new_clip_def_id, _)) = midi_clip_replacements.get(&original.clip_id) {
                    duplicate.clip_id = *new_clip_def_id;
                }
                duplicate
            }).collect();

            // Collect old->new MIDI clip ID pairs for cache copying
            let cache_copies: Vec<(u32, u32)> = clips_to_duplicate.iter()
                .filter_map(|original| {
                    let (_, new_clip) = midi_clip_replacements.get(&original.clip_id)?;
                    let old_midi_id = document.audio_clips.get(&original.clip_id)?.midi_clip_id()?;
                    if let lightningbeam_core::clip::AudioClipType::Midi { midi_clip_id: new_midi_id } = new_clip.clip_type {
                        Some((old_midi_id, new_midi_id))
                    } else {
                        None
                    }
                })
                .collect();

            (clips_to_duplicate, midi_clip_replacements, duplicates, cache_copies)
        };
        // document borrow is now released

        let new_ids: Vec<uuid::Uuid> = duplicates.iter().map(|d| d.id).collect();

        // Copy MIDI event cache entries
        for (old_midi_id, new_midi_id) in cache_copies {
            if let Some(events) = self.midi_event_cache.get(&old_midi_id).cloned() {
                self.midi_event_cache.insert(new_midi_id, events);
            }
        }

        // Register the new MIDI clip definitions in the document
        for (_, (new_clip_def_id, new_clip)) in &midi_clip_replacements {
            self.action_executor.document_mut().audio_clips.insert(*new_clip_def_id, new_clip.clone());
        }

        for duplicate in duplicates {
            let action = AddClipInstanceAction::new(active_layer_id, duplicate);

            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: &self.layer_to_track_map,
                    clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                };
                if let Err(e) = self.action_executor.execute_with_backend(Box::new(action), &mut backend_context) {
                    eprintln!("Duplicate clip failed: {}", e);
                }
            } else {
                if let Err(e) = self.action_executor.execute(Box::new(action)) {
                    eprintln!("Duplicate clip failed: {}", e);
                }
            }
        }

        // Select the new duplicates instead of the originals
        self.selection.clear_clip_instances();
        for id in new_ids {
            self.selection.add_clip_instance(id);
        }
    }

    fn switch_layout(&mut self, index: usize) {
        self.current_layout_index = index;
        self.current_layout = self.layouts[index].layout.clone();

        // Clear pane instances so they rebuild with new layout
        self.pane_instances.clear();
    }

    fn apply_layout_action(&mut self, action: LayoutAction) {
        match action {
            LayoutAction::SplitHorizontal(path, percent) => {
                split_node(&mut self.current_layout, &path, true, percent);
            }
            LayoutAction::SplitVertical(path, percent) => {
                split_node(&mut self.current_layout, &path, false, percent);
            }
            LayoutAction::RemoveSplit(path) => {
                remove_split(&mut self.current_layout, &path);
            }
            LayoutAction::EnterSplitPreviewHorizontal => {
                self.split_preview_mode = SplitPreviewMode::Active {
                    is_horizontal: false, // horizontal divider = vertical grid (top/bottom)
                    hovered_pane: None,
                    split_percent: 50.0,
                };
            }
            LayoutAction::EnterSplitPreviewVertical => {
                self.split_preview_mode = SplitPreviewMode::Active {
                    is_horizontal: true, // vertical divider = horizontal grid (left/right)
                    hovered_pane: None,
                    split_percent: 50.0,
                };
            }
        }
    }

    /// Resolve a pending region-select cut once its selection has been cleared.
    ///
    /// A region/lasso selection cuts the geometry (an undoable `"Region select"` action)
    /// and selects the resulting sub-pieces. When that selection is deselected:
    ///   - if nothing was edited in between, the cut(s) are healed (undone) so the lasso
    ///     was non-destructive — the shape returns to whole;
    ///   - if anything was edited, everything is kept (the edits — and the cut they rely
    ///     on — stay on the undo stack), i.e. the change is "merged" in place.
    /// We tell the two apart by walking the undo stack down to the recorded base depth and
    /// only undoing while the top action is itself a region-select cut; the first non-cut
    /// action (an edit) stops the heal and leaves everything intact.
    fn resolve_pending_region_cut(&mut self) {
        let Some(base) = self.pending_region_cut_base else { return };
        // Only resolve once the region selection has actually been deselected.
        if self.selection.has_geometry_selection() {
            return;
        }
        while self.action_executor.undo_depth() > base {
            if self.action_executor.undo_description().as_deref() == Some("Region select") {
                let _ = self.action_executor.undo();
            } else {
                break; // an edit sits on top → the user changed something; keep it all.
            }
        }
        self.pending_region_cut_base = None;
    }

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            // File menu
            MenuAction::NewFile => {
                println!("Menu: New File");
                self.request_switch(PendingAction::NewFile);
            }
            MenuAction::NewWindow => {
                println!("Menu: New Window");
                // TODO: Implement new window (requires multi-window support)
            }
            MenuAction::Save => {
                use rfd::FileDialog;

                // A recovered file has no real home yet — Save behaves as Save As so the work lands
                // where the user wants, not back in the app data dir.
                let real_path = self
                    .current_file_path
                    .clone()
                    .filter(|p| !AutosaveState::is_recovery_path(p));
                if let Some(path) = real_path {
                    // Save to existing path
                    self.save_to_file(path);
                } else {
                    // No real path (untitled or recovered): fall through to Save As
                    if let Some(path) = FileDialog::new()
                        .add_filter("Lightningbeam Project", &["beam"])
                        .set_file_name("Untitled.beam")
                        .save_file()
                    {
                        self.save_to_file(path);
                    }
                }
            }
            MenuAction::SaveAs => {
                use rfd::FileDialog;

                let dialog = FileDialog::new()
                    .add_filter("Lightningbeam Project", &["beam"])
                    .set_file_name("Untitled.beam");

                // Default to the current file's directory — but not the recovery dir (a recovered
                // file's parent), which the user never chose and shouldn't be steered back into.
                let dialog = match self
                    .current_file_path
                    .as_ref()
                    .filter(|p| !AutosaveState::is_recovery_path(p))
                    .and_then(|p| p.parent())
                {
                    Some(parent) => dialog.set_directory(parent),
                    None => dialog,
                };

                if let Some(path) = dialog.save_file() {
                    self.save_to_file(path);
                }
            }
            MenuAction::OpenFile => {
                use rfd::FileDialog;

                // Pick the file first, then (if there are unsaved changes) prompt to save.
                if let Some(path) = FileDialog::new()
                    .add_filter("Lightningbeam Project", &["beam"])
                    .pick_file()
                {
                    self.request_switch(PendingAction::Open(path));
                }
            }
            MenuAction::OpenRecent(index) => {
                let recent_files = self.config.get_recent_files();

                if let Some(path) = recent_files.get(index) {
                    self.request_switch(PendingAction::Open(path.clone()));
                }
            }
            MenuAction::ClearRecentFiles => {
                self.config.clear_recent_files();
                self.update_recent_files_menu();
            }
            MenuAction::Revert => {
                println!("Menu: Revert");
                // TODO: Implement revert
            }
            MenuAction::Import | MenuAction::ImportToLibrary => {
                let auto_place = matches!(action, MenuAction::Import);

                // TODO: Implement auto-placement when auto_place is true

                use lightningbeam_core::file_types::*;
                use rfd::FileDialog;

                // Build file filter from extension constants
                let all_extensions: Vec<&str> = all_supported_extensions();

                // Build dialog with filters in order based on last used filter
                // The first filter added is the default in most file dialogs
                let mut dialog = FileDialog::new().set_title("Import Asset");

                // Add filters in order, with the last-used filter first
                match self.last_import_filter {
                    ImportFilter::All => {
                        dialog = dialog
                            .add_filter("All Supported", &all_extensions)
                            .add_filter("Images", IMAGE_EXTENSIONS)
                            .add_filter("Audio", AUDIO_EXTENSIONS)
                            .add_filter("Video", VIDEO_EXTENSIONS)
                            .add_filter("MIDI", MIDI_EXTENSIONS);
                    }
                    ImportFilter::Images => {
                        dialog = dialog
                            .add_filter("Images", IMAGE_EXTENSIONS)
                            .add_filter("All Supported", &all_extensions)
                            .add_filter("Audio", AUDIO_EXTENSIONS)
                            .add_filter("Video", VIDEO_EXTENSIONS)
                            .add_filter("MIDI", MIDI_EXTENSIONS);
                    }
                    ImportFilter::Audio => {
                        dialog = dialog
                            .add_filter("Audio", AUDIO_EXTENSIONS)
                            .add_filter("All Supported", &all_extensions)
                            .add_filter("Images", IMAGE_EXTENSIONS)
                            .add_filter("Video", VIDEO_EXTENSIONS)
                            .add_filter("MIDI", MIDI_EXTENSIONS);
                    }
                    ImportFilter::Video => {
                        dialog = dialog
                            .add_filter("Video", VIDEO_EXTENSIONS)
                            .add_filter("All Supported", &all_extensions)
                            .add_filter("Images", IMAGE_EXTENSIONS)
                            .add_filter("Audio", AUDIO_EXTENSIONS)
                            .add_filter("MIDI", MIDI_EXTENSIONS);
                    }
                    ImportFilter::Midi => {
                        dialog = dialog
                            .add_filter("MIDI", MIDI_EXTENSIONS)
                            .add_filter("All Supported", &all_extensions)
                            .add_filter("Images", IMAGE_EXTENSIONS)
                            .add_filter("Audio", AUDIO_EXTENSIONS)
                            .add_filter("Video", VIDEO_EXTENSIONS);
                    }
                }

                let file = dialog.pick_file();

                if let Some(path) = file {
                    let _import_timer = std::time::Instant::now();
                    // Get extension and detect file type
                    let extension = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");

                    // SVG imports as a new vector layer (not a placeable asset).
                    let imported_asset = if extension.eq_ignore_ascii_case("svg") {
                        self.import_svg_file(&path);
                        None
                    } else { match get_file_type(extension) {
                        Some(FileType::Image) => {
                            self.last_import_filter = ImportFilter::Images;
                            self.import_image(&path)
                        }
                        Some(FileType::Audio) => {
                            self.last_import_filter = ImportFilter::Audio;
                            self.import_audio(&path)
                        }
                        Some(FileType::Video) => {
                            self.last_import_filter = ImportFilter::Video;
                            self.import_video(&path)
                        }
                        Some(FileType::Midi) => {
                            self.last_import_filter = ImportFilter::Midi;
                            self.import_midi(&path)
                        }
                        Some(FileType::Vector) => None, // handled by the svg intercept above
                        None => {
                            println!("Unsupported file type: {}", extension);
                            None
                        }
                    } };

                    eprintln!("[TIMING] import took {:.1}ms", _import_timer.elapsed().as_secs_f64() * 1000.0);
                    // Auto-place if this is "Import" (not "Import to Library")
                    if auto_place {
                        if let Some(asset_info) = imported_asset {
                            let _place_timer = std::time::Instant::now();
                            self.auto_place_asset(asset_info);
                            eprintln!("[TIMING] auto_place took {:.1}ms", _place_timer.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    eprintln!("[TIMING] total import+place took {:.1}ms", _import_timer.elapsed().as_secs_f64() * 1000.0);
                }
            }
            MenuAction::Export => {
                println!("Menu: Export");
                let timeline_endpoint = self.action_executor.document().calculate_timeline_endpoint();
                let project_name = self.current_file_path.as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| self.action_executor.document().name.clone());

                // Build document hint for smart export-type defaulting.
                let hint = {
                    use lightningbeam_core::layer::AnyLayer;
                    use export::dialog::DocumentHint;
                    fn scan(layers: &[AnyLayer], hint: &mut DocumentHint) {
                        for l in layers {
                            match l {
                                AnyLayer::Video(_)  => hint.has_video  = true,
                                AnyLayer::Audio(_)  => hint.has_audio  = true,
                                AnyLayer::Raster(_) => hint.has_raster = true,
                                AnyLayer::Vector(_) | AnyLayer::Effect(_) | AnyLayer::Text(_) => hint.has_vector = true,
                                AnyLayer::Group(g)  => scan(&g.children, hint),
                            }
                        }
                    }
                    let doc = self.action_executor.document();
                    let mut h = DocumentHint {
                        has_video:    false,
                        has_audio:    false,
                        has_raster:   false,
                        has_vector:   false,
                        current_time: doc.current_time,
                    };
                    scan(&doc.root.children, &mut h);
                    h
                };

                self.export_dialog.open(
                    timeline_endpoint,
                    &project_name,
                    &hint,
                    &self.config.last_audio_artist,
                    &self.config.last_audio_album,
                );
            }
            MenuAction::Quit => {
                println!("Menu: Quit");
                std::process::exit(0);
            }

            // Edit menu
            MenuAction::Undo => {
                // An uncommitted floating selection (paste not yet merged) lives
                // outside the action stack.  Cancelling it IS the undo — dismiss
                // it and don't pop anything from the stack.
                if self.selection.raster_floating.is_some() {
                    self.cancel_raster_floating();
                    return;
                }
                // Page in the target raster frame first so its dirty-rect diff has a
                // resident base to apply onto.
                let hint = self.action_executor.peek_undo_raster_hint();
                self.ensure_raster_resident_for_undo(hint);
                let undo_succeeded = if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    let mut backend_context = lightningbeam_core::action::BackendContext {
                        audio_controller: Some(&mut *controller),
                        layer_to_track_map: &self.layer_to_track_map,
                        clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                    };
                    match self.action_executor.undo_with_backend(&mut backend_context) {
                        Ok(true) => {
                            println!("Undid: {}", self.action_executor.redo_description().unwrap_or_default());
                            true
                        }
                        Ok(false) => { println!("Nothing to undo"); false }
                        Err(e) => { eprintln!("Undo failed: {}", e); false }
                    }
                } else {
                    match self.action_executor.undo() {
                        Ok(true) => {
                            println!("Undid: {}", self.action_executor.redo_description().unwrap_or_default());
                            true
                        }
                        Ok(false) => { println!("Nothing to undo"); false }
                        Err(e) => { eprintln!("Undo failed: {}", e); false }
                    }
                };
                // Rebuild MIDI cache after undo (backend_context dropped, borrows released)
                if undo_succeeded {
                    let multi = self.action_executor.last_redo_all_midi_events();
                    if !multi.is_empty() {
                        for (clip_id, events) in multi {
                            self.midi_event_cache.insert(clip_id, events);
                        }
                    } else if let Some((clip_id, events)) = self.action_executor.last_redo_midi_events()
                        .map(|(id, ev)| (id, ev.to_vec()))
                    {
                        self.midi_event_cache.insert(clip_id, events);
                    } else {
                        let midi_update = self.action_executor.last_redo_midi_notes()
                            .map(|(id, notes)| (id, notes.to_vec()));
                        if let Some((clip_id, notes)) = midi_update {
                            self.rebuild_midi_cache_entry(clip_id, &notes);
                        }
                    }
                    // Stale vertex/edge/face IDs from before the undo would
                    // crash selection rendering on the restored (smaller) DCEL.
                    self.selection.clear_geometry_selection();
                }
            }
            MenuAction::Redo => {
                let hint = self.action_executor.peek_redo_raster_hint();
                self.ensure_raster_resident_for_undo(hint);
                let redo_succeeded = if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    let mut backend_context = lightningbeam_core::action::BackendContext {
                        audio_controller: Some(&mut *controller),
                        layer_to_track_map: &self.layer_to_track_map,
                        clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                    };
                    match self.action_executor.redo_with_backend(&mut backend_context) {
                        Ok(true) => {
                            println!("Redid: {}", self.action_executor.undo_description().unwrap_or_default());
                            true
                        }
                        Ok(false) => { println!("Nothing to redo"); false }
                        Err(e) => { eprintln!("Redo failed: {}", e); false }
                    }
                } else {
                    match self.action_executor.redo() {
                        Ok(true) => {
                            println!("Redid: {}", self.action_executor.undo_description().unwrap_or_default());
                            true
                        }
                        Ok(false) => { println!("Nothing to redo"); false }
                        Err(e) => { eprintln!("Redo failed: {}", e); false }
                    }
                };
                // Rebuild MIDI cache after redo (backend_context dropped, borrows released)
                if redo_succeeded {
                    let multi = self.action_executor.last_undo_all_midi_events();
                    if !multi.is_empty() {
                        for (clip_id, events) in multi {
                            self.midi_event_cache.insert(clip_id, events);
                        }
                    } else if let Some((clip_id, events)) = self.action_executor.last_undo_midi_events()
                        .map(|(id, ev)| (id, ev.to_vec()))
                    {
                        self.midi_event_cache.insert(clip_id, events);
                    } else {
                        let midi_update = self.action_executor.last_undo_midi_notes()
                            .map(|(id, notes)| (id, notes.to_vec()));
                        if let Some((clip_id, notes)) = midi_update {
                            self.rebuild_midi_cache_entry(clip_id, &notes);
                        }
                    }
                    self.selection.clear_geometry_selection();
                }
            }
            MenuAction::Cut => {
                self.clipboard_copy_selection();
                self.clipboard_delete_selection();
            }
            MenuAction::Copy => {
                self.clipboard_copy_selection();
            }
            MenuAction::Paste => {
                self.clipboard_paste();
            }
            MenuAction::Delete => {
                self.clipboard_delete_selection();
            }
            MenuAction::SelectAll => {
                println!("Menu: Select All");
                // TODO: Implement select all
            }
            MenuAction::SelectNone => {
                println!("Menu: Select None");
                // TODO: Implement select none
            }
            MenuAction::Preferences => {
                self.preferences_dialog.open(&self.config, &self.theme);
            }

            // Modify menu
            MenuAction::Group => {
                match &self.focus {
                    lightningbeam_core::selection::FocusSelection::Layers(ids) if ids.len() >= 2 => {
                        let parent_group_id = find_parent_group_id(self.action_executor.document(), &ids[0]);
                        let group_id = uuid::Uuid::new_v4();
                        let action = lightningbeam_core::actions::GroupLayersAction::new(
                            ids.clone(), parent_group_id, group_id,
                        );
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Failed to group layers: {}", e);
                        } else {
                            self.active_layer_id = Some(group_id);
                            self.focus = lightningbeam_core::selection::FocusSelection::Layers(vec![group_id]);
                        }
                    }
                    lightningbeam_core::selection::FocusSelection::Nodes(_) => {
                        self.pending_node_group = true;
                    }
                    _ => {
                        // Group selected geometry into a group clip + clip instance.
                        if let Some(layer_id) = self.active_layer_id {
                            if self.selection.has_geometry_selection() {
                                let fills: Vec<_> = self.selection.selected_fills().iter().copied().collect();
                                let edges: Vec<_> = self.selection.selected_edges().iter().copied().collect();
                                let clip_id = uuid::Uuid::new_v4();
                                let instance_id = uuid::Uuid::new_v4();
                                let action = lightningbeam_core::actions::GroupAction::new(
                                    layer_id, self.playback_time, fills, edges, clip_id, instance_id,
                                );
                                if let Err(e) = self.action_executor.execute(Box::new(action)) {
                                    eprintln!("Failed to group: {}", e);
                                } else {
                                    self.selection.clear();
                                    self.selection.add_clip_instance(instance_id);
                                }
                            }
                            // (Grouping existing clip instances is a separate op, not wired.)
                        }
                    }
                }
            }
            MenuAction::ConvertToMovieClip => {
                if let Some(layer_id) = self.active_layer_id {
                    if self.selection.has_geometry_selection() {
                        let fills: Vec<_> = self.selection.selected_fills().iter().copied().collect();
                        let edges: Vec<_> = self.selection.selected_edges().iter().copied().collect();
                        let clip_id = uuid::Uuid::new_v4();
                        let instance_id = uuid::Uuid::new_v4();
                        let action = lightningbeam_core::actions::ConvertToMovieClipAction::new(
                            layer_id, self.playback_time, fills, edges, clip_id, instance_id,
                        );
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Failed to convert to movie clip: {}", e);
                        } else {
                            self.selection.clear();
                            self.selection.add_clip_instance(instance_id);
                        }
                    }
                }
            }
            MenuAction::SendToBack => {
                self.reorder_selected_clips(false);
            }
            MenuAction::BringToFront => {
                self.reorder_selected_clips(true);
            }
            MenuAction::SplitClip => {
                self.split_clips_at_playhead();
            }
            MenuAction::DuplicateClip => {
                self.duplicate_selected_clips();
            }

            // Layer menu
            MenuAction::AddLayer => {
                // Create a new vector layer with a default name
                let editing_clip_id = self.editing_context.current_clip_id();
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                let layer_count = context_layers.len();
                let layer_name = format!("Layer {}", layer_count + 1);

                let action = lightningbeam_core::actions::AddLayerAction::new_vector(layer_name)
                    .with_target_clip(editing_clip_id);
                let _ = self.action_executor.execute(Box::new(action));

                // Select the newly created layer (last in context)
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                if let Some(last_layer) = context_layers.last() {
                    self.active_layer_id = Some(last_layer.id());
                }
            }
            MenuAction::AddVideoLayer => {
                let editing_clip_id = self.editing_context.current_clip_id();
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                let layer_number = context_layers.len() + 1;
                let layer_name = format!("Video {}", layer_number);
                let new_layer = lightningbeam_core::layer::AnyLayer::Video(
                    lightningbeam_core::layer::VideoLayer::new(&layer_name)
                );

                let action = lightningbeam_core::actions::AddLayerAction::new(new_layer)
                    .with_target_clip(editing_clip_id);
                let _ = self.action_executor.execute(Box::new(action));

                // Set it as the active layer
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                if let Some(last_layer) = context_layers.last() {
                    self.active_layer_id = Some(last_layer.id());
                }
            }
            MenuAction::AddAudioTrack => {
                // Create a new sampled audio layer with a default name
                let editing_clip_id = self.editing_context.current_clip_id();
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                let layer_count = context_layers.len();
                let layer_name = format!("Audio Track {}", layer_count + 1);

                // Create audio layer in document
                let audio_layer = AudioLayer::new_sampled(layer_name.clone());
                let action = lightningbeam_core::actions::AddLayerAction::new(AnyLayer::Audio(audio_layer))
                    .with_target_clip(editing_clip_id);
                let _ = self.action_executor.execute(Box::new(action));

                // Get the newly created layer ID
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                if let Some(last_layer) = context_layers.last() {
                    let layer_id = last_layer.id();
                    self.active_layer_id = Some(layer_id);

                    // If inside a clip, ensure a metatrack exists for it
                    let parent_track = editing_clip_id.and_then(|cid| self.ensure_metatrack_for_clip(cid));

                    // Create corresponding daw-backend audio track
                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        match controller.create_audio_track_sync(layer_name.clone(), parent_track) {
                            Ok(track_id) => {
                                // Store bidirectional mapping
                                self.layer_to_track_map.insert(layer_id, track_id);
                                self.track_to_layer_map.insert(track_id, layer_id);
                                println!("✅ Created {} (backend TrackId: {}, parent: {:?})", layer_name, track_id, parent_track);
                            }
                            Err(e) => {
                                eprintln!("⚠️  Failed to create daw-backend audio track for {}: {}", layer_name, e);
                                eprintln!("   Layer created but will be silent until backend track is available");
                            }
                        }
                    }
                }
            }
            MenuAction::AddMidiTrack => {
                // Create a new MIDI audio layer with a default name
                let editing_clip_id = self.editing_context.current_clip_id();
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                let layer_count = context_layers.len();
                let layer_name = format!("MIDI Track {}", layer_count + 1);

                // Create MIDI layer in document
                let midi_layer = AudioLayer::new_midi(layer_name.clone());
                let action = lightningbeam_core::actions::AddLayerAction::new(AnyLayer::Audio(midi_layer))
                    .with_target_clip(editing_clip_id);
                let _ = self.action_executor.execute(Box::new(action));

                // Get the newly created layer ID
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                if let Some(last_layer) = context_layers.last() {
                    let layer_id = last_layer.id();
                    self.active_layer_id = Some(layer_id);

                    // If inside a clip, ensure a metatrack exists for it
                    let parent_track = editing_clip_id.and_then(|cid| self.ensure_metatrack_for_clip(cid));

                    // Create corresponding daw-backend MIDI track
                    if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                        match controller.create_midi_track_sync(layer_name.clone(), parent_track) {
                            Ok(track_id) => {
                                // Store bidirectional mapping
                                self.layer_to_track_map.insert(layer_id, track_id);
                                self.track_to_layer_map.insert(track_id, layer_id);

                                // Load default instrument into the track
                                if let Err(e) = default_instrument::load_default_instrument(&mut *controller, track_id) {
                                    eprintln!("⚠️  Failed to load default instrument for {}: {}", layer_name, e);
                                } else {
                                    println!("✅ Created {} (backend TrackId: {}, instrument: {})",
                                             layer_name, track_id, default_instrument::default_instrument_name());
                                }
                            }
                            Err(e) => {
                                eprintln!("⚠️  Failed to create daw-backend MIDI track for {}: {}", layer_name, e);
                                eprintln!("   Layer created but will be silent until backend track is available");
                            }
                        }
                    } else {
                        println!("⚠️  Audio engine not initialized - {} created but will be silent", layer_name);
                    }
                }
            }
            MenuAction::AddRasterLayer => {
                use lightningbeam_core::raster_layer::RasterLayer;
                let editing_clip_id = self.editing_context.current_clip_id();
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                let layer_number = context_layers.len() + 1;
                let layer_name = format!("Raster {}", layer_number);

                let doc = self.action_executor.document();
                let (doc_w, doc_h) = (doc.width as u32, doc.height as u32);
                let _ = doc;
                let mut layer = RasterLayer::new(layer_name);
                layer.ensure_keyframe_at(self.playback_time, doc_w, doc_h);
                let action = lightningbeam_core::actions::AddLayerAction::new(AnyLayer::Raster(layer))
                    .with_target_clip(editing_clip_id);
                let _ = self.action_executor.execute(Box::new(action));

                // Set newly created layer as active
                let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
                if let Some(last_layer) = context_layers.last() {
                    self.active_layer_id = Some(last_layer.id());
                }
            }
            MenuAction::AddTestClip => {
                // Create a test vector clip and add it to the library (not to timeline)
                use lightningbeam_core::clip::VectorClip;
                use lightningbeam_core::layer::{VectorLayer, AnyLayer};
                use lightningbeam_core::shape::{Shape, ShapeColor};
                use kurbo::{Circle, Rect, Shape as KurboShape};

                // Generate unique name based on existing clip count
                let clip_count = self.action_executor.document().vector_clips.len();
                let clip_name = format!("Test Clip {}", clip_count + 1);

                let mut test_clip = VectorClip::new(&clip_name, 400.0, 400.0, 5.0);

                // Create a layer with some shapes
                let layer = VectorLayer::new("Shapes");

                // Create a red circle shape
                let circle_path = Circle::new((100.0, 100.0), 50.0).to_path(0.1);
                let mut circle_shape = Shape::new(circle_path);
                circle_shape.fill_color = Some(ShapeColor::rgb(255, 0, 0));

                // Create a blue rectangle shape
                let rect_path = Rect::new(200.0, 50.0, 350.0, 150.0).to_path(0.1);
                let mut rect_shape = Shape::new(rect_path);
                rect_shape.fill_color = Some(ShapeColor::rgb(0, 0, 255));

                // TODO: DCEL - test shape creation not yet implemented
                let _ = (circle_shape, rect_shape);

                // Add the layer to the clip
                test_clip.layers.add_root(AnyLayer::Vector(layer));

                // Add to document's clip library only (user drags from Asset Library to timeline)
                let _clip_id = self.action_executor.document_mut().add_vector_clip(test_clip);
                println!("Added '{}' to Asset Library (drag to timeline to use)", clip_name);
            }
            MenuAction::DeleteLayer => {
                println!("Menu: Delete Layer");
                // TODO: Implement delete layer
            }
            MenuAction::ToggleLayerVisibility => {
                use lightningbeam_core::actions::{SetLayerPropertiesAction, set_layer_properties::LayerProperty};
                use lightningbeam_core::layer::LayerTrait;
                if let Some(layer_id) = self.active_layer_id {
                    let cur = self.action_executor.document()
                        .get_layer(&layer_id)
                        .map(|l| l.visible());
                    if let Some(cur) = cur {
                        let action = SetLayerPropertiesAction::new(layer_id, LayerProperty::Visible(!cur));
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Toggle layer visibility: {}", e);
                        }
                    }
                } else {
                    println!("Toggle Layer Visibility: no active layer");
                }
            }
            MenuAction::ShowMasterTrack => {
                // Toggle show_master_track on all Timeline pane instances
                for pane in self.pane_instances.values_mut() {
                    if let panes::PaneInstance::Timeline(t) = pane {
                        t.show_master_track = !t.show_master_track;
                    }
                }
            }

            // Timeline menu
            MenuAction::NewKeyframe | MenuAction::AddKeyframeAtPlayhead => {
                if let Some(layer_id) = self.active_layer_id {
                    let document = self.action_executor.document();
                    let is_raster = matches!(document.get_layer(&layer_id), Some(AnyLayer::Raster(_)));
                    if is_raster {
                        // Raster layers: insert a blank cel at the playhead.
                        let (doc_w, doc_h) = (document.width as u32, document.height as u32);
                        let action = lightningbeam_core::actions::AddRasterKeyframeAction::new(
                            layer_id, self.playback_time, doc_w, doc_h,
                        );
                        if let Err(e) = self.action_executor.execute(Box::new(action)) {
                            eprintln!("Failed to add raster keyframe: {}", e);
                        }
                    } else {
                        // Determine which selected objects are shape instances vs clip instances
                        let _shape_ids: Vec<uuid::Uuid> = Vec::new();
                        let mut clip_ids = Vec::new();
                        if let Some(AnyLayer::Vector(vl)) = document.get_layer(&layer_id) {
                            // TODO: DCEL - shape instance lookup disabled during migration
                            // (was: get_shape_in_keyframe to check which selected objects are shapes)
                            for &id in self.selection.clip_instances() {
                                if vl.clip_instances.iter().any(|ci| ci.id == id) {
                                    clip_ids.push(id);
                                }
                            }
                        }
                        // For vector layers, always create a shape keyframe (even without clip selection)
                        if document.get_layer(&layer_id).map_or(false, |l| matches!(l, AnyLayer::Vector(_))) || !clip_ids.is_empty() {
                            let action = lightningbeam_core::actions::SetKeyframeAction::new(
                                layer_id,
                                self.playback_time,
                                clip_ids,
                            );
                            if let Err(e) = self.action_executor.execute(Box::new(action)) {
                                eprintln!("Failed to set keyframe: {}", e);
                            }
                        }
                    }
                }
            }
            MenuAction::NewBlankKeyframe => {
                println!("Menu: New Blank Keyframe");
                // TODO: Implement new blank keyframe
            }
            MenuAction::DeleteFrame => {
                println!("Menu: Delete Frame");
                // TODO: Implement delete frame
            }
            MenuAction::DuplicateKeyframe => {
                println!("Menu: Duplicate Keyframe");
                // TODO: Implement duplicate keyframe
            }
            // AddKeyframeAtPlayhead handled above together with NewKeyframe
            MenuAction::AddMotionTween => {
                println!("Menu: Add Motion Tween");
                // TODO: Implement add motion tween
            }
            MenuAction::AddShapeTween => {
                if let Some(layer_id) = self.active_layer_id {
                    let action = lightningbeam_core::actions::SetTweenAction::new(
                        layer_id,
                        self.playback_time,
                        lightningbeam_core::layer::TweenType::Shape,
                    );
                    if let Err(e) = self.action_executor.execute(Box::new(action)) {
                        eprintln!("Failed to add shape tween: {}", e);
                    }
                }
            }
            MenuAction::ReturnToStart => {
                println!("Menu: Return to Start");
                // TODO: Implement return to start
            }
            MenuAction::Play => {
                println!("Menu: Play");
                // TODO: Implement play/pause
            }
            MenuAction::ToggleCountIn => {
                // Only effective when metronome is enabled (count-in requires a click track)
                if self.metronome_enabled {
                    self.count_in_enabled = !self.count_in_enabled;
                }
            }

            // View menu
            MenuAction::ZoomIn => {
                self.pending_view_action = Some(MenuAction::ZoomIn);
            }
            MenuAction::ZoomOut => {
                self.pending_view_action = Some(MenuAction::ZoomOut);
            }
            MenuAction::ActualSize => {
                self.pending_view_action = Some(MenuAction::ActualSize);
            }
            MenuAction::RecenterView => {
                self.pending_view_action = Some(MenuAction::RecenterView);
            }
            MenuAction::ToggleOnionSkin => {
                self.onion_skin.enabled = !self.onion_skin.enabled;
            }
            MenuAction::NextLayout => {
                println!("Menu: Next Layout");
                let next_index = (self.current_layout_index + 1) % self.layouts.len();
                self.switch_layout(next_index);
            }
            MenuAction::PreviousLayout => {
                println!("Menu: Previous Layout");
                let prev_index = if self.current_layout_index == 0 {
                    self.layouts.len() - 1
                } else {
                    self.current_layout_index - 1
                };
                self.switch_layout(prev_index);
            }
            MenuAction::SwitchLayout(index) => {
                println!("Menu: Switch to Layout {}", index);
                if index < self.layouts.len() {
                    self.switch_layout(index);
                }
            }

            // Help menu
            MenuAction::About => {
                println!("Menu: About");
                // TODO: Implement about dialog
            }

            // Lightningbeam menu (macOS)
            MenuAction::Settings => {
                println!("Menu: Settings");
                // TODO: Implement settings
            }
            MenuAction::CloseWindow => {
                println!("Menu: Close Window");
                // TODO: Implement close window
            }
        }
    }

    /// Prepare document for saving by storing current UI layout
    /// Save the current document to a .beam file
    /// Assemble the background save command for `path`: prepares the document (layout), clones it,
    /// and snapshots the waveform/thumbnail side data. Shared by manual save and background autosave.
    fn build_save_command(
        &mut self,
        path: std::path::PathBuf,
        progress_tx: std::sync::mpsc::Sender<FileProgress>,
    ) -> FileCommand {
        // Snapshot the document and stamp the current UI layout onto the SNAPSHOT — never the live
        // document. Mutating the live doc here (as the old prepare_document_for_save did via
        // Arc::make_mut) would touch UI state and could deep-clone the whole document if a render
        // callback holds a reference — unacceptable for a frequent background autosave. The live
        // doc's `ui_layout` is only read at save/serialize time, so leaving it stale is harmless.
        let mut document = self.action_executor.document().clone();
        document.ui_layout = Some(self.current_layout.clone());
        if self.current_layout_index < self.layouts.len() {
            document.ui_layout_base = Some(self.layouts[self.current_layout_index].name.clone());
        }
        let waveform_blobs: std::collections::HashMap<usize, Vec<u8>> = self
            .waveform_pyramid_blobs
            .iter()
            .map(|(&idx, blob)| (idx, blob.as_ref().clone()))
            .collect();
        let (thumbnail_snapshot, complete_thumbnail_clips) = {
            let vm = self.video_manager.lock().unwrap();
            (vm.snapshot_all_thumbnails(), vm.complete_thumbnail_clips())
        };
        FileCommand::Save {
            path,
            document,
            layer_to_track_map: self.layer_to_track_map.clone(),
            large_media_mode: self.config.large_media_default,
            waveform_blobs,
            thumbnail_snapshot,
            complete_thumbnail_clips,
            progress_tx,
        }
    }

    fn save_to_file(&mut self, path: std::path::PathBuf) {
        println!("Saving to: {}", path.display());

        if self.audio_controller.is_none() {
            eprintln!("❌ Audio system not initialized");
            return;
        }

        // Create progress channel
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let command = self.build_save_command(path.clone(), progress_tx);

        if let Err(e) = self.file_command_tx.send(command) {
            eprintln!("❌ Failed to send save command: {}", e);
            return;
        }

        // Store operation state
        self.file_operation = Some(FileOperation::Saving {
            path,
            progress_rx,
        });
    }

    /// Mark the current document state as the autosave baseline — there is nothing new to recover
    /// (called after a manual save, and after the document is replaced by new/load). Clears the
    /// pending-event flag and resets the throttle so the next real change schedules cleanly.
    fn reset_autosave_baseline(&mut self) {
        self.autosave.baseline_epoch = self.action_executor.epoch();
        self.autosave.pending_event = false;
        self.autosave.last_time = None;
        // A fresh/loaded document also starts unmodified vs. its on-disk form.
        self.saved_epoch = self.action_executor.epoch();
        self.media_modified = false;
    }

    /// Whether the document has unsaved changes vs. the user's file (edits since the last manual
    /// save, or an import/recording that `epoch` doesn't track). Recovered work counts as unsaved
    /// until the user gives it a real home — its only copy is the transient recovery container.
    fn document_modified(&self) -> bool {
        self.action_executor.epoch() != self.saved_epoch
            || self.media_modified
            || self
                .current_file_path
                .as_ref()
                .is_some_and(|p| AutosaveState::is_recovery_path(p))
    }

    /// Background crash-recovery autosave: poll any in-flight recovery write, then (if the document
    /// is dirty, throttled to `AUTOSAVE_INTERVAL_SECS`, and no manual save is running) dispatch a
    /// write of the current state into the per-session recovery container. Fully background — reuses
    /// the file worker; the only UI-thread cost is the same document/side-data snapshot a manual
    /// save does. Never touches `current_file_path` or the raster `dirty` flags (the recovery file
    /// is a separate container from the user's file).
    fn maybe_autosave(&mut self, ctx: &egui::Context) {
        // Drain progress from an in-flight recovery write.
        if let Some(rx) = &self.autosave.progress_rx {
            while let Ok(p) = rx.try_recv() {
                match p {
                    FileProgress::Done => self.autosave.in_flight = false,
                    FileProgress::Error(e) => {
                        eprintln!("⚠️ [AUTOSAVE] recovery write failed: {}", e);
                        self.autosave.in_flight = false;
                    }
                    _ => {}
                }
            }
            if !self.autosave.in_flight {
                self.autosave.progress_rx = None;
            }
        }

        let Some(recovery_path) = self.autosave.recovery_path.clone() else { return };
        if self.autosave.in_flight || self.audio_controller.is_none() {
            return;
        }
        // Don't compete with a manual save on the single worker.
        if matches!(self.file_operation, Some(FileOperation::Saving { .. })) {
            return;
        }

        let epoch = self.action_executor.epoch();
        let dirty = epoch != self.autosave.baseline_epoch || self.autosave.pending_event;
        if !dirty {
            return;
        }
        if let Some(t) = self.autosave.last_time {
            let elapsed = t.elapsed().as_secs_f64();
            if elapsed < self.autosave.interval_secs {
                // Dirty but throttled — wake up when the interval elapses even if the app goes idle,
                // so an edit-then-idle session still gets its recovery snapshot.
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    (self.autosave.interval_secs - elapsed).max(0.1),
                ));
                return;
            }
        }

        let (tx, rx) = std::sync::mpsc::channel();
        let command = self.build_save_command(recovery_path, tx);
        if self.file_command_tx.send(command).is_err() {
            return;
        }
        self.autosave.in_flight = true;
        self.autosave.progress_rx = Some(rx);
        self.autosave.baseline_epoch = epoch;
        self.autosave.pending_event = false;
        self.autosave.last_time = Some(std::time::Instant::now());
        eprintln!("💾 [AUTOSAVE] recovery snapshot dispatched");
    }

    /// Show the crash-recovery prompt when a previous session left a recovery file behind. Recover
    /// loads it as an untitled document; Discard deletes it; Later keeps it for the next launch.
    fn render_recovery_prompt(&mut self, ctx: &egui::Context) {
        // Don't prompt over an in-flight file op (including a recovery load we just started).
        if self.autosave.leftover_recoveries.is_empty() || self.file_operation.is_some() {
            return;
        }
        let path = self.autosave.leftover_recoveries[0].clone();

        #[derive(PartialEq)]
        enum Choice { Recover, Discard, Later }
        let mut choice: Option<Choice> = None;

        egui::Modal::new(egui::Id::new("crash_recovery_modal")).show(ctx, |ui| {
            ui.set_width(crate::mobile::dialog_width(ctx, 460.0));
            ui.heading("Recover unsaved work?");
            ui.add_space(6.0);
            ui.label(
                "Lightningbeam didn't shut down cleanly last time. You have unsaved work from your \
                 previous session — recover it?",
            );
            if self.autosave.leftover_recoveries.len() > 1 {
                ui.add_space(4.0);
                ui.weak(format!(
                    "{} snapshots available; this shows the most recent first.",
                    self.autosave.leftover_recoveries.len()
                ));
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    choice = Some(Choice::Recover);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(Choice::Discard);
                }
                if ui.button("Later").clicked() {
                    choice = Some(Choice::Later);
                }
            });
        });

        match choice {
            Some(Choice::Recover) => {
                self.autosave.leftover_recoveries.remove(0);
                // Rename out of the `session-*` namespace before opening it, so it isn't offered
                // again next launch — but keep the file, since recovered raster keyframes page in
                // from it on demand (deleting it would lose paged pixels). It opens as the current
                // file; the user relocates the work with Save As. Old `recovered-*` files are
                // garbage-collected at startup.
                let open_path = match path.file_name().and_then(|n| n.to_str()) {
                    Some(name) => {
                        let renamed =
                            path.with_file_name(name.replacen("session-", "recovered-", 1));
                        if std::fs::rename(&path, &renamed).is_ok() { renamed } else { path }
                    }
                    None => path,
                };
                self.load_from_file(open_path);
            }
            Some(Choice::Discard) => {
                let _ = std::fs::remove_file(&path);
                self.autosave.leftover_recoveries.remove(0);
            }
            Some(Choice::Later) => {
                // Keep the files on disk but stop prompting this session.
                self.autosave.leftover_recoveries.clear();
            }
            None => {}
        }
    }

    /// The single "save changes?" prompt for any unsaved-work exit point — file switches (New /
    /// Open / Open Recent) and quitting. Also intercepts the window-close request. Save persists
    /// first (Save As for untitled/recovered docs) then runs the action; Don't Save runs it and
    /// discards; Cancel stays put.
    fn render_unsaved_prompt(&mut self, ctx: &egui::Context) {
        // Intercept a window-close request with unsaved work → veto it and queue the Quit prompt.
        if ctx.input(|i| i.viewport().close_requested()) && !self.confirmed_close {
            if self.document_modified() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.unsaved_prompt = Some(PendingAction::Quit);
            }
            // Unmodified → let the close proceed.
        }

        let Some(pending) = self.unsaved_prompt.as_ref() else { return };
        // The "…before X?" tail + the affirmative button label, per action.
        let (desc, discard_label) = match pending {
            PendingAction::NewFile => ("starting a new file", "Don't Save"),
            PendingAction::Open(_) => ("opening another file", "Don't Save"),
            PendingAction::Quit => ("quitting", "Discard & Quit"),
        };

        #[derive(PartialEq)]
        enum Answer {
            Save,
            Discard,
            Cancel,
        }
        let mut answer: Option<Answer> = None;

        egui::Modal::new(egui::Id::new("unsaved_changes_modal")).show(ctx, |ui| {
            ui.set_width(crate::mobile::dialog_width(ctx, 440.0));
            ui.heading("Save changes?");
            ui.add_space(6.0);
            ui.label(format!(
                "This document has unsaved changes. Save them before {desc}?"
            ));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    answer = Some(Answer::Save);
                }
                if ui.button(discard_label).clicked() {
                    answer = Some(Answer::Discard);
                }
                if ui.button("Cancel").clicked() {
                    answer = Some(Answer::Cancel);
                }
            });
        });

        match answer {
            Some(Answer::Cancel) => {
                self.unsaved_prompt = None;
            }
            Some(Answer::Discard) => {
                if let Some(action) = self.unsaved_prompt.take() {
                    self.do_action(action, ctx);
                }
            }
            Some(Answer::Save) => {
                let action = self.unsaved_prompt.take();
                // Save to the existing file, or Save As for an untitled / recovered document.
                let real_path = self
                    .current_file_path
                    .clone()
                    .filter(|p| !AutosaveState::is_recovery_path(p));
                let target = match real_path {
                    Some(p) => Some(p),
                    None => rfd::FileDialog::new()
                        .add_filter("Lightningbeam Project", &["beam"])
                        .set_file_name("Untitled.beam")
                        .save_file(),
                };
                match target {
                    Some(path) => {
                        // Run the action once the save completes.
                        self.after_save = action;
                        self.save_to_file(path);
                    }
                    None => {
                        // Save As cancelled → abort, keep the document (still unsaved).
                    }
                }
            }
            None => {}
        }
    }

    /// New File: tear down the current project and return to the start screen. (The guard for
    /// unsaved changes lives in the menu handler; this is the actual action.)
    fn do_new_file(&mut self) {
        // Tear down the backend (stops old instruments/voices immediately) and clear the app-side
        // track maps + backend-derived caches.
        self.reset_audio_backend();

        // Reset UI state and return to the start screen.
        self.current_file_path = None;
        self.selection.clear();
        self.editing_context = EditingContext::default();
        self.active_layer_id = None;
        self.playback_time = 0.0;
        self.is_playing = false;
        self.pane_instances.clear();
        self.project_generation += 1;
        self.app_mode = AppMode::StartScreen;
    }

    /// Carry out a deferred action once the unsaved-changes prompt is resolved (or when there was
    /// nothing unsaved to begin with).
    fn do_action(&mut self, action: PendingAction, ctx: &egui::Context) {
        match action {
            PendingAction::NewFile => self.do_new_file(),
            PendingAction::Open(path) => self.load_from_file(path),
            PendingAction::Quit => {
                self.confirmed_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Begin a file switch (New / Open / Open Recent), prompting to save first if the document has
    /// unsaved changes. Only ever called with `NewFile`/`Open` from the menus, so the immediate path
    /// needs no `ctx` (only `Quit`, driven by the close interceptor, does).
    fn request_switch(&mut self, action: PendingAction) {
        if self.document_modified() {
            self.unsaved_prompt = Some(action);
        } else {
            match action {
                PendingAction::NewFile => self.do_new_file(),
                PendingAction::Open(path) => self.load_from_file(path),
                PendingAction::Quit => {}
            }
        }
    }

    /// Load a document from a .beam file
    fn load_from_file(&mut self, path: std::path::PathBuf) {
        println!("Loading from: {}", path.display());

        if self.audio_controller.is_none() {
            eprintln!("❌ Audio system not initialized");
            return;
        }

        // Create progress channel
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();

        // Send load command to worker thread
        let command = FileCommand::Load {
            path: path.clone(),
            progress_tx,
        };

        if let Err(e) = self.file_command_tx.send(command) {
            eprintln!("❌ Failed to send load command: {}", e);
            return;
        }

        // Store operation state
        self.file_operation = Some(FileOperation::Loading {
            path,
            progress_rx,
        });
    }

    /// Update the "Open Recent" menu to reflect current config
    fn update_recent_files_menu(&mut self) {
        if let Some(menu_system) = &mut self.menu_system {
            let recent_files = self.config.get_recent_files();
            menu_system.update_recent_files(&recent_files);
        }
    }

    /// Restore UI layout from loaded document
    fn restore_layout_from_document(&mut self) {
        let doc = self.action_executor.document();

        // Restore saved layout if present
        if let Some(saved_layout) = &doc.ui_layout {
            self.current_layout = saved_layout.clone();

            // Try to find matching base layout by name
            if let Some(base_name) = &doc.ui_layout_base {
                if let Some(index) = self.layouts.iter().position(|l| &l.name == base_name) {
                    self.current_layout_index = index;
                } else {
                    // Base layout not found (maybe renamed/removed), default to first
                    self.current_layout_index = 0;
                }
            }

            println!("✅ Restored UI layout from save file");
        } else {
            // No saved layout (old file format or new project)
            // Keep the default (first layout)
            self.current_layout_index = 0;
            self.current_layout = self.layouts[0].layout.clone();
            println!("ℹ️  No saved layout found, using default");
        }

        // Clear existing pane instances so they rebuild with new layout
        self.pane_instances.clear();
    }

    /// Apply loaded project data (called after successful load in background)
    fn apply_loaded_project(&mut self, mut loaded_project: lightningbeam_core::file_io::LoadedProject, path: std::path::PathBuf) {
        use lightningbeam_core::action::ActionExecutor;

        let apply_start = std::time::Instant::now();
        eprintln!("📊 [APPLY] Starting apply_loaded_project() on UI thread...");

        // Check for missing files
        if !loaded_project.missing_files.is_empty() {
            eprintln!("⚠️ {} missing files", loaded_project.missing_files.len());
            for missing in &loaded_project.missing_files {
                eprintln!("   - {}", missing.original_path.display());
            }
            // TODO Phase 5: Show recovery dialog
        }

        // Tear down the previously-open project's backend tracks/instruments before restoring this
        // file's audio pool + tracks, so an open-over-open doesn't leave orphaned tracks resident in
        // the backend. Reset is a command; the audio-pool/track restoration below uses queries, which
        // the audio thread drains after all commands each callback, so the ordering holds.
        self.reset_audio_backend();

        // Replace document
        let step1_start = std::time::Instant::now();
        self.action_executor = ActionExecutor::new(loaded_project.document);
        // Freshly loaded document is clean → rebase the autosave epoch (no spurious recovery write).
        self.reset_autosave_baseline();
        eprintln!("📊 [APPLY] Step 1: Replace document took {:.2}ms", step1_start.elapsed().as_secs_f64() * 1000.0);

        // Restore UI layout from loaded document
        let step2_start = std::time::Instant::now();
        self.restore_layout_from_document();
        eprintln!("📊 [APPLY] Step 2: Restore UI layout took {:.2}ms", step2_start.elapsed().as_secs_f64() * 1000.0);

        // Snapshot persisted waveform pyramids (with each entry's source sample
        // rate) before the backend consumes the entries below. Restored into the
        // GPU caches after the controller borrow ends, so a load needn't re-decode.
        let loaded_waveforms: Vec<(usize, u32, Vec<u8>)> = loaded_project
            .audio_pool_entries
            .iter()
            .filter_map(|e| e.waveform_blob.as_ref().map(|b| (e.pool_index, e.sample_rate, b.clone())))
            .collect();

        // Streamed (packed) audio entries that have NO persisted pyramid yet
        // (e.g. saved before waveform persistence): generate one in the background
        // from the packed blob so the overview appears without a full decode.
        // (pool_index, media_id, codec/ext, sample_rate)
        let streamed_needing_waveform: Vec<(usize, String, Option<String>, u32)> = loaded_project
            .audio_pool_entries
            .iter()
            .filter(|e| e.waveform_blob.is_none() && e.embedded_data.is_none())
            .filter_map(|e| {
                e.media_id.clone().map(|id| {
                    let ext = std::path::Path::new(&e.name)
                        .extension()
                        .and_then(|x| x.to_str())
                        .map(|s| s.to_lowercase());
                    (e.pool_index, id, ext, e.sample_rate)
                })
            })
            .collect();

        // Load audio pool FIRST (before setting project, so clips can reference pool entries)
        let step3_start = std::time::Instant::now();
        if let Some(ref controller_arc) = self.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            let audio_pool_entries = loaded_project.audio_pool_entries;

            eprintln!("📊 [APPLY] Step 3: Starting audio pool load...");
            if let Err(e) = controller.load_audio_pool(audio_pool_entries, &path) {
                eprintln!("❌ Failed to load audio pool: {}", e);
                return;
            }
            eprintln!("📊 [APPLY] Step 3: Load audio pool took {:.2}ms", step3_start.elapsed().as_secs_f64() * 1000.0);

            // Install the packed-media byte-source factory for this container before
            // SetProject, so bulk activation can stream container-packed audio.
            if lightningbeam_core::beam_archive::BeamArchive::is_sqlite(&path) {
                let factory = lightningbeam_core::file_io::blob_source_factory(&path);
                if let Err(e) = controller.set_blob_source_factory(factory) {
                    eprintln!("⚠️ [APPLY] Failed to install blob source factory: {}", e);
                }
            }

            // Now set project (clips can now reference the loaded pool entries)
            let step4_start = std::time::Instant::now();
            if let Err(e) = controller.set_project(loaded_project.audio_project) {
                eprintln!("❌ Failed to set project: {}", e);
                return;
            }
            eprintln!("📊 [APPLY] Step 4: Set audio project took {:.2}ms", step4_start.elapsed().as_secs_f64() * 1000.0);

            // Sync BPM/time signature to metronome
            let doc = self.action_executor.document();
            controller.set_tempo(
                doc.bpm() as f32,
                (doc.time_signature.numerator, doc.time_signature.denominator),
            );
        }

        // Restore waveform overviews from the persisted pyramids (no re-decode).
        // Clear first so stale pools from the previous project don't linger;
        // on-demand audio repopulates raw_audio_cache lazily at render time.
        self.raw_audio_cache.clear();
        self.waveform_gpu_dirty.clear();
        self.waveform_minmax_pools.clear();
        self.waveform_pyramid_blobs.clear();
        for (pool_index, sample_rate, blob) in loaded_waveforms {
            match daw_backend::audio::waveform_pyramid::WaveformPyramid::from_bytes(&blob) {
                Ok(pyramid) => {
                    let b = pyramid.floor_samples_per_texel.max(1);
                    let channels = pyramid.channels;
                    let floor = pyramid.floor();
                    let mut packed = Vec::with_capacity(floor.len() * 4);
                    for t in floor {
                        packed.push(t.l_min);
                        packed.push(t.l_max);
                        packed.push(t.r_min);
                        packed.push(t.r_max);
                    }
                    self.raw_audio_cache.insert(pool_index, (Arc::new(packed), sample_rate, channels));
                    self.waveform_minmax_pools.insert(pool_index, b);
                    self.waveform_pyramid_blobs.insert(pool_index, Arc::new(blob));
                    self.waveform_gpu_dirty.insert(pool_index);
                }
                Err(e) => eprintln!(
                    "⚠️ [APPLY] Failed to parse persisted waveform for pool {}: {}",
                    pool_index, e
                ),
            }
        }

        // Background-generate overviews for streamed audio that has no persisted
        // pyramid yet (older saves). Streams the packed blob via the factory and
        // sends the floor through the same channel `update()` already drains; the
        // next save then persists it.
        if !streamed_needing_waveform.is_empty() {
            let wf_tx = self.waveform_result_tx.clone();
            let floor_b = self.config.waveform_floor_samples_per_texel.max(1);
            let beam_path = path.clone();
            std::thread::spawn(move || {
                let factory = lightningbeam_core::file_io::blob_source_factory(&beam_path);
                for (pool_index, media_id, ext, sample_rate) in streamed_needing_waveform {
                    let src = match factory.open(&media_id) {
                        Ok(s) => s,
                        Err(e) => {
                            eprintln!("[APPLY] waveform gen: open packed {} failed: {}", media_id, e);
                            continue;
                        }
                    };
                    match daw_backend::audio::disk_reader::build_waveform_pyramid_from_source(
                        src,
                        ext.as_deref(),
                        floor_b,
                    ) {
                        Ok(pyramid) => {
                            let floor = pyramid.floor();
                            let mut packed = Vec::with_capacity(floor.len() * 4);
                            for t in floor {
                                packed.push(t.l_min);
                                packed.push(t.l_max);
                                packed.push(t.r_min);
                                packed.push(t.r_max);
                            }
                            let blob = pyramid.to_bytes();
                            let _ = wf_tx.send((
                                pool_index,
                                packed,
                                sample_rate,
                                pyramid.channels,
                                pyramid.floor_samples_per_texel,
                                blob,
                            ));
                        }
                        Err(e) => eprintln!("[APPLY] waveform gen failed for pool {}: {}", pool_index, e),
                    }
                }
            });
        }

        // Reset state and restore track mappings
        let step5_start = std::time::Instant::now();
        self.layer_to_track_map.clear();
        self.track_to_layer_map.clear();

        if !loaded_project.layer_to_track_map.is_empty() {
            // Restore saved mapping (connects UI layers to loaded backend tracks with effects graphs)
            for (&layer_id, &track_id) in &loaded_project.layer_to_track_map {
                self.layer_to_track_map.insert(layer_id, track_id);
                self.track_to_layer_map.insert(track_id, layer_id);
            }
            eprintln!("📊 [APPLY] Step 5: Restored {} track mappings from file in {:.2}ms",
                loaded_project.layer_to_track_map.len(), step5_start.elapsed().as_secs_f64() * 1000.0);
        } else {
            eprintln!("📊 [APPLY] Step 5: No saved track mappings (old file format)");
        }

        // Sync any audio layers that don't have a mapping yet (new layers, or old file format)
        let step6_start = std::time::Instant::now();
        self.sync_audio_layers_to_backend();
        eprintln!("📊 [APPLY] Step 6: Sync audio layers took {:.2}ms", step6_start.elapsed().as_secs_f64() * 1000.0);

        // Increment project generation to force node graph pane reload
        self.project_generation += 1;

        // Fetch raw audio for all audio clips in the loaded project
        let step7_start = std::time::Instant::now();
        let pool_indices: Vec<usize> = self.action_executor.document()
            .audio_clips.values()
            .filter_map(|clip| {
                if let lightningbeam_core::clip::AudioClipType::Sampled { audio_pool_index } = &clip.clip_type {
                    Some(*audio_pool_index)
                } else {
                    None
                }
            })
            .collect();

        let mut raw_fetched = 0;
        for pool_index in pool_indices {
            if !self.raw_audio_cache.contains_key(&pool_index) {
                if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    match controller.get_pool_audio_samples(pool_index) {
                        Ok((samples, sr, ch)) => {
                            self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sr, ch));
                            self.waveform_gpu_dirty.insert(pool_index);
                            raw_fetched += 1;
                        }
                        Err(e) => eprintln!("Failed to fetch raw audio for pool {}: {}", pool_index, e),
                    }
                }
            }
        }
        eprintln!("📊 [APPLY] Step 7: Fetched {} raw audio samples in {:.2}ms", raw_fetched, step7_start.elapsed().as_secs_f64() * 1000.0);

        // Rebuild MIDI event cache for all MIDI clips (needed for timeline/piano roll rendering)
        let step8_start = std::time::Instant::now();
        self.midi_event_cache.clear();
        let midi_clip_ids: Vec<u32> = self.action_executor.document()
            .audio_clips.values()
            .filter_map(|clip| clip.midi_clip_id())
            .collect();

        let mut midi_fetched = 0;
        if let Some(ref controller_arc) = self.audio_controller {
            let mut controller = controller_arc.lock().unwrap();
            for clip_id in midi_clip_ids {
                // track_id is unused by the query, pass 0
                match controller.query_midi_clip(0, clip_id) {
                    Ok(clip_data) => {
                        self.midi_event_cache.insert(clip_id, clip_data.events);
                        midi_fetched += 1;
                    }
                    Err(e) => eprintln!("Failed to fetch MIDI clip {}: {}", clip_id, e),
                }
            }
        }
        eprintln!("📊 [APPLY] Step 8: Rebuilt MIDI event cache for {} clips in {:.2}ms", midi_fetched, step8_start.elapsed().as_secs_f64() * 1000.0);

        // Restore persisted video thumbnails (decode + insert off-thread). Only
        // *complete* packs are skipped from regeneration; *partial* packs are
        // restored AND have generation resumed below (it skips the keyframes already
        // restored), so a save mid-generation continues from where it left off.
        let skip_thumbnail_clips: std::collections::HashSet<uuid::Uuid> = loaded_project
            .thumbnail_blobs
            .iter()
            .filter(|(_, blob)| thumbnail_blob_is_complete(blob))
            .map(|(id, _)| *id)
            .collect();
        if !loaded_project.thumbnail_blobs.is_empty() {
            let blobs = std::mem::take(&mut loaded_project.thumbnail_blobs);
            let video_manager = Arc::clone(&self.video_manager);
            std::thread::spawn(move || {
                for (clip_id, blob) in blobs {
                    let complete = thumbnail_blob_is_complete(&blob);
                    let thumbs = decode_thumbnail_blob(&blob);
                    let mut vm = video_manager.lock().unwrap();
                    for (ts, rgba) in thumbs {
                        vm.insert_thumbnail(&clip_id, ts, rgba);
                    }
                    if complete {
                        vm.mark_thumbnails_complete(&clip_id);
                    }
                }
            });
        }

        // Set the loaded container path BEFORE registering videos: a packed video
        // clip resolves to `VideoSource::Packed { db_path: current_file_path, .. }`,
        // so this must be in place or the clip falls back to its (possibly missing)
        // external file and renders black.
        self.current_file_path = Some(path.clone());

        // Re-register video clips with the VideoManager (decoder + keyframe index +
        // thumbnails). Restored video_clips carry only metadata + file_path; without
        // this they render black with no thumbnails.
        let step9_start = std::time::Instant::now();
        self.register_loaded_videos(&skip_thumbnail_clips);
        eprintln!("📊 [APPLY] Step 9: Registered loaded videos in {:.2}ms", step9_start.elapsed().as_secs_f64() * 1000.0);

        // Reset playback state
        self.playback_time = 0.0;
        self.is_playing = false;
        // Point the raster paging store at the loaded container so faulting works.
        self.raster_store.set_path(self.current_file_path.clone());

        // Add to recent files — but never a recovery file (it's an internal, transient container in
        // the app data dir, not a project the user opened).
        if !AutosaveState::is_recovery_path(&path) {
            self.config.add_recent_file(path.clone());
            self.update_recent_files_menu();
        }

        // Set active layer
        if let Some(first) = self.action_executor.document().root.children.first() {
            self.active_layer_id = Some(first.id());
        }

        eprintln!("📊 [APPLY] ✅ Total apply_loaded_project() time: {:.2}ms", apply_start.elapsed().as_secs_f64() * 1000.0);
        println!("✅ Loaded from: {}", path.display());
    }

    /// Register every video clip in the loaded document with the `VideoManager`
    /// so it can decode and display. Mirrors the import path's setup (decoder +
    /// background keyframe index + thumbnails) but does NOT re-extract audio —
    /// the extracted audio is already restored via the audio pool.
    fn register_loaded_videos(&mut self, skip_thumbnail_clips: &std::collections::HashSet<uuid::Uuid>) {
        let doc_width = self.action_executor.document().width as u32;
        let doc_height = self.action_executor.document().height as u32;

        // Snapshot (id, media_id, path) so we don't hold the document borrow while
        // locking the VideoManager / spawning threads.
        let db_path = self.current_file_path.clone();
        let videos: Vec<_> = self
            .action_executor
            .document()
            .video_clips
            .values()
            .map(|c| (c.id, c.media_id, c.file_path.clone()))
            .collect();

        for (clip_id, media_id, file_path) in videos {
            // Packed video streams from the container blob; referenced video opens
            // its external file (skip if it's gone).
            let source = match (media_id, &db_path) {
                (Some(mid), Some(db)) => {
                    lightningbeam_core::video::VideoSource::Packed { db_path: db.clone(), media_id: mid }
                }
                _ => {
                    if !std::path::Path::new(&file_path).exists() {
                        eprintln!("⚠️ [APPLY] Video file not found, skipping: {}", file_path);
                        continue;
                    }
                    lightningbeam_core::video::VideoSource::Path(file_path.clone())
                }
            };

            {
                let mut video_mgr = self.video_manager.lock().unwrap();
                if let Err(e) = video_mgr.load_video(clip_id, source, doc_width, doc_height) {
                    eprintln!("⚠️ [APPLY] Failed to load video {}: {}", clip_id, e);
                    continue;
                }
            }

            self.spawn_keyframe_index(clip_id);
            // Skip regeneration for clips whose thumbnails were restored from the
            // container — they're being decoded + inserted in the background.
            if !skip_thumbnail_clips.contains(&clip_id) {
                self.spawn_thumbnail_generation(clip_id);
            }
        }
    }

    /// Spawn a background thread that builds the keyframe (seek) index for a
    /// video clip. The clip must already be registered via `load_video`.
    ///
    /// The slow packet scan runs holding **no** lock — only brief locks bracket it
    /// (to grab the decoder handle and to store the result) — so it never blocks
    /// playback or the UI (which both need the VideoManager / decoder locks).
    fn spawn_keyframe_index(&self, clip_id: uuid::Uuid) {
        let video_manager = Arc::clone(&self.video_manager);
        std::thread::spawn(move || {
            let decoder_arc = {
                let vm = video_manager.lock().unwrap();
                vm.get_decoder(&clip_id)
            };
            let Some(decoder_arc) = decoder_arc else { return; };

            let (source, stream_index) = {
                let decoder = decoder_arc.lock().unwrap();
                decoder.keyframe_scan_params()
            };

            // Slow scan, no locks held.
            match lightningbeam_core::video::VideoDecoder::scan_keyframes(&source, stream_index) {
                Ok(positions) => {
                    let count = positions.len();
                    decoder_arc.lock().unwrap().set_keyframe_index(positions);
                    println!("  Built keyframe index ({} keyframes) for video clip {}", count, clip_id);
                }
                Err(e) => eprintln!("Failed to build keyframe index for {}: {}", clip_id, e),
            }
        });
    }

    /// Spawn a background thread that (re)generates timeline thumbnails for a
    /// video clip. Uses a **dedicated** decoder (independent of the playback
    /// decoder) and samples at keyframes, so it neither blocks playback nor pays
    /// the cost of decoding whole GOPs. Safe to call on its own to refresh a
    /// single clip's thumbnails.
    fn spawn_thumbnail_generation(&self, clip_id: uuid::Uuid) {
        let video_manager = Arc::clone(&self.video_manager);
        std::thread::spawn(move || {
            // Grab the source from the playback decoder (brief lock), then do all
            // decoding on an independent decoder (a fresh BlobReader for packed video).
            let source = {
                let decoder_arc = {
                    let vm = video_manager.lock().unwrap();
                    vm.get_decoder(&clip_id)
                };
                let Some(decoder_arc) = decoder_arc else { return; };
                let decoder = decoder_arc.lock().unwrap();
                decoder.source()
            };

            let vm_insert = Arc::clone(&video_manager);
            let vm_skip = Arc::clone(&video_manager);
            let result = lightningbeam_core::video::generate_keyframe_thumbnails(
                source,
                5.0,
                128,
                // Resume: skip keyframes already covered (e.g. restored from a
                // partial persisted pack), so generation only fills the gaps.
                |ts| vm_skip.lock().unwrap().has_thumbnail_near(&clip_id, ts, 2.5),
                |ts, rgba| {
                    let mut vm = vm_insert.lock().unwrap();
                    vm.insert_thumbnail(&clip_id, ts, rgba);
                },
            );
            match result {
                Ok(()) => {
                    // Full keyframe pass finished — mark complete so this set is
                    // worth persisting (a partial set is left unmarked → not saved).
                    vm_insert.lock().unwrap().mark_thumbnails_complete(&clip_id);
                }
                Err(e) => eprintln!("Thumbnail generation failed for {}: {}", clip_id, e),
            }
        });
    }

    /// If `path` is a large media file (≥ the large-media threshold) and the user
    /// hasn't yet chosen a default pack-vs-reference policy, queue the one-time
    /// prompt. The actual storage decision happens at save time from the chosen
    /// preference; this only establishes that preference.
    fn note_possible_large_media(&mut self, path: &std::path::Path) {
        use lightningbeam_core::file_io::LargeMediaMode;
        if self.config.large_media_default != LargeMediaMode::Ask || self.large_media_prompt.is_some() {
            return;
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if size >= lightningbeam_core::beam_archive::LARGE_MEDIA_THRESHOLD {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string());
            self.large_media_prompt = Some(name);
        }
    }

    /// Render the one-time "pack vs reference large media" prompt, if pending.
    /// The choice is persisted to config as the future default.
    fn render_large_media_prompt(&mut self, ctx: &egui::Context) {
        use lightningbeam_core::file_io::LargeMediaMode;
        let Some(name) = self.large_media_prompt.clone() else {
            return;
        };
        let threshold_gb = lightningbeam_core::beam_archive::LARGE_MEDIA_THRESHOLD as f64
            / (1024.0 * 1024.0 * 1024.0);
        let mut choice: Option<LargeMediaMode> = None;
        egui::Window::new("Large media file")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!("\"{}\" is larger than {:.0} GB.", name, threshold_gb));
                ui.add_space(6.0);
                ui.label("How should large media be stored in projects from now on?");
                ui.add_space(6.0);
                ui.label("• Pack — copy the bytes into the .beam file (self-contained, larger project).");
                ui.label("• Reference — keep the file on disk and store only its path (smaller project; the file must stay put).");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Pack into project").clicked() {
                        choice = Some(LargeMediaMode::Pack);
                    }
                    if ui.button("Reference external file").clicked() {
                        choice = Some(LargeMediaMode::Reference);
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("You can change this later in Preferences.")
                        .weak()
                        .small(),
                );
            });
        if let Some(mode) = choice {
            self.config.large_media_default = mode;
            self.config.save();
            self.large_media_prompt = None;
        }
    }

    /// Import an image file as an ImageAsset
    /// Import an `.svg` file as a new vector layer (one static keyframe at the playhead).
    fn import_svg_file(&mut self, path: &std::path::Path) {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                let msg = format!("Failed to read SVG: {}", e);
                eprintln!("❌ {} ({})", msg, path.display());
                notifications::notify_error("SVG Import Failed", &msg);
                return;
            }
        };

        let graph = match svg_import::import_svg(&bytes) {
            Ok(g) => g,
            Err(e) => {
                eprintln!("❌ {}", e);
                notifications::notify_error("SVG Import Failed", &e);
                return;
            }
        };

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("SVG")
            .to_string();

        // Build a vector layer holding the imported graph as a keyframe at the current time.
        let mut layer = lightningbeam_core::layer::VectorLayer::new(name);
        let mut keyframe = lightningbeam_core::layer::ShapeKeyframe::new(self.playback_time);
        keyframe.graph = graph;
        layer.keyframes.push(keyframe);

        let editing_clip_id = self.editing_context.current_clip_id();
        let action = lightningbeam_core::actions::AddLayerAction::new(
            lightningbeam_core::layer::AnyLayer::Vector(layer),
        )
        .with_target_clip(editing_clip_id);
        if let Err(e) = self.action_executor.execute(Box::new(action)) {
            eprintln!("❌ Failed to add imported SVG layer: {}", e);
            return;
        }

        // Select the newly created layer.
        let context_layers = self.action_executor.document().context_layers(editing_clip_id.as_ref());
        if let Some(last_layer) = context_layers.last() {
            self.active_layer_id = Some(last_layer.id());
        }
        self.last_import_filter = ImportFilter::Images;
    }

    fn import_image(&mut self, path: &std::path::Path) -> Option<ImportedAssetInfo> {
        use lightningbeam_core::clip::ImageAsset;
        // Imported media lives outside the action/undo system, so flag it for the next autosave.
        self.autosave.pending_event = true;
        self.media_modified = true;
        self.note_possible_large_media(path);

        // Get filename for asset name
        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled Image")
            .to_string();

        // Load image to get dimensions
        match image::open(path) {
            Ok(img) => {
                let (width, height) = (img.width(), img.height());

                // Read raw file data for embedding
                let data = match std::fs::read(path) {
                    Ok(data) => Some(data),
                    Err(e) => {
                        eprintln!("Warning: Could not embed image data: {}", e);
                        None
                    }
                };

                // Create image asset
                let mut asset = ImageAsset::new(&name, path, width, height);
                asset.data = data;

                // Add to document
                let asset_id = self.action_executor.document_mut().add_image_asset(asset);
                println!("Imported image '{}' ({}x{}) - ID: {}", name, width, height, asset_id);

                Some(ImportedAssetInfo {
                    clip_id: asset_id,
                    clip_type: panes::DragClipType::Image,
                    name,
                    dimensions: Some((width as f64, height as f64)),
                    duration: 0.0, // Images have no duration
                    linked_audio_clip_id: None,
                })
            }
            Err(e) => {
                eprintln!("Failed to load image '{}': {}", path.display(), e);
                None
            }
        }
    }

    /// Import an audio file via daw-backend (async — non-blocking)
    /// to the engine for async import. The engine memory-maps WAV files or sets
    /// up stream decoding for compressed formats. An `AudioFileReady` event is
    /// emitted when the file is playback-ready; the event handler populates the
    /// GPU waveform cache.
    fn import_audio(&mut self, path: &std::path::Path) -> Option<ImportedAssetInfo> {
        use lightningbeam_core::clip::AudioClip;
        self.autosave.pending_event = true;
        self.media_modified = true;
        self.note_possible_large_media(path);

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled Audio")
            .to_string();

        // Read metadata without decoding (fast — sub-millisecond)
        let metadata = match daw_backend::io::read_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Failed to read audio metadata '{}': {}", path.display(), e);
                return None;
            }
        };

        let duration = metadata.duration;
        let channels = metadata.channels;
        let sample_rate = metadata.sample_rate;

        if let Some(ref controller_arc) = self.audio_controller {
            // Import synchronously to get the real pool index from the engine.
            // NOTE: briefly blocks the UI thread (sub-ms for PCM mmap; a few ms
            // for compressed streaming init).
            let pool_index = {
                let mut controller = controller_arc.lock().unwrap();
                match controller.import_audio_sync(path.to_path_buf()) {
                    Ok(idx) => idx,
                    Err(e) => {
                        eprintln!("Failed to import audio '{}': {}", path.display(), e);
                        return None;
                    }
                }
            };

            let clip = AudioClip::new_sampled(&name, pool_index, duration);
            let clip_id = self.action_executor.document_mut().add_audio_clip(clip);

            println!("Imported audio '{}' ({:.1}s, {}ch, {}Hz) - ID: {} (pool: {})",
                name, duration, channels, sample_rate, clip_id, pool_index);

            Some(ImportedAssetInfo {
                clip_id,
                clip_type: panes::DragClipType::AudioSampled,
                name,
                dimensions: None,
                duration,
                linked_audio_clip_id: None,
            })
        } else {
            eprintln!("Cannot import audio: audio engine not initialized");
            None
        }
    }

    /// Rebuild a MIDI event cache entry from backend note format.
    /// Called after undo/redo to keep the cache consistent with the backend.
    fn rebuild_midi_cache_entry(&mut self, clip_id: u32, notes: &[(f64, u8, u8, f64)]) {
        let mut events: Vec<daw_backend::audio::midi::MidiEvent> = Vec::with_capacity(notes.len() * 2);
        for &(start_time, note, velocity, duration) in notes {
            let on = daw_backend::audio::midi::MidiEvent::note_on(daw_backend::Beats(start_time), 0, note, velocity);
            events.push(on);
            let off = daw_backend::audio::midi::MidiEvent::note_off(daw_backend::Beats(start_time + duration), 0, note, 0);
            events.push(off);
        }
        events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
        self.midi_event_cache.insert(clip_id, events);
    }

    /// Import a MIDI file via daw-backend
    fn import_midi(&mut self, path: &std::path::Path) -> Option<ImportedAssetInfo> {
        use lightningbeam_core::clip::AudioClip;

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled MIDI")
            .to_string();

        // Load MIDI file via daw-backend
        match daw_backend::io::midi_file::load_midi_file(path, 0, 44100) {
            Ok(midi_clip) => {
                let duration = midi_clip.duration;
                let event_count = midi_clip.events.len();

                let processed_events = midi_clip.events.clone();

                // Add to backend MIDI clip pool FIRST and get the backend clip ID
                if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    controller.add_midi_clip_to_pool(midi_clip.clone());
                    let backend_clip_id = midi_clip.id; // The backend clip ID

                    // Cache MIDI events in frontend for rendering (thumbnails & timeline piano roll)
                    self.midi_event_cache.insert(backend_clip_id, processed_events);

                    // Create frontend MIDI clip referencing the backend pool
                    let clip = AudioClip::new_midi(&name, backend_clip_id, duration);
                    let frontend_clip_id = self.action_executor.document_mut().add_audio_clip(clip);

                    println!("Imported MIDI '{}' ({:.1}s, {} total events) - Frontend ID: {}, Backend ID: {}",
                        name, duration.beats_to_f64(), event_count, frontend_clip_id, backend_clip_id);

                    Some(ImportedAssetInfo {
                        clip_id: frontend_clip_id,
                        clip_type: panes::DragClipType::AudioMidi,
                        name,
                        dimensions: None,
                        duration: duration.beats_to_f64(),
                        linked_audio_clip_id: None,
                    })
                } else {
                    eprintln!("⚠️  Cannot import MIDI: audio system not available");
                    None
                }
            }
            Err(e) => {
                eprintln!("Failed to load MIDI '{}': {}", path.display(), e);
                None
            }
        }
    }

    /// Import a video file (placeholder - decoder not yet ported)
    fn import_video(&mut self, path: &std::path::Path) -> Option<ImportedAssetInfo> {
        use lightningbeam_core::clip::VideoClip;
        use lightningbeam_core::video::probe_video;
        self.autosave.pending_event = true;
        self.media_modified = true;
        self.note_possible_large_media(path);

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled Video")
            .to_string();

        let path_str = path.to_string_lossy().to_string();

        // Probe video for metadata (freshly imported video is referenced until the
        // first save packs it).
        let metadata = match probe_video(&lightningbeam_core::video::VideoSource::Path(path_str.clone())) {
            Ok(meta) => meta,
            Err(e) => {
                eprintln!("Failed to probe video '{}': {}", name, e);
                return None;
            }
        };

        // Create video clip with real metadata
        let clip = VideoClip::new(
            &name,
            path_str.clone(),
            metadata.width as f64,
            metadata.height as f64,
            metadata.duration,
            metadata.fps,
        );

        let clip_id = clip.id;

        // Load video into VideoManager (without building keyframe index)
        let doc_width = self.action_executor.document().width as u32;
        let doc_height = self.action_executor.document().height as u32;

        let mut video_mgr = self.video_manager.lock().unwrap();
        if let Err(e) = video_mgr.load_video(
            clip_id,
            lightningbeam_core::video::VideoSource::Path(path_str.clone()),
            doc_width,
            doc_height,
        ) {
            eprintln!("Failed to load video '{}': {}", name, e);
            return None;
        }
        drop(video_mgr);

        // Spawn background thread to build keyframe index asynchronously
        self.spawn_keyframe_index(clip_id);

        // Register the video's audio track as a streaming pool entry if present.
        if metadata.has_audio {
            if let Some(ref audio_controller) = self.audio_controller {
                let path_clone = path_str.clone();
                let video_clip_id = clip_id;
                let video_name = name.clone();
                let video_duration = metadata.duration;
                let audio_controller_clone = Arc::clone(audio_controller);
                let tx = self.audio_extraction_tx.clone();
                let wf_tx = self.waveform_result_tx.clone();
                let floor_b = self.config.waveform_floor_samples_per_texel.max(1);

                std::thread::spawn(move || {
                    use lightningbeam_core::clip::AudioClip;

                    // Add the video's audio track as a streaming pool entry — it is
                    // decoded on demand from the video file by the disk reader's
                    // FFmpeg backend. No extraction to disk or RAM.
                    let mut controller = audio_controller_clone.lock().unwrap();
                    let pool_index =
                        match controller.add_video_audio_sync(std::path::PathBuf::from(&path_clone)) {
                            Ok(index) => index,
                            Err(e) => {
                                drop(controller);
                                eprintln!("Failed to add video audio stream: {}", e);
                                let _ = tx.send(AudioExtractionResult::Error {
                                    video_clip_id,
                                    error: format!("Failed to add video audio: {}", e),
                                });
                                return;
                            }
                        };
                    // Pull the audio track's channels/sample_rate for reporting
                    // (duration comes from the video itself, so the clip length
                    // matches the video clip exactly).
                    let (_dur, sample_rate, channels) = controller
                        .get_pool_file_info(pool_index)
                        .unwrap_or((video_duration, 0, 0));
                    drop(controller);

                    let audio_clip_name = format!("{} (Audio)", video_name);
                    let audio_clip =
                        AudioClip::new_sampled(&audio_clip_name, pool_index, video_duration);

                    let _ = tx.send(AudioExtractionResult::Success {
                        video_clip_id,
                        audio_clip,
                        pool_index,
                        video_name,
                        channels,
                        sample_rate,
                    });

                    // Build the min/max waveform overview pyramid by streaming the
                    // video's audio through FFmpeg once. Video-audio has no in-RAM
                    // samples, so without this the clip would draw no waveform.
                    use daw_backend::audio::disk_reader::{build_waveform_pyramid, SourceKind};
                    match build_waveform_pyramid(
                        std::path::Path::new(&path_clone),
                        SourceKind::VideoAudio,
                        floor_b,
                    ) {
                        Ok(pyramid) => {
                            // Pack the floor level interleaved as (Lmin,Lmax,Rmin,Rmax)
                            // f32 per texel — the layout the GPU min/max upload expects.
                            let floor = pyramid.floor();
                            let mut packed = Vec::with_capacity(floor.len() * 4);
                            for t in floor {
                                packed.push(t.l_min);
                                packed.push(t.l_max);
                                packed.push(t.r_min);
                                packed.push(t.r_max);
                            }
                            // Serialize the whole pyramid for persistence in the .beam
                            // container (so a later load is a disk read, not a re-decode).
                            let blob = pyramid.to_bytes();
                            let _ = wf_tx.send((
                                pool_index,
                                packed,
                                sample_rate,
                                channels,
                                pyramid.floor_samples_per_texel,
                                blob,
                            ));
                        }
                        Err(e) => {
                            eprintln!("Failed to build waveform pyramid for video audio: {}", e);
                        }
                    }
                });
            } else {
                eprintln!("  ⚠️  Video has audio but audio engine not initialized - skipping extraction");
            }
        }

        // Spawn background thread for thumbnail generation. The video becomes
        // displayable as soon as the decoder is registered (above); thumbnails
        // fill in without holding the VideoManager lock during decode.
        self.spawn_thumbnail_generation(clip_id);

        // Add clip to document
        let clip_id = self.action_executor.document_mut().add_video_clip(clip);

        println!("Imported video '{}' ({}x{}, {:.2}s @ {:.0}fps) - ID: {}",
            name,
            metadata.width,
            metadata.height,
            metadata.duration,
            metadata.fps,
            clip_id
        );

        if metadata.has_audio {
            println!("  Extracting audio track in background...");
        }

        Some(ImportedAssetInfo {
            clip_id,
            clip_type: panes::DragClipType::Video,
            name,
            dimensions: Some((metadata.width as f64, metadata.height as f64)),
            duration: metadata.duration,
            linked_audio_clip_id: None, // Audio extraction happens async in background thread
        })
    }

    /// Auto-place an imported asset at playhead time
    /// Places images at document center, video/audio clips on appropriate layers
    fn auto_place_asset(&mut self, asset_info: ImportedAssetInfo) {
        use lightningbeam_core::clip::ClipInstance;
        use lightningbeam_core::layer::*;

        let drop_time = self.playback_time;
        let drop_beats = self.action_executor.document().tempo_map().seconds_to_beats(Seconds(drop_time));

        // Find or create a compatible layer
        let document = self.action_executor.document();
        let mut target_layer_id = None;

        // Check if active layer is compatible
        if let Some(active_id) = self.active_layer_id {
            if let Some(layer) = document.get_layer(&active_id) {
                if panes::layer_matches_clip_type(layer, asset_info.clip_type) {
                    target_layer_id = Some(active_id);
                }
            }
        }

        // If no compatible active layer, create a new layer
        if target_layer_id.is_none() {
            let layer_name = format!("{} Layer", match asset_info.clip_type {
                panes::DragClipType::Vector => "Vector",
                panes::DragClipType::Video => "Video",
                panes::DragClipType::AudioSampled => "Audio",
                panes::DragClipType::AudioMidi => "MIDI",
                panes::DragClipType::Image => "Image",
                panes::DragClipType::Effect => "Effect",
            });
            let new_layer = panes::create_layer_for_clip_type(asset_info.clip_type, &layer_name);

            // Create and execute add layer action
            let action = lightningbeam_core::actions::AddLayerAction::new(new_layer);
            let _ = self.action_executor.execute(Box::new(action));

            // Get the newly created layer ID (it's the last child in the document)
            let doc = self.action_executor.document();
            if let Some(last_layer) = doc.root.children.last() {
                let layer_id = last_layer.id();
                target_layer_id = Some(layer_id);

                // Update active layer to the new layer
                self.active_layer_id = target_layer_id;

                // If inside a clip, ensure a metatrack exists for it
                let editing_clip_id = self.editing_context.current_clip_id();
                let parent_track = editing_clip_id.and_then(|cid| self.ensure_metatrack_for_clip(cid));

                // Create a backend audio/MIDI track and add the mapping
                if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    match asset_info.clip_type {
                        panes::DragClipType::AudioSampled => {
                            match controller.create_audio_track_sync(layer_name.clone(), parent_track) {
                                Ok(track_id) => {
                                    self.layer_to_track_map.insert(layer_id, track_id);
                                    self.track_to_layer_map.insert(track_id, layer_id);
                                }
                                Err(e) => eprintln!("Failed to create audio track for auto-place: {}", e),
                            }
                        }
                        panes::DragClipType::AudioMidi => {
                            match controller.create_midi_track_sync(layer_name.clone(), parent_track) {
                                Ok(track_id) => {
                                    self.layer_to_track_map.insert(layer_id, track_id);
                                    self.track_to_layer_map.insert(track_id, layer_id);
                                }
                                Err(e) => eprintln!("Failed to create MIDI track for auto-place: {}", e),
                            }
                        }
                        _ => {} // Other types don't need backend tracks
                    }
                }
            }
        }

        // Add clip instance or shape to the target layer
        if let Some(layer_id) = target_layer_id {
            // For images, create an image-filled rectangle on the (vector) layer,
            // centered on the canvas at native size.
            if asset_info.clip_type == panes::DragClipType::Image {
                let (img_w, img_h) = asset_info.dimensions.unwrap_or((100.0, 100.0));
                let (doc_w, doc_h) = {
                    let d = self.action_executor.document();
                    (d.width as f64, d.height as f64)
                };
                let x = (doc_w - img_w) / 2.0;
                let y = (doc_h - img_h) / 2.0;
                let action = lightningbeam_core::actions::AddShapeAction::image_rect(
                    layer_id, self.playback_time, x, y, img_w, img_h, asset_info.clip_id,
                );
                if let Err(e) = self.action_executor.execute(Box::new(action)) {
                    eprintln!("Failed to place image: {}", e);
                }
            } else {
                // For clips, create a clip instance
                let mut clip_instance = ClipInstance::new(asset_info.clip_id)
                    .with_timeline_start(drop_beats);

                // For video clips, scale to fit and center in document
                if asset_info.clip_type == panes::DragClipType::Video {
                    if let Some((video_width, video_height)) = asset_info.dimensions {
                        let doc = self.action_executor.document();
                        // Fit uniformly + centered (preserve aspect). Shared with the timeline drag path.
                        clip_instance.transform.fit_centered(video_width, video_height, doc.width, doc.height);
                    }
                } else {
                    // Audio clips are centered in document
                    let doc = self.action_executor.document();
                    clip_instance.transform.x = doc.width / 2.0;
                    clip_instance.transform.y = doc.height / 2.0;
                }

                // Save instance ID for potential grouping
                let video_instance_id = clip_instance.id;

                // Create and execute action for video/audio with backend sync
                let action = lightningbeam_core::actions::AddClipInstanceAction::new(
                    layer_id,
                    clip_instance,
                );

                // Execute with backend synchronization (same as drag-from-library)
                if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    let mut backend_context = lightningbeam_core::action::BackendContext {
                        audio_controller: Some(&mut *controller),
                        layer_to_track_map: &self.layer_to_track_map,
                        clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                    };

                    if let Err(e) = self.action_executor.execute_with_backend(Box::new(action), &mut backend_context) {
                        eprintln!("❌ Failed to execute AddClipInstanceAction with backend: {}", e);
                    }
                } else {
                    // No audio controller, just execute without backend
                    let _ = self.action_executor.execute(Box::new(action));
                }

                // If video has linked audio, auto-place it and create group
                if let Some(linked_audio_clip_id) = asset_info.linked_audio_clip_id {
                    // Find or create sampled audio track
                    let audio_layer_id = {
                        let doc = self.action_executor.document();
                        panes::find_sampled_audio_track(doc)
                    }.unwrap_or_else(|| {
                        // Create new sampled audio layer
                        let audio_layer = AudioLayer::new_sampled("Audio Track");
                        self.action_executor.document_mut().root.add_child(
                            AnyLayer::Audio(audio_layer)
                        )
                    });

                    // Sync newly created audio layer with backend BEFORE adding clip instances
                    self.sync_audio_layers_to_backend();

                    // Create audio clip instance at same timeline position
                    let audio_instance = ClipInstance::new(linked_audio_clip_id)
                        .with_timeline_start(drop_beats);
                    let audio_instance_id = audio_instance.id;

                    // Execute audio action with backend sync
                    let audio_action = lightningbeam_core::actions::AddClipInstanceAction::new(
                        audio_layer_id,
                        audio_instance,
                    );

                    // Execute with backend synchronization
                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        let mut backend_context = lightningbeam_core::action::BackendContext {
                            audio_controller: Some(&mut *controller),
                            layer_to_track_map: &self.layer_to_track_map,
                            clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                        };

                        if let Err(e) = self.action_executor.execute_with_backend(Box::new(audio_action), &mut backend_context) {
                            eprintln!("❌ Failed to execute audio AddClipInstanceAction with backend: {}", e);
                        }
                    } else {
                        let _ = self.action_executor.execute(Box::new(audio_action));
                    }

                    // Create instance group linking video and audio
                    let mut group = lightningbeam_core::instance_group::InstanceGroup::new();
                    group.add_member(layer_id, video_instance_id);
                    group.add_member(audio_layer_id, audio_instance_id);
                    self.action_executor.document_mut().add_instance_group(group);
                }
            }
        }
    }

    /// Auto-place extracted audio for a video that was already placed
    fn auto_place_extracted_audio(&mut self, video_clip_id: uuid::Uuid, audio_clip_id: uuid::Uuid) {
        use lightningbeam_core::clip::ClipInstance;
        use lightningbeam_core::layer::*;

        // Find the video clip instance in the document
        let document = self.action_executor.document();
        let mut video_instance_info: Option<(uuid::Uuid, Beats, bool)> = None; // (layer_id, timeline_start [beats], already_in_group)

        // Search root layers for a video clip instance with matching clip_id
        for layer in &document.root.children {
            match layer {
                AnyLayer::Video(video_layer) => {
                    for instance in &video_layer.clip_instances {
                        if instance.clip_id == video_clip_id {
                            video_instance_info = Some((video_layer.layer.id, instance.timeline_start, false));
                            break;
                        }
                    }
                }
                AnyLayer::Group(group) => {
                    for child in &group.children {
                        if let AnyLayer::Video(video_layer) = child {
                            for instance in &video_layer.clip_instances {
                                if instance.clip_id == video_clip_id {
                                    video_instance_info = Some((video_layer.layer.id, instance.timeline_start, true));
                                    break;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            if video_instance_info.is_some() {
                break;
            }
        }

        // If we found a video instance, wrap it in a GroupLayer with a new AudioLayer
        if let Some((video_layer_id, timeline_start, already_in_group)) = video_instance_info {
            if already_in_group {
                // Video is already in a group (shouldn't happen normally, but handle it)
                println!("   ℹ️ Video already in a group layer, skipping auto-group");
                return;
            }

            // Get video name for the group
            let video_name = self.action_executor.document().video_clips
                .get(&video_clip_id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Video".to_string());

            // Remove the VideoLayer from root
            let video_layer_opt = {
                let doc = self.action_executor.document_mut();
                let idx = doc.root.children.iter().position(|l| l.id() == video_layer_id);
                idx.map(|i| doc.root.children.remove(i))
            };

            let Some(video_layer) = video_layer_opt else {
                eprintln!("❌ Could not find video layer {} in root to move into group", video_layer_id);
                return;
            };

            // Create AudioLayer for the extracted audio
            let audio_layer = AudioLayer::new_sampled("Audio");
            let audio_layer_id = audio_layer.layer.id;

            // Build GroupLayer containing both
            let mut group = GroupLayer::new(video_name);
            group.expanded = false; // start collapsed
            group.add_child(video_layer);
            group.add_child(AnyLayer::Audio(audio_layer));
            let group_id = group.layer.id;

            // Add GroupLayer to root
            self.action_executor.document_mut().root.add_child(AnyLayer::Group(group));

            // Sync backend (creates metatrack for group + audio track as child)
            self.sync_audio_layers_to_backend();

            // Create audio clip instance at same timeline position as video
            let audio_instance = ClipInstance::new(audio_clip_id)
                .with_timeline_start(timeline_start);

            // Execute audio clip placement with backend sync
            let audio_action = lightningbeam_core::actions::AddClipInstanceAction::new(
                audio_layer_id,
                audio_instance,
            );

            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                let mut backend_context = lightningbeam_core::action::BackendContext {
                    audio_controller: Some(&mut *controller),
                    layer_to_track_map: &self.layer_to_track_map,
                    clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                };

                if let Err(e) = self.action_executor.execute_with_backend(Box::new(audio_action), &mut backend_context) {
                    eprintln!("❌ Failed to place extracted audio clip: {}", e);
                }
            } else {
                let _ = self.action_executor.execute(Box::new(audio_action));
            }

            println!("   🔗 Created group layer '{}' with video + audio", group_id);
        }
    }

    /// Handle audio extraction results from background thread
    fn handle_audio_extraction_result(&mut self, result: AudioExtractionResult) {
        match result {
            AudioExtractionResult::Success {
                video_clip_id,
                audio_clip,
                pool_index,
                video_name,
                channels,
                sample_rate,
            } => {
                // Add AudioClip to document
                let audio_clip_id = self.action_executor.document_mut().add_audio_clip(audio_clip);

                // Update VideoClip's linked_audio_clip_id
                if let Some(video_clip) = self.action_executor.document_mut().video_clips
                    .get_mut(&video_clip_id)
                {
                    video_clip.linked_audio_clip_id = Some(audio_clip_id);

                    // Get audio clip duration for logging
                    let duration = self.action_executor.document().audio_clips
                        .get(&audio_clip_id)
                        .map(|c| c.content_duration().native())
                        .unwrap_or(0.0);

                    println!("✅ Extracted audio from '{}' ({:.1}s, {}ch, {}Hz) - AudioClip ID: {}",
                        video_name,
                        duration,
                        channels,
                        sample_rate,
                        audio_clip_id
                    );

                    // Fetch raw audio samples for GPU waveform rendering
                    if let Some(ref controller_arc) = self.audio_controller {
                        let mut controller = controller_arc.lock().unwrap();
                        match controller.get_pool_audio_samples(pool_index) {
                            Ok((samples, sr, ch)) => {
                                self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sr, ch));
                                self.waveform_gpu_dirty.insert(pool_index);
                            }
                            Err(e) => eprintln!("Failed to fetch raw audio for extracted audio: {}", e),
                        }
                    }

                    // Auto-place extracted audio if the video was auto-placed
                    self.auto_place_extracted_audio(video_clip_id, audio_clip_id);
                } else {
                    eprintln!("⚠️  Audio extracted but VideoClip {} not found (may have been deleted)", video_clip_id);
                }
            }
            AudioExtractionResult::Error { video_clip_id, error } => {
                eprintln!("❌ Failed to extract audio from video {}: {}", video_clip_id, error);
            }
        }
    }
}

impl eframe::App for EditorApp {
    /// Clean shutdown → not a crash → delete this session's recovery file so it isn't offered for
    /// restore on the next launch. (If we crash instead, `on_exit` never runs and the file remains.)
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(path) = self.autosave.recovery_path.take() {
            let _ = std::fs::remove_file(&path);
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.tablet.poll(ctx, raw_input, self.selected_tool);

        // Pressure and tilt are now in the global AtomicU32 cells (tablet::current_pressure /
        // current_tilt) and will be picked up by make_stroke_point on the next stroke tick.
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let _frame_start = std::time::Instant::now();

        // Apply the theme's palette to egui's global visuals so the standard widgets (dialogs, pane
        // chrome, text) share the same colors as the mobile UI and respond to light/dark/user themes.
        self.theme.apply_to_egui(ctx);
        // On mobile, enlarge egui spacing/sizing so widgets in panes/dialogs are touch-friendly.
        if self.mobile_active() {
            mobile::apply_touch_style(ctx);
        }

        // Background crash-recovery autosave (cheap early-out unless dirty + interval elapsed).
        self.maybe_autosave(ctx);
        // Offer to restore a previous session's unsaved work (if a recovery file was left behind).
        self.render_recovery_prompt(ctx);
        // Prompt to save unsaved changes before switching files (New / Open / Open Recent).
        self.render_unsaved_prompt(ctx);

        // === Raster fault-in (Phase 3 paging) ===
        // The canvas records raster keyframe ids whose `raw_pixels` weren't resident
        // (it can't mutate the document while rendering). Drain that sink here, BEFORE
        // any rendering, and fault the pixels in from the project container. A drain of
        // an empty set is ~free, so this is cheap when everything is already resident.
        {
            // 1) Apply completed background page-ins.
            let mut any_applied = false;
            while let Ok((kf_id, result)) = self.raster_load_result_rx.try_recv() {
                self.raster_loads_inflight.remove(&kf_id);
                let doc = self.action_executor.document_mut();
                let kf = doc.all_layers_mut().into_iter().find_map(|l| match l {
                    lightningbeam_core::layer::AnyLayer::Raster(rl) => {
                        rl.keyframes.iter_mut().find(|kf| kf.id == kf_id)
                    }
                    _ => None,
                });
                if let Some(kf) = kf {
                    // Don't clobber pixels edited while the load was in flight.
                    if kf.needs_fault_in && kf.raw_pixels.is_empty() {
                        if let Some(pixels) = result {
                            kf.raw_pixels = pixels;
                            kf.texture_dirty = true;
                            any_applied = true;
                            // Record fault-in recency: most-recently paged-in is
                            // moved to the back so it's evicted last. The shown
                            // frame is always the most recent, so it's protected.
                            self.raster_resident_lru.retain(|id| *id != kf_id);
                            self.raster_resident_lru.push_back(kf_id);
                        }
                        // Resolved (loaded, or genuinely absent/blank).
                        kf.needs_fault_in = false;
                    }
                }
            }
            if any_applied {
                ctx.request_repaint();
            }

            // 1b) Evict over-budget resident keyframes (fault-in-recency LRU).
            // Drop the pixels of the oldest *clean* keyframes and re-arm their
            // fault-in so they re-page on revisit. DIRTY keyframes are never
            // evicted (their pixels aren't in the container yet — evicting would
            // silently lose the unsaved edit), so we just unpin them from the LRU.
            while self.raster_resident_lru.len() > Self::RASTER_RESIDENT_MAX {
                let old = self.raster_resident_lru.pop_front().unwrap();
                let doc = self.action_executor.document_mut();
                let kf = doc.all_layers_mut().into_iter().find_map(|l| match l {
                    lightningbeam_core::layer::AnyLayer::Raster(rl) => {
                        rl.keyframes.iter_mut().find(|kf| kf.id == old)
                    }
                    _ => None,
                });
                if let Some(kf) = kf {
                    if !kf.dirty && !kf.raw_pixels.is_empty() {
                        kf.raw_pixels = Vec::new();
                        kf.needs_fault_in = true;
                        kf.texture_dirty = true;
                    }
                }
            }

            // 2) Dispatch background page-ins: reactive misses, plus (during playback)
            //    a prefetch of upcoming keyframes so their full pixels are resident
            //    before the playhead reaches them — otherwise every frame shows the
            //    low-res proxy and the proxy→full pops read as flicker.
            let mut to_load: Vec<uuid::Uuid> = {
                let mut s = self.raster_fault_requests.lock().unwrap();
                s.drain().collect()
            };
            if self.is_playing && self.raster_store.has_path() {
                const PREFETCH_AHEAD: usize = 4;
                let t = self.playback_time;
                let doc = self.action_executor.document();
                for layer in doc.all_layers() {
                    if let lightningbeam_core::layer::AnyLayer::Raster(rl) = layer {
                        for kf in rl.keyframes.iter().filter(|kf| kf.time >= t).take(PREFETCH_AHEAD) {
                            if kf.raw_pixels.is_empty() && kf.needs_fault_in {
                                to_load.push(kf.id);
                            }
                        }
                    }
                }
            }
            if self.raster_store.has_path() {
                for kf_id in to_load {
                    if self.raster_loads_inflight.contains(&kf_id) {
                        continue;
                    }
                    self.raster_loads_inflight.insert(kf_id);
                    let store = self.raster_store.clone();
                    let tx = self.raster_load_result_tx.clone();
                    std::thread::spawn(move || {
                        let pixels = store.load_pixels(kf_id);
                        let _ = tx.send((kf_id, pixels));
                    });
                }
            }

            // Keep ticking while page-ins are running so their results get applied
            // promptly (a background thread can't wake egui on its own).
            if !self.raster_loads_inflight.is_empty() {
                ctx.request_repaint();
            }
        }

        // Consume any pending tool switch from the tablet (eraser in/out).
        if let Some(tool) = self.tablet.pending_tool_switch.take() {
            self.selected_tool = tool;
        }

        // Force continuous repaint if we have pending waveform updates
        // This ensures thumbnails update immediately when waveform data arrives
        if !self.audio_pools_with_new_waveforms.is_empty() {
            ctx.request_repaint();
        }

        // Poll audio extraction results from background threads
        while let Ok(result) = self.audio_extraction_rx.try_recv() {
            self.handle_audio_extraction_result(result);
        }

        // Poll completed waveform-overview pyramids (packed min/max floors).
        // Stored in the same cache the GPU upload reads from, but flagged as
        // min/max so the renderer uploads via the (Lmin,Lmax,Rmin,Rmax) path.
        while let Ok((pool_index, packed, sample_rate, channels, b, blob)) = self.waveform_result_rx.try_recv() {
            self.raw_audio_cache.insert(pool_index, (Arc::new(packed), sample_rate, channels));
            self.waveform_minmax_pools.insert(pool_index, b.max(1));
            self.waveform_pyramid_blobs.insert(pool_index, Arc::new(blob));
            self.waveform_gpu_dirty.insert(pool_index);
            self.audio_pools_with_new_waveforms.insert(pool_index);
        }

        // Webcam management: open/close based on camera_enabled layers, poll frames
        {
            let any_camera_enabled = self.action_executor.document().all_layers().iter().any(|layer| {
                matches!(layer, lightningbeam_core::layer::AnyLayer::Video(v) if v.camera_enabled)
            });

            if any_camera_enabled && self.webcam.is_none() {
                // Try to open the default camera
                if let Some(device) = lightningbeam_core::webcam::default_camera() {
                    match lightningbeam_core::webcam::WebcamCapture::open(&device) {
                        Ok(cam) => {
                            eprintln!("[WEBCAM] Opened camera: {}", device.name);
                            self.webcam = Some(cam);
                        }
                        Err(e) => {
                            eprintln!("[WEBCAM] Failed to open camera: {}", e);
                        }
                    }
                }
            } else if !any_camera_enabled && self.webcam.is_some() {
                eprintln!("[WEBCAM] Closing camera (no layers with camera enabled)");
                self.webcam = None;
                self.webcam_frame = None;
            }

            // Poll latest frame from webcam
            if let Some(webcam) = &mut self.webcam {
                if let Some(frame) = webcam.poll_frame() {
                    self.webcam_frame = Some(frame.clone());
                    ctx.request_repaint(); // Keep repainting while camera is active
                }
            }
        }

        // Check for native menu events (macOS)
        if let Some(menu_system) = &self.menu_system {
            if let Some(action) = menu_system.check_events() {
                self.handle_menu_action(action);
            }
        }

        // Handle pending auto-reopen (first frame only)
        if let Some(path) = self.pending_auto_reopen.take() {
            self.load_from_file(path);
            // Will switch to editor mode when file finishes loading
        }

        // NOTE: Missing raw audio samples for newly imported files will arrive
        // via AudioDecodeProgress events (compressed) or inline with AudioFileReady
        // (PCM). No blocking query needed here.
        // For project loading, audio files are re-imported which also sends events.

        // Initialize and update effect thumbnail generator (GPU-based effect previews)
        if let Some(render_state) = frame.wgpu_render_state() {
            let device = &render_state.device;
            let queue = &render_state.queue;

            // Initialize on first GPU access
            if self.effect_thumbnail_generator.is_none() {
                self.effect_thumbnail_generator = Some(EffectThumbnailGenerator::new(device, queue));
                println!("✅ Effect thumbnail generator initialized");
            }

            // Process effect thumbnail invalidations from previous frame
            // This happens BEFORE UI rendering so asset library will see empty GPU cache
            // We only invalidate GPU cache here - asset library will see the list during render
            // and invalidate its own ThumbnailCache
            if !self.effect_thumbnails_to_invalidate.is_empty() {
                if let Some(generator) = &mut self.effect_thumbnail_generator {
                    for effect_id in &self.effect_thumbnails_to_invalidate {
                        generator.invalidate(effect_id);
                    }
                }
                // DON'T clear here - asset library still needs to see these during render
            }

            // Generate pending effect thumbnails (up to 2 per frame to avoid stalls)
            if let Some(generator) = &mut self.effect_thumbnail_generator {
                // Combine built-in effects from registry with custom effects from document
                let mut all_effects: HashMap<Uuid, lightningbeam_core::effect::EffectDefinition> = HashMap::new();
                for def in lightningbeam_core::effect_registry::EffectRegistry::get_all() {
                    all_effects.insert(def.id, def);
                }
                for (id, def) in &self.action_executor.document().effect_definitions {
                    all_effects.insert(*id, def.clone());
                }

                let generated = generator.generate_pending(device, queue, &all_effects, 2);
                if generated > 0 {
                    // Request repaint to continue generating remaining thumbnails
                    if generator.pending_count() > 0 {
                        ctx.request_repaint();
                    }
                }
            }
        }

        // Handle file operation progress
        if let Some(ref mut operation) = self.file_operation {
            // Set wait cursor
            ctx.set_cursor_icon(egui::CursorIcon::Progress);

            // Poll for progress updates
            let mut operation_complete = false;
            let mut loaded_project_data: Option<(lightningbeam_core::file_io::LoadedProject, std::path::PathBuf)> = None;
            let mut update_recent_menu = false; // Track if we need to update recent files menu
            // An action that was waiting on this save (from the unsaved-changes prompt), to run
            // after the file_operation borrow ends.
            let mut after_save: Option<PendingAction> = None;

            match operation {
                FileOperation::Saving { ref mut progress_rx, ref path } => {
                    while let Ok(progress) = progress_rx.try_recv() {
                        match progress {
                            FileProgress::Done => {
                                println!("✅ Save complete!");
                                self.current_file_path = Some(path.clone());
                                // Manual save persisted everything to the user's file → no unsaved
                                // work to recover; rebase the autosave epoch so it stays quiet until
                                // the next real edit. (Inlined rather than reset_autosave_baseline()
                                // to avoid a second &mut self borrow inside the file_operation match.)
                                self.autosave.baseline_epoch = self.action_executor.epoch();
                                self.autosave.pending_event = false;
                                self.autosave.last_time = None;
                                // The document now matches its file → no unsaved changes.
                                self.saved_epoch = self.action_executor.epoch();
                                self.media_modified = false;
                                // If a file switch was waiting on this save, run it after the borrow.
                                after_save = self.after_save.take();
                                // Container path may be new (Save As); update the
                                // raster paging store so future faults read the right file.
                                self.raster_store.set_path(self.current_file_path.clone());

                                // All raster keyframes are now persisted in the
                                // container, so none are dirty — clearing the flag
                                // lets eviction reclaim their pixels again.
                                {
                                    use lightningbeam_core::layer::AnyLayer;
                                    let mut resident_ids = Vec::new();
                                    let doc = self.action_executor.document_mut();
                                    for layer in doc.all_layers_mut() {
                                        if let AnyLayer::Raster(rl) = layer {
                                            for kf in rl.keyframes.iter_mut() {
                                                kf.dirty = false;
                                                if !kf.raw_pixels.is_empty() {
                                                    resident_ids.push(kf.id);
                                                }
                                            }
                                        }
                                    }
                                    // Frames edited this session were unpinned from the
                                    // eviction LRU while dirty; now that they're clean and
                                    // persisted, re-track them so the resident bound applies
                                    // (the next fault-in trims the oldest, not the shown one).
                                    for id in resident_ids {
                                        self.raster_resident_lru.retain(|x| *x != id);
                                        self.raster_resident_lru.push_back(id);
                                    }
                                }

                                // Add to recent files
                                self.config.add_recent_file(path.clone());
                                update_recent_menu = true;

                                operation_complete = true;
                            }
                            FileProgress::Error(e) => {
                                eprintln!("❌ Save error: {}", e);
                                operation_complete = true;
                            }
                            _ => {
                                // Other progress states - just keep going
                            }
                        }
                    }

                    // Render progress dialog
                    egui::Window::new("Saving Project")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Spinner::new());
                                ui.add_space(8.0);
                                ui.label("Saving project...");
                                ui.label(format!("Path: {}", path.display()));
                            });
                        });
                }
                FileOperation::Loading { ref mut progress_rx, ref path } => {
                    while let Ok(progress) = progress_rx.try_recv() {
                        match progress {
                            FileProgress::Complete(Ok(loaded_project)) => {
                                println!("✅ Load complete!");
                                // Store data to apply after dialog closes
                                loaded_project_data = Some((loaded_project, path.clone()));
                                operation_complete = true;
                            }
                            FileProgress::Complete(Err(e)) => {
                                eprintln!("❌ Load error: {}", e);
                                operation_complete = true;
                            }
                            FileProgress::Error(e) => {
                                eprintln!("❌ Load error: {}", e);
                                operation_complete = true;
                            }
                            _ => {
                                // Other progress states - just keep going
                            }
                        }
                    }

                    // Render progress dialog
                    egui::Window::new("Loading Project")
                        .collapsible(false)
                        .resizable(false)
                        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                        .show(ctx, |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add(egui::Spinner::new());
                                ui.add_space(8.0);
                                ui.label("Loading project...");
                                ui.label(format!("Path: {}", path.display()));
                            });
                        });
                }
            }

            // Clear operation if complete
            if operation_complete {
                self.file_operation = None;
            }

            // Apply loaded project data if available
            if let Some((loaded_project, path)) = loaded_project_data {
                self.apply_loaded_project(loaded_project, path);
                // Switch to editor mode after loading a project
                self.app_mode = AppMode::Editor;
            }

            // Update recent files menu if needed
            if update_recent_menu {
                self.update_recent_files_menu();
            }

            // An action that was waiting on this save ("Save" in the prompt → New/Open/Quit) runs now.
            if let Some(action) = after_save {
                self.do_action(action, ctx);
            }

            // Request repaint to keep updating progress
            ctx.request_repaint();
        }

        let _pre_events_ms = _frame_start.elapsed().as_secs_f64() * 1000.0;
        // Keep repainting while waiting for graph preset loads to complete
        if self.pending_graph_loads.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            ctx.request_repaint();
        }

        // Drain recording mirror buffer for live waveform display
        if self.is_recording {
            if let Some(ref mut mirror_rx) = self.recording_mirror_rx {
                let mut drained = 0usize;
                if let Some(entry) = self.raw_audio_cache.get_mut(&usize::MAX) {
                    let samples = Arc::make_mut(&mut entry.0);
                    while let Ok(sample) = mirror_rx.pop() {
                        samples.push(sample);
                        drained += 1;
                    }
                }
                if drained > 0 {
                    self.waveform_gpu_dirty.insert(usize::MAX);
                    ctx.request_repaint();
                }
            }
        }

        // Poll audio events from the audio engine
        if let Some(event_rx) = &mut self.audio_event_rx {
            while let Ok(event) = event_rx.pop() {
                    use daw_backend::AudioEvent;
                    match event {
                        AudioEvent::PlaybackPosition(time) => {
                            self.playback_time = time;
                        }
                        AudioEvent::PlaybackStopped => {
                            self.is_playing = false;
                        }
                        AudioEvent::ExportProgress { frames_rendered, total_frames } => {
                            // Update export progress dialog with actual render progress
                            let progress = frames_rendered as f32 / total_frames as f32;
                            self.export_progress_dialog.update_progress(
                                format!("Rendering: {} / {} frames", frames_rendered, total_frames),
                                progress,
                            );
                            ctx.request_repaint();
                        }
                        AudioEvent::ExportFinalizing => {
                            self.export_progress_dialog.update_progress(
                                "Finalizing...".to_string(),
                                1.0,
                            );
                            ctx.request_repaint();
                        }
                        AudioEvent::WaveformChunksReady { pool_index, .. } => {
                            // Skip synchronous audio queries during export (audio thread is blocked)
                            let is_exporting = self.export_orchestrator.as_ref()
                                .map_or(false, |o| o.is_exporting());

                            if !is_exporting && !self.raw_audio_cache.contains_key(&pool_index) {
                                if let Some(ref controller_arc) = self.audio_controller {
                                    let mut controller = controller_arc.lock().unwrap();
                                    match controller.get_pool_audio_samples(pool_index) {
                                        Ok((samples, sr, ch)) => {
                                            self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sr, ch));
                                            self.waveform_gpu_dirty.insert(pool_index);
                                            self.audio_pools_with_new_waveforms.insert(pool_index);
                                        }
                                        Err(e) => eprintln!("Failed to fetch raw audio for pool {}: {}", pool_index, e),
                                    }
                                }
                            }

                            ctx.request_repaint();
                        }
                        // Recording events
                        AudioEvent::RecordingStarted(track_id, backend_clip_id, rec_sample_rate, rec_channels) => {
                            println!("🎤 Recording started on track {:?}, backend_clip_id={}", track_id, backend_clip_id);

                            // Create clip in document and add instance to the layer for this track
                            if let Some(&layer_id) = self.track_to_layer_map.get(&track_id) {
                                if self.recording_layer_ids.contains(&layer_id) {
                                    use lightningbeam_core::clip::{AudioClip, ClipInstance};

                                    // Create a recording-in-progress clip (no pool index yet)
                                    let clip = AudioClip::new_recording("Recording...");
                                    let doc_clip_id = self.action_executor.document_mut().add_audio_clip(clip);

                                    // Create clip instance on the layer (recording_start_time is seconds)
                                    let rec_start_beats = self.action_executor.document().tempo_map().seconds_to_beats(Seconds(self.recording_start_time));
                                    let clip_instance = ClipInstance::new(doc_clip_id)
                                        .with_timeline_start(rec_start_beats);

                                    let clip_instance_id = clip_instance.id;

                                    // Add instance to layer (works for root and inside movie clips)
                                    if let Some(layer) = self.action_executor.document_mut().get_layer_mut(&layer_id) {
                                        if let lightningbeam_core::layer::AnyLayer::Audio(audio_layer) = layer {
                                            audio_layer.clip_instances.push(clip_instance);
                                            println!("✅ Created recording clip instance on layer {}", layer_id);
                                        }
                                    }

                                    // Insert mapping so the snapshot cache assigns the doc UUID to this recording clip
                                    self.clip_instance_to_backend_map.insert(
                                        clip_instance_id,
                                        lightningbeam_core::action::BackendClipInstanceId::Audio(backend_clip_id),
                                    );

                                    // Store mapping for later updates
                                    self.recording_clips.insert(layer_id, backend_clip_id);
                                }
                            }

                            // Initialize live waveform cache for recording
                            self.raw_audio_cache.insert(usize::MAX, (Arc::new(Vec::new()), rec_sample_rate, rec_channels));

                            ctx.request_repaint();
                        }
                        AudioEvent::RecordingProgress(_backend_clip_id, duration) => {
                            // Update clip duration as recording progresses
                            // Find which layer this backend clip belongs to via recording_clips
                            let layer_id = self.recording_clips.iter()
                                .find(|(_, &cid)| cid == _backend_clip_id)
                                .map(|(&lid, _)| lid);
                            if let Some(layer_id) = layer_id {
                                // First, find the doc clip_id from the layer (read-only borrow)
                                let doc_clip_id = {
                                    let document = self.action_executor.document();
                                    document.get_layer(&layer_id)
                                        .and_then(|layer| {
                                            if let lightningbeam_core::layer::AnyLayer::Audio(audio_layer) = layer {
                                                audio_layer.clip_instances.last().map(|i| i.clip_id)
                                            } else {
                                                None
                                            }
                                        })
                                };

                                // Then update the clip duration (mutable borrow)
                                if let Some(doc_clip_id) = doc_clip_id {
                                    if let Some(clip) = self.action_executor.document_mut().audio_clips.get_mut(&doc_clip_id) {
                                        if clip.is_recording() {
                                            clip.set_content_duration(ClipDuration::Seconds(duration));
                                        }
                                    }
                                }
                            }
                            ctx.request_repaint();
                        }
                        AudioEvent::RecordingStopped(_backend_clip_id, pool_index, _waveform) => {
                            eprintln!("[STOP] AudioEvent::RecordingStopped received (pool_index={})", pool_index);

                            // Clean up live recording waveform cache
                            self.raw_audio_cache.remove(&usize::MAX);
                            self.waveform_gpu_dirty.remove(&usize::MAX);

                            // Fetch raw audio samples for GPU waveform rendering
                            if let Some(ref controller_arc) = self.audio_controller {
                                let mut controller = controller_arc.lock().unwrap();
                                match controller.get_pool_audio_samples(pool_index) {
                                    Ok((samples, sr, ch)) => {
                                        self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sr, ch));
                                        self.waveform_gpu_dirty.insert(pool_index);
                                        self.audio_pools_with_new_waveforms.insert(pool_index);
                                    }
                                    Err(e) => eprintln!("Failed to fetch raw audio after recording: {}", e),
                                }
                            }

                            // Get accurate duration from backend (not calculated from waveform peaks)
                            let duration = if let Some(ref controller_arc) = self.audio_controller {
                                let mut controller = controller_arc.lock().unwrap();
                                match controller.get_pool_file_info(pool_index) {
                                    Ok((dur, _, _)) => {
                                        eprintln!("[AUDIO] Got duration from backend: {:.4}s", dur);
                                        self.audio_duration_cache.insert(pool_index, dur);
                                        dur
                                    }
                                    Err(e) => {
                                        eprintln!("⚠️  Failed to get pool file info: {}", e);
                                        0.0
                                    }
                                }
                            } else {
                                0.0
                            };

                            // Finalize the recording clip with real pool_index and duration
                            // and sync to backend for playback
                            // Find which layer this recording belongs to via recording_clips
                            let recording_layer = self.recording_clips.iter()
                                .find(|(_, &cid)| cid == _backend_clip_id)
                                .map(|(&lid, _)| lid);
                            if let Some(layer_id) = recording_layer {
                                // First, find the clip instance and clip id
                                let (clip_id, instance_id, _timeline_start, _trim_start) = {
                                    let document = self.action_executor.document();
                                    document.get_layer(&layer_id)
                                        .and_then(|layer| {
                                            if let lightningbeam_core::layer::AnyLayer::Audio(audio_layer) = layer {
                                                audio_layer.clip_instances.last().map(|instance| {
                                                    (instance.clip_id, instance.id, instance.timeline_start, instance.trim_start)
                                                })
                                            } else {
                                                None
                                            }
                                        })
                                        .unwrap_or((uuid::Uuid::nil(), uuid::Uuid::nil(), Beats::ZERO, 0.0))
                                };

                                if !clip_id.is_nil() {
                                    // Finalize the clip (update pool_index and duration).
                                    self.autosave.pending_event = true;
                                    if let Some(clip) = self.action_executor.document_mut().audio_clips.get_mut(&clip_id) {
                                        if clip.finalize_recording(pool_index, duration) {
                                            clip.name = format!("Recording {}", pool_index);
                                            eprintln!("[AUDIO] Finalized recording clip: pool={}, duration={:.4}s", pool_index, duration);
                                        }
                                    }

                                    // The backend already has the clip at _backend_clip_id (created
                                    // during handle_start_recording and finalized in handle_stop_recording).
                                    // Map the document instance_id → the existing backend clip so that
                                    // delete/move/trim actions can reference it correctly.
                                    // DO NOT call AddAudioClipSync — that would create a duplicate clip.
                                    let backend_id = lightningbeam_core::action::BackendClipInstanceId::Audio(_backend_clip_id);
                                    self.clip_instance_to_backend_map.insert(instance_id, backend_id);
                                    eprintln!("[AUDIO] Mapped doc instance {} → backend clip {}", instance_id, _backend_clip_id);

                                    // Register the finished recording as an already-applied action so
                                    // it bumps the epoch (marks the document modified for save-on-close
                                    // and autosave) and can be undone/redone like any other edit.
                                    let clip_instance = self.layer_to_track_map.get(&layer_id).copied().and_then(|track_id| {
                                        self.action_executor.document()
                                            .get_layer(&layer_id)
                                            .and_then(|l| if let AnyLayer::Audio(al) = l {
                                                al.clip_instances.iter().find(|ci| ci.id == instance_id).cloned()
                                            } else { None })
                                            .map(|ci| (track_id, ci))
                                    });
                                    if let Some((track_id, clip_instance)) = clip_instance {
                                        let action = lightningbeam_core::actions::AddClipInstanceAction::already_applied(
                                            layer_id, clip_instance, track_id, backend_id,
                                        );
                                        self.action_executor.push_applied(Box::new(action));
                                    } else {
                                        // Couldn't build the action; still mark modified so the recording
                                        // isn't silently lost on close.
                                        self.media_modified = true;
                                    }
                                }
                            }

                            // Remove this layer from active recordings
                            if let Some(layer_id) = recording_layer {
                                self.recording_layer_ids.retain(|id| *id != layer_id);
                                self.recording_clips.remove(&layer_id);
                            }
                            // Clear global recording state only when all recordings are done
                            if self.recording_layer_ids.is_empty() {
                                self.is_recording = false;
                                self.recording_clips.clear();
                            }
                            ctx.request_repaint();
                        }
                        AudioEvent::RecordingError(message) => {
                            eprintln!("❌ Recording error: {}", message);
                            self.is_recording = false;
                            self.recording_clips.clear();
                            self.recording_layer_ids.clear();
                            ctx.request_repaint();
                        }
                        AudioEvent::MidiRecordingProgress(_track_id, clip_id, duration, notes) => {
                            // Update clip duration in document (so timeline bar grows)
                            // Find layer for this track via track_to_layer_map
                            let midi_layer_id = self.track_to_layer_map.get(&_track_id)
                                .filter(|lid| self.recording_layer_ids.contains(lid))
                                .copied();
                            if let Some(layer_id) = midi_layer_id {
                                // Lazily create the doc clip + instance on the first progress event
                                // (there is no MidiRecordingStarted event from the backend).
                                //
                                // MidiClipId (clip_id) is the content ID; MidiClipInstanceId is
                                // the placement ID used in the snapshot and backend operations.
                                // We need to store the instance ID, not the content ID, so that
                                // build_audio_clip_cache can correlate mc.id → doc UUID.
                                // Command::CreateMidiClip has already been processed and the
                                // snapshot refreshed by the time this event arrives.
                                let backend_instance_id: u32 = if let Some(ref controller_arc) = self.audio_controller {
                                    let controller = controller_arc.lock().unwrap();
                                    let snap = controller.clip_snapshot();
                                    let snap = snap.read().unwrap();
                                    snap.midi.get(&_track_id)
                                        .and_then(|instances| instances.iter().find(|mc| mc.clip_id == clip_id))
                                        .map(|mc| mc.id)
                                        .unwrap_or(clip_id)
                                } else {
                                    clip_id
                                };

                                // Find the Midi-typed clip instance the timeline already created.
                                // Register it in the map (using the correct instance ID, not the
                                // content ID) so trim/move actions can find it via the snapshot.
                                let already_mapped = self.clip_instance_to_backend_map.values().any(|v| {
                                    matches!(v, lightningbeam_core::action::BackendClipInstanceId::Midi(id) if *id == backend_instance_id)
                                });
                                let doc_clip_id = {
                                    let doc = self.action_executor.document();
                                    doc.audio_clip_by_midi_clip_id(clip_id).map(|(id, _)| id)
                                };
                                if let Some(doc_clip_id) = doc_clip_id {
                                    if !already_mapped {
                                        // Find the clip instance for this clip on the layer
                                        let instance_id = {
                                            let doc = self.action_executor.document();
                                            doc.get_layer(&layer_id)
                                                .and_then(|l| {
                                                    if let lightningbeam_core::layer::AnyLayer::Audio(al) = l {
                                                        al.clip_instances.iter()
                                                            .find(|ci| ci.clip_id == doc_clip_id)
                                                            .map(|ci| ci.id)
                                                    } else { None }
                                                })
                                        };
                                        if let Some(instance_id) = instance_id {
                                            self.clip_instance_to_backend_map.insert(
                                                instance_id,
                                                lightningbeam_core::action::BackendClipInstanceId::Midi(backend_instance_id),
                                            );
                                        }
                                    }
                                    // Update the clip's duration so the timeline bar grows
                                    if let Some(clip) = self.action_executor.document_mut().audio_clips.get_mut(&doc_clip_id) {
                                        clip.set_content_duration(ClipDuration::Beats(duration));
                                    }
                                }
                            }

                            // Update midi_event_cache with notes captured so far.
                            // Notes from get_notes_with_active() are already in beats.
                            {
                                let mut events: Vec<daw_backend::audio::midi::MidiEvent> = Vec::with_capacity(notes.len() * 2);
                                for &(start_beats, note, velocity, dur_beats) in &notes {
                                    events.push(daw_backend::audio::midi::MidiEvent::note_on(start_beats, 0, note, velocity));
                                    events.push(daw_backend::audio::midi::MidiEvent::note_off(start_beats + dur_beats, 0, note, 0));
                                }
                                events.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
                                self.midi_event_cache.insert(clip_id, events);
                            }
                            ctx.request_repaint();
                        }
                        AudioEvent::MidiRecordingStopped(track_id, clip_id, note_count) => {
                            println!("🎹 MIDI recording stopped: track={:?}, clip_id={}, {} notes",
                                     track_id, clip_id, note_count);

                            // Query backend for the definitive final note data
                            if let Some(ref controller_arc) = self.audio_controller {
                                let mut controller = controller_arc.lock().unwrap();
                                match controller.query_midi_clip(track_id, clip_id) {
                                    Ok(midi_clip_data) => {
                                        drop(controller);
                                        self.midi_event_cache.insert(clip_id, midi_clip_data.events.clone());

                                        // Update document clip with final duration and name
                                        let doc_clip_id = self.action_executor.document()
                                            .audio_clip_by_midi_clip_id(clip_id)
                                            .map(|(id, _)| id);
                                        if let Some(doc_clip_id) = doc_clip_id {
                                            if let Some(clip) = self.action_executor.document_mut().audio_clips.get_mut(&doc_clip_id) {
                                                clip.set_content_duration(ClipDuration::Beats(Beats(midi_clip_data.duration)));
                                                clip.name = format!("MIDI Recording {}", clip_id);
                                            }
                                        }

                                        println!("✅ Finalized MIDI recording: {} notes, {:.2}s",
                                                 note_count, midi_clip_data.duration);
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to query MIDI clip data after recording: {}", e);
                                        // Cache was already populated by last MidiRecordingProgress event
                                    }
                                }
                            }

                            // Register the finished MIDI recording as an already-applied action so it
                            // marks the document modified (save-on-close / autosave) and is undoable,
                            // like the audio path. The backend instance id was mapped during
                            // MidiRecordingProgress.
                            if let Some(&layer_id) = self.track_to_layer_map.get(&track_id) {
                                let doc_clip_id = self.action_executor.document()
                                    .audio_clip_by_midi_clip_id(clip_id).map(|(id, _)| id);
                                if let Some(doc_clip_id) = doc_clip_id {
                                    let instance = self.action_executor.document()
                                        .get_layer(&layer_id)
                                        .and_then(|l| if let AnyLayer::Audio(al) = l {
                                            al.clip_instances.iter().find(|ci| ci.clip_id == doc_clip_id).cloned()
                                        } else { None });
                                    if let Some(instance) = instance {
                                        if let Some(&backend_id) = self.clip_instance_to_backend_map.get(&instance.id) {
                                            let action = lightningbeam_core::actions::AddClipInstanceAction::already_applied(
                                                layer_id, instance, track_id, backend_id,
                                            );
                                            self.action_executor.push_applied(Box::new(action));
                                        } else {
                                            // No backend mapping (e.g. snapshot lookup missed); still mark
                                            // modified so the recording isn't silently lost on close.
                                            self.media_modified = true;
                                        }
                                    }
                                }
                            }

                            // Remove this MIDI layer from active recordings
                            if let Some(&layer_id) = self.track_to_layer_map.get(&track_id) {
                                self.recording_layer_ids.retain(|id| *id != layer_id);
                                self.recording_clips.remove(&layer_id);
                            }
                            if self.recording_layer_ids.is_empty() {
                                self.is_recording = false;
                                self.recording_clips.clear();
                            }
                            ctx.request_repaint();
                        }
                        AudioEvent::AudioFileReady { pool_index, path, channels, sample_rate, duration, format } => {
                            println!("Audio file ready: pool={}, path={}, ch={}, sr={}, {:.1}s, {:?}",
                                     pool_index, path, channels, sample_rate, duration, format);
                            // For PCM (mmap'd) files, raw samples are available immediately
                            // via the pool's data() accessor. Fetch them for GPU waveform.
                            if format == daw_backend::io::AudioFormat::Pcm {
                                if let Some(ref controller_arc) = self.audio_controller {
                                    let mut controller = controller_arc.lock().unwrap();
                                    match controller.get_pool_audio_samples(pool_index) {
                                        Ok((samples, sr, ch)) => {
                                            self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sr, ch));
                                            self.waveform_gpu_dirty.insert(pool_index);
                                        }
                                        Err(e) => eprintln!("Failed to fetch raw audio for pool {}: {}", pool_index, e),
                                    }
                                }
                            }
                            // For compressed files, waveform data arrives progressively
                            // via AudioDecodeProgress events.
                            ctx.request_repaint();
                        }
                        AudioEvent::AudioDecodeProgress { pool_index, samples, sample_rate, channels } => {
                            // Samples arrive as deltas — append to existing cache
                            if let Some(entry) = self.raw_audio_cache.get_mut(&pool_index) {
                                Arc::make_mut(&mut entry.0).extend_from_slice(&samples);
                            } else {
                                self.raw_audio_cache.insert(pool_index, (Arc::new(samples), sample_rate, channels));
                            }
                            self.waveform_gpu_dirty.insert(pool_index);
                            ctx.request_repaint();
                        }
                        AudioEvent::GraphPresetLoaded(track_id, preset_name) => {
                            // Preset was loaded on the audio thread — bump generation
                            // so the node graph pane reloads from backend
                            self.project_generation += 1;
                            // Decrement pending counter (saturating to avoid underflow from
                            // loads not initiated by the preset browser, e.g. default instruments)
                            let _ = self.pending_graph_loads.fetch_update(
                                std::sync::atomic::Ordering::Relaxed,
                                std::sync::atomic::Ordering::Relaxed,
                                |v| if v > 0 { Some(v - 1) } else { Some(0) },
                            );
                            // Auto-rename the MIDI layer to match the preset name, unless
                            // the user has already given it a custom name
                            if let Some(&layer_uuid) = self.track_to_layer_map.get(&track_id) {
                                let doc = self.action_executor.document_mut();
                                if let Some(layer) = doc.get_layer_mut(&layer_uuid) {
                                    use lightningbeam_core::layer::LayerTrait;
                                    if !layer.has_custom_name() {
                                        layer.set_name(preset_name);
                                    }
                                }
                            }
                            ctx.request_repaint();
                        }
                        AudioEvent::InputLevel(peak) => {
                            self.input_level = self.input_level.max(peak);
                        }
                        AudioEvent::OutputLevel(peak_l, peak_r) => {
                            self.output_level.0 = self.output_level.0.max(peak_l);
                            self.output_level.1 = self.output_level.1.max(peak_r);
                        }
                        AudioEvent::TrackLevels(levels) => {
                            for (track_id, peak) in levels {
                                let entry = self.track_levels.entry(track_id).or_insert(0.0);
                                *entry = entry.max(peak);
                            }
                        }
                        _ => {} // Ignore other events for now
                    }
                }

        }

        // Update input monitoring based on active layer (only send command when changed)
        {
            let should_monitor = self.audio_controller.is_some() && self.active_layer_id.map_or(false, |layer_id| {
                let doc = self.action_executor.document();
                if let Some(layer) = doc.get_layer(&layer_id) {
                    matches!(layer, lightningbeam_core::layer::AnyLayer::Audio(a) if a.audio_layer_type == lightningbeam_core::layer::AudioLayerType::Sampled)
                } else {
                    false
                }
            });
            if should_monitor != self.last_input_monitoring {
                self.last_input_monitoring = should_monitor;
                if let Some(controller) = &self.audio_controller {
                    if let Ok(mut ctrl) = controller.try_lock() {
                        ctrl.set_input_monitoring(should_monitor);
                    }
                }
            }
        }

        // Decay VU meter levels (~1.5s full fall at 60fps)
        {
            let decay = 0.97f32;
            self.input_level *= decay;
            self.output_level.0 *= decay;
            self.output_level.1 *= decay;
            for level in self.track_levels.values_mut() {
                *level *= decay;
            }
            // Request repaint while any level is visible
            let any_active = self.input_level > 0.001
                || self.output_level.0 > 0.001 || self.output_level.1 > 0.001
                || self.track_levels.values().any(|&v| v > 0.001);
            if any_active {
                ctx.request_repaint();
            }
        }

        let _post_events_ms = _frame_start.elapsed().as_secs_f64() * 1000.0;

        // Request continuous repaints when playing to update time display
        if self.is_playing {
            ctx.request_repaint();
        }

        // Handle export dialog
        if let Some(export_result) = self.export_dialog.render(ctx) {
            use export::dialog::ExportResult;

            // Create orchestrator if needed
            if self.export_orchestrator.is_none() {
                self.export_orchestrator = Some(export::ExportOrchestrator::new());
            }

            // Clone before the &mut self.export_orchestrator borrow below.
            let shared_device = self.shared_device.clone();

            let export_started = if let Some(orchestrator) = &mut self.export_orchestrator {
                match export_result {
                    ExportResult::Image(settings, output_path) => {
                        println!("🖼 [MAIN] Starting image export: {}", output_path.display());
                        let doc = self.action_executor.document();
                        orchestrator.start_image_export(
                            settings,
                            output_path,
                            doc.width  as u32,
                            doc.height as u32,
                        );
                        false // image export is silent (no progress dialog)
                    }
                    ExportResult::Svg(time, output_path) => {
                        println!("🖋 [MAIN] Exporting SVG: {}", output_path.display());
                        let svg = lightningbeam_core::svg_export::document_to_svg(
                            self.action_executor.document(),
                            time,
                        );
                        if let Err(err) = std::fs::write(&output_path, svg) {
                            eprintln!("❌ Failed to write SVG: {}", err);
                        } else {
                            println!("✅ SVG written: {}", output_path.display());
                        }
                        false // synchronous; no progress dialog
                    }
                    ExportResult::Gif(settings, output_path) => {
                        println!("🎞 [MAIN] Starting GIF export: {}", output_path.display());
                        let doc = self.action_executor.document();
                        orchestrator.start_gif_export(
                            settings,
                            output_path,
                            doc.width  as u32,
                            doc.height as u32,
                        );
                        true // background encode with progress dialog
                    }
                    ExportResult::AudioOnly(settings, output_path) => {
                        println!("🎵 [MAIN] Starting audio-only export: {}", output_path.display());

                        // Remember Artist/Album so they prefill next time.
                        if !settings.metadata.artist.is_empty() {
                            self.config.last_audio_artist = settings.metadata.artist.clone();
                        }
                        if !settings.metadata.album.is_empty() {
                            self.config.last_audio_album = settings.metadata.album.clone();
                        }
                        self.config.save();

                        if let Some(audio_controller) = &self.audio_controller {
                            orchestrator.start_audio_export(
                                settings,
                                output_path,
                                Arc::clone(audio_controller),
                            );
                            true
                        } else {
                            eprintln!("❌ Cannot export audio: Audio controller not available");
                            false
                        }
                    }
                    ExportResult::VideoOnly(settings, output_path) => {
                        println!("🎬 [MAIN] Starting video-only export: {}", output_path.display());

                        match orchestrator.start_video_export(
                            settings,
                            output_path,
                            self.action_executor.document(),
                            Arc::clone(&self.video_manager),
                            self.raster_store.clone(),
                            self.current_file_path.clone(),
                            shared_device.clone(),
                        ) {
                            Ok(()) => true,
                            Err(err) => {
                                eprintln!("❌ Failed to start video export: {}", err);
                                false
                            }
                        }
                    }
                    ExportResult::VideoWithAudio(video_settings, audio_settings, output_path) => {
                        println!("🎬🎵 [MAIN] Starting video+audio export: {}", output_path.display());

                        if let Some(audio_controller) = &self.audio_controller {
                            match orchestrator.start_video_with_audio_export(
                                video_settings,
                                audio_settings,
                                output_path,
                                Arc::clone(audio_controller),
                                self.action_executor.document(),
                                Arc::clone(&self.video_manager),
                                self.raster_store.clone(),
                                self.current_file_path.clone(),
                                shared_device.clone(),
                            ) {
                                Ok(()) => true,
                                Err(err) => {
                                    eprintln!("❌ Failed to start video+audio export: {}", err);
                                    false
                                }
                            }
                        } else {
                            eprintln!("❌ Cannot export with audio: Audio controller not available");
                            false
                        }
                    }
                }
            } else {
                false
            };

            // Open progress dialog if export started successfully
            if export_started {
                self.export_progress_dialog.open();
            }
        }

        // Render export progress dialog and handle cancel
        if self.export_progress_dialog.render(ctx) {
            // User clicked Cancel: stop + tear down the export, then dismiss the dialog.
            if let Some(orchestrator) = &mut self.export_orchestrator {
                orchestrator.cancel();
            }
            self.export_progress_dialog.close();
        }

        // Keep requesting repaints while export progress dialog is open
        if self.export_progress_dialog.open {
            ctx.request_repaint();
        }

        // Render preferences dialog
        let mobile = self.mobile_active();
        if let Some(result) = self.preferences_dialog.render(ctx, &mut self.config, &mut self.theme, mobile) {
            if result.buffer_size_changed {
                println!("⚠️  Audio buffer size will be applied on next app restart");
            }
            // Apply new keybindings if changed
            if let Some(new_keymap) = result.new_keymap {
                self.keymap = new_keymap;
                // Update native menu accelerator labels
                if let Some(menu_system) = &self.menu_system {
                    menu_system.apply_keybindings(&self.keymap);
                }
            }
        }

        // One-time prompt: how to store large media (pack vs reference).
        self.render_large_media_prompt(ctx);

        // Render video frames incrementally (if video export in progress)
        let exporting = self.export_orchestrator.as_ref().map_or(false, |o| o.is_exporting());
        if exporting {
            // Keep the UI loop alive so progress is polled/drained even when the video is
            // produced on a background thread (zero-copy path) that emits no UI-thread frames.
            // Poll at ~6 Hz (not 60): the progress bar doesn't need more, and repainting the full
            // editor every frame steals CPU/GPU from the background render thread, slowing export.
            ctx.request_repaint_after(std::time::Duration::from_millis(160));
            if let Some(render_state) = frame.wgpu_render_state() {
                let device = &render_state.device;
                let queue = &render_state.queue;

                // Build the renderer + image cache ONCE per export (reused every frame).
                // The image cache is given the container path so lazily-paged image assets
                // (Phase 4 Tier 1) decode during export.
                if self.export_renderer.is_none() {
                    self.export_renderer = vello::Renderer::new(
                        device,
                        vello::RendererOptions {
                            use_cpu: false,
                            antialiasing_support: vello::AaSupport::all(),
                            num_init_threads: None,
                            pipeline_cache: None,
                        },
                    ).ok();
                    let mut ic = lightningbeam_core::renderer::ImageCache::new();
                    ic.set_container_path(self.current_file_path.clone());
                    self.export_image_cache = Some(ic);
                }

                if let (Some(renderer), Some(image_cache), Some(orchestrator)) = (
                    self.export_renderer.as_mut(),
                    self.export_image_cache.as_mut(),
                    self.export_orchestrator.as_mut(),
                ) {
                    // Drive incremental video export.
                    if let Ok(has_more) = orchestrator.render_next_video_frame(
                        self.action_executor.document_mut(),
                        device,
                        queue,
                        renderer,
                        image_cache,
                        &self.video_manager,
                        Some(&self.raster_store),
                    ) {
                        if has_more {
                            ctx.request_repaint();
                        }
                    }

                    // Drive incremental GIF export (one frame rendered + streamed per call).
                    match orchestrator.render_next_gif_frame(
                        self.action_executor.document_mut(),
                        device,
                        queue,
                        renderer,
                        image_cache,
                        &self.video_manager,
                        Some(&self.raster_store),
                    ) {
                        Ok(true)  => { ctx.request_repaint(); } // more frames to render
                        Ok(false) => {}                          // done or not a GIF export
                        Err(e)    => { eprintln!("GIF export failed: {e}"); }
                    }

                    // Drive single-frame image export (two-frame async: render then readback).
                    match orchestrator.render_image_frame(
                        self.action_executor.document_mut(),
                        device,
                        queue,
                        renderer,
                        image_cache,
                        &self.video_manager,
                        self.selection.raster_floating.as_ref(),
                        Some(&self.raster_store),
                    ) {
                        Ok(false) => { ctx.request_repaint(); } // readback pending
                        Ok(true)  => {}                          // done or cancelled
                        Err(e)    => { eprintln!("Image export failed: {e}"); }
                    }
                }
            }
        } else if self.export_renderer.is_some() {
            // Export finished — free the renderer + cache.
            self.export_renderer = None;
            self.export_image_cache = None;
        }

        // Poll export orchestrator for progress — only while there's something to report
        // (otherwise this runs every repaint forever, spamming logs and wasting work). The
        // orchestrator clears its state once the terminal Complete/Error is consumed.
        if let Some(orchestrator) = self.export_orchestrator.as_mut().filter(|o| o.has_pending_progress()) {
            // Only log occasionally to avoid spam
            use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};
            static POLL_COUNT: AtomicU32 = AtomicU32::new(0);
            let count = POLL_COUNT.fetch_add(1, AtomicOrdering::Relaxed) + 1;
            if count % 60 == 0 {
                println!("🔍 [MAIN] Polling orchestrator (poll #{})...", count);
            }
            if let Some(progress) = orchestrator.poll_progress() {
                match progress {
                    lightningbeam_core::export::ExportProgress::Started { total_frames } => {
                        println!("Export started: {} frames", total_frames);
                        self.export_progress_dialog.update_progress(
                            "Starting export...".to_string(),
                            0.0,
                        );
                        ctx.request_repaint(); // Keep repainting during export
                    }
                    lightningbeam_core::export::ExportProgress::FrameRendered { frame, total } => {
                        let progress = frame as f32 / total as f32;
                        self.export_progress_dialog.update_progress(
                            format!("Rendering frame {} of {}", frame, total),
                            progress,
                        );
                        ctx.request_repaint();
                    }
                    lightningbeam_core::export::ExportProgress::AudioRendered => {
                        self.export_progress_dialog.update_progress(
                            "Rendering audio...".to_string(),
                            0.5,
                        );
                        ctx.request_repaint();
                    }
                    lightningbeam_core::export::ExportProgress::Finalizing => {
                        self.export_progress_dialog.update_progress(
                            "Finalizing export...".to_string(),
                            0.9,
                        );
                        ctx.request_repaint();
                    }
                    lightningbeam_core::export::ExportProgress::Complete { ref output_path } => {
                        println!("✅ Export complete: {}", output_path.display());
                        self.export_progress_dialog.update_progress(
                            format!("Export complete: {}", output_path.display()),
                            1.0,
                        );
                        // Close the progress dialog after a brief delay
                        self.export_progress_dialog.close();

                        // Send desktop notification (fire-and-forget; never blocks the UI)
                        notifications::notify_export_complete(output_path);
                    }
                    lightningbeam_core::export::ExportProgress::Error { ref message } => {
                        eprintln!("❌ Export error: {}", message);
                        self.export_progress_dialog.update_progress(
                            format!("Error: {}", message),
                            0.0,
                        );
                        // Keep the dialog open to show the error

                        // Send desktop notification for error (fire-and-forget)
                        notifications::notify_export_error(message);
                    }
                }
            }

            // Request repaint while exporting to update progress
            if orchestrator.is_exporting() {
                ctx.request_repaint();
            }
        }

        // Top menu bar (egui-rendered on desktop). On mobile the shell's ⌕ palette + ⋯ overflow
        // expose every command, so the desktop menu bar is suppressed.
        if !self.mobile_active() {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            if let Some(menu_system) = &self.menu_system {
                let recent_files = self.config.get_recent_files();
                let layout_names: Vec<String> = self.layouts.iter().map(|l| l.name.clone()).collect();

                // Determine timeline measures mode for conditional menu items
                let timeline_is_measures = self.pane_instances.values().any(|p| {
                    if let panes::PaneInstance::Timeline(t) = p { t.is_measures_mode() } else { false }
                });

                // Checked actions show "✔ Label"; hidden actions are not rendered at all
                let master_track_shown = self.pane_instances.values().any(|p| {
                    if let panes::PaneInstance::Timeline(t) = p { t.show_master_track } else { false }
                });
                let checked: &[crate::menu::MenuAction] = match (self.count_in_enabled && self.metronome_enabled, master_track_shown) {
                    (true, true) => &[crate::menu::MenuAction::ToggleCountIn, crate::menu::MenuAction::ShowMasterTrack],
                    (true, false) => &[crate::menu::MenuAction::ToggleCountIn],
                    (false, true) => &[crate::menu::MenuAction::ShowMasterTrack],
                    (false, false) => &[],
                };
                let hidden: &[crate::menu::MenuAction] = if timeline_is_measures && self.metronome_enabled {
                    &[]
                } else {
                    &[crate::menu::MenuAction::ToggleCountIn]
                };

                if let Some(action) = menu_system.render_egui_menu_bar(
                    ui, &recent_files, Some(&self.keymap),
                    &layout_names, self.current_layout_index,
                    checked, hidden,
                ) {
                    self.handle_menu_action(action);
                }
            }
        });
        }

        // Render start screen or editor based on app mode
        if self.app_mode == AppMode::StartScreen {
            if self.mobile_active() {
                mobile::intent::render(self, ctx);
            } else {
                self.render_start_screen(ctx);
            }
            return; // Skip editor rendering
        }

        // Skip rendering the editor while a file is loading — the loading dialog
        // (rendered earlier) is all the user needs to see. This avoids showing
        // the default empty layout behind the dialog for several seconds.
        if matches!(self.file_operation, Some(FileOperation::Loading { .. })) {
            egui::CentralPanel::default().show(ctx, |_ui| {});
            return;
        }

        // Test mode sidebar (debug builds only) — must be before CentralPanel
        #[cfg(debug_assertions)]
        let test_mode_replay = test_mode::render_sidebar(ctx, &mut self.test_mode);
        // Apply tool changes from replay
        #[cfg(debug_assertions)]
        if let Some(ref tool_name) = test_mode_replay.tool_change {
            if let Some(tool) = test_mode::parse_tool(tool_name) {
                self.selected_tool = tool;
            }
        }

        // Main pane area (editor mode)
        let mut layout_action: Option<LayoutAction> = None;
        // Per-frame scratch consumed by the post-render drain (see FrameScratch).
        // Declared outside the closure because some of it (clipboard_consumed) is read
        // after the panel closes.
        let mut scratch = FrameScratch::default();
        #[cfg(debug_assertions)]
        {
            scratch.synthetic_input = test_mode_replay.synthetic_input;
        }
        // On mobile the shell is full-bleed to the window edges (it paints its own background), so
        // drop the central panel's default inner margin — otherwise every pane is inset by it while
        // the unclipped keyboard overdraws into it, leaving an inconsistent gutter.
        let central = egui::CentralPanel::default();
        let central = if self.mobile_active() {
            central.frame(egui::Frame::default())
        } else {
            central
        };
        central.show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();

            // Reset hovered divider each frame
            self.hovered_divider = None;

            // Sync clip instance transforms from animation data at current playback time.
            // This ensures selection boxes, hit testing, and interactive editing see the
            // animated transform values, not just the base values on the ClipInstance struct.
            {
                let time = self.playback_time;
                let document = self.action_executor.document_mut();
                // Keep document.current_time synced from the audio playback position every frame.
                // The renderer reads current_time for animation eval + video-frame decode, so this
                // must not depend on any particular pane (e.g. the timeline) being rendered —
                // otherwise the Stage freezes during playback when that pane is off-screen.
                document.current_time = time;
                // Bake animation transforms for root layers
                for layer in document.root.children.iter_mut() {
                    if let lightningbeam_core::layer::AnyLayer::Vector(vl) = layer {
                        for ci in &mut vl.clip_instances {
                            let (t, opacity) = vl.layer.animation_data.eval_clip_instance_transform(
                                ci.id, time, &ci.transform, ci.opacity,
                            );
                            ci.transform = t;
                            ci.opacity = opacity;
                        }
                    }
                }
                // Bake animation transforms for layers inside movie clips
                for clip in document.vector_clips.values_mut() {
                    for layer_node in clip.layers.roots.iter_mut() {
                        if let lightningbeam_core::layer::AnyLayer::Vector(vl) = &mut layer_node.data {
                            for ci in &mut vl.clip_instances {
                                let (t, opacity) = vl.layer.animation_data.eval_clip_instance_transform(
                                    ci.id, time, &ci.transform, ci.opacity,
                                );
                                ci.transform = t;
                                ci.opacity = opacity;
                            }
                        }
                    }
                }
            }

            // Build the per-frame context (single source of truth for SharedPaneState,
            // shared by the desktop and mobile paths). The bundle borrows `self` + `scratch`;
            // it is dropped before the post-render drain below re-touches them.
            let is_mobile = self.mobile_active();
            let mut bundle = self.build_frame(&mut scratch);

            if is_mobile {
                // Phone shell: a single hero surface + top tabs + resizable timeline ribbon
                // + a fixed transport floor, reusing the same panes the desktop layout uses.
                mobile::render_mobile_shell(
                    ui,
                    available_rect,
                    &mut bundle.rc,
                    bundle.mobile_state,
                );

                // Drive recording from the application, not the Timeline pane's render pass, so a
                // REC request (from the music pane header) and count-in progression work regardless
                // of whether the Timeline is currently visible. The Timeline pane still owns the
                // recording *state*; we just tick it here each frame.
                let rc = &mut bundle.rc;
                for pane in rc.pane_instances.values_mut() {
                    if let PaneInstance::Timeline(t) = pane {
                        t.check_pending_recording_start(&mut rc.shared);
                        if *rc.shared.pending_record_toggle {
                            *rc.shared.pending_record_toggle = false;
                            t.toggle_recording(&mut rc.shared);
                        }
                        // Stopping playback stops recording (stop_recording clears is_recording,
                        // so this fires once).
                        if *rc.shared.is_recording && !*rc.shared.is_playing {
                            t.stop_recording(&mut rc.shared);
                        }
                        break;
                    }
                }
            } else {
                render_layout_node(
                    ui,
                    bundle.current_layout,
                    available_rect,
                    bundle.drag_state,
                    bundle.hovered_divider,
                    bundle.selected_pane,
                    &mut layout_action,
                    bundle.split_preview_mode,
                    &Vec::new(), // Root path
                    &mut bundle.rc,
                );
            }
            drop(bundle);

            // Process collected effect thumbnail requests
            if !scratch.effect_thumbnail_requests.is_empty() {
                if let Some(generator) = &mut self.effect_thumbnail_generator {
                    generator.request_thumbnails(&scratch.effect_thumbnail_requests);
                }
            }


            // Execute action on the best handler (two-phase dispatch)
            if let Some(action) = &self.pending_view_action {
                if let Some(best_handler) = scratch.pending_handlers.iter().min_by_key(|h| h.priority) {
                    // Look up the pane instance and execute the action
                    if let Some(pane_instance) = self.pane_instances.get_mut(&best_handler.pane_path) {
                        match pane_instance {
                            panes::PaneInstance::Stage(stage_pane) => {
                                stage_pane.execute_view_action(action, best_handler.zoom_center);
                            }
                            _ => {} // Other pane types don't handle view actions yet
                        }
                    }
                }
                // Clear the pending action after execution
                self.pending_view_action = None;
            }

            // Sync any new audio layers created during this frame to the backend
            // This handles layers created directly (e.g., auto-created audio tracks for video+audio)
            // Must happen BEFORE executing actions so the layer-to-track mapping is available
            self.sync_audio_layers_to_backend();

            // Execute all pending actions (two-phase dispatch)
            for action in std::mem::take(&mut scratch.pending_actions) {
                // Record action for test mode (debug builds only)
                #[cfg(debug_assertions)]
                let action_desc = action.description();

                // Create backend context for actions that need backend sync
                if let Some(ref controller_arc) = self.audio_controller {
                    let mut controller = controller_arc.lock().unwrap();
                    let mut backend_context = lightningbeam_core::action::BackendContext {
                        audio_controller: Some(&mut *controller),
                        layer_to_track_map: &self.layer_to_track_map,
                        clip_instance_to_backend_map: &mut self.clip_instance_to_backend_map,
                    };

                    // Execute action with backend synchronization
                    if let Err(e) = self.action_executor.execute_with_backend(action, &mut backend_context) {
                        eprintln!("Action execution failed: {}", e);
                    }
                } else {
                    // No audio system available, execute without backend
                    let _ = self.action_executor.execute(action);
                }

                #[cfg(debug_assertions)]
                self.test_mode.record_event(
                    lightningbeam_core::test_mode::TestEventKind::ActionExecuted {
                        description: action_desc,
                    },
                );
            }

            // Process menu actions queued by pane context menus
            for action in std::mem::take(&mut scratch.pending_menu_actions) {
                self.handle_menu_action(action);
            }

            // Process webcam recording commands from timeline
            if let Some(cmd) = self.webcam_record_command.take() {
                match cmd {
                    panes::WebcamRecordCommand::Start { .. } => {
                        // Ensure webcam is open
                        if self.webcam.is_none() {
                            if let Some(device) = lightningbeam_core::webcam::default_camera() {
                                match lightningbeam_core::webcam::WebcamCapture::open(&device) {
                                    Ok(cam) => {
                                        eprintln!("[WEBCAM] Opened camera for recording: {}", device.name);
                                        self.webcam = Some(cam);
                                    }
                                    Err(e) => {
                                        eprintln!("[WEBCAM] Failed to open camera for recording: {}", e);
                                    }
                                }
                            }
                        }
                        if let Some(webcam) = &mut self.webcam {
                            // Generate output path in project directory or temp
                            let recording_dir = if let Some(ref file_path) = self.current_file_path {
                                file_path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf()
                            } else {
                                std::env::temp_dir()
                            };
                            let timestamp = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            let codec = lightningbeam_core::webcam::RecordingCodec::H264; // TODO: read from preferences
                            let ext = match codec {
                                lightningbeam_core::webcam::RecordingCodec::H264 => "mp4",
                                lightningbeam_core::webcam::RecordingCodec::Lossless => "mkv",
                            };
                            let recording_path = recording_dir.join(format!("webcam_recording_{}.{}", timestamp, ext));
                            match webcam.start_recording(recording_path, codec) {
                                Ok(()) => {
                                    eprintln!("[WEBCAM] Recording started");
                                }
                                Err(e) => {
                                    eprintln!("[WEBCAM] Failed to start recording: {}", e);
                                }
                            }
                        }
                    }
                    panes::WebcamRecordCommand::Stop => {
                        eprintln!("[STOP] Webcam stop command processed (main.rs handler)");
                        // Find the webcam recording layer before stopping (need it for cleanup)
                        let webcam_layer_id = {
                            let document = self.action_executor.document();
                            self.recording_layer_ids.iter().copied().find(|lid| {
                                document.get_layer(lid).map_or(false, |l| {
                                    matches!(l, lightningbeam_core::layer::AnyLayer::Video(v) if v.camera_enabled)
                                })
                            })
                        };
                        if let Some(webcam) = &mut self.webcam {
                            let stop_t = std::time::Instant::now();
                            match webcam.stop_recording() {
                                Ok(result) => {
                                    eprintln!("[STOP] webcam.stop_recording() returned in {:.1}ms", stop_t.elapsed().as_secs_f64() * 1000.0);
                                    let file_path_str = result.file_path.to_string_lossy().to_string();
                                    eprintln!("[WEBCAM] Recording saved to: {} (recorder duration={:.4}s)", file_path_str, result.duration);
                                    // Create VideoClip + ClipInstance from recorded file
                                    if let Some(layer_id) = webcam_layer_id {
                                        match lightningbeam_core::video::probe_video(&lightningbeam_core::video::VideoSource::Path(file_path_str.clone())) {
                                            Ok(info) => {
                                                use lightningbeam_core::clip::{VideoClip, ClipInstance};
                                                let clip = VideoClip {
                                                    id: Uuid::new_v4(),
                                                    name: result.file_path.file_name()
                                                        .and_then(|n| n.to_str())
                                                        .unwrap_or("Webcam Recording")
                                                        .to_string(),
                                                    file_path: file_path_str.clone(),
                                                    width: info.width as f64,
                                                    height: info.height as f64,
                                                    duration: info.duration,
                                                    frame_rate: info.fps,
                                                    linked_audio_clip_id: None,
                                                    media_id: None,
                                                    folder_id: None,
                                                };
                                                let clip_id = clip.id;
                                                let duration = clip.duration;
                                                self.action_executor.document_mut().video_clips.insert(clip_id, clip);

                                                // recording_start_time and duration are seconds; convert to beats.
                                                let tempo_map = self.action_executor.document().tempo_map();
                                                let rec_start_beats = tempo_map.seconds_to_beats(Seconds(self.recording_start_time));
                                                let dur_beats = tempo_map.seconds_to_beats(tempo_map.beats_to_seconds(rec_start_beats) + Seconds(duration)) - rec_start_beats;
                                                let mut clip_instance = ClipInstance::new(clip_id)
                                                    .with_timeline_start(rec_start_beats)
                                                    .with_timeline_duration(dur_beats);

                                                // Scale to fit document and center (like drag-dropped videos)
                                                {
                                                    let doc = self.action_executor.document();
                                                    let video_width = info.width as f64;
                                                    let video_height = info.height as f64;
                                                    let scale_x = doc.width / video_width;
                                                    let scale_y = doc.height / video_height;
                                                    let uniform_scale = scale_x.min(scale_y);
                                                    clip_instance.transform.scale_x = uniform_scale;
                                                    clip_instance.transform.scale_y = uniform_scale;
                                                    let scaled_w = video_width * uniform_scale;
                                                    let scaled_h = video_height * uniform_scale;
                                                    clip_instance.transform.x = (doc.width - scaled_w) / 2.0;
                                                    clip_instance.transform.y = (doc.height - scaled_h) / 2.0;
                                                }

                                                if let Some(layer) = self.action_executor.document_mut().get_layer_mut(&layer_id) {
                                                    if let lightningbeam_core::layer::AnyLayer::Video(video_layer) = layer {
                                                        video_layer.clip_instances.push(clip_instance);
                                                    }
                                                }

                                                // Load into video manager for playback
                                                // Use the video's native dimensions so decoded frames
                                                // match the VideoClip width/height the renderer uses
                                                // for the display rect.
                                                {
                                                    let mut vm = self.video_manager.lock().unwrap();
                                                    if let Err(e) = vm.load_video(clip_id, lightningbeam_core::video::VideoSource::Path(file_path_str.clone()), info.width, info.height) {
                                                        eprintln!("[WEBCAM] Failed to load recorded video: {}", e);
                                                    }
                                                }

                                                // Build keyframe index + thumbnails in background.
                                                self.spawn_keyframe_index(clip_id);
                                                self.spawn_thumbnail_generation(clip_id);

                                                eprintln!(
                                                    "[WEBCAM] probe_video: duration={:.4}s, fps={:.1}, {}x{}. Using probe duration for clip.",
                                                    info.duration, info.fps, info.width, info.height,
                                                );
                                            }
                                            Err(e) => {
                                                eprintln!("[WEBCAM] Failed to probe recorded video: {}", e);
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[WEBCAM] Failed to stop recording: {}", e);
                                    // webcam layer cleanup handled by recording_layer_ids.clear() below
                                }
                            }
                        }
                        // Remove webcam layer from active recordings
                        if let Some(wid) = webcam_layer_id {
                            self.recording_layer_ids.retain(|id| *id != wid);
                        }
                        if self.recording_layer_ids.is_empty() {
                            self.is_recording = false;
                            self.recording_clips.clear();
                        }
                    }
                }
            }

            // Process editing context navigation (enter/exit movie clips)
            if let Some((clip_id, instance_id, parent_layer_id)) = scratch.pending_enter_clip {
                let entry = EditingContextEntry {
                    clip_id,
                    instance_id,
                    parent_layer_id,
                    saved_playback_time: self.playback_time,
                    saved_active_layer_id: self.active_layer_id,
                };
                self.editing_context.push(entry);
                self.selection.clear();
                // Set active layer to the clip's first layer
                let first_layer_id = self.action_executor.document()
                    .get_vector_clip(&clip_id)
                    .and_then(|clip| clip.layers.roots.first())
                    .map(|node| node.data.id());
                self.active_layer_id = first_layer_id;
                // Reset playback time to 0 when entering a clip
                self.playback_time = 0.0;
            }
            if self.commit_raster_floating_if_any {
                self.commit_raster_floating_if_any = false;
                self.commit_raster_floating();
            }

            if scratch.pending_exit_clip {
                if let Some(entry) = self.editing_context.pop() {
                    self.selection.clear();
                    self.active_layer_id = entry.saved_active_layer_id;
                    self.playback_time = entry.saved_playback_time;
                }
            }

            // Breadcrumb jump: exit up to a specific depth (restoring that level's saved context).
            if let Some(depth) = scratch.pending_exit_to_depth.take() {
                if let Some(entry) = self.editing_context.exit_to_depth(depth) {
                    self.selection.clear();
                    self.active_layer_id = entry.saved_active_layer_id;
                    self.playback_time = entry.saved_playback_time;
                }
            }

            // Set cursor based on hover state
            if let Some((_, is_horizontal)) = self.hovered_divider {
                if is_horizontal {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                } else {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }
            }
        });

        // Handle ESC key and click-outside to cancel split preview
        if let SplitPreviewMode::Active { hovered_pane, .. } = &self.split_preview_mode {
            let should_cancel = ctx.input(|i| {
                // Cancel on ESC key
                if i.key_pressed(egui::Key::Escape) {
                    return true;
                }
                // Cancel on click outside any pane
                if i.pointer.primary_clicked() && hovered_pane.is_none() {
                    return true;
                }
                false
            });

            if should_cancel {
                self.split_preview_mode = SplitPreviewMode::None;
            }
        }

        // Apply layout action after rendering to avoid borrow issues
        if let Some(action) = layout_action {
            self.apply_layout_action(action);
        }

        // Check keyboard shortcuts AFTER UI is rendered
        // This ensures text fields have had a chance to claim focus first
        let wants_keyboard = ctx.wants_keyboard_input();

        // Space bar toggles play/pause (only when no text input is focused)
        if !wants_keyboard && ctx.input(|i| self.keymap.action_pressed(keymap::AppAction::TogglePlayPause, i)) {
            self.is_playing = !self.is_playing;
            if let Some(ref controller_arc) = self.audio_controller {
                let mut controller = controller_arc.lock().unwrap();
                if self.is_playing {
                    controller.play();
                } else {
                    controller.pause();
                }
            }
        }

        ctx.input(|i| {
            // Handle clipboard events (Ctrl+C/X/V) — winit converts these to
            // Event::Copy/Cut/Paste instead of regular key events, so
            // check_shortcuts won't see them via key_pressed().
            // Skip if a pane (e.g. piano roll) already handled the clipboard event.
            let mut clipboard_handled = scratch.clipboard_consumed;
            if !scratch.clipboard_consumed {
                for event in &i.events {
                    match event {
                        egui::Event::Copy => {
                            self.handle_menu_action(MenuAction::Copy);
                            clipboard_handled = true;
                        }
                        egui::Event::Cut => {
                            self.handle_menu_action(MenuAction::Cut);
                            clipboard_handled = true;
                        }
                        egui::Event::Paste(_) => {
                            self.handle_menu_action(MenuAction::Paste);
                            clipboard_handled = true;
                        }
                        // When text/plain is absent from the system clipboard egui-winit
                        // falls through to a Key event instead of Event::Paste.
                        egui::Event::Key {
                            key: egui::Key::V,
                            pressed: true,
                            modifiers,
                            ..
                        } if modifiers.ctrl || modifiers.command => {
                            self.handle_menu_action(MenuAction::Paste);
                            clipboard_handled = true;
                        }
                        _ => {}
                    }
                }
            }

            // Check menu shortcuts that use modifiers (Cmd+S, etc.) - allow even when typing
            // But skip shortcuts without modifiers when keyboard input is claimed (e.g., virtual piano)
            // Also skip clipboard actions (Copy/Cut/Paste) if already handled above to prevent
            // double-firing when egui emits both Event::Key{V} and key_pressed(V) is true.
            if let Some(action) = MenuSystem::check_shortcuts(i, Some(&self.keymap)) {
                let is_clipboard = matches!(action, MenuAction::Copy | MenuAction::Cut | MenuAction::Paste);
                // Only trigger if keyboard isn't claimed OR the shortcut uses modifiers
                if !wants_keyboard || i.modifiers.ctrl || i.modifiers.command || i.modifiers.alt || i.modifiers.shift {
                    if !(is_clipboard && clipboard_handled) {
                        self.handle_menu_action(action);
                    }
                }
            }

            // Check tool shortcuts (only if no text input is focused;
            // modifier guard is encoded in the bindings themselves — default tool bindings have no modifiers)
            if !wants_keyboard {
                use lightningbeam_core::tool::Tool;
                use crate::keymap::AppAction;

                let tool_map: &[(AppAction, Tool)] = &[
                    (AppAction::ToolSelect, Tool::Select),
                    (AppAction::ToolDraw, Tool::Draw),
                    (AppAction::ToolTransform, Tool::Transform),
                    (AppAction::ToolRectangle, Tool::Rectangle),
                    (AppAction::ToolEllipse, Tool::Ellipse),
                    (AppAction::ToolPaintBucket, Tool::PaintBucket),
                    (AppAction::ToolEyedropper, Tool::Eyedropper),
                    (AppAction::ToolLine, Tool::Line),
                    (AppAction::ToolPolygon, Tool::Polygon),
                    (AppAction::ToolBezierEdit, Tool::BezierEdit),
                    (AppAction::ToolText, Tool::Text),
                    (AppAction::ToolRegionSelect, Tool::RegionSelect),
                    (AppAction::ToolErase, Tool::Erase),
                    (AppAction::ToolSmudge, Tool::Smudge),
                    (AppAction::ToolSelectLasso, Tool::SelectLasso),
                    (AppAction::ToolSplit, Tool::Split),
                ];
                for &(action, tool) in tool_map {
                    if self.keymap.action_pressed(action, i) {
                        self.selected_tool = tool;
                        break;
                    }
                }
            }
        });

        // Record tool changes for test mode (debug builds only)
        #[cfg(debug_assertions)]
        {
            // Use a simple static to track previous tool for change detection
            use std::sync::atomic::{AtomicU8, Ordering};
            static PREV_TOOL: AtomicU8 = AtomicU8::new(255);
            let tool_byte = self.selected_tool as u8;
            let prev = PREV_TOOL.swap(tool_byte, Ordering::Relaxed);
            if prev != tool_byte && prev != 255 {
                self.test_mode.record_event(
                    lightningbeam_core::test_mode::TestEventKind::ToolChanged {
                        tool: format!("{:?}", self.selected_tool),
                    },
                );
            }
        }

        // Escape key: cancel floating raster selection, or clear the geometry selection
        if !wants_keyboard && ctx.input(|i| self.keymap.action_pressed(keymap::AppAction::CancelAction, i)) {
            if self.selection.raster_floating.is_some() {
                self.cancel_raster_floating();
            } else if self.selection.raster_selection.is_some() {
                self.selection.raster_selection = None;
            } else if self.selection.has_geometry_selection() {
                self.selection.clear_geometry_selection();
            }
        }

        // After all input is processed, if a region selection was deselected without any
        // intervening edit, heal (undo) its cut so the lasso is non-destructive.
        self.resolve_pending_region_cut();

        // F3 debug overlay toggle (works even when text input is active)
        if ctx.input(|i| self.keymap.action_pressed(keymap::AppAction::ToggleDebugOverlay, i)) {
            self.debug_overlay_visible = !self.debug_overlay_visible;
        }

        // F5 test mode toggle (debug builds only)
        #[cfg(debug_assertions)]
        if ctx.input(|i| self.keymap.action_pressed(keymap::AppAction::ToggleTestMode, i)) {
            self.test_mode.active = !self.test_mode.active;
            if self.test_mode.active {
                self.test_mode.refresh_test_list();
            }
        }

        // Clear the set of audio pools with new waveforms at the end of the frame
        // (Thumbnails have been invalidated above, so this can be cleared for next frame)
        if !self.audio_pools_with_new_waveforms.is_empty() {
            println!("🧹 [UPDATE] Clearing waveform update set: {:?}", self.audio_pools_with_new_waveforms);
        }
        self.audio_pools_with_new_waveforms.clear();

        // Render F3 debug overlay on top of everything
        if self.debug_overlay_visible {
            let stats = self.debug_stats_collector.collect(
                ctx,
                &self.gpu_info,
                self.audio_controller.as_ref(),
            );
            debug_overlay::render_debug_overlay(ctx, &stats);

        }

        // Render custom cursor overlay (on top of everything including debug overlay)
        custom_cursor::render_overlay(ctx, &mut self.cursor_cache);

        let frame_ms = _frame_start.elapsed().as_secs_f64() * 1000.0;
        if frame_ms > 50.0 {
            eprintln!("[TIMING] SLOW FRAME: {:.1}ms (pre-events={:.1}, events={:.1}, post-events={:.1})",
                frame_ms, _pre_events_ms, _post_events_ms - _pre_events_ms, frame_ms - _post_events_ms);
        }
    }

}

/// Context for rendering operations - bundles all mutable state needed during rendering
/// Wraps SharedPaneState + pane_instances for layout rendering.
/// pane_instances is kept separate from SharedPaneState so we can borrow
/// a specific pane instance mutably while passing the rest as &mut SharedPaneState.
struct RenderContext<'a> {
    shared: panes::SharedPaneState<'a>,
    pane_instances: &'a mut HashMap<NodePath, PaneInstance>,
}

/// Per-frame scratch state used to build the `RenderContext`. These values live only
/// for the duration of one `update()` frame: panes write into them during rendering
/// and the post-render drain consumes them. Grouping them lets [`EditorApp::build_frame`]
/// hand back a context that borrows both `self` and this scratch, so the desktop and
/// mobile render paths share one construction site for the ~80-field `SharedPaneState`.
#[derive(Default)]
struct FrameScratch {
    fallback_pane_priority: Option<u32>,
    pending_handlers: Vec<panes::ViewActionHandler>,
    pending_enter_clip: Option<(Uuid, Uuid, Uuid)>,
    pending_exit_clip: bool,
    pending_exit_to_depth: Option<usize>,
    pending_actions: Vec<Box<dyn lightningbeam_core::action::Action>>,
    pending_menu_actions: Vec<MenuAction>,
    effect_thumbnail_requests: Vec<Uuid>,
    clipboard_consumed: bool,
    empty_thumbnail_cache: HashMap<Uuid, Vec<u8>>,
    #[cfg(debug_assertions)]
    synthetic_input: Option<test_mode::SyntheticInput>,
}

/// Everything `update()` hands to a per-frame layout renderer: the [`RenderContext`]
/// plus the layout-editing borrows that ride alongside it (these are passed to
/// `render_layout_node` as separate `&mut` args and are disjoint from `SharedPaneState`).
/// Built by [`EditorApp::build_frame`] from a single `&mut self`, which is what lets the
/// huge `SharedPaneState` literal have one home shared by the desktop and mobile paths.
struct FrameBundle<'a> {
    rc: RenderContext<'a>,
    current_layout: &'a mut LayoutNode,
    drag_state: &'a mut DragState,
    hovered_divider: &'a mut Option<(NodePath, bool)>,
    selected_pane: &'a mut Option<NodePath>,
    split_preview_mode: &'a mut SplitPreviewMode,
    mobile_state: &'a mut mobile::MobileState,
}

impl EditorApp {
    /// Whether the mobile shell is active this frame (runtime override wins over the env flag).
    fn mobile_active(&self) -> bool {
        self.mobile_ui_override.unwrap_or(self.mobile_ui)
    }

    /// Build the per-frame [`FrameBundle`]. The returned bundle borrows `self` and
    /// `scratch` for `'a`; it must be dropped before the post-render drain touches
    /// `self`/`scratch` again. This is the single source of truth for `SharedPaneState`.
    fn build_frame<'a>(&'a mut self, scratch: &'a mut FrameScratch) -> FrameBundle<'a> {
        let is_mobile = self.mobile_active();
        FrameBundle {
            rc: RenderContext {
                shared: panes::SharedPaneState {
                    is_mobile,
                    is_portrait: true, // set by the mobile shell from the available rect
                    keyboard_octave: &mut self.keyboard_octave,
                    keyboard_pan_x: &mut self.keyboard_pan_x,
                    instrument_show_roll: false,
                    open_instrument_browser: &mut self.open_instrument_browser,
                    pending_record_toggle: &mut self.pending_record_toggle,
                    mobile_context_menu: &mut self.mobile_context_menu,
                    container_path: self.current_file_path.clone(),
                    onion: {
                        // Onion skinning is disabled during playback.
                        let mut o = self.onion_skin;
                        o.enabled = o.enabled && !self.is_playing;
                        o
                    },
                    onion_skin: &mut self.onion_skin,
                    tool_icon_cache: &mut self.tool_icon_cache,
                    icon_cache: &mut self.icon_cache,
                    selected_tool: &mut self.selected_tool,
                    fill_color: &mut self.fill_color,
                    stroke_color: &mut self.stroke_color,
                    active_color_mode: &mut self.active_color_mode,
                    pending_view_action: &mut self.pending_view_action,
                    fallback_pane_priority: &mut scratch.fallback_pane_priority,
                    pending_handlers: &mut scratch.pending_handlers,
                    theme: &self.theme,
                    action_executor: &mut self.action_executor,
                    selection: &mut self.selection,
                    focus: &mut self.focus,
                    editing_clip_id: self.editing_context.current_clip_id(),
                    editing_instance_id: self.editing_context.current_instance_id(),
                    editing_parent_layer_id: self.editing_context.current_parent_layer_id(),
                    editing_clip_path: self.editing_context.clip_path(),
                    pending_enter_clip: &mut scratch.pending_enter_clip,
                    pending_exit_clip: &mut scratch.pending_exit_clip,
                    pending_exit_to_depth: &mut scratch.pending_exit_to_depth,
                    active_layer_id: &mut self.active_layer_id,
                    tool_state: &mut self.tool_state,
                    pending_actions: &mut scratch.pending_actions,
                    draw_simplify_mode: &mut self.draw_simplify_mode,
                    rdp_tolerance: &mut self.rdp_tolerance,
                    schneider_max_error: &mut self.schneider_max_error,
                    raster_settings: &mut self.raster_settings,
                    audio_controller: self.audio_controller.as_ref(),
                    clip_snapshot: self.audio_controller.as_ref().map(|arc| {
                        arc.lock().unwrap().clip_snapshot()
                    }),
                    audio_input_opener: &mut self.audio_input,
                    audio_input_stream: &mut self.audio_input_stream,
                    audio_buffer_size: self.audio_buffer_size,
                    video_manager: &self.video_manager,
                    playback_time: &mut self.playback_time,
                    is_playing: &mut self.is_playing,
                    is_recording: &mut self.is_recording,
                    metronome_enabled: &mut self.metronome_enabled,
                    count_in_enabled: &mut self.count_in_enabled,
                    recording_clips: &mut self.recording_clips,
                    recording_start_time: &mut self.recording_start_time,
                    recording_layer_ids: &mut self.recording_layer_ids,
                    dragging_asset: &mut self.dragging_asset,
                    stroke_width: &mut self.stroke_width,
                    fill_enabled: &mut self.fill_enabled,
                    snap_enabled: &mut self.snap_enabled,
                    paint_bucket_gap_tolerance: &mut self.paint_bucket_gap_tolerance,
                    polygon_sides: &mut self.polygon_sides,
                    layer_to_track_map: &self.layer_to_track_map,
                    clip_instance_to_backend_map: &self.clip_instance_to_backend_map,
                    midi_event_cache: &mut self.midi_event_cache,
                    audio_pools_with_new_waveforms: &self.audio_pools_with_new_waveforms,
                    raw_audio_cache: &self.raw_audio_cache,
                    waveform_gpu_dirty: &mut self.waveform_gpu_dirty,
                    waveform_minmax_pools: &self.waveform_minmax_pools,
                    raster_fault_requests: &self.raster_fault_requests,
                    effect_to_load: &mut self.effect_to_load,
                    effect_thumbnail_requests: &mut scratch.effect_thumbnail_requests,
                    effect_thumbnail_cache: self.effect_thumbnail_generator.as_ref()
                        .map(|g| g.thumbnail_cache())
                        .unwrap_or(&scratch.empty_thumbnail_cache),
                    effect_thumbnails_to_invalidate: &mut self.effect_thumbnails_to_invalidate,
                    webcam_frame: self.webcam_frame.clone(),
                    webcam_record_command: &mut self.webcam_record_command,
                    target_format: self.target_format,
                    pending_menu_actions: &mut scratch.pending_menu_actions,
                    clipboard_manager: &mut self.clipboard_manager,
                    input_level: self.input_level,
                    output_level: self.output_level,
                    track_levels: &self.track_levels,
                    track_to_layer_map: &self.track_to_layer_map,
                    waveform_stereo: self.config.waveform_stereo,
                    project_generation: &mut self.project_generation,
                    graph_topology_generation: &mut self.graph_topology_generation,
                    script_to_edit: &mut self.script_to_edit,
                    script_saved: &mut self.script_saved,
                    pending_region_cut_base: &mut self.pending_region_cut_base,
                    region_select_mode: &mut self.region_select_mode,
                    lasso_mode: &mut self.lasso_mode,
                    pending_graph_loads: &self.pending_graph_loads,
                    clipboard_consumed: &mut scratch.clipboard_consumed,
                    keymap: &self.keymap,
                    pending_node_group: &mut self.pending_node_group,
                    pending_node_ungroup: &mut self.pending_node_ungroup,
                    #[cfg(debug_assertions)]
                    test_mode: &mut self.test_mode,
                    #[cfg(debug_assertions)]
                    synthetic_input: &mut scratch.synthetic_input,
                    brush_preview_pixels: &self.brush_preview_pixels,
                },
                pane_instances: &mut self.pane_instances,
            },
            current_layout: &mut self.current_layout,
            drag_state: &mut self.drag_state,
            hovered_divider: &mut self.hovered_divider,
            selected_pane: &mut self.selected_pane,
            split_preview_mode: &mut self.split_preview_mode,
            mobile_state: &mut self.mobile_state,
        }
    }
}

/// Find which GroupLayer (if any) contains the given layer as a direct child.
/// Returns None if the layer is at document root level.
fn find_parent_group_id(doc: &lightningbeam_core::document::Document, layer_id: &uuid::Uuid) -> Option<uuid::Uuid> {
    fn search_children(children: &[lightningbeam_core::layer::AnyLayer], target: &uuid::Uuid) -> Option<uuid::Uuid> {
        for child in children {
            if let lightningbeam_core::layer::AnyLayer::Group(g) = child {
                // Check if target is a direct child of this group
                if g.children.iter().any(|c| c.id() == *target) {
                    return Some(g.layer.id);
                }
                // Recurse into nested groups
                if let Some(found) = search_children(&g.children, target) {
                    return Some(found);
                }
            }
        }
        None
    }
    search_children(&doc.root.children, layer_id)
}

/// Recursively render a layout node with drag support
fn render_layout_node(
    ui: &mut egui::Ui,
    node: &mut LayoutNode,
    rect: egui::Rect,
    drag_state: &mut DragState,
    hovered_divider: &mut Option<(NodePath, bool)>,
    selected_pane: &mut Option<NodePath>,
    layout_action: &mut Option<LayoutAction>,
    split_preview_mode: &mut SplitPreviewMode,
    path: &NodePath,
    ctx: &mut RenderContext,
) {
    match node {
        LayoutNode::Pane { name } => {
            render_pane(ui, name, rect, selected_pane, layout_action, split_preview_mode, path, ctx);
        }
        LayoutNode::HorizontalGrid { percent, children } => {
            // Handle dragging
            if drag_state.is_dragging && drag_state.node_path == *path {
                if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
                    // Calculate new percentage based on pointer position
                    let new_percent = ((pointer_pos.x - rect.left()) / rect.width() * 100.0)
                        .clamp(10.0, 90.0); // Clamp to prevent too small panes
                    *percent = new_percent;
                }
            }

            // Split horizontally (left | right)
            let split_x = rect.left() + (rect.width() * *percent / 100.0);

            let left_rect = egui::Rect::from_min_max(rect.min, egui::pos2(split_x, rect.max.y));

            let right_rect =
                egui::Rect::from_min_max(egui::pos2(split_x, rect.min.y), rect.max);

            // Render children
            let mut left_path = path.clone();
            left_path.push(0);
            render_layout_node(
                ui,
                &mut children[0],
                left_rect,
                drag_state,
                hovered_divider,
                selected_pane,
                layout_action,
                split_preview_mode,
                &left_path,
                ctx,
            );

            let mut right_path = path.clone();
            right_path.push(1);
            render_layout_node(
                ui,
                &mut children[1],
                right_rect,
                drag_state,
                hovered_divider,
                selected_pane,
                layout_action,
                split_preview_mode,
                &right_path,
                ctx,
            );

            // Draw divider with interaction
            let divider_width = 8.0;
            let divider_rect = egui::Rect::from_min_max(
                egui::pos2(split_x - divider_width / 2.0, rect.min.y),
                egui::pos2(split_x + divider_width / 2.0, rect.max.y),
            );

            let divider_id = ui.id().with(("divider", path));
            let response = ui.interact(divider_rect, divider_id, egui::Sense::click_and_drag());

            // Check if pointer is over divider
            if response.hovered() {
                *hovered_divider = Some((path.clone(), true));
            }

            // Handle drag start
            if response.drag_started() {
                drag_state.is_dragging = true;
                drag_state.node_path = path.clone();
                drag_state.is_horizontal = true;
            }

            // Handle drag end
            if response.drag_stopped() {
                drag_state.is_dragging = false;
            }

            // Context menu on right-click
            response.context_menu(|ui| {
                ui.set_min_width(180.0);

                if ui.button("Split Horizontal ->").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewHorizontal);
                    ui.close();
                }

                if ui.button("Split Vertical |").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewVertical);
                    ui.close();
                }

                ui.separator();

                if ui.button("< Join Left").clicked() {
                    let mut path_keep_right = path.clone();
                    path_keep_right.push(1); // Remove left, keep right child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_right));
                    ui.close();
                }

                if ui.button("Join Right >").clicked() {
                    let mut path_keep_left = path.clone();
                    path_keep_left.push(0); // Remove right, keep left child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_left));
                    ui.close();
                }

            });

            // Visual feedback
            let divider_color = if response.hovered() || response.dragged() {
                egui::Color32::from_gray(120)
            } else {
                egui::Color32::from_gray(60)
            };

            ui.painter().vline(
                split_x,
                rect.y_range(),
                egui::Stroke::new(2.0, divider_color),
            );
        }
        LayoutNode::VerticalGrid { percent, children } => {
            // Handle dragging
            if drag_state.is_dragging && drag_state.node_path == *path {
                if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
                    // Calculate new percentage based on pointer position
                    let new_percent = ((pointer_pos.y - rect.top()) / rect.height() * 100.0)
                        .clamp(10.0, 90.0); // Clamp to prevent too small panes
                    *percent = new_percent;
                }
            }

            // Split vertically (top / bottom)
            let split_y = rect.top() + (rect.height() * *percent / 100.0);

            let top_rect = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, split_y));

            let bottom_rect =
                egui::Rect::from_min_max(egui::pos2(rect.min.x, split_y), rect.max);

            // Render children
            let mut top_path = path.clone();
            top_path.push(0);
            render_layout_node(
                ui,
                &mut children[0],
                top_rect,
                drag_state,
                hovered_divider,
                selected_pane,
                layout_action,
                split_preview_mode,
                &top_path,
                ctx,
            );

            let mut bottom_path = path.clone();
            bottom_path.push(1);
            render_layout_node(
                ui,
                &mut children[1],
                bottom_rect,
                drag_state,
                hovered_divider,
                selected_pane,
                layout_action,
                split_preview_mode,
                &bottom_path,
                ctx,
            );

            // Draw divider with interaction
            let divider_height = 8.0;
            let divider_rect = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, split_y - divider_height / 2.0),
                egui::pos2(rect.max.x, split_y + divider_height / 2.0),
            );

            let divider_id = ui.id().with(("divider", path));
            let response = ui.interact(divider_rect, divider_id, egui::Sense::click_and_drag());

            // Check if pointer is over divider
            if response.hovered() {
                *hovered_divider = Some((path.clone(), false));
            }

            // Handle drag start
            if response.drag_started() {
                drag_state.is_dragging = true;
                drag_state.node_path = path.clone();
                drag_state.is_horizontal = false;
            }

            // Handle drag end
            if response.drag_stopped() {
                drag_state.is_dragging = false;
            }

            // Context menu on right-click
            response.context_menu(|ui| {
                ui.set_min_width(180.0);

                if ui.button("Split Horizontal ->").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewHorizontal);
                    ui.close();
                }

                if ui.button("Split Vertical |").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewVertical);
                    ui.close();
                }

                ui.separator();

                if ui.button("^ Join Up").clicked() {
                    let mut path_keep_bottom = path.clone();
                    path_keep_bottom.push(1); // Remove top, keep bottom child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_bottom));
                    ui.close();
                }

                if ui.button("Join Down v").clicked() {
                    let mut path_keep_top = path.clone();
                    path_keep_top.push(0); // Remove bottom, keep top child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_top));
                    ui.close();
                }

            });

            // Visual feedback
            let divider_color = if response.hovered() || response.dragged() {
                egui::Color32::from_gray(120)
            } else {
                egui::Color32::from_gray(60)
            };

            ui.painter().hline(
                rect.x_range(),
                split_y,
                egui::Stroke::new(2.0, divider_color),
            );
        }
    }
}

/// Render a single pane with its content
fn render_pane(
    ui: &mut egui::Ui,
    pane_name: &mut String,
    rect: egui::Rect,
    selected_pane: &mut Option<NodePath>,
    layout_action: &mut Option<LayoutAction>,
    split_preview_mode: &mut SplitPreviewMode,
    path: &NodePath,
    ctx: &mut RenderContext,
) {
    let pane_type = PaneType::from_name(pane_name);

    // Define header and content areas
    let header_height = 40.0;
    let header_rect = egui::Rect::from_min_size(
        rect.min,
        egui::vec2(rect.width(), header_height),
    );
    let content_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(0.0, header_height),
        egui::vec2(rect.width(), rect.height() - header_height),
    );

    // Draw header background
    let header_bg = ctx.shared.theme.bg_color(&[".pane-header"], ui.ctx(), egui::Color32::from_rgb(35, 35, 35));
    ui.painter().rect_filled(header_rect, 0.0, header_bg);

    // Draw content background
    let pane_id = pane_type.map(pane_type_css_id);
    let bg_color = if let Some(pane_id) = pane_id {
        ctx.shared.theme.bg_color(&[pane_id, ".pane-content"], ui.ctx(), pane_color(pane_type.unwrap()))
    } else {
        egui::Color32::from_rgb(40, 40, 40)
    };
    ui.painter().rect_filled(content_rect, 0.0, bg_color);

    // Draw border around entire pane
    let border_color = ctx.shared.theme.border_color(&[".pane-chrome"], ui.ctx(), egui::Color32::from_gray(80));
    let border_width = 1.0;
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(border_width, border_color),
        egui::StrokeKind::Middle,
    );

    // Draw header separator line
    let sep_color = ctx.shared.theme.border_color(&[".pane-chrome-separator"], ui.ctx(), egui::Color32::from_gray(50));
    ui.painter().hline(
        rect.x_range(),
        header_rect.max.y,
        egui::Stroke::new(1.0, sep_color),
    );

    // Render icon button in header (left side)
    let icon_size = 24.0;
    let icon_padding = 8.0;
    let icon_button_rect = egui::Rect::from_min_size(
        header_rect.min + egui::vec2(icon_padding, icon_padding),
        egui::vec2(icon_size, icon_size),
    );

    // Draw icon button background
    let icon_btn_bg = ctx.shared.theme.bg_color(&[".pane-icon-button"], ui.ctx(), egui::Color32::from_rgba_premultiplied(50, 50, 50, 200));
    ui.painter().rect_filled(icon_button_rect, 4.0, icon_btn_bg);

    // Load and render icon if available
    if let Some(pane_type) = pane_type {
        if let Some(icon) = ctx.shared.icon_cache.get_or_load(pane_type, ui.ctx()) {
            let icon_texture_id = icon.id();
            let icon_rect = icon_button_rect.shrink(2.0); // Small padding inside button
            ui.painter().image(
                icon_texture_id,
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    // Make icon button interactive (show pane type menu on click)
    let icon_button_id = ui.id().with(("icon_button", path));
    let icon_response = ui.interact(icon_button_rect, icon_button_id, egui::Sense::click());

    if icon_response.hovered() {
        ui.painter().rect_stroke(
            icon_button_rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(180)),
            egui::StrokeKind::Middle,
        );
    }

    // Show pane type selector menu on left click using new Popup API
    egui::containers::Popup::menu(&icon_response)
        .show(|ui| {
            ui.set_min_width(200.0);
            ui.label("Select Pane Type:");
            ui.separator();

            for pane_type_option in PaneType::all() {
                // Load icon for this pane type
                if let Some(icon) = ctx.shared.icon_cache.get_or_load(*pane_type_option, ui.ctx()) {
                    ui.horizontal(|ui| {
                        // Show icon
                        let icon_texture_id = icon.id();
                        let icon_size = egui::vec2(16.0, 16.0);
                        ui.add(egui::Image::new((icon_texture_id, icon_size)));

                        // Show label with selection
                        if ui.selectable_label(
                            pane_type == Some(*pane_type_option),
                            pane_type_option.display_name()
                        ).clicked() {
                            *pane_name = pane_type_option.to_name().to_string();
                        }
                    });
                } else {
                    // Fallback if icon fails to load
                    if ui.selectable_label(
                        pane_type == Some(*pane_type_option),
                        pane_type_option.display_name()
                    ).clicked() {
                        *pane_name = pane_type_option.to_name().to_string();
                    }
                }
            }
        });

    // Secondary tab selector for music/instrument panes
    let secondary_tab_types = [
        PaneType::VirtualPiano,
        PaneType::PianoRoll,
        PaneType::NodeEditor,
    ];
    let show_secondary_tabs = pane_type
        .map(|pt| secondary_tab_types.contains(&pt))
        .unwrap_or(false);

    let tab_size = 24.0;
    let secondary_selector_extra_width = if show_secondary_tabs {
        8.0 + 3.0 * tab_size + 8.0
    } else {
        0.0
    };

    if show_secondary_tabs {
        let n = secondary_tab_types.len();
        let selector_start_x = icon_button_rect.max.x + 8.0;
        let corner_r = 4.0_f32;
        let selector_rect = egui::Rect::from_min_size(
            egui::pos2(selector_start_x, header_rect.min.y + icon_padding),
            egui::vec2(n as f32 * tab_size, tab_size),
        );

        // Shared background
        ui.painter().rect_filled(
            selector_rect,
            corner_r,
            egui::Color32::from_rgba_premultiplied(50, 50, 50, 200),
        );

        for (i, &tab_type) in secondary_tab_types.iter().enumerate() {
            let tab_x = selector_start_x + i as f32 * tab_size;
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(tab_x, header_rect.min.y + icon_padding),
                egui::vec2(tab_size, tab_size),
            );

            let is_active = pane_type == Some(tab_type);

            // Active tab highlight with per-corner rounding
            if is_active {
                let cr = corner_r as u8;
                let rounding = egui::CornerRadius {
                    nw: if i == 0 { cr } else { 0 },
                    sw: if i == 0 { cr } else { 0 },
                    ne: if i == n - 1 { cr } else { 0 },
                    se: if i == n - 1 { cr } else { 0 },
                };
                ui.painter().rect_filled(
                    tab_rect,
                    rounding,
                    egui::Color32::from_rgba_premultiplied(60, 90, 150, 230),
                );
            }

            // Divider lines between tabs
            if i > 0 {
                let divider_color = if is_active || pane_type == Some(secondary_tab_types[i - 1]) {
                    egui::Color32::from_rgba_premultiplied(80, 110, 170, 180)
                } else {
                    egui::Color32::from_gray(70)
                };
                ui.painter().vline(
                    tab_x,
                    tab_rect.y_range(),
                    egui::Stroke::new(1.0, divider_color),
                );
            }

            // Icon
            if let Some(icon) = ctx.shared.icon_cache.get_or_load(tab_type, ui.ctx()) {
                let icon_texture_id = icon.id();
                ui.painter().image(
                    icon_texture_id,
                    tab_rect.shrink(3.0),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }

            // Interaction
            let tab_id = ui.id().with(("secondary_tab", path, i));
            let tab_response = ui.interact(tab_rect, tab_id, egui::Sense::click());

            if tab_response.hovered() && !is_active {
                ui.painter().rect_filled(
                    tab_rect,
                    egui::CornerRadius {
                        nw: if i == 0 { corner_r as u8 } else { 0 },
                        sw: if i == 0 { corner_r as u8 } else { 0 },
                        ne: if i == n - 1 { corner_r as u8 } else { 0 },
                        se: if i == n - 1 { corner_r as u8 } else { 0 },
                    },
                    egui::Color32::from_rgba_premultiplied(70, 70, 70, 180),
                );
            }

            if tab_response.clicked() {
                *pane_name = tab_type.to_name().to_string();
            }
        }

        // Outer border
        ui.painter().rect_stroke(
            selector_rect,
            corner_r,
            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
            egui::StrokeKind::Middle,
        );
    }

    // Draw pane title in header
    let title_text = if let Some(pane_type) = pane_type {
        pane_type.display_name()
    } else {
        pane_name.as_str()
    };
    let title_x_start = icon_padding * 2.0 + icon_size + 8.0 + secondary_selector_extra_width;
    let title_pos = header_rect.min + egui::vec2(title_x_start, header_height / 2.0);
    ui.painter().text(
        title_pos,
        egui::Align2::LEFT_CENTER,
        title_text,
        egui::FontId::proportional(14.0),
        egui::Color32::from_gray(220),
    );

    // Create header controls area (positioned after title)
    let title_width = 150.0; // Approximate width for title
    let header_controls_rect = egui::Rect::from_min_size(
        header_rect.min + egui::vec2(title_x_start + title_width, 0.0),
        egui::vec2(header_rect.width() - (title_x_start + title_width), header_height),
    );

    // Render pane-specific header controls (if pane has them)
    if let Some(pane_type) = pane_type {
        // Get or create pane instance for header rendering
        let needs_new_instance = ctx.pane_instances
            .get(path)
            .map(|instance| instance.pane_type() != pane_type)
            .unwrap_or(true);

        if needs_new_instance {
            ctx.pane_instances.insert(path.clone(), panes::PaneInstance::new(pane_type));
        }

        if let Some(pane_instance) = ctx.pane_instances.get_mut(path) {
            let mut header_ui = ui.new_child(egui::UiBuilder::new().max_rect(header_controls_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
            pane_instance.render_header(&mut header_ui, &mut ctx.shared);
        }
    }

    // Make pane content clickable (use content rect, not header, for split preview interaction)
    let pane_id = ui.id().with(("pane", path));
    let response = ui.interact(content_rect, pane_id, egui::Sense::click());

    // Render pane-specific content using trait-based system
    if let Some(pane_type) = pane_type {
        // Get or create pane instance for this path
        // Check if we need a new instance (either doesn't exist or type changed)
        let needs_new_instance = ctx.pane_instances
            .get(path)
            .map(|instance| instance.pane_type() != pane_type)
            .unwrap_or(true);

        if needs_new_instance {
            ctx.pane_instances.insert(path.clone(), PaneInstance::new(pane_type));
        }

        // Get the pane instance and render its content
        if let Some(pane_instance) = ctx.pane_instances.get_mut(path) {
            pane_instance.render_content(ui, content_rect, path, &mut ctx.shared);
        }
    } else {
        // Unknown pane type - draw placeholder
        let content_text = "Unknown pane type";
        let text_pos = content_rect.center();
        ui.painter().text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            content_text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_gray(150),
        );
    }

    // Handle split preview mode (rendered AFTER pane content for proper z-ordering)
    if let SplitPreviewMode::Active {
        is_horizontal,
        hovered_pane,
        split_percent,
    } = split_preview_mode
    {
        // Check if mouse is over this pane
        if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(pointer_pos) {
                // Update hovered pane
                *hovered_pane = Some(path.clone());

                // Calculate split percentage based on mouse position
                *split_percent = if *is_horizontal {
                    ((pointer_pos.x - rect.left()) / rect.width() * 100.0).clamp(10.0, 90.0)
                } else {
                    ((pointer_pos.y - rect.top()) / rect.height() * 100.0).clamp(10.0, 90.0)
                };

                // Render split preview overlay
                let grey_overlay = egui::Color32::from_rgba_premultiplied(128, 128, 128, 30);

                if *is_horizontal {
                    let split_x = rect.left() + (rect.width() * *split_percent / 100.0);

                    // First half
                    let first_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(split_x, rect.max.y),
                    );
                    ui.painter().rect_filled(first_rect, 0.0, grey_overlay);

                    // Second half
                    let second_rect = egui::Rect::from_min_max(
                        egui::pos2(split_x, rect.min.y),
                        rect.max,
                    );
                    ui.painter().rect_filled(second_rect, 0.0, grey_overlay);

                    // Divider line
                    ui.painter().vline(
                        split_x,
                        rect.y_range(),
                        egui::Stroke::new(2.0, egui::Color32::BLACK),
                    );
                } else {
                    let split_y = rect.top() + (rect.height() * *split_percent / 100.0);

                    // First half
                    let first_rect = egui::Rect::from_min_max(
                        rect.min,
                        egui::pos2(rect.max.x, split_y),
                    );
                    ui.painter().rect_filled(first_rect, 0.0, grey_overlay);

                    // Second half
                    let second_rect = egui::Rect::from_min_max(
                        egui::pos2(rect.min.x, split_y),
                        rect.max,
                    );
                    ui.painter().rect_filled(second_rect, 0.0, grey_overlay);

                    // Divider line
                    ui.painter().hline(
                        rect.x_range(),
                        split_y,
                        egui::Stroke::new(2.0, egui::Color32::BLACK),
                    );
                }

                // Create a high-priority interaction for split preview (rendered last = highest priority)
                let split_preview_id = ui.id().with(("split_preview", path));
                let split_response = ui.interact(rect, split_preview_id, egui::Sense::click());

                // If clicked, perform the split
                if split_response.clicked() {
                    if *is_horizontal {
                        *layout_action = Some(LayoutAction::SplitHorizontal(path.clone(), *split_percent));
                    } else {
                        *layout_action = Some(LayoutAction::SplitVertical(path.clone(), *split_percent));
                    }
                    // Exit preview mode
                    *split_preview_mode = SplitPreviewMode::None;
                }
            }
        }
    } else if response.clicked() {
        *selected_pane = Some(path.clone());
    }
}

/// Get a color for each pane type for visualization
fn pane_color(pane_type: PaneType) -> egui::Color32 {
    match pane_type {
        PaneType::Stage => egui::Color32::from_rgb(30, 40, 50),
        PaneType::Timeline => egui::Color32::from_rgb(40, 30, 50),
        PaneType::Toolbar => egui::Color32::from_rgb(50, 40, 30),
        PaneType::Infopanel => egui::Color32::from_rgb(30, 50, 40),
        PaneType::Outliner => egui::Color32::from_rgb(40, 50, 30),
        PaneType::PianoRoll => egui::Color32::from_rgb(55, 35, 45),
        PaneType::VirtualPiano => egui::Color32::from_rgb(45, 35, 55),
        PaneType::NodeEditor => egui::Color32::from_rgb(30, 45, 50),
        PaneType::PresetBrowser => egui::Color32::from_rgb(50, 45, 30),
        PaneType::AssetLibrary => egui::Color32::from_rgb(45, 50, 35),
        PaneType::ScriptEditor => egui::Color32::from_rgb(35, 30, 55),
    }
}

/// CSS ID selector for a pane type (e.g., PaneType::Stage -> "#stage")
fn pane_type_css_id(pane_type: PaneType) -> &'static str {
    match pane_type {
        PaneType::Stage => "#stage",
        PaneType::Timeline => "#timeline",
        PaneType::Toolbar => "#toolbar",
        PaneType::Infopanel => "#infopanel",
        PaneType::Outliner => "#outliner",
        PaneType::PianoRoll => "#piano-roll",
        PaneType::VirtualPiano => "#virtual-piano",
        PaneType::NodeEditor => "#node-editor",
        PaneType::PresetBrowser => "#preset-browser",
        PaneType::AssetLibrary => "#asset-library",
        PaneType::ScriptEditor => "#shader-editor",
    }
}

/// Split a pane node into a horizontal or vertical grid with two copies of the pane
fn split_node(root: &mut LayoutNode, path: &NodePath, is_horizontal: bool, percent: f32) {
    if path.is_empty() {
        // Split the root node
        if let LayoutNode::Pane { name } = root {
            let pane_name = name.clone();
            let new_node = if is_horizontal {
                LayoutNode::HorizontalGrid {
                    percent,
                    children: [
                        Box::new(LayoutNode::Pane { name: pane_name.clone() }),
                        Box::new(LayoutNode::Pane { name: pane_name }),
                    ],
                }
            } else {
                LayoutNode::VerticalGrid {
                    percent,
                    children: [
                        Box::new(LayoutNode::Pane { name: pane_name.clone() }),
                        Box::new(LayoutNode::Pane { name: pane_name }),
                    ],
                }
            };
            *root = new_node;
        }
    } else {
        // Navigate to parent and split the child
        navigate_to_node(root, &path[..path.len() - 1], &mut |node| {
            let child_index = path[path.len() - 1];
            match node {
                LayoutNode::HorizontalGrid { children, .. }
                | LayoutNode::VerticalGrid { children, .. } => {
                    if let LayoutNode::Pane { name } = &*children[child_index] {
                        let pane_name = name.clone();
                        let new_node = if is_horizontal {
                            LayoutNode::HorizontalGrid {
                                percent,
                                children: [
                                    Box::new(LayoutNode::Pane { name: pane_name.clone() }),
                                    Box::new(LayoutNode::Pane { name: pane_name }),
                                ],
                            }
                        } else {
                            LayoutNode::VerticalGrid {
                                percent,
                                children: [
                                    Box::new(LayoutNode::Pane { name: pane_name.clone() }),
                                    Box::new(LayoutNode::Pane { name: pane_name }),
                                ],
                            }
                        };
                        children[child_index] = Box::new(new_node);
                    }
                }
                _ => {}
            }
        });
    }
}

/// Remove a split by replacing it with one of its children
/// The path includes the split node path plus which child to keep (0 or 1 as last element)
fn remove_split(root: &mut LayoutNode, path: &NodePath) {
    if path.is_empty() {
        return; // Can't remove if path is empty
    }

    // Last element indicates which child to keep (0 or 1)
    let child_to_keep = path[path.len() - 1];

    // Path to the split node is everything except the last element
    let split_path = &path[..path.len() - 1];

    if split_path.is_empty() {
        // Removing root split - replace root with the chosen child
        if let LayoutNode::HorizontalGrid { children, .. }
        | LayoutNode::VerticalGrid { children, .. } = root
        {
            *root = (*children[child_to_keep]).clone();
        }
    } else {
        // Navigate to parent of the split node and replace it
        let parent_path = &split_path[..split_path.len() - 1];
        let split_index = split_path[split_path.len() - 1];

        navigate_to_node(root, parent_path, &mut |node| {
            match node {
                LayoutNode::HorizontalGrid { children, .. }
                | LayoutNode::VerticalGrid { children, .. } => {
                    // Get the split node's chosen child
                    if let LayoutNode::HorizontalGrid { children: split_children, .. }
                    | LayoutNode::VerticalGrid { children: split_children, .. } =
                        &*children[split_index]
                    {
                        // Replace the split node with the chosen child
                        children[split_index] = split_children[child_to_keep].clone();
                    }
                }
                _ => {}
            }
        });
    }
}

/// Navigate to a node at the given path and apply a function to it
fn navigate_to_node<F>(node: &mut LayoutNode, path: &[usize], f: &mut F)
where
    F: FnMut(&mut LayoutNode),
{
    if path.is_empty() {
        f(node);
    } else {
        match node {
            LayoutNode::HorizontalGrid { children, .. }
            | LayoutNode::VerticalGrid { children, .. } => {
                navigate_to_node(&mut children[path[0]], &path[1..], f);
            }
            _ => {}
        }
    }
}
