/// Native menu implementation using muda
///
/// This module creates the native menu bar with all menu items matching
/// the JavaScript version's menu structure.
///
/// Menu definitions are centralized to allow generating both native menus
/// and keyboard shortcut handlers from a single source.

use eframe::egui;
use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    Menu, MenuItem, PredefinedMenuItem, Submenu,
};

/// Keyboard shortcut definition
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Shortcut {
    pub key: ShortcutKey,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Keys that can be used in shortcuts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ShortcutKey {
    // Letters
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    // Digits
    Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9,
    // Function keys
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    // Arrow keys
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight,
    // Symbols
    Comma, Minus, Equals,
    #[allow(dead_code)] // Completes keyboard mapping set
    Plus,
    BracketLeft, BracketRight,
    Semicolon, Quote, Period, Slash, Backtick,
    // Special
    Space, Escape, Enter, Tab, Backspace, Delete,
    Home, End, PageUp, PageDown,
}

impl ShortcutKey {
    /// Convert to the corresponding `egui::Key`.
    ///
    /// Note: we maintain our own `ShortcutKey` enum rather than using `egui::Key` directly
    /// because `egui::Key` only implements `serde::{Serialize, Deserialize}` behind the
    /// `serde` cargo feature, which we do not enable for egui. Enabling it would couple
    /// our persisted config format to egui's internal variant names, which could change
    /// between egui version upgrades and silently break user keybind files. `ShortcutKey`
    /// gives us a stable, self-owned serialization surface. The tradeoff is this one
    /// exhaustive mapping; the display and input-matching methods below both delegate to
    /// `egui::Key` so there is no further duplication.
    pub fn to_egui_key(self) -> egui::Key {
        match self {
            Self::A => egui::Key::A, Self::B => egui::Key::B, Self::C => egui::Key::C,
            Self::D => egui::Key::D, Self::E => egui::Key::E, Self::F => egui::Key::F,
            Self::G => egui::Key::G, Self::H => egui::Key::H, Self::I => egui::Key::I,
            Self::J => egui::Key::J, Self::K => egui::Key::K, Self::L => egui::Key::L,
            Self::M => egui::Key::M, Self::N => egui::Key::N, Self::O => egui::Key::O,
            Self::P => egui::Key::P, Self::Q => egui::Key::Q, Self::R => egui::Key::R,
            Self::S => egui::Key::S, Self::T => egui::Key::T, Self::U => egui::Key::U,
            Self::V => egui::Key::V, Self::W => egui::Key::W, Self::X => egui::Key::X,
            Self::Y => egui::Key::Y, Self::Z => egui::Key::Z,
            Self::Num0 => egui::Key::Num0, Self::Num1 => egui::Key::Num1,
            Self::Num2 => egui::Key::Num2, Self::Num3 => egui::Key::Num3,
            Self::Num4 => egui::Key::Num4, Self::Num5 => egui::Key::Num5,
            Self::Num6 => egui::Key::Num6, Self::Num7 => egui::Key::Num7,
            Self::Num8 => egui::Key::Num8, Self::Num9 => egui::Key::Num9,
            Self::F1 => egui::Key::F1,   Self::F2 => egui::Key::F2,
            Self::F3 => egui::Key::F3,   Self::F4 => egui::Key::F4,
            Self::F5 => egui::Key::F5,   Self::F6 => egui::Key::F6,
            Self::F7 => egui::Key::F7,   Self::F8 => egui::Key::F8,
            Self::F9 => egui::Key::F9,   Self::F10 => egui::Key::F10,
            Self::F11 => egui::Key::F11, Self::F12 => egui::Key::F12,
            Self::ArrowUp => egui::Key::ArrowUp, Self::ArrowDown => egui::Key::ArrowDown,
            Self::ArrowLeft => egui::Key::ArrowLeft, Self::ArrowRight => egui::Key::ArrowRight,
            Self::Comma => egui::Key::Comma, Self::Minus => egui::Key::Minus,
            Self::Equals => egui::Key::Equals, Self::Plus => egui::Key::Plus,
            Self::BracketLeft => egui::Key::OpenBracket,
            Self::BracketRight => egui::Key::CloseBracket,
            Self::Semicolon => egui::Key::Semicolon, Self::Quote => egui::Key::Quote,
            Self::Period => egui::Key::Period, Self::Slash => egui::Key::Slash,
            Self::Backtick => egui::Key::Backtick,
            Self::Space => egui::Key::Space, Self::Escape => egui::Key::Escape,
            Self::Enter => egui::Key::Enter, Self::Tab => egui::Key::Tab,
            Self::Backspace => egui::Key::Backspace, Self::Delete => egui::Key::Delete,
            Self::Home => egui::Key::Home, Self::End => egui::Key::End,
            Self::PageUp => egui::Key::PageUp, Self::PageDown => egui::Key::PageDown,
        }
    }

    /// Short human-readable name for this key (e.g. "A", "F1", "Delete").
    /// Delegates to `egui::Key::name()` so the strings stay consistent with
    /// what egui itself would display.
    pub fn display_name(self) -> &'static str {
        self.to_egui_key().name()
    }

    /// Try to convert an egui Key to a ShortcutKey
    pub fn from_egui_key(key: egui::Key) -> Option<Self> {
        Some(match key {
            egui::Key::A => Self::A, egui::Key::B => Self::B, egui::Key::C => Self::C,
            egui::Key::D => Self::D, egui::Key::E => Self::E, egui::Key::F => Self::F,
            egui::Key::G => Self::G, egui::Key::H => Self::H, egui::Key::I => Self::I,
            egui::Key::J => Self::J, egui::Key::K => Self::K, egui::Key::L => Self::L,
            egui::Key::M => Self::M, egui::Key::N => Self::N, egui::Key::O => Self::O,
            egui::Key::P => Self::P, egui::Key::Q => Self::Q, egui::Key::R => Self::R,
            egui::Key::S => Self::S, egui::Key::T => Self::T, egui::Key::U => Self::U,
            egui::Key::V => Self::V, egui::Key::W => Self::W, egui::Key::X => Self::X,
            egui::Key::Y => Self::Y, egui::Key::Z => Self::Z,
            egui::Key::Num0 => Self::Num0, egui::Key::Num1 => Self::Num1,
            egui::Key::Num2 => Self::Num2, egui::Key::Num3 => Self::Num3,
            egui::Key::Num4 => Self::Num4, egui::Key::Num5 => Self::Num5,
            egui::Key::Num6 => Self::Num6, egui::Key::Num7 => Self::Num7,
            egui::Key::Num8 => Self::Num8, egui::Key::Num9 => Self::Num9,
            egui::Key::F1 => Self::F1, egui::Key::F2 => Self::F2,
            egui::Key::F3 => Self::F3, egui::Key::F4 => Self::F4,
            egui::Key::F5 => Self::F5, egui::Key::F6 => Self::F6,
            egui::Key::F7 => Self::F7, egui::Key::F8 => Self::F8,
            egui::Key::F9 => Self::F9, egui::Key::F10 => Self::F10,
            egui::Key::F11 => Self::F11, egui::Key::F12 => Self::F12,
            egui::Key::ArrowUp => Self::ArrowUp, egui::Key::ArrowDown => Self::ArrowDown,
            egui::Key::ArrowLeft => Self::ArrowLeft, egui::Key::ArrowRight => Self::ArrowRight,
            egui::Key::Comma => Self::Comma, egui::Key::Minus => Self::Minus,
            egui::Key::Equals => Self::Equals, egui::Key::Plus => Self::Plus,
            egui::Key::OpenBracket => Self::BracketLeft, egui::Key::CloseBracket => Self::BracketRight,
            egui::Key::Semicolon => Self::Semicolon, egui::Key::Quote => Self::Quote,
            egui::Key::Period => Self::Period, egui::Key::Slash => Self::Slash,
            egui::Key::Backtick => Self::Backtick,
            egui::Key::Space => Self::Space, egui::Key::Escape => Self::Escape,
            egui::Key::Enter => Self::Enter, egui::Key::Tab => Self::Tab,
            egui::Key::Backspace => Self::Backspace, egui::Key::Delete => Self::Delete,
            egui::Key::Home => Self::Home, egui::Key::End => Self::End,
            egui::Key::PageUp => Self::PageUp, egui::Key::PageDown => Self::PageDown,
            _ => return None,
        })
    }
}

impl Shortcut {
    pub const fn new(key: ShortcutKey, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self { key, ctrl, shift, alt }
    }

    /// Short hint string suitable for tool tooltips (e.g. "F", "Ctrl+S").
    pub fn hint_text(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.ctrl  { parts.push("Ctrl"); }
        if self.shift { parts.push("Shift"); }
        if self.alt   { parts.push("Alt"); }
        parts.push(self.key.display_name());
        parts.join("+")
    }

    /// Convert to muda Accelerator
    pub fn to_muda_accelerator(&self) -> Accelerator {
        let mut modifiers = Modifiers::empty();
        if self.ctrl {
            #[cfg(target_os = "macos")]
            { modifiers |= Modifiers::META; }
            #[cfg(not(target_os = "macos"))]
            { modifiers |= Modifiers::CONTROL; }
        }
        if self.shift {
            modifiers |= Modifiers::SHIFT;
        }
        if self.alt {
            modifiers |= Modifiers::ALT;
        }

        let code = match self.key {
            ShortcutKey::A => Code::KeyA,
            ShortcutKey::B => Code::KeyB,
            ShortcutKey::C => Code::KeyC,
            ShortcutKey::D => Code::KeyD,
            ShortcutKey::E => Code::KeyE,
            ShortcutKey::F => Code::KeyF,
            ShortcutKey::G => Code::KeyG,
            ShortcutKey::H => Code::KeyH,
            ShortcutKey::I => Code::KeyI,
            ShortcutKey::J => Code::KeyJ,
            ShortcutKey::K => Code::KeyK,
            ShortcutKey::L => Code::KeyL,
            ShortcutKey::M => Code::KeyM,
            ShortcutKey::N => Code::KeyN,
            ShortcutKey::O => Code::KeyO,
            ShortcutKey::P => Code::KeyP,
            ShortcutKey::Q => Code::KeyQ,
            ShortcutKey::R => Code::KeyR,
            ShortcutKey::S => Code::KeyS,
            ShortcutKey::T => Code::KeyT,
            ShortcutKey::U => Code::KeyU,
            ShortcutKey::V => Code::KeyV,
            ShortcutKey::W => Code::KeyW,
            ShortcutKey::X => Code::KeyX,
            ShortcutKey::Y => Code::KeyY,
            ShortcutKey::Z => Code::KeyZ,
            ShortcutKey::Num0 => Code::Digit0,
            ShortcutKey::Num1 => Code::Digit1,
            ShortcutKey::Num2 => Code::Digit2,
            ShortcutKey::Num3 => Code::Digit3,
            ShortcutKey::Num4 => Code::Digit4,
            ShortcutKey::Num5 => Code::Digit5,
            ShortcutKey::Num6 => Code::Digit6,
            ShortcutKey::Num7 => Code::Digit7,
            ShortcutKey::Num8 => Code::Digit8,
            ShortcutKey::Num9 => Code::Digit9,
            ShortcutKey::F1 => Code::F1,
            ShortcutKey::F2 => Code::F2,
            ShortcutKey::F3 => Code::F3,
            ShortcutKey::F4 => Code::F4,
            ShortcutKey::F5 => Code::F5,
            ShortcutKey::F6 => Code::F6,
            ShortcutKey::F7 => Code::F7,
            ShortcutKey::F8 => Code::F8,
            ShortcutKey::F9 => Code::F9,
            ShortcutKey::F10 => Code::F10,
            ShortcutKey::F11 => Code::F11,
            ShortcutKey::F12 => Code::F12,
            ShortcutKey::ArrowUp => Code::ArrowUp,
            ShortcutKey::ArrowDown => Code::ArrowDown,
            ShortcutKey::ArrowLeft => Code::ArrowLeft,
            ShortcutKey::ArrowRight => Code::ArrowRight,
            ShortcutKey::Comma => Code::Comma,
            ShortcutKey::Minus => Code::Minus,
            ShortcutKey::Equals => Code::Equal,
            ShortcutKey::Plus => Code::Equal, // Same key as equals
            ShortcutKey::BracketLeft => Code::BracketLeft,
            ShortcutKey::BracketRight => Code::BracketRight,
            ShortcutKey::Semicolon => Code::Semicolon,
            ShortcutKey::Quote => Code::Quote,
            ShortcutKey::Period => Code::Period,
            ShortcutKey::Slash => Code::Slash,
            ShortcutKey::Backtick => Code::Backquote,
            ShortcutKey::Space => Code::Space,
            ShortcutKey::Escape => Code::Escape,
            ShortcutKey::Enter => Code::Enter,
            ShortcutKey::Tab => Code::Tab,
            ShortcutKey::Backspace => Code::Backspace,
            ShortcutKey::Delete => Code::Delete,
            ShortcutKey::Home => Code::Home,
            ShortcutKey::End => Code::End,
            ShortcutKey::PageUp => Code::PageUp,
            ShortcutKey::PageDown => Code::PageDown,
        };

        Accelerator::new(if modifiers.is_empty() { None } else { Some(modifiers) }, code)
    }

    /// Check if this shortcut matches the current egui input state
    pub fn matches_egui_input(&self, input: &egui::InputState) -> bool {
        // Check modifiers first
        if self.ctrl != input.modifiers.ctrl {
            return false;
        }
        if self.shift != input.modifiers.shift {
            return false;
        }
        if self.alt != input.modifiers.alt {
            return false;
        }

        input.key_pressed(self.key.to_egui_key())
    }
}

/// All possible menu actions that can be triggered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    // File menu
    NewFile,
    NewWindow,
    Save,
    SaveAs,
    OpenFile,
    OpenRecent(usize), // Index into recent files list
    ClearRecentFiles,  // Clear recent files list
    Revert,
    Import,
    ImportToLibrary,
    Export,
    Quit,

    // Edit menu
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Delete,
    SelectAll,
    SelectNone,
    Preferences,

    // Modify menu
    Group,
    ConvertToMovieClip,
    SendToBack,
    BringToFront,
    SplitClip,
    DuplicateClip,

    // Layer menu
    AddLayer,
    AddVideoLayer,
    AddAudioTrack,
    AddMidiTrack,
    AddRasterLayer,
    AddTestClip, // For testing: adds a test clip to the asset library
    DeleteLayer,
    ToggleLayerVisibility,
    ShowMasterTrack,

    // Timeline menu
    NewKeyframe,
    NewBlankKeyframe,
    DeleteFrame,
    DuplicateKeyframe,
    AddKeyframeAtPlayhead,
    AddMotionTween,
    AddShapeTween,
    ReturnToStart,
    Play,
    ToggleCountIn,

    // View menu
    ZoomIn,
    ZoomOut,
    ActualSize,
    RecenterView,
    ToggleOnionSkin,
    NextLayout,
    PreviousLayout,
    #[allow(dead_code)] // Handler exists in main.rs, menu item not yet wired
    SwitchLayout(usize),

    // Help menu
    About,

    // Lightningbeam menu (macOS)
    Settings,
    CloseWindow,
}

/// Menu item definition
pub struct MenuItemDef {
    pub label: &'static str,
    pub action: MenuAction,
    pub shortcut: Option<Shortcut>,
}

/// Menu structure definition - can be an item, separator, or submenu
pub enum MenuDef {
    Item(&'static MenuItemDef),
    Separator,
    Submenu {
        label: &'static str,
        children: &'static [MenuDef],
    },
}

// Shortcut constants for clarity
const CTRL: bool = true;
const SHIFT: bool = true;
#[allow(dead_code)]
const ALT: bool = true;
const NO_CTRL: bool = false;
const NO_SHIFT: bool = false;
const NO_ALT: bool = false;

// Central menu definitions - single source of truth
impl MenuItemDef {
    // File menu items
    const NEW_FILE: Self = Self { label: "New file...", action: MenuAction::NewFile, shortcut: Some(Shortcut::new(ShortcutKey::N, CTRL, NO_SHIFT, NO_ALT)) };
    const NEW_WINDOW: Self = Self { label: "New Window", action: MenuAction::NewWindow, shortcut: Some(Shortcut::new(ShortcutKey::N, CTRL, SHIFT, NO_ALT)) };
    const SAVE: Self = Self { label: "Save", action: MenuAction::Save, shortcut: Some(Shortcut::new(ShortcutKey::S, CTRL, NO_SHIFT, NO_ALT)) };
    const SAVE_AS: Self = Self { label: "Save As...", action: MenuAction::SaveAs, shortcut: Some(Shortcut::new(ShortcutKey::S, CTRL, SHIFT, NO_ALT)) };
    const OPEN_FILE: Self = Self { label: "Open File...", action: MenuAction::OpenFile, shortcut: Some(Shortcut::new(ShortcutKey::O, CTRL, NO_SHIFT, NO_ALT)) };
    const REVERT: Self = Self { label: "Revert", action: MenuAction::Revert, shortcut: None };
    const IMPORT: Self = Self { label: "Import...", action: MenuAction::Import, shortcut: Some(Shortcut::new(ShortcutKey::I, CTRL, NO_SHIFT, NO_ALT)) };
    const IMPORT_TO_LIBRARY: Self = Self { label: "Import to Library...", action: MenuAction::ImportToLibrary, shortcut: Some(Shortcut::new(ShortcutKey::I, CTRL, SHIFT, NO_ALT)) };
    const EXPORT: Self = Self { label: "Export...", action: MenuAction::Export, shortcut: Some(Shortcut::new(ShortcutKey::E, CTRL, SHIFT, NO_ALT)) };
    const QUIT: Self = Self { label: "Quit", action: MenuAction::Quit, shortcut: Some(Shortcut::new(ShortcutKey::Q, CTRL, NO_SHIFT, NO_ALT)) };

    // Edit menu items
    const UNDO: Self = Self { label: "Undo", action: MenuAction::Undo, shortcut: Some(Shortcut::new(ShortcutKey::Z, CTRL, NO_SHIFT, NO_ALT)) };
    const REDO: Self = Self { label: "Redo", action: MenuAction::Redo, shortcut: Some(Shortcut::new(ShortcutKey::Z, CTRL, SHIFT, NO_ALT)) };
    const CUT: Self = Self { label: "Cut", action: MenuAction::Cut, shortcut: Some(Shortcut::new(ShortcutKey::X, CTRL, NO_SHIFT, NO_ALT)) };
    const COPY: Self = Self { label: "Copy", action: MenuAction::Copy, shortcut: Some(Shortcut::new(ShortcutKey::C, CTRL, NO_SHIFT, NO_ALT)) };
    const PASTE: Self = Self { label: "Paste", action: MenuAction::Paste, shortcut: Some(Shortcut::new(ShortcutKey::V, CTRL, NO_SHIFT, NO_ALT)) };
    const DELETE: Self = Self { label: "Delete", action: MenuAction::Delete, shortcut: Some(Shortcut::new(ShortcutKey::Delete, NO_CTRL, NO_SHIFT, NO_ALT)) };
    const SELECT_ALL: Self = Self { label: "Select All", action: MenuAction::SelectAll, shortcut: Some(Shortcut::new(ShortcutKey::A, CTRL, NO_SHIFT, NO_ALT)) };
    const SELECT_NONE: Self = Self { label: "Select None", action: MenuAction::SelectNone, shortcut: Some(Shortcut::new(ShortcutKey::A, CTRL, SHIFT, NO_ALT)) };
    const PREFERENCES: Self = Self { label: "Preferences", action: MenuAction::Preferences, shortcut: None };

    // Modify menu items
    const GROUP: Self = Self { label: "Group", action: MenuAction::Group, shortcut: Some(Shortcut::new(ShortcutKey::G, CTRL, NO_SHIFT, NO_ALT)) };
    const CONVERT_TO_MOVIE_CLIP: Self = Self { label: "Convert to Movie Clip", action: MenuAction::ConvertToMovieClip, shortcut: None };
    const SEND_TO_BACK: Self = Self { label: "Send to back", action: MenuAction::SendToBack, shortcut: None };
    const BRING_TO_FRONT: Self = Self { label: "Bring to front", action: MenuAction::BringToFront, shortcut: None };
    const SPLIT_CLIP: Self = Self { label: "Split Clip", action: MenuAction::SplitClip, shortcut: Some(Shortcut::new(ShortcutKey::K, CTRL, NO_SHIFT, NO_ALT)) };
    const DUPLICATE_CLIP: Self = Self { label: "Duplicate Clip", action: MenuAction::DuplicateClip, shortcut: Some(Shortcut::new(ShortcutKey::D, CTRL, NO_SHIFT, NO_ALT)) };

    // Layer menu items
    const ADD_LAYER: Self = Self { label: "Add Layer", action: MenuAction::AddLayer, shortcut: Some(Shortcut::new(ShortcutKey::L, CTRL, SHIFT, NO_ALT)) };
    const ADD_VIDEO_LAYER: Self = Self { label: "Add Video Layer", action: MenuAction::AddVideoLayer, shortcut: None };
    const ADD_AUDIO_TRACK: Self = Self { label: "Add Audio Track", action: MenuAction::AddAudioTrack, shortcut: None };
    const ADD_MIDI_TRACK: Self = Self { label: "Add MIDI Track", action: MenuAction::AddMidiTrack, shortcut: None };
    const ADD_RASTER_LAYER: Self = Self { label: "Add Raster Layer", action: MenuAction::AddRasterLayer, shortcut: None };
    const ADD_TEST_CLIP: Self = Self { label: "Add Test Clip to Library", action: MenuAction::AddTestClip, shortcut: None };
    const DELETE_LAYER: Self = Self { label: "Delete Layer", action: MenuAction::DeleteLayer, shortcut: None };
    const TOGGLE_LAYER_VISIBILITY: Self = Self { label: "Hide/Show Layer", action: MenuAction::ToggleLayerVisibility, shortcut: None };
    const SHOW_MASTER_TRACK: Self = Self { label: "Show Master Track", action: MenuAction::ShowMasterTrack, shortcut: None };

    // Timeline menu items
    const NEW_KEYFRAME: Self = Self { label: "New Keyframe", action: MenuAction::NewKeyframe, shortcut: Some(Shortcut::new(ShortcutKey::K, NO_CTRL, NO_SHIFT, NO_ALT)) };
    const NEW_BLANK_KEYFRAME: Self = Self { label: "New Blank Keyframe", action: MenuAction::NewBlankKeyframe, shortcut: None };
    const DELETE_FRAME: Self = Self { label: "Delete Frame", action: MenuAction::DeleteFrame, shortcut: None };
    const DUPLICATE_KEYFRAME: Self = Self { label: "Duplicate Keyframe", action: MenuAction::DuplicateKeyframe, shortcut: None };
    const ADD_KEYFRAME_AT_PLAYHEAD: Self = Self { label: "Add Keyframe at Playhead", action: MenuAction::AddKeyframeAtPlayhead, shortcut: None };
    const ADD_MOTION_TWEEN: Self = Self { label: "Add Motion Tween", action: MenuAction::AddMotionTween, shortcut: None };
    const ADD_SHAPE_TWEEN: Self = Self { label: "Add Shape Tween", action: MenuAction::AddShapeTween, shortcut: None };
    const RETURN_TO_START: Self = Self { label: "Return to start", action: MenuAction::ReturnToStart, shortcut: None };
    const PLAY: Self = Self { label: "Play", action: MenuAction::Play, shortcut: None };
    const COUNT_IN: Self = Self { label: "Count In", action: MenuAction::ToggleCountIn, shortcut: None };

    // View menu items
    const ZOOM_IN: Self = Self { label: "Zoom In", action: MenuAction::ZoomIn, shortcut: Some(Shortcut::new(ShortcutKey::Equals, CTRL, NO_SHIFT, NO_ALT)) };
    const ZOOM_OUT: Self = Self { label: "Zoom Out", action: MenuAction::ZoomOut, shortcut: Some(Shortcut::new(ShortcutKey::Minus, CTRL, NO_SHIFT, NO_ALT)) };
    const ACTUAL_SIZE: Self = Self { label: "Actual Size", action: MenuAction::ActualSize, shortcut: Some(Shortcut::new(ShortcutKey::Num0, CTRL, NO_SHIFT, NO_ALT)) };
    const RECENTER_VIEW: Self = Self { label: "Recenter View", action: MenuAction::RecenterView, shortcut: None };
    const TOGGLE_ONION_SKIN: Self = Self { label: "Onion Skinning", action: MenuAction::ToggleOnionSkin, shortcut: Some(Shortcut::new(ShortcutKey::O, NO_CTRL, NO_SHIFT, NO_ALT)) };
    const NEXT_LAYOUT: Self = Self { label: "Next Layout", action: MenuAction::NextLayout, shortcut: Some(Shortcut::new(ShortcutKey::BracketRight, CTRL, NO_SHIFT, NO_ALT)) };
    const PREVIOUS_LAYOUT: Self = Self { label: "Previous Layout", action: MenuAction::PreviousLayout, shortcut: Some(Shortcut::new(ShortcutKey::BracketLeft, CTRL, NO_SHIFT, NO_ALT)) };

    // Help menu items
    const ABOUT: Self = Self { label: "About...", action: MenuAction::About, shortcut: None };

    // macOS app menu items
    const SETTINGS: Self = Self { label: "Settings", action: MenuAction::Settings, shortcut: Some(Shortcut::new(ShortcutKey::Comma, CTRL, NO_SHIFT, NO_ALT)) };
    const CLOSE_WINDOW: Self = Self { label: "Close Window", action: MenuAction::CloseWindow, shortcut: Some(Shortcut::new(ShortcutKey::W, CTRL, NO_SHIFT, NO_ALT)) };
    #[allow(dead_code)] // Used in #[cfg(target_os = "macos")] block
    const QUIT_MACOS: Self = Self { label: "Quit Lightningbeam", action: MenuAction::Quit, shortcut: Some(Shortcut::new(ShortcutKey::Q, CTRL, NO_SHIFT, NO_ALT)) };
    #[allow(dead_code)]
    const ABOUT_MACOS: Self = Self { label: "About Lightningbeam", action: MenuAction::About, shortcut: None };

    /// Get all menu items with shortcuts (for keyboard handling)
    pub fn all_with_shortcuts() -> &'static [&'static MenuItemDef] {
        &[
            &Self::NEW_FILE, &Self::NEW_WINDOW, &Self::SAVE, &Self::SAVE_AS,
            &Self::OPEN_FILE, &Self::IMPORT, &Self::IMPORT_TO_LIBRARY, &Self::EXPORT, &Self::QUIT,
            &Self::UNDO, &Self::REDO, &Self::CUT, &Self::COPY, &Self::PASTE,
            &Self::DELETE, &Self::SELECT_ALL, &Self::SELECT_NONE,
            &Self::GROUP, &Self::ADD_LAYER, &Self::NEW_KEYFRAME,
            &Self::ZOOM_IN, &Self::ZOOM_OUT, &Self::ACTUAL_SIZE,
            &Self::TOGGLE_ONION_SKIN,
            &Self::NEXT_LAYOUT, &Self::PREVIOUS_LAYOUT,
            &Self::SETTINGS, &Self::CLOSE_WINDOW,
        ]
    }

    /// Get the complete menu structure definition
    pub const fn menu_structure() -> &'static [MenuDef] {
        &[
            // File menu
            MenuDef::Submenu {
                label: "File",
                children: &[
                    MenuDef::Item(&Self::NEW_FILE),
                    MenuDef::Item(&Self::NEW_WINDOW),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SAVE),
                    MenuDef::Item(&Self::SAVE_AS),
                    MenuDef::Separator,
                    MenuDef::Submenu {
                        label: "Open Recent",
                        children: &[], // TODO: Dynamic recent files
                    },
                    MenuDef::Item(&Self::OPEN_FILE),
                    MenuDef::Item(&Self::REVERT),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::IMPORT),
                    MenuDef::Item(&Self::IMPORT_TO_LIBRARY),
                    MenuDef::Item(&Self::EXPORT),
                    #[cfg(not(target_os = "macos"))]
                    MenuDef::Separator,
                    #[cfg(not(target_os = "macos"))]
                    MenuDef::Item(&Self::QUIT),
                ],
            },
            // Edit menu
            MenuDef::Submenu {
                label: "Edit",
                children: &[
                    MenuDef::Item(&Self::UNDO),
                    MenuDef::Item(&Self::REDO),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::CUT),
                    MenuDef::Item(&Self::COPY),
                    MenuDef::Item(&Self::PASTE),
                    MenuDef::Item(&Self::DELETE),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SELECT_ALL),
                    MenuDef::Item(&Self::SELECT_NONE),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::PREFERENCES),
                ],
            },
            // Modify menu
            MenuDef::Submenu {
                label: "Modify",
                children: &[
                    MenuDef::Item(&Self::GROUP),
                    MenuDef::Item(&Self::CONVERT_TO_MOVIE_CLIP),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SEND_TO_BACK),
                    MenuDef::Item(&Self::BRING_TO_FRONT),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SPLIT_CLIP),
                    MenuDef::Item(&Self::DUPLICATE_CLIP),
                ],
            },
            // Layer menu
            MenuDef::Submenu {
                label: "Layer",
                children: &[
                    MenuDef::Item(&Self::ADD_LAYER),
                    MenuDef::Item(&Self::ADD_VIDEO_LAYER),
                    MenuDef::Item(&Self::ADD_AUDIO_TRACK),
                    MenuDef::Item(&Self::ADD_MIDI_TRACK),
                    MenuDef::Item(&Self::ADD_RASTER_LAYER),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::ADD_TEST_CLIP),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::DELETE_LAYER),
                    MenuDef::Item(&Self::TOGGLE_LAYER_VISIBILITY),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SHOW_MASTER_TRACK),
                ],
            },
            // Timeline menu
            MenuDef::Submenu {
                label: "Timeline",
                children: &[
                    MenuDef::Item(&Self::NEW_KEYFRAME),
                    MenuDef::Item(&Self::NEW_BLANK_KEYFRAME),
                    MenuDef::Item(&Self::DELETE_FRAME),
                    MenuDef::Item(&Self::DUPLICATE_KEYFRAME),
                    MenuDef::Item(&Self::ADD_KEYFRAME_AT_PLAYHEAD),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::ADD_MOTION_TWEEN),
                    MenuDef::Item(&Self::ADD_SHAPE_TWEEN),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::RETURN_TO_START),
                    MenuDef::Item(&Self::PLAY),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::COUNT_IN),
                ],
            },
            // View menu
            MenuDef::Submenu {
                label: "View",
                children: &[
                    MenuDef::Item(&Self::ZOOM_IN),
                    MenuDef::Item(&Self::ZOOM_OUT),
                    MenuDef::Item(&Self::ACTUAL_SIZE),
                    MenuDef::Item(&Self::RECENTER_VIEW),
                    MenuDef::Item(&Self::TOGGLE_ONION_SKIN),
                    MenuDef::Separator,
                    MenuDef::Submenu {
                        label: "Layout",
                        children: &[
                            MenuDef::Item(&Self::NEXT_LAYOUT),
                            MenuDef::Item(&Self::PREVIOUS_LAYOUT),
                            // TODO: Dynamic layout list
                        ],
                    },
                ],
            },
            // Help menu
            MenuDef::Submenu {
                label: "Help",
                children: &[
                    MenuDef::Item(&Self::ABOUT),
                ],
            },
        ]
    }

    /// Get macOS app menu structure
    #[cfg(target_os = "macos")]
    pub const fn macos_app_menu() -> MenuDef {
        MenuDef::Submenu {
            label: "Lightningbeam",
            children: &[
                MenuDef::Item(&Self::ABOUT_MACOS),
                MenuDef::Separator,
                MenuDef::Item(&Self::SETTINGS),
                MenuDef::Separator,
                MenuDef::Item(&Self::CLOSE_WINDOW),
                MenuDef::Item(&Self::QUIT_MACOS),
            ],
        }
    }
}

/// Menu system that holds all menu items and can dispatch actions
pub struct MenuSystem {
    #[allow(dead_code)]
    menu: Menu,
    items: Vec<(MenuItem, MenuAction)>,
    /// Reference to "Open Recent" submenu for dynamic updates
    open_recent_submenu: Option<Submenu>,
}

impl MenuSystem {
    /// Create a new menu system with all menus and items
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let mut items = Vec::new();
        let mut open_recent_submenu: Option<Submenu> = None;

        // Platform-specific: Add "Lightningbeam" menu on macOS
        #[cfg(target_os = "macos")]
        {
            Self::build_submenu(&menu, &MenuItemDef::macos_app_menu(), &mut items, &mut open_recent_submenu)?;
        }

        // Build all menus from the centralized structure
        for menu_def in MenuItemDef::menu_structure() {
            Self::build_submenu(&menu, menu_def, &mut items, &mut open_recent_submenu)?;
        }

        Ok(Self { menu, items, open_recent_submenu })
    }

    /// Build a top-level submenu and append to menu
    fn build_submenu(
        menu: &Menu,
        def: &MenuDef,
        items: &mut Vec<(MenuItem, MenuAction)>,
        open_recent_submenu: &mut Option<Submenu>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let MenuDef::Submenu { label, children } = def {
            let submenu = Submenu::new(*label, true);
            for child in *children {
                Self::build_menu_item(&submenu, child, items, open_recent_submenu)?;
            }
            menu.append(&submenu)?;
        }
        Ok(())
    }

    /// Recursively build menu items within a submenu
    fn build_menu_item(
        parent: &Submenu,
        def: &MenuDef,
        items: &mut Vec<(MenuItem, MenuAction)>,
        open_recent_submenu: &mut Option<Submenu>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match def {
            MenuDef::Item(item_def) => {
                let accelerator = item_def.shortcut.as_ref().map(|s| s.to_muda_accelerator());
                let item = MenuItem::new(item_def.label, true, accelerator);
                items.push((item.clone(), item_def.action));
                parent.append(&item)?;
            }
            MenuDef::Separator => {
                parent.append(&PredefinedMenuItem::separator())?;
            }
            MenuDef::Submenu { label, children } => {
                let submenu = Submenu::new(*label, true);

                // Capture reference if this is "Open Recent"
                if *label == "Open Recent" {
                    *open_recent_submenu = Some(submenu.clone());
                }

                for child in *children {
                    Self::build_menu_item(&submenu, child, items, open_recent_submenu)?;
                }
                parent.append(&submenu)?;
            }
        }
        Ok(())
    }

    /// Update "Open Recent" submenu with current recent files
    /// Call this after menu creation and whenever recent files change
    pub fn update_recent_files(&mut self, recent_files: &[std::path::PathBuf]) {
        if let Some(submenu) = &self.open_recent_submenu {

            // Clear existing items
            while submenu.items().len() > 0 {
                let _ = submenu.remove_at(0);
            }

            // Add recent file items
            for (index, path) in recent_files.iter().enumerate() {
                let display_name = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let item = MenuItem::new(&display_name, true, None);
                if submenu.append(&item).is_ok() {
                    self.items.push((item.clone(), MenuAction::OpenRecent(index)));
                }
            }

            // Add separator and clear option if we have items
            if !recent_files.is_empty() {
                let _ = submenu.append(&PredefinedMenuItem::separator());
            }

            // Add "Clear Recent Files" item
            let clear_item = MenuItem::new("Clear Recent Files", true, None);
            if submenu.append(&clear_item).is_ok() {
                self.items.push((clear_item.clone(), MenuAction::ClearRecentFiles));
            }
        }
    }

    /// Initialize native menus for macOS (app-wide, doesn't require window handle)
    #[cfg(target_os = "macos")]
    pub fn init_for_macos(&self) {
        self.menu.init_for_nsapp();
    }

    /// Check if any menu item was triggered and return the action
    pub fn check_events(&self) -> Option<MenuAction> {
        for (item, action) in &self.items {
            if let Ok(event) = muda::MenuEvent::receiver().try_recv() {
                if event.id == item.id() {
                    return Some(*action);
                }
            }
        }
        None
    }

    /// Check keyboard shortcuts from egui input and return the action.
    /// If a KeymapManager is provided, uses remapped bindings; otherwise falls back to static defaults.
    pub fn check_shortcuts(input: &egui::InputState, keymap: Option<&crate::keymap::KeymapManager>) -> Option<MenuAction> {
        if let Some(km) = keymap {
            // Check all menu actions through the keymap
            for def in MenuItemDef::all_with_shortcuts() {
                if let Ok(app_action) = crate::keymap::AppAction::try_from(def.action) {
                    if km.action_pressed(app_action, input) {
                        return Some(def.action);
                    }
                }
            }
            None
        } else {
            for def in MenuItemDef::all_with_shortcuts() {
                if let Some(shortcut) = &def.shortcut {
                    if shortcut.matches_egui_input(input) {
                        return Some(def.action);
                    }
                }
            }
            None
        }
    }

    /// Measure the minimum width needed for a menu's contents.
    /// Accounts for label text + gap + shortcut text + padding.
    fn measure_menu_width(ui: &egui::Ui, children: &[MenuDef], keymap: Option<&crate::keymap::KeymapManager>) -> f32 {
        let label_font = egui::FontId::proportional(14.0);
        let shortcut_font = egui::FontId::proportional(12.0);
        let gap = 24.0; // space between label and shortcut
        let padding = 16.0; // left + right padding

        let mut max_width: f32 = 0.0;
        for child in children {
            match child {
                MenuDef::Item(item_def) => {
                    let label_width = ui.fonts_mut(|f| f.layout_no_wrap(item_def.label.to_string(), label_font.clone(), egui::Color32::WHITE).size().x);
                    let effective_shortcut = if let Some(km) = keymap {
                        if let Ok(app_action) = crate::keymap::AppAction::try_from(item_def.action) {
                            km.get(app_action)
                        } else {
                            item_def.shortcut
                        }
                    } else {
                        item_def.shortcut
                    };
                    let shortcut_width = if let Some(shortcut) = &effective_shortcut {
                        let text = Self::format_shortcut(shortcut);
                        ui.fonts_mut(|f| f.layout_no_wrap(text, shortcut_font.clone(), egui::Color32::WHITE).size().x) + gap
                    } else {
                        0.0
                    };
                    max_width = max_width.max(label_width + shortcut_width);
                }
                MenuDef::Submenu { label, .. } => {
                    let label_width = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), label_font.clone(), egui::Color32::WHITE).size().x);
                    max_width = max_width.max(label_width + 20.0); // extra space for submenu arrow
                }
                MenuDef::Separator => {}
            }
        }
        max_width + padding
    }

    /// Render egui menu bar from the same menu structure (for Linux/Windows)
    pub fn render_egui_menu_bar(
        &self,
        ui: &mut egui::Ui,
        recent_files: &[std::path::PathBuf],
        keymap: Option<&crate::keymap::KeymapManager>,
        layout_names: &[String],
        current_layout_index: usize,
        checked_actions: &[MenuAction],
        hidden_actions: &[MenuAction],
    ) -> Option<MenuAction> {
        let mut action = None;
        let ctx = ui.ctx().clone();
        let menus = MenuItemDef::menu_structure();

        egui::MenuBar::new().ui(ui, |ui| {
            // Phase 1: render all top-level buttons and collect responses.
            // For non-submenu items (separators, bare actions), render them inline.
            let mut button_entries: Vec<(egui::Response, egui::Id, &MenuDef)> = Vec::new();
            for menu_def in menus {
                if let MenuDef::Submenu { label, .. } = menu_def {
                    let response = ui.button(*label);
                    let popup_id = egui::Popup::default_response_id(&response);
                    button_entries.push((response, popup_id, menu_def));
                } else if let Some(a) = self.render_menu_def(ui, menu_def, recent_files, keymap, layout_names, current_layout_index, checked_actions, hidden_actions) {
                    action = Some(a);
                }
            }

            // Phase 2: hover-to-switch between top-level menus.
            // If one of our menu popups is open and the user hovers a different button, switch.
            let any_ours_open = button_entries.iter().any(|(_, pid, _)| egui::Popup::is_id_open(&ctx, *pid));
            if any_ours_open {
                for (response, popup_id, _) in &button_entries {
                    if response.hovered() && !egui::Popup::is_id_open(&ctx, *popup_id) {
                        // open_id closes all other popups and opens this one
                        egui::Popup::open_id(&ctx, *popup_id);
                        break;
                    }
                }
            }

            // Phase 3: show popups via standard Popup::menu.
            // Popup::menu sets UiKind::Menu, Frame::popup, menu_style, and MenuState::mark_shown,
            // so SubMenuButton works correctly for nested submenus.
            for (response, _, menu_def) in button_entries {
                if let MenuDef::Submenu { children, .. } = menu_def {
                    let popup_result = egui::Popup::menu(&response).show(|ui| {
                        let min_width = Self::measure_menu_width(ui, children, keymap);
                        ui.set_width(min_width);
                        let mut a = None;
                        for child in *children {
                            if let Some(result) = self.render_menu_def(ui, child, recent_files, keymap, layout_names, current_layout_index, checked_actions, hidden_actions) {
                                a = Some(result);
                                ui.close();
                            }
                        }
                        a
                    });
                    if let Some(r) = popup_result {
                        if let Some(a) = r.inner {
                            action = Some(a);
                        }
                    }
                }
            }
        });

        action
    }

    /// Recursively render a MenuDef as egui UI
    fn render_menu_def(
        &self,
        ui: &mut egui::Ui,
        def: &MenuDef,
        recent_files: &[std::path::PathBuf],
        keymap: Option<&crate::keymap::KeymapManager>,
        layout_names: &[String],
        current_layout_index: usize,
        checked_actions: &[MenuAction],
        hidden_actions: &[MenuAction],
    ) -> Option<MenuAction> {
        match def {
            MenuDef::Item(item_def) => {
                if hidden_actions.contains(&item_def.action) {
                    return None;
                }
                if Self::render_menu_item(ui, item_def, keymap, checked_actions) {
                    Some(item_def.action)
                } else {
                    None
                }
            }
            MenuDef::Separator => {
                ui.separator();
                None
            }
            MenuDef::Submenu { label, children } => {
                let (_, popup) = egui::containers::menu::SubMenuButton::new(*label)
                    .ui(ui, |ui| {
                        if *label == "Open Recent" {
                            let mut action = None;
                            for (index, path) in recent_files.iter().enumerate() {
                                let display_name = path
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("Unknown");
                                if ui.button(display_name).clicked() {
                                    action = Some(MenuAction::OpenRecent(index));
                                    ui.close();
                                }
                            }
                            if !recent_files.is_empty() {
                                ui.separator();
                            }
                            if ui.button("Clear Recent Files").clicked() {
                                action = Some(MenuAction::ClearRecentFiles);
                                ui.close();
                            }
                            action
                        } else if *label == "Layout" {
                            let mut action = None;
                            for child in *children {
                                if let Some(a) = self.render_menu_def(ui, child, recent_files, keymap, layout_names, current_layout_index, checked_actions, hidden_actions) {
                                    action = Some(a);
                                    ui.close();
                                }
                            }
                            if !layout_names.is_empty() {
                                ui.separator();
                                for (index, name) in layout_names.iter().enumerate() {
                                    let entry = if index == current_layout_index {
                                        format!("* {}", name)
                                    } else {
                                        name.clone()
                                    };
                                    if ui.button(entry).clicked() {
                                        action = Some(MenuAction::SwitchLayout(index));
                                        ui.close();
                                    }
                                }
                            }
                            action
                        } else {
                            let mut action = None;
                            for child in *children {
                                if let Some(a) = self.render_menu_def(ui, child, recent_files, keymap, layout_names, current_layout_index, checked_actions, hidden_actions) {
                                    action = Some(a);
                                    ui.close();
                                }
                            }
                            action
                        }
                    });
                popup.and_then(|r| r.inner)
            }
        }
    }

    /// Render a single menu item with label and shortcut
    fn render_menu_item(ui: &mut egui::Ui, def: &MenuItemDef, keymap: Option<&crate::keymap::KeymapManager>, checked_actions: &[MenuAction]) -> bool {
        // Look up shortcut from keymap if available, otherwise use static default
        let effective_shortcut = if let Some(km) = keymap {
            if let Ok(app_action) = crate::keymap::AppAction::try_from(def.action) {
                km.get(app_action)
            } else {
                def.shortcut
            }
        } else {
            def.shortcut
        };
        let shortcut_text = if let Some(shortcut) = &effective_shortcut {
            Self::format_shortcut(shortcut)
        } else {
            String::new()
        };

        let desired_width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(desired_width, ui.spacing().interact_size.y),
            egui::Sense::click(),
        );

        if ui.is_rect_visible(rect) {
            // Highlight on hover
            if response.hovered() {
                ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
            }

            // Draw label text left-aligned
            let text_color = if response.hovered() {
                ui.visuals().widgets.hovered.text_color()
            } else {
                ui.visuals().widgets.inactive.text_color()
            };
            let label_pos = rect.min + egui::vec2(4.0, (rect.height() - 14.0) / 2.0);
            let label = if checked_actions.contains(&def.action) {
                format!("✔ {}", def.label)
            } else {
                def.label.to_owned()
            };
            ui.painter().text(
                label_pos,
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(14.0),
                text_color,
            );

            // Draw shortcut text right-aligned
            if !shortcut_text.is_empty() {
                let shortcut_pos = rect.max - egui::vec2(4.0, (rect.height() - 12.0) / 2.0);
                ui.painter().text(
                    shortcut_pos,
                    egui::Align2::RIGHT_BOTTOM,
                    &shortcut_text,
                    egui::FontId::proportional(12.0),
                    ui.visuals().weak_text_color(),
                );
            }
        }

        response.clicked()
    }

    /// Format shortcut for display (e.g., "Ctrl+S")
    pub fn format_shortcut(shortcut: &Shortcut) -> String {
        let mut parts = Vec::new();

        if shortcut.ctrl {
            parts.push("Ctrl");
        }
        if shortcut.shift {
            parts.push("Shift");
        }
        if shortcut.alt {
            parts.push("Alt");
        }

        parts.push(shortcut.key.display_name());

        parts.join("+")
    }

    /// Update native menu accelerator labels to match the current keymap
    pub fn apply_keybindings(&self, keymap: &crate::keymap::KeymapManager) {
        for (item, menu_action) in &self.items {
            if let Ok(app_action) = crate::keymap::AppAction::try_from(*menu_action) {
                let accelerator = keymap.get(app_action)
                    .map(|s| s.to_muda_accelerator());
                let _ = item.set_accelerator(accelerator);
            }
        }
    }

    /// Update menu item text dynamically (e.g., for Undo/Redo with action names)
    #[allow(dead_code)]
    pub fn update_undo_text(&self, action_name: Option<&str>) {
        // Find the Undo menu item and update its text
        for (item, action) in &self.items {
            if *action == MenuAction::Undo {
                let text = if let Some(name) = action_name {
                    format!("Undo {}", name)
                } else {
                    "Undo".to_string()
                };
                let _ = item.set_text(text);
                break;
            }
        }
    }

    /// Update menu item text dynamically for Redo
    #[allow(dead_code)]
    pub fn update_redo_text(&self, action_name: Option<&str>) {
        for (item, action) in &self.items {
            if *action == MenuAction::Redo {
                let text = if let Some(name) = action_name {
                    format!("Redo {}", name)
                } else {
                    "Redo".to_string()
                };
                let _ = item.set_text(text);
                break;
            }
        }
    }

    /// Enable or disable a menu item
    #[allow(dead_code)]
    pub fn set_enabled(&self, action: MenuAction, enabled: bool) {
        for (item, item_action) in &self.items {
            if *item_action == action {
                let _ = item.set_enabled(enabled);
                break;
            }
        }
    }
}
