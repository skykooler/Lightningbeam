/// Pane implementations for the editor
///
/// Each pane type has its own module with implementation details.
/// Panes can hold local state and access shared state through SharedPaneState.

use eframe::egui;
use lightningbeam_core::{pane::PaneType, tool::Tool};
use uuid::Uuid;

// Type alias for node paths (matches main.rs)
pub type NodePath = Vec<usize>;

/// Handler information for view actions (zoom, pan, etc.)
/// Used for two-phase dispatch: register during render, execute after
#[derive(Clone)]
pub struct ViewActionHandler {
    pub priority: u32,
    pub pane_path: NodePath,
    pub zoom_center: egui::Vec2,
}

/// Clip type for drag-and-drop operations
/// Distinguishes between different clip/layer type combinations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragClipType {
    /// Vector animation clip
    Vector,
    /// Video clip
    Video,
    /// Sampled audio clip (WAV, MP3, etc.)
    AudioSampled,
    /// MIDI clip
    AudioMidi,
    /// Static image asset
    Image,
    /// Effect (shader-based visual effect)
    Effect,
}

/// Information about an asset being dragged from the Asset Library
#[derive(Debug, Clone)]
pub struct DraggingAsset {
    /// The clip ID being dragged
    pub clip_id: Uuid,
    /// Type of clip (determines compatible layer types)
    pub clip_type: DragClipType,
    /// Display name
    pub name: String,
    /// Duration in seconds
    #[allow(dead_code)] // Populated during drag, consumed when drag-and-drop features expand
    pub duration: f64,
    /// Dimensions (width, height) for vector/video clips, None for audio
    pub dimensions: Option<(f64, f64)>,
    /// Optional linked audio clip ID (for video clips with extracted audio)
    pub linked_audio_clip_id: Option<Uuid>,
}

/// Command for webcam recording (issued by timeline, processed by main)
#[derive(Debug)]
#[allow(dead_code)]
pub enum WebcamRecordCommand {
    /// Start recording on the given video layer
    // TODO: remove layer_id — recording_layer_ids now tracks which layers are recording
    Start { layer_id: uuid::Uuid },
    /// Stop current webcam recording
    Stop,
}

pub mod toolbar;
pub mod stage;
pub mod gradient_editor;
pub mod timeline;
pub mod infopanel;
pub mod outliner;
pub mod piano_roll;
pub mod virtual_piano;
pub mod node_editor;
pub mod node_graph;
pub mod preset_browser;
pub mod asset_library;
pub mod shader_editor;

/// Which color mode is active for the eyedropper tool
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Fill,
    Stroke,
}

impl Default for ColorMode {
    fn default() -> Self {
        ColorMode::Fill
    }
}

/// Helper functions for layer/clip type matching and creation

/// Check if a clip type matches a layer type
pub fn layer_matches_clip_type(layer: &lightningbeam_core::layer::AnyLayer, clip_type: DragClipType) -> bool {
    use lightningbeam_core::layer::*;
    match (layer, clip_type) {
        (AnyLayer::Vector(_), DragClipType::Vector) => true,
        (AnyLayer::Vector(_), DragClipType::Image) => true, // Images go on vector layers as shapes
        (AnyLayer::Video(_), DragClipType::Video) => true,
        (AnyLayer::Audio(audio), DragClipType::AudioSampled) => {
            audio.audio_layer_type == AudioLayerType::Sampled
        }
        (AnyLayer::Audio(audio), DragClipType::AudioMidi) => {
            audio.audio_layer_type == AudioLayerType::Midi
        }
        (AnyLayer::Effect(_), DragClipType::Effect) => true,
        _ => false,
    }
}

/// Create a new layer of the appropriate type for a clip
pub fn create_layer_for_clip_type(clip_type: DragClipType, name: &str) -> lightningbeam_core::layer::AnyLayer {
    use lightningbeam_core::layer::*;
    use lightningbeam_core::effect_layer::EffectLayer;
    match clip_type {
        DragClipType::Vector => AnyLayer::Vector(VectorLayer::new(name)),
        DragClipType::Video => AnyLayer::Video(VideoLayer::new(name)),
        DragClipType::AudioSampled => AnyLayer::Audio(AudioLayer::new_sampled(name)),
        DragClipType::AudioMidi => AnyLayer::Audio(AudioLayer::new_midi(name)),
        // Images are placed as shapes on vector layers, not their own layer type
        DragClipType::Image => AnyLayer::Vector(VectorLayer::new(name)),
        DragClipType::Effect => AnyLayer::Effect(EffectLayer::new(name)),
    }
}

/// Find an existing sampled audio track in the document
/// Returns the layer ID if found, None otherwise
pub fn find_sampled_audio_track(document: &lightningbeam_core::document::Document) -> Option<uuid::Uuid> {
    use lightningbeam_core::layer::*;
    for layer in &document.root.children {
        if let AnyLayer::Audio(audio_layer) = layer {
            if audio_layer.audio_layer_type == AudioLayerType::Sampled {
                return Some(audio_layer.layer.id);
            }
        }
    }
    None
}

/// Shared state that all panes can access
pub struct SharedPaneState<'a> {
    pub tool_icon_cache: &'a mut crate::ToolIconCache,
    #[allow(dead_code)] // Used by pane chrome rendering in main.rs
    pub icon_cache: &'a mut crate::IconCache,
    pub selected_tool: &'a mut Tool,
    pub fill_color: &'a mut egui::Color32,
    pub stroke_color: &'a mut egui::Color32,
    /// Tracks which color (fill or stroke) was last interacted with, for eyedropper tool
    pub active_color_mode: &'a mut ColorMode,
    pub pending_view_action: &'a mut Option<crate::menu::MenuAction>,
    /// Tracks the priority of the best fallback pane for view actions
    /// Lower number = higher priority. None = no fallback pane seen yet
    /// Priority order: Stage(0) > Timeline(1) > PianoRoll(2) > NodeEditor(3)
    pub fallback_pane_priority: &'a mut Option<u32>,
    pub theme: &'a crate::theme::Theme,
    /// Registry of handlers for the current pending action
    /// Panes register themselves here during render, execution happens after
    pub pending_handlers: &'a mut Vec<ViewActionHandler>,
    /// Action executor for immediate action execution (for shape tools to avoid flicker)
    /// Also provides read-only access to the document via action_executor.document()
    pub action_executor: &'a mut lightningbeam_core::action::ActionExecutor,
    /// Current selection state (mutable for tools to modify)
    pub selection: &'a mut lightningbeam_core::selection::Selection,
    /// Document-level focus: tracks the most recently selected thing(s) of any type
    pub focus: &'a mut lightningbeam_core::selection::FocusSelection,
    /// Which VectorClip is being edited (None = document root)
    pub editing_clip_id: Option<uuid::Uuid>,
    /// The clip instance ID being edited
    pub editing_instance_id: Option<uuid::Uuid>,
    /// The parent layer ID containing the clip instance being edited
    pub editing_parent_layer_id: Option<uuid::Uuid>,
    /// Request to enter a movie clip for editing: (clip_id, instance_id, parent_layer_id)
    pub pending_enter_clip: &'a mut Option<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    /// Request to exit the current movie clip
    pub pending_exit_clip: &'a mut bool,
    /// Currently active layer ID
    pub active_layer_id: &'a mut Option<uuid::Uuid>,
    /// Current tool interaction state (mutable for tools to modify)
    pub tool_state: &'a mut lightningbeam_core::tool::ToolState,
    /// Actions to execute after rendering completes (two-phase dispatch)
    pub pending_actions: &'a mut Vec<Box<dyn lightningbeam_core::action::Action>>,
    /// Draw tool configuration
    pub draw_simplify_mode: &'a mut lightningbeam_core::tool::SimplifyMode,
    pub rdp_tolerance: &'a mut f64,
    pub schneider_max_error: &'a mut f64,
    /// All per-tool raster paint settings (replaces 20+ individual fields).
    pub raster_settings: &'a mut crate::tools::RasterToolSettings,
    /// Audio engine controller for playback control (wrapped in Arc<Mutex<>> for thread safety)
    pub audio_controller: Option<&'a std::sync::Arc<std::sync::Mutex<daw_backend::EngineController>>>,
    /// Snapshot of all audio/MIDI clip instances from the backend (for timeline rendering).
    /// Updated by the audio thread after each mutation; UI reads it each frame.
    pub clip_snapshot: Option<std::sync::Arc<std::sync::RwLock<daw_backend::AudioClipSnapshot>>>,
    /// Opener for the microphone/line-in stream — consumed on first use.
    pub audio_input_opener: &'a mut Option<daw_backend::InputStreamOpener>,
    /// Live input stream handle; kept alive while recording is active.
    pub audio_input_stream: &'a mut Option<cpal::Stream>,
    /// Buffer size (frames) used for the output stream, passed to the input stream opener.
    pub audio_buffer_size: u32,
    /// Video manager for video decoding and frame caching
    pub video_manager: &'a std::sync::Arc<std::sync::Mutex<lightningbeam_core::video::VideoManager>>,
    /// Maps all layer/group/clip UUIDs to backend track IDs (audio, MIDI, and metatracks)
    pub layer_to_track_map: &'a std::collections::HashMap<Uuid, daw_backend::TrackId>,
    /// Maps document clip instance UUIDs to backend clip instance IDs (for action dispatch)
    pub clip_instance_to_backend_map: &'a std::collections::HashMap<Uuid, lightningbeam_core::action::BackendClipInstanceId>,
    /// Global playback state
    pub playback_time: &'a mut f64,  // Current playback position in seconds
    pub is_playing: &'a mut bool,    // Whether playback is currently active
    /// Recording state
    pub is_recording: &'a mut bool,  // Whether recording is currently active
    pub recording_clips: &'a mut std::collections::HashMap<uuid::Uuid, u32>, // layer_id -> clip_id
    pub recording_start_time: &'a mut f64,  // Playback time when recording started
    pub recording_layer_ids: &'a mut Vec<uuid::Uuid>,  // Layers being recorded to
    /// Asset being dragged from Asset Library (for cross-pane drag-and-drop)
    pub dragging_asset: &'a mut Option<DraggingAsset>,
    // Tool-specific options for infopanel
    /// Stroke width for drawing tools (Draw, Rectangle, Ellipse, Line, Polygon)
    pub stroke_width: &'a mut f64,
    /// Whether to fill shapes when drawing (Rectangle, Ellipse, Polygon)
    pub fill_enabled: &'a mut bool,
    /// Whether to snap to geometry when editing vectors
    pub snap_enabled: &'a mut bool,
    /// Fill gap tolerance for paint bucket tool
    pub paint_bucket_gap_tolerance: &'a mut f64,
    /// Number of sides for polygon tool
    pub polygon_sides: &'a mut u32,
    /// Cache of MIDI events for rendering (keyed by backend midi_clip_id).
    /// Mutable so panes can update the cache immediately on edits (avoiding 1-frame snap-back).
    /// NOTE: If an action later fails during execution, the cache may be out of sync with the
    /// backend. This is acceptable because MIDI note edits are simple and unlikely to fail.
    /// Undo/redo rebuilds affected entries from the backend to restore consistency.
    pub midi_event_cache: &'a mut std::collections::HashMap<u32, Vec<daw_backend::audio::midi::MidiEvent>>,
    /// Audio pool indices that got new raw audio data this frame (for thumbnail invalidation)
    pub audio_pools_with_new_waveforms: &'a std::collections::HashSet<usize>,
    /// Raw audio samples for GPU waveform rendering (pool_index -> (samples, sample_rate, channels))
    pub raw_audio_cache: &'a std::collections::HashMap<usize, (std::sync::Arc<Vec<f32>>, u32, u32)>,
    /// Pool indices needing GPU waveform texture upload
    pub waveform_gpu_dirty: &'a mut std::collections::HashSet<usize>,
    /// Effect ID to load into shader editor (set by asset library, consumed by shader editor)
    pub effect_to_load: &'a mut Option<Uuid>,
    /// Queue for effect thumbnail requests (effect IDs to generate thumbnails for)
    pub effect_thumbnail_requests: &'a mut Vec<Uuid>,
    /// Cache of generated effect thumbnails (effect_id -> RGBA data)
    pub effect_thumbnail_cache: &'a std::collections::HashMap<Uuid, Vec<u8>>,
    /// Effect IDs whose thumbnails should be invalidated (e.g., after shader edit)
    pub effect_thumbnails_to_invalidate: &'a mut Vec<Uuid>,
    /// Latest webcam capture frame (None if no camera is active)
    pub webcam_frame: Option<lightningbeam_core::webcam::CaptureFrame>,
    /// Pending webcam recording commands (processed by main.rs after render)
    pub webcam_record_command: &'a mut Option<WebcamRecordCommand>,
    /// Surface texture format for GPU rendering (Rgba8Unorm or Bgra8Unorm depending on platform)
    pub target_format: wgpu::TextureFormat,
    /// Menu actions queued by panes (e.g. context menu items), processed by main after rendering
    pub pending_menu_actions: &'a mut Vec<crate::menu::MenuAction>,
    /// Clipboard manager for cut/copy/paste operations
    pub clipboard_manager: &'a mut lightningbeam_core::clipboard::ClipboardManager,
    // VU meter levels
    pub input_level: f32,
    pub output_level: (f32, f32),
    pub track_levels: &'a std::collections::HashMap<daw_backend::TrackId, f32>,
    #[allow(dead_code)] // Available for panes that need reverse track->layer lookup
    pub track_to_layer_map: &'a std::collections::HashMap<daw_backend::TrackId, Uuid>,
    /// Whether to show waveforms as stacked stereo (true) or combined mono (false)
    pub waveform_stereo: bool,
    /// Generation counter - incremented on project load to force reloads
    pub project_generation: &'a mut u64,
    /// Incremented whenever node graph topology changes (add/remove node or connection).
    /// Used by the timeline to know when to refresh automation lane caches.
    pub graph_topology_generation: &'a mut u64,
    /// Script ID to open in the script editor (set by node graph "Edit Script" action)
    pub script_to_edit: &'a mut Option<Uuid>,
    /// Script ID that was just saved (triggers auto-recompile of nodes using it)
    pub script_saved: &'a mut Option<Uuid>,
    /// Active region selection (temporary split state)
    pub region_selection: &'a mut Option<lightningbeam_core::selection::RegionSelection>,
    /// Region select mode (Rectangle or Lasso)
    pub region_select_mode: &'a mut lightningbeam_core::tool::RegionSelectMode,
    /// Lasso select sub-mode (Freehand / Polygonal / Magnetic)
    pub lasso_mode: &'a mut lightningbeam_core::tool::LassoMode,
    /// Counter for in-flight graph preset loads — increment when sending a
    /// GraphLoadPreset command so the repaint loop stays alive until the
    /// audio thread sends GraphPresetLoaded back
    pub pending_graph_loads: &'a std::sync::Arc<std::sync::atomic::AtomicU32>,
    /// Set by panes (e.g. piano roll) when they handle Ctrl+C/X/V internally,
    /// so main.rs skips its own clipboard handling for the current frame
    pub clipboard_consumed: &'a mut bool,
    /// Remappable keyboard shortcut manager
    pub keymap: &'a crate::keymap::KeymapManager,
    /// Set by raster selection tools when they need main to commit the floating
    /// selection before starting a new interaction.
    pub commit_raster_floating_if_any: &'a mut bool,
    /// Set by MenuAction::Group when focus is Nodes — consumed by node graph pane
    pub pending_node_group: &'a mut bool,
    /// Set by MenuAction::Group (ungroup variant) when focus is Nodes — consumed by node graph pane
    pub pending_node_ungroup: &'a mut bool,
    /// Test mode state for event recording (debug builds only)
    #[cfg(debug_assertions)]
    pub test_mode: &'a mut crate::test_mode::TestModeState,
    /// Synthetic input from test mode replay (debug builds only)
    #[cfg(debug_assertions)]
    pub synthetic_input: &'a mut Option<crate::test_mode::SyntheticInput>,
    /// GPU-rendered brush preview thumbnails.  Populated by `VelloCallback::prepare()`
    /// on the first frame; panes (e.g. infopanel) convert the pixel data to egui
    /// TextureHandles.  Each entry is `(width, height, sRGB-premultiplied RGBA bytes)`.
    pub brush_preview_pixels: &'a std::sync::Arc<std::sync::Mutex<Vec<(u32, u32, Vec<u8>)>>>,
}

/// Trait for pane rendering
///
/// Panes implement this trait to provide custom rendering logic.
/// The header is optional and typically used for controls (e.g., Timeline playback).
/// The content area is the main body of the pane.
pub trait PaneRenderer {
    /// Render the optional header section with controls
    ///
    /// Returns true if a header was rendered, false if no header
    fn render_header(&mut self, _ui: &mut egui::Ui, _shared: &mut SharedPaneState) -> bool {
        false // Default: no header
    }

    /// Render the main content area
    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    );

    /// Get the display name of this pane
    #[allow(dead_code)] // Implemented by all panes, dispatch infrastructure complete
    fn name(&self) -> &str;
}

/// Enum wrapper for all pane implementations (enum dispatch pattern)
pub enum PaneInstance {
    Stage(stage::StagePane),
    Timeline(timeline::TimelinePane),
    Toolbar(toolbar::ToolbarPane),
    Infopanel(infopanel::InfopanelPane),
    Outliner(outliner::OutlinerPane),
    PianoRoll(piano_roll::PianoRollPane),
    VirtualPiano(virtual_piano::VirtualPianoPane),
    NodeEditor(node_editor::NodeEditorPane),
    PresetBrowser(preset_browser::PresetBrowserPane),
    AssetLibrary(asset_library::AssetLibraryPane),
    ScriptEditor(shader_editor::ShaderEditorPane),
}

impl PaneInstance {
    /// Create a new pane instance for the given type
    pub fn new(pane_type: PaneType) -> Self {
        match pane_type {
            PaneType::Stage => PaneInstance::Stage(stage::StagePane::new()),
            PaneType::Timeline => PaneInstance::Timeline(timeline::TimelinePane::new()),
            PaneType::Toolbar => PaneInstance::Toolbar(toolbar::ToolbarPane::new()),
            PaneType::Infopanel => PaneInstance::Infopanel(infopanel::InfopanelPane::new()),
            PaneType::Outliner => PaneInstance::Outliner(outliner::OutlinerPane::new()),
            PaneType::PianoRoll => PaneInstance::PianoRoll(piano_roll::PianoRollPane::new()),
            PaneType::VirtualPiano => PaneInstance::VirtualPiano(virtual_piano::VirtualPianoPane::new()),
            PaneType::NodeEditor => PaneInstance::NodeEditor(node_editor::NodeEditorPane::new()),
            PaneType::PresetBrowser => {
                PaneInstance::PresetBrowser(preset_browser::PresetBrowserPane::new())
            }
            PaneType::AssetLibrary => {
                PaneInstance::AssetLibrary(asset_library::AssetLibraryPane::new())
            }
            PaneType::ScriptEditor => {
                PaneInstance::ScriptEditor(shader_editor::ShaderEditorPane::new())
            }
        }
    }

    /// Get the pane type of this instance
    pub fn pane_type(&self) -> PaneType {
        match self {
            PaneInstance::Stage(_) => PaneType::Stage,
            PaneInstance::Timeline(_) => PaneType::Timeline,
            PaneInstance::Toolbar(_) => PaneType::Toolbar,
            PaneInstance::Infopanel(_) => PaneType::Infopanel,
            PaneInstance::Outliner(_) => PaneType::Outliner,
            PaneInstance::PianoRoll(_) => PaneType::PianoRoll,
            PaneInstance::VirtualPiano(_) => PaneType::VirtualPiano,
            PaneInstance::NodeEditor(_) => PaneType::NodeEditor,
            PaneInstance::PresetBrowser(_) => PaneType::PresetBrowser,
            PaneInstance::AssetLibrary(_) => PaneType::AssetLibrary,
            PaneInstance::ScriptEditor(_) => PaneType::ScriptEditor,
        }
    }
}

impl PaneRenderer for PaneInstance {
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
        match self {
            PaneInstance::Stage(p) => p.render_header(ui, shared),
            PaneInstance::Timeline(p) => p.render_header(ui, shared),
            PaneInstance::Toolbar(p) => p.render_header(ui, shared),
            PaneInstance::Infopanel(p) => p.render_header(ui, shared),
            PaneInstance::Outliner(p) => p.render_header(ui, shared),
            PaneInstance::PianoRoll(p) => p.render_header(ui, shared),
            PaneInstance::VirtualPiano(p) => p.render_header(ui, shared),
            PaneInstance::NodeEditor(p) => p.render_header(ui, shared),
            PaneInstance::PresetBrowser(p) => p.render_header(ui, shared),
            PaneInstance::AssetLibrary(p) => p.render_header(ui, shared),
            PaneInstance::ScriptEditor(p) => p.render_header(ui, shared),
        }
    }

    fn render_content(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        path: &NodePath,
        shared: &mut SharedPaneState,
    ) {
        match self {
            PaneInstance::Stage(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::Timeline(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::Toolbar(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::Infopanel(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::Outliner(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::PianoRoll(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::VirtualPiano(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::NodeEditor(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::PresetBrowser(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::AssetLibrary(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::ScriptEditor(p) => p.render_content(ui, rect, path, shared),
        }
    }

    fn name(&self) -> &str {
        match self {
            PaneInstance::Stage(p) => p.name(),
            PaneInstance::Timeline(p) => p.name(),
            PaneInstance::Toolbar(p) => p.name(),
            PaneInstance::Infopanel(p) => p.name(),
            PaneInstance::Outliner(p) => p.name(),
            PaneInstance::PianoRoll(p) => p.name(),
            PaneInstance::VirtualPiano(p) => p.name(),
            PaneInstance::NodeEditor(p) => p.name(),
            PaneInstance::PresetBrowser(p) => p.name(),
            PaneInstance::AssetLibrary(p) => p.name(),
            PaneInstance::ScriptEditor(p) => p.name(),
        }
    }
}
