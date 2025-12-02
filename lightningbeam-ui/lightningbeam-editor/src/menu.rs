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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    pub key: ShortcutKey,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// Keys that can be used in shortcuts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
    // Letters
    A, C, E, G, I, K, L, N, O, Q, S, V, W, X, Z,
    // Numbers
    Num0,
    // Symbols
    Comma, Minus, Equals, Plus,
    BracketLeft, BracketRight,
    // Special
    Delete,
}

impl Shortcut {
    pub const fn new(key: ShortcutKey, ctrl: bool, shift: bool, alt: bool) -> Self {
        Self { key, ctrl, shift, alt }
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
            ShortcutKey::C => Code::KeyC,
            ShortcutKey::E => Code::KeyE,
            ShortcutKey::G => Code::KeyG,
            ShortcutKey::I => Code::KeyI,
            ShortcutKey::K => Code::KeyK,
            ShortcutKey::L => Code::KeyL,
            ShortcutKey::N => Code::KeyN,
            ShortcutKey::O => Code::KeyO,
            ShortcutKey::Q => Code::KeyQ,
            ShortcutKey::S => Code::KeyS,
            ShortcutKey::V => Code::KeyV,
            ShortcutKey::W => Code::KeyW,
            ShortcutKey::X => Code::KeyX,
            ShortcutKey::Z => Code::KeyZ,
            ShortcutKey::Num0 => Code::Digit0,
            ShortcutKey::Comma => Code::Comma,
            ShortcutKey::Minus => Code::Minus,
            ShortcutKey::Equals => Code::Equal,
            ShortcutKey::Plus => Code::Equal, // Same key as equals
            ShortcutKey::BracketLeft => Code::BracketLeft,
            ShortcutKey::BracketRight => Code::BracketRight,
            ShortcutKey::Delete => Code::Delete,
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

        // Check key
        let key = match self.key {
            ShortcutKey::A => egui::Key::A,
            ShortcutKey::C => egui::Key::C,
            ShortcutKey::E => egui::Key::E,
            ShortcutKey::G => egui::Key::G,
            ShortcutKey::I => egui::Key::I,
            ShortcutKey::K => egui::Key::K,
            ShortcutKey::L => egui::Key::L,
            ShortcutKey::N => egui::Key::N,
            ShortcutKey::O => egui::Key::O,
            ShortcutKey::Q => egui::Key::Q,
            ShortcutKey::S => egui::Key::S,
            ShortcutKey::V => egui::Key::V,
            ShortcutKey::W => egui::Key::W,
            ShortcutKey::X => egui::Key::X,
            ShortcutKey::Z => egui::Key::Z,
            ShortcutKey::Num0 => egui::Key::Num0,
            ShortcutKey::Comma => egui::Key::Comma,
            ShortcutKey::Minus => egui::Key::Minus,
            ShortcutKey::Equals => egui::Key::Equals,
            ShortcutKey::Plus => egui::Key::Plus,
            ShortcutKey::BracketLeft => egui::Key::OpenBracket,
            ShortcutKey::BracketRight => egui::Key::CloseBracket,
            ShortcutKey::Delete => egui::Key::Delete,
        };

        input.key_pressed(key)
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
    SendToBack,
    BringToFront,

    // Layer menu
    AddLayer,
    AddVideoLayer,
    AddAudioTrack,
    AddMidiTrack,
    AddTestClip, // For testing: adds a test clip to the asset library
    DeleteLayer,
    ToggleLayerVisibility,

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

    // View menu
    ZoomIn,
    ZoomOut,
    ActualSize,
    RecenterView,
    NextLayout,
    PreviousLayout,
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
    const IMPORT: Self = Self { label: "Import...", action: MenuAction::Import, shortcut: Some(Shortcut::new(ShortcutKey::I, CTRL, SHIFT, NO_ALT)) };
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
    const SEND_TO_BACK: Self = Self { label: "Send to back", action: MenuAction::SendToBack, shortcut: None };
    const BRING_TO_FRONT: Self = Self { label: "Bring to front", action: MenuAction::BringToFront, shortcut: None };

    // Layer menu items
    const ADD_LAYER: Self = Self { label: "Add Layer", action: MenuAction::AddLayer, shortcut: Some(Shortcut::new(ShortcutKey::L, CTRL, SHIFT, NO_ALT)) };
    const ADD_VIDEO_LAYER: Self = Self { label: "Add Video Layer", action: MenuAction::AddVideoLayer, shortcut: None };
    const ADD_AUDIO_TRACK: Self = Self { label: "Add Audio Track", action: MenuAction::AddAudioTrack, shortcut: None };
    const ADD_MIDI_TRACK: Self = Self { label: "Add MIDI Track", action: MenuAction::AddMidiTrack, shortcut: None };
    const ADD_TEST_CLIP: Self = Self { label: "Add Test Clip to Library", action: MenuAction::AddTestClip, shortcut: None };
    const DELETE_LAYER: Self = Self { label: "Delete Layer", action: MenuAction::DeleteLayer, shortcut: None };
    const TOGGLE_LAYER_VISIBILITY: Self = Self { label: "Hide/Show Layer", action: MenuAction::ToggleLayerVisibility, shortcut: None };

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

    // View menu items
    const ZOOM_IN: Self = Self { label: "Zoom In", action: MenuAction::ZoomIn, shortcut: Some(Shortcut::new(ShortcutKey::Equals, CTRL, NO_SHIFT, NO_ALT)) };
    const ZOOM_OUT: Self = Self { label: "Zoom Out", action: MenuAction::ZoomOut, shortcut: Some(Shortcut::new(ShortcutKey::Minus, CTRL, NO_SHIFT, NO_ALT)) };
    const ACTUAL_SIZE: Self = Self { label: "Actual Size", action: MenuAction::ActualSize, shortcut: Some(Shortcut::new(ShortcutKey::Num0, CTRL, NO_SHIFT, NO_ALT)) };
    const RECENTER_VIEW: Self = Self { label: "Recenter View", action: MenuAction::RecenterView, shortcut: None };
    const NEXT_LAYOUT: Self = Self { label: "Next Layout", action: MenuAction::NextLayout, shortcut: Some(Shortcut::new(ShortcutKey::BracketRight, CTRL, NO_SHIFT, NO_ALT)) };
    const PREVIOUS_LAYOUT: Self = Self { label: "Previous Layout", action: MenuAction::PreviousLayout, shortcut: Some(Shortcut::new(ShortcutKey::BracketLeft, CTRL, NO_SHIFT, NO_ALT)) };

    // Help menu items
    const ABOUT: Self = Self { label: "About...", action: MenuAction::About, shortcut: None };

    // macOS app menu items
    const SETTINGS: Self = Self { label: "Settings", action: MenuAction::Settings, shortcut: Some(Shortcut::new(ShortcutKey::Comma, CTRL, NO_SHIFT, NO_ALT)) };
    const CLOSE_WINDOW: Self = Self { label: "Close Window", action: MenuAction::CloseWindow, shortcut: Some(Shortcut::new(ShortcutKey::W, CTRL, NO_SHIFT, NO_ALT)) };
    const QUIT_MACOS: Self = Self { label: "Quit Lightningbeam", action: MenuAction::Quit, shortcut: Some(Shortcut::new(ShortcutKey::Q, CTRL, NO_SHIFT, NO_ALT)) };
    const ABOUT_MACOS: Self = Self { label: "About Lightningbeam", action: MenuAction::About, shortcut: None };

    /// Get all menu items with shortcuts (for keyboard handling)
    pub fn all_with_shortcuts() -> &'static [&'static MenuItemDef] {
        &[
            &Self::NEW_FILE, &Self::NEW_WINDOW, &Self::SAVE, &Self::SAVE_AS,
            &Self::OPEN_FILE, &Self::IMPORT, &Self::EXPORT, &Self::QUIT,
            &Self::UNDO, &Self::REDO, &Self::CUT, &Self::COPY, &Self::PASTE,
            &Self::DELETE, &Self::SELECT_ALL, &Self::SELECT_NONE,
            &Self::GROUP, &Self::ADD_LAYER, &Self::NEW_KEYFRAME,
            &Self::ZOOM_IN, &Self::ZOOM_OUT, &Self::ACTUAL_SIZE,
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
                    MenuDef::Separator,
                    MenuDef::Item(&Self::SEND_TO_BACK),
                    MenuDef::Item(&Self::BRING_TO_FRONT),
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
                    MenuDef::Separator,
                    MenuDef::Item(&Self::ADD_TEST_CLIP),
                    MenuDef::Separator,
                    MenuDef::Item(&Self::DELETE_LAYER),
                    MenuDef::Item(&Self::TOGGLE_LAYER_VISIBILITY),
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

    /// Check keyboard shortcuts from egui input and return the action
    /// This works cross-platform and complements native menus
    pub fn check_shortcuts(input: &egui::InputState) -> Option<MenuAction> {
        for def in MenuItemDef::all_with_shortcuts() {
            if let Some(shortcut) = &def.shortcut {
                if shortcut.matches_egui_input(input) {
                    return Some(def.action);
                }
            }
        }
        None
    }

    /// Render egui menu bar from the same menu structure (for Linux/Windows)
    pub fn render_egui_menu_bar(&self, ui: &mut egui::Ui, recent_files: &[std::path::PathBuf]) -> Option<MenuAction> {
        let mut action = None;

        egui::menu::bar(ui, |ui| {
            for menu_def in MenuItemDef::menu_structure() {
                if let Some(a) = self.render_menu_def(ui, menu_def, recent_files) {
                    action = Some(a);
                }
            }
        });

        action
    }

    /// Recursively render a MenuDef as egui UI
    fn render_menu_def(&self, ui: &mut egui::Ui, def: &MenuDef, recent_files: &[std::path::PathBuf]) -> Option<MenuAction> {
        match def {
            MenuDef::Item(item_def) => {
                if Self::render_menu_item(ui, item_def) {
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
                let mut action = None;
                ui.menu_button(*label, |ui| {
                    // Special handling for "Open Recent" submenu
                    if *label == "Open Recent" {
                        // Render dynamic recent files
                        for (index, path) in recent_files.iter().enumerate() {
                            let display_name = path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .unwrap_or("Unknown");

                            if ui.button(display_name).clicked() {
                                action = Some(MenuAction::OpenRecent(index));
                                ui.close_menu();
                            }
                        }

                        // Add separator and clear option if we have items
                        if !recent_files.is_empty() {
                            ui.separator();
                        }

                        if ui.button("Clear Recent Files").clicked() {
                            action = Some(MenuAction::ClearRecentFiles);
                            ui.close_menu();
                        }
                    } else {
                        // Normal submenu rendering
                        for child in *children {
                            if let Some(a) = self.render_menu_def(ui, child, recent_files) {
                                action = Some(a);
                                ui.close_menu();
                            }
                        }
                    }
                });
                action
            }
        }
    }

    /// Render a single menu item with label and shortcut
    fn render_menu_item(ui: &mut egui::Ui, def: &MenuItemDef) -> bool {
        let shortcut_text = if let Some(shortcut) = &def.shortcut {
            Self::format_shortcut(shortcut)
        } else {
            String::new()
        };

        // Set minimum width for menu items to prevent cramping
        ui.set_min_width(180.0);

        if shortcut_text.is_empty() {
            ui.add(egui::Button::new(def.label).min_size(egui::vec2(0.0, 0.0))).clicked()
        } else {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 20.0; // More space between label and shortcut

                let button = ui.add(egui::Button::new(def.label).min_size(egui::vec2(0.0, 0.0)));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(&shortcut_text).weak().size(12.0));
                });

                button.clicked()
            }).inner
        }
    }

    /// Format shortcut for display (e.g., "Ctrl+S")
    fn format_shortcut(shortcut: &Shortcut) -> String {
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

        let key_name = match shortcut.key {
            ShortcutKey::A => "A",
            ShortcutKey::C => "C",
            ShortcutKey::E => "E",
            ShortcutKey::G => "G",
            ShortcutKey::I => "I",
            ShortcutKey::K => "K",
            ShortcutKey::L => "L",
            ShortcutKey::N => "N",
            ShortcutKey::O => "O",
            ShortcutKey::Q => "Q",
            ShortcutKey::S => "S",
            ShortcutKey::V => "V",
            ShortcutKey::W => "W",
            ShortcutKey::X => "X",
            ShortcutKey::Z => "Z",
            ShortcutKey::Num0 => "0",
            ShortcutKey::Comma => ",",
            ShortcutKey::Minus => "-",
            ShortcutKey::Equals => "=",
            ShortcutKey::Plus => "+",
            ShortcutKey::BracketLeft => "[",
            ShortcutKey::BracketRight => "]",
            ShortcutKey::Delete => "Del",
        };
        parts.push(key_name);

        parts.join("+")
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
