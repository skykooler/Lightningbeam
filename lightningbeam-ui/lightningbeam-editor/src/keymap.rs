//! Remappable keyboard shortcuts system
//!
//! Provides a unified `AppAction` enum for all bindable actions, a `KeymapManager`
//! for runtime shortcut lookup, and `KeybindingConfig` for persistent storage of
//! user overrides.

use std::collections::HashMap;
use eframe::egui;
use serde::{Serialize, Deserialize};
use crate::menu::{MenuAction, Shortcut, ShortcutKey};

/// Unified enum of every bindable action in the application.
///
/// Excludes virtual piano keys (keyboard-layout-dependent, not user-preference shortcuts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AppAction {
    // === File menu ===
    NewFile,
    NewWindow,
    Save,
    SaveAs,
    OpenFile,
    Revert,
    Import,
    ImportToLibrary,
    Export,
    Quit,

    // === Edit menu ===
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    SelectNone,
    Preferences,

    // === Modify menu ===
    Group,
    ConvertToMovieClip,
    SendToBack,
    BringToFront,
    SplitClip,
    DuplicateClip,

    // === Layer menu ===
    AddLayer,
    AddVideoLayer,
    AddAudioTrack,
    AddMidiTrack,
    AddRasterLayer,
    AddTestClip,
    DeleteLayer,
    ToggleLayerVisibility,

    // === Timeline menu ===
    NewKeyframe,
    NewBlankKeyframe,
    DeleteFrame,
    DuplicateKeyframe,
    AddKeyframeAtPlayhead,
    AddMotionTween,
    AddShapeTween,
    ReturnToStart,
    Play,

    // === View menu ===
    ZoomIn,
    ZoomOut,
    ActualSize,
    RecenterView,
    NextLayout,
    PreviousLayout,

    // === Help ===
    About,

    // === macOS / Window ===
    Settings,
    CloseWindow,

    // === Tool shortcuts (no modifiers) ===
    ToolSelect,
    ToolDraw,
    ToolTransform,
    ToolRectangle,
    ToolEllipse,
    ToolPaintBucket,
    ToolEyedropper,
    ToolLine,
    ToolPolygon,
    ToolBezierEdit,
    ToolText,
    ToolRegionSelect,
    ToolErase,
    ToolSmudge,
    ToolSelectLasso,
    ToolSplit,

    // === Global shortcuts ===
    TogglePlayPause,
    CancelAction,
    ToggleDebugOverlay,
    ToggleOnionSkin,
    #[cfg(debug_assertions)]
    ToggleTestMode,

    // === Pane-local shortcuts ===
    PianoRollDelete,
    StageDelete,
    NodeGraphGroup,
    NodeGraphUngroup,
    NodeGraphRename,
}

impl AppAction {
    /// Category name for grouping in the preferences UI
    pub fn category(&self) -> &'static str {
        match self {
            Self::NewFile | Self::NewWindow | Self::Save | Self::SaveAs |
            Self::OpenFile | Self::Revert | Self::Import | Self::ImportToLibrary |
            Self::Export | Self::Quit => "File",

            Self::Undo | Self::Redo | Self::Cut | Self::Copy | Self::Paste |
            Self::Delete | Self::SelectAll | Self::SelectNone | Self::Preferences => "Edit",

            Self::Group | Self::ConvertToMovieClip | Self::SendToBack |
            Self::BringToFront | Self::SplitClip | Self::DuplicateClip => "Modify",

            Self::AddLayer | Self::AddVideoLayer | Self::AddAudioTrack |
            Self::AddMidiTrack | Self::AddRasterLayer | Self::AddTestClip | Self::DeleteLayer |
            Self::ToggleLayerVisibility => "Layer",

            Self::NewKeyframe | Self::NewBlankKeyframe | Self::DeleteFrame |
            Self::DuplicateKeyframe | Self::AddKeyframeAtPlayhead |
            Self::AddMotionTween | Self::AddShapeTween |
            Self::ReturnToStart | Self::Play => "Timeline",

            Self::ZoomIn | Self::ZoomOut | Self::ActualSize | Self::RecenterView |
            Self::NextLayout | Self::PreviousLayout => "View",

            Self::About => "Help",
            Self::Settings | Self::CloseWindow => "Window",

            Self::ToolSelect | Self::ToolDraw | Self::ToolTransform |
            Self::ToolRectangle | Self::ToolEllipse | Self::ToolPaintBucket |
            Self::ToolEyedropper | Self::ToolLine | Self::ToolPolygon |
            Self::ToolBezierEdit | Self::ToolText | Self::ToolRegionSelect |
            Self::ToolErase | Self::ToolSmudge | Self::ToolSelectLasso | Self::ToolSplit => "Tools",

            Self::TogglePlayPause | Self::CancelAction |
            Self::ToggleDebugOverlay | Self::ToggleOnionSkin => "Global",
            #[cfg(debug_assertions)]
            Self::ToggleTestMode => "Global",

            Self::PianoRollDelete | Self::StageDelete |
            Self::NodeGraphGroup | Self::NodeGraphUngroup |
            Self::NodeGraphRename => "Pane",
        }
    }

    /// Conflict scope: actions can only conflict with other actions in the same scope.
    /// Pane-local actions each get their own scope (they're isolated to their pane),
    /// everything else shares the "global" scope.
    pub fn conflict_scope(&self) -> &'static str {
        match self {
            Self::PianoRollDelete => "pane:piano_roll",
            Self::StageDelete => "pane:stage",
            Self::NodeGraphGroup | Self::NodeGraphUngroup |
            Self::NodeGraphRename => "pane:node_graph",
            _ => "global",
        }
    }

    /// Human-readable display name for the preferences UI
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::NewFile => "New File",
            Self::NewWindow => "New Window",
            Self::Save => "Save",
            Self::SaveAs => "Save As",
            Self::OpenFile => "Open File",
            Self::Revert => "Revert",
            Self::Import => "Import",
            Self::ImportToLibrary => "Import to Library",
            Self::Export => "Export",
            Self::Quit => "Quit",
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::Delete => "Delete",
            Self::SelectAll => "Select All",
            Self::SelectNone => "Select None",
            Self::Preferences => "Preferences",
            Self::Group => "Group",
            Self::ConvertToMovieClip => "Convert to Movie Clip",
            Self::SendToBack => "Send to Back",
            Self::BringToFront => "Bring to Front",
            Self::SplitClip => "Split Clip",
            Self::DuplicateClip => "Duplicate Clip",
            Self::AddLayer => "Add Layer",
            Self::AddVideoLayer => "Add Video Layer",
            Self::AddAudioTrack => "Add Audio Track",
            Self::AddMidiTrack => "Add MIDI Track",
            Self::AddRasterLayer => "Add Raster Layer",
            Self::AddTestClip => "Add Test Clip",
            Self::DeleteLayer => "Delete Layer",
            Self::ToggleLayerVisibility => "Toggle Layer Visibility",
            Self::NewKeyframe => "New Keyframe",
            Self::NewBlankKeyframe => "New Blank Keyframe",
            Self::DeleteFrame => "Delete Frame",
            Self::DuplicateKeyframe => "Duplicate Keyframe",
            Self::AddKeyframeAtPlayhead => "Add Keyframe at Playhead",
            Self::AddMotionTween => "Add Motion Tween",
            Self::AddShapeTween => "Add Shape Tween",
            Self::ReturnToStart => "Return to Start",
            Self::Play => "Play",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::ActualSize => "Actual Size",
            Self::RecenterView => "Recenter View",
            Self::NextLayout => "Next Layout",
            Self::PreviousLayout => "Previous Layout",
            Self::About => "About",
            Self::Settings => "Settings",
            Self::CloseWindow => "Close Window",
            Self::ToolSelect => "Select Tool",
            Self::ToolDraw => "Draw Tool",
            Self::ToolTransform => "Transform Tool",
            Self::ToolRectangle => "Rectangle Tool",
            Self::ToolEllipse => "Ellipse Tool",
            Self::ToolPaintBucket => "Paint Bucket Tool",
            Self::ToolEyedropper => "Eyedropper Tool",
            Self::ToolLine => "Line Tool",
            Self::ToolPolygon => "Polygon Tool",
            Self::ToolBezierEdit => "Bezier Edit Tool",
            Self::ToolText => "Text Tool",
            Self::ToolRegionSelect => "Region Select Tool",
            Self::ToolErase => "Erase Tool",
            Self::ToolSmudge => "Smudge Tool",
            Self::ToolSelectLasso => "Lasso Select Tool",
            Self::ToolSplit => "Split Tool",
            Self::TogglePlayPause => "Toggle Play/Pause",
            Self::CancelAction => "Cancel / Escape",
            Self::ToggleDebugOverlay => "Toggle Debug Overlay",
            Self::ToggleOnionSkin => "Toggle Onion Skinning",
            #[cfg(debug_assertions)]
            Self::ToggleTestMode => "Toggle Test Mode",
            Self::PianoRollDelete => "Piano Roll: Delete",
            Self::StageDelete => "Stage: Delete",
            Self::NodeGraphGroup => "Node Graph: Group",
            Self::NodeGraphUngroup => "Node Graph: Ungroup",
            Self::NodeGraphRename => "Node Graph: Rename",
        }
    }

    /// All action variants (for iteration)
    pub fn all() -> &'static [AppAction] {
        &[
            Self::NewFile, Self::NewWindow, Self::Save, Self::SaveAs,
            Self::OpenFile, Self::Revert, Self::Import, Self::ImportToLibrary,
            Self::Export, Self::Quit,
            Self::Undo, Self::Redo, Self::Cut, Self::Copy, Self::Paste,
            Self::Delete, Self::SelectAll, Self::SelectNone, Self::Preferences,
            Self::Group, Self::ConvertToMovieClip, Self::SendToBack,
            Self::BringToFront, Self::SplitClip, Self::DuplicateClip,
            Self::AddLayer, Self::AddVideoLayer, Self::AddAudioTrack,
            Self::AddMidiTrack, Self::AddTestClip, Self::DeleteLayer,
            Self::ToggleLayerVisibility,
            Self::NewKeyframe, Self::NewBlankKeyframe, Self::DeleteFrame,
            Self::DuplicateKeyframe, Self::AddKeyframeAtPlayhead,
            Self::AddMotionTween, Self::AddShapeTween,
            Self::ReturnToStart, Self::Play,
            Self::ZoomIn, Self::ZoomOut, Self::ActualSize, Self::RecenterView,
            Self::NextLayout, Self::PreviousLayout,
            Self::About, Self::Settings, Self::CloseWindow,
            Self::ToolSelect, Self::ToolDraw, Self::ToolTransform,
            Self::ToolRectangle, Self::ToolEllipse, Self::ToolPaintBucket,
            Self::ToolEyedropper, Self::ToolLine, Self::ToolPolygon,
            Self::ToolBezierEdit, Self::ToolText, Self::ToolRegionSelect,
            Self::ToolErase, Self::ToolSmudge, Self::ToolSelectLasso, Self::ToolSplit,
            Self::TogglePlayPause, Self::CancelAction, Self::ToggleDebugOverlay, Self::ToggleOnionSkin,
            #[cfg(debug_assertions)]
            Self::ToggleTestMode,
            Self::PianoRollDelete, Self::StageDelete,
            Self::NodeGraphGroup, Self::NodeGraphUngroup, Self::NodeGraphRename,
        ]
    }
}

// === Conversions between MenuAction and AppAction ===

impl From<MenuAction> for AppAction {
    fn from(action: MenuAction) -> Self {
        match action {
            MenuAction::NewFile => Self::NewFile,
            MenuAction::NewWindow => Self::NewWindow,
            MenuAction::Save => Self::Save,
            MenuAction::SaveAs => Self::SaveAs,
            MenuAction::OpenFile => Self::OpenFile,
            MenuAction::OpenRecent(_) => Self::OpenFile, // not directly mappable
            MenuAction::ClearRecentFiles => Self::OpenFile, // not directly mappable
            MenuAction::Revert => Self::Revert,
            MenuAction::Import => Self::Import,
            MenuAction::ImportToLibrary => Self::ImportToLibrary,
            MenuAction::Export => Self::Export,
            MenuAction::Quit => Self::Quit,
            MenuAction::Undo => Self::Undo,
            MenuAction::Redo => Self::Redo,
            MenuAction::Cut => Self::Cut,
            MenuAction::Copy => Self::Copy,
            MenuAction::Paste => Self::Paste,
            MenuAction::Delete => Self::Delete,
            MenuAction::SelectAll => Self::SelectAll,
            MenuAction::SelectNone => Self::SelectNone,
            MenuAction::Preferences => Self::Preferences,
            MenuAction::Group => Self::Group,
            MenuAction::ConvertToMovieClip => Self::ConvertToMovieClip,
            MenuAction::SendToBack => Self::SendToBack,
            MenuAction::BringToFront => Self::BringToFront,
            MenuAction::SplitClip => Self::SplitClip,
            MenuAction::DuplicateClip => Self::DuplicateClip,
            MenuAction::AddLayer => Self::AddLayer,
            MenuAction::AddVideoLayer => Self::AddVideoLayer,
            MenuAction::AddAudioTrack => Self::AddAudioTrack,
            MenuAction::AddMidiTrack => Self::AddMidiTrack,
            MenuAction::AddRasterLayer => Self::AddRasterLayer,
            MenuAction::AddTestClip => Self::AddTestClip,
            MenuAction::DeleteLayer => Self::DeleteLayer,
            MenuAction::ToggleLayerVisibility => Self::ToggleLayerVisibility,
            MenuAction::ToggleOnionSkin => Self::ToggleOnionSkin,
            MenuAction::ShowMasterTrack => Self::ToggleLayerVisibility, // not directly mappable
            MenuAction::NewKeyframe => Self::NewKeyframe,
            MenuAction::NewBlankKeyframe => Self::NewBlankKeyframe,
            MenuAction::DeleteFrame => Self::DeleteFrame,
            MenuAction::DuplicateKeyframe => Self::DuplicateKeyframe,
            MenuAction::AddKeyframeAtPlayhead => Self::AddKeyframeAtPlayhead,
            MenuAction::AddMotionTween => Self::AddMotionTween,
            MenuAction::AddShapeTween => Self::AddShapeTween,
            MenuAction::ReturnToStart => Self::ReturnToStart,
            MenuAction::Play => Self::Play,
            MenuAction::ToggleCountIn => Self::Play, // not directly mappable to AppAction
            MenuAction::ZoomIn => Self::ZoomIn,
            MenuAction::ZoomOut => Self::ZoomOut,
            MenuAction::ActualSize => Self::ActualSize,
            MenuAction::RecenterView => Self::RecenterView,
            MenuAction::NextLayout => Self::NextLayout,
            MenuAction::PreviousLayout => Self::PreviousLayout,
            MenuAction::SwitchLayout(_) => Self::NextLayout, // not directly mappable
            MenuAction::About => Self::About,
            MenuAction::Settings => Self::Settings,
            MenuAction::CloseWindow => Self::CloseWindow,
        }
    }
}

impl TryFrom<AppAction> for MenuAction {
    type Error = ();
    fn try_from(action: AppAction) -> Result<Self, ()> {
        Ok(match action {
            AppAction::NewFile => MenuAction::NewFile,
            AppAction::NewWindow => MenuAction::NewWindow,
            AppAction::Save => MenuAction::Save,
            AppAction::SaveAs => MenuAction::SaveAs,
            AppAction::OpenFile => MenuAction::OpenFile,
            AppAction::Revert => MenuAction::Revert,
            AppAction::Import => MenuAction::Import,
            AppAction::ImportToLibrary => MenuAction::ImportToLibrary,
            AppAction::Export => MenuAction::Export,
            AppAction::Quit => MenuAction::Quit,
            AppAction::Undo => MenuAction::Undo,
            AppAction::Redo => MenuAction::Redo,
            AppAction::Cut => MenuAction::Cut,
            AppAction::Copy => MenuAction::Copy,
            AppAction::Paste => MenuAction::Paste,
            AppAction::Delete => MenuAction::Delete,
            AppAction::SelectAll => MenuAction::SelectAll,
            AppAction::SelectNone => MenuAction::SelectNone,
            AppAction::Preferences => MenuAction::Preferences,
            AppAction::Group => MenuAction::Group,
            AppAction::ConvertToMovieClip => MenuAction::ConvertToMovieClip,
            AppAction::SendToBack => MenuAction::SendToBack,
            AppAction::BringToFront => MenuAction::BringToFront,
            AppAction::SplitClip => MenuAction::SplitClip,
            AppAction::DuplicateClip => MenuAction::DuplicateClip,
            AppAction::AddLayer => MenuAction::AddLayer,
            AppAction::AddVideoLayer => MenuAction::AddVideoLayer,
            AppAction::AddAudioTrack => MenuAction::AddAudioTrack,
            AppAction::AddMidiTrack => MenuAction::AddMidiTrack,
            AppAction::AddRasterLayer => MenuAction::AddRasterLayer,
            AppAction::AddTestClip => MenuAction::AddTestClip,
            AppAction::DeleteLayer => MenuAction::DeleteLayer,
            AppAction::ToggleLayerVisibility => MenuAction::ToggleLayerVisibility,
            AppAction::ToggleOnionSkin => MenuAction::ToggleOnionSkin,
            AppAction::NewKeyframe => MenuAction::NewKeyframe,
            AppAction::NewBlankKeyframe => MenuAction::NewBlankKeyframe,
            AppAction::DeleteFrame => MenuAction::DeleteFrame,
            AppAction::DuplicateKeyframe => MenuAction::DuplicateKeyframe,
            AppAction::AddKeyframeAtPlayhead => MenuAction::AddKeyframeAtPlayhead,
            AppAction::AddMotionTween => MenuAction::AddMotionTween,
            AppAction::AddShapeTween => MenuAction::AddShapeTween,
            AppAction::ReturnToStart => MenuAction::ReturnToStart,
            AppAction::Play => MenuAction::Play,
            AppAction::ZoomIn => MenuAction::ZoomIn,
            AppAction::ZoomOut => MenuAction::ZoomOut,
            AppAction::ActualSize => MenuAction::ActualSize,
            AppAction::RecenterView => MenuAction::RecenterView,
            AppAction::NextLayout => MenuAction::NextLayout,
            AppAction::PreviousLayout => MenuAction::PreviousLayout,
            AppAction::About => MenuAction::About,
            AppAction::Settings => MenuAction::Settings,
            AppAction::CloseWindow => MenuAction::CloseWindow,
            // Non-menu actions
            _ => return Err(()),
        })
    }
}

// Also need TryFrom<MenuAction> for AppAction (used in menu.rs check_shortcuts)
impl AppAction {
    /// Try to convert from a MenuAction (fails for OpenRecent/ClearRecentFiles/SwitchLayout)
    pub fn try_from(action: MenuAction) -> Result<Self, ()> {
        match action {
            MenuAction::OpenRecent(_) | MenuAction::ClearRecentFiles | MenuAction::SwitchLayout(_) => Err(()),
            other => Ok(Self::from(other)),
        }
    }
}

/// Return the `AppAction` that activates the given tool, if one exists.
/// `Tool::Split` has no tool-shortcut action (it's triggered via the menu).
pub fn tool_app_action(tool: lightningbeam_core::tool::Tool) -> Option<AppAction> {
    use lightningbeam_core::tool::Tool;
    match tool {
        Tool::Select      => Some(AppAction::ToolSelect),
        Tool::Draw        => Some(AppAction::ToolDraw),
        Tool::Transform   => Some(AppAction::ToolTransform),
        Tool::Rectangle   => Some(AppAction::ToolRectangle),
        Tool::Ellipse     => Some(AppAction::ToolEllipse),
        Tool::PaintBucket => Some(AppAction::ToolPaintBucket),
        Tool::Eyedropper  => Some(AppAction::ToolEyedropper),
        Tool::Line        => Some(AppAction::ToolLine),
        Tool::Polygon     => Some(AppAction::ToolPolygon),
        Tool::BezierEdit  => Some(AppAction::ToolBezierEdit),
        Tool::Text        => Some(AppAction::ToolText),
        Tool::RegionSelect => Some(AppAction::ToolRegionSelect),
        Tool::Erase       => Some(AppAction::ToolErase),
        Tool::Smudge      => Some(AppAction::ToolSmudge),
        Tool::SelectLasso => Some(AppAction::ToolSelectLasso),
        Tool::Split       => Some(AppAction::ToolSplit),
        // New tools have no keybinding yet
        _ => None,
    }
}

// === Default bindings ===

/// Build the complete default bindings map from the current hardcoded shortcuts
pub fn all_defaults() -> HashMap<AppAction, Option<Shortcut>> {
    use crate::menu::MenuItemDef;

    let mut defaults = HashMap::new();

    // Menu action defaults (from MenuItemDef constants)
    for def in MenuItemDef::all_with_shortcuts() {
        if let Ok(app_action) = AppAction::try_from(def.action) {
            defaults.insert(app_action, def.shortcut);
        }
    }

    // Also add menu items without shortcuts
    let no_shortcut: &[AppAction] = &[
        AppAction::Revert, AppAction::Preferences, AppAction::ConvertToMovieClip,
        AppAction::SendToBack, AppAction::BringToFront,
        AppAction::AddVideoLayer, AppAction::AddAudioTrack, AppAction::AddMidiTrack,
        AppAction::AddTestClip, AppAction::DeleteLayer, AppAction::ToggleLayerVisibility,
        AppAction::NewBlankKeyframe, AppAction::DeleteFrame, AppAction::DuplicateKeyframe,
        AppAction::AddKeyframeAtPlayhead, AppAction::AddMotionTween, AppAction::AddShapeTween,
        AppAction::ReturnToStart, AppAction::Play, AppAction::RecenterView, AppAction::About,
    ];
    for &action in no_shortcut {
        defaults.entry(action).or_insert(None);
    }

    // Tool shortcuts (bare keys, no modifiers)
    let nc = false;
    let ns = false;
    let na = false;
    defaults.insert(AppAction::ToolSelect,      Some(Shortcut::new(ShortcutKey::V, nc, ns, na)));
    defaults.insert(AppAction::ToolDraw,         Some(Shortcut::new(ShortcutKey::P, nc, ns, na)));
    defaults.insert(AppAction::ToolTransform,    Some(Shortcut::new(ShortcutKey::Q, nc, ns, na)));
    defaults.insert(AppAction::ToolRectangle,    Some(Shortcut::new(ShortcutKey::R, nc, ns, na)));
    defaults.insert(AppAction::ToolEllipse,      Some(Shortcut::new(ShortcutKey::E, nc, ns, na)));
    defaults.insert(AppAction::ToolPaintBucket,  Some(Shortcut::new(ShortcutKey::B, nc, ns, na)));
    defaults.insert(AppAction::ToolEyedropper,   Some(Shortcut::new(ShortcutKey::I, nc, ns, na)));
    defaults.insert(AppAction::ToolLine,         Some(Shortcut::new(ShortcutKey::L, nc, ns, na)));
    defaults.insert(AppAction::ToolPolygon,      Some(Shortcut::new(ShortcutKey::G, nc, ns, na)));
    defaults.insert(AppAction::ToolBezierEdit,   Some(Shortcut::new(ShortcutKey::A, nc, ns, na)));
    defaults.insert(AppAction::ToolText,         Some(Shortcut::new(ShortcutKey::T, nc, ns, na)));
    defaults.insert(AppAction::ToolRegionSelect,  Some(Shortcut::new(ShortcutKey::S, nc, ns, na)));
    defaults.insert(AppAction::ToolErase,         Some(Shortcut::new(ShortcutKey::X, nc, ns, na)));
    defaults.insert(AppAction::ToolSmudge,        Some(Shortcut::new(ShortcutKey::U, nc, ns, na)));
    defaults.insert(AppAction::ToolSelectLasso,   Some(Shortcut::new(ShortcutKey::F, nc, ns, na)));
    defaults.insert(AppAction::ToolSplit,         Some(Shortcut::new(ShortcutKey::C, nc, ns, na)));

    // Global shortcuts
    defaults.insert(AppAction::TogglePlayPause,    Some(Shortcut::new(ShortcutKey::Space, nc, ns, na)));
    defaults.insert(AppAction::CancelAction,       Some(Shortcut::new(ShortcutKey::Escape, nc, ns, na)));
    defaults.insert(AppAction::ToggleDebugOverlay, Some(Shortcut::new(ShortcutKey::F3, nc, ns, na)));
    defaults.insert(AppAction::ToggleOnionSkin,    Some(Shortcut::new(ShortcutKey::O, nc, ns, na)));
    #[cfg(debug_assertions)]
    defaults.insert(AppAction::ToggleTestMode,     Some(Shortcut::new(ShortcutKey::F5, nc, ns, na)));

    // Pane-local shortcuts
    defaults.insert(AppAction::PianoRollDelete,   Some(Shortcut::new(ShortcutKey::Delete, nc, ns, na)));
    defaults.insert(AppAction::StageDelete,       Some(Shortcut::new(ShortcutKey::Delete, nc, ns, na)));
    defaults.insert(AppAction::NodeGraphGroup,    Some(Shortcut::new(ShortcutKey::G, true, ns, na)));
    defaults.insert(AppAction::NodeGraphUngroup,  Some(Shortcut::new(ShortcutKey::G, true, true, na)));
    defaults.insert(AppAction::NodeGraphRename,   Some(Shortcut::new(ShortcutKey::F2, nc, ns, na)));

    defaults
}

// === KeybindingConfig (persisted in AppConfig) ===

/// Sparse override map: only stores non-default bindings.
/// `None` value means "unbound" (user explicitly cleared the binding).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeybindingConfig {
    #[serde(default)]
    pub overrides: HashMap<AppAction, Option<Shortcut>>,
}

impl KeybindingConfig {
    /// Compute effective bindings by merging defaults with overrides
    pub fn effective_bindings(&self) -> HashMap<AppAction, Option<Shortcut>> {
        let mut bindings = all_defaults();
        for (action, shortcut) in &self.overrides {
            bindings.insert(*action, *shortcut);
        }
        bindings
    }

    /// Reset all overrides (revert to defaults)
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        self.overrides.clear();
    }
}

// === KeymapManager (runtime lookup) ===

/// Runtime shortcut lookup table, built from KeybindingConfig.
/// Consulted everywhere shortcuts are checked.
pub struct KeymapManager {
    /// action -> shortcut (None means unbound)
    bindings: HashMap<AppAction, Option<Shortcut>>,
    /// Reverse lookup: shortcut -> list of actions (for conflict detection)
    #[allow(dead_code)]
    reverse: HashMap<Shortcut, Vec<AppAction>>,
}

impl KeymapManager {
    /// Build from a KeybindingConfig
    pub fn new(config: &KeybindingConfig) -> Self {
        let bindings = config.effective_bindings();
        let mut reverse: HashMap<Shortcut, Vec<AppAction>> = HashMap::new();
        for (&action, shortcut) in &bindings {
            if let Some(s) = shortcut {
                reverse.entry(*s).or_default().push(action);
            }
        }
        Self { bindings, reverse }
    }

    /// Get the shortcut bound to an action (None = unbound)
    pub fn get(&self, action: AppAction) -> Option<Shortcut> {
        self.bindings.get(&action).copied().flatten()
    }

    /// Check if the shortcut for an action was pressed this frame
    pub fn action_pressed(&self, action: AppAction, input: &egui::InputState) -> bool {
        if let Some(shortcut) = self.get(action) {
            shortcut.matches_egui_input(input)
        } else {
            false
        }
    }

    /// Check if the action was pressed, also accepting Backspace as alias for Delete
    pub fn action_pressed_with_backspace(&self, action: AppAction, input: &egui::InputState) -> bool {
        if self.action_pressed(action, input) {
            return true;
        }
        // Also check Backspace as a secondary trigger for delete-like actions
        if let Some(shortcut) = self.get(action) {
            if shortcut.key == ShortcutKey::Delete {
                let backspace_shortcut = Shortcut::new(ShortcutKey::Backspace, shortcut.ctrl, shortcut.shift, shortcut.alt);
                return backspace_shortcut.matches_egui_input(input);
            }
        }
        false
    }

    /// Find all conflicts (two+ actions in the same scope sharing the same shortcut).
    /// Pane-local actions are scoped to their pane and can't conflict across panes.
    #[allow(dead_code)]
    pub fn conflicts(&self) -> Vec<(AppAction, AppAction, Shortcut)> {
        // Group by (scope, shortcut)
        let mut by_scope: HashMap<(&str, Shortcut), Vec<AppAction>> = HashMap::new();
        for (shortcut, actions) in &self.reverse {
            for &action in actions {
                by_scope.entry((action.conflict_scope(), *shortcut)).or_default().push(action);
            }
        }
        let mut conflicts = Vec::new();
        for ((_, shortcut), actions) in &by_scope {
            if actions.len() > 1 {
                for i in 0..actions.len() {
                    for j in (i + 1)..actions.len() {
                        conflicts.push((actions[i], actions[j], *shortcut));
                    }
                }
            }
        }
        conflicts
    }

    /// Set a binding for live editing (used in preferences dialog).
    /// Does NOT persist — call `to_config()` to get the persistable form.
    #[allow(dead_code)]
    pub fn set_binding(&mut self, action: AppAction, shortcut: Option<Shortcut>) {
        // Remove old reverse entry
        if let Some(old) = self.bindings.get(&action).copied().flatten() {
            if let Some(actions) = self.reverse.get_mut(&old) {
                actions.retain(|a| *a != action);
                if actions.is_empty() {
                    self.reverse.remove(&old);
                }
            }
        }
        // Set new binding
        self.bindings.insert(action, shortcut);
        if let Some(s) = shortcut {
            self.reverse.entry(s).or_default().push(action);
        }
    }

    /// Convert current state to a sparse config (only non-default entries)
    #[allow(dead_code)]
    pub fn to_config(&self) -> KeybindingConfig {
        let defaults = all_defaults();
        let mut overrides = HashMap::new();
        for (&action, &shortcut) in &self.bindings {
            let default = defaults.get(&action).copied().flatten();
            if shortcut != default {
                overrides.insert(action, shortcut);
            }
        }
        KeybindingConfig { overrides }
    }
}
