//! Omnibutton — a floating action button whose radial holds the active layer's drawing tools plus
//! a "+New" petal (create layers / import / clip ops) and, when there are extra tools, a "⋯ more"
//! petal that opens a full-tools grid. Tools drive `shared.selected_tool`; create actions dispatch
//! `MenuAction`s. The radial fans up and to the left, away from the thumb. Pane-internal adds
//! (nodes/notes/instruments) are handled inside their panes (P6), not here.

use eframe::egui;
use lightningbeam_core::layer::{AnyLayer, LayerType};
use lightningbeam_core::selection::FocusSelection;
use lightningbeam_core::tool::Tool;

use super::{icons, MobileState};
use crate::menu::MenuAction;
use crate::RenderContext;

const C_AMBER: egui::Color32 = egui::Color32::from_rgb(0xf4, 0xa3, 0x40);
const C_DARK: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x13, 0x0a);
const C_PANEL: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x24, 0x2c);
const C_PANEL2: egui::Color32 = egui::Color32::from_rgb(0x27, 0x2d, 0x37);
const C_LINE: egui::Color32 = egui::Color32::from_rgb(0x36, 0x3d, 0x49);
const C_BRIGHT: egui::Color32 = egui::Color32::from_rgb(0xea, 0xee, 0xf3);
const C_DIM: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x95, 0xa1);

const FAB_R: f32 = 26.0;
const IR: f32 = 20.0;
const PRIMARY: [Tool; 14] = [
    Tool::Select,
    Tool::Draw,
    Tool::Transform,
    Tool::Rectangle,
    Tool::Ellipse,
    Tool::Line,
    Tool::Polygon,
    Tool::PaintBucket,
    Tool::Gradient,
    Tool::Eyedropper,
    Tool::Text,
    Tool::Erase,
    Tool::BezierEdit,
    Tool::Split,
];
const RING_CAP: usize = 10;

#[derive(Clone, Copy)]
enum Special {
    More,
    New,
}

fn tool_icon(t: Tool) -> &'static str {
    match t {
        Tool::Select | Tool::RegionSelect => icons::MOUSE_POINTER_2,
        Tool::SelectLasso => icons::LASSO_SELECT,
        Tool::Draw | Tool::Pencil | Tool::Pen => icons::PENCIL,
        Tool::Transform => icons::VECTOR_SQUARE,
        Tool::Rectangle => icons::SQUARE,
        Tool::Ellipse => icons::CIRCLE,
        Tool::Line => icons::MINUS,
        Tool::Polygon => icons::HEXAGON,
        Tool::PaintBucket => icons::PAINT_BUCKET,
        Tool::Gradient => icons::BLEND,
        Tool::Eyedropper => icons::PIPETTE,
        Tool::Text => icons::TYPE,
        Tool::Erase => icons::ERASER,
        Tool::BezierEdit => icons::PEN_TOOL,
        Tool::MagicWand | Tool::QuickSelect => icons::WAND_SPARKLES,
        Tool::Split => icons::SCISSORS,
        _ => icons::BRUSH,
    }
}

fn active_layer_type(rc: &RenderContext) -> Option<LayerType> {
    rc.shared
        .active_layer_id
        .and_then(|id| rc.shared.action_executor.document().get_layer(&id))
        .map(|layer| match layer {
            AnyLayer::Vector(_) => LayerType::Vector,
            AnyLayer::Audio(_) => LayerType::Audio,
            AnyLayer::Video(_) => LayerType::Video,
            AnyLayer::Effect(_) => LayerType::Effect,
            AnyLayer::Group(_) => LayerType::Group,
            AnyLayer::Raster(_) => LayerType::Raster,
            AnyLayer::Text(_) => LayerType::Text,
        })
}

/// The create-menu items: (label, icon, action, enabled). Split/Duplicate need a selected clip.
fn create_items(rc: &RenderContext) -> Vec<(&'static str, &'static str, MenuAction, bool)> {
    let clip_sel = matches!(&*rc.shared.focus, FocusSelection::ClipInstances(ids) if !ids.is_empty());
    vec![
        ("Vector", icons::PEN_TOOL, MenuAction::AddLayer, true),
        ("Audio", icons::AUDIO_WAVEFORM, MenuAction::AddAudioTrack, true),
        ("MIDI", icons::PIANO, MenuAction::AddMidiTrack, true),
        ("Raster", icons::BRUSH, MenuAction::AddRasterLayer, true),
        ("Video", icons::CLAPPERBOARD, MenuAction::AddVideoLayer, true),
        ("Group", icons::FOLDER, MenuAction::Group, true),
        ("Import", icons::FILE_PLUS, MenuAction::Import, true),
        ("Split", icons::SCISSORS, MenuAction::SplitClip, clip_sel),
        ("Duplicate", icons::COPY, MenuAction::DuplicateClip, clip_sel),
    ]
}

fn close_all(state: &mut MobileState) {
    state.omni_open = false;
    state.omni_grid_open = false;
    state.omni_create_open = false;
}

pub fn render(ui: &mut egui::Ui, region: egui::Rect, rc: &mut RenderContext, state: &mut MobileState) {
    let ctx_tools: Vec<Tool> = Tool::for_layer_type(active_layer_type(rc)).to_vec();
    let primary: Vec<Tool> = PRIMARY
        .iter()
        .copied()
        .filter(|t| ctx_tools.contains(t))
        .take(RING_CAP)
        .collect();
    let has_more = ctx_tools.len() > primary.len();

    // Tools are only relevant when the Stage is on screen. Off-Stage the omnibutton is purely a
    // "+New" create button.
    let stage_slot = super::STACK
        .iter()
        .position(|p| *p == super::StackPane::Stage)
        .unwrap_or(2);
    let stage_visible = (state.window_top..state.window_top + state.window_count).contains(&stage_slot);
    if !stage_visible {
        state.omni_open = false; // no tool radial off-Stage
    }

    let fab_center = egui::pos2(region.right() - 18.0 - FAB_R, region.bottom() - 18.0 - FAB_R);

    if state.omni_grid_open {
        // Full-tools grid.
        let cells: Vec<Cell> = ctx_tools
            .iter()
            .map(|t| Cell {
                icon: tool_icon(*t),
                label: t.display_name(),
                selected: *rc.shared.selected_tool == *t,
                enabled: true,
            })
            .collect();
        let (close, clicked) = draw_grid(ui, region, "Tools", &cells);
        if let Some(i) = clicked {
            *rc.shared.selected_tool = ctx_tools[i];
            close_all(state);
        } else if close {
            close_all(state);
        }
    } else if state.omni_create_open {
        // Create grid.
        let items = create_items(rc);
        let cells: Vec<Cell> = items
            .iter()
            .map(|it| Cell {
                icon: it.1,
                label: it.0,
                selected: false,
                enabled: it.3,
            })
            .collect();
        let (close, clicked) = draw_grid(ui, region, "New", &cells);
        if let Some(i) = clicked {
            rc.shared.pending_menu_actions.push(items[i].2);
            close_all(state);
        } else if close {
            close_all(state);
        }
    } else if state.omni_open {
        // Radial: scrim, then tool petals + special petals (more / new).
        let scrim = ui.interact(region, ui.id().with("mobile_omni_scrim"), egui::Sense::click());
        ui.painter()
            .rect_filled(region, 0.0, egui::Color32::from_rgba_premultiplied(8, 10, 14, 140));
        if scrim.clicked() {
            state.omni_open = false;
        }

        let mut specials: Vec<Special> = Vec::new();
        if has_more {
            specials.push(Special::More);
        }
        specials.push(Special::New);
        let n = primary.len() + specials.len();

        for k in 0..n {
            let frac = if n > 1 { k as f32 / (n as f32 - 1.0) } else { 0.5 };
            let deg = 74.0 + frac * 126.0; // 74°..200°, up and to the left
            let a = deg.to_radians();
            let radius = if k % 2 == 0 { 114.0 } else { 170.0 };
            let c = fab_center + egui::vec2(radius * a.cos(), -radius * a.sin());
            let rect = egui::Rect::from_center_size(c, egui::vec2(IR * 2.0, IR * 2.0));

            if k < primary.len() {
                let t = primary[k];
                let resp = ui.interact(rect, ui.id().with(("mobile_omni_tool", k)), egui::Sense::click());
                let selected = *rc.shared.selected_tool == t;
                petal(ui, c, if selected { C_AMBER } else { petal_bg(&resp) }, tool_icon(t), if selected { C_DARK } else { C_BRIGHT });
                if resp.clicked() {
                    *rc.shared.selected_tool = t;
                    state.omni_open = false;
                }
            } else {
                match specials[k - primary.len()] {
                    Special::More => {
                        let resp = ui.interact(rect, ui.id().with("mobile_omni_more"), egui::Sense::click());
                        petal(ui, c, petal_bg(&resp), icons::MENU, C_BRIGHT);
                        if resp.clicked() {
                            state.omni_grid_open = true;
                        }
                    }
                    Special::New => {
                        let resp = ui.interact(rect, ui.id().with("mobile_omni_new"), egui::Sense::click());
                        petal(ui, c, C_AMBER, icons::PLUS, C_DARK);
                        if resp.hovered() {
                            ui.painter().circle_stroke(c, IR, egui::Stroke::new(1.5, C_BRIGHT));
                        }
                        if resp.clicked() {
                            state.omni_create_open = true;
                        }
                    }
                }
            }
        }
    }

    // The FAB itself (on top). Closed → current tool icon; open → ✕.
    let fab_rect = egui::Rect::from_center_size(fab_center, egui::vec2(FAB_R * 2.0, FAB_R * 2.0));
    let fresp = ui.interact(fab_rect, ui.id().with("mobile_omni_fab"), egui::Sense::click());
    ui.painter().circle_filled(fab_center, FAB_R, C_AMBER);
    let open = state.omni_open || state.omni_grid_open || state.omni_create_open;
    let glyph = if open {
        icons::X
    } else if stage_visible {
        tool_icon(*rc.shared.selected_tool)
    } else {
        icons::PLUS // off-Stage: a create button
    };
    ui.painter()
        .text(fab_center, egui::Align2::CENTER_CENTER, glyph, icons::font(22.0), C_DARK);
    if fresp.clicked() {
        if open {
            close_all(state);
        } else if stage_visible {
            state.omni_open = true;
        } else {
            state.omni_create_open = true;
        }
    }
}

fn petal_bg(resp: &egui::Response) -> egui::Color32 {
    if resp.hovered() {
        C_LINE
    } else {
        C_PANEL2
    }
}

fn petal(ui: &egui::Ui, c: egui::Pos2, bg: egui::Color32, glyph: &str, fg: egui::Color32) {
    ui.painter().circle_filled(c, IR, bg);
    ui.painter().circle_stroke(c, IR, egui::Stroke::new(1.0, C_LINE));
    ui.painter().text(c, egui::Align2::CENTER_CENTER, glyph, icons::font(18.0), fg);
}

struct Cell {
    icon: &'static str,
    label: &'static str,
    selected: bool,
    enabled: bool,
}

/// A modal grid sheet of icon+label cells. Returns (backdrop-tapped, Some(clicked-enabled-index)).
fn draw_grid(ui: &mut egui::Ui, region: egui::Rect, title: &str, cells: &[Cell]) -> (bool, Option<usize>) {
    let scrim = ui.interact(region, ui.id().with(("mobile_omni_gridscrim", title)), egui::Sense::click());
    ui.painter()
        .rect_filled(region, 0.0, egui::Color32::from_rgba_premultiplied(8, 10, 14, 170));

    let panel = region.shrink2(egui::vec2(16.0, 40.0));
    ui.painter().rect_filled(panel, 14.0, C_PANEL);
    ui.painter().rect_stroke(panel, 14.0, egui::Stroke::new(1.0, C_LINE), egui::StrokeKind::Inside);
    ui.painter().text(
        egui::pos2(panel.left() + 16.0, panel.top() + 18.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(14.0),
        C_BRIGHT,
    );

    let cols = 4usize;
    let n = cells.len();
    let rows = n.div_ceil(cols).max(1);
    let grid = egui::Rect::from_min_max(
        egui::pos2(panel.left() + 10.0, panel.top() + 36.0),
        egui::pos2(panel.right() - 10.0, panel.bottom() - 10.0),
    );
    let cw = grid.width() / cols as f32;
    let chh = (grid.height() / rows as f32).min(96.0);
    let mut clicked = None;
    for (i, cell) in cells.iter().enumerate() {
        let col = (i % cols) as f32;
        let row = (i / cols) as f32;
        let r = egui::Rect::from_min_size(
            egui::pos2(grid.left() + col * cw, grid.top() + row * chh),
            egui::vec2(cw, chh),
        )
        .shrink(5.0);
        let resp = if cell.enabled {
            ui.interact(r, ui.id().with(("mobile_omni_cell", title, i)), egui::Sense::click())
        } else {
            ui.interact(r, ui.id().with(("mobile_omni_cell_off", title, i)), egui::Sense::hover())
        };
        let bg = if cell.selected {
            C_AMBER
        } else if cell.enabled && resp.hovered() {
            C_PANEL2
        } else {
            C_PANEL
        };
        ui.painter().rect_filled(r, 10.0, bg);
        ui.painter().rect_stroke(r, 10.0, egui::Stroke::new(1.0, C_LINE), egui::StrokeKind::Inside);
        let fg = if cell.selected {
            C_DARK
        } else if cell.enabled {
            C_BRIGHT
        } else {
            C_LINE
        };
        ui.painter().text(
            egui::pos2(r.center().x, r.top() + r.height() * 0.38),
            egui::Align2::CENTER_CENTER,
            cell.icon,
            icons::font(22.0),
            fg,
        );
        ui.painter().text(
            egui::pos2(r.center().x, r.bottom() - 12.0),
            egui::Align2::CENTER_CENTER,
            cell.label,
            egui::FontId::proportional(9.5),
            if cell.selected { C_DARK } else if cell.enabled { C_DIM } else { C_LINE },
        );
        if cell.enabled && resp.clicked() {
            clicked = Some(i);
        }
    }
    (scrim.clicked() && clicked.is_none(), clicked)
}
