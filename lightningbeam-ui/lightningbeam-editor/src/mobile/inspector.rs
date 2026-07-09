//! Selection inspector bottom sheet. When something is selected/focused it rises above the
//! transport, showing the focused object's properties by reusing `InfopanelPane` full-bleed.
//! Jump-to chips slide the stack window to the related surface; the ✕ deselects.

use eframe::egui;
use lightningbeam_core::pane::PaneType;
use lightningbeam_core::selection::FocusSelection;

use super::{surface, MobileState, Palette, MOBILE_NS};
use crate::panes::{NodePath, SharedPaneState};
use crate::RenderContext;

const GRAB_H: f32 = 16.0;
const HEAD_H: f32 = 44.0;

fn inspector_path() -> NodePath {
    vec![MOBILE_NS, 200]
}

/// Whether anything is selected/focused (i.e. the inspector should be shown).
pub fn is_active(shared: &SharedPaneState) -> bool {
    !shared.focus.is_none() || !shared.selection.is_empty()
}

/// A cheap content-sensitive signature of the current selection, so the shell can detect when the
/// selection *changes* (to re-show a manually-dismissed inspector for the new thing).
pub fn selection_sig(shared: &SharedPaneState) -> u64 {
    let mut h: u64 = 0;
    for id in shared.selection.clip_instances() {
        h ^= (id.as_u128() as u64).wrapping_mul(0x9E3779B97F4A7C15);
    }
    for f in shared.selection.selected_fills() {
        h ^= (f.idx() as u64).wrapping_mul(0xD1B54A32D192ED03);
    }
    for e in shared.selection.selected_edges() {
        h ^= (e.idx() as u64).wrapping_mul(0xA24BAED4963EE407);
    }
    h
}

/// The stack slot (see `super::STACK`) where the current selection lives, so we can tell if the
/// sheet would cover it. Geometry/selection lives on the Stage; clips/layers on the Timeline; etc.
#[allow(dead_code)] // selection→pane mapping; kept for reflow/jump heuristics
pub fn target_slot(shared: &SharedPaneState) -> usize {
    match &*shared.focus {
        FocusSelection::Notes { .. } => 4,          // PianoRoll
        FocusSelection::Nodes(_) => 5,              // Node/Instrument
        FocusSelection::Assets(_) => 1,             // Asset Library
        FocusSelection::ClipInstances(_) | FocusSelection::Layers(_) => 3, // Timeline
        FocusSelection::Geometry { .. } | FocusSelection::None => 2,       // Stage
    }
}

/// A short title describing what's selected.
fn title(shared: &SharedPaneState) -> String {
    use lightningbeam_core::layer::{AnyLayer, AudioLayerType};
    let plural = |n: usize, s: &str| {
        if n == 1 {
            format!("1 {s}")
        } else {
            format!("{n} {s}s")
        }
    };
    match &*shared.focus {
        FocusSelection::Layers(ids) => {
            let doc = shared.action_executor.document();
            match ids.len() {
                0 => "Layer".to_string(),
                1 => {
                    if let Some(l) = doc.get_layer(&ids[0]) {
                        let ty = match l {
                            AnyLayer::Vector(_) => "Vector",
                            AnyLayer::Audio(a) => match a.audio_layer_type {
                                AudioLayerType::Midi => "MIDI",
                                AudioLayerType::Sampled => "Audio",
                            },
                            AnyLayer::Video(_) => "Video",
                            AnyLayer::Effect(_) => "Effect",
                            AnyLayer::Group(_) => "Group",
                            AnyLayer::Raster(_) => "Raster",
                            AnyLayer::Text(_) => "Text",
                        };
                        format!("{} · {} layer", l.name(), ty)
                    } else {
                        "Layer".to_string()
                    }
                }
                n => plural(n, "layer"),
            }
        }
        FocusSelection::ClipInstances(ids) => plural(ids.len(), "clip"),
        FocusSelection::Notes { indices, .. } => plural(indices.len().max(1), "note"),
        FocusSelection::Nodes(ids) => plural(ids.len(), "node"),
        FocusSelection::Assets(ids) => plural(ids.len(), "asset"),
        FocusSelection::Geometry { .. } | FocusSelection::None => "Selection".to_string(),
    }
}

/// A jump-to chip: its label and the stack window it brings into view (top, count). These reframe
/// to a related surface with the object still selected.
struct Chip {
    label: &'static str,
    window: (usize, usize),
}

const CHIPS: [Chip; 2] = [
    Chip { label: "Timeline", window: (3, 1) }, // Timeline = STACK index 3
    Chip { label: "Nodes", window: (5, 1) },    // Node/Instrument = STACK index 5
];

pub fn render(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    region: egui::Rect,
    rc: &mut RenderContext,
    state: &mut MobileState,
    pal: &Palette,
    landscape: bool,
) {
    // Panel background + border, with rounded corners on the edge that faces the content (top in
    // portrait, left in landscape).
    let radius = if landscape {
        egui::CornerRadius { nw: 14, ne: 0, sw: 14, se: 0 }
    } else {
        egui::CornerRadius { nw: 14, ne: 14, sw: 0, se: 0 }
    };
    ui.painter().rect_filled(rect, radius, pal.surface_alt);
    ui.painter().rect_stroke(rect, radius, egui::Stroke::new(1.0, pal.line), egui::StrokeKind::Inside);

    // Grab handle — top strip (portrait, drags height) or left strip (landscape, drags width).
    // `content_area` is the panel minus the grab strip.
    let content_area = if landscape {
        let grab = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.left() + GRAB_H, rect.bottom()));
        let gresp = ui.interact(grab, ui.id().with("mobile_inspector_grab"), egui::Sense::drag());
        let pill = egui::Rect::from_center_size(grab.center(), egui::vec2(4.0, 34.0));
        ui.painter().rect_filled(pill, 2.0, if gresp.hovered() || gresp.dragged() { pal.accent } else { pal.line });
        if gresp.dragged() && region.width() > 1.0 {
            state.inspector_width_frac = (state.inspector_width_frac - gresp.drag_delta().x / region.width()).clamp(0.2, 0.7);
        }
        if gresp.hovered() || gresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        egui::Rect::from_min_max(egui::pos2(grab.right(), rect.top()), rect.max)
    } else {
        let grab = egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), rect.top() + GRAB_H));
        let gresp = ui.interact(grab, ui.id().with("mobile_inspector_grab"), egui::Sense::drag());
        let pill = egui::Rect::from_center_size(grab.center(), egui::vec2(34.0, 4.0));
        ui.painter().rect_filled(pill, 2.0, if gresp.hovered() || gresp.dragged() { pal.accent } else { pal.line });
        if gresp.dragged() && region.height() > 1.0 {
            state.inspector_frac = (state.inspector_frac - gresp.drag_delta().y / region.height()).clamp(0.2, 0.85);
        }
        if gresp.hovered() || gresp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        egui::Rect::from_min_max(egui::pos2(rect.left(), grab.bottom()), rect.max)
    };

    // Single header row (egui widgets — they inherit the mobile touch sizing + theme visuals):
    // title on the left, [Timeline] [Nodes] jump buttons + ✕ close on the right.
    let head = egui::Rect::from_min_max(
        content_area.min,
        egui::pos2(content_area.right(), content_area.top() + HEAD_H),
    );
    let title_text = title(&rc.shared);
    let mut jump: Option<(usize, usize)> = None;
    let mut do_close = false;
    let mut hui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(head.shrink2(egui::vec2(12.0, 5.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    hui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .add(egui::Button::new(egui::RichText::new(super::icons::X).font(super::icons::font(18.0))).frame(false))
            .clicked()
        {
            do_close = true;
        }
        ui.add_space(4.0);
        // Reversed so the visual order stays Timeline, then Nodes.
        for chip in CHIPS.iter().rev() {
            if ui.button(chip.label).clicked() {
                jump = Some(chip.window);
            }
        }
        // Title fills the remaining space on the left.
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(title_text).color(pal.text)).truncate());
        });
    });
    if do_close {
        rc.shared.selection.clear();
        *rc.shared.focus = FocusSelection::None;
    }
    if let Some((top, count)) = jump {
        state.window_top = top;
        state.window_count = count;
        state.weights = [1.0, 1.0, 1.0];
        state.anim = None;
    }

    // Properties content — reuse the Infopanel full-bleed.
    let content = egui::Rect::from_min_max(
        egui::pos2(content_area.left(), head.bottom()),
        content_area.max,
    );
    if content.height() > 1.0 {
        surface::render_surface_fullbleed(ui, content, &inspector_path(), PaneType::Infopanel, rc);
    }
}
