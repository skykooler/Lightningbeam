/// Pane implementations for the editor
///
/// Each pane type has its own module with implementation details.
/// Panes can hold local state and access shared state through SharedPaneState.

use eframe::egui;
use lightningbeam_core::{pane::PaneType, tool::Tool};

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

pub mod toolbar;
pub mod stage;
pub mod timeline;
pub mod infopanel;
pub mod outliner;
pub mod piano_roll;
pub mod node_editor;
pub mod preset_browser;

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

/// Shared state that all panes can access
pub struct SharedPaneState<'a> {
    pub tool_icon_cache: &'a mut crate::ToolIconCache,
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
    fn render_header(&mut self, ui: &mut egui::Ui, shared: &mut SharedPaneState) -> bool {
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
    NodeEditor(node_editor::NodeEditorPane),
    PresetBrowser(preset_browser::PresetBrowserPane),
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
            PaneType::NodeEditor => PaneInstance::NodeEditor(node_editor::NodeEditorPane::new()),
            PaneType::PresetBrowser => {
                PaneInstance::PresetBrowser(preset_browser::PresetBrowserPane::new())
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
            PaneInstance::NodeEditor(_) => PaneType::NodeEditor,
            PaneInstance::PresetBrowser(_) => PaneType::PresetBrowser,
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
            PaneInstance::NodeEditor(p) => p.render_header(ui, shared),
            PaneInstance::PresetBrowser(p) => p.render_header(ui, shared),
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
            PaneInstance::NodeEditor(p) => p.render_content(ui, rect, path, shared),
            PaneInstance::PresetBrowser(p) => p.render_content(ui, rect, path, shared),
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
            PaneInstance::NodeEditor(p) => p.name(),
            PaneInstance::PresetBrowser(p) => p.name(),
        }
    }
}
