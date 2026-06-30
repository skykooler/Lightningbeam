//! Mobile / phone UI shell.
//!
//! Developed on desktop behind the `LB_MOBILE_UI` env var (see [`is_mobile_env`]).
//!
//! The shell is a single **vertical sliding-window stack** of the existing panes: a fixed
//! top→bottom order, with a window of 2 or 3 consecutive panes visible at once. Dragging the
//! dividers between panes and the top/bottom screen edges grows, shrinks, or slides the window
//! (the R1–R6 state machine in `stack.rs`). A transport bar is pinned at the bottom as the floor.
//!
//! Each visible pane reuses the existing `PaneInstance::render_content` full-bleed (see
//! `surface.rs`). Per the wireframe, the Toolbar becomes the omnibutton and the Infopanel becomes
//! the selection inspector (later phases), so neither is in the stack.

use eframe::egui;
use lightningbeam_core::pane::PaneType;

use crate::panes::NodePath;
use crate::RenderContext;

pub mod icons;
mod stack;
mod surface;
mod transport;

/// Reserved sentinel namespace for mobile pane-instance paths. Desktop layout paths are built
/// from small child indices, so prefixing with `usize::MAX` guarantees mobile slots never alias a
/// real layout path in the shared `pane_instances` map.
pub const MOBILE_NS: usize = usize::MAX;

const TRANSPORT_H: f32 = 60.0;

/// Returns true if the mobile UI is requested via the `LB_MOBILE_UI` env var.
/// Any non-empty value other than "0" enables it.
pub fn is_mobile_env() -> bool {
    std::env::var("LB_MOBILE_UI")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// The panes that make up the vertical stack, in fixed top→bottom order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackPane {
    Outliner,
    AssetLibrary,
    Stage,
    Timeline,
    PianoRoll,
    VirtualPiano,
    /// Node editor, or (toggled) the instrument/preset browser.
    NodeInstrument,
    ScriptEditor,
}

/// The stack order. Index into this is a pane's stable slot id.
pub const STACK: [StackPane; 8] = [
    StackPane::Outliner,
    StackPane::AssetLibrary,
    StackPane::Stage,
    StackPane::Timeline,
    StackPane::PianoRoll,
    StackPane::VirtualPiano,
    StackPane::NodeInstrument,
    StackPane::ScriptEditor,
];

impl StackPane {
    /// The backing pane type. The Node/Instrument slot picks NodeEditor or PresetBrowser based on
    /// the toggle.
    pub fn pane_type(self, show_instruments: bool) -> PaneType {
        match self {
            StackPane::Outliner => PaneType::Outliner,
            StackPane::AssetLibrary => PaneType::AssetLibrary,
            StackPane::Stage => PaneType::Stage,
            StackPane::Timeline => PaneType::Timeline,
            StackPane::PianoRoll => PaneType::PianoRoll,
            StackPane::VirtualPiano => PaneType::VirtualPiano,
            StackPane::NodeInstrument => {
                if show_instruments {
                    PaneType::PresetBrowser
                } else {
                    PaneType::NodeEditor
                }
            }
            StackPane::ScriptEditor => PaneType::ScriptEditor,
        }
    }

    pub fn label(self, show_instruments: bool) -> &'static str {
        match self {
            StackPane::Outliner => "Outliner",
            StackPane::AssetLibrary => "Assets",
            StackPane::Stage => "Stage",
            StackPane::Timeline => "Timeline",
            StackPane::PianoRoll => "Piano Roll",
            StackPane::VirtualPiano => "Keys",
            StackPane::NodeInstrument => {
                if show_instruments {
                    "Instruments"
                } else {
                    "Nodes"
                }
            }
            StackPane::ScriptEditor => "Script",
        }
    }
}

/// `pane_instances` key for a stack slot.
fn slot_path(slot: usize) -> NodePath {
    vec![MOBILE_NS, slot]
}

/// An in-progress drag of a stack handle.
#[derive(Debug, Clone, Copy)]
pub struct StackDrag {
    pub handle: stack::Handle,
    /// Accumulated pointer offset (px) since the drag began (downward positive).
    pub offset: f32,
}

/// Persistent mobile-shell state, cached on `EditorApp`.
pub struct MobileState {
    /// Index into `STACK` of the topmost visible pane.
    pub window_top: usize,
    /// Number of visible panes (2 or 3).
    pub window_count: usize,
    /// Node/Instrument band: false = node editor, true = instrument/preset browser.
    pub show_instruments: bool,
    /// Active handle drag (transient).
    pub drag: Option<StackDrag>,
}

impl Default for MobileState {
    fn default() -> Self {
        Self {
            // Launch on {Stage, Timeline}.
            window_top: 2,
            window_count: 2,
            show_instruments: false,
            drag: None,
        }
    }
}

/// Render the whole mobile shell into `available_rect`.
pub fn render_mobile_shell(
    ui: &mut egui::Ui,
    available_rect: egui::Rect,
    rc: &mut RenderContext,
    state: &mut MobileState,
) {
    // Background (wireframe device color; bands paint over most of it).
    ui.painter()
        .rect_filled(available_rect, 0.0, egui::Color32::from_rgb(0x14, 0x16, 0x1b));

    let transport_rect = egui::Rect::from_min_max(
        egui::pos2(available_rect.left(), available_rect.bottom() - TRANSPORT_H),
        available_rect.max,
    );
    let stack_rect = egui::Rect::from_min_max(
        available_rect.min,
        egui::pos2(available_rect.right(), transport_rect.top()),
    );

    stack::render(ui, stack_rect, rc, state);

    // Transport floor: drawn last = always on top, the persistent spine.
    transport::render(ui, transport_rect, &mut rc.shared);
}
