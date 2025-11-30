use eframe::egui;
use lightningbeam_core::layout::{LayoutDefinition, LayoutNode};
use lightningbeam_core::pane::PaneType;
use lightningbeam_core::tool::Tool;
use std::collections::HashMap;
use clap::Parser;
use uuid::Uuid;

mod panes;
use panes::{PaneInstance, PaneRenderer, SharedPaneState};

mod widgets;

mod menu;
use menu::{MenuAction, MenuSystem};

mod theme;
use theme::{Theme, ThemeMode};

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
}

fn main() -> eframe::Result {
    println!("🚀 Starting Lightningbeam Editor...");

    // Parse command line arguments
    let args = Args::parse();

    // Determine theme mode from arguments
    let theme_mode = if args.light {
        ThemeMode::Light
    } else if args.dark {
        ThemeMode::Dark
    } else {
        ThemeMode::System
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
    let icon_data = include_bytes!("../../../src-tauri/icons/icon.png");
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

    let mut viewport_builder = egui::ViewportBuilder::default()
        .with_inner_size([1920.0, 1080.0])
        .with_title("Lightningbeam Editor")
        .with_app_id("lightningbeam-editor"); // Set app_id for Wayland

    if let Some(icon) = icon_image {
        viewport_builder = viewport_builder.with_icon(icon);
    }

    let options = eframe::NativeOptions {
        viewport: viewport_builder,
        ..Default::default()
    };

    eframe::run_native(
        "Lightningbeam Editor",
        options,
        Box::new(move |cc| Ok(Box::new(EditorApp::new(cc, layouts, theme)))),
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

/// Icon cache for pane type icons
struct IconCache {
    icons: HashMap<PaneType, egui_extras::RetainedImage>,
    assets_path: std::path::PathBuf,
}

impl IconCache {
    fn new() -> Self {
        let assets_path = std::path::PathBuf::from(
            std::env::var("HOME").unwrap_or_else(|_| "/home/skyler".to_string())
        ).join("Dev/Lightningbeam-2/src/assets");

        Self {
            icons: HashMap::new(),
            assets_path,
        }
    }

    fn get_or_load(&mut self, pane_type: PaneType) -> Option<&egui_extras::RetainedImage> {
        if !self.icons.contains_key(&pane_type) {
            // Load and cache the icon
            let icon_path = self.assets_path.join(pane_type.icon_file());
            if let Ok(image) = egui_extras::RetainedImage::from_svg_bytes(
                pane_type.icon_file(),
                &std::fs::read(&icon_path).unwrap_or_default(),
            ) {
                self.icons.insert(pane_type, image);
            }
        }
        self.icons.get(&pane_type)
    }
}

/// Icon cache for tool icons
struct ToolIconCache {
    icons: HashMap<Tool, egui::TextureHandle>,
    assets_path: std::path::PathBuf,
}

impl ToolIconCache {
    fn new() -> Self {
        let assets_path = std::path::PathBuf::from(
            std::env::var("HOME").unwrap_or_else(|_| "/home/skyler".to_string())
        ).join("Dev/Lightningbeam-2/src/assets");

        Self {
            icons: HashMap::new(),
            assets_path,
        }
    }

    fn get_or_load(&mut self, tool: Tool, ctx: &egui::Context) -> Option<&egui::TextureHandle> {
        if !self.icons.contains_key(&tool) {
            // Load SVG and rasterize at high resolution using resvg
            let icon_path = self.assets_path.join(tool.icon_file());
            if let Ok(svg_data) = std::fs::read(&icon_path) {
                // Rasterize at 3x size for crisp display (180px for 60px display)
                let render_size = 180;

                if let Ok(tree) = resvg::usvg::Tree::from_data(&svg_data, &resvg::usvg::Options::default()) {
                    let pixmap_size = tree.size().to_int_size();
                    let scale_x = render_size as f32 / pixmap_size.width() as f32;
                    let scale_y = render_size as f32 / pixmap_size.height() as f32;
                    let scale = scale_x.min(scale_y);

                    let final_size = resvg::usvg::Size::from_wh(
                        pixmap_size.width() as f32 * scale,
                        pixmap_size.height() as f32 * scale,
                    ).unwrap_or(resvg::usvg::Size::from_wh(render_size as f32, render_size as f32).unwrap());

                    if let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(
                        final_size.width() as u32,
                        final_size.height() as u32,
                    ) {
                        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
                        resvg::render(&tree, transform, &mut pixmap.as_mut());

                        // Convert RGBA8 to egui ColorImage
                        let rgba_data = pixmap.data();
                        let size = [pixmap.width() as usize, pixmap.height() as usize];
                        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba_data);

                        // Upload to GPU
                        let texture = ctx.load_texture(
                            tool.icon_file(),
                            color_image,
                            egui::TextureOptions::LINEAR,
                        );
                        self.icons.insert(tool, texture);
                    }
                }
            }
        }
        self.icons.get(&tool)
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
    icon_cache: IconCache,
    tool_icon_cache: ToolIconCache,
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
    tool_state: lightningbeam_core::tool::ToolState, // Current tool interaction state
    // Draw tool configuration
    draw_simplify_mode: lightningbeam_core::tool::SimplifyMode, // Current simplification mode for draw tool
    rdp_tolerance: f64, // RDP simplification tolerance (default: 10.0)
    schneider_max_error: f64, // Schneider curve fitting max error (default: 30.0)
    // Audio engine integration
    audio_system: Option<daw_backend::AudioSystem>, // Audio system (must be kept alive for stream)
    // Playback state (global for all panes)
    playback_time: f64, // Current playback position in seconds (persistent - save with document)
    is_playing: bool,   // Whether playback is currently active (transient - don't save)
    // Asset drag-and-drop state
    dragging_asset: Option<panes::DraggingAsset>, // Asset being dragged from Asset Library
    // Import dialog state
    last_import_filter: ImportFilter, // Last used import filter (remembered across imports)
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
    fn new(cc: &eframe::CreationContext, layouts: Vec<LayoutDefinition>, theme: Theme) -> Self {
        let current_layout = layouts[0].layout.clone();

        // Initialize native menu system
        let menu_system = MenuSystem::new().ok();

        // Create default document with a simple test scene
        let mut document = lightningbeam_core::document::Document::with_size("Untitled Animation", 1920.0, 1080.0)
            .with_duration(10.0)
            .with_framerate(60.0);

        // Add a test layer with a simple shape to visualize
        use lightningbeam_core::layer::{AnyLayer, VectorLayer};
        use lightningbeam_core::object::ShapeInstance;
        use lightningbeam_core::shape::{Shape, ShapeColor};
        use vello::kurbo::{Circle, Shape as KurboShape};

        let circle = Circle::new((200.0, 150.0), 50.0);
        let path = circle.to_path(0.1);
        let shape = Shape::new(path).with_fill(ShapeColor::rgb(100, 150, 250));
        let object = ShapeInstance::new(shape.id);

        let mut vector_layer = VectorLayer::new("Layer 1");
        vector_layer.add_shape(shape);
        vector_layer.add_object(object);
        let layer_id = document.root.add_child(AnyLayer::Vector(vector_layer));

        // Wrap document in ActionExecutor
        let action_executor = lightningbeam_core::action::ActionExecutor::new(document);

        // Initialize audio system (keep the whole system to maintain the audio stream)
        let audio_system = match daw_backend::AudioSystem::new(None, 256) {
            Ok(audio_system) => {
                println!("✅ Audio engine initialized successfully");
                Some(audio_system)
            }
            Err(e) => {
                eprintln!("❌ Failed to initialize audio engine: {}", e);
                eprintln!("   Playback will be disabled");
                None
            }
        };

        Self {
            layouts,
            current_layout_index: 0,
            current_layout,
            drag_state: DragState::default(),
            hovered_divider: None,
            selected_pane: None,
            split_preview_mode: SplitPreviewMode::default(),
            icon_cache: IconCache::new(),
            tool_icon_cache: ToolIconCache::new(),
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
            tool_state: lightningbeam_core::tool::ToolState::default(),
            draw_simplify_mode: lightningbeam_core::tool::SimplifyMode::Smooth, // Default to smooth curves
            rdp_tolerance: 10.0, // Default RDP tolerance
            schneider_max_error: 30.0, // Default Schneider max error
            audio_system,
            playback_time: 0.0, // Start at beginning
            is_playing: false,  // Start paused
            dragging_asset: None, // No asset being dragged initially
            last_import_filter: ImportFilter::default(), // Default to "All Supported"
        }
    }

    fn switch_layout(&mut self, index: usize) {
        self.current_layout_index = index;
        self.current_layout = self.layouts[index].layout.clone();
    }

    fn current_layout_def(&self) -> &LayoutDefinition {
        &self.layouts[self.current_layout_index]
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

    fn handle_menu_action(&mut self, action: MenuAction) {
        match action {
            // File menu
            MenuAction::NewFile => {
                println!("Menu: New File");
                // TODO: Implement new file
            }
            MenuAction::NewWindow => {
                println!("Menu: New Window");
                // TODO: Implement new window
            }
            MenuAction::Save => {
                println!("Menu: Save");
                // TODO: Implement save
            }
            MenuAction::SaveAs => {
                println!("Menu: Save As");
                // TODO: Implement save as
            }
            MenuAction::OpenFile => {
                println!("Menu: Open File");
                // TODO: Implement open file
            }
            MenuAction::Revert => {
                println!("Menu: Revert");
                // TODO: Implement revert
            }
            MenuAction::Import => {
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
                    // Get extension and detect file type
                    let extension = path.extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("");

                    match get_file_type(extension) {
                        Some(FileType::Image) => {
                            self.last_import_filter = ImportFilter::Images;
                            self.import_image(&path);
                        }
                        Some(FileType::Audio) => {
                            self.last_import_filter = ImportFilter::Audio;
                            self.import_audio(&path);
                        }
                        Some(FileType::Video) => {
                            self.last_import_filter = ImportFilter::Video;
                            self.import_video(&path);
                        }
                        Some(FileType::Midi) => {
                            self.last_import_filter = ImportFilter::Midi;
                            self.import_midi(&path);
                        }
                        None => {
                            println!("Unsupported file type: {}", extension);
                        }
                    }
                }
            }
            MenuAction::Export => {
                println!("Menu: Export");
                // TODO: Implement export
            }
            MenuAction::Quit => {
                println!("Menu: Quit");
                std::process::exit(0);
            }

            // Edit menu
            MenuAction::Undo => {
                if self.action_executor.undo() {
                    println!("Undid: {}", self.action_executor.redo_description().unwrap_or_default());
                } else {
                    println!("Nothing to undo");
                }
            }
            MenuAction::Redo => {
                if self.action_executor.redo() {
                    println!("Redid: {}", self.action_executor.undo_description().unwrap_or_default());
                } else {
                    println!("Nothing to redo");
                }
            }
            MenuAction::Cut => {
                println!("Menu: Cut");
                // TODO: Implement cut
            }
            MenuAction::Copy => {
                println!("Menu: Copy");
                // TODO: Implement copy
            }
            MenuAction::Paste => {
                println!("Menu: Paste");
                // TODO: Implement paste
            }
            MenuAction::Delete => {
                println!("Menu: Delete");
                // TODO: Implement delete
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
                println!("Menu: Preferences");
                // TODO: Implement preferences dialog
            }

            // Modify menu
            MenuAction::Group => {
                println!("Menu: Group");
                // TODO: Implement group
            }
            MenuAction::SendToBack => {
                println!("Menu: Send to Back");
                // TODO: Implement send to back
            }
            MenuAction::BringToFront => {
                println!("Menu: Bring to Front");
                // TODO: Implement bring to front
            }

            // Layer menu
            MenuAction::AddLayer => {
                // Create a new vector layer with a default name
                let layer_count = self.action_executor.document().root.children.len();
                let layer_name = format!("Layer {}", layer_count + 1);

                let action = lightningbeam_core::actions::AddLayerAction::new_vector(layer_name);
                self.action_executor.execute(Box::new(action));

                // Select the newly created layer (last child in the document)
                if let Some(last_layer) = self.action_executor.document().root.children.last() {
                    self.active_layer_id = Some(last_layer.id());
                }
            }
            MenuAction::AddVideoLayer => {
                println!("Menu: Add Video Layer");
                // TODO: Implement add video layer
            }
            MenuAction::AddAudioTrack => {
                println!("Menu: Add Audio Track");
                // TODO: Implement add audio track
            }
            MenuAction::AddMidiTrack => {
                println!("Menu: Add MIDI Track");
                // TODO: Implement add MIDI track
            }
            MenuAction::AddTestClip => {
                // Create a test vector clip and add it to the library (not to timeline)
                use lightningbeam_core::clip::VectorClip;
                use lightningbeam_core::layer::{VectorLayer, AnyLayer};
                use lightningbeam_core::shape::{Shape, ShapeColor};
                use lightningbeam_core::object::ShapeInstance;
                use kurbo::{Circle, Rect, Shape as KurboShape};

                // Generate unique name based on existing clip count
                let clip_count = self.action_executor.document().vector_clips.len();
                let clip_name = format!("Test Clip {}", clip_count + 1);

                let mut test_clip = VectorClip::new(&clip_name, 400.0, 400.0, 5.0);

                // Create a layer with some shapes
                let mut layer = VectorLayer::new("Shapes");

                // Create a red circle shape
                let circle_path = Circle::new((100.0, 100.0), 50.0).to_path(0.1);
                let mut circle_shape = Shape::new(circle_path);
                circle_shape.fill_color = Some(ShapeColor::rgb(255, 0, 0));
                let circle_id = circle_shape.id;
                layer.add_shape(circle_shape);

                // Create a blue rectangle shape
                let rect_path = Rect::new(200.0, 50.0, 350.0, 150.0).to_path(0.1);
                let mut rect_shape = Shape::new(rect_path);
                rect_shape.fill_color = Some(ShapeColor::rgb(0, 0, 255));
                let rect_id = rect_shape.id;
                layer.add_shape(rect_shape);

                // Add shape instances
                layer.shape_instances.push(ShapeInstance::new(circle_id));
                layer.shape_instances.push(ShapeInstance::new(rect_id));

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
                println!("Menu: Toggle Layer Visibility");
                // TODO: Implement toggle layer visibility
            }

            // Timeline menu
            MenuAction::NewKeyframe => {
                println!("Menu: New Keyframe");
                // TODO: Implement new keyframe
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
            MenuAction::AddKeyframeAtPlayhead => {
                println!("Menu: Add Keyframe at Playhead");
                // TODO: Implement add keyframe at playhead
            }
            MenuAction::AddMotionTween => {
                println!("Menu: Add Motion Tween");
                // TODO: Implement add motion tween
            }
            MenuAction::AddShapeTween => {
                println!("Menu: Add Shape Tween");
                // TODO: Implement add shape tween
            }
            MenuAction::ReturnToStart => {
                println!("Menu: Return to Start");
                // TODO: Implement return to start
            }
            MenuAction::Play => {
                println!("Menu: Play");
                // TODO: Implement play/pause
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

    /// Import an image file as an ImageAsset
    fn import_image(&mut self, path: &std::path::Path) {
        use lightningbeam_core::clip::ImageAsset;

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
            }
            Err(e) => {
                eprintln!("Failed to load image '{}': {}", path.display(), e);
            }
        }
    }

    /// Import an audio file via daw-backend
    fn import_audio(&mut self, path: &std::path::Path) {
        use daw_backend::io::audio_file::AudioFile;
        use lightningbeam_core::clip::{AudioClip, AudioClipType};

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled Audio")
            .to_string();

        // Load audio file via daw-backend
        match AudioFile::load(path) {
            Ok(audio_file) => {
                let duration = audio_file.frames as f64 / audio_file.sample_rate as f64;
                let channels = audio_file.channels;
                let sample_rate = audio_file.sample_rate;

                // Add to audio engine pool if available
                if let Some(ref mut audio_system) = self.audio_system {
                    // Send audio data to the engine
                    let path_str = path.to_string_lossy().to_string();
                    audio_system.controller.add_audio_file(
                        path_str.clone(),
                        audio_file.data,
                        channels,
                        sample_rate,
                    );

                    // For now, use a placeholder pool index (the engine will assign the real one)
                    // In a full implementation, we'd wait for the AudioFileAdded event
                    let pool_index = self.action_executor.document().audio_clips.len();

                    // Create audio clip in document
                    let clip = AudioClip::new_sampled(&name, pool_index, duration);
                    let clip_id = self.action_executor.document_mut().add_audio_clip(clip);
                    println!("Imported audio '{}' ({:.1}s, {}ch, {}Hz) - ID: {}",
                        name, duration, channels, sample_rate, clip_id);
                } else {
                    eprintln!("Cannot import audio: audio engine not initialized");
                }
            }
            Err(e) => {
                eprintln!("Failed to load audio '{}': {}", path.display(), e);
            }
        }
    }

    /// Import a MIDI file via daw-backend
    fn import_midi(&mut self, path: &std::path::Path) {
        use lightningbeam_core::clip::{AudioClip, AudioClipType, MidiEvent};

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled MIDI")
            .to_string();

        // Load MIDI file via daw-backend
        // Note: daw-backend's load_midi_file returns a MidiClip with events
        match daw_backend::io::midi_file::load_midi_file(path, 0, 44100) {
            Ok(midi_clip) => {
                // Convert daw-backend MidiEvents to our MidiEvent type
                let events: Vec<MidiEvent> = midi_clip.events.iter().map(|e| {
                    MidiEvent::new(e.timestamp, e.status, e.data1, e.data2)
                }).collect();

                let duration = midi_clip.duration;

                // Create MIDI audio clip in document
                let clip = AudioClip::new_midi(&name, duration, events, false);
                let clip_id = self.action_executor.document_mut().add_audio_clip(clip);
                println!("Imported MIDI '{}' ({:.1}s, {} events) - ID: {}",
                    name, duration, midi_clip.events.len(), clip_id);
            }
            Err(e) => {
                eprintln!("Failed to load MIDI '{}': {}", path.display(), e);
            }
        }
    }

    /// Import a video file (placeholder - decoder not yet ported)
    fn import_video(&mut self, path: &std::path::Path) {
        use lightningbeam_core::clip::VideoClip;

        let name = path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled Video")
            .to_string();

        // TODO: Use video decoder to get actual dimensions/duration
        // For now, create a placeholder with default values
        let clip = VideoClip::new(
            &name,
            path.to_string_lossy().to_string(),
            1920.0,  // Default width (TODO: probe video)
            1080.0,  // Default height (TODO: probe video)
            0.0,     // Duration unknown (TODO: probe video)
            30.0,    // Default frame rate (TODO: probe video)
        );

        let clip_id = self.action_executor.document_mut().add_video_clip(clip);
        println!("Imported video '{}' (placeholder - dimensions/duration unknown) - ID: {}", name, clip_id);
        println!("Note: Video decoder not yet ported. Video preview unavailable.");
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Disable egui's built-in Ctrl+Plus/Minus zoom behavior
        // We handle zoom ourselves for the Stage pane
        ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
        });

        // Check for native menu events (macOS)
        if let Some(menu_system) = &self.menu_system {
            if let Some(action) = menu_system.check_events() {
                self.handle_menu_action(action);
            }
        }

        // Poll audio events from the audio engine
        if let Some(audio_system) = &mut self.audio_system {
            if let Some(event_rx) = &mut audio_system.event_rx {
                while let Ok(event) = event_rx.pop() {
                    use daw_backend::AudioEvent;
                    match event {
                        AudioEvent::PlaybackPosition(time) => {
                            self.playback_time = time;
                        }
                        AudioEvent::PlaybackStopped => {
                            self.is_playing = false;
                        }
                        _ => {} // Ignore other events for now
                    }
                }
            }
        }

        // Request continuous repaints when playing to update time display
        if self.is_playing {
            ctx.request_repaint();
        }

        // Top menu bar (egui-rendered on all platforms)
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            if let Some(action) = MenuSystem::render_egui_menu_bar(ui) {
                self.handle_menu_action(action);
            }
        });

        // Main pane area
        let mut layout_action: Option<LayoutAction> = None;
        egui::CentralPanel::default().show(ctx, |ui| {
            let available_rect = ui.available_rect_before_wrap();

            // Reset hovered divider each frame
            self.hovered_divider = None;

            // Track fallback pane priority for view actions (reset each frame)
            let mut fallback_pane_priority: Option<u32> = None;

            // Registry for view action handlers (two-phase dispatch)
            let mut pending_handlers: Vec<panes::ViewActionHandler> = Vec::new();

            // Registry for actions to execute after rendering (two-phase dispatch)
            let mut pending_actions: Vec<Box<dyn lightningbeam_core::action::Action>> = Vec::new();

            // Create render context
            let mut ctx = RenderContext {
                tool_icon_cache: &mut self.tool_icon_cache,
                icon_cache: &mut self.icon_cache,
                selected_tool: &mut self.selected_tool,
                fill_color: &mut self.fill_color,
                stroke_color: &mut self.stroke_color,
                active_color_mode: &mut self.active_color_mode,
                pane_instances: &mut self.pane_instances,
                pending_view_action: &mut self.pending_view_action,
                fallback_pane_priority: &mut fallback_pane_priority,
                pending_handlers: &mut pending_handlers,
                theme: &self.theme,
                action_executor: &mut self.action_executor,
                selection: &mut self.selection,
                active_layer_id: &mut self.active_layer_id,
                tool_state: &mut self.tool_state,
                pending_actions: &mut pending_actions,
                draw_simplify_mode: &mut self.draw_simplify_mode,
                rdp_tolerance: &mut self.rdp_tolerance,
                schneider_max_error: &mut self.schneider_max_error,
                audio_controller: self.audio_system.as_mut().map(|sys| &mut sys.controller),
                playback_time: &mut self.playback_time,
                is_playing: &mut self.is_playing,
                dragging_asset: &mut self.dragging_asset,
            };

            render_layout_node(
                ui,
                &mut self.current_layout,
                available_rect,
                &mut self.drag_state,
                &mut self.hovered_divider,
                &mut self.selected_pane,
                &mut layout_action,
                &mut self.split_preview_mode,
                &Vec::new(), // Root path
                &mut ctx,
            );

            // Execute action on the best handler (two-phase dispatch)
            if let Some(action) = &self.pending_view_action {
                if let Some(best_handler) = pending_handlers.iter().min_by_key(|h| h.priority) {
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

            // Execute all pending actions (two-phase dispatch)
            for action in pending_actions {
                self.action_executor.execute(action);
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

        ctx.input(|i| {
            // Check menu shortcuts (these use modifiers, so allow even when typing)
            if let Some(action) = MenuSystem::check_shortcuts(i) {
                self.handle_menu_action(action);
            }

            // Check tool shortcuts (only if no modifiers are held AND no text input is focused)
            if !wants_keyboard && !i.modifiers.ctrl && !i.modifiers.shift && !i.modifiers.alt && !i.modifiers.command {
                use lightningbeam_core::tool::Tool;

                if i.key_pressed(egui::Key::V) {
                    self.selected_tool = Tool::Select;
                } else if i.key_pressed(egui::Key::P) {
                    self.selected_tool = Tool::Draw;
                } else if i.key_pressed(egui::Key::Q) {
                    self.selected_tool = Tool::Transform;
                } else if i.key_pressed(egui::Key::R) {
                    self.selected_tool = Tool::Rectangle;
                } else if i.key_pressed(egui::Key::E) {
                    self.selected_tool = Tool::Ellipse;
                } else if i.key_pressed(egui::Key::B) {
                    self.selected_tool = Tool::PaintBucket;
                } else if i.key_pressed(egui::Key::I) {
                    self.selected_tool = Tool::Eyedropper;
                } else if i.key_pressed(egui::Key::L) {
                    self.selected_tool = Tool::Line;
                } else if i.key_pressed(egui::Key::G) {
                    self.selected_tool = Tool::Polygon;
                } else if i.key_pressed(egui::Key::A) {
                    self.selected_tool = Tool::BezierEdit;
                } else if i.key_pressed(egui::Key::T) {
                    self.selected_tool = Tool::Text;
                }
            }
        });
    }

}

/// Context for rendering operations - bundles all mutable state needed during rendering
/// This avoids having 25+ individual parameters in rendering functions
struct RenderContext<'a> {
    tool_icon_cache: &'a mut ToolIconCache,
    icon_cache: &'a mut IconCache,
    selected_tool: &'a mut Tool,
    fill_color: &'a mut egui::Color32,
    stroke_color: &'a mut egui::Color32,
    active_color_mode: &'a mut panes::ColorMode,
    pane_instances: &'a mut HashMap<NodePath, PaneInstance>,
    pending_view_action: &'a mut Option<MenuAction>,
    fallback_pane_priority: &'a mut Option<u32>,
    pending_handlers: &'a mut Vec<panes::ViewActionHandler>,
    theme: &'a Theme,
    action_executor: &'a mut lightningbeam_core::action::ActionExecutor,
    selection: &'a mut lightningbeam_core::selection::Selection,
    active_layer_id: &'a mut Option<Uuid>,
    tool_state: &'a mut lightningbeam_core::tool::ToolState,
    pending_actions: &'a mut Vec<Box<dyn lightningbeam_core::action::Action>>,
    draw_simplify_mode: &'a mut lightningbeam_core::tool::SimplifyMode,
    rdp_tolerance: &'a mut f64,
    schneider_max_error: &'a mut f64,
    audio_controller: Option<&'a mut daw_backend::EngineController>,
    playback_time: &'a mut f64,
    is_playing: &'a mut bool,
    dragging_asset: &'a mut Option<panes::DraggingAsset>,
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
                    ui.close_menu();
                }

                if ui.button("Split Vertical |").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewVertical);
                    ui.close_menu();
                }

                ui.separator();

                if ui.button("< Join Left").clicked() {
                    let mut path_keep_right = path.clone();
                    path_keep_right.push(1); // Remove left, keep right child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_right));
                    ui.close_menu();
                }

                if ui.button("Join Right >").clicked() {
                    let mut path_keep_left = path.clone();
                    path_keep_left.push(0); // Remove right, keep left child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_left));
                    ui.close_menu();
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
                    ui.close_menu();
                }

                if ui.button("Split Vertical |").clicked() {
                    *layout_action = Some(LayoutAction::EnterSplitPreviewVertical);
                    ui.close_menu();
                }

                ui.separator();

                if ui.button("^ Join Up").clicked() {
                    let mut path_keep_bottom = path.clone();
                    path_keep_bottom.push(1); // Remove top, keep bottom child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_bottom));
                    ui.close_menu();
                }

                if ui.button("Join Down v").clicked() {
                    let mut path_keep_top = path.clone();
                    path_keep_top.push(0); // Remove bottom, keep top child
                    *layout_action = Some(LayoutAction::RemoveSplit(path_keep_top));
                    ui.close_menu();
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
    ui.painter().rect_filled(
        header_rect,
        0.0,
        egui::Color32::from_rgb(35, 35, 35),
    );

    // Draw content background
    let bg_color = if let Some(pane_type) = pane_type {
        pane_color(pane_type)
    } else {
        egui::Color32::from_rgb(40, 40, 40)
    };
    ui.painter().rect_filled(content_rect, 0.0, bg_color);

    // Draw border around entire pane
    let border_color = egui::Color32::from_gray(80);
    let border_width = 1.0;
    ui.painter().rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(border_width, border_color),
        egui::StrokeKind::Middle,
    );

    // Draw header separator line
    ui.painter().hline(
        rect.x_range(),
        header_rect.max.y,
        egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
    );

    // Render icon button in header (left side)
    let icon_size = 24.0;
    let icon_padding = 8.0;
    let icon_button_rect = egui::Rect::from_min_size(
        header_rect.min + egui::vec2(icon_padding, icon_padding),
        egui::vec2(icon_size, icon_size),
    );

    // Draw icon button background
    ui.painter().rect_filled(
        icon_button_rect,
        4.0,
        egui::Color32::from_rgba_premultiplied(50, 50, 50, 200),
    );

    // Load and render icon if available
    if let Some(pane_type) = pane_type {
        if let Some(icon) = ctx.icon_cache.get_or_load(pane_type) {
            let icon_texture_id = icon.texture_id(ui.ctx());
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

    // Show pane type selector menu on left click
    let menu_id = ui.id().with(("pane_type_menu", path));
    if icon_response.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(menu_id));
    }

    egui::popup::popup_below_widget(ui, menu_id, &icon_response, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
        ui.set_min_width(200.0);
        ui.label("Select Pane Type:");
        ui.separator();

        for pane_type_option in PaneType::all() {
            // Load icon for this pane type
            if let Some(icon) = ctx.icon_cache.get_or_load(*pane_type_option) {
                ui.horizontal(|ui| {
                    // Show icon
                    let icon_texture_id = icon.texture_id(ui.ctx());
                    let icon_size = egui::vec2(16.0, 16.0);
                    ui.add(egui::Image::new((icon_texture_id, icon_size)));

                    // Show label with selection
                    if ui.selectable_label(
                        pane_type == Some(*pane_type_option),
                        pane_type_option.display_name()
                    ).clicked() {
                        *pane_name = pane_type_option.to_name().to_string();
                        ui.memory_mut(|mem| mem.close_popup());
                    }
                });
            } else {
                // Fallback if icon fails to load
                if ui.selectable_label(
                    pane_type == Some(*pane_type_option),
                    pane_type_option.display_name()
                ).clicked() {
                    *pane_name = pane_type_option.to_name().to_string();
                    ui.memory_mut(|mem| mem.close_popup());
                }
            }
        }
    });

    // Draw pane title in header
    let title_text = if let Some(pane_type) = pane_type {
        pane_type.display_name()
    } else {
        pane_name.as_str()
    };
    let title_pos = header_rect.min + egui::vec2(icon_padding * 2.0 + icon_size + 8.0, header_height / 2.0);
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
        header_rect.min + egui::vec2(icon_padding * 2.0 + icon_size + 8.0 + title_width, 0.0),
        egui::vec2(header_rect.width() - (icon_padding * 2.0 + icon_size + 8.0 + title_width), header_height),
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
            let mut shared = panes::SharedPaneState {
                tool_icon_cache: ctx.tool_icon_cache,
                icon_cache: ctx.icon_cache,
                selected_tool: ctx.selected_tool,
                fill_color: ctx.fill_color,
                stroke_color: ctx.stroke_color,
                active_color_mode: ctx.active_color_mode,
                pending_view_action: ctx.pending_view_action,
                fallback_pane_priority: ctx.fallback_pane_priority,
                theme: ctx.theme,
                pending_handlers: ctx.pending_handlers,
                action_executor: ctx.action_executor,
                selection: ctx.selection,
                active_layer_id: ctx.active_layer_id,
                tool_state: ctx.tool_state,
                pending_actions: ctx.pending_actions,
                draw_simplify_mode: ctx.draw_simplify_mode,
                rdp_tolerance: ctx.rdp_tolerance,
                schneider_max_error: ctx.schneider_max_error,
                audio_controller: ctx.audio_controller.as_mut().map(|c| &mut **c),
                playback_time: ctx.playback_time,
                is_playing: ctx.is_playing,
                dragging_asset: ctx.dragging_asset,
            };
            pane_instance.render_header(&mut header_ui, &mut shared);
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
            // Create shared state
            let mut shared = SharedPaneState {
                tool_icon_cache: ctx.tool_icon_cache,
                icon_cache: ctx.icon_cache,
                selected_tool: ctx.selected_tool,
                fill_color: ctx.fill_color,
                stroke_color: ctx.stroke_color,
                active_color_mode: ctx.active_color_mode,
                pending_view_action: ctx.pending_view_action,
                fallback_pane_priority: ctx.fallback_pane_priority,
                theme: ctx.theme,
                pending_handlers: ctx.pending_handlers,
                action_executor: ctx.action_executor,
                selection: ctx.selection,
                active_layer_id: ctx.active_layer_id,
                tool_state: ctx.tool_state,
                pending_actions: ctx.pending_actions,
                draw_simplify_mode: ctx.draw_simplify_mode,
                rdp_tolerance: ctx.rdp_tolerance,
                schneider_max_error: ctx.schneider_max_error,
                audio_controller: ctx.audio_controller.as_mut().map(|c| &mut **c),
                playback_time: ctx.playback_time,
                is_playing: ctx.is_playing,
                dragging_asset: ctx.dragging_asset,
            };

            // Render pane content (header was already rendered above)
            pane_instance.render_content(ui, content_rect, path, &mut shared);
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

/// Render toolbar with tool buttons
fn render_toolbar(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    tool_icon_cache: &mut ToolIconCache,
    selected_tool: &mut Tool,
    path: &NodePath,
) {
    let button_size = 60.0; // 50% bigger (was 40.0)
    let button_padding = 8.0;
    let button_spacing = 4.0;

    // Calculate how many columns we can fit
    let available_width = rect.width() - (button_padding * 2.0);
    let columns = ((available_width + button_spacing) / (button_size + button_spacing)).floor() as usize;
    let columns = columns.max(1); // At least 1 column

    let mut x = rect.left() + button_padding;
    let mut y = rect.top() + button_padding;
    let mut col = 0;

    for tool in Tool::all() {
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(x, y),
            egui::vec2(button_size, button_size),
        );

        // Check if this is the selected tool
        let is_selected = *selected_tool == *tool;

        // Button background
        let bg_color = if is_selected {
            egui::Color32::from_rgb(70, 100, 150) // Highlighted blue
        } else {
            egui::Color32::from_rgb(50, 50, 50)
        };
        ui.painter().rect_filled(button_rect, 4.0, bg_color);

        // Load and render tool icon
        if let Some(icon) = tool_icon_cache.get_or_load(*tool, ui.ctx()) {
            let icon_rect = button_rect.shrink(8.0); // Padding inside button
            ui.painter().image(
                icon.id(),
                icon_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Make button interactive (include path to ensure unique IDs across panes)
        let button_id = ui.id().with(("tool_button", path, *tool as usize));
        let response = ui.interact(button_rect, button_id, egui::Sense::click());

        // Check for click first
        if response.clicked() {
            *selected_tool = *tool;
        }

        if response.hovered() {
            ui.painter().rect_stroke(
                button_rect,
                4.0,
                egui::Stroke::new(2.0, egui::Color32::from_gray(180)),
                egui::StrokeKind::Middle,
            );
        }

        // Show tooltip with tool name and shortcut (consumes response)
        response.on_hover_text(format!("{} ({})", tool.display_name(), tool.shortcut_hint()));

        // Draw selection border
        if is_selected {
            ui.painter().rect_stroke(
                button_rect,
                4.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(100, 150, 255)),
                egui::StrokeKind::Middle,
            );
        }

        // Move to next position in grid
        col += 1;
        if col >= columns {
            // Move to next row
            col = 0;
            x = rect.left() + button_padding;
            y += button_size + button_spacing;
        } else {
            // Move to next column
            x += button_size + button_spacing;
        }
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
        PaneType::NodeEditor => egui::Color32::from_rgb(30, 45, 50),
        PaneType::PresetBrowser => egui::Color32::from_rgb(50, 45, 30),
        PaneType::AssetLibrary => egui::Color32::from_rgb(45, 50, 35),
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
