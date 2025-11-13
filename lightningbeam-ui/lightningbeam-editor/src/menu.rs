/// Native menu implementation using muda
///
/// This module creates the native menu bar with all menu items matching
/// the JavaScript version's menu structure.

use muda::{
    accelerator::{Accelerator, Code, Modifiers},
    Menu, MenuItem, PredefinedMenuItem, Submenu,
};

/// All possible menu actions that can be triggered
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    // File menu
    NewFile,
    NewWindow,
    Save,
    SaveAs,
    OpenFile,
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

/// Menu system that holds all menu items and can dispatch actions
pub struct MenuSystem {
    #[allow(dead_code)]
    menu: Menu,
    items: Vec<(MenuItem, MenuAction)>,
}

impl MenuSystem {
    /// Create a new menu system with all menus and items
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let menu = Menu::new();
        let mut items = Vec::new();

        // Platform-specific: Add "Lightningbeam" menu on macOS
        #[cfg(target_os = "macos")]
        {
            let app_menu = Submenu::new("Lightningbeam", true);

            let about_item = MenuItem::new("About Lightningbeam", true, None);
            items.push((about_item.clone(), MenuAction::About));
            app_menu.append(&about_item)?;

            app_menu.append(&PredefinedMenuItem::separator())?;

            let settings_item = MenuItem::new(
                "Settings",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::Comma)),
            );
            items.push((settings_item.clone(), MenuAction::Settings));
            app_menu.append(&settings_item)?;

            app_menu.append(&PredefinedMenuItem::separator())?;

            let close_item = MenuItem::new(
                "Close Window",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyW)),
            );
            items.push((close_item.clone(), MenuAction::CloseWindow));
            app_menu.append(&close_item)?;

            let quit_item = MenuItem::new(
                "Quit Lightningbeam",
                true,
                Some(Accelerator::new(Some(Modifiers::META), Code::KeyQ)),
            );
            items.push((quit_item.clone(), MenuAction::Quit));
            app_menu.append(&quit_item)?;

            menu.append(&app_menu)?;
        }

        // File menu
        let file_menu = Submenu::new("File", true);

        let new_file = MenuItem::new(
            "New file...",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyN)),
        );
        items.push((new_file.clone(), MenuAction::NewFile));
        file_menu.append(&new_file)?;

        let new_window = MenuItem::new(
            "New Window",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyN,
            )),
        );
        items.push((new_window.clone(), MenuAction::NewWindow));
        file_menu.append(&new_window)?;

        file_menu.append(&PredefinedMenuItem::separator())?;

        let save = MenuItem::new(
            "Save",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyS)),
        );
        items.push((save.clone(), MenuAction::Save));
        file_menu.append(&save)?;

        let save_as = MenuItem::new(
            "Save As...",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyS,
            )),
        );
        items.push((save_as.clone(), MenuAction::SaveAs));
        file_menu.append(&save_as)?;

        file_menu.append(&PredefinedMenuItem::separator())?;

        // Open Recent submenu (placeholder for now)
        let open_recent = Submenu::new("Open Recent", true);
        file_menu.append(&open_recent)?;

        let open_file = MenuItem::new(
            "Open File...",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyO)),
        );
        items.push((open_file.clone(), MenuAction::OpenFile));
        file_menu.append(&open_file)?;

        let revert = MenuItem::new("Revert", true, None);
        items.push((revert.clone(), MenuAction::Revert));
        file_menu.append(&revert)?;

        file_menu.append(&PredefinedMenuItem::separator())?;

        let import = MenuItem::new(
            "Import...",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyI,
            )),
        );
        items.push((import.clone(), MenuAction::Import));
        file_menu.append(&import)?;

        let export = MenuItem::new(
            "Export...",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyE,
            )),
        );
        items.push((export.clone(), MenuAction::Export));
        file_menu.append(&export)?;

        // On non-macOS, add Quit to File menu
        #[cfg(not(target_os = "macos"))]
        {
            file_menu.append(&PredefinedMenuItem::separator())?;
            let quit = MenuItem::new(
                "Quit",
                true,
                Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyQ)),
            );
            items.push((quit.clone(), MenuAction::Quit));
            file_menu.append(&quit)?;
        }

        menu.append(&file_menu)?;

        // Edit menu
        let edit_menu = Submenu::new("Edit", true);

        let undo = MenuItem::new(
            "Undo",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyZ)),
        );
        items.push((undo.clone(), MenuAction::Undo));
        edit_menu.append(&undo)?;

        let redo = MenuItem::new(
            "Redo",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyZ,
            )),
        );
        items.push((redo.clone(), MenuAction::Redo));
        edit_menu.append(&redo)?;

        edit_menu.append(&PredefinedMenuItem::separator())?;

        let cut = MenuItem::new(
            "Cut",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyX)),
        );
        items.push((cut.clone(), MenuAction::Cut));
        edit_menu.append(&cut)?;

        let copy = MenuItem::new(
            "Copy",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyC)),
        );
        items.push((copy.clone(), MenuAction::Copy));
        edit_menu.append(&copy)?;

        let paste = MenuItem::new(
            "Paste",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyV)),
        );
        items.push((paste.clone(), MenuAction::Paste));
        edit_menu.append(&paste)?;

        let delete = MenuItem::new(
            "Delete",
            true,
            Some(Accelerator::new(None, Code::Delete)),
        );
        items.push((delete.clone(), MenuAction::Delete));
        edit_menu.append(&delete)?;

        edit_menu.append(&PredefinedMenuItem::separator())?;

        let select_all = MenuItem::new(
            "Select All",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyA)),
        );
        items.push((select_all.clone(), MenuAction::SelectAll));
        edit_menu.append(&select_all)?;

        let select_none = MenuItem::new(
            "Select None",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyA,
            )),
        );
        items.push((select_none.clone(), MenuAction::SelectNone));
        edit_menu.append(&select_none)?;

        edit_menu.append(&PredefinedMenuItem::separator())?;

        let preferences = MenuItem::new("Preferences", true, None);
        items.push((preferences.clone(), MenuAction::Preferences));
        edit_menu.append(&preferences)?;

        menu.append(&edit_menu)?;

        // Modify menu
        let modify_menu = Submenu::new("Modify", true);

        let group = MenuItem::new(
            "Group",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::KeyG)),
        );
        items.push((group.clone(), MenuAction::Group));
        modify_menu.append(&group)?;

        modify_menu.append(&PredefinedMenuItem::separator())?;

        let send_to_back = MenuItem::new("Send to back", true, None);
        items.push((send_to_back.clone(), MenuAction::SendToBack));
        modify_menu.append(&send_to_back)?;

        let bring_to_front = MenuItem::new("Bring to front", true, None);
        items.push((bring_to_front.clone(), MenuAction::BringToFront));
        modify_menu.append(&bring_to_front)?;

        menu.append(&modify_menu)?;

        // Layer menu
        let layer_menu = Submenu::new("Layer", true);

        let add_layer = MenuItem::new(
            "Add Layer",
            true,
            Some(Accelerator::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::KeyL,
            )),
        );
        items.push((add_layer.clone(), MenuAction::AddLayer));
        layer_menu.append(&add_layer)?;

        let add_video_layer = MenuItem::new("Add Video Layer", true, None);
        items.push((add_video_layer.clone(), MenuAction::AddVideoLayer));
        layer_menu.append(&add_video_layer)?;

        let add_audio_track = MenuItem::new("Add Audio Track", true, None);
        items.push((add_audio_track.clone(), MenuAction::AddAudioTrack));
        layer_menu.append(&add_audio_track)?;

        let add_midi_track = MenuItem::new("Add MIDI Track", true, None);
        items.push((add_midi_track.clone(), MenuAction::AddMidiTrack));
        layer_menu.append(&add_midi_track)?;

        layer_menu.append(&PredefinedMenuItem::separator())?;

        let delete_layer = MenuItem::new("Delete Layer", true, None);
        items.push((delete_layer.clone(), MenuAction::DeleteLayer));
        layer_menu.append(&delete_layer)?;

        let toggle_layer = MenuItem::new("Hide/Show Layer", true, None);
        items.push((toggle_layer.clone(), MenuAction::ToggleLayerVisibility));
        layer_menu.append(&toggle_layer)?;

        menu.append(&layer_menu)?;

        // Timeline menu
        let timeline_menu = Submenu::new("Timeline", true);

        let new_keyframe = MenuItem::new(
            "New Keyframe",
            true,
            Some(Accelerator::new(None, Code::KeyK)),
        );
        items.push((new_keyframe.clone(), MenuAction::NewKeyframe));
        timeline_menu.append(&new_keyframe)?;

        let new_blank_keyframe = MenuItem::new("New Blank Keyframe", true, None);
        items.push((new_blank_keyframe.clone(), MenuAction::NewBlankKeyframe));
        timeline_menu.append(&new_blank_keyframe)?;

        let delete_frame = MenuItem::new("Delete Frame", true, None);
        items.push((delete_frame.clone(), MenuAction::DeleteFrame));
        timeline_menu.append(&delete_frame)?;

        let duplicate_keyframe = MenuItem::new("Duplicate Keyframe", true, None);
        items.push((duplicate_keyframe.clone(), MenuAction::DuplicateKeyframe));
        timeline_menu.append(&duplicate_keyframe)?;

        let add_keyframe_playhead = MenuItem::new("Add Keyframe at Playhead", true, None);
        items.push((add_keyframe_playhead.clone(), MenuAction::AddKeyframeAtPlayhead));
        timeline_menu.append(&add_keyframe_playhead)?;

        timeline_menu.append(&PredefinedMenuItem::separator())?;

        let motion_tween = MenuItem::new("Add Motion Tween", true, None);
        items.push((motion_tween.clone(), MenuAction::AddMotionTween));
        timeline_menu.append(&motion_tween)?;

        let shape_tween = MenuItem::new("Add Shape Tween", true, None);
        items.push((shape_tween.clone(), MenuAction::AddShapeTween));
        timeline_menu.append(&shape_tween)?;

        timeline_menu.append(&PredefinedMenuItem::separator())?;

        let return_to_start = MenuItem::new("Return to start", true, None);
        items.push((return_to_start.clone(), MenuAction::ReturnToStart));
        timeline_menu.append(&return_to_start)?;

        let play = MenuItem::new("Play", true, None);
        items.push((play.clone(), MenuAction::Play));
        timeline_menu.append(&play)?;

        menu.append(&timeline_menu)?;

        // View menu
        let view_menu = Submenu::new("View", true);

        let zoom_in = MenuItem::new(
            "Zoom In",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::Equal)),
        );
        items.push((zoom_in.clone(), MenuAction::ZoomIn));
        view_menu.append(&zoom_in)?;

        let zoom_out = MenuItem::new(
            "Zoom Out",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::Minus)),
        );
        items.push((zoom_out.clone(), MenuAction::ZoomOut));
        view_menu.append(&zoom_out)?;

        let actual_size = MenuItem::new(
            "Actual Size",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::Digit0)),
        );
        items.push((actual_size.clone(), MenuAction::ActualSize));
        view_menu.append(&actual_size)?;

        let recenter = MenuItem::new("Recenter View", true, None);
        items.push((recenter.clone(), MenuAction::RecenterView));
        view_menu.append(&recenter)?;

        view_menu.append(&PredefinedMenuItem::separator())?;

        // Layout submenu
        let layout_submenu = Submenu::new("Layout", true);

        let next_layout = MenuItem::new(
            "Next Layout",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::BracketRight)),
        );
        items.push((next_layout.clone(), MenuAction::NextLayout));
        layout_submenu.append(&next_layout)?;

        let prev_layout = MenuItem::new(
            "Previous Layout",
            true,
            Some(Accelerator::new(Some(Modifiers::CONTROL), Code::BracketLeft)),
        );
        items.push((prev_layout.clone(), MenuAction::PreviousLayout));
        layout_submenu.append(&prev_layout)?;

        // TODO: Add dynamic layout list with checkmarks for current layout
        // This will need to be updated when layouts change

        view_menu.append(&layout_submenu)?;
        menu.append(&view_menu)?;

        // Help menu
        let help_menu = Submenu::new("Help", true);

        let about = MenuItem::new("About...", true, None);
        items.push((about.clone(), MenuAction::About));
        help_menu.append(&about)?;

        menu.append(&help_menu)?;

        Ok(Self { menu, items })
    }

    /// Initialize the menu for the application window
    #[cfg(target_os = "linux")]
    pub fn init_for_gtk(&self, window: &gtk::ApplicationWindow, container: Option<&gtk::Box>) -> Result<(), Box<dyn std::error::Error>> {
        self.menu.init_for_gtk_window(window, container)?;
        Ok(())
    }

    /// Initialize the menu for macOS (app-wide)
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
