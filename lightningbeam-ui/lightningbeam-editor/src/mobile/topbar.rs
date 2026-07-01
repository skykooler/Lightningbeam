//! The mobile top bar: the project filename on the left, and on the right ⌕ (command palette) and
//! ⋯ (overflow commands). Both open a modal sheet whose items dispatch `MenuAction`s (which
//! `main.rs::handle_menu_action` already implements). Per the wireframe these are the
//! "commands/destinations", as opposed to the omnibutton's "tools/objects".

use eframe::egui;

use super::{icons, MobileState, Palette};
use crate::menu::{MenuAction, MenuDef, MenuItemDef};
use crate::RenderContext;

const BTN: f32 = 40.0;

/// Curated overflow (⋯) commands.
fn overflow_items() -> [(&'static str, MenuAction); 9] {
    [
        ("Save", MenuAction::Save),
        ("Save As…", MenuAction::SaveAs),
        ("Open File…", MenuAction::OpenFile),
        ("New file…", MenuAction::NewFile),
        ("Import…", MenuAction::Import),
        ("Export…", MenuAction::Export),
        ("Undo", MenuAction::Undo),
        ("Redo", MenuAction::Redo),
        ("Preferences", MenuAction::Preferences),
    ]
}

/// Flatten the whole menu tree into (path-label, action) for the command palette.
fn flatten(defs: &'static [MenuDef], prefix: &str, out: &mut Vec<(String, MenuAction)>) {
    for d in defs {
        match d {
            MenuDef::Item(item) => out.push((format!("{prefix}{}", item.label), item.action)),
            MenuDef::Submenu { label, children } => {
                flatten(children, &format!("{prefix}{label} › "), out);
            }
            MenuDef::Separator => {}
        }
    }
}

fn all_commands() -> Vec<(String, MenuAction)> {
    let mut v = Vec::new();
    flatten(MenuItemDef::menu_structure(), "", &mut v);
    v
}

pub fn render(
    ui: &mut egui::Ui,
    bar: egui::Rect,
    full: egui::Rect,
    rc: &mut RenderContext,
    state: &mut MobileState,
    pal: &Palette,
) {
    ui.painter().rect_filled(bar, 0.0, pal.header);
    ui.painter().hline(bar.x_range(), bar.bottom(), egui::Stroke::new(1.0, pal.line));

    // Filename (or app name when unsaved).
    let name = rc
        .shared
        .container_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Lightningbeam".to_string());
    ui.painter().text(
        egui::pos2(bar.left() + 14.0, bar.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::proportional(14.0),
        pal.text,
    );

    // Right cluster: ⌕ (palette) then ⋯ (overflow).
    let overflow_rect = egui::Rect::from_min_size(egui::pos2(bar.right() - BTN, bar.top()), egui::vec2(BTN, bar.height()));
    let palette_rect = egui::Rect::from_min_size(egui::pos2(bar.right() - 2.0 * BTN, bar.top()), egui::vec2(BTN, bar.height()));

    let sresp = ui.interact(palette_rect, ui.id().with("mobile_topbar_search"), egui::Sense::click());
    ui.painter().text(palette_rect.center(), egui::Align2::CENTER_CENTER, icons::SEARCH, icons::font(17.0),
        if sresp.hovered() || state.palette_open { pal.text } else { pal.text_dim });
    if sresp.clicked() {
        state.palette_open = !state.palette_open;
        state.overflow_open = false;
        state.palette_query.clear();
    }

    let oresp = ui.interact(overflow_rect, ui.id().with("mobile_topbar_overflow"), egui::Sense::click());
    ui.painter().text(overflow_rect.center(), egui::Align2::CENTER_CENTER, icons::ELLIPSIS, icons::font(18.0),
        if oresp.hovered() || state.overflow_open { pal.text } else { pal.text_dim });
    if oresp.clicked() {
        state.overflow_open = !state.overflow_open;
        state.palette_open = false;
    }

    if state.overflow_open {
        render_overflow(ui, full, rc, state, pal);
    } else if state.palette_open {
        render_palette(ui, full, rc, state, pal);
    }
}

/// Common modal scrim + panel. Returns (backdrop-tapped, panel inner rect).
fn open_panel(ui: &mut egui::Ui, full: egui::Rect, id: &str, pal: &Palette) -> (bool, egui::Rect) {
    let scrim = ui.interact(full, ui.id().with(("mobile_topbar_scrim", id)), egui::Sense::click());
    ui.painter().rect_filled(full, 0.0, pal.scrim);
    let panel = egui::Rect::from_min_max(
        egui::pos2(full.left() + 16.0, full.top() + 44.0),
        egui::pos2(full.right() - 16.0, full.bottom() - 60.0),
    );
    ui.painter().rect_filled(panel, 14.0, pal.surface);
    ui.painter().rect_stroke(panel, 14.0, egui::Stroke::new(1.0, pal.line), egui::StrokeKind::Inside);
    (scrim.clicked(), panel)
}

fn command_button(ui: &mut egui::Ui, r: egui::Rect, label: &str, key: (&str, usize), pal: &Palette) -> bool {
    let resp = ui.interact(r, ui.id().with(("mobile_cmd", key.0, key.1)), egui::Sense::click());
    ui.painter().rect_filled(r, 8.0, if resp.hovered() { pal.surface_alt } else { pal.surface });
    ui.painter().hline(r.x_range(), r.bottom(), egui::Stroke::new(1.0, pal.line));
    ui.painter().text(
        egui::pos2(r.left() + 14.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        pal.text,
    );
    resp.clicked()
}

fn render_overflow(ui: &mut egui::Ui, full: egui::Rect, rc: &mut RenderContext, state: &mut MobileState, pal: &Palette) {
    let (backdrop, panel) = open_panel(ui, full, "overflow", pal);
    let mut close = backdrop;
    let items = overflow_items();
    let row_h = 44.0;
    let inner = panel.shrink(8.0);
    for (i, (label, action)) in items.iter().enumerate() {
        let r = egui::Rect::from_min_size(
            egui::pos2(inner.left(), inner.top() + i as f32 * row_h),
            egui::vec2(inner.width(), row_h),
        );
        if r.bottom() > inner.bottom() {
            break;
        }
        if command_button(ui, r, label, ("of", i), pal) {
            rc.shared.pending_menu_actions.push(*action);
            close = true;
        }
    }
    if close {
        state.overflow_open = false;
    }
}

fn render_palette(ui: &mut egui::Ui, full: egui::Rect, rc: &mut RenderContext, state: &mut MobileState, pal: &Palette) {
    let (backdrop, panel) = open_panel(ui, full, "palette", pal);
    let mut close = backdrop;
    let inner = panel.shrink(8.0);

    // Search field (real egui widget).
    let field = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), 30.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let te = child.add(
        egui::TextEdit::singleline(&mut state.palette_query)
            .hint_text("Search commands…")
            .desired_width(inner.width()),
    );
    te.request_focus();

    // Filtered list.
    let q = state.palette_query.to_lowercase();
    let cmds = all_commands();
    let row_h = 38.0;
    let list_top = inner.top() + 38.0;
    let mut y = list_top;
    for (i, (label, action)) in cmds.iter().enumerate() {
        if !q.is_empty() && !label.to_lowercase().contains(&q) {
            continue;
        }
        let r = egui::Rect::from_min_size(egui::pos2(inner.left(), y), egui::vec2(inner.width(), row_h));
        if r.bottom() > inner.bottom() {
            break;
        }
        if command_button(ui, r, label, ("pal", i), pal) {
            rc.shared.pending_menu_actions.push(*action);
            close = true;
        }
        y += row_h;
    }
    if close {
        state.palette_open = false;
    }
}
