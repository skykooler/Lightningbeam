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
mod inspector;
pub mod intent;
mod omni;
mod palette;
mod stack;
mod surface;
mod topbar;
mod transport;

use palette::Palette;

/// Reserved sentinel namespace for mobile pane-instance paths. Desktop layout paths are built
/// from small child indices, so prefixing with `usize::MAX` guarantees mobile slots never alias a
/// real layout path in the shared `pane_instances` map.
pub const MOBILE_NS: usize = usize::MAX;

const TRANSPORT_H: f32 = 60.0;
const TOPBAR_H: f32 = 40.0;

/// Clamp a desktop dialog width to fit the current screen (with side margins). A no-op on wide
/// desktop screens (`min` keeps the desired width); on a phone-aspect window it shrinks to fit.
pub fn dialog_width(ctx: &egui::Context, desired: f32) -> f32 {
    let avail = ctx.screen_rect().width() - 24.0;
    desired.min(avail.max(200.0))
}

/// Enlarge egui's spacing/sizing so the standard widgets (buttons, dropdowns, sliders, text fields)
/// in panes and dialogs are touch-friendly. Applied every frame after the theme visuals when the
/// mobile shell is active. The mobile chrome (transport, omnibutton, headers) is hand-sized already;
/// this targets the egui-widget content inside panes.
pub fn apply_touch_style(ctx: &egui::Context) {
    use egui::{FontId, TextStyle};
    ctx.style_mut(|s| {
        let sp = &mut s.spacing;
        // Minimum touch target height for buttons/sliders/checkboxes.
        sp.interact_size = egui::vec2(sp.interact_size.x.max(44.0), 38.0);
        sp.button_padding = egui::vec2(12.0, 9.0);
        sp.item_spacing = egui::vec2(10.0, 10.0);
        sp.slider_width = 200.0;
        sp.slider_rail_height = 10.0;
        sp.combo_width = 200.0;
        sp.combo_height = 360.0;
        sp.text_edit_width = 220.0;
        sp.icon_width = 24.0;
        sp.icon_width_inner = 14.0;
        sp.icon_spacing = 8.0;
        sp.scroll.bar_width = 16.0;
        sp.menu_margin = egui::Margin::same(8);
        // Larger text for touch legibility.
        s.text_styles.insert(TextStyle::Body, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Button, FontId::proportional(15.0));
        s.text_styles.insert(TextStyle::Monospace, FontId::monospace(13.0));
        s.text_styles.insert(TextStyle::Small, FontId::proportional(12.0));
    });
}

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

/// A short ease between two stack layouts (resize snap, membership change, or fullscreen toggle).
/// Both endpoints are full configs so the same animation covers panes resizing AND panes
/// entering/leaving the window.
#[derive(Debug, Clone, Copy)]
pub struct LayoutAnim {
    pub from_top: usize,
    pub from_count: usize,
    pub from_w: [f32; 3],
    pub to_top: usize,
    pub to_count: usize,
    pub to_w: [f32; 3],
    /// egui time (seconds) when the animation started (may be back-dated to continue a drag).
    pub start: f64,
}

/// Persistent mobile-shell state, cached on `EditorApp`.
pub struct MobileState {
    /// Index into `STACK` of the topmost visible pane.
    pub window_top: usize,
    /// Number of visible panes (1, 2, or 3; 1 = a single pane filling the stack).
    pub window_count: usize,
    /// Relative heights of the visible panes (first `window_count` entries are used; normalized on
    /// use). Reset to even on any membership change; adjusted by intermediate divider snapping.
    pub weights: [f32; 3],
    /// Node/Instrument band: false = node editor, true = instrument/preset browser.
    pub show_instruments: bool,
    /// Inspector sheet height as a fraction of the region above the transport.
    pub inspector_frac: f32,
    /// Whether the inspector sheet is currently shown. Gated to appear on pointer *release* (a tap),
    /// not on press, so press+drag interactions aren't interrupted by the sheet popping up.
    pub inspector_visible: bool,
    /// Screen-y of the tap that opened the inspector — used to decide whether the tapped thing would
    /// be hidden by the sheet (only then do we reflow the stack).
    pub inspector_anchor_y: f32,
    /// Set when the user taps outside the sheet to dismiss it; suppresses re-showing until the
    /// selection changes. Reset when the selection signature below changes.
    pub inspector_dismissed: bool,
    /// Cheap signature of the current selection, to detect when it changes (re-show the inspector).
    pub inspector_sel_sig: u64,
    /// Whether the omnibutton radial tool menu is open.
    pub omni_open: bool,
    /// Whether the omnibutton "more" grid (all tools) is open.
    pub omni_grid_open: bool,
    /// Whether the omnibutton "+New" create grid is open.
    pub omni_create_open: bool,
    /// Top-bar overflow (⋯ commands) sheet open.
    pub overflow_open: bool,
    /// Top-bar command palette (⌕) open, and its search query.
    pub palette_open: bool,
    pub palette_query: String,
    /// Active handle drag (transient).
    pub drag: Option<StackDrag>,
    /// In-flight layout ease (transient).
    pub anim: Option<LayoutAnim>,
}

impl Default for MobileState {
    fn default() -> Self {
        Self {
            // Launch on {Stage, Timeline}.
            window_top: 2,
            window_count: 2,
            weights: [1.0, 1.0, 1.0],
            show_instruments: false,
            inspector_frac: 0.45,
            inspector_visible: false,
            inspector_anchor_y: 0.0,
            inspector_dismissed: false,
            inspector_sel_sig: 0,
            omni_open: false,
            omni_grid_open: false,
            omni_create_open: false,
            overflow_open: false,
            palette_open: false,
            palette_query: String::new(),
            drag: None,
            anim: None,
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
    let pal = Palette::from_theme(rc.shared.theme, ui.ctx());

    // Background (device color; bands paint over most of it).
    ui.painter().rect_filled(available_rect, 0.0, pal.bg);

    let topbar_rect = egui::Rect::from_min_max(
        available_rect.min,
        egui::pos2(available_rect.right(), available_rect.top() + TOPBAR_H),
    );
    let transport_rect = egui::Rect::from_min_max(
        egui::pos2(available_rect.left(), available_rect.bottom() - TRANSPORT_H),
        available_rect.max,
    );
    // Region between the top bar and the transport, shared between the stack and (when something is
    // selected) the inspector sheet that rises above the transport.
    let region = egui::Rect::from_min_max(
        egui::pos2(available_rect.left(), topbar_rect.bottom()),
        egui::pos2(available_rect.right(), transport_rect.top()),
    );
    // The inspector sheet appears on pointer *release* (a tap), not on press — so press+drag on the
    // canvas isn't interrupted by the sheet popping up. Track visibility: hide when nothing's
    // selected; show once the pointer is up with a selection; keep it shown while it's being
    // interacted with (pointer down but still selected).
    let active = inspector::is_active(&rc.shared);
    let any_down = ui.input(|i| i.pointer.any_down());
    let sel_sig = inspector::selection_sig(&rc.shared);
    if sel_sig != state.inspector_sel_sig {
        // Selection changed → a fresh thing to inspect; clear any prior manual dismissal.
        state.inspector_dismissed = false;
        state.inspector_sel_sig = sel_sig;
    }
    if !active {
        state.inspector_visible = false;
        state.inspector_dismissed = false;
    } else if !state.inspector_dismissed && !any_down {
        if !state.inspector_visible {
            // Just opened (tap released): anchor to the release position for the reflow test below.
            state.inspector_anchor_y = ui
                .input(|i| i.pointer.interact_pos())
                .map(|p| p.y)
                .unwrap_or(region.bottom());
        }
        state.inspector_visible = true;
    }
    let mut inspector_shown = state.inspector_visible;
    let sheet_h = if inspector_shown {
        (region.height() * state.inspector_frac).clamp(120.0, region.height() - 60.0)
    } else {
        0.0
    };
    let mut sheet_top = region.bottom() - sheet_h;

    // Tapping outside the sheet (but not on the transport) dismisses (hides) the inspector. We only
    // hide it — NOT clear the selection — so actions dispatched from overlays (context menu, "+New"
    // grid) still see the selection. It stays dismissed until the selection changes (sig above).
    if inspector_shown {
        let sheet_rect = egui::Rect::from_min_max(egui::pos2(region.left(), sheet_top), region.max);
        let dismiss = ui.input(|i| {
            i.pointer.primary_pressed()
                && i.pointer.press_origin().map_or(false, |p| {
                    !sheet_rect.contains(p) && !transport_rect.contains(p)
                })
        });
        if dismiss {
            state.inspector_visible = false;
            state.inspector_dismissed = true;
            inspector_shown = false;
            sheet_top = region.bottom();
        }
    }

    // Reflow (shrink the stack above the sheet) only when the thing that was tapped would actually be
    // hidden by the sheet — i.e. the tap landed below where the sheet now sits. Otherwise just
    // overlay. Restoring on dismiss is automatic — reflow only changes the render rect.
    let covered = inspector_shown && state.inspector_anchor_y > sheet_top;
    let stack_rect = if covered {
        egui::Rect::from_min_max(region.min, egui::pos2(region.right(), sheet_top))
    } else {
        region
    };

    stack::render(ui, stack_rect, rc, state, &pal);

    if inspector_shown {
        let sheet_rect = egui::Rect::from_min_max(egui::pos2(region.left(), sheet_top), region.max);
        inspector::render(ui, sheet_rect, region.height(), rc, state, &pal);
    }

    // Transport floor: drawn last = always on top, the persistent spine.
    transport::render(ui, transport_rect, &mut rc.shared, &pal);

    // Omnibutton FAB (radial tool menu) — drawn above the stack region, on top of everything else.
    omni::render(ui, region, rc, state, &pal);

    // Top bar (filename + ⌕ palette + ⋯ commands). Its menus overlay the whole shell, so it's last.
    topbar::render(ui, topbar_rect, available_rect, rc, state, &pal);

    // Long-press context menu (populated by whichever pane was long-pressed). Persistent popup,
    // dispatched via pending_menu_actions.
    render_context_menu(ui, available_rect, rc);
}

/// Render the pane-populated long-press context menu (`shared.mobile_context_menu`) as a persistent
/// popup: it stays open until an item is chosen or the user taps outside it. Styled to match the
/// timeline's context menu (default themed popup frame + full-width, touch-height items).
fn render_context_menu(ui: &mut egui::Ui, available: egui::Rect, rc: &mut RenderContext) {
    let Some(menu) = rc.shared.mobile_context_menu.clone() else {
        return;
    };
    // Keep the popup on screen (rough clamp against its estimated size).
    let est = egui::vec2(200.0, menu.items.len() as f32 * ui.spacing().interact_size.y + 16.0);
    let pos = egui::pos2(
        menu.pos.x.min(available.right() - est.x - 8.0).max(available.left() + 8.0),
        menu.pos.y.min(available.bottom() - est.y - 8.0).max(available.top() + 8.0),
    );

    let mut chosen: Option<crate::menu::MenuAction> = None;
    let area = egui::Area::new(ui.id().with("mobile_ctx_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .interactable(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                for (label, action) in &menu.items {
                    let w = ui.available_width();
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(w, ui.spacing().interact_size.y), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, 2.0, ui.visuals().widgets.hovered.bg_fill);
                        }
                        let tc = if resp.hovered() {
                            ui.visuals().widgets.hovered.text_color()
                        } else {
                            ui.visuals().widgets.inactive.text_color()
                        };
                        ui.painter().text(
                            rect.min + egui::vec2(10.0, (rect.height() - 14.0) / 2.0),
                            egui::Align2::LEFT_TOP,
                            label,
                            egui::FontId::proportional(14.0),
                            tc,
                        );
                    }
                    if resp.clicked() {
                        chosen = Some(*action);
                    }
                }
            });
        });

    // Dismiss on item choice or a primary click outside the popup. (The secondary click that opened
    // it won't dismiss, so there's no just-opened race.)
    let primary_click = ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary));
    let dismiss = chosen.is_some() || (primary_click && !area.response.contains_pointer());
    if let Some(action) = chosen {
        rc.shared.pending_menu_actions.push(action);
    }
    if dismiss {
        *rc.shared.mobile_context_menu = None;
    }
}
